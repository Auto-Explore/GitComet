use super::*;

impl GixRepo {
    pub(super) fn log_page_cache_key(
        &self,
        mode: HistoryMode,
        seed: super::LogPageSeed,
        shallow: &super::ShallowSnapshot,
        limit: usize,
        cursor: Option<&LogCursor>,
        author: Option<&AuthorFilter>,
    ) -> super::LogPageCacheKey {
        super::LogPageCacheKey {
            mode,
            seed,
            shallow: shallow.clone(),
            limit,
            last_seen: cursor.map(|cursor| cursor.last_seen.clone()),
            resume_from: cursor.and_then(|cursor| cursor.resume_from.clone()),
            author: author.cloned(),
        }
    }

    /// Serves a cached page and refreshes its successor's LRU position.
    pub(super) fn cached_log_page(&self, key: &super::LogPageCacheKey) -> Option<Arc<LogPage>> {
        let mut cache = self.log_page_cache.lock().expect("log page cache");
        let index = cache.iter().position(|entry| &entry.key == key)?;
        let entry = cache.remove(index);
        let page = Arc::clone(&entry.page);
        let successor_key = page
            .next_cursor
            .as_ref()
            .map(|cursor| super::LogPageCacheKey {
                mode: key.mode,
                seed: key.seed.clone(),
                shallow: key.shallow.clone(),
                limit: key.limit,
                last_seen: Some(cursor.last_seen.clone()),
                resume_from: cursor.resume_from.clone(),
                author: key.author.clone(),
            });
        cache.push(entry);

        if let Some(successor_key) = successor_key
            && let Some(successor) = cache.iter().position(|entry| entry.key == successor_key)
        {
            let successor = cache.remove(successor);
            cache.push(successor);
        }
        Some(page)
    }

