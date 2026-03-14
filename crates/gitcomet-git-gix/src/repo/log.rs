use super::GixRepo;
use super::history::gix_head_id_or_none;
use crate::util::{
    bytes_to_text_preserving_utf8, parse_git_log_pretty_records, path_buf_from_git_bytes,
    run_git_capture, unix_seconds_to_system_time, unix_seconds_to_system_time_or_epoch,
};
use gitcomet_core::domain::{
    Commit, CommitDetails, CommitFileChange, CommitId, LogCursor, LogPage, ReflogEntry, StashEntry,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::Result;
use gix::bstr::ByteSlice as _;
use gix::traverse::commit::simple::CommitTimeOrder;
use rustc_hash::FxHashSet as HashSet;
use std::path::Path;

struct CursorGate<'a> {
    cursor: Option<&'a LogCursor>,
    started: bool,
}

impl<'a> CursorGate<'a> {
    fn new(cursor: Option<&'a LogCursor>) -> Self {
        Self {
            cursor,
            started: cursor.is_none(),
        }
    }

    fn should_skip(&mut self, id: &str) -> bool {
        if self.started {
            return false;
        }

        let Some(cursor) = self.cursor else {
            self.started = true;
            return false;
        };

        if cursor.last_seen.0 == id {
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
                id: CommitId(line.new_oid.to_string()),
                message: bytes_to_text_preserving_utf8(line.message.as_ref()),
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

fn commit_from_walk_info<'repo>(
    info: &gix::revision::walk::Info<'repo>,
    id: String,
) -> Result<Commit> {
    let commit_obj = info
        .object()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit object: {e}"))))?;

    let summary = commit_obj
        .message_raw_sloppy()
        .lines()
        .next()
        .unwrap_or_default()
        .to_str_lossy()
        .into_owned();

    let author = commit_obj
        .author()
        .map(|s| s.name.to_str_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());

    let seconds = commit_obj.time().map(|t| t.seconds).unwrap_or(0);
    let time = unix_seconds_to_system_time_or_epoch(seconds);

    let parent_ids = info
        .parent_ids()
        .map(|parent_id| CommitId(parent_id.detach().to_string()))
        .collect::<Vec<_>>();

    Ok(Commit {
        id: CommitId(id),
        parent_ids,
        summary,
        author,
        time,
    })
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
    let mut result: Vec<Commit> = Vec::with_capacity(limit.min(2048));
    let mut next_cursor: Option<LogCursor> = None;

    for commit in commits {
        let commit = commit?;
        if cursor_gate.should_skip(commit.id.as_ref()) {
            continue;
        }

        if result.len() >= limit {
            next_cursor = result.last().map(|c| LogCursor {
                last_seen: c.id.clone(),
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
    paginate_commits(
        walk.map(|result| {
            let info =
                result.map_err(|e| Error::new(ErrorKind::Backend(format!("gix walk: {e}"))))?;
            let id = info.id().detach().to_string();
            commit_from_walk_info(&info, id)
        }),
        limit,
        cursor,
    )
}

impl GixRepo {
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
        if limit == 0 {
            return Ok(empty_log_page());
        }

        let repo = self._repo.to_thread_local();
        let Some(head_id) = gix_head_id_or_none(&repo)? else {
            return Ok(empty_log_page());
        };

        let walk = repo
            .rev_walk([head_id])
            .sorting(gix::revision::walk::Sorting::ByCommitTime(
                CommitTimeOrder::NewestFirst,
            ))
            .first_parent_only()
            .all()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix rev_walk: {e}"))))?;
        log_page_from_walk(walk, limit, cursor)
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
            .map(|parent| CommitId(parent.to_string()))
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
                    new_id: CommitId(line.new_oid.to_string()),
                    message: bytes_to_text_preserving_utf8(line.message.as_ref()),
                    time: unix_seconds_to_system_time(line.signature.time.seconds),
                    selector: format!("HEAD@{{{index}}}"),
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
            last_seen: CommitId("c2".to_string()),
        };
        let mut gate = CursorGate::new(Some(&cursor));

        assert!(gate.should_skip("c1"));
        assert!(gate.should_skip("c2"));
        assert!(!gate.should_skip("c3"));
        assert!(!gate.should_skip("c4"));
    }
}
