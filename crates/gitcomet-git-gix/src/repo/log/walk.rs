use super::*;

pub(crate) fn empty_log_page() -> LogPage {
    LogPage {
        commits: Vec::new(),
        next_cursor: None,
    }
}

pub(crate) fn object_id_from_commit_id(id: &CommitId) -> Option<gix::ObjectId> {
    gix::ObjectId::from_hex(id.as_ref().as_bytes()).ok()
}

pub(crate) fn log_paged_walk_handle(repo: &gix::ThreadSafeRepository) -> gix::OdbHandleArc {
    gix::odb::memory::Proxy::from(gix::odb::Cache::from(repo.objects.to_handle()))
        .with_write_passthrough()
}

/// The cancellation token currently associated with a resumable walk.
///
/// A walk can outlive several page requests, so its object lookup cannot retain
/// the token from the request that originally built it.
#[derive(Clone, Default)]
pub(crate) struct LogWalkCancellation(Arc<std::sync::RwLock<Option<CancellationToken>>>);

impl LogWalkCancellation {
    pub(crate) fn new(cancellation: Option<&CancellationToken>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(cancellation.cloned())))
    }

    pub(crate) fn replace(&self, cancellation: Option<&CancellationToken>) {
        *self.0.write().expect("log walk cancellation") = cancellation.cloned();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0
            .read()
            .expect("log walk cancellation")
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }
}

/// Object lookup used by the expensive date-order setup pass.
///
/// gix's topo builder has no cancellation hook of its own. On repositories
/// without a commit-graph it resolves an object for every reachable commit, so
/// checking at this boundary makes that slow path promptly cancellable without
/// changing the traversal algorithm.
pub(crate) struct CancellableLogWalkFind {
    inner: gix::OdbHandleArc,
    cancellation: LogWalkCancellation,
}

impl gix::objs::Find for CancellableLogWalkFind {
    fn try_find<'a>(
        &self,
        id: &gix::oid,
        buffer: &'a mut Vec<u8>,
    ) -> std::result::Result<Option<gix::objs::Data<'a>>, gix::objs::find::Error> {
        if self.cancellation.is_cancelled() {
            return Err(
                std::io::Error::new(std::io::ErrorKind::Interrupted, "log walk cancelled").into(),
            );
        }
        gix::objs::Find::try_find(&self.inner, id, buffer)
    }
}

/// Read and parse the shallow boundary once for a log request.
///
/// The parsed ids are both the cache fingerprint and the traversal input. This
/// deliberately bypasses gix's shared shallow snapshot, whose refresh decision
/// is mtime-only and can therefore retain old contents after a timestamp
/// collision.
pub(crate) fn shallow_snapshot(repo: &gix::Repository) -> Result<super::ShallowSnapshot> {
    let path = repo.shallow_file();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(super::ShallowSnapshot::default());
        }
        Err(error) => {
            return Err(Error::new(ErrorKind::Backend(format!(
                "read shallow boundary {}: {error}",
                path.display()
            ))));
        }
    };

    let commits = bytes
        .lines()
        .map(gix::ObjectId::from_hex)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| {
            Error::new(ErrorKind::Backend(format!(
                "parse shallow boundary {}: {error}",
                path.display()
            )))
        })?;
    Ok(super::ShallowSnapshot::from_commits(commits))
}

