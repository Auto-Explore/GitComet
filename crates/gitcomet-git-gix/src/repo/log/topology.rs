//! Date-order topology for repositories without a commit-graph. Read each
//! reachable header once, then sort compact row numbers instead of repeatedly
//! decoding objects during the explore, in-degree and output walks.
use super::read_ahead::PackedCommits;
use super::*;
use gitcomet_core::history_perf::{Work, record};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

mod graph_scan;

/// How long a waiter sleeps between checks of its own cancellation.
const WAIT: std::time::Duration = std::time::Duration::from_millis(20);

#[derive(Default)]
pub(crate) struct TopologyCache {
    state: Mutex<CacheState>,
    /// Signalled whenever a build ends.
    changed: Condvar,
}

#[derive(Default)]
struct CacheState {
    cached: Option<CachedTopology>,
    /// At most one build per store, bootstrap pages included.
    build: Option<Arc<Flight>>,
}

struct CachedTopology {
    generation: u64,
    tips: Vec<gix::ObjectId>,
    topology: Arc<Topology>,
}

/// A build in progress, shared by every request for the same tips.
struct Flight {
    generation: u64,
    tips: Vec<gix::ObjectId>,
    /// Requests that still want the build; it is cancelled when none do.
    subscribers: AtomicUsize,
    /// Set when the cache is cleared mid-build: the build still serves its
    /// subscribers, but its topology is not cached.
    superseded: AtomicBool,
    /// How the build ended, for the subscribers that waited on it.
    outcome: OnceLock<Outcome>,
}

type Outcome = std::result::Result<Arc<Topology>, Arc<gix::Error>>;

/// A build's error, given to each of its subscribers. gix classifies it by
/// its source, so a missing object still reads as one.
#[derive(Debug)]
struct SharedError(Arc<gix::Error>);

impl std::fmt::Display for SharedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SharedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.0)
    }
}

fn shared_error(error: Arc<gix::Error>) -> gix::Error {
    gix::Error::from_error(SharedError(error))
}

impl Flight {
    fn is_cancelled(&self) -> bool {
        self.subscribers.load(Ordering::Acquire) == 0
    }

    /// `None` once cancelled: an abandoned build never gains a subscriber.
    fn subscribe(self: &Arc<Self>) -> Option<Subscription> {
        self.subscribers
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count != 0).then_some(count + 1)
            })
            .ok()
            .map(|_| Subscription(Arc::clone(self)))
    }
}

/// A topology and the tips it was built from.
type Previous = (Arc<Topology>, Vec<gix::ObjectId>);

struct Subscription(Arc<Flight>);

impl Drop for Subscription {
    fn drop(&mut self) {
        self.0.subscribers.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Ends its flight and wakes the waiters, also when the build fails or panics.
struct Builder<'a> {
    cache: &'a TopologyCache,
    flight: Arc<Flight>,
    /// `None` until the build ends, and after a panic.
    outcome: Option<Outcome>,
    /// The topology cached for other tips of the same store, and those tips.
    previous: Option<Previous>,
}

impl Drop for Builder<'_> {
    fn drop(&mut self) {
        let mut state = self
            .cache
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.build = None;
        if let Some(outcome) = self.outcome.take() {
            // A failed or abandoned build cannot replace the last complete
            // snapshot. Only one snapshot is retained; active walks pin their
            // own immutable table across ref changes.
            if let Ok(topology) = &outcome
                && !self.flight.is_cancelled()
                && !self.flight.superseded.load(Ordering::Acquire)
            {
                state.cached = Some(CachedTopology {
                    generation: self.flight.generation,
                    tips: self.flight.tips.clone(),
                    topology: Arc::clone(topology),
                });
            }
            let _ = self.flight.outcome.set(outcome);
        }
        drop(state);
        self.cache.changed.notify_all();
    }
}

impl TopologyCache {
    /// Forgets the cached topology. A build in progress finishes for its
    /// subscribers without waiting here, and caches nothing.
    pub(in super::super) fn clear(&self) {
        let mut state = self.state.lock().expect("log topology cache");
        if let Some(flight) = &state.build {
            flight.superseded.store(true, Ordering::Release);
        }
        state.cached = None;
    }

