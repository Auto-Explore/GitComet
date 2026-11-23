//! The scan of a history that has a commit-graph.
//!
//! The graph's commits lie in the order of their IDs, so following parents
//! through it reads memory at random: a record, the row table and an ID for
//! every commit, 255 ns each on a history of 1.8 million. Once the tips are
//! seen to reach many commits, the records are read front to back into one
//! small array, on several threads, and the scan follows parents through
//! that array alone, numbering rows as it meets them, which keeps a commit
//! near its parents for the ordering that follows. The IDs are copied last,
//! again on several threads.

use super::{Scanned, check_cancelled, read_header, too_large};
use crate::repo::log::log_paged_walk_handle;
use crate::repo::log::walk::DecodeThreadBudget;
use gitcomet_core::history_perf::{Work, record};
use gix::commitgraph::{Graph, Position};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const NONE: u32 = u32::MAX;
/// As a second parent: the commit has three parents or more, all in `many`.
const MANY: u32 = u32::MAX - 1;
/// Commits a thread takes at a time, between looks at whether to stop.
const CHUNK: usize = 65_536;

/// Commits followed record by record before every record is read in bulk: a
/// record read at random costs about as much as 64 read front to back, and a
/// short branch of a large history reads few.
fn bulk_after(commits: usize) -> usize {
    (commits / 256).max(4_096)
}

fn corrupt_edge<E>(_: E) -> gix::Error {
    gix::error::corruption("Commit-graph parent edge").into()
}

/// A commit's first two parents, as positions in the graph, and its
/// committer time: one cache line for what the scan reads of a commit.
#[derive(Clone, Copy)]
struct Header {
    first: u32,
    second: u32,
    time: i64,
}

struct Headers {
    headers: Vec<Header>,
    /// Every parent of a commit that has three or more.
    many: FxHashMap<u32, Vec<u32>>,
}

/// Runs `work` on this thread and on the decode threads free to help, at
/// most `most` in all. Each is handed a check of whether to stop: only this
/// thread may ask about cancellation, and tells the others through it.
fn on_decode_threads<T: Send>(
    most: usize,
    cancelled: &dyn Fn() -> bool,
    work: impl Fn(&dyn Fn() -> bool) -> T + Sync,
) -> gix::Result<Vec<T>> {
    let budget = DecodeThreadBudget::claim();
    let helpers = budget.helpers().min(most.saturating_sub(1));
    let stop = AtomicBool::new(false);
    let done = std::thread::scope(|scope| {
        let helpers: Vec<_> = (0..helpers)
            .map(|_| scope.spawn(|| work(&|| stop.load(Ordering::Relaxed))))
            .collect();
        let mut done = vec![work(&|| {
            if cancelled() {
                stop.store(true, Ordering::Relaxed);
            }
            stop.load(Ordering::Relaxed)
        })];
        for helper in helpers {
            done.push(
                helper
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            );
        }
        done
    });
    check_cancelled(stop.load(Ordering::Relaxed))?;
    Ok(done)
}

impl Headers {
    fn read(graph: &Graph, chunk: usize, cancelled: &dyn Fn() -> bool) -> gix::Result<Self> {
        record(Work::CommitGraphRead);
        let commits = graph.num_commits() as usize;
        if commits >= MANY as usize {
            return Err(too_large());
        }
        let root = Header {
            first: NONE,
            second: NONE,
            time: 0,
        };
        let mut headers = vec![root; commits];
        let chunks = Mutex::new(headers.chunks_mut(chunk).enumerate());
        // Reads chunks until none is left: the commits with three parents or
        // more, or the corrupt edge that ended it.
        let read = |stopped: &dyn Fn() -> bool| -> gix::Result<Vec<(u32, Vec<u32>)>> {
            let mut many = Vec::new();
            loop {
                if stopped() {
                    return Ok(many);
                }
                let next = chunks
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .next();
                let Some((at_chunk, headers)) = next else {
                    return Ok(many);
                };
                for (at, header) in headers.iter_mut().enumerate() {
                    let position = (at_chunk * chunk + at) as u32;
                    let commit = graph.commit_at(Position(position));
                    header.time = commit.committer_timestamp() as i64;
                    let mut parents = commit
                        .iter_parents()
                        .map(|parent| parent.map(|parent| parent.0).map_err(corrupt_edge));
                    let Some(first) = parents.next().transpose()? else {
                        continue;
                    };
                    header.first = first;
                    let Some(second) = parents.next().transpose()? else {
                        continue;
                    };
                    header.second = second;
                    if let Some(third) = parents.next().transpose()? {
                        let mut all = vec![first, second, third];
                        for parent in parents {
                            all.push(parent?);
                        }
                        header.second = MANY;
                        many.push((position, all));
                    }
                }
            }
        };
        let mut many = FxHashMap::default();
        for read in on_decode_threads(commits.div_ceil(chunk), cancelled, read)? {
            many.extend(read?);
        }
        Ok(Self { headers, many })
    }

    /// A commit's parents in their order.
    fn parents(&self, position: u32) -> impl Iterator<Item = u32> + '_ {
        let header = self.headers[position as usize];
        let pair = [header.first, header.second];
        let (few, many): (usize, &[u32]) = if header.second == MANY {
            (0, &self.many[&position])
        } else {
            (
                pair.iter().take_while(|&&parent| parent != NONE).count(),
                &[],
            )
        };
        pair.into_iter().take(few).chain(many.iter().copied())
    }
}

/// A commit by where its header is.
#[derive(Clone, Copy)]
enum Node {
    Graph(u32),
    /// Newer than the graph, by its place in `Rows::outside`.
    Outside(u32),
}