/// The commit filter a paged walk over `repo` needs.
///
/// On a shallow repository the boundary commits record parents that are not in
/// the object database; they have to be skipped or the traversal walks off the
/// end of what was cloned. `repo.rev_walk(..)` installs exactly this, but its
/// filter borrows the repository, and a walk parked in the walk cache outlives
/// any such borrow — hence the owned handle and the boxed closure.
pub(crate) fn log_paged_walk_filter(
    repo: &gix::ThreadSafeRepository,
    shallow: &super::ShallowSnapshot,
) -> super::LogPagedWalkFilter {
    if !shallow.is_shallow() {
        return Box::new(|_| true);
    }

    let shallow_commits = Arc::clone(&shallow.0);
    let objects = log_paged_walk_handle(repo);
    let mut grafted_parents_to_skip: Vec<gix::ObjectId> = Vec::new();
    let mut buf = Vec::new();
    let filter: super::LogPagedWalkFilter = Box::new(move |id| {
        let id = id.to_owned();
        if let Ok(index) = grafted_parents_to_skip.binary_search(&id) {
            grafted_parents_to_skip.remove(index);
            return false;
        }
        if shallow_commits.binary_search(&id).is_ok()
            && let Ok(commit) = objects.find_commit_iter(&id, &mut buf)
        {
            grafted_parents_to_skip.extend(commit.parent_ids());
            grafted_parents_to_skip.sort();
        }
        true
    });
    filter
}

pub(crate) fn new_commit_time_walk(
    repo: &gix::ThreadSafeRepository,
    tips: &[gix::ObjectId],
    parents: gix::traverse::commit::Parents,
    filter: super::LogPagedWalkFilter,
) -> Result<super::LogPagedWalk> {
    let commit_graph = repo
        .to_thread_local()
        .commit_graph_if_enabled()
        .ok()
        .flatten();
    Ok(super::LogPagedWalk::CommitTime(
        gix::traverse::commit::Simple::filtered(
            tips.iter().copied(),
            log_paged_walk_handle(repo),
            filter,
        )
        .sorting(gix::traverse::commit::simple::Sorting::ByCommitTime(
            CommitTimeOrder::NewestFirst,
        ))
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix walk: {e}"))))?
        // Set after the sorting, the way `rev_walk` does: first-parent mode
        // walks the chain in order rather than by date, and asking for it
        // swaps the queue.
        .parents(parents)
        .commit_graph(commit_graph),
    ))
}

pub(crate) fn new_log_paged_walk(
    repo: &gix::ThreadSafeRepository,
    tips: impl IntoIterator<Item = gix::ObjectId>,
    mode: HistoryMode,
    shallow: &super::ShallowSnapshot,
    cancellation: Option<&CancellationToken>,
    mut chunks: Option<&mut ChunkEmitter<'_>>,
) -> Result<super::LogPagedWalkState> {
    let parents = if mode == HistoryMode::FirstParent {
        gix::traverse::commit::Parents::First
    } else {
        gix::traverse::commit::Parents::All
    };
    // Collected because the commit-date queue is also the fallback below, and a
    // fallback cannot re-consume an iterator the topo builder already drained.
    let tips: Vec<gix::ObjectId> = tips.into_iter().collect();
    let filter = log_paged_walk_filter(repo, shallow);
    let walk_cancellation = LogWalkCancellation::new(cancellation);

    // The topo walk reports all parents in first-parent mode and cannot cross a
    // shallow boundary whose missing parents are resolved during its setup.
    let walk = if mode == HistoryMode::FirstParent || shallow.is_shallow() {
        new_commit_time_walk(repo, &tips, parents, filter)?
    } else {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        if let Some(chunks) = chunks.as_mut() {
            chunks.ordering_started();
        }
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        let commit_graph = repo
            .to_thread_local()
            .commit_graph_if_enabled()
            .ok()
            .flatten();
        let find = CancellableLogWalkFind {
            inner: log_paged_walk_handle(repo),
            cancellation: walk_cancellation.clone(),
        };
        match gix::traverse::commit::topo::Builder::new(find)
            .with_predicate(filter)
            .with_tips(tips.iter().copied())
            .sorting(gix::traverse::commit::topo::Sorting::DateOrder)
            .parents(parents)
            .with_commit_graph(commit_graph)
            .build()
        {
            Ok(walk) => {
                if let Some(cancellation) = cancellation {
                    cancellation.check_cancelled()?;
                }
                super::LogPagedWalk::DateOrder(walk)
            }
            // Fall back lazily when a missing ancestor prevents the in-degree pass.
            Err(error) if topo_build_error_is_missing_object(&error) => {
                if let Some(cancellation) = cancellation {
                    cancellation.check_cancelled()?;
                }
                let filter = log_paged_walk_filter(repo, shallow);
                new_commit_time_walk(repo, &tips, parents, filter)?
            }
            Err(error) => {
                if let Some(cancellation) = cancellation {
                    cancellation.check_cancelled()?;
                }
                return Err(Error::new(ErrorKind::Backend(format!(
                    "gix date-order walk: {error}"
                ))));
            }
        }
    };
    Ok(super::LogPagedWalkState {
        pending: std::collections::VecDeque::new(),
        walk,
        cancellation: walk_cancellation,
    })
}