    /// The cached topology, or the claim to build it. A request whose tips
    /// are being built subscribes to that build; any other waits its turn.
    /// Both leave as soon as their own request is cancelled.
    fn join(
        &self,
        tips: &[gix::ObjectId],
        cancellation: &LogWalkCancellation,
        generation: u64,
    ) -> gix::Result<std::result::Result<Arc<Topology>, Builder<'_>>> {
        let mut state = self.state.lock().expect("log topology cache");
        let mut joined: Option<Subscription> = None;
        loop {
            check_cancelled(cancellation.is_cancelled())?;
            if let Some(cached) = state
                .cached
                .as_ref()
                .filter(|cached| cached.generation == generation && cached.tips == tips)
            {
                return Ok(Ok(Arc::clone(&cached.topology)));
            }
            if let Some(outcome) = joined.as_ref().and_then(|joined| joined.0.outcome.get()) {
                return outcome.clone().map(Ok).map_err(shared_error);
            }
            let Some(flight) = &state.build else {
                let flight = Arc::new(Flight {
                    generation,
                    tips: tips.to_vec(),
                    subscribers: AtomicUsize::new(1),
                    superseded: AtomicBool::new(false),
                    outcome: OnceLock::new(),
                });
                state.build = Some(Arc::clone(&flight));
                let previous = state
                    .cached
                    .as_ref()
                    .filter(|cached| cached.generation == generation)
                    .map(|cached| (Arc::clone(&cached.topology), cached.tips.clone()));
                return Ok(Err(Builder {
                    cache: self,
                    flight,
                    outcome: None,
                    previous,
                }));
            };
            if joined
                .as_ref()
                .is_none_or(|joined| !Arc::ptr_eq(&joined.0, flight))
            {
                joined = (flight.generation == generation && flight.tips == tips)
                    .then(|| flight.subscribe())
                    .flatten();
            }
            state = self
                .changed
                .wait_timeout(state, WAIT)
                .expect("log topology cache")
                .0;
        }
    }

    /// One build at a time, bootstrap pages included, shared by the requests
    /// for its tips. It stops only when every subscriber has left.
    fn get_or_build(
        &self,
        tips: &[gix::ObjectId],
        cancellation: &LogWalkCancellation,
        generation: u64,
        build: impl FnOnce(&dyn Fn() -> bool, Option<&Previous>) -> gix::Result<Topology>,
    ) -> gix::Result<Arc<Topology>> {
        let mut builder = match self.join(tips, cancellation, generation)? {
            Ok(topology) => return Ok(topology),
            Err(builder) => builder,
        };
        let flight = Arc::clone(&builder.flight);
        // The flight's first subscriber is this request, until it is cancelled.
        let lease = std::cell::Cell::new(Some(Subscription(Arc::clone(&flight))));
        let cancelled = || {
            if cancellation.is_cancelled() {
                lease.take();
            }
            flight.is_cancelled()
        };
        let result = build(&cancelled, builder.previous.as_ref())
            .map(Arc::new)
            .map_err(Arc::new);
        cancelled();
        builder.outcome = Some(result.clone());
        drop(builder);
        check_cancelled(cancellation.is_cancelled())?;
        result.map_err(shared_error)
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
    ) -> gix::Result<Self> {
        check_cancelled(cancellation.is_cancelled())?;
        let topology = match cache {
            Some((cache, generation)) => {
                cache.get_or_build(tips, cancellation, generation, |cancelled, previous| {
                    if let Some((older, older_tips)) = previous
                        && let Some(grown) = older.grown(repo, older_tips, tips, cancelled)
                    {
                        return Ok(grown);
                    }
                    Topology::build(repo, tips, cancelled)
                })?
            }
            None => Arc::new(Topology::build(repo, tips, &|| {
                cancellation.is_cancelled()
            })?),
        };
        Ok(Self { topology, next: 0 })
    }
}

/// A whole walk as the tables of a history index: see
/// `HistoryIndex::from_rows`.
pub(super) struct IndexTables {
    pub(super) ids: Vec<u8>,
    pub(super) parent_offsets: Vec<u32>,
    pub(super) parents: Vec<u32>,
    /// Rows with the parents of a stash: a second parent whose only parent
    /// is the first, and a third, if any, without parents.
    pub(super) stash_shaped: Vec<u32>,
}

