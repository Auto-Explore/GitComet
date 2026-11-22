//! Commit headers read ahead of a header scan.
//!
//! The scan learns of a commit from its child, one at a time, so a second
//! thread has nothing to read. A pack written by Git starts with its commits,
//! though: the pack index names them before any is read, and their headers
//! are read on several threads at once.

use super::walk::{DECODE_BATCH, DecodeThreadBudget, log_paged_walk_handle};
use gitcomet_core::history_perf::{Work, record_many};
use gix::odb::HeaderExt as _;
use gix::prelude::FindExt as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A pack, or a run of commits, with fewer objects is left to the scan.
const MIN_OBJECTS: usize = 8_192;
/// Index entries looked at to find where a pack's leading commits end.
const SAMPLES: usize = 2_048;
/// This many samples in a row that are not commits end the run.
const RUN_END: usize = 8;
/// The time of a candidate the scan has to read itself.
const UNREAD: i64 = i64::MIN;
const NONE: u32 = u32::MAX;

/// The objects in the packs' leading runs of commits, by ID, with the header
/// of each that is a commit whose parents are all among them.
pub(super) struct PackedCommits {
    hash_len: usize,
    /// Sorted, so a candidate's number is its place here.
    ids: Vec<u8>,
    /// The first candidate whose ID starts with each two bytes.
    fanout: Box<[u32]>,
    headers: Vec<Header>,
    /// The parents after the first of each merge, behind their count.
    merges: Vec<u32>,
}

/// One candidate's header in one place: the scan meets candidates in no
/// order, and most commits have a single parent.
#[derive(Clone, Copy)]
struct Header {
    time: i64,
    /// A candidate number, or `NONE`.
    first_parent: u32,
    /// Where the other parents are in `merges`, or `NONE`.
    merge: u32,
}

const UNREAD_HEADER: Header = Header {
    time: UNREAD,
    first_parent: NONE,
    merge: NONE,
};

/// The headers of a run of consecutive candidates.
#[derive(Default)]
struct Part {
    headers: Vec<Header>,
    merges: Vec<u32>,
    read: u64,
}

impl Part {
    fn push(&mut self, time: Option<i64>, parents: &[u32]) {
        let Some(time) = time else {
            self.headers.push(UNREAD_HEADER);
            return;
        };
        let merge = match parents {
            [] | [_] => NONE,
            [_, others @ ..] => {
                let merge = self.merges.len() as u32;
                self.merges.push(others.len() as u32);
                self.merges.extend_from_slice(others);
                merge
            }
        };
        self.headers.push(Header {
            time,
            first_parent: parents.first().copied().unwrap_or(NONE),
            merge,
        });
        self.read += 1;
    }
}