pub(crate) fn topo_build_error_is_missing_object(
    error: &gix::traverse::commit::topo::Error,
) -> bool {
    matches!(
        error,
        gix::traverse::commit::topo::Error::Find(
            gix::objs::find::existing_iter::Error::NotFound { .. }
        )
    )
}

pub(crate) fn apply_first_parent_resume_hint(page: &mut LogPage) {
    if let Some(cursor) = page.next_cursor.as_mut() {
        cursor.resume_from = page
            .commits
            .last()
            .and_then(|commit| commit.parent_ids.first().cloned());
    }
}

pub(crate) fn reflog_unborn_head_error(repo: &gix::Repository) -> Error {
    let branch = repo
        .head_name()
        .ok()
        .flatten()
        .map(|name| {
            let name = name.as_bstr().to_str_lossy();
            name.strip_prefix("refs/heads/")
                .unwrap_or(name.as_ref())
                .to_string()
        })
        .unwrap_or_else(|| "HEAD".to_string());
    let detail = format!("fatal: your current branch '{branch}' does not have any commits yet");
    let stderr = format!("{detail}\n").into_bytes();
    Error::new(ErrorKind::Git(GitFailure::new(
        "git reflog",
        GitFailureId::CommandFailed,
        Some(128),
        Vec::new(),
        stderr,
        Some(detail),
    )))
}

pub(crate) fn paginate_commits(
    commits: impl Iterator<Item = Result<Commit>>,
    limit: usize,
    cursor: Option<&LogCursor>,
) -> Result<LogPage> {
    if limit == 0 {
        return Ok(empty_log_page());
    }

    let mut cursor_gate = CursorGate::new(cursor);
    let mut result: Vec<Commit> = Vec::with_capacity(limit);
    let mut next_cursor: Option<LogCursor> = None;

    for commit in commits {
        let commit = commit?;
        if cursor_gate.should_skip(commit.id.as_ref()) {
            continue;
        }

        if result.len() >= limit {
            next_cursor = result.last().map(|c| LogCursor {
                last_seen: c.id.clone(),
                resume_from: None,
                resume_token: None,
            });
            break;
        }

        result.push(commit);
    }

    Ok(LogPage {
        commits: result,
        next_cursor,
    })
}

/// Reports a page as it is built, throttled so a walk that runs for seconds
/// updates the caller a handful of times a second instead of per commit.
pub(crate) struct ChunkEmitter<'a> {
    on_chunk: &'a mut dyn FnMut(LogChunk),
    next_emit_at: std::time::Instant,
    interval: std::time::Duration,
    scanned: u64,
}

pub(crate) const CHUNK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(120);

impl<'a> ChunkEmitter<'a> {
    pub(crate) fn new(on_chunk: &'a mut dyn FnMut(LogChunk)) -> Self {
        Self::with_interval(on_chunk, CHUNK_INTERVAL)
    }

    pub(crate) fn with_interval(
        on_chunk: &'a mut dyn FnMut(LogChunk),
        interval: std::time::Duration,
    ) -> Self {
        Self {
            on_chunk,
            next_emit_at: std::time::Instant::now() + interval,
            interval,
            scanned: 0,
        }
    }