impl TopologyWalk {
    /// Every commit of the walk in its order, each parent as its place in
    /// that order. The topology holds parents as rows already, so nothing is
    /// looked up by ID, which an index built from the walk's IDs has to do
    /// for every parent.
    pub(super) fn index_tables(&self, cancelled: &dyn Fn() -> bool) -> gix::Result<IndexTables> {
        let topology = &*self.topology;
        let parents_of = |row: u32| {
            &topology.parents[topology.offsets[row as usize] as usize
                ..topology.offsets[row as usize + 1] as usize]
        };
        let mut place = vec![0u32; topology.order.len()];
        for (at, &row) in topology.order.iter().enumerate() {
            place[row as usize] = at as u32;
        }
        check_cancelled(cancelled())?;
        let mut tables = IndexTables {
            ids: Vec::with_capacity(topology.ids.len()),
            parent_offsets: Vec::with_capacity(topology.offsets.len()),
            parents: Vec::with_capacity(topology.parents.len()),
            stash_shaped: Vec::new(),
        };
        tables.parent_offsets.push(0);
        for (at, &row) in topology.order.iter().enumerate() {
            if at % 65_536 == 0 {
                check_cancelled(cancelled())?;
            }
            let parents = parents_of(row);
            let stash_shaped = match *parents {
                [base, index] => parents_of(index) == [base],
                [base, index, untracked] => {
                    parents_of(index) == [base] && parents_of(untracked).is_empty()
                }
                _ => false,
            };
            if stash_shaped {
                tables.stash_shaped.push(at as u32);
            }
            tables.ids.extend_from_slice(topology.id_bytes(row));
            tables
                .parents
                .extend(parents.iter().map(|&parent| place[parent as usize]));
            tables.parent_offsets.push(tables.parents.len() as u32);
        }
        Ok(tables)
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
    /// Rows by ID, made when a lookup first needs it: see [`Topology::grown`].
    sorted: std::sync::OnceLock<Box<[u32]>>,
}

impl Topology {
    fn id(&self, row: usize) -> gix::ObjectId {
        gix::ObjectId::from_bytes_or_panic(self.id_bytes(row as u32))
    }

    fn id_bytes(&self, row: u32) -> &[u8] {
        &self.ids[row as usize * self.hash_len..][..self.hash_len]
    }

