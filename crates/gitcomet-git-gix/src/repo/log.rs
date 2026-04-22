use super::history::gix_head_id_or_none;
use super::{GixRepo, bstr_to_arc_str, oid_to_arc_str};
use crate::util::{
    bytes_to_text_preserving_utf8, parse_git_log_pretty_records, path_buf_from_git_bytes,
    run_git_capture, unix_seconds_to_system_time, unix_seconds_to_system_time_or_epoch,
};
use gitcomet_core::domain::{
    Commit, CommitDetails, CommitFileChange, CommitId, CommitParentIds, HistoryMode, LogCursor,
    LogPage, ReflogEntry, StashEntry,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::Result;
use gix::bstr::ByteSlice as _;
use gix::objs::FindExt as _;
use gix::traverse::commit::simple::CommitTimeOrder;
use rustc_hash::FxHashSet as HashSet;
use std::path::Path;
use std::sync::Arc;

struct CursorGate<'a> {
    last_seen: Option<&'a str>,
    started: bool,
}

impl<'a> CursorGate<'a> {
    fn new(cursor: Option<&'a LogCursor>) -> Self {
        Self {
            last_seen: cursor.map(|cursor| cursor.last_seen.as_ref()),
            started: cursor.is_none(),
        }
    }

    fn should_skip(&mut self, id: &str) -> bool {
        self.should_skip_hex(id)
    }

    fn should_skip_oid(&mut self, id: &gix::oid) -> bool {
        if self.started {
            return false;
        }

        let mut buf = gix::hash::Kind::hex_buf();
        self.should_skip_hex(id.hex_to_buf(&mut buf))
    }

    fn should_skip_hex(&mut self, id: &str) -> bool {
        if self.started {
            return false;
        }

        let Some(last_seen) = self.last_seen else {
            self.started = true;
            return false;
        };

        if last_seen == id {
            self.started = true;
        }

        true
    }
}

fn reflog_lines_rev(
    platform: &mut gix::refs::file::log::iter::Platform<'_, '_>,
    context: &str,
    limit: Option<usize>,
) -> Result<Vec<gix::refs::log::Line>> {
    if limit == Some(0) {
        return Ok(Vec::new());
    }

    let Some(iter) = platform
        .rev()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix reflog {context}: {e}"))))?
    else {
        return Ok(Vec::new());
    };

    let mut lines = Vec::new();
    for line in iter {
        let line =
            line.map_err(|e| Error::new(ErrorKind::Backend(format!("gix reflog {context}: {e}"))))?;
        lines.push(line);
        if let Some(limit) = limit
            && lines.len() >= limit
        {
            break;
        }
    }
    Ok(lines)
}

fn stash_reflog_lines(
    repo: &gix::Repository,
    limit: Option<usize>,
) -> Result<Vec<gix::refs::log::Line>> {
    let Some(reference) = repo.try_find_reference("refs/stash").map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix try_find_reference refs/stash: {e}"
        )))
    })?
    else {
        return Ok(Vec::new());
    };

    let mut platform = reference.log_iter();
    reflog_lines_rev(&mut platform, "refs/stash", limit)
}

pub(super) fn stash_reflog_entries(repo: &gix::Repository) -> Result<Vec<StashEntry>> {
    stash_reflog_lines(repo, None)?
        .into_iter()
        .enumerate()
        .filter(|(_, line)| !line.new_oid.is_null())
        .map(|(index, line)| {
            let created_at = unix_seconds_to_system_time(line.signature.time.seconds);
            Ok(StashEntry {
                index,
                id: CommitId(oid_to_arc_str(&line.new_oid)),
                message: bstr_to_arc_str(line.message.as_ref()),
                created_at,
            })
        })
        .collect()
}

pub(super) fn stash_reflog_tips(
    repo: &gix::Repository,
    limit: usize,
) -> Result<Vec<gix::ObjectId>> {
    let mut tips = Vec::new();
    let mut seen = HashSet::default();
    for line in stash_reflog_lines(repo, Some(limit))? {
        let id = line.new_oid;
        if !id.is_null() && seen.insert(id) {
            tips.push(id);
        }
    }
    Ok(tips)
}