impl PackedCommits {
    /// `None` when no pack is worth the threads, or once `cancelled`.
    pub(super) fn read(
        repo: &gix::ThreadSafeRepository,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Self> {
        // One helper even when the budget is spent: alone, this thread would
        // read every packed commit where the scan reads only those the tips
        // reach.
        let budget = DecodeThreadBudget::claim();
        let helpers = budget.helpers().max(1);
        let objects = log_paged_walk_handle(repo);
        let hash_len = repo.objects.object_hash().len_in_bytes();
        let ids = candidates(repo, &objects, helpers)?;
        let count = ids.len() / hash_len;
        u32::try_from(count).ok()?;
        let mut table = Self {
            hash_len,
            fanout: fanout(&ids, hash_len),
            ids,
            headers: Vec::with_capacity(count),
            merges: Vec::new(),
        };

        let chunks = count.div_ceil(DECODE_BATCH);
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        // Only the calling thread may ask `cancelled`; it tells the others.
        let work = |objects: &gix::OdbHandleArc, cancelled: Option<&dyn Fn() -> bool>| {
            let mut parts = Vec::new();
            let mut buffer = Vec::new();
            let mut parents = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                if cancelled.is_some_and(|cancelled| cancelled()) {
                    stop.store(true, Ordering::Relaxed);
                    break;
                }
                let chunk = next.fetch_add(1, Ordering::Relaxed);
                if chunk >= chunks {
                    break;
                }
                let mut part = Part::default();
                for slot in chunk * DECODE_BATCH..((chunk + 1) * DECODE_BATCH).min(count) {
                    let time = table.read_header(objects, slot as u32, &mut buffer, &mut parents);
                    part.push(time, &parents);
                }
                parts.push((chunk, part));
            }
            parts
        };
        let mut parts = std::thread::scope(|scope| {
            let helpers: Vec<_> = (0..helpers)
                .map(|_| scope.spawn(|| work(&log_paged_walk_handle(repo), None)))
                .collect();
            let mut parts = work(&objects, Some(cancelled));
            for helper in helpers {
                // A helper that panicked leaves its candidates to the scan.
                parts.extend(helper.join().unwrap_or_default());
            }
            parts
        });
        drop(budget);
        if stop.load(Ordering::Relaxed) {
            return None;
        }

        parts.sort_unstable_by_key(|(chunk, _)| *chunk);
        let mut parts = parts.into_iter().peekable();
        let mut read = 0;
        for chunk in 0..chunks {
            let Some((_, part)) = parts.next_if(|(read, _)| *read == chunk) else {
                let slots = DECODE_BATCH.min(count - chunk * DECODE_BATCH);
                table
                    .headers
                    .extend(std::iter::repeat_n(UNREAD_HEADER, slots));
                continue;
            };
            let base = u32::try_from(table.merges.len()).ok()?;
            u32::try_from(table.merges.len() + part.merges.len()).ok()?;
            table
                .headers
                .extend(part.headers.iter().map(|header| Header {
                    merge: match header.merge {
                        NONE => NONE,
                        merge => base + merge,
                    },
                    ..*header
                }));
            table.merges.extend(part.merges);
            read += part.read;
        }
        record_many(Work::LogHeaderReadAhead, read);
        Some(table)
    }

    pub(super) fn len(&self) -> usize {
        self.headers.len()
    }

    pub(super) fn id_at(&self, slot: u32) -> &gix::oid {
        let start = slot as usize * self.hash_len;
        gix::oid::from_bytes_unchecked(&self.ids[start..start + self.hash_len])
    }

    pub(super) fn lookup(&self, id: &gix::oid) -> Option<u32> {
        let id = id.as_bytes();
        let prefix = usize::from(u16::from_be_bytes([id[0], id[1]]));
        let mut low = self.fanout[prefix] as usize;
        let mut high = self.fanout[prefix + 1] as usize;
        while low < high {
            let middle = low + (high - low) / 2;
            let start = middle * self.hash_len;
            match self.ids[start..start + self.hash_len].cmp(id) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Some(middle as u32),
            }
        }
        None
    }

    /// The committer time and parents of candidate `slot`, if it was read.
    pub(super) fn commit(&self, slot: u32) -> Option<(i64, impl Iterator<Item = u32> + '_)> {
        let header = self.headers[slot as usize];
        if header.time == UNREAD {
            return None;
        }
        let others = match header.merge {
            NONE => &[][..],
            merge => {
                let start = merge as usize + 1;
                &self.merges[start..start + self.merges[merge as usize] as usize]
            }
        };
        let first = Some(header.first_parent).filter(|&parent| parent != NONE);
        Some((header.time, first.into_iter().chain(others.iter().copied())))
    }

    /// The committer time of candidate `slot`, its parents left in `parents`
    /// as candidate numbers. `None` leaves it to the scan: an object that is
    /// no commit or cannot be read, or a parent that is no candidate.
    fn read_header(
        &self,
        objects: &gix::OdbHandleArc,
        slot: u32,
        buffer: &mut Vec<u8>,
        parents: &mut Vec<u32>,
    ) -> Option<i64> {
        parents.clear();
        let id = self.id_at(slot);
        // Asked first, so that a stray blob is never inflated.
        if !is_commit(objects, id) {
            return None;
        }
        let mut time = 0;
        for token in objects.find_commit_iter(id, buffer).ok()? {
            use gix::objs::commit::ref_iter::Token;
            match token.ok()? {
                Token::Parent { id } => parents.push(self.lookup(&id)?),
                Token::Committer { signature } => {
                    time = signature.seconds();
                    break;
                }
                Token::Tree { .. } | Token::Author { .. } => {}
                _ => break,
            }
        }
        (time != UNREAD).then_some(time)
    }
}

fn is_commit(objects: &gix::OdbHandleArc, id: &gix::oid) -> bool {
    objects
        .header(id)
        .is_ok_and(|header| header.kind() == gix::objs::Kind::Commit)
}