    fn sorted(&self) -> &[u32] {
        self.sorted.get_or_init(|| {
            // Sort on a contiguous prefix; the ID table settles equal ones.
            let mut keyed: Vec<(u64, u32)> = (0..self.times.len() as u32)
                .map(|row| {
                    let prefix = self.id_bytes(row)[..8].try_into().expect("an ID's prefix");
                    (u64::from_be_bytes(prefix), row)
                })
                .collect();
            keyed.sort_unstable_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| self.id_bytes(a.1).cmp(self.id_bytes(b.1)))
            });
            keyed.into_iter().map(|(_, row)| row).collect()
        })
    }

    fn row_of(&self, id: &gix::oid) -> Option<u32> {
        let sorted = self.sorted();
        sorted
            .binary_search_by(|&row| self.id_bytes(row).cmp(id.as_bytes()))
            .ok()
            .map(|at| sorted[at])
    }

    /// The topology for `tips`, made from this one, built for `older_tips`,
    /// and the commits `tips` add: see [`super::extend`]. `None` when a
    /// rebuild could differ. Rows keep their numbers; added rows follow them.
    fn grown(
        &self,
        repo: &gix::ThreadSafeRepository,
        older_tips: &[gix::ObjectId],
        tips: &[gix::ObjectId],
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Self> {
        let floor = self.times[*self.order.first()? as usize];
        let row_of = |id: &gix::oid| self.row_of(id);
        let added = super::extend::Added::find(
            &log_paged_walk_handle(repo),
            tips,
            &row_of,
            Work::LogWalkObjectRead,
            cancelled,
        )
        .ok()??;
        if !added.keeps(older_tips, &row_of, self.parents.iter().copied()) {
            return None;
        }
        let added_order = added.date_order(floor)?;
        let rows = self.times.len();
        let first = u32::try_from(rows).ok()?;
        first.checked_add(u32::try_from(added.ids.len()).ok()?)?;

        let mut ids = Vec::with_capacity(self.ids.len() + added.ids.len() * self.hash_len);
        ids.extend_from_slice(&self.ids);
        let mut offsets = self.offsets.to_vec();
        let mut parents = self.parents.to_vec();
        let mut times = self.times.to_vec();
        for (row, id) in added.ids.iter().enumerate() {
            ids.extend_from_slice(id.as_bytes());
            for parent in &added.parents[row] {
                parents.push(match added.rows.get(parent) {
                    Some(&added_row) => first + added_row,
                    None => self.row_of(parent)?,
                });
            }
            offsets.push(u32::try_from(parents.len()).ok()?);
            times.push(added.times[row]);
        }
        let mut order = Vec::with_capacity(rows + added_order.len());
        order.extend(added_order.iter().map(|row| first + row));
        order.extend_from_slice(&self.order);

        // By ID: the added rows merged into the older rows.
        let mut added_sorted: Vec<u32> = (0..added.ids.len() as u32).collect();
        added_sorted.sort_unstable_by_key(|&row| added.ids[row as usize]);
        let mut sorted = Vec::with_capacity(order.len());
        let mut pending = added_sorted.into_iter().peekable();
        for &row in self.sorted() {
            while let Some(next) =
                pending.next_if(|&next| added.ids[next as usize].as_bytes() < self.id_bytes(row))
            {
                sorted.push(first + next);
            }
            sorted.push(row);
        }
        sorted.extend(pending.map(|row| first + row));

        record(Work::LogTopologyExtend);
        Some(Self {
            hash_len: self.hash_len,
            ids: ids.into_boxed_slice(),
            offsets: offsets.into_boxed_slice(),
            parents: parents.into_boxed_slice(),
            times: times.into_boxed_slice(),
            order: order.into_boxed_slice(),
            sorted: sorted.into_boxed_slice().into(),
        })
    }

    fn build(
        repo: &gix::ThreadSafeRepository,
        tips: &[gix::ObjectId],
        cancelled: &dyn Fn() -> bool,
    ) -> gix::Result<Self> {
        record(Work::LogTopologyBuild);
        let hash_len = repo.objects.object_hash().len_in_bytes();
        let graph = repo
            .to_thread_local()
            .commit_graph_if_enabled()
            .ok()
            .flatten();
        let scanned = match graph
            .as_ref()
            .map(|graph| graph_scan::scan(repo, graph, tips, cancelled))
        {
            Some(Ok(scanned)) => scanned,
            // A damaged graph is read past, as gix's own walk reads past it.
            Some(Err(_)) if !cancelled() => scan_objects(repo, tips, cancelled)?,
            Some(Err(error)) => return Err(error),
            None => scan_objects(repo, tips, cancelled)?,
        };
        // Nothing of the scan belongs in a parked page cursor or a cached
        // topology snapshot.
        drop(graph);
        Self::ordered(hash_len, scanned, cancelled)
    }

    /// Puts scanned rows in date order, children before parents.
    fn ordered(
        hash_len: usize,
        scanned: Scanned,
        cancelled: &dyn Fn() -> bool,
    ) -> gix::Result<Self> {
        let Scanned {
            ids,
            offsets,
            parents,
            times,
            mut degrees,
            tip_rows,
        } = scanned;
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
            check_cancelled(cancelled())?;
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
            return Err(gix::error::corruption("History topology contains a cycle").into());
        }
        Ok(Self {
            hash_len,
            ids: ids.into_boxed_slice(),
            offsets: offsets.into_boxed_slice(),
            parents: parents.into_boxed_slice(),
            times: times.into_boxed_slice(),
            order: order.into_boxed_slice(),
            sorted: std::sync::OnceLock::new(),
        })
    }
}

/// Every commit reachable from the tips as a row, with its parents as rows
/// and the number of rows naming it as a parent: what the ordering takes.
struct Scanned {
    ids: Vec<u8>,
    offsets: Vec<u32>,
    parents: Vec<u32>,
    times: Vec<i64>,
    degrees: Vec<u32>,
    /// The tips' rows in the tips' order, each once.
    tip_rows: Vec<u32>,
}

/// Scans without a commit-graph: each header is read from its object, or
/// from the packs' commits read ahead on several threads.
fn scan_objects(
    repo: &gix::ThreadSafeRepository,
    tips: &[gix::ObjectId],
    cancelled: &dyn Fn() -> bool,
) -> gix::Result<Scanned> {
    let hash_len = repo.objects.object_hash().len_in_bytes();
    let packed = PackedCommits::read(repo, cancelled);
    let table = match &packed {
        Some(packed) => Table::Packed(packed),
        None => Table::None,
    };
    let mut rows = Rows::new(table);
    let mut tip_rows = Vec::new();
    for &tip in tips {
        let known = rows.len();
        let row = rows.of_id(tip)?;
        if row as usize == known {
            tip_rows.push(row);
        }
    }
    let objects = log_paged_walk_handle(repo);
    let mut buffer = Vec::new();
    let mut offsets = vec![0];
    let mut parents = Vec::new();
    let mut times = Vec::new();
    let mut row = 0;
    while row < rows.len() {
        check_cancelled(cancelled())?;
        let mut time = 0;
        // A table holds the header: no object to find or inflate.
        let held = match (table, rows.position(row)) {
            (Table::Packed(packed), Some(position)) => match packed.commit(position) {
                Some((at, held)) => {
                    time = at;
                    for parent in held {
                        parents.push(rows.child_of_position(parent)?);
                    }
                    true
                }
                None => false,
            },
            _ => false,
        };
        if !held {
            let id =
                gix::ObjectId::from_bytes_or_panic(&rows.ids[row * hash_len..(row + 1) * hash_len]);
            time = read_header(&objects, &id, &mut buffer, |parent| {
                parents.push(rows.child_of_id(parent)?);
                Ok(())
            })?;
        }
        times.push(time);
        offsets.push(u32::try_from(parents.len()).map_err(|_| too_large())?);
        row += 1;
    }
    let Rows { ids, degrees, .. } = rows;
    Ok(Scanned {
        ids,
        offsets,
        parents,
        times,
        degrees,
        tip_rows,
    })
}