    /// Shares the page with the cache instead of copying it in and out: the
    /// state keeps the same `Arc`, so a served page costs no commit clones.
    pub(super) fn finish_log_page(
        &self,
        key: super::LogPageCacheKey,
        page: LogPage,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Arc<LogPage>> {
        let page = Arc::new(page);
        self.store_log_page(key, &page);
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        Ok(page)
    }

    pub(super) fn store_log_page(&self, key: super::LogPageCacheKey, page: &Arc<LogPage>) {
        let row_limit = super::super::LOG_PAGE_CACHE_ROW_LIMIT;
        if page.commits.len() > row_limit {
            return;
        }
        let mut cache = self.log_page_cache.lock().expect("log page cache");
        if let Some(index) = cache.iter().position(|entry| entry.key == key) {
            cache.remove(index);
        }
        let mut rows = cache
            .iter()
            .map(|entry| entry.page.commits.len())
            .sum::<usize>();
        while !cache.is_empty()
            && (cache.len() >= super::LOG_PAGE_CACHE_LIMIT || rows + page.commits.len() > row_limit)
        {
            rows -= cache.remove(0).page.commits.len();
        }
        cache.push(super::LogPageCacheEntry {
            key,
            page: Arc::clone(page),
        });
    }

    pub(super) fn log_file_follow_cache_key(
        path: &Path,
        head_oid: Option<gix::ObjectId>,
    ) -> super::LogFileFollowCacheKey {
        super::LogFileFollowCacheKey {
            head_oid,
            path: path.to_path_buf(),
        }
    }

    pub(super) fn cached_log_file_follow_commits(
        &self,
        key: &super::LogFileFollowCacheKey,
    ) -> Option<Arc<Vec<Commit>>> {
        let mut cache = self
            .log_file_follow_cache
            .lock()
            .expect("log file follow cache");
        let index = cache.iter().position(|entry| &entry.key == key)?;
        let entry = cache.remove(index);
        let commits = Arc::clone(&entry.commits);
        cache.push(entry);
        Some(commits)
    }

    pub(super) fn store_log_file_follow_commits(
        &self,
        key: super::LogFileFollowCacheKey,
        commits: Arc<Vec<Commit>>,
    ) {
        let mut cache = self
            .log_file_follow_cache
            .lock()
            .expect("log file follow cache");
        if let Some(index) = cache.iter().position(|entry| entry.key == key) {
            cache.remove(index);
        }
        if cache.len() >= super::LOG_FILE_FOLLOW_CACHE_LIMIT {
            cache.remove(0);
        }
        cache.push(super::LogFileFollowCacheEntry { key, commits });
    }

    pub(super) fn take_log_paged_walk(
        &self,
        token: &str,
        mode: HistoryMode,
        tips: &[gix::ObjectId],
        shallow: &super::ShallowSnapshot,
        author: Option<&AuthorFilter>,
    ) -> Option<super::LogPagedWalkState> {
        let mut cache = self
            .log_paged_walk_cache
            .lock()
            .expect("log paged walk cache");
        let index = cache.entries.iter().position(|entry| {
            entry.token.as_ref() == token
                && entry.mode == mode
                && entry.tips.as_ref() == tips
                && &entry.shallow == shallow
                && entry.author.as_ref() == author
        })?;
        Some(cache.entries.remove(index).state)
    }

    pub(super) fn store_log_paged_walk(
        &self,
        mode: HistoryMode,
        tips: &Arc<[gix::ObjectId]>,
        shallow: &super::ShallowSnapshot,
        author: Option<&AuthorFilter>,
        state: super::LogPagedWalkState,
    ) -> Arc<str> {
        let mut cache = self
            .log_paged_walk_cache
            .lock()
            .expect("log paged walk cache");
        let token: Arc<str> = Arc::from(cache.next_id.to_string());
        cache.next_id = cache.next_id.wrapping_add(1);
        if cache.entries.len() >= super::LOG_PAGED_WALK_CACHE_LIMIT {
            cache.entries.remove(0);
        }
        // Date-order walks retain in-degree state proportional to history size.
        if state.walk.is_date_order() {
            while cache
                .entries
                .iter()
                .filter(|entry| entry.state.walk.is_date_order())
                .count()
                >= super::LOG_PAGED_TOPO_WALK_CACHE_LIMIT
            {
                let Some(oldest) = cache
                    .entries
                    .iter()
                    .position(|entry| entry.state.walk.is_date_order())
                else {
                    break;
                };
                cache.entries.remove(oldest);
            }
        }
        cache.entries.push(super::LogPagedWalkCacheEntry {
            token: Arc::clone(&token),
            mode,
            tips: Arc::clone(tips),
            shallow: shallow.clone(),
            author: author.cloned(),
            state,
        });
        token
    }

    pub(in super::super) fn resolve_file_path_at_commit_impl(
        &self,
        path: &Path,
        commit: &CommitId,
    ) -> Result<Option<PathBuf>> {
        // Fast path: the file is named `path` in this commit already.
        if self.path_exists_in_commit_tree(commit, path) {
            return Ok(Some(path.to_path_buf()));
        }
        // Otherwise the file is named differently in this commit; follow renames
        // to find the name it has in that commit's tree.
        self.resolve_renamed_path_at_commit(path, commit)
    }

    /// Whether `path` is present in the tree of `commit`. Best-effort: any lookup
    /// failure (bad rev, missing object) is treated as "not present".
    pub(super) fn path_exists_in_commit_tree(&self, commit: &CommitId, path: &Path) -> bool {
        let repo = self.repo();
        let Ok(commit) = find_commit_by_id(&repo, commit) else {
            return false;
        };
        let Ok(tree) = commit.tree() else {
            return false;
        };
        matches!(tree.lookup_entry_by_path(path), Ok(Some(_)))
    }

    /// Find the file's name in `commit`'s tree by following renames from `path`.
    /// Runs `git log --follow --name-status` and reads the entry for `commit`:
    /// a rename yields its destination; a plain change yields its path; a
    /// deletion (the followed name was renamed away at `commit`) is resolved to
    /// the rename's destination via `git diff-tree -M`.
    pub(super) fn resolve_renamed_path_at_commit(
        &self,
        path: &Path,
        commit: &CommitId,
    ) -> Result<Option<PathBuf>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("core.quotePath=false")
            .arg("log")
            .arg("--follow")
            .arg("--name-status")
            .arg("-M")
            // Record separator (0x1e) before each commit hash so records can be
            // split unambiguously from the name-status lines that follow.
            .arg("--format=%x1e%H")
            .arg("--")
            .arg(path);
        let output = run_git_capture(cmd, "git log --follow --name-status")?;

        let target = commit.as_ref();
        for record in output.split('\u{1e}') {
            let mut lines = record.lines().map(str::trim).filter(|l| !l.is_empty());
            let Some(hash) = lines.next() else {
                continue;
            };
            if hash != target {
                continue;
            }
            // The pathspec filters output to the followed file, so the first
            // status line is the one we want.
            if let Some(status_line) = lines.next() {
                return self.interpret_name_status_for_commit(status_line, commit);
            }
            return Ok(None);
        }
        Ok(None)
    }