fn fanout(ids: &[u8], hash_len: usize) -> Box<[u32]> {
    let mut fanout = vec![0u32; (1 << 16) + 1];
    for id in ids.chunks_exact(hash_len) {
        fanout[usize::from(u16::from_be_bytes([id[0], id[1]])) + 1] += 1;
    }
    for prefix in 1..fanout.len() {
        fanout[prefix] += fanout[prefix - 1];
    }
    fanout.into()
}

/// The sorted IDs of the objects in every pack's leading run of commits.
fn candidates(
    repo: &gix::ThreadSafeRepository,
    objects: &gix::OdbHandleArc,
    helpers: usize,
) -> Option<Vec<u8>> {
    let hash = repo.objects.object_hash();
    let mut directories = vec![repo.objects.path().to_owned()];
    directories.extend(repo.objects.alternate_db_paths().unwrap_or_default());
    let mut runs = Vec::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory.join("pack")) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.extension().is_none_or(|extension| extension != "idx") {
                continue;
            }
            let Ok(index) = gix::odb::pack::index::File::at(&path, hash) else {
                continue;
            };
            runs.extend(commit_run(&index, objects, helpers));
        }
    }
    if runs.len() < 2 {
        return runs.pop();
    }
    // An object may be in two packs. The runs are sorted already, which a
    // stable sort merges rather than sorts.
    let mut ids: Vec<&[u8]> = runs
        .iter()
        .flat_map(|run| run.chunks_exact(hash.len_in_bytes()))
        .collect();
    ids.sort();
    ids.dedup();
    Some(ids.concat())
}

/// The IDs of the objects `index`'s pack holds before the end of its leading
/// run of commits, in index order; `None` for a pack that does not start with
/// one, as a pack written by `git fast-import` does not.
///
/// Where the run ends is found from a sample of the index, looked at from the
/// front of the pack and only as far as the run goes: a commit's kind is
/// cheap to ask, that of a blob at the end of a delta chain is not. A tree or
/// a blob that ends up among the candidates costs that one question.
fn commit_run(
    index: &gix::odb::pack::index::File,
    objects: &gix::OdbHandleArc,
    helpers: usize,
) -> Option<Vec<u8>> {
    let count = index.num_objects() as usize;
    if count < MIN_OBJECTS {
        return None;
    }
    let samples = SAMPLES.min(count);
    let mut sampled: Vec<(u64, u32)> = (0..samples)
        .map(|sample| {
            let entry = (sample * count / samples) as u32;
            (index.pack_offset_at_index(entry), entry)
        })
        .collect();
    sampled.sort_unstable();
    let mut end = u64::MAX;
    let (mut commits, mut strays, mut others) = (0, 0, 0);
    for (sample, &(_, entry)) in sampled.iter().enumerate() {
        if is_commit(objects, index.oid_at_index(entry)) {
            commits += 1;
            strays += others;
            others = 0;
            continue;
        }
        others += 1;
        if others == RUN_END {
            end = sampled[sample + 1 - RUN_END].0;
            break;
        }
    }
    // Clearly too few commits to be worth threads (the sample only estimates
    // their number), or commits scattered among other objects: the scan reads
    // those where it needs them.
    if commits * count / samples < MIN_OBJECTS / 2 || strays * 10 > commits {
        return None;
    }

    let take = |entries: std::ops::Range<usize>| -> Vec<u8> {
        entries
            .map(|entry| entry as u32)
            .filter(|&entry| index.pack_offset_at_index(entry) < end)
            .flat_map(|entry| index.oid_at_index(entry).as_bytes())
            .copied()
            .collect()
    };
    let part = count.div_ceil(helpers + 1);
    let ids = std::thread::scope(|scope| {
        let take = &take;
        let helpers: Vec<_> = (1..=helpers)
            .map(|helper| {
                scope.spawn(move || {
                    take((helper * part).min(count)..((helper + 1) * part).min(count))
                })
            })
            .collect();
        let mut ids = take(0..part.min(count));
        for helper in helpers {
            ids.extend(helper.join().ok()?);
        }
        Some(ids)
    })?;
    (ids.len() / index.object_hash().len_in_bytes() >= MIN_OBJECTS).then_some(ids)
}