/// Reads a commit's parents and committer time from its object.
fn read_header(
    objects: &impl gix::objs::Find,
    id: &gix::oid,
    buffer: &mut Vec<u8>,
    mut parent: impl FnMut(gix::ObjectId) -> gix::Result<()>,
) -> gix::Result<i64> {
    use gix::objs::FindExt as _;
    record(Work::LogWalkObjectRead);
    for token in objects.find_commit_iter(id, buffer)? {
        use gix::objs::commit::ref_iter::Token;
        match token? {
            Token::Parent { id } => parent(id)?,
            Token::Committer { signature } => return Ok(signature.seconds()),
            Token::Tree { .. } | Token::Author { .. } => {}
            _ => break,
        }
    }
    Ok(0)
}

/// Commit headers held in a table and addressed by their position in it,
/// which makes finding a commit's row an array read.
#[derive(Clone, Copy)]
enum Table<'a> {
    None,
    Packed(&'a PackedCommits),
}

impl<'a> Table<'a> {
    fn len(self) -> usize {
        match self {
            Self::None => 0,
            Self::Packed(packed) => packed.len(),
        }
    }

    fn lookup(self, id: &gix::oid) -> Option<u32> {
        match self {
            Self::None => None,
            Self::Packed(packed) => packed.lookup(id),
        }
    }

    fn id_at(self, position: u32) -> &'a gix::oid {
        match self {
            Self::None => unreachable!("no table gave a position"),
            Self::Packed(packed) => packed.id_at(position),
        }
    }
}

/// Row numbers for the commits a scan meets, in the order it meets them: by
/// table position where a table holds the commit, by ID otherwise.
struct Rows<'a> {
    table: Table<'a>,
    /// Indexed by table position; `u32::MAX` until the commit is met.
    by_position: Vec<u32>,
    by_id: FxHashMap<gix::ObjectId, u32>,
    ids: Vec<u8>,
    /// Each row's table position, or `u32::MAX` outside the table.
    positions: Vec<u32>,
    /// How many rows met so far have the row as a parent.
    degrees: Vec<u32>,
}

impl<'a> Rows<'a> {
    fn new(table: Table<'a>) -> Self {
        Self {
            table,
            by_position: vec![u32::MAX; table.len()],
            by_id: FxHashMap::default(),
            ids: Vec::new(),
            positions: Vec::new(),
            degrees: Vec::new(),
        }
    }

    fn len(&self) -> usize {
        self.degrees.len()
    }

    fn position(&self, row: usize) -> Option<u32> {
        Some(self.positions[row]).filter(|&position| position != u32::MAX)
    }

    fn push(&mut self, id: &gix::oid, position: u32) -> gix::Result<u32> {
        let row = u32::try_from(self.len()).map_err(|_| too_large())?;
        self.ids.extend_from_slice(id.as_bytes());
        self.positions.push(position);
        self.degrees.push(0);
        Ok(row)
    }

    fn of_position(&mut self, position: u32) -> gix::Result<u32> {
        let known = self.by_position[position as usize];
        if known != u32::MAX {
            return Ok(known);
        }
        let row = self.push(self.table.id_at(position), position)?;
        self.by_position[position as usize] = row;
        Ok(row)
    }

    fn of_id(&mut self, id: gix::ObjectId) -> gix::Result<u32> {
        if let Some(position) = self.table.lookup(&id) {
            return self.of_position(position);
        }
        if let Some(&row) = self.by_id.get(&id) {
            return Ok(row);
        }
        let row = self.push(&id, u32::MAX)?;
        self.by_id.insert(id, row);
        Ok(row)
    }

