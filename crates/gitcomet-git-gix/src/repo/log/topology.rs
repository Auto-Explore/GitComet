//! Date-order topology for repositories without a commit-graph. Read each
//! reachable header once, then sort compact row numbers instead of repeatedly
//! decoding objects during the explore, in-degree and output walks.
use super::*;
use gitcomet_core::history_perf::{Work, record};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

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
    built: Option<Arc<Topology>>,
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
        // A failed or abandoned build cannot replace the last complete
        // snapshot. Only one snapshot is retained; active walks pin their own
        // immutable table across ref changes.
        if let Some(topology) = self.built.take()
            && !self.flight.is_cancelled()
        {
            state.cached = Some(CachedTopology {
                generation: self.flight.generation,
                tips: self.flight.tips.clone(),
                topology,
            });
        }
        drop(state);
        self.cache.changed.notify_all();
    }
}

impl TopologyCache {
    pub(in super::super) fn clear(&self) {
        let mut state = self.state.lock().expect("log topology cache");
        // Wait out a build, which would otherwise publish after this.
        while state.build.is_some() {
            state = self.changed.wait(state).expect("log topology cache");
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
            let Some(flight) = &state.build else {
                let flight = Arc::new(Flight {
                    generation,
                    tips: tips.to_vec(),
                    subscribers: AtomicUsize::new(1),
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
                    built: None,
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
        let result = build(&cancelled, builder.previous.as_ref()).map(Arc::new);
        cancelled();
        builder.built = result.as_ref().ok().cloned();
        drop(builder);
        check_cancelled(cancellation.is_cancelled())?;
        result
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
        let mut rows = Rows::new(graph.as_ref());
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
            if let (Some(graph), Some(position)) = (rows.graph, rows.position(row)) {
                // The graph's own tables: no object to find or inflate.
                let commit = graph.commit_at(position);
                time = commit.committer_timestamp() as i64;
                for parent in commit.iter_parents() {
                    let parent = parent.map_err(|_| {
                        gix::Error::from(gix::error::corruption("Commit-graph parent edge"))
                    })?;
                    parents.push(rows.child_of_position(parent)?);
                }
            } else {
                let id = gix::ObjectId::from_bytes_or_panic(
                    &rows.ids[row * hash_len..(row + 1) * hash_len],
                );
                record(Work::LogWalkObjectRead);
                for token in objects.find_commit_iter(&id, &mut buffer)? {
                    use gix::objs::commit::ref_iter::Token;
                    match token? {
                        Token::Parent { id } => parents.push(rows.child_of_id(id)?),
                        Token::Committer { signature } => {
                            time = signature.seconds();
                            break;
                        }
                        Token::Tree { .. } | Token::Author { .. } => {}
                        _ => break,
                    }
                }
            }
            times.push(time);
            offsets.push(u32::try_from(parents.len()).map_err(|_| too_large())?);
            row += 1;
        }
        let Rows {
            ids, mut degrees, ..
        } = rows;
        // The lookup tables went with `rows`. Release the object store too
        // before sorting; neither belongs in a parked page cursor or a cached
        // topology snapshot.
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

/// Row numbers for the commits a scan meets, in the order it meets them: by
/// commit-graph position where the graph holds the commit, by ID otherwise.
struct Rows<'a> {
    graph: Option<&'a gix::commitgraph::Graph>,
    /// Indexed by commit-graph position; `u32::MAX` until the commit is met.
    by_position: Vec<u32>,
    by_id: FxHashMap<gix::ObjectId, u32>,
    ids: Vec<u8>,
    /// Each row's commit-graph position, or `u32::MAX` outside the graph.
    positions: Vec<u32>,
    /// How many rows met so far have the row as a parent.
    degrees: Vec<u32>,
}

impl<'a> Rows<'a> {
    fn new(graph: Option<&'a gix::commitgraph::Graph>) -> Self {
        Self {
            graph,
            by_position: vec![u32::MAX; graph.map_or(0, |graph| graph.num_commits() as usize)],
            by_id: FxHashMap::default(),
            ids: Vec::new(),
            positions: Vec::new(),
            degrees: Vec::new(),
        }
    }

    fn len(&self) -> usize {
        self.degrees.len()
    }

    fn position(&self, row: usize) -> Option<gix::commitgraph::Position> {
        Some(self.positions[row])
            .filter(|&position| position != u32::MAX)
            .map(gix::commitgraph::Position)
    }

    fn push(&mut self, id: &gix::oid, position: u32) -> gix::Result<u32> {
        let row = u32::try_from(self.len()).map_err(|_| too_large())?;
        self.ids.extend_from_slice(id.as_bytes());
        self.positions.push(position);
        self.degrees.push(0);
        Ok(row)
    }

    fn of_position(&mut self, position: gix::commitgraph::Position) -> gix::Result<u32> {
        let known = self.by_position[position.0 as usize];
        if known != u32::MAX {
            return Ok(known);
        }
        let graph = self.graph.expect("a position comes from the graph");
        let row = self.push(graph.id_at(position), position.0)?;
        self.by_position[position.0 as usize] = row;
        Ok(row)
    }

    fn of_id(&mut self, id: gix::ObjectId) -> gix::Result<u32> {
        if let Some(position) = self.graph.and_then(|graph| graph.lookup(id)) {
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

    /// The row of a parent named by commit-graph position, counted as one.
    fn child_of_position(&mut self, parent: gix::commitgraph::Position) -> gix::Result<u32> {
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

    /// A gated builder with one subscribed waiter for the same tips.
    fn flight() -> Shared {
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
            build(&builds, Some(started_tx), Some(gate)),
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