fn reference_commit_id(mut reference: gix::Reference<'_>) -> Result<Option<gix::ObjectId>> {
    let ref_name = reference.name().as_bstr().to_str_lossy().into_owned();
    match reference.peel_to_commit() {
        Ok(commit) => Ok(Some(commit.id().detach())),
        Err(gix::reference::peel::to_kind::Error::PeelObject(
            gix::object::peel::to_kind::Error::NotFound { .. },
        )) => Ok(None),
        Err(e) => Err(Error::new(ErrorKind::Backend(format!(
            "gix peel commit ref {ref_name}: {e}"
        )))),
    }
}

fn commit_from_walk_info(
    info: &gix::revision::walk::Info<'_>,
    decode_state: &mut CommitDecodeState,
) -> Result<Commit> {
    let id = info.id();
    let commit = id
        .repo
        .objects
        .find_commit(id.as_ref(), &mut decode_state.decode_buf)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit object: {e}"))))?;

    let summary_bytes = commit.message.lines().next().unwrap_or_default();
    let summary = bstr_to_arc_str(summary_bytes);

    let author = match commit.author() {
        Ok(s) => decode_state.author_cache.intern(s.name.as_ref()),
        Err(_) => Arc::from("unknown"),
    };

    let seconds = info
        .commit_time
        .unwrap_or_else(|| commit.committer().map(|t| t.seconds()).unwrap_or(0));
    let time = unix_seconds_to_system_time_or_epoch(seconds);

    let commit_id = decode_state
        .next_commit_id_cache
        .reuse_or_new(id.as_ref(), || CommitId(oid_to_arc_str(id.as_ref())));

    let mut parent_ids = CommitParentIds::new();
    parent_ids.reserve(info.parent_ids.len());
    if info.parent_ids.is_empty() {
        decode_state.next_commit_id_cache.clear();
    }
    for (index, parent_id) in info.parent_ids.iter().enumerate() {
        let parent_commit_id = CommitId(oid_to_arc_str(parent_id));
        if index == 0 {
            decode_state
                .next_commit_id_cache
                .remember(parent_id, &parent_commit_id);
        }
        parent_ids.push(parent_commit_id);
    }

    Ok(Commit {
        id: commit_id,
        parent_ids,
        summary,
        author,
        time,
    })
}

#[derive(Default)]
struct CommitDecodeState {
    decode_buf: Vec<u8>,
    author_cache: RepeatedAuthorCache,
    next_commit_id_cache: NextCommitIdCache,
}

#[derive(Default)]
struct RepeatedAuthorCache {
    raw_name: Vec<u8>,
    value: Option<Arc<str>>,
}

impl RepeatedAuthorCache {
    fn intern(&mut self, name: &[u8]) -> Arc<str> {
        if let Some(value) = self.value.as_ref()
            && self.raw_name.as_slice() == name
        {
            return Arc::clone(value);
        }

        self.raw_name.clear();
        self.raw_name.extend_from_slice(name);
        let value = bstr_to_arc_str(name);
        self.value = Some(Arc::clone(&value));
        value
    }
}

#[derive(Default)]
struct NextCommitIdCache {
    raw_id: Vec<u8>,
    value: Option<CommitId>,
}

impl NextCommitIdCache {
    fn reuse_or_new(&self, oid: &gix::oid, make: impl FnOnce() -> CommitId) -> CommitId {
        if let Some(value) = self.value.as_ref()
            && self.raw_id.as_slice() == oid.as_bytes()
        {
            return value.clone();
        }
        make()
    }

    fn remember(&mut self, oid: &gix::oid, value: &CommitId) {
        self.raw_id.clear();
        self.raw_id.extend_from_slice(oid.as_bytes());
        self.value = Some(value.clone());
    }

    fn clear(&mut self) {
        self.raw_id.clear();
        self.value = None;
    }
}

fn commit_file_change_from_diff(
    change: gix::object::tree::diff::ChangeDetached,
) -> Result<Option<CommitFileChange>> {
    use gitcomet_core::domain::FileStatusKind;
    use gix::object::tree::diff::ChangeDetached;

    let (location, is_tree, kind) = match change {
        ChangeDetached::Addition {
            entry_mode,
            location,
            ..
        } => (location, entry_mode.is_tree(), FileStatusKind::Added),
        ChangeDetached::Deletion {
            entry_mode,
            location,
            ..
        } => (location, entry_mode.is_tree(), FileStatusKind::Deleted),
        ChangeDetached::Modification {
            previous_entry_mode,
            entry_mode,
            location,
            ..
        } => (
            location,
            previous_entry_mode.is_tree() || entry_mode.is_tree(),
            FileStatusKind::Modified,
        ),
        ChangeDetached::Rewrite {
            source_entry_mode,
            entry_mode,
            location,
            copy,
            ..
        } => (
            location,
            source_entry_mode.is_tree() || entry_mode.is_tree(),
            if copy {
                FileStatusKind::Added
            } else {
                FileStatusKind::Renamed
            },
        ),
    };

    if is_tree {
        return Ok(None);
    }
    Ok(Some(CommitFileChange {
        path: path_buf_from_git_bytes(location.as_ref(), "gix commit details diff path")?,
        kind,
    }))
}

