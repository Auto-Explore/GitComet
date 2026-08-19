use super::history::gix_head_id_or_none;
use super::{GixRepo, bstr_to_arc_str, oid_to_arc_str};
use crate::util::{
    bytes_to_text_preserving_utf8, parse_git_log_pretty_records_from_reader,
    path_buf_from_git_bytes, run_git_capture, run_git_parsed_stdout, unix_seconds_to_system_time,
    unix_seconds_to_system_time_or_epoch,
};
use gitcomet_core::domain::{
    Commit, CommitDetails, CommitFileChange, CommitId, CommitParentIds, EMPTY_TREE_ID, HistoryMode,
    LogCursor, LogPage, RecentCommitMessage, ReflogEntry, StashEntry,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::{CancellationToken, LogChunk, Result};
use gix::bstr::ByteSlice as _;
use gix::objs::FindExt as _;
use gix::traverse::commit::simple::CommitTimeOrder;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const RECENT_COMMIT_MESSAGES_MAX_LIMIT: usize = 100;

fn recent_commit_message_limits(limit: usize) -> Option<(usize, usize)> {
    let limit = limit.min(RECENT_COMMIT_MESSAGES_MAX_LIMIT);
    if limit == 0 {
        return None;
    }

    let scan_limit = limit
        .saturating_mul(5)
        .min(RECENT_COMMIT_MESSAGES_MAX_LIMIT)
        .max(limit);
    Some((limit, scan_limit))
}

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

    let mut lines = Vec::with_capacity(limit.unwrap_or(0));
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
    let mut tips = Vec::with_capacity(limit);
    let mut seen = FxHashSet::with_capacity_and_hasher(limit, Default::default());
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

/// A normalized author filter.
///
/// Normalizing once, here, is what keeps the needle, the head-page cache key and
/// the paged-walk cache key spelling the same filter the same way: a cache hit
/// that disagreed with the walk cache would hand back a resume token the walk
/// cache then rejects, turning an O(1) resume into a fresh walk of the history.
///
/// Folding is ASCII-only, matching the author picker in the UI, so a name picked
/// from that list is exactly the name the walk looks for. The needle must be
/// folded the same way it is compared — an `str::to_lowercase` needle tested
/// with `eq_ignore_ascii_case` cannot match its own name back, because a
/// Unicode-lowercased 'Á' never equals the 'Á' it came from.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct AuthorFilter(String);

impl AuthorFilter {
    /// `None` for "every author" — which an all-whitespace filter also means.
    fn new(author: Option<&str>) -> Option<Self> {
        let author = author?.trim();
        (!author.is_empty()).then(|| Self(author.to_ascii_lowercase()))
    }

    /// Case-insensitive substring match against an author name. Allocation-free,
    /// because this runs once per visited commit.
    fn matches(&self, name: &[u8]) -> bool {
        let needle = self.0.as_bytes();
        name.len() >= needle.len()
            && name
                .windows(needle.len())
                .any(|window| window.eq_ignore_ascii_case(needle))
    }
}

/// Decodes one commit, or returns `None` when `author_filter` rejects it.
///
/// The author is read straight off the decoded object and tested *before*
/// anything is built from it, so a commit the filter rejects costs one object
/// read and no allocation. On a repository the size of Chromium a filtered page
/// visits every one of ~1.8M commits, so what the rejected ones cost is what
/// the whole operation costs.
///
/// Takes the walk's fields rather than its `Info`, so the decoders can be handed
/// a batch to split between them without cloning the parent ids of every commit
/// visited.
fn commit_from_walk_parts(
    repo: &gix::Repository,
    id: &gix::oid,
    parent_ids: &[gix::ObjectId],
    commit_time: Option<gix::date::SecondsSinceUnixEpoch>,
    decode_state: &mut CommitDecodeState,
    author_filter: Option<&AuthorFilter>,
) -> Result<Option<Commit>> {
    let commit = repo
        .objects
        .find_commit(id, &mut decode_state.decode_buf)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit object: {e}"))))?;

    let author_name = commit.author().map(|author| author.name).ok();
    if let Some(filter) = author_filter
        && !author_name.is_some_and(|name| filter.matches(name.as_ref()))
    {
        return Ok(None);
    }

    let summary_bytes = commit.message.lines().next().unwrap_or_default();
    let summary = bstr_to_arc_str(summary_bytes);

    let author = match author_name {
        Some(name) => decode_state.author_cache.intern(name.as_ref()),
        None => Arc::from("unknown"),
    };

    let seconds =
        commit_time.unwrap_or_else(|| commit.committer().map(|t| t.seconds()).unwrap_or(0));
    let time = unix_seconds_to_system_time_or_epoch(seconds);

    let commit_id = decode_state
        .next_commit_id_cache
        .reuse_or_new(id, || CommitId(oid_to_arc_str(id)));

    let mut ids = CommitParentIds::new();
    ids.reserve(parent_ids.len());
    if parent_ids.is_empty() {
        decode_state.next_commit_id_cache.clear();
    }
    for (index, parent_id) in parent_ids.iter().enumerate() {
        let parent_commit_id = CommitId(oid_to_arc_str(parent_id));
        if index == 0 {
            decode_state
                .next_commit_id_cache
                .remember(parent_id, &parent_commit_id);
        }
        ids.push(parent_commit_id);
    }

    Ok(Some(Commit {
        id: commit_id,
        parent_ids: ids,
        summary,
        author,
        time,
    }))
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

/// Per-file line stats are skipped entirely for commits touching more files
/// than this, so pathological commits can't stall the details panel.
const COMMIT_STATS_MAX_FILES: usize = 400;
/// Blobs larger than this are treated as "stats unknown" instead of diffed.
const COMMIT_STATS_MAX_BLOB_BYTES: usize = 4 * 1024 * 1024;
/// Git's binary heuristic: a NUL byte within the leading window.
const COMMIT_STATS_BINARY_SNIFF_BYTES: usize = 8000;

fn commit_stats_blob_bytes(repo: &gix::Repository, id: Option<gix::ObjectId>) -> Option<Vec<u8>> {
    let Some(id) = id.filter(|id| !id.is_null()) else {
        // No blob on this side (pure addition/deletion) diffs as empty content.
        return Some(Vec::new());
    };
    let object = repo.find_object(id).ok()?;
    if object.kind != gix::object::Kind::Blob {
        return None;
    }
    Some(object.detach().data)
}

fn commit_stats_looks_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(COMMIT_STATS_BINARY_SNIFF_BYTES)].contains(&0)
}

