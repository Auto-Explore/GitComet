//! Date-order topology for repositories without a commit-graph. Read each
//! reachable header once, then sort compact row numbers instead of repeatedly
//! decoding objects during the explore, in-degree and output walks.
use super::*;
use gitcomet_core::history_perf::{Work, record};
use gix::error::ErrorExt as _;
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct TopologyCache(Mutex<Option<CachedTopology>>);

struct CachedTopology {
    generation: u64,
    tips: Vec<gix::ObjectId>,
    topology: Arc<Topology>,
}

impl TopologyCache {
    pub(in super::super) fn clear(&self) {
        *self.0.lock().expect("log topology cache") = None;
    }

    fn get_or_build(
        &self,
        repo: &gix::ThreadSafeRepository,
        tips: &[gix::ObjectId],
        cancellation: &LogWalkCancellation,
        generation: u64,
    ) -> gix::ExnResult<Arc<Topology>> {
        // Serialize initial bootstrap builds too, before windows request their
        // full index. Waiters can leave promptly when their request is cancelled.
        let mut slot = loop {
            check_cancelled(cancellation)?;
            match self.0.try_lock() {
                Ok(slot) => break slot,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            }
        };
        if let Some(cached) = slot
            .as_ref()
            .filter(|cached| cached.generation == generation && cached.tips == tips)
        {
            return Ok(Arc::clone(&cached.topology));
        }
        // A cancelled or failed build cannot replace the last complete
        // snapshot. Only one snapshot is retained;
        // active walks pin their own immutable table across ref changes.
        let topology = Arc::new(Topology::build(repo, tips, cancellation)?);
        check_cancelled(cancellation)?;
        *slot = Some(CachedTopology {
            generation,
            tips: tips.to_vec(),
            topology: Arc::clone(&topology),
        });
        Ok(topology)
    }
}

pub(in super::super) struct TopologyWalk {
    topology: Arc<Topology>,
    next: usize,
}

impl TopologyWalk {
    pub(super) fn new(
        repo: &gix::ThreadSafeRepository,
        tips: &[gix::ObjectId],
        cancellation: &LogWalkCancellation,
        cache: Option<(&TopologyCache, u64)>,
    ) -> gix::ExnResult<Self> {
        check_cancelled(cancellation)?;
        let topology = match cache {
            Some((cache, generation)) => {
                cache.get_or_build(repo, tips, cancellation, generation)?
            }
            None => Arc::new(Topology::build(repo, tips, cancellation)?),
        };
        Ok(Self { topology, next: 0 })
    }
}

impl Iterator for TopologyWalk {
    type Item = gix::traverse::commit::Info;

    fn next(&mut self) -> Option<Self::Item> {
        let topology = &self.topology;
        let row = *topology.order.get(self.next)? as usize;
        self.next += 1;
        Some(gix::traverse::commit::Info {
            id: topology.id(row),
            parent_ids: topology.parents
                [topology.offsets[row] as usize..topology.offsets[row + 1] as usize]
                .iter()
                .map(|&parent| topology.id(parent as usize))
                .collect(),
            generation: None,
            commit_time: Some(topology.times[row]),
        })
    }
}

struct Topology {
    hash_len: usize,
    ids: Box<[u8]>,
    offsets: Box<[u32]>,
    parents: Box<[u32]>,
    times: Box<[i64]>,
    order: Box<[u32]>,
}

impl Topology {
    fn id(&self, row: usize) -> gix::ObjectId {
        gix::ObjectId::from_bytes_or_panic(
            &self.ids[row * self.hash_len..(row + 1) * self.hash_len],
        )
    }