fn commit_file_changes(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    parent_ids: &[gix::ObjectId],
) -> Result<Vec<CommitFileChange>> {
    if parent_ids.len() > 1 {
        return Ok(Vec::new());
    }

    let commit_tree = commit
        .tree()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit tree: {e}"))))?;
    let parent_tree = parent_ids
        .first()
        .map(|&id| {
            repo.find_commit(id)
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix parent commit: {e}"))))?
                .tree()
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix parent tree: {e}"))))
        })
        .transpose()?;
    let changes = repo
        .diff_tree_to_tree(parent_tree.as_ref(), &commit_tree, None)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix diff_tree_to_tree: {e}"))))?;

    changes
        .into_iter()
        .filter_map(|change| commit_file_change_from_diff(change).transpose())
        .collect()
}

fn empty_log_page() -> LogPage {
    LogPage {
        commits: Vec::new(),
        next_cursor: None,
    }
}

fn object_id_from_commit_id(id: &CommitId) -> Option<gix::ObjectId> {
    gix::ObjectId::from_hex(id.as_ref().as_bytes()).ok()
}

fn log_paged_walk_handle(repo: &gix::ThreadSafeRepository) -> gix::OdbHandleArc {
    gix::odb::memory::Proxy::from(gix::odb::Cache::from(repo.objects.to_handle()))
        .with_write_passthrough()
}

fn new_log_paged_walk(
    repo: &gix::ThreadSafeRepository,
    head_id: gix::ObjectId,
) -> Result<super::LogPagedWalkState> {
    let walk = gix::traverse::commit::Simple::new([head_id], log_paged_walk_handle(repo))
        .sorting(gix::traverse::commit::simple::Sorting::ByCommitTime(
            CommitTimeOrder::NewestFirst,
        ))
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix walk: {e}"))))?;
    Ok(super::LogPagedWalkState {
        pending: None,
        walk,
    })
}

fn apply_first_parent_resume_hint(page: &mut LogPage) {
    if let Some(cursor) = page.next_cursor.as_mut() {
        cursor.resume_from = page
            .commits
            .last()
            .and_then(|commit| commit.parent_ids.first().cloned());
    }
}