fn commit_stats_line_count(bytes: &[u8]) -> u32 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count();
    let trailing = usize::from(*bytes.last().expect("checked non-empty") != b'\n');
    u32::try_from(newlines + trailing).unwrap_or(u32::MAX)
}

/// Added/removed line counts between two blob versions; `(None, None)` when
/// either side is binary, too large, or unreadable.
fn commit_file_line_stats(
    repo: &gix::Repository,
    old_id: Option<gix::ObjectId>,
    new_id: Option<gix::ObjectId>,
) -> (Option<u32>, Option<u32>) {
    let Some(old) = commit_stats_blob_bytes(repo, old_id) else {
        return (None, None);
    };
    let Some(new) = commit_stats_blob_bytes(repo, new_id) else {
        return (None, None);
    };
    if old.len() > COMMIT_STATS_MAX_BLOB_BYTES
        || new.len() > COMMIT_STATS_MAX_BLOB_BYTES
        || commit_stats_looks_binary(&old)
        || commit_stats_looks_binary(&new)
    {
        return (None, None);
    }

    // One side empty means every line of the other side changed; skip the diff.
    if old.is_empty() || new.is_empty() {
        return (
            Some(commit_stats_line_count(&new)),
            Some(commit_stats_line_count(&old)),
        );
    }

    use gix::diff::blob::InternedInput;
    let input = InternedInput::new(old.as_slice(), new.as_slice());
    let diff = gix::diff::blob::Diff::compute(gix::diff::blob::Algorithm::Histogram, &input);
    (Some(diff.count_additions()), Some(diff.count_removals()))
}

fn commit_file_change_from_diff(
    repo: &gix::Repository,
    change: gix::object::tree::diff::ChangeDetached,
    compute_stats: bool,
) -> Result<Option<CommitFileChange>> {
    use gitcomet_core::domain::FileStatusKind;
    use gix::object::tree::diff::ChangeDetached;

    let (location, is_tree, is_submodule, kind, old_id, new_id) = match change {
        ChangeDetached::Addition {
            entry_mode,
            location,
            id,
            ..
        } => (
            location,
            entry_mode.is_tree(),
            entry_mode.is_commit(),
            FileStatusKind::Added,
            None,
            Some(id),
        ),
        ChangeDetached::Deletion {
            entry_mode,
            location,
            id,
            ..
        } => (
            location,
            entry_mode.is_tree(),
            entry_mode.is_commit(),
            FileStatusKind::Deleted,
            Some(id),
            None,
        ),
        ChangeDetached::Modification {
            previous_entry_mode,
            entry_mode,
            location,
            previous_id,
            id,
        } => (
            location,
            previous_entry_mode.is_tree() || entry_mode.is_tree(),
            previous_entry_mode.is_commit() || entry_mode.is_commit(),
            FileStatusKind::Modified,
            Some(previous_id),
            Some(id),
        ),
        ChangeDetached::Rewrite {
            source_entry_mode,
            entry_mode,
            location,
            copy,
            source_id,
            id,
            ..
        } => (
            location,
            source_entry_mode.is_tree() || entry_mode.is_tree(),
            source_entry_mode.is_commit() || entry_mode.is_commit(),
            if copy {
                FileStatusKind::Added
            } else {
                FileStatusKind::Renamed
            },
            Some(source_id),
            Some(id),
        ),
    };

    if is_tree {
        return Ok(None);
    }

    let (additions, deletions) = if compute_stats && !is_submodule {
        commit_file_line_stats(repo, old_id, new_id)
    } else {
        (None, None)
    };

    Ok(Some(CommitFileChange {
        path: path_buf_from_git_bytes(location.as_ref(), "gix commit details diff path")?,
        kind,
        is_submodule,
        additions,
        deletions,
    }))
}

/// Diff two trees (an absent `old_tree` means an empty tree, i.e. every path in
/// `new_tree` is an addition) into the flat `CommitFileChange` list used by both
/// commit details (parent → commit) and range comparisons (from → to).
fn tree_diff_file_changes(
    repo: &gix::Repository,
    old_tree: Option<&gix::Tree<'_>>,
    new_tree: &gix::Tree<'_>,
) -> Result<Vec<CommitFileChange>> {
    let changes = repo
        .diff_tree_to_tree(old_tree, new_tree, None)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix diff_tree_to_tree: {e}"))))?;

    let compute_stats = changes.len() <= COMMIT_STATS_MAX_FILES;
    changes
        .into_iter()
        .filter_map(|change| commit_file_change_from_diff(repo, change, compute_stats).transpose())
        .collect()
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

    tree_diff_file_changes(repo, parent_tree.as_ref(), &commit_tree)
}

/// List the files that differ between two commits (`from` → `to`), for the
/// compare-selected-commits feature. `from` is the base/older side.
pub(crate) fn diff_range_files(
    repo: &gix::Repository,
    from: &CommitId,
    to: &CommitId,
) -> Result<Vec<CommitFileChange>> {
    // An absent base already means "no content" to the tree diff, which is
    // exactly what the empty tree stands for — so resolve it as absence rather
    // than through the object database, which is not guaranteed to hold it.
    let from_tree = (from.as_ref() != EMPTY_TREE_ID)
        .then(|| commit_tree_for_id(repo, from, "gix range from"))
        .transpose()?;
    let to_tree = commit_tree_for_id(repo, to, "gix range to")?;
    tree_diff_file_changes(repo, from_tree.as_ref(), &to_tree)
}