    /// Make the expensive ordering phase visible before gix begins its
    /// in-degree pass. That pass cannot yield page rows yet, so an empty prefix
    /// with a zero count is the only truthful chunk available at this point.
    pub(crate) fn ordering_started(&mut self) {
        self.next_emit_at = std::time::Instant::now() + self.interval;
        (self.on_chunk)(LogChunk {
            commits: Vec::new(),
            scanned: self.scanned,
        });
    }

    /// Counts `count` more visited commits and reports the page so far once the
    /// interval has elapsed — including when nothing new matched, so a filter
    /// that is finding nothing still shows that it is working.
    pub(crate) fn visited(&mut self, count: u64, commits: &[Commit]) {
        self.scanned += count;
        if std::time::Instant::now() < self.next_emit_at {
            return;
        }
        self.next_emit_at = std::time::Instant::now() + self.interval;
        (self.on_chunk)(LogChunk {
            commits: commits.to_vec(),
            scanned: self.scanned,
        });
    }
}

/// Whether `mode` wants a commit with `parent_count` parents.
///
/// `FirstParent` and `AllBranches` shape the walk itself — the parents it
/// follows, the tips it starts from — rather than filtering what it yields, so
/// everything those walks produce belongs on the page.
pub(crate) fn mode_includes(mode: HistoryMode, parent_count: usize) -> bool {
    match mode {
        HistoryMode::FullReachable | HistoryMode::FirstParent | HistoryMode::AllBranches => true,
        HistoryMode::NoMerges => parent_count < 2,
        HistoryMode::MergesOnly => parent_count > 1,
    }
}

pub(crate) const DECODE_BATCH: usize = 2_048;
/// Check the progress clock after this many visited candidates. Keeping the
/// clock read out of the per-commit hot path matters on million-commit walks,
/// while this still updates promptly when cursor or mode filtering rejects an
/// entire decode batch's worth of commits.
pub(crate) const CHUNK_VISIT_STRIDE: u64 = 256;
/// Commit-decode threads in flight across the whole process. Object inflation is
/// what a filtered walk spends its time on and it parallelizes cleanly, but the
/// budget is shared: several repositories loading at once must not multiply into
/// as many decode threads as they have walks.
pub(crate) const DECODE_THREADS_MAX: usize = 8;
pub(crate) const DECODE_PARALLEL_MIN: usize = 96;
pub(crate) static DECODE_THREADS_IN_USE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Helper threads claimed out of [`DECODE_THREADS_MAX`] for one page build,
/// released on drop. A page that gets none still decodes, just on the thread
/// building it.
pub(crate) struct DecodeThreadBudget(usize);

impl DecodeThreadBudget {
    pub(crate) fn claim() -> Self {
        use std::sync::atomic::Ordering;
        let want = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(DECODE_THREADS_MAX)
            .saturating_sub(1);
        let mut in_use = DECODE_THREADS_IN_USE.load(Ordering::Relaxed);
        loop {
            let claimed = want.min(DECODE_THREADS_MAX.saturating_sub(in_use));
            if claimed == 0 {
                return Self(0);
            }
            match DECODE_THREADS_IN_USE.compare_exchange_weak(
                in_use,
                in_use + claimed,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Self(claimed),
                Err(observed) => in_use = observed,
            }
        }
    }
}