    fn child_of(&mut self, parent: u32) -> gix::Result<u32> {
        let degree = &mut self.degrees[parent as usize];
        *degree = degree.checked_add(1).ok_or_else(too_large)?;
        Ok(parent)
    }

    /// The row of a parent named by table position, counted as one.
    fn child_of_position(&mut self, parent: u32) -> gix::Result<u32> {
        let parent = self.of_position(parent)?;
        self.child_of(parent)
    }

    /// The row of a parent named by ID, counted as one.
    fn child_of_id(&mut self, parent: gix::ObjectId) -> gix::Result<u32> {
        let parent = self.of_id(parent)?;
        self.child_of(parent)
    }
}

fn too_large() -> gix::Error {
    gix::error::corruption("History topology exceeds the supported row or edge count").into()
}

fn check_cancelled(cancelled: bool) -> gix::Result<()> {
    if cancelled {
        return Err(gix::Error::from_error(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "log walk cancelled",
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::services::CancellationToken;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::mpsc;
    use std::time::Duration;

    type Build =
        Box<dyn FnOnce(&dyn Fn() -> bool, Option<&Previous>) -> gix::Result<Topology> + Send>;

    /// Adds commits `first..=last` to a repository through `git fast-import`,
    /// each on some of the dozen before it: merges of up to five parents,
    /// with times that repeat and run backwards.
    fn import_commits(root: &std::path::Path, seed: u64, first: u64, last: u64) {
        let mut stream = String::new();
        for commit in first..=last {
            let mut state = seed ^ commit.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut below = |limit: u64| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) % limit
            };
            let time = 1_700_000_000 + (commit / 4) * 10 - below(3) * 25;
            stream.push_str(&format!(
                "commit refs/heads/tip{}\nmark :{commit}\n\
                 committer T <t@example.invalid> {time} +0000\ndata 2\nc\n",
                commit % 7
            ));
            let reach = (commit - 1).min(12);
            let mut parents = Vec::new();
            let wanted = match below(20) {
                _ if commit == 1 => 0,
                0..=2 => 3 + below(3),
                3..=6 => 2,
                _ => 1,
            };
            while (parents.len() as u64) < wanted.min(reach) {
                let parent = commit - 1 - below(reach);
                if !parents.contains(&parent) {
                    parents.push(parent);
                }
            }
            for (at, parent) in parents.iter().enumerate() {
                let edge = if at == 0 { "from" } else { "merge" };
                stream.push_str(&format!("{edge} :{parent}\n"));
            }
            stream.push('\n');
        }
        let marks = root.join(".git/marks");
        let import = format!("--import-marks-if-exists={}", marks.display());
        let export = format!("--export-marks={}", marks.display());
        crate::repo::log::tests::git_with_stdin(
            root,
            &["fast-import", "--quiet", "--force", &import, &export],
            stream.as_bytes(),
        );
    }

    /// The commits of a topology in its order.
    fn walked(topology: Topology) -> Vec<(gix::ObjectId, Vec<gix::ObjectId>, Option<i64>)> {
        let walk = TopologyWalk {
            topology: Arc::new(topology),
            next: 0,
        };
        walk.map(|info| (info.id, info.parent_ids.to_vec(), info.commit_time))
            .collect()
    }

    /// The graph's commits are read in chunks on several threads and followed
    /// from compact records; the order must be the one the objects give.
    #[test]
    fn a_commit_graph_scan_orders_a_history_as_the_object_scan_does() {
        use crate::test_support::{git_stdout, git_success, init_test_repo};
        for seed in [1, 2, 3] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            init_test_repo(root);
            // A graph in two files, and commits newer than both.
            import_commits(root, seed, 1, 260);
            git_success(
                root,
                &["commit-graph", "write", "--reachable", "--split=no-merge"],
            );
            import_commits(root, seed, 261, 460);
            git_success(
                root,
                &["commit-graph", "write", "--reachable", "--split=no-merge"],
            );
            import_commits(root, seed, 461, 500);
            let graphs = std::fs::read_dir(root.join(".git/objects/info/commit-graphs"))
                .unwrap()
                .filter(|entry| {
                    let name = entry.as_ref().unwrap().file_name();
                    name.to_string_lossy().ends_with(".graph")
                })
                .count();
            assert_eq!(graphs, 2);

            // Every branch, one of them twice, and an ancestor of one.
            let mut tips: Vec<gix::ObjectId> = git_stdout(
                root,
                &["for-each-ref", "--format=%(objectname)", "refs/heads"],
            )
            .lines()
            .map(|id| gix::ObjectId::from_hex(id.as_bytes()).unwrap())
            .collect();
            tips.push(tips[2]);
            let ancestor = git_stdout(root, &["rev-parse", "refs/heads/tip3~3"]);
            tips.push(gix::ObjectId::from_hex(ancestor.trim().as_bytes()).unwrap());

            let store = gix::open(root).unwrap().into_sync();
            let graph = store
                .to_thread_local()
                .commit_graph_if_enabled()
                .unwrap()
                .unwrap();
            let never = || false;
            let capture = gitcomet_core::history_perf::capture();
            let from_graph = graph_scan::scan_in_chunks(&store, &graph, &tips, &never, 64).unwrap();
            let read = gitcomet_core::history_perf::count(Work::LogWalkObjectRead);
            drop(capture);
            let from_graph = walked(Topology::ordered(20, from_graph, &never).unwrap());
            let from_objects = scan_objects(&store, &tips, &never).unwrap();
            let from_objects = walked(Topology::ordered(20, from_objects, &never).unwrap());
            assert_eq!(from_graph, from_objects, "seed {seed}");

            let reachable = git_stdout(root, &["rev-list", "--count", "--branches"]);
            assert_eq!(
                from_graph.len().to_string(),
                reachable.trim(),
                "seed {seed}"
            );
            let many = from_graph
                .iter()
                .filter(|(_, parents, _)| parents.len() > 2)
                .count();
            assert!(
                many > 5,
                "seed {seed}: {many} commits of three parents or more"
            );
            // Only the commits newer than the graph are read as objects.
            assert!((1..=40).contains(&read), "seed {seed}: {read} objects read");
        }
    }

    fn tips() -> Vec<gix::ObjectId> {
        vec![gix::ObjectId::null(gix::hash::Kind::Sha1)]
    }

    /// Counts itself, then polls for cancellation as the row loop does until
    /// `gate` opens.
    fn build(
        builds: &Arc<AtomicUsize>,
        started: Option<mpsc::Sender<()>>,
        gate: Option<mpsc::Receiver<()>>,
    ) -> Build {
        let builds = builds.clone();
        Box::new(move |cancelled, _| {
            builds.fetch_add(1, SeqCst);
            if let Some(started) = started {
                started.send(()).unwrap();
            }
            while let Some(gate) = &gate {
                check_cancelled(cancelled())?;
                if gate.recv_timeout(Duration::from_millis(1))
                    != Err(mpsc::RecvTimeoutError::Timeout)
                {
                    break;
                }
            }
            Ok(Topology {
                hash_len: 20,
                ids: [].into(),
                offsets: [0].into(),
                parents: [].into(),
                times: [].into(),
                order: [].into(),
                sorted: Default::default(),
            })
        })
    }

    fn wait_for_subscribers(cache: &TopologyCache, count: usize) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !cache
            .state
            .lock()
            .unwrap()
            .build
            .as_ref()
            .is_some_and(|flight| flight.subscribers.load(SeqCst) == count)
        {
            assert!(std::time::Instant::now() < deadline, "no subscription");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    struct Shared {
        builds: Arc<AtomicUsize>,
        cache: Arc<TopologyCache>,
        tokens: [CancellationToken; 2],
        release: mpsc::Sender<()>,
        builder: std::thread::JoinHandle<gix::Result<Arc<Topology>>>,
        waiter: mpsc::Receiver<gix::Result<Arc<Topology>>>,
    }

    /// `build`, ending as a commit gone missing would end it.
    fn failing(build: Build) -> Build {
        Box::new(move |cancelled, previous| {
            build(cancelled, previous)?;
            let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "a commit is missing");
            Err(gix::Error::from_error(missing))
        })
    }

    /// A gated builder with one subscribed waiter for the same tips.
    fn flight() -> Shared {
        flight_with(|build| build)
    }

    /// [`flight`] with the builder's build passed through `wrap`.
    fn flight_with(wrap: fn(Build) -> Build) -> Shared {
        let builds = Arc::new(AtomicUsize::new(0));
        let cache = Arc::new(TopologyCache::default());
        let tokens = [CancellationToken::new(), CancellationToken::new()];
        let (started_tx, started) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (done_tx, waiter) = mpsc::channel();
        let request = |token: &CancellationToken, build: Build| {
            let (cache, token) = (cache.clone(), token.clone());
            move || cache.get_or_build(&tips(), &LogWalkCancellation::new(Some(&token)), 1, build)
        };
        let builder = std::thread::spawn(request(
            &tokens[0],
            wrap(build(&builds, Some(started_tx), Some(gate))),
        ));
        started.recv().unwrap();
        let wait = request(&tokens[1], build(&builds, None, None));
        std::thread::spawn(move || done_tx.send(wait()));
        wait_for_subscribers(&cache, 2);
        Shared {
            builds,
            cache,
            tokens,
            release,
            builder,
            waiter,
        }
    }

    fn is_cancelled(result: gix::Result<Arc<Topology>>) -> bool {
        result.is_err_and(|error| error.to_string().contains("cancelled"))
    }

    #[test]
    fn a_waiter_receives_the_single_build() {
        let flight = flight();
        flight.release.send(()).unwrap();
        let built = flight.builder.join().unwrap().unwrap();
        let waited = flight.waiter.recv().unwrap().unwrap();
        assert!(Arc::ptr_eq(&built, &waited));
        assert_eq!(flight.builds.load(SeqCst), 1);
    }

    /// A store invalidation clears the cache under the store's own locks, so
    /// clearing must not wait for a build; the build still serves its
    /// subscriber, and caches nothing.
    #[test]
    fn clearing_the_cache_leaves_a_build_to_its_subscribers() {
        let flight = flight();
        let (cleared_tx, cleared) = mpsc::channel();
        let cache = flight.cache.clone();
        std::thread::spawn(move || {
            cache.clear();
            cleared_tx.send(()).unwrap();
        });
        cleared
            .recv_timeout(Duration::from_secs(2))
            .expect("clearing must not wait for the build");
        flight.release.send(()).unwrap();
        let built = flight.builder.join().unwrap().unwrap();
        let waited = flight.waiter.recv().unwrap().unwrap();
        assert!(Arc::ptr_eq(&built, &waited));
        assert_eq!(flight.builds.load(SeqCst), 1);
        let later = build(&flight.builds, None, None);
        flight
            .cache
            .get_or_build(&tips(), &LogWalkCancellation::default(), 1, later)
            .unwrap();
        assert_eq!(flight.builds.load(SeqCst), 2, "nothing was cached");
    }

    /// Its subscriber gets a failed build's error, classified as gix
    /// classified it, instead of building again.
    #[test]
    fn a_failed_build_fails_its_subscriber_as_it_failed() {
        let flight = flight_with(failing);
        flight.release.send(()).unwrap();
        let not_found =
            |result: gix::Result<Arc<Topology>>| result.is_err_and(|e| e.is_not_found());
        assert!(not_found(flight.builder.join().unwrap()));
        assert!(not_found(flight.waiter.recv().unwrap()));
        assert_eq!(flight.builds.load(SeqCst), 1);
    }

    #[test]
    fn a_cancelled_waiter_leaves_while_the_build_completes() {
        let flight = flight();
        flight.tokens[1].cancel();
        let waited = flight
            .waiter
            .recv_timeout(Duration::from_secs(2))
            .expect("a cancelled waiter must not wait for the build");
        assert!(is_cancelled(waited));
        flight.release.send(()).unwrap();
        flight.builder.join().unwrap().unwrap();
        assert_eq!(flight.builds.load(SeqCst), 1);
    }

    #[test]
    fn a_cancelled_builder_finishes_for_the_remaining_subscriber() {
        let flight = flight();
        flight.tokens[0].cancel();
        // The build is still polling for cancellation here and must not see it.
        std::thread::sleep(Duration::from_millis(20));
        let _ = flight.release.send(());
        assert!(is_cancelled(flight.builder.join().unwrap()));
        flight.waiter.recv().unwrap().unwrap();
        assert_eq!(
            flight.builds.load(SeqCst),
            1,
            "the subscriber must not restart the build"
        );
    }

    #[test]
    fn a_build_stops_with_its_last_subscriber() {
        let flight = flight();
        flight.tokens[1].cancel();
        assert!(is_cancelled(flight.waiter.recv().unwrap()));
        flight.tokens[0].cancel();
        // The gate stays closed: only cancellation can end this build.
        assert!(is_cancelled(flight.builder.join().unwrap()));
        let later = build(&flight.builds, None, None);
        flight
            .cache
            .get_or_build(&tips(), &LogWalkCancellation::default(), 1, later)
            .unwrap();
        assert_eq!(
            flight.builds.load(SeqCst),
            2,
            "an abandoned build is not cached"
        );
    }
}