/// Row numbers for the commits the scan meets, in the order it meets them.
struct Rows<'a> {
    graph: &'a Graph,
    nodes: Vec<Node>,
    /// How many rows met so far have the row as a parent.
    degrees: Vec<u32>,
    /// Indexed by position in the graph; `NONE` until the commit is met.
    by_position: Vec<u32>,
    /// The IDs of commits outside the graph, and their rows.
    outside: Vec<gix::ObjectId>,
    by_id: FxHashMap<gix::ObjectId, u32>,
}

impl Rows<'_> {
    fn push(&mut self, node: Node) -> gix::Result<u32> {
        let row = u32::try_from(self.nodes.len())
            .ok()
            .filter(|&row| row < NONE)
            .ok_or_else(too_large)?;
        self.nodes.push(node);
        self.degrees.push(0);
        Ok(row)
    }

    fn of_position(&mut self, position: u32) -> gix::Result<u32> {
        let known = self.by_position[position as usize];
        if known != NONE {
            return Ok(known);
        }
        let row = self.push(Node::Graph(position))?;
        self.by_position[position as usize] = row;
        Ok(row)
    }

    fn of_id(&mut self, id: gix::ObjectId) -> gix::Result<u32> {
        if let Some(position) = self.graph.lookup(id) {
            return self.of_position(position.0);
        }
        if let Some(&row) = self.by_id.get(&id) {
            return Ok(row);
        }
        let outside = u32::try_from(self.outside.len()).map_err(|_| too_large())?;
        let row = self.push(Node::Outside(outside))?;
        self.outside.push(id);
        self.by_id.insert(id, row);
        Ok(row)
    }

    /// `parent`, counted as the parent of one more row.
    fn child_of(&mut self, parent: u32) -> u32 {
        self.degrees[parent as usize] += 1;
        parent
    }
}

pub(super) fn scan(
    repo: &gix::ThreadSafeRepository,
    graph: &Graph,
    tips: &[gix::ObjectId],
    cancelled: &dyn Fn() -> bool,
) -> gix::Result<Scanned> {
    let bulk = bulk_after(graph.num_commits() as usize);
    scan_in_chunks(repo, graph, tips, cancelled, CHUNK, bulk)
}

/// [`scan`], sharing the graph out `chunk` commits at a time once `bulk`
/// commits were followed record by record.
pub(super) fn scan_in_chunks(
    repo: &gix::ThreadSafeRepository,
    graph: &Graph,
    tips: &[gix::ObjectId],
    cancelled: &dyn Fn() -> bool,
    chunk: usize,
    bulk: usize,
) -> gix::Result<Scanned> {
    let mut headers = None;
    let mut rows = Rows {
        graph,
        nodes: Vec::new(),
        degrees: Vec::new(),
        by_position: vec![NONE; graph.num_commits() as usize],
        outside: Vec::new(),
        by_id: FxHashMap::default(),
    };
    let mut seen = FxHashSet::default();
    let mut tip_rows = Vec::new();
    for &tip in tips {
        let row = rows.of_id(tip)?;
        if seen.insert(row) {
            tip_rows.push(row);
        }
    }
    drop(seen);

    let objects = log_paged_walk_handle(repo);
    let mut buffer = Vec::new();
    let mut offsets = vec![0];
    let mut parents = Vec::new();
    let mut times = Vec::new();
    let mut row = 0;
    while row < rows.nodes.len() {
        if row % chunk == 0 {
            check_cancelled(cancelled())?;
        }
        if row == bulk {
            headers = Some(Headers::read(graph, chunk, cancelled)?);
        }
        let time = match (rows.nodes[row], &headers) {
            (Node::Graph(position), Some(headers)) => {
                for parent in headers.parents(position) {
                    let parent = rows.of_position(parent)?;
                    parents.push(rows.child_of(parent));
                }
                headers.headers[position as usize].time
            }
            (Node::Graph(position), None) => {
                let commit = graph.commit_at(Position(position));
                for parent in commit.iter_parents() {
                    let parent = rows.of_position(parent.map_err(corrupt_edge)?.0)?;
                    parents.push(rows.child_of(parent));
                }
                commit.committer_timestamp() as i64
            }
            (Node::Outside(outside), _) => {
                check_cancelled(cancelled())?;
                let id = rows.outside[outside as usize];
                read_header(&objects, &id, &mut buffer, |parent| {
                    let parent = rows.of_id(parent)?;
                    parents.push(rows.child_of(parent));
                    Ok(())
                })?
            }
        };
        times.push(time);
        offsets.push(u32::try_from(parents.len()).map_err(|_| too_large())?);
        row += 1;
    }
    drop(objects);
    drop(headers);
    let Rows {
        nodes,
        degrees,
        outside,
        ..
    } = rows;

    // A row's ID is wherever its ID sorts in the graph: one read at random
    // each, shared out.
    let hash_len = graph.object_hash().len_in_bytes();
    let mut ids = vec![0u8; nodes.len() * hash_len];
    let chunks = Mutex::new(ids.chunks_mut(chunk * hash_len).zip(nodes.chunks(chunk)));
    on_decode_threads(nodes.len().div_ceil(chunk), cancelled, |stopped| {
        loop {
            if stopped() {
                return;
            }
            let next = chunks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .next();
            let Some((ids, nodes)) = next else {
                return;
            };
            for (id, node) in ids.chunks_exact_mut(hash_len).zip(nodes) {
                id.copy_from_slice(match *node {
                    Node::Graph(position) => graph.id_at(Position(position)).as_bytes(),
                    Node::Outside(at) => outside[at as usize].as_bytes(),
                });
            }
        }
    })?;
    Ok(Scanned {
        ids,
        offsets,
        parents,
        times,
        degrees,
        tip_rows,
    })
}