impl Drop for DecodeThreadBudget {
    fn drop(&mut self) {
        DECODE_THREADS_IN_USE.fetch_sub(self.0, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The decoders for one page build: a repository handle and its scratch buffers
/// per thread, plus the thread budget they were sized against.
///
/// Built once per page rather than once per batch. `to_thread_local` clones the
/// object database handle and its caches, and a fresh [`CommitDecodeState`]
/// starts with an empty inflate buffer that has to grow again — paid per batch,
/// that is what forces batches to be large, and large batches are what leaves
/// the walk cache holding thousands of undecided commits per parked walk.
pub(crate) struct DecodeWorkers {
    _budget: DecodeThreadBudget,
    workers: Vec<(gix::Repository, CommitDecodeState)>,
}

impl DecodeWorkers {
    pub(crate) fn new(repo: &gix::ThreadSafeRepository) -> Self {
        let budget = DecodeThreadBudget::claim();
        let workers = (0..=budget.0)
            .map(|_| (repo.to_thread_local(), CommitDecodeState::default()))
            .collect();
        Self {
            _budget: budget,
            workers,
        }
    }

    /// Decodes a batch of commits into `out`, dropping the ones `author`
    /// rejects and preserving walk order.
    ///
    /// The traversal itself is cheap once it is reading a commit-graph, so the
    /// remaining cost is the object read per commit — which is what gets spread
    /// across the workers here.
    pub(crate) fn decode(
        &mut self,
        infos: &[gix::traverse::commit::Info],
        author: Option<&AuthorFilter>,
        out: &mut Vec<Option<Commit>>,
    ) -> Result<()> {
        fn decode_chunk(
            (repo, decode_state): &mut (gix::Repository, CommitDecodeState),
            chunk: &[gix::traverse::commit::Info],
            author: Option<&AuthorFilter>,
            out: &mut Vec<Option<Commit>>,
        ) -> Result<()> {
            for info in chunk {
                out.push(commit_from_walk_parts(
                    repo,
                    &info.id,
                    &info.parent_ids,
                    info.commit_time,
                    decode_state,
                    author,
                )?);
            }
            Ok(())
        }

        out.clear();
        out.reserve(infos.len());

        let threads = if infos.len() < DECODE_PARALLEL_MIN {
            1
        } else {
            self.workers.len()
        };
        if threads <= 1 {
            return decode_chunk(&mut self.workers[0], infos, author, out);
        }

        let chunk_len = infos.len().div_ceil(threads).max(1);
        let mut parts: Vec<Result<Vec<Option<Commit>>>> = Vec::with_capacity(threads);
        std::thread::scope(|scope| {
            let handles: Vec<_> = infos
                .chunks(chunk_len)
                .zip(self.workers.iter_mut())
                .map(|(chunk, worker)| {
                    scope.spawn(move || {
                        let mut decoded = Vec::with_capacity(chunk.len());
                        decode_chunk(worker, chunk, author, &mut decoded)?;
                        Ok(decoded)
                    })
                })
                .collect();
            for handle in handles {
                parts.push(handle.join().unwrap_or_else(|_| {
                    Err(Error::new(ErrorKind::Backend(
                        "gix commit decode worker panicked".to_string(),
                    )))
                }));
            }
        });

        for part in parts {
            out.extend(part?);
        }
        Ok(())
    }
}

/// Builds a page from a resumable walk, decoding a batch of commits at a time.
///
/// Returns the commits found and whether the walk still has more to give; the
/// caller parks `walk_state` in the walk cache so the next page picks up where
/// this one stopped instead of re-traversing from the tip.
pub(crate) fn log_page_from_paged_walk_state(
    repo: &gix::ThreadSafeRepository,
    walk_state: &mut super::LogPagedWalkState,
    limit: usize,
    mut cursor_gate: Option<&mut CursorGate<'_>>,
    cancellation: Option<&CancellationToken>,
    author: Option<&AuthorFilter>,
    mut chunks: Option<&mut ChunkEmitter<'_>>,
    mut include: impl FnMut(&gix::traverse::commit::Info) -> bool,
) -> Result<(Vec<Commit>, bool)> {
    let mut workers = DecodeWorkers::new(repo);
    let mut commits = Vec::with_capacity(limit);
    let mut batch: Vec<gix::traverse::commit::Info> = Vec::with_capacity(DECODE_BATCH);
    let mut decoded: Vec<Option<Commit>> = Vec::with_capacity(DECODE_BATCH);
    let mut walk_done = false;
    let mut visited_since_emit = 0u64;

    while !walk_done {
        // An unfiltered page gathers only what it can use plus one commit to
        // prove another page exists. A filtered page has an unknown hit rate.
        let batch_cap = match author {
            Some(_) => DECODE_BATCH,
            None => DECODE_BATCH.min(limit.saturating_sub(commits.len()).saturating_add(1)),
        };

        // Gathering ids is the cheap half — with a commit-graph it touches no
        // objects at all — so the gate and the mode predicate run here, before
        // anything is handed to the decoders.
        batch.clear();
        while batch.len() < batch_cap {
            let Some(info) = walk_state.pending.pop_front() else {
                // Checked per commit walked, not per batch: a mode predicate or
                // a cursor gate that rejects everything can traverse an entire
                // history without filling one batch, and a superseded walk that
                // cannot be stopped holds a repo-load thread for all of it.
                if let Some(cancellation) = cancellation {
                    cancellation.check_cancelled()?;
                }
                let info = match walk_state.walk.next() {
                    None => {
                        walk_done = true;
                        break;
                    }
                    Some(Ok(info)) => info,
                    Some(Err(error)) => {
                        // The cancellable object lookup surfaces through gix's
                        // iterator error. Preserve cancellation as its public
                        // error kind instead of misreporting it as a backend
                        // failure.
                        if let Some(cancellation) = cancellation {
                            cancellation.check_cancelled()?;
                        }
                        return Err(Error::new(ErrorKind::Backend(format!("gix walk: {error}"))));
                    }
                };

                visited_since_emit = visited_since_emit.saturating_add(1);
                if visited_since_emit >= CHUNK_VISIT_STRIDE {
                    if let Some(chunks) = chunks.as_deref_mut() {
                        chunks.visited(visited_since_emit, &commits);
                    }
                    visited_since_emit = 0;
                }

                if let Some(cursor_gate) = cursor_gate.as_deref_mut()
                    && cursor_gate.should_skip_oid(info.id.as_ref())
                {
                    continue;
                }
                if !include(&info) {
                    continue;
                }
                batch.push(info);
                continue;
            };
            batch.push(info);
        }

        if batch.is_empty() {
            if let Some(chunks) = chunks.as_deref_mut() {
                chunks.visited(visited_since_emit, &commits);
            }
            break;
        }

        // Park the lookahead commit without decoding it.
        let decode_len = match author {
            Some(_) => batch.len(),
            None => batch.len().min(limit.saturating_sub(commits.len())),
        };
        for info in batch.drain(decode_len..).rev() {
            walk_state.pending.push_front(info);
        }

        workers.decode(&batch, author, &mut decoded)?;

        for (index, commit) in decoded.drain(..).enumerate() {
            let Some(commit) = commit else {
                continue;
            };
            // The limit is checked against *matching* commits: reporting "there
            // is more" because the next commit of any author exists would hand
            // the caller a cursor whose page re-walks the rest of history to
            // return nothing.
            if commits.len() >= limit {
                // Everything from here on is undecided; put it back for the
                // next page, in walk order.
                for info in batch.drain(index..).rev() {
                    walk_state.pending.push_front(info);
                }
                if let Some(chunks) = chunks.as_deref_mut() {
                    chunks.visited(visited_since_emit, &commits);
                }
                return Ok((commits, true));
            }
            commits.push(commit);
        }

        // Reported after the batch lands, so a chunk always carries everything
        // found so far rather than trailing a batch behind.
        if let Some(chunks) = chunks.as_deref_mut() {
            chunks.visited(visited_since_emit, &commits);
        }
        visited_since_emit = 0;

        if commits.len() >= limit {
            if !walk_state.pending.is_empty() {
                return Ok((commits, true));
            }
            if walk_done {
                return Ok((commits, false));
            }
            // A filtered page can fill exactly at the end of a decode batch.
            // Only another matching commit or actual EOF can answer whether a
            // successor exists, so keep walking instead of guessing from the
            // empty pending queue.
        }
    }

    Ok((commits, false))
}