/// Resolve a comparison endpoint to the tree it names. Peels to a tree rather
/// than to a commit so a bare tree spec resolves too — the empty tree is how the
/// changes a root commit introduces are expressed, and it is not a commit.
fn commit_tree_for_id<'repo>(
    repo: &'repo gix::Repository,
    id: &CommitId,
    context: &str,
) -> Result<gix::Tree<'repo>> {
    let spec = id.as_ref();
    repo.rev_parse_single(spec)
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "{context} rev-parse {spec}: {e}"
            )))
        })?
        .object()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("{context} object {spec}: {e}"))))?
        .peel_to_tree()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("{context} peel {spec}: {e}"))))
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

/// The commit filter a paged walk over `repo` needs.
///
/// On a shallow repository the boundary commits record parents that are not in
/// the object database; they have to be skipped or the traversal walks off the
/// end of what was cloned. `repo.rev_walk(..)` installs exactly this, but its
/// filter borrows the repository, and a walk parked in the walk cache outlives
/// any such borrow — hence the owned handle and the boxed closure.
fn log_paged_walk_filter(repo: &gix::ThreadSafeRepository) -> Result<super::LogPagedWalkFilter> {
    let shallow_commits = repo
        .to_thread_local()
        .shallow_commits()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix shallow commits: {e}"))))?;
    let Some(shallow_commits) = shallow_commits else {
        return Ok(Box::new(|_| true));
    };

    let objects = log_paged_walk_handle(repo);
    let mut grafted_parents_to_skip: Vec<gix::ObjectId> = Vec::new();
    let mut buf = Vec::new();
    Ok(Box::new(move |id| {
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
    }))
}