fn reflog_unborn_head_error(repo: &gix::Repository) -> Error {
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

fn paginate_commits(
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

fn log_page_from_walk<'repo, E>(
    walk: impl Iterator<Item = std::result::Result<gix::revision::walk::Info<'repo>, E>>,
    limit: usize,
    cursor: Option<&LogCursor>,
) -> Result<LogPage>
where
    E: std::fmt::Display,
{
    let mut decode_state = CommitDecodeState::default();
    let mut cursor_gate = CursorGate::new(cursor);
    let mut commits: Vec<Commit> = Vec::with_capacity(limit);
    let mut next_cursor = None;

    for result in walk {
        let info = result.map_err(|e| Error::new(ErrorKind::Backend(format!("gix walk: {e}"))))?;
        if cursor_gate.should_skip_oid(info.id().as_ref()) {
            continue;
        }

        if commits.len() >= limit {
            next_cursor = commits.last().map(|commit| LogCursor {
                last_seen: commit.id.clone(),
                resume_from: None,
                resume_token: None,
            });
            break;
        }

        commits.push(commit_from_walk_info(&info, &mut decode_state)?);
    }

    Ok(LogPage {
        commits,
        next_cursor,
    })
}

fn log_page_from_walk_filtered<'repo, E>(
    walk: impl Iterator<Item = std::result::Result<gix::revision::walk::Info<'repo>, E>>,
    limit: usize,
    cursor: Option<&LogCursor>,
    mut include: impl FnMut(&gix::revision::walk::Info<'repo>) -> bool,
) -> Result<LogPage>
where
    E: std::fmt::Display,
{
    let mut decode_state = CommitDecodeState::default();
    let mut cursor_gate = CursorGate::new(cursor);
    let mut commits: Vec<Commit> = Vec::with_capacity(limit);
    let mut next_cursor = None;

    for result in walk {
        let info = result.map_err(|e| Error::new(ErrorKind::Backend(format!("gix walk: {e}"))))?;
        if cursor_gate.should_skip_oid(info.id().as_ref()) {
            continue;
        }
        if !include(&info) {
            continue;
        }

        if commits.len() >= limit {
            next_cursor = commits.last().map(|commit| LogCursor {
                last_seen: commit.id.clone(),
                resume_from: None,
                resume_token: None,
            });
            break;
        }

        commits.push(commit_from_walk_info(&info, &mut decode_state)?);
    }

    Ok(LogPage {
        commits,
        next_cursor,
    })
}

fn log_page_from_paged_walk_state(
    repo: &gix::Repository,
    walk_state: &mut super::LogPagedWalkState,
    limit: usize,
    mut cursor_gate: Option<&mut CursorGate<'_>>,
    mut include: impl FnMut(&gix::traverse::commit::Info) -> bool,
) -> Result<(Vec<Commit>, bool)> {
    fn process_paged_walk_info(
        repo: &gix::Repository,
        info: gix::traverse::commit::Info,
        limit: usize,
        commits: &mut Vec<Commit>,
        decode_state: &mut CommitDecodeState,
        cursor_gate: Option<&mut CursorGate<'_>>,
        include: &mut impl FnMut(&gix::traverse::commit::Info) -> bool,
    ) -> Result<Option<gix::traverse::commit::Info>> {
        if let Some(cursor_gate) = cursor_gate
            && cursor_gate.should_skip_oid(info.id.as_ref())
        {
            return Ok(None);
        }
        if !include(&info) {
            return Ok(None);
        }
        if commits.len() >= limit {
            return Ok(Some(info));
        }

        let info = gix::revision::walk::Info::new(info, repo);
        commits.push(commit_from_walk_info(&info, decode_state)?);
        Ok(None)
    }

    let mut decode_state = CommitDecodeState::default();
    let mut commits = Vec::with_capacity(limit);

    if let Some(info) = walk_state.pending.take()
        && let Some(info) = process_paged_walk_info(
            repo,
            info,
            limit,
            &mut commits,
            &mut decode_state,
            cursor_gate.as_deref_mut(),
            &mut include,
        )?
    {
        walk_state.pending = Some(info);
        return Ok((commits, true));
    }

    for result in walk_state.walk.by_ref() {
        let info = result.map_err(|e| Error::new(ErrorKind::Backend(format!("gix walk: {e}"))))?;
        if let Some(info) = process_paged_walk_info(
            repo,
            info,
            limit,
            &mut commits,
            &mut decode_state,
            cursor_gate.as_deref_mut(),
            &mut include,
        )? {
            walk_state.pending = Some(info);
            return Ok((commits, true));
        }
    }

    Ok((commits, false))
}

impl GixRepo {
    fn log_head_page_cache_key(
        mode: HistoryMode,
        head_oid: Option<gix::ObjectId>,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> super::LogHeadPageCacheKey {
        super::LogHeadPageCacheKey {
            mode,
            head_oid,
            limit,
            last_seen: cursor.map(|cursor| cursor.last_seen.clone()),
            resume_from: cursor.and_then(|cursor| cursor.resume_from.clone()),
        }
    }

    fn cached_log_head_page(&self, key: &super::LogHeadPageCacheKey) -> Option<LogPage> {
        let mut cache = self
            .log_head_page_cache
            .lock()
            .expect("log head page cache");
        let index = cache.iter().position(|entry| &entry.key == key)?;
        let entry = cache.remove(index);
        let page = entry.page.clone();
        cache.push(entry);
        Some(page)
    }

    fn store_log_head_page(&self, key: super::LogHeadPageCacheKey, page: &LogPage) {
        let mut cache = self
            .log_head_page_cache
            .lock()
            .expect("log head page cache");
        if let Some(index) = cache.iter().position(|entry| entry.key == key) {
            cache.remove(index);
        }
        if cache.len() >= super::LOG_HEAD_PAGE_CACHE_LIMIT {
            cache.remove(0);
        }
        cache.push(super::LogHeadPageCacheEntry {
            key,
            page: page.clone(),
        });
    }

    fn take_log_paged_walk(
        &self,
        token: &str,
        mode: HistoryMode,
        head_oid: gix::ObjectId,
    ) -> Option<super::LogPagedWalkState> {
        let mut cache = self
            .log_paged_walk_cache
            .lock()
            .expect("log paged walk cache");
        let index = cache.entries.iter().position(|entry| {
            entry.token.as_ref() == token && entry.mode == mode && entry.head_oid == head_oid
        })?;
        Some(cache.entries.remove(index).state)
    }

    fn store_log_paged_walk(
        &self,
        mode: HistoryMode,
        head_oid: gix::ObjectId,
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
        cache.entries.push(super::LogPagedWalkCacheEntry {
            token: Arc::clone(&token),
            mode,
            head_oid,
            state,
        });
        token
    }

    fn log_follow_commits(&self, path: &Path, max_count: Option<usize>) -> Result<Vec<Commit>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("log")
            .arg("--follow")
            .arg("--date=unix")
            .arg("--pretty=format:%H%x1f%P%x1f%an%x1f%ct%x1f%s%x1e");
        if let Some(max_count) = max_count {
            cmd.arg(format!("-n{max_count}"));
        }
        cmd.arg("--").arg(path);

        let output = run_git_capture(cmd, "git log --follow")?;
        Ok(parse_git_log_pretty_records(&output).commits)
    }

    pub(super) fn log_head_page_impl(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        self.log_history_mode_page_impl(HistoryMode::FirstParent, limit, cursor)
    }

    pub(super) fn log_history_mode_page_impl(
        &self,
        mode: HistoryMode,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        if limit == 0 {
            return Ok(empty_log_page());
        }

        if mode == HistoryMode::AllBranches {
            return self.log_all_branches_page_impl(limit, cursor);
        }

        let repo = self._repo.to_thread_local();
        let head_id = gix_head_id_or_none(&repo)?;
        let cache_key = Self::log_head_page_cache_key(mode, head_id, limit, cursor);
        if let Some(page) = self.cached_log_head_page(&cache_key) {
            return Ok(page);
        }

        let page = match mode {
            HistoryMode::FirstParent => {
                if let Some(resume_tip) = cursor
                    .and_then(|cursor| cursor.resume_from.as_ref())
                    .and_then(object_id_from_commit_id)
                {
                    let walk = repo
                        .rev_walk([resume_tip])
                        .sorting(gix::revision::walk::Sorting::ByCommitTime(
                            CommitTimeOrder::NewestFirst,
                        ))
                        .first_parent_only()
                        .all()
                        .map_err(|e| {
                            Error::new(ErrorKind::Backend(format!("gix rev_walk: {e}")))
                        })?;
                    let mut page = log_page_from_walk(walk, limit, None)?;
                    apply_first_parent_resume_hint(&mut page);
                    page
                } else if let Some(head_id) = head_id {
                    let walk = repo
                        .rev_walk([head_id])
                        .sorting(gix::revision::walk::Sorting::ByCommitTime(
                            CommitTimeOrder::NewestFirst,
                        ))
                        .first_parent_only()
                        .all()
                        .map_err(|e| {
                            Error::new(ErrorKind::Backend(format!("gix rev_walk: {e}")))
                        })?;
                    let mut page = log_page_from_walk(walk, limit, cursor)?;
                    apply_first_parent_resume_hint(&mut page);
                    page
                } else {
                    empty_log_page()
                }
            }
            HistoryMode::FullReachable | HistoryMode::NoMerges | HistoryMode::MergesOnly => {
                let Some(head_id) = head_id else {
                    return Ok(empty_log_page());
                };
                if repo.is_shallow() {
                    let walk = repo
                        .rev_walk([head_id])
                        .sorting(gix::revision::walk::Sorting::ByCommitTime(
                            CommitTimeOrder::NewestFirst,
                        ))
                        .all()
                        .map_err(|e| {
                            Error::new(ErrorKind::Backend(format!("gix rev_walk: {e}")))
                        })?;
                    match mode {
                        HistoryMode::FullReachable => log_page_from_walk(walk, limit, cursor)?,
                        HistoryMode::NoMerges => {
                            log_page_from_walk_filtered(walk, limit, cursor, |info| {
                                info.parent_ids.len() < 2
                            })?
                        }
                        HistoryMode::MergesOnly => {
                            log_page_from_walk_filtered(walk, limit, cursor, |info| {
                                info.parent_ids.len() > 1
                            })?
                        }
                        HistoryMode::FirstParent | HistoryMode::AllBranches => unreachable!(),
                    }
                } else {
                    let cached_walk_state = cursor
                        .and_then(|cursor| cursor.resume_token.as_deref())
                        .and_then(|token| self.take_log_paged_walk(token, mode, head_id));
                    let mut cursor_gate = cursor
                        .filter(|_| cached_walk_state.is_none())
                        .map(|cursor| CursorGate::new(Some(cursor)));
                    let mut walk_state = if let Some(walk_state) = cached_walk_state {
                        walk_state
                    } else {
                        // Opaque tokens can go stale after cache eviction or head changes.
                        // Fall back to `last_seen` semantics by rebuilding the skip gate.
                        new_log_paged_walk(&self._repo, head_id)?
                    };
                    let (commits, has_more) = log_page_from_paged_walk_state(
                        &repo,
                        &mut walk_state,
                        limit,
                        cursor_gate.as_mut(),
                        |info| match mode {
                            HistoryMode::FullReachable => true,
                            HistoryMode::NoMerges => info.parent_ids.len() < 2,
                            HistoryMode::MergesOnly => info.parent_ids.len() > 1,
                            HistoryMode::FirstParent | HistoryMode::AllBranches => unreachable!(),
                        },
                    )?;
                    let next_cursor = if has_more {
                        commits.last().map(|commit| LogCursor {
                            last_seen: commit.id.clone(),
                            resume_from: None,
                            resume_token: Some(
                                self.store_log_paged_walk(mode, head_id, walk_state),
                            ),
                        })
                    } else {
                        None
                    };
                    LogPage {
                        commits,
                        next_cursor,
                    }
                }
            }
            HistoryMode::AllBranches => unreachable!(),
        };

        self.store_log_head_page(cache_key, &page);
        Ok(page)
    }

    pub(super) fn log_all_branches_page_impl(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        if limit == 0 {
            return Ok(empty_log_page());
        }

        let repo = self._repo.to_thread_local();
        let head_id = gix_head_id_or_none(&repo)?;

        let refs = repo
            .references()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references: {e}"))))?;

        // Emulate `git log --all`: include all refs under `refs/`, not just `refs/heads` and
        // `refs/remotes`. Some repositories (e.g. Chromium) use additional namespaces like
        // `refs/branch-heads/*`.
        let mut tips = Vec::new();
        let mut seen = HashSet::default();
        if let Some(head_id) = head_id {
            tips.push(head_id);
            seen.insert(head_id);
        }

        let iter = refs
            .all()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references(all): {e}"))))?;
        for reference in iter {
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

        // `git log --all` includes only `refs/stash` tip, but users expect history scope=all
        // to also surface older stash entries (reflog-backed). Add stash reflog commits as extra
        // walk tips so stash rows can be rendered consistently in history graph.
        for id in stash_reflog_tips(&repo, 50).unwrap_or_default() {
            if seen.insert(id) {
                tips.push(id);
            }
        }

        if tips.is_empty() {
            return Ok(empty_log_page());
        }

        let walk = repo
            .rev_walk(tips)
            .sorting(gix::revision::walk::Sorting::ByCommitTime(
                CommitTimeOrder::NewestFirst,
            ))
            .all()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix rev_walk: {e}"))))?;
        log_page_from_walk(walk, limit, cursor)
    }

    pub(super) fn log_file_page_impl(
        &self,
        path: &Path,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        if limit == 0 {
            return Ok(empty_log_page());
        }

        // Only the first page is bounded. `git log --follow` does not combine
        // reliably with `--skip` across renames, so cursor pages still need to
        // scan the full follow history and paginate it in-process.
        let max_count = cursor.is_none().then_some(limit.saturating_add(1));
        let commits = self.log_follow_commits(path, max_count)?;
        paginate_commits(commits.into_iter().map(Ok), limit, cursor)
    }

    pub(super) fn commit_details_impl(&self, id: &CommitId) -> Result<CommitDetails> {
        let repo = self._repo.to_thread_local();
        let spec = id.as_ref();
        let commit = repo
            .rev_parse_single(spec)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix rev-parse {spec}: {e}"))))?
            .object()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit object {spec}: {e}"))))?
            .peel_to_commit()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel commit {spec}: {e}"))))?;

        let message = bytes_to_text_preserving_utf8(commit.message_raw_sloppy().as_ref())
            .trim_end()
            .to_string();
        let committed_at = commit
            .time()
            .map(|time| time.format_or_unix(gix::date::time::format::ISO8601_STRICT))
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit time {spec}: {e}"))))?;
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
            committed_at,
            parent_ids,
            files,
        })
    }

    pub(super) fn reflog_head_impl(&self, limit: usize) -> Result<Vec<ReflogEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let repo = self._repo.to_thread_local();
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
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_gate_skips_until_after_last_seen() {
        let cursor = LogCursor {
            last_seen: CommitId("c2".into()),
            resume_from: None,
            resume_token: None,
        };
        let mut gate = CursorGate::new(Some(&cursor));

        assert!(gate.should_skip("c1"));
        assert!(gate.should_skip("c2"));
        assert!(!gate.should_skip("c3"));
        assert!(!gate.should_skip("c4"));
    }

    #[test]
    fn object_id_from_commit_id_rejects_invalid_hex() {
        assert!(object_id_from_commit_id(&CommitId("not-a-sha".into())).is_none());
    }

    #[test]
    fn apply_first_parent_resume_hint_uses_first_parent_of_last_commit() {
        let mut page = LogPage {
            commits: vec![
                Commit {
                    id: CommitId("c1".into()),
                    parent_ids: CommitParentIds::from_vec(vec![CommitId("p0".into())]),
                    summary: Arc::from("one"),
                    author: Arc::from("you"),
                    time: std::time::SystemTime::UNIX_EPOCH,
                },
                Commit {
                    id: CommitId("c2".into()),
                    parent_ids: CommitParentIds::from_vec(vec![
                        CommitId("p1".into()),
                        CommitId("p2".into()),
                    ]),
                    summary: Arc::from("two"),
                    author: Arc::from("you"),
                    time: std::time::SystemTime::UNIX_EPOCH,
                },
            ],
            next_cursor: Some(LogCursor {
                last_seen: CommitId("c2".into()),
                resume_from: None,
                resume_token: None,
            }),
        };

        apply_first_parent_resume_hint(&mut page);

        assert_eq!(
            page.next_cursor
                .as_ref()
                .and_then(|cursor| cursor.resume_from.clone()),
            Some(CommitId("p1".into()))
        );
    }

    #[test]
    fn apply_first_parent_resume_hint_clears_stale_resume_hint_when_no_parent_exists() {
        let mut page = LogPage {
            commits: vec![Commit {
                id: CommitId("c1".into()),
                parent_ids: CommitParentIds::new(),
                summary: Arc::from("one"),
                author: Arc::from("you"),
                time: std::time::SystemTime::UNIX_EPOCH,
            }],
            next_cursor: Some(LogCursor {
                last_seen: CommitId("c1".into()),
                resume_from: Some(CommitId("stale".into())),
                resume_token: None,
            }),
        };

        apply_first_parent_resume_hint(&mut page);

        assert_eq!(
            page.next_cursor.as_ref().expect("next cursor").resume_from,
            None
        );
    }

    #[test]
    fn repeated_author_cache_reuses_arc_for_identical_names() {
        let mut cache = RepeatedAuthorCache::default();

        let first = cache.intern(b"Bench");
        let second = cache.intern(b"Bench");
        let third = cache.intern(b"Other");

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&second, &third));
    }

    #[test]
    fn next_commit_id_cache_reuses_commit_id_for_matching_first_parent() {
        let mut cache = NextCommitIdCache::default();

        let parent = CommitId(Arc::from("1111111111111111111111111111111111111111"));
        let oid = gix::ObjectId::from_hex(parent.as_ref().as_bytes()).expect("valid oid");
        cache.remember(oid.as_ref(), &parent);

        let reused = cache.reuse_or_new(oid.as_ref(), || CommitId(Arc::from("other")));
        let other_oid = gix::ObjectId::from_hex(b"2222222222222222222222222222222222222222")
            .expect("valid oid");
        let fresh = cache.reuse_or_new(other_oid.as_ref(), || CommitId(Arc::from("fresh")));

        assert!(Arc::ptr_eq(&parent.0, &reused.0));
        assert_eq!(fresh.as_ref(), "fresh");
    }
}