    /// Interpret one `--name-status` line (`<status>\t<path>[\t<path2>]`) as the
    /// file's name in the commit's tree.
    pub(super) fn interpret_name_status_for_commit(
        &self,
        status_line: &str,
        commit: &CommitId,
    ) -> Result<Option<PathBuf>> {
        let mut fields = status_line.split('\t');
        let status = fields.next().unwrap_or_default();
        let first = fields.next();
        let second = fields.next();
        let to_path = |s: &str| path_buf_from_git_bytes(s.as_bytes(), "git name-status path");
        match status.chars().next() {
            // Rename/copy: the destination is the name in this commit's tree.
            Some('R') | Some('C') => second.map(to_path).transpose(),
            // Added/modified/type-change: the listed path is the name here.
            Some('A') | Some('M') | Some('T') => first.map(to_path).transpose(),
            // Deleted under the followed name: it was renamed away at this commit,
            // so the tree holds the rename destination — recover it.
            Some('D') => match first.map(to_path).transpose()? {
                Some(old) => self.rename_destination_at_commit(commit, &old),
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// The hex object id of `commit`'s first parent, or `None` for a root commit
    /// (or any lookup failure).
    pub(super) fn first_parent_id(&self, commit: &CommitId) -> Option<String> {
        let repo = self.repo();
        let commit = find_commit_by_id(&repo, commit).ok()?;
        commit.parent_ids().next().map(|parent| parent.to_string())
    }

    /// The destination path of a rename of `old_path` introduced by `commit`,
    /// using rename detection against its parent.
    pub(super) fn rename_destination_at_commit(
        &self,
        commit: &CommitId,
        old_path: &Path,
    ) -> Result<Option<PathBuf>> {
        // Diff against the first parent explicitly. A bare `git diff-tree <merge>`
        // emits no per-file rows for a merge commit (it needs -m/-c/--cc), which
        // would silently fail to resolve a rename introduced at a merge; passing
        // both endpoints makes diff-tree produce a normal first-parent diff. For a
        // non-merge commit this is identical to the implicit single-arg form, and
        // for a root commit (no parent) there is no rename to find.
        let Some(parent) = self.first_parent_id(commit) else {
            return Ok(None);
        };
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("core.quotePath=false")
            .arg("diff-tree")
            .arg("-M")
            .arg("-r")
            .arg("--name-status")
            .arg("--no-commit-id")
            .arg(&parent)
            .arg(commit.as_ref());
        let output = run_git_capture(cmd, "git diff-tree -M")?;

        for line in output.lines() {
            let mut fields = line.split('\t');
            let status = fields.next().unwrap_or_default();
            if !status.starts_with('R') && !status.starts_with('C') {
                continue;
            }
            let (Some(old), Some(new)) = (fields.next(), fields.next()) else {
                continue;
            };
            let old = path_buf_from_git_bytes(old.as_bytes(), "git diff-tree old path")?;
            if old == old_path {
                return Ok(Some(path_buf_from_git_bytes(
                    new.as_bytes(),
                    "git diff-tree new path",
                )?));
            }
        }
        Ok(None)
    }

    pub(super) fn log_follow_commits(
        &self,
        path: &Path,
        max_count: Option<usize>,
    ) -> Result<Vec<Commit>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("log")
            .arg("--follow")
            .arg("--date=unix")
            .arg("--pretty=format:%H%x1f%P%x1f%an%x1f%ct%x1f%s%x1e");
        if let Some(max_count) = max_count {
            cmd.arg(format!("-n{max_count}"));
        }
        cmd.arg("--").arg(path);

        run_git_parsed_stdout(cmd, "git log --follow", false, |stdout| {
            parse_git_log_pretty_records_from_reader(stdout).map(|page| page.commits)
        })
    }

    pub(in super::super) fn log_head_page_impl(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<Arc<LogPage>> {
        self.log_history_mode_page_impl(HistoryMode::FirstParent, limit, cursor)
    }

    pub(in super::super) fn log_head_page_cancellable_impl(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<Arc<LogPage>> {
        self.log_history_mode_page_cancellable_impl(
            HistoryMode::FirstParent,
            limit,
            cursor,
            cancellation,
        )
    }

    pub(in super::super) fn log_history_mode_page_impl(
        &self,
        mode: HistoryMode,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<Arc<LogPage>> {
        self.log_history_mode_page_impl_inner(mode, None, limit, cursor, None, None)
    }

    pub(in super::super) fn log_history_mode_page_cancellable_impl(
        &self,
        mode: HistoryMode,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<Arc<LogPage>> {
        self.log_history_mode_page_impl_inner(mode, None, limit, cursor, Some(cancellation), None)
    }

    pub(in super::super) fn log_history_mode_page_filtered_cancellable_impl(
        &self,
        mode: HistoryMode,
        author: Option<&str>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<Arc<LogPage>> {
        self.log_history_mode_page_impl_inner(mode, author, limit, cursor, Some(cancellation), None)
    }

    /// Filtered, cancellable, streaming variant: `on_chunk` sees the page as it
    /// fills in. The one entry point the app uses — the plain variants above
    /// exist for callers with no filter and nothing to cancel. See
    /// [`gitcomet_core::services::GitRepository::log_history_mode_page_streaming`].
    pub(in super::super) fn log_history_mode_page_streaming_impl(
        &self,
        mode: HistoryMode,
        author: Option<&str>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
        on_chunk: &mut dyn FnMut(LogChunk),
    ) -> Result<Arc<LogPage>> {
        let mut chunks = ChunkEmitter::new(on_chunk);
        self.log_history_mode_page_impl_inner(
            mode,
            author,
            limit,
            cursor,
            Some(cancellation),
            Some(&mut chunks),
        )
    }

    /// One page from the resumable walk for `mode` over `tips`.
    ///
    /// The cursor's token resumes the walk that built the previous page, which
    /// is what keeps paging O(page) instead of O(history): a filtered walk that
    /// had to cross the whole repository to fill one page would otherwise cross
    /// it again, and again, for every page after it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn log_paged_page(
        &self,
        mode: HistoryMode,
        tips: Arc<[gix::ObjectId]>,
        shallow: &super::ShallowSnapshot,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: Option<&CancellationToken>,
        author: Option<&AuthorFilter>,
        mut chunks: Option<&mut ChunkEmitter<'_>>,
    ) -> Result<LogPage> {
        if tips.is_empty() {
            return Ok(empty_log_page());
        }

        let cached_walk_state = cursor
            .and_then(|cursor| cursor.resume_token.as_deref())
            .and_then(|token| self.take_log_paged_walk(token, mode, &tips, shallow, author));

        // Tokens go stale on cache eviction or a change of tips, and then the
        // walk has to be rebuilt. A first-parent cursor carries `resume_from`,
        // which names the next commit outright, so that walk restarts there;
        // anything else restarts at the tips and skips forward to `last_seen`.
        // Only first-parent walks may read it: on any other mode the commit it
        // names is one of many at that depth, and starting there would drop
        // every branch beside it.
        let resume_tip = cursor
            .filter(|_| mode == HistoryMode::FirstParent)
            .and_then(|cursor| cursor.resume_from.as_ref())
            .and_then(object_id_from_commit_id);
        let (mut walk_state, mut cursor_gate) = match (cached_walk_state, resume_tip) {
            (Some(walk_state), _) => (walk_state, None),
            (None, Some(resume_tip)) => (
                new_log_paged_walk(
                    &self._repo,
                    [resume_tip],
                    mode,
                    shallow,
                    cancellation,
                    chunks.as_deref_mut(),
                )?,
                None,
            ),
            (None, None) => (
                new_log_paged_walk(
                    &self._repo,
                    tips.iter().copied(),
                    mode,
                    shallow,
                    cancellation,
                    chunks.as_deref_mut(),
                )?,
                cursor.map(|cursor| CursorGate::new(Some(cursor))),
            ),
        };
        // A parked walk outlives the request that created it. Rebind the lookup
        // hook before resuming so an old token cannot poison this page, and so
        // this page's cancellation reaches work performed inside gix.
        walk_state.cancellation.replace(cancellation);

        let (commits, has_more) = log_page_from_paged_walk_state(
            &self._repo,
            &mut walk_state,
            limit,
            cursor_gate.as_mut(),
            cancellation,
            author,
            chunks,
            |info| mode_includes(mode, info.parent_ids.len()),
        )?;

        let next_cursor = has_more
            .then(|| commits.last())
            .flatten()
            .map(|commit| LogCursor {
                last_seen: commit.id.clone(),
                resume_from: None,
                resume_token: Some(
                    self.store_log_paged_walk(mode, &tips, shallow, author, walk_state),
                ),
            });
        let mut page = LogPage {
            commits,
            next_cursor,
        };
        if mode == HistoryMode::FirstParent {
            // A second way back into the history, for when the token is gone.
            apply_first_parent_resume_hint(&mut page);
        }
        Ok(page)
    }

    pub(super) fn log_history_mode_page_impl_inner(
        &self,
        mode: HistoryMode,
        author: Option<&str>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: Option<&CancellationToken>,
        mut chunks: Option<&mut ChunkEmitter<'_>>,
    ) -> Result<Arc<LogPage>> {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        if limit == 0 {
            return Ok(Arc::new(empty_log_page()));
        }

        // Normalized once, here, so the matcher and both caches downstream can
        // only ever see the same spelling of the filter.
        let author = AuthorFilter::new(author);
        let author = author.as_ref();

        if mode == HistoryMode::AllBranches {
            return self.log_all_branches_page_impl_inner(
                limit,
                cursor,
                cancellation,
                author,
                chunks.as_deref_mut(),
            );
        }

        let repo = self.repo();
        let shallow = shallow_snapshot(&repo)?;
        let head_id = gix_head_id_or_none(&repo)?;
        let cache_key = self.log_page_cache_key(
            mode,
            super::LogPageSeed::Head(head_id),
            &shallow,
            limit,
            cursor,
            author,
        );
        if let Some(page) = self.cached_log_page(&cache_key) {
            return Ok(page);
        }

        let page = match head_id {
            Some(head_id) => self.log_paged_page(
                mode,
                Arc::from(vec![head_id]),
                &shallow,
                limit,
                cursor,
                cancellation,
                author,
                chunks,
            )?,
            None => empty_log_page(),
        };

        self.finish_log_page(cache_key, page, cancellation)
    }

    pub(in super::super) fn log_all_branches_page_impl(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<Arc<LogPage>> {
        self.log_all_branches_page_impl_inner(limit, cursor, None, None, None)
    }

    pub(in super::super) fn log_all_branches_page_cancellable_impl(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<Arc<LogPage>> {
        self.log_all_branches_page_impl_inner(limit, cursor, Some(cancellation), None, None)
    }

    pub(super) fn all_branches_tips(
        &self,
        repo: &gix::Repository,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Arc<[gix::ObjectId]>> {
        use rustc_hash::FxHasher;
        use std::hash::{Hash as _, Hasher as _};

        let refs = repo
            .references()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references: {e}"))))?;

        // Fingerprint pass: names, raw targets and followed symbolic chains
        // only, no object lookups.
        let head_id = gix_head_id_or_none(repo)?;
        let mut hasher = FxHasher::default();
        head_id.hash(&mut hasher);
        let mut ref_count = 0usize;
        let iter = refs
            .all()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references(all): {e}"))))?;
        for reference in iter {
            if let Some(cancellation) = cancellation {
                cancellation.check_cancelled()?;
            }
            let mut reference = reference
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix ref iter: {e}"))))?;
            if matches!(
                reference.name().category(),
                Some(gix::reference::Category::Tag)
            ) {
                continue;
            }
            super::super::git_ops::hash_reference_identity(&mut hasher, &mut reference);
            ref_count += 1;
        }
        // Older stash entries are reflog-only and need explicit tips.
        let stash_tips = stash_reflog_tips(repo, 50).unwrap_or_default();
        stash_tips.hash(&mut hasher);
        let fingerprint = hasher.finish();

        if let Some(cached) = self
            .all_branches_tips
            .lock()
            .expect("all branches tips cache")
            .as_ref()
            .filter(|cached| cached.fingerprint == fingerprint)
        {
            return Ok(Arc::clone(&cached.tips));
        }

        // Include custom ref namespaces while leaving tags out of the graph.
        let capacity = ref_count + stash_tips.len() + 1;
        let mut tips = Vec::with_capacity(capacity);
        let mut seen = FxHashSet::with_capacity_and_hasher(capacity, Default::default());
        if let Some(head_id) = head_id {
            tips.push(head_id);
            seen.insert(head_id);
        }

        let iter = refs
            .all()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references(all): {e}"))))?;
        for reference in iter {
            if let Some(cancellation) = cancellation {
                cancellation.check_cancelled()?;
            }
            let reference = reference
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix ref iter: {e}"))))?;
            if matches!(
                reference.name().category(),
                Some(gix::reference::Category::Tag)
            ) {
                continue;
            }
            let Some(id) = reference_commit_id(reference)? else {
                continue;
            };
            if seen.insert(id) {
                tips.push(id);
            }
        }

        for id in stash_tips {
            if seen.insert(id) {
                tips.push(id);
            }
        }

        tips.sort_unstable();

        let mut cached = self
            .all_branches_tips
            .lock()
            .expect("all branches tips cache");
        // Keep the previous allocation when the tips are unchanged so page
        // cache keys seeded from it keep comparing equal cheaply.
        let tips: Arc<[gix::ObjectId]> = match cached.as_ref() {
            Some(previous) if previous.tips.as_ref() == tips.as_slice() => {
                Arc::clone(&previous.tips)
            }
            _ => Arc::from(tips),
        };
        *cached = Some(crate::repo::AllBranchesTipsCacheEntry {
            fingerprint,
            tips: Arc::clone(&tips),
        });
        Ok(tips)
    }

    pub(super) fn log_all_branches_page_impl_inner(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: Option<&CancellationToken>,
        author: Option<&AuthorFilter>,
        chunks: Option<&mut ChunkEmitter<'_>>,
    ) -> Result<Arc<LogPage>> {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        if limit == 0 {
            return Ok(Arc::new(empty_log_page()));
        }

        let repo = self.repo();
        let shallow = shallow_snapshot(&repo)?;
        let tips = self.all_branches_tips(&repo, cancellation)?;

        let cache_key = self.log_page_cache_key(
            HistoryMode::AllBranches,
            super::LogPageSeed::Tips(Arc::clone(&tips)),
            &shallow,
            limit,
            cursor,
            author,
        );
        if let Some(page) = self.cached_log_page(&cache_key) {
            return Ok(page);
        }

        let page = self.log_paged_page(
            HistoryMode::AllBranches,
            tips,
            &shallow,
            limit,
            cursor,
            cancellation,
            author,
            chunks,
        )?;
        self.finish_log_page(cache_key, page, cancellation)
    }

    pub(in super::super) fn log_file_page_impl(
        &self,
        path: &Path,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<Arc<LogPage>> {
        if limit == 0 {
            return Ok(Arc::new(empty_log_page()));
        }

        // Only the first page is bounded. `git log --follow` does not combine
        // reliably with `--skip` across renames. Cursor pages cache the full
        // follow result so repeated "load more" requests do not rescan history.
        if cursor.is_none() {
            // One past the limit tells the page whether more follow; a limit
            // that cannot grow reads as "every commit" and drops the bound.
            let commits = self.log_follow_commits(path, limit.checked_add(1))?;
            return paginate_commits(commits.into_iter().map(Ok), limit, cursor).map(Arc::new);
        }

        let repo = self.repo();
        let head_oid = gix_head_id_or_none(&repo)?;
        let cache_key = Self::log_file_follow_cache_key(path, head_oid);
        let commits = if let Some(commits) = self.cached_log_file_follow_commits(&cache_key) {
            commits
        } else {
            let commits = Arc::new(self.log_follow_commits(path, None)?);
            self.store_log_file_follow_commits(cache_key, Arc::clone(&commits));
            commits
        };
        // Clone only the tail from the cursor on: the follow history can be
        // tens of thousands of commits, and the cursor gate would discard every
        // clone before `last_seen` anyway.
        let start = cursor
            .and_then(|cursor| {
                commits
                    .iter()
                    .position(|commit| commit.id == cursor.last_seen)
            })
            .unwrap_or(0);
        paginate_commits(commits[start..].iter().cloned().map(Ok), limit, cursor).map(Arc::new)
    }

    pub(in super::super) fn commit_details_impl(&self, id: &CommitId) -> Result<CommitDetails> {
        let repo = self.repo();
        let spec = id.as_ref();
        let commit = find_commit_by_id(&repo, id)?;

        let message = bytes_to_text_preserving_utf8(commit.message_raw_sloppy().as_ref())
            .trim_end()
            .to_string();
        let (author_name, author_email, authored_at_unix) = match commit.author() {
            Ok(signature) => (
                bytes_to_text_preserving_utf8(signature.name.as_ref()),
                bytes_to_text_preserving_utf8(signature.email.as_ref()),
                signature.time().ok().map(|time| time.seconds).unwrap_or(0),
            ),
            Err(_) => (String::new(), String::new(), 0),
        };
        let commit_time = commit
            .time()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit time {spec}: {e}"))))?;
        let committed_at = commit_time.format_or_unix(gix::date::time::format::ISO8601_STRICT);
        let committed_at_unix = commit_time.seconds;
        let parent_oids = commit
            .parent_ids()
            .map(|parent| parent.detach())
            .collect::<Vec<_>>();
        let parent_ids = parent_oids
            .iter()
            .map(|parent| CommitId(oid_to_arc_str(parent)))
            .collect::<Vec<_>>();
        let files = commit_file_changes(&repo, &commit, &parent_oids)?;

        Ok(CommitDetails {
            id: id.clone(),
            message,
            author_name,
            author_email,
            authored_at_unix,
            committed_at,
            committed_at_unix,
            parent_ids,
            files,
        })
    }

    pub(in super::super) fn diff_range_files_impl(
        &self,
        from: &CommitId,
        to: Option<&CommitId>,
    ) -> Result<Vec<CommitFileChange>> {
        match to {
            Some(to) => {
                let repo = self.repo();
                diff_range_files(&repo, from, to)
            }
            // Working-tree tip: the newer side is the live worktree, which has no
            // tree object, so shell out to `git diff <from>` for the file list
            // (consistent with the unified diff shown in the main pane).
            None => super::submodules::diff_commit_to_worktree_files(&self.spec.workdir, from),
        }
    }

    pub(in super::super) fn commit_messages_impl(&self, ids: &[CommitId]) -> Result<Vec<String>> {
        let repo = self.repo();
        ids.iter()
            .map(|id| {
                let commit = find_commit_by_id(&repo, id)?;
                Ok(
                    bytes_to_text_preserving_utf8(commit.message_raw_sloppy().as_ref())
                        .trim_end()
                        .to_string(),
                )
            })
            .collect()
    }

    pub(in super::super) fn topologically_order_commits_impl(
        &self,
        ids: &[CommitId],
    ) -> Result<Vec<CommitId>> {
        let repo = self.repo();
        let mut object_ids = Vec::with_capacity(ids.len());
        let mut selected = FxHashMap::with_capacity_and_hasher(ids.len(), Default::default());
        for (ix, id) in ids.iter().enumerate() {
            let spec = id.as_ref();
            let object_id = find_commit_by_id(&repo, id)?.id().detach();
            if selected.insert(object_id, ix).is_some() {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "duplicate commit in replay order: {spec}"
                ))));
            }
            object_ids.push(object_id);
        }

        // Discover the nearest selected ancestors of every selected commit.
        // Traversal stops at a selected node: that node's own edges carry the
        // remaining transitive dependency, avoiding unnecessary history walks.
        let mut children = vec![Vec::<usize>::new(); ids.len()];
        let mut pending_parents = vec![0usize; ids.len()];
        for (descendant_ix, &descendant) in object_ids.iter().enumerate() {
            let commit = repo.find_commit(descendant).map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "gix find commit {}: {e}",
                    ids[descendant_ix]
                )))
            })?;
            let mut stack = commit
                .parent_ids()
                .map(|parent| parent.detach())
                .collect::<Vec<_>>();
            let mut visited = FxHashSet::default();
            let mut direct_selected_ancestors = FxHashSet::default();
            while let Some(candidate) = stack.pop() {
                if !visited.insert(candidate) {
                    continue;
                }
                if let Some(&ancestor_ix) = selected.get(&candidate) {
                    if ancestor_ix != descendant_ix && direct_selected_ancestors.insert(ancestor_ix)
                    {
                        children[ancestor_ix].push(descendant_ix);
                        pending_parents[descendant_ix] += 1;
                    }
                    continue;
                }
                let ancestor = repo.find_commit(candidate).map_err(|e| {
                    Error::new(ErrorKind::Backend(format!(
                        "gix traverse ancestors of {}: {e}",
                        ids[descendant_ix]
                    )))
                })?;
                stack.extend(ancestor.parent_ids().map(|parent| parent.detach()));
            }
        }

        // Kahn's algorithm with input position as the ready-queue tie-break.
        let mut emitted = vec![false; ids.len()];
        let mut ordered = Vec::with_capacity(ids.len());
        while let Some(next) = (0..ids.len()).find(|&ix| !emitted[ix] && pending_parents[ix] == 0) {
            emitted[next] = true;
            ordered.push(ids[next].clone());
            for &child in &children[next] {
                pending_parents[child] -= 1;
            }
        }
        if ordered.len() != ids.len() {
            return Err(Error::new(ErrorKind::Backend(
                "commit graph contains a cycle while ordering replay commits".to_string(),
            )));
        }
        Ok(ordered)
    }

    pub(in super::super) fn recent_commit_messages_impl(
        &self,
        limit: usize,
    ) -> Result<Vec<RecentCommitMessage>> {
        let Some((limit, scan_limit)) = recent_commit_message_limits(limit) else {
            return Ok(Vec::new());
        };

        let page = self.log_history_mode_page_impl(HistoryMode::FirstParent, scan_limit, None)?;
        let repo = self.repo();
        let mut seen = FxHashSet::default();
        let mut messages = Vec::with_capacity(limit);

        for commit in page.commits.iter() {
            let commit_object = find_commit_by_id(&repo, &commit.id)?;
            let message =
                bytes_to_text_preserving_utf8(commit_object.message_raw_sloppy().as_ref())
                    .trim_end()
                    .to_string();
            if message.trim().is_empty() || !seen.insert(message.clone()) {
                continue;
            }

            messages.push(RecentCommitMessage {
                id: commit.id.clone(),
                summary: commit.summary.clone(),
                message,
            });
            if messages.len() >= limit {
                break;
            }
        }

        Ok(messages)
    }

    pub(in super::super) fn reflog_head_impl(&self, limit: usize) -> Result<Vec<ReflogEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let repo = self.repo();
        if gix_head_id_or_none(&repo)?.is_none() {
            return Err(reflog_unborn_head_error(&repo));
        }

        let head = repo
            .head()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head: {e}"))))?;
        let mut platform = head.log_iter();
        reflog_lines_rev(&mut platform, "HEAD", Some(limit))?
            .into_iter()
            .enumerate()
            .map(|(index, line)| {
                Ok(ReflogEntry {
                    index,
                    new_id: CommitId(oid_to_arc_str(&line.new_oid)),
                    message: bstr_to_arc_str(line.message.as_ref()),
                    time: unix_seconds_to_system_time(line.signature.time.seconds),
                    selector: format!("HEAD@{{{index}}}").into(),
                    author: bstr_to_arc_str(line.signature.name.as_ref()),
                })
            })
            .collect()
    }
}

/// Resolves a `CommitId` to its commit. Ids are full hex, so the object is
/// looked up directly; the revspec parser (and its prefix disambiguation
/// against every pack) only runs for anything that is not a plain id.
fn find_commit_by_id<'repo>(
    repo: &'repo gix::Repository,
    id: &CommitId,
) -> Result<gix::Commit<'repo>> {
    let spec = id.as_ref();
    let object = match object_id_from_commit_id(id) {
        Some(oid) => repo.find_object(oid).map_err(|e| {
            Error::new(ErrorKind::Backend(format!("gix commit object {spec}: {e}")))
        })?,
        None => repo
            .rev_parse_single(spec)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix rev-parse {spec}: {e}"))))?
            .object()
            .map_err(|e| {
                Error::new(ErrorKind::Backend(format!("gix commit object {spec}: {e}")))
            })?,
    };
    object
        .peel_to_commit()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel commit {spec}: {e}"))))
}