/// A resumable walk of `mode` seeded from `tips`.
fn new_log_paged_walk(
    repo: &gix::ThreadSafeRepository,
    tips: impl IntoIterator<Item = gix::ObjectId>,
    mode: HistoryMode,
) -> Result<super::LogPagedWalkState> {
    // Without the commit-graph the traversal decodes every commit object just to
    // read its parents and date, and the page builder then decodes the same
    // objects again — two inflates per commit across the whole history on a
    // filtered walk. `repo.rev_walk(..)` wires the graph up by default; this
    // walk is built from `Simple` directly, so it has to ask for it.
    let commit_graph = repo
        .to_thread_local()
        .commit_graph_if_enabled()
        .ok()
        .flatten();
    let walk = gix::traverse::commit::Simple::filtered(
        tips,
        log_paged_walk_handle(repo),
        log_paged_walk_filter(repo)?,
    )
    .sorting(gix::traverse::commit::simple::Sorting::ByCommitTime(
        CommitTimeOrder::NewestFirst,
    ))
    .map_err(|e| Error::new(ErrorKind::Backend(format!("gix walk: {e}"))))?
    // Set after the sorting, the way `rev_walk` does: first-parent mode walks the
    // chain in order rather than by date, and asking for it swaps the queue.
    .parents(if mode == HistoryMode::FirstParent {
        gix::traverse::commit::Parents::First
    } else {
        gix::traverse::commit::Parents::All
    })
    .commit_graph(commit_graph);
    Ok(super::LogPagedWalkState {
        pending: std::collections::VecDeque::new(),
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

/// Reports a page as it is built, throttled so a walk that runs for seconds
/// updates the caller a handful of times a second instead of per commit.
pub(super) struct ChunkEmitter<'a> {
    on_chunk: &'a mut dyn FnMut(LogChunk),
    next_emit_at: std::time::Instant,
    scanned: u64,
}

const CHUNK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(120);
/// How often the clock is consulted. Reading it per commit would cost more than
/// the throttling saves on a million-commit walk; a thousand commits is well
/// under a millisecond of work, far below the emit interval. Kept equal to
/// [`DECODE_BATCH`] so a batched caller consults the clock once per batch.
const CHUNK_CLOCK_STRIDE: u64 = DECODE_BATCH as u64;

impl<'a> ChunkEmitter<'a> {
    pub(super) fn new(on_chunk: &'a mut dyn FnMut(LogChunk)) -> Self {
        Self {
            on_chunk,
            next_emit_at: std::time::Instant::now() + CHUNK_INTERVAL,
            scanned: 0,
        }
    }

    /// Counts `count` more visited commits and reports the page so far once the
    /// interval has elapsed — including when nothing new matched, so a filter
    /// that is finding nothing still shows that it is working.
    ///
    /// Callers that count one commit at a time only reach the clock once per
    /// stride; a batched caller passes a whole batch, which is the stride, so
    /// its quotient always moves and every batch consults the clock.
    fn visited(&mut self, count: u64, commits: &[Commit]) {
        let before = self.scanned;
        self.scanned += count;
        if before / CHUNK_CLOCK_STRIDE == self.scanned / CHUNK_CLOCK_STRIDE
            || std::time::Instant::now() < self.next_emit_at
        {
            return;
        }
        self.next_emit_at = std::time::Instant::now() + CHUNK_INTERVAL;
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
fn mode_includes(mode: HistoryMode, parent_count: usize) -> bool {
    match mode {
        HistoryMode::FullReachable | HistoryMode::FirstParent | HistoryMode::AllBranches => true,
        HistoryMode::NoMerges => parent_count < 2,
        HistoryMode::MergesOnly => parent_count > 1,
    }
}

/// Commits handed to one round of decoding.
///
/// A page that fills mid-batch parks the rest of that batch in
/// [`super::LogPagedWalkState::pending`], and the walk cache holds it until the
/// walk is resumed or evicted — up to `LOG_PAGED_WALK_CACHE_LIMIT` walks at 72
/// bytes an entry, so the batch size is what bounds that. The rounds can be this
/// small because [`DecodeWorkers`] outlive them: the per-round cost is a thread
/// spawn and join, not a repository handle and fresh inflate buffers.
///
/// Measured on the 100k-commit rare-author benchmark: 8192 costs ~472ms and
/// retains up to ~19MB, 2048 costs ~481ms and retains up to ~4.7MB, 1024 costs
/// ~491ms. 2% for a quarter of the worst-case retention is the trade taken here.
const DECODE_BATCH: usize = 2_048;
/// Commit-decode threads in flight across the whole process. Object inflation is
/// what a filtered walk spends its time on and it parallelizes cleanly, but the
/// budget is shared: several repositories loading at once must not multiply into
/// as many decode threads as they have walks.
const DECODE_THREADS_MAX: usize = 8;
/// Below this a batch decodes on the calling thread alone — the spawn and join
/// round trip costs more than it saves.
const DECODE_PARALLEL_MIN: usize = 256;
static DECODE_THREADS_IN_USE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Helper threads claimed out of [`DECODE_THREADS_MAX`] for one page build,
/// released on drop. A page that gets none still decodes, just on the thread
/// building it.
struct DecodeThreadBudget(usize);

impl DecodeThreadBudget {
    fn claim() -> Self {
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
struct DecodeWorkers {
    _budget: DecodeThreadBudget,
    workers: Vec<(gix::Repository, CommitDecodeState)>,
}

impl DecodeWorkers {
    fn new(repo: &gix::ThreadSafeRepository) -> Self {
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
    fn decode(
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
fn log_page_from_paged_walk_state(
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

    while !walk_done {
        // Gathering ids is the cheap half — with a commit-graph it touches no
        // objects at all — so the gate and the mode predicate run here, before
        // anything is handed to the decoders.
        batch.clear();
        while batch.len() < DECODE_BATCH {
            let Some(info) = walk_state.pending.pop_front() else {
                // Checked per commit walked, not per batch: a mode predicate or
                // a cursor gate that rejects everything can traverse an entire
                // history without filling one batch, and a superseded walk that
                // cannot be stopped holds a repo-load thread for all of it.
                if let Some(cancellation) = cancellation {
                    cancellation.check_cancelled()?;
                }
                match walk_state.walk.next() {
                    None => {
                        walk_done = true;
                        break;
                    }
                    Some(result) => {
                        let info = result.map_err(|e| {
                            Error::new(ErrorKind::Backend(format!("gix walk: {e}")))
                        })?;
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
                    }
                }
            };
            batch.push(info);
        }

        if batch.is_empty() {
            break;
        }

        workers.decode(&batch, author, &mut decoded)?;
        let scanned = batch.len() as u64;

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
                return Ok((commits, true));
            }
            commits.push(commit);
        }

        // Reported after the batch lands, so a chunk always carries everything
        // found so far rather than trailing a batch behind.
        if let Some(chunks) = chunks.as_deref_mut() {
            chunks.visited(scanned, &commits);
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
        author: Option<&AuthorFilter>,
    ) -> super::LogHeadPageCacheKey {
        super::LogHeadPageCacheKey {
            mode,
            head_oid,
            limit,
            last_seen: cursor.map(|cursor| cursor.last_seen.clone()),
            resume_from: cursor.and_then(|cursor| cursor.resume_from.clone()),
            author: author.cloned(),
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

    fn log_file_follow_cache_key(
        path: &Path,
        head_oid: Option<gix::ObjectId>,
    ) -> super::LogFileFollowCacheKey {
        super::LogFileFollowCacheKey {
            head_oid,
            path: path.to_path_buf(),
        }
    }

    fn cached_log_file_follow_commits(
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

    fn store_log_file_follow_commits(
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

    fn take_log_paged_walk(
        &self,
        token: &str,
        mode: HistoryMode,
        tips: &[gix::ObjectId],
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
                && entry.author.as_ref() == author
        })?;
        Some(cache.entries.remove(index).state)
    }

    fn store_log_paged_walk(
        &self,
        mode: HistoryMode,
        tips: &Arc<[gix::ObjectId]>,
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
        cache.entries.push(super::LogPagedWalkCacheEntry {
            token: Arc::clone(&token),
            mode,
            tips: Arc::clone(tips),
            author: author.cloned(),
            state,
        });
        token
    }

    pub(super) fn resolve_file_path_at_commit_impl(
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
    fn path_exists_in_commit_tree(&self, commit: &CommitId, path: &Path) -> bool {
        let repo = self._repo.to_thread_local();
        let Ok(id) = repo.rev_parse_single(commit.as_ref()) else {
            return false;
        };
        let Ok(object) = id.object() else {
            return false;
        };
        let Ok(tree) = object.peel_to_tree() else {
            return false;
        };
        matches!(tree.lookup_entry_by_path(path), Ok(Some(_)))
    }

    /// Find the file's name in `commit`'s tree by following renames from `path`.
    /// Runs `git log --follow --name-status` and reads the entry for `commit`:
    /// a rename yields its destination; a plain change yields its path; a
    /// deletion (the followed name was renamed away at `commit`) is resolved to
    /// the rename's destination via `git diff-tree -M`.
    fn resolve_renamed_path_at_commit(
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
    fn interpret_name_status_for_commit(
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
    fn first_parent_id(&self, commit: &CommitId) -> Option<String> {
        let repo = self._repo.to_thread_local();
        let id = repo.rev_parse_single(commit.as_ref()).ok()?.detach();
        let commit = repo.find_commit(id).ok()?;
        commit.parent_ids().next().map(|parent| parent.to_string())
    }

    /// The destination path of a rename of `old_path` introduced by `commit`,
    /// using rename detection against its parent.
    fn rename_destination_at_commit(
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

        run_git_parsed_stdout(cmd, "git log --follow", false, |stdout| {
            parse_git_log_pretty_records_from_reader(stdout).map(|page| page.commits)
        })
    }

    pub(super) fn log_head_page_impl(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        self.log_history_mode_page_impl(HistoryMode::FirstParent, limit, cursor)
    }

    pub(super) fn log_head_page_cancellable_impl(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<LogPage> {
        self.log_history_mode_page_cancellable_impl(
            HistoryMode::FirstParent,
            limit,
            cursor,
            cancellation,
        )
    }

    pub(super) fn log_history_mode_page_impl(
        &self,
        mode: HistoryMode,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        self.log_history_mode_page_impl_inner(mode, None, limit, cursor, None, None)
    }

    pub(super) fn log_history_mode_page_cancellable_impl(
        &self,
        mode: HistoryMode,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<LogPage> {
        self.log_history_mode_page_impl_inner(mode, None, limit, cursor, Some(cancellation), None)
    }

    /// Filtered, cancellable, streaming variant: `on_chunk` sees the page as it
    /// fills in. The one entry point the app uses — the plain variants above
    /// exist for callers with no filter and nothing to cancel. See
    /// [`gitcomet_core::services::GitRepository::log_history_mode_page_streaming`].
    pub(super) fn log_history_mode_page_streaming_impl(
        &self,
        mode: HistoryMode,
        author: Option<&str>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
        on_chunk: &mut dyn FnMut(LogChunk),
    ) -> Result<LogPage> {
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
    fn log_paged_page(
        &self,
        mode: HistoryMode,
        tips: Arc<[gix::ObjectId]>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: Option<&CancellationToken>,
        author: Option<&AuthorFilter>,
        chunks: Option<&mut ChunkEmitter<'_>>,
    ) -> Result<LogPage> {
        if tips.is_empty() {
            return Ok(empty_log_page());
        }

        let cached_walk_state = cursor
            .and_then(|cursor| cursor.resume_token.as_deref())
            .and_then(|token| self.take_log_paged_walk(token, mode, &tips, author));

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
            (None, Some(resume_tip)) => {
                (new_log_paged_walk(&self._repo, [resume_tip], mode)?, None)
            }
            (None, None) => (
                new_log_paged_walk(&self._repo, tips.iter().copied(), mode)?,
                cursor.map(|cursor| CursorGate::new(Some(cursor))),
            ),
        };

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
                resume_token: Some(self.store_log_paged_walk(mode, &tips, author, walk_state)),
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

    fn log_history_mode_page_impl_inner(
        &self,
        mode: HistoryMode,
        author: Option<&str>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: Option<&CancellationToken>,
        mut chunks: Option<&mut ChunkEmitter<'_>>,
    ) -> Result<LogPage> {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        if limit == 0 {
            return Ok(empty_log_page());
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

        let repo = self._repo.to_thread_local();
        let head_id = gix_head_id_or_none(&repo)?;
        let cache_key = Self::log_head_page_cache_key(mode, head_id, limit, cursor, author);
        if let Some(page) = self.cached_log_head_page(&cache_key) {
            return Ok(page);
        }

        let page = match head_id {
            Some(head_id) => self.log_paged_page(
                mode,
                Arc::from(vec![head_id]),
                limit,
                cursor,
                cancellation,
                author,
                chunks,
            )?,
            None => empty_log_page(),
        };

        self.store_log_head_page(cache_key, &page);
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        Ok(page)
    }

    pub(super) fn log_all_branches_page_impl(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        self.log_all_branches_page_impl_inner(limit, cursor, None, None, None)
    }

    pub(super) fn log_all_branches_page_cancellable_impl(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<LogPage> {
        self.log_all_branches_page_impl_inner(limit, cursor, Some(cancellation), None, None)
    }

    fn log_all_branches_page_impl_inner(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: Option<&CancellationToken>,
        author: Option<&AuthorFilter>,
        chunks: Option<&mut ChunkEmitter<'_>>,
    ) -> Result<LogPage> {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
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
        let mut seen = FxHashSet::default();
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

        // `git log --all` includes only `refs/stash` tip, but users expect history scope=all
        // to also surface older stash entries (reflog-backed). Add stash reflog commits as extra
        // walk tips so stash rows can be rendered consistently in history graph.
        for id in stash_reflog_tips(&repo, 50).unwrap_or_default() {
            if seen.insert(id) {
                tips.push(id);
            }
        }

        // The tips identify the walk in the walk cache, so they have to come out
        // the same way each page: ref enumeration order is not guaranteed, and a
        // reshuffled list would reject a perfectly good resume token. Sorting
        // costs the walk nothing — every tip is just a seed, and the traversal
        // orders what it yields by commit time regardless.
        tips.sort();

        self.log_paged_page(
            HistoryMode::AllBranches,
            Arc::from(tips),
            limit,
            cursor,
            cancellation,
            author,
            chunks,
        )
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
        // reliably with `--skip` across renames. Cursor pages cache the full
        // follow result so repeated "load more" requests do not rescan history.
        if cursor.is_none() {
            let commits = self.log_follow_commits(path, Some(limit.saturating_add(1)))?;
            return paginate_commits(commits.into_iter().map(Ok), limit, cursor);
        }

        let repo = self._repo.to_thread_local();
        let head_oid = gix_head_id_or_none(&repo)?;
        let cache_key = Self::log_file_follow_cache_key(path, head_oid);
        let commits = if let Some(commits) = self.cached_log_file_follow_commits(&cache_key) {
            commits
        } else {
            let commits = Arc::new(self.log_follow_commits(path, None)?);
            self.store_log_file_follow_commits(cache_key, Arc::clone(&commits));
            commits
        };
        paginate_commits(commits.iter().cloned().map(Ok), limit, cursor)
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

    pub(super) fn diff_range_files_impl(
        &self,
        from: &CommitId,
        to: Option<&CommitId>,
    ) -> Result<Vec<CommitFileChange>> {
        match to {
            Some(to) => {
                let repo = self._repo.to_thread_local();
                diff_range_files(&repo, from, to)
            }
            // Working-tree tip: the newer side is the live worktree, which has no
            // tree object, so shell out to `git diff <from>` for the file list
            // (consistent with the unified diff shown in the main pane).
            None => super::submodules::diff_commit_to_worktree_files(&self.spec.workdir, from),
        }
    }

    pub(super) fn commit_messages_impl(&self, ids: &[CommitId]) -> Result<Vec<String>> {
        let repo = self._repo.to_thread_local();
        ids.iter()
            .map(|id| {
                let spec = id.as_ref();
                let commit = repo
                    .rev_parse_single(spec)
                    .map_err(|e| {
                        Error::new(ErrorKind::Backend(format!("gix rev-parse {spec}: {e}")))
                    })?
                    .object()
                    .map_err(|e| {
                        Error::new(ErrorKind::Backend(format!("gix commit object {spec}: {e}")))
                    })?
                    .peel_to_commit()
                    .map_err(|e| {
                        Error::new(ErrorKind::Backend(format!("gix peel commit {spec}: {e}")))
                    })?;
                Ok(
                    bytes_to_text_preserving_utf8(commit.message_raw_sloppy().as_ref())
                        .trim_end()
                        .to_string(),
                )
            })
            .collect()
    }

    pub(super) fn topologically_order_commits_impl(
        &self,
        ids: &[CommitId],
    ) -> Result<Vec<CommitId>> {
        let repo = self._repo.to_thread_local();
        let mut object_ids = Vec::with_capacity(ids.len());
        let mut selected = FxHashMap::with_capacity_and_hasher(ids.len(), Default::default());
        for (ix, id) in ids.iter().enumerate() {
            let spec = id.as_ref();
            let object_id = repo
                .rev_parse_single(spec)
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix rev-parse {spec}: {e}"))))?
                .object()
                .map_err(|e| {
                    Error::new(ErrorKind::Backend(format!("gix commit object {spec}: {e}")))
                })?
                .peel_to_commit()
                .map_err(|e| {
                    Error::new(ErrorKind::Backend(format!("gix peel commit {spec}: {e}")))
                })?
                .id()
                .detach();
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

    pub(super) fn recent_commit_messages_impl(
        &self,
        limit: usize,
    ) -> Result<Vec<RecentCommitMessage>> {
        let Some((limit, scan_limit)) = recent_commit_message_limits(limit) else {
            return Ok(Vec::new());
        };

        let page = self.log_history_mode_page_impl(HistoryMode::FirstParent, scan_limit, None)?;
        let repo = self._repo.to_thread_local();
        let mut seen = FxHashSet::default();
        let mut messages = Vec::with_capacity(limit);

        for commit in page.commits {
            let spec = commit.id.as_ref();
            let object = repo.rev_parse_single(spec).map_err(|e| {
                Error::new(ErrorKind::Backend(format!("gix rev-parse {spec}: {e}")))
            })?;
            let commit_object = object
                .object()
                .map_err(|e| {
                    Error::new(ErrorKind::Backend(format!("gix commit object {spec}: {e}")))
                })?
                .peel_to_commit()
                .map_err(|e| {
                    Error::new(ErrorKind::Backend(format!("gix peel commit {spec}: {e}")))
                })?;
            let message =
                bytes_to_text_preserving_utf8(commit_object.message_raw_sloppy().as_ref())
                    .trim_end()
                    .to_string();
            if message.trim().is_empty() || !seen.insert(message.clone()) {
                continue;
            }

            messages.push(RecentCommitMessage {
                id: commit.id,
                summary: commit.summary,
                message,
            });
            if messages.len() >= limit {
                break;
            }
        }

        Ok(messages)
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
                    author: bstr_to_arc_str(line.signature.name.as_ref()),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git_success(workdir: &Path, args: &[&str]) {
        let mut cmd = crate::util::git_workdir_cmd_for(workdir);
        let output = cmd.args(args).output().expect("spawn git");
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(workdir: &Path, args: &[&str]) -> String {
        let mut cmd = crate::util::git_workdir_cmd_for(workdir);
        let output = cmd.args(args).output().expect("spawn git");
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8(output.stdout)
            .expect("utf8 stdout")
            .trim()
            .to_string()
    }

    fn init_test_repo(workdir: &Path) {
        git_success(workdir, &["init"]);
        for args in [
            ["config", "core.autocrlf", "false"].as_slice(),
            ["config", "core.eol", "lf"].as_slice(),
            ["config", "commit.gpgsign", "false"].as_slice(),
            ["config", "user.name", "Test User"].as_slice(),
            ["config", "user.email", "test@example.com"].as_slice(),
        ] {
            git_success(workdir, args);
        }
    }

    fn write_file(workdir: &Path, relative: &str, contents: &str) {
        let path = workdir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(path, contents).expect("write file");
    }

    fn commit_file(workdir: &Path, path: &str, contents: &str, message: &str) {
        write_file(workdir, path, contents);
        git_success(workdir, &["add", path]);
        git_success(workdir, &["commit", "-m", message]);
    }

    fn open_repo(workdir: &Path) -> GixRepo {
        let thread_safe_repo = gix::open(workdir).expect("open repo").into_sync();
        GixRepo::new(workdir.to_path_buf(), thread_safe_repo)
    }

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
    fn diff_range_files_lists_changes_between_two_commits() {
        use gitcomet_core::domain::FileStatusKind;
        use gitcomet_core::services::GitRepository;

        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path();
        init_test_repo(repo);

        // Base commit: two files.
        write_file(repo, "keep.txt", "one\ntwo\n");
        write_file(repo, "gone.txt", "delete me\n");
        git_success(repo, &["add", "."]);
        git_success(repo, &["commit", "-m", "base"]);
        let from = git_stdout(repo, &["rev-parse", "HEAD"]);

        // Target commit: modify keep.txt, delete gone.txt, add new.txt.
        write_file(repo, "keep.txt", "one\ntwo\nthree\n");
        fs::remove_file(repo.join("gone.txt")).expect("remove gone.txt");
        write_file(repo, "new.txt", "brand new\n");
        git_success(repo, &["add", "-A"]);
        git_success(repo, &["commit", "-m", "target"]);
        let to = git_stdout(repo, &["rev-parse", "HEAD"]);

        let opened = open_repo(repo);
        let mut files = opened
            .diff_range_files(&CommitId(from.into()), Some(&CommitId(to.into())))
            .expect("diff_range_files should succeed");
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let by_path: Vec<(String, FileStatusKind)> = files
            .iter()
            .map(|f| (f.path.to_string_lossy().into_owned(), f.kind))
            .collect();
        assert_eq!(
            by_path,
            vec![
                ("gone.txt".to_string(), FileStatusKind::Deleted),
                ("keep.txt".to_string(), FileStatusKind::Modified),
                ("new.txt".to_string(), FileStatusKind::Added),
            ]
        );
    }

    #[test]
    fn diff_range_files_lists_changes_against_the_working_tree() {
        use gitcomet_core::domain::FileStatusKind;
        use gitcomet_core::services::GitRepository;

        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path();
        init_test_repo(repo);

        // Committed base: two files.
        write_file(repo, "keep.txt", "one\ntwo\n");
        write_file(repo, "gone.txt", "delete me\n");
        git_success(repo, &["add", "."]);
        git_success(repo, &["commit", "-m", "base"]);
        let from = git_stdout(repo, &["rev-parse", "HEAD"]);

        // Uncommitted worktree changes: modify keep.txt (unstaged), delete
        // gone.txt (staged), add new.txt (staged). `git diff <from>` compares the
        // commit directly to the worktree, so all three show; the untracked
        // scratch file does not.
        write_file(repo, "keep.txt", "one\ntwo\nthree\n");
        fs::remove_file(repo.join("gone.txt")).expect("remove gone.txt");
        write_file(repo, "new.txt", "brand new\n");
        git_success(repo, &["add", "new.txt", "gone.txt"]);
        write_file(repo, "untracked.txt", "scratch\n");

        let opened = open_repo(repo);
        // `None` tip = compare `from` against the working tree.
        let mut files = opened
            .diff_range_files(&CommitId(from.into()), None)
            .expect("diff_range_files should succeed");
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let by_path: Vec<(String, FileStatusKind)> = files
            .iter()
            .map(|f| (f.path.to_string_lossy().into_owned(), f.kind))
            .collect();
        assert_eq!(
            by_path,
            vec![
                ("gone.txt".to_string(), FileStatusKind::Deleted),
                ("keep.txt".to_string(), FileStatusKind::Modified),
                ("new.txt".to_string(), FileStatusKind::Added),
            ]
        );
    }

    /// A gitlink has to be flagged as a submodule on both comparison paths. The
    /// tree-diff path reads the entry mode; the working-tree path only gets modes
    /// out of `git diff --raw`, so a plain `--name-status` listing would render
    /// the same submodule as an ordinary file.
    #[test]
    fn diff_range_files_flags_a_submodule_pointer_against_the_working_tree() {
        use gitcomet_core::services::GitRepository;

        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path();
        init_test_repo(repo);

        write_file(repo, "keep.txt", "one\n");
        git_success(repo, &["add", "."]);
        git_success(repo, &["commit", "-m", "base"]);
        let from = git_stdout(repo, &["rev-parse", "HEAD"]);

        // A gitlink staged straight into the index — no submodule clone needed
        // to produce the 160000 entry mode the flag is derived from. The
        // directory has to exist or `git diff <commit>` skips the entry when
        // comparing against the working tree.
        fs::create_dir_all(repo.join("vendor/sub")).expect("create gitlink dir");
        git_success(
            repo,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "160000,1111111111111111111111111111111111111111,vendor/sub",
            ],
        );

        let opened = open_repo(repo);
        let files = opened
            .diff_range_files(&CommitId(from.into()), None)
            .expect("diff_range_files should succeed");
        let gitlink = files
            .iter()
            .find(|f| f.path.to_string_lossy() == "vendor/sub")
            .expect("the gitlink should be listed");
        assert!(
            gitlink.is_submodule,
            "a gitlink must be reported as a submodule, not as a plain file"
        );
        assert!(
            files
                .iter()
                .all(|f| f.is_submodule == (f.path.to_string_lossy() == "vendor/sub")),
            "ordinary files must not be flagged as submodules"
        );
    }

    /// A rename is the one `git diff --raw` record that carries *two* path
    /// fields instead of one. Mis-counting them shifts every following record by
    /// a field, silently pairing paths with the wrong statuses for the rest of
    /// the listing, so the shape is worth pinning directly.
    #[test]
    fn diff_range_files_parses_renames_against_the_working_tree() {
        use gitcomet_core::domain::FileStatusKind;
        use gitcomet_core::services::GitRepository;

        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path();
        init_test_repo(repo);

        // Enough identical content that git scores the move as a rename.
        let body = "alpha\nbeta\ngamma\ndelta\nepsilon\n";
        write_file(repo, "old_name.txt", body);
        write_file(repo, "untouched.txt", "steady\n");
        git_success(repo, &["add", "."]);
        git_success(repo, &["commit", "-m", "base"]);
        let from = git_stdout(repo, &["rev-parse", "HEAD"]);

        // Rename, then change a second file so a mis-parse would visibly shift
        // the records that follow the two-path one.
        fs::remove_file(repo.join("old_name.txt")).expect("remove old_name.txt");
        write_file(repo, "new_name.txt", body);
        write_file(repo, "untouched.txt", "steady\nplus one\n");
        git_success(repo, &["add", "-A"]);

        let opened = open_repo(repo);
        let mut files = opened
            .diff_range_files(&CommitId(from.into()), None)
            .expect("diff_range_files should succeed");
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let by_path: Vec<(String, FileStatusKind)> = files
            .iter()
            .map(|f| (f.path.to_string_lossy().into_owned(), f.kind))
            .collect();
        assert_eq!(
            by_path,
            vec![
                ("new_name.txt".to_string(), FileStatusKind::Renamed),
                ("untouched.txt".to_string(), FileStatusKind::Modified),
            ],
            "the rename must report its destination path, and the record after \
             it must not be shifted"
        );
    }

    /// The empty tree is how the changes a root commit *introduces* are
    /// expressed — a root has no parent to diff from — so it has to resolve as a
    /// comparison base even though it is not a commit.
    #[test]
    fn diff_range_files_accepts_the_empty_tree_as_a_base() {
        use gitcomet_core::domain::{EMPTY_TREE_ID, FileStatusKind};
        use gitcomet_core::services::GitRepository;

        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path();
        init_test_repo(repo);

        write_file(repo, "a.txt", "one\n");
        write_file(repo, "b.txt", "two\n");
        git_success(repo, &["add", "."]);
        git_success(repo, &["commit", "-m", "root"]);
        let root = git_stdout(repo, &["rev-parse", "HEAD"]);

        let opened = open_repo(repo);
        let mut files = opened
            .diff_range_files(
                &CommitId(EMPTY_TREE_ID.into()),
                Some(&CommitId(root.into())),
            )
            .expect("the empty tree should resolve as a base");
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let by_path: Vec<(String, FileStatusKind)> = files
            .iter()
            .map(|f| (f.path.to_string_lossy().into_owned(), f.kind))
            .collect();
        // Everything the root commit introduces shows up, rather than nothing.
        assert_eq!(
            by_path,
            vec![
                ("a.txt".to_string(), FileStatusKind::Added),
                ("b.txt".to_string(), FileStatusKind::Added),
            ]
        );
    }

    #[test]
    fn rename_destination_at_commit_resolves_rename_introduced_at_merge() {
        // Regression: a bare `git diff-tree <merge>` prints nothing (it needs
        // -m/-c/--cc), so resolving a rename introduced at a merge commit used to
        // fail and the followed file fell back to a now-nonexistent path. The fix
        // diffs against the first parent explicitly, which works for merges too.
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path();
        init_test_repo(repo);

        commit_file(repo, "src/old.txt", "alpha\nbeta\ngamma\n", "base");
        let main = git_stdout(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);

        // A side branch edits the file so the merge is a real (non-ff) merge.
        git_success(repo, &["checkout", "-b", "feature"]);
        commit_file(
            repo,
            "src/old.txt",
            "alpha\nbeta two\ngamma\n",
            "edit on feature",
        );

        // Back on the mainline, advance unrelated history, then merge feature but
        // resolve it by renaming the file — a rename that exists only at the merge
        // commit relative to its first parent.
        git_success(repo, &["checkout", &main]);
        commit_file(repo, "other.txt", "x\n", "main advance");
        git_success(repo, &["merge", "--no-commit", "--no-ff", "feature"]);
        fs::create_dir_all(repo.join("lib")).expect("create lib dir");
        git_success(repo, &["mv", "src/old.txt", "lib/new.txt"]);
        git_success(repo, &["commit", "-m", "evil merge rename"]);
        let merge = git_stdout(repo, &["rev-parse", "HEAD"]);

        let opened = open_repo(repo);
        let resolved = opened
            .rename_destination_at_commit(&CommitId(merge.into()), Path::new("src/old.txt"))
            .expect("rename resolution should not error");
        assert_eq!(resolved, Some(PathBuf::from("lib/new.txt")));
    }

    #[test]
    fn recent_commit_message_limits_cap_large_requests_without_panicking() {
        assert_eq!(recent_commit_message_limits(0), None);
        assert_eq!(recent_commit_message_limits(1), Some((1, 5)));
        assert_eq!(recent_commit_message_limits(10), Some((10, 50)));
        assert_eq!(recent_commit_message_limits(20), Some((20, 100)));
        assert_eq!(recent_commit_message_limits(21), Some((21, 100)));
        assert_eq!(recent_commit_message_limits(100), Some((100, 100)));
        assert_eq!(recent_commit_message_limits(101), Some((100, 100)));
        assert_eq!(recent_commit_message_limits(usize::MAX), Some((100, 100)));
    }

    #[test]
    fn recent_commit_messages_large_limit_reads_available_messages() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "tracked.txt", "one\n", "first");
        commit_file(workdir, "tracked.txt", "two\n", "second");
        commit_file(workdir, "tracked.txt", "three\n", "third");

        let repo = open_repo(workdir);
        let messages = repo
            .recent_commit_messages_impl(usize::MAX)
            .expect("recent commit messages");

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].message, "third");
        assert_eq!(messages[1].message, "second");
        assert_eq!(messages[2].message, "first");
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

    #[test]
    fn cursor_file_history_pages_reuse_cached_follow_history() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "tracked.txt", "one\n", "one");
        commit_file(workdir, "tracked.txt", "two\n", "two");
        git_success(workdir, &["mv", "tracked.txt", "renamed.txt"]);
        git_success(workdir, &["commit", "-m", "rename"]);
        commit_file(workdir, "renamed.txt", "four\n", "four");

        let repo = open_repo(workdir);
        let page1 = repo
            .log_file_page_impl(Path::new("renamed.txt"), 1, None)
            .expect("first file log page");
        assert_eq!(page1.commits.len(), 1);
        assert!(page1.next_cursor.is_some());
        assert!(
            repo.log_file_follow_cache
                .lock()
                .expect("log file follow cache")
                .is_empty(),
            "first page should stay bounded and avoid the full-history cache"
        );

        let page2 = repo
            .log_file_page_impl(Path::new("renamed.txt"), 1, page1.next_cursor.as_ref())
            .expect("second file log page");
        assert_eq!(page2.commits.len(), 1);
        assert!(page2.next_cursor.is_some());

        let cached_commits = {
            let cache = repo
                .log_file_follow_cache
                .lock()
                .expect("log file follow cache");
            assert_eq!(cache.len(), 1);
            assert_eq!(cache[0].key.path.as_path(), Path::new("renamed.txt"));
            assert_eq!(cache[0].commits.len(), 4);
            Arc::clone(&cache[0].commits)
        };

        let page3 = repo
            .log_file_page_impl(Path::new("renamed.txt"), 1, page2.next_cursor.as_ref())
            .expect("third file log page");
        assert_eq!(page3.commits.len(), 1);

        let cache = repo
            .log_file_follow_cache
            .lock()
            .expect("log file follow cache");
        assert_eq!(cache.len(), 1);
        assert!(
            Arc::ptr_eq(&cached_commits, &cache[0].commits),
            "third page should use the cached full follow result"
        );
    }

    #[test]
    fn reflog_head_entries_carry_the_committer_as_author() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "a.txt", "one\n", "first");
        commit_file(workdir, "a.txt", "two\n", "second");

        let repo = open_repo(workdir);
        let entries = repo.reflog_head_impl(10).expect("reflog_head_impl");

        assert_eq!(entries.len(), 2);
        // Newest first: the reflog is read in reverse, matching `HEAD@{0}`
        // being the current position.
        assert_eq!(entries[0].selector.as_ref(), "HEAD@{0}");
        assert_eq!(entries[1].selector.as_ref(), "HEAD@{1}");
        for entry in &entries {
            // `user.name` from `init_test_repo` is what git records as the
            // reflog line's committer identity.
            assert_eq!(entry.author.as_ref(), "Test User");
        }
    }

    #[test]
    fn reflog_head_impl_returns_empty_for_a_zero_limit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        commit_file(workdir, "a.txt", "one\n", "first");

        let repo = open_repo(workdir);
        let entries = repo.reflog_head_impl(0).expect("reflog_head_impl");
        assert!(entries.is_empty());
    }
}