    fn build(
        repo: &gix::ThreadSafeRepository,
        tips: &[gix::ObjectId],
        cancellation: &LogWalkCancellation,
    ) -> gix::ExnResult<Self> {
        record(Work::LogTopologyBuild);
        let hash_len = repo.objects.object_hash().len_in_bytes();
        let mut ids = Vec::new();
        let mut rows = FxHashMap::default();
        let mut degrees = Vec::new();
        let mut tip_rows = Vec::new();
        for &tip in tips {
            if !rows.contains_key(&tip) {
                let row = intern(tip, &mut rows, &mut ids, &mut degrees)?;
                tip_rows.push(row);
            }
        }
        let objects = log_paged_walk_handle(repo);
        let mut buffer = Vec::new();
        let mut offsets = vec![0];
        let mut parents = Vec::new();
        let mut times = Vec::new();
        let mut row = 0;
        while row < degrees.len() {
            check_cancelled(cancellation)?;
            let id = gix::ObjectId::from_bytes_or_panic(&ids[row * hash_len..(row + 1) * hash_len]);
            record(Work::LogWalkObjectRead);
            let commit = objects.find_commit_iter(&id, &mut buffer)?;
            let mut time = 0;
            for token in commit {
                use gix::objs::commit::ref_iter::Token;
                match token.map_err(|error| error.erased())? {
                    Token::Parent { id } => {
                        let parent = intern(id, &mut rows, &mut ids, &mut degrees)?;
                        degrees[parent as usize] = degrees[parent as usize]
                            .checked_add(1)
                            .ok_or_else(too_large)?;
                        parents.push(parent);
                    }
                    Token::Committer { signature } => {
                        time = signature.seconds();
                        break;
                    }
                    Token::Tree { .. } | Token::Author { .. } => {}
                    _ => break,
                }
            }
            times.push(time);
            offsets.push(u32::try_from(parents.len()).map_err(|_| too_large())?);
            row += 1;
        }
        // Release the lookup map and the object store before sorting; neither
        // belongs in a parked page cursor or a cached topology snapshot.
        drop(rows);
        drop(objects);
        drop(buffer);

        // Use the same queue and insertion order as gix's date-order walker,
        // including its handling of equal timestamps and redundant tips.
        let mut ready = gix::revwalk::PriorityQueue::new();
        for row in tip_rows {
            if degrees[row as usize] == 0 {
                ready.insert(times[row as usize], row);
            }
        }
        let mut order = Vec::with_capacity(times.len());
        while let Some(row) = ready.pop_value() {
            check_cancelled(cancellation)?;
            order.push(row);
            for &parent in
                &parents[offsets[row as usize] as usize..offsets[row as usize + 1] as usize]
            {
                degrees[parent as usize] -= 1;
                if degrees[parent as usize] == 0 {
                    ready.insert(times[parent as usize], parent);
                }
            }
        }
        if order.len() != times.len() {
            return Err(gix::error::corruption("History topology contains a cycle").raise_erased());
        }
        Ok(Self {
            hash_len,
            ids: ids.into_boxed_slice(),
            offsets: offsets.into_boxed_slice(),
            parents: parents.into_boxed_slice(),
            times: times.into_boxed_slice(),
            order: order.into_boxed_slice(),
        })
    }
}

fn intern(
    id: gix::ObjectId,
    rows: &mut FxHashMap<gix::ObjectId, u32>,
    ids: &mut Vec<u8>,
    degrees: &mut Vec<u32>,
) -> gix::ExnResult<u32> {
    match rows.entry(id) {
        std::collections::hash_map::Entry::Occupied(entry) => Ok(*entry.get()),
        std::collections::hash_map::Entry::Vacant(entry) => {
            let row = u32::try_from(degrees.len()).map_err(|_| too_large())?;
            entry.insert(row);
            ids.extend_from_slice(id.as_bytes());
            degrees.push(0);
            Ok(row)
        }
    }
}

fn too_large() -> gix::Exn {
    gix::error::corruption("History topology exceeds the supported row or edge count")
        .raise_erased()
}

fn check_cancelled(cancellation: &LogWalkCancellation) -> gix::ExnResult<()> {
    if cancellation.is_cancelled() {
        return Err(
            std::io::Error::new(std::io::ErrorKind::Interrupted, "log walk cancelled")
                .raise_erased(),
        );
    }
    Ok(())
}
