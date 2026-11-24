use super::{
    GitlinkIndexState, GixRepo, configured_submodule_row, gitlink_commit_id_in_object,
    head_gitlink_commit_id, object_id_from_commit_id, object_id_to_commit_id, open_gitlink_repo,
    pathbuf_from_gix_path, submodule_repo_workdir,
};
use crate::repo::comparison::{
    NumstatCounts, git_range_numstat_counts, git_range_status_changes, parse_numstat_field,
};
use crate::util::{
    git_workdir_cmd_for, path_buf_from_git_bytes, push_read_only_diff_flags,
    run_git_capture_bytes_cancellable,
};
use gitcomet_core::domain::{
    CommitId, FileStatus, RepoStatus, Submodule, SubmoduleDiffRange, SubmoduleDiffRangeKind,
    SubmoduleDiffSummary, SubmoduleDiffSummaryMode, SubmoduleInnerChange, SubmoduleStatus,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use std::path::Path;

const SUBMODULE_HISTORY_UNAVAILABLE_REASON: &str = "Submodule history is not available locally.";
const SUBMODULE_POINTER_SIDE_UNAVAILABLE_REASON: &str =
    "Only one side of the submodule pointer is available.";

/// Look up just this gitlink, including conflict stages, without collecting
/// every file in an index or opening any sibling submodule repositories.
fn index_gitlink_at_path(index: &gix::index::State, path: &Path) -> Option<GitlinkIndexState> {
    let key = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(path).ok()?);
    let mut result = None;
    for stage in [
        gix::index::entry::Stage::Unconflicted,
        gix::index::entry::Stage::Base,
        gix::index::entry::Stage::Ours,
        gix::index::entry::Stage::Theirs,
    ] {
        let Some(entry) = index.entry_by_path_and_stage(key.as_ref(), stage) else {
            continue;
        };
        if entry.mode != gix::index::entry::Mode::COMMIT {
            continue;
        }
        let state = result.get_or_insert_with(GitlinkIndexState::default);
        state.record(entry.id, stage);
    }
    result
}

pub(super) fn submodule_worktree_diff_summary(
    repo: &gix::Repository,
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<SubmoduleDiffSummary> {
    cancellation.check_cancelled()?;
    let index = crate::refs::index_or_load_from_head_or_empty(repo)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))).with_source(e))?;
    let gitlink = index_gitlink_at_path(&index, path);
    if gitlink.is_none() {
        // The sidebar includes nested paths. Resolve their pointers against
        // the owning repository, following only the ancestors of the target.
        for ancestor in path
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            cancellation.check_cancelled()?;
            if index_gitlink_at_path(&index, ancestor).is_some()
                && let Some(nested) = open_gitlink_repo(repo, ancestor)?
            {
                let relative = path.strip_prefix(ancestor).expect("path ancestor");
                let mut summary = submodule_worktree_diff_summary(&nested, relative, cancellation)?;
                summary.path = ancestor.join(summary.path);
                return Ok(summary);
            }
        }
    }
    let mut submodule = None;
    let mut configured_repo = None;
    if let Some(gitlink) = gitlink {
        if let Some(configured) = crate::refs::submodules(repo).map_err(|e| {
            Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))).with_source(e)
        })? {
            for candidate in configured {
                cancellation.check_cancelled()?;
                let candidate_path = candidate.path().map_err(|e| {
                    Error::new(ErrorKind::Backend(format!("gix submodule path: {e}")))
                })?;
                if pathbuf_from_gix_path(candidate_path.as_ref())? == path {
                    let (row, opened) =
                        configured_submodule_row(repo, candidate, path.to_path_buf(), gitlink)?;
                    submodule = Some(row);
                    configured_repo = opened;
                    break;
                }
            }
        }
        if submodule.is_none() {
            submodule = Some(Submodule {
                path: path.to_path_buf(),
                recorded_head: gitlink.index_head_or_null(repo),
                checked_out_head: None,
                status: SubmoduleStatus::MissingMapping,
            });
        }
    }
    cancellation.check_cancelled()?;
    let head_gitlink = head_gitlink_commit_id(repo, path)?;
    let (summary_path, status, index_gitlink, checked_out_head) = match submodule {
        Some(submodule) => (
            submodule.path,
            Some(submodule.status),
            Some(submodule.recorded_head),
            submodule.checked_out_head,
        ),
        None => {
            if head_gitlink.is_none() {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "submodule '{}' is not configured in this repository",
                    path.display()
                ))));
            }
            (path.to_path_buf(), None, None, None)
        }
    };

    let nested_workdir = submodule_repo_workdir(repo).join(&summary_path);
    // Reuse the repository `configured_submodule_row` already opened rather than
    // opening the same directory twice. Conflicted and uninitialised submodules
    // return none, and still fall back to the on-disk checkout.
    let nested_repo = match configured_repo {
        Some(nested) => Some(nested),
        None => open_gitlink_repo(repo, &summary_path)?,
    };
    let (live_staged, live_unstaged) =
        submodule_live_inner_changes(&nested_workdir, nested_repo.as_ref(), cancellation)?;

    let not_loaded_reason = (nested_repo.is_none() || checked_out_head.is_none())
        .then_some("Submodule is not loaded locally.".to_string());

    let ranges = vec![
        build_submodule_range(
            &nested_workdir,
            nested_repo.as_ref(),
            SubmoduleDiffRangeKind::StagedPointer,
            head_gitlink,
            index_gitlink.clone(),
            None,
            cancellation,
        )?,
        build_submodule_range(
            &nested_workdir,
            nested_repo.as_ref(),
            SubmoduleDiffRangeKind::UnstagedPointer,
            index_gitlink,
            checked_out_head.clone(),
            not_loaded_reason,
            cancellation,
        )?,
    ];

    Ok(SubmoduleDiffSummary {
        path: summary_path,
        mode: SubmoduleDiffSummaryMode::Worktree,
        status,
        checkout_available: nested_repo.is_some(),
        commit_id: None,
        parent_commit_id: None,
        checked_out_head,
        ranges,
        live_staged,
        live_unstaged,
    })
}

pub(super) fn submodule_commit_diff_summary(
    repo: &gix::Repository,
    commit_id: &CommitId,
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<SubmoduleDiffSummary> {
    cancellation.check_cancelled()?;
    let parent_commit_id = first_parent_commit_id(repo, commit_id)?;
    let from = match parent_commit_id.as_ref() {
        Some(parent_commit_id) => {
            gitlink_commit_id_at_revision(repo, parent_commit_id.as_ref(), path)?
        }
        None => None,
    };
    let to = gitlink_commit_id_at_revision(repo, commit_id.as_ref(), path)?;

    let nested_workdir = submodule_repo_workdir(repo).join(path);
    let nested_repo = open_gitlink_repo(repo, path)?;
    let unavailable_reason = if nested_repo.is_none() {
        Some(SUBMODULE_HISTORY_UNAVAILABLE_REASON.to_string())
    } else {
        None
    };

    let ranges = vec![build_submodule_range(
        &nested_workdir,
        nested_repo.as_ref(),
        SubmoduleDiffRangeKind::CommitHistory,
        from,
        to,
        unavailable_reason,
        cancellation,
    )?];

    Ok(SubmoduleDiffSummary {
        path: path.to_path_buf(),
        mode: SubmoduleDiffSummaryMode::CommitHistory,
        status: None,
        checkout_available: nested_repo.is_some(),
        commit_id: Some(commit_id.clone()),
        parent_commit_id,
        checked_out_head: None,
        ranges,
        live_staged: Vec::new(),
        live_unstaged: Vec::new(),
    })
}

fn submodule_live_inner_changes(
    nested_workdir: &Path,
    nested_repo: Option<&gix::Repository>,
    cancellation: &CancellationToken,
) -> Result<(Vec<SubmoduleInnerChange>, Vec<SubmoduleInnerChange>)> {
    let Some(nested_repo) = nested_repo else {
        return Ok((Vec::new(), Vec::new()));
    };

    let nested_status_repo = GixRepo::new(
        nested_workdir.to_path_buf(),
        nested_repo.clone().into_sync(),
    )?;
    let RepoStatus { staged, unstaged } =
        nested_status_repo.status_cancellable_impl(cancellation)?;
    let staged_counts = git_numstat_counts(nested_workdir, true, cancellation)?;
    let unstaged_counts = git_numstat_counts(nested_workdir, false, cancellation)?;
    Ok((
        submodule_inner_changes_from_status(&staged, &staged_counts, cancellation)?,
        submodule_inner_changes_from_status(&unstaged, &unstaged_counts, cancellation)?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_submodule_range(
    nested_workdir: &Path,
    nested_repo: Option<&gix::Repository>,
    kind: SubmoduleDiffRangeKind,
    from: Option<CommitId>,
    to: Option<CommitId>,
    unavailable_reason: Option<String>,
    cancellation: &CancellationToken,
) -> Result<SubmoduleDiffRange> {
    cancellation.check_cancelled()?;
    let unavailable_reason = unavailable_reason
        .or_else(|| submodule_range_unavailable_reason(nested_repo, from.as_ref(), to.as_ref()));

    let changes = if unavailable_reason.is_none() {
        match (nested_repo, from.as_ref(), to.as_ref()) {
            (_, Some(from), Some(to)) if from == to => Vec::new(),
            (Some(_), Some(from), Some(to)) => {
                submodule_range_changes_from_commits(nested_workdir, from, to, cancellation)?
            }
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };

    Ok(SubmoduleDiffRange {
        kind,
        from,
        to,
        changes,
        unavailable_reason,
    })
}

fn submodule_range_unavailable_reason(
    nested_repo: Option<&gix::Repository>,
    from: Option<&CommitId>,
    to: Option<&CommitId>,
) -> Option<String> {
    let (Some(from), Some(to)) = (from, to) else {
        return Some(SUBMODULE_POINTER_SIDE_UNAVAILABLE_REASON.to_string());
    };
    let Some(nested_repo) = nested_repo else {
        return Some(SUBMODULE_HISTORY_UNAVAILABLE_REASON.to_string());
    };
    if from == to {
        return None;
    }
    if !submodule_commit_available(nested_repo, from)
        || !submodule_commit_available(nested_repo, to)
    {
        return Some(SUBMODULE_HISTORY_UNAVAILABLE_REASON.to_string());
    }
    None
}

fn submodule_commit_available(repo: &gix::Repository, commit_id: &CommitId) -> bool {
    object_id_from_commit_id(commit_id, repo.object_hash())
        .and_then(|object_id| repo.find_commit(object_id).ok())
        .is_some()
}

fn submodule_range_changes_from_commits(
    nested_workdir: &Path,
    from: &CommitId,
    to: &CommitId,
    cancellation: &CancellationToken,
) -> Result<Vec<SubmoduleInnerChange>> {
    let status_changes =
        git_range_status_changes(nested_workdir, from, Some(to), None, cancellation)?;
    let counts = git_range_numstat_counts(nested_workdir, from, Some(to), None, cancellation)?;
    status_changes
        .into_iter()
        .map(|change| {
            cancellation.check_cancelled()?;
            let (additions, deletions) = change.line_counts(&counts);
            Ok(SubmoduleInnerChange::new(change.path, change.kind)
                .with_line_counts(additions, deletions)
                .with_old_path(change.old_path))
        })
        .collect()
}

fn submodule_inner_changes_from_status(
    entries: &[FileStatus],
    counts: &NumstatCounts,
    cancellation: &CancellationToken,
) -> Result<Vec<SubmoduleInnerChange>> {
    entries
        .iter()
        .map(|entry| {
            cancellation.check_cancelled()?;
            let (additions, deletions) = counts.get(&entry.path).cloned().unwrap_or((None, None));
            Ok(SubmoduleInnerChange::new(entry.path.clone(), entry.kind)
                .with_line_counts(additions, deletions))
        })
        .collect()
}

fn git_numstat_counts(
    workdir: &Path,
    cached: bool,
    cancellation: &CancellationToken,
) -> Result<NumstatCounts> {
    let mut command = git_workdir_cmd_for(workdir);
    push_read_only_diff_flags(&mut command);
    command.arg("diff").args(["--no-ext-diff", "--no-textconv"]);
    if cached {
        command.arg("--cached");
    }
    // A gitlink would count its "Subproject commit" line, and these rows
    // carry no mode to drop that by.
    command
        .arg("--numstat")
        .arg("-z")
        .arg("--no-renames")
        .arg("--ignore-submodules=all");
    let label = if cached {
        "git diff --cached --numstat -z --no-renames --ignore-submodules=all"
    } else {
        "git diff --numstat -z --no-renames --ignore-submodules=all"
    };
    let output = run_git_capture_bytes_cancellable(command, label, cancellation)?;

    let mut counts = NumstatCounts::default();
    for record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        cancellation.check_cancelled()?;
        let mut fields = record.splitn(3, |byte| *byte == b'\t');
        let additions = parse_numstat_field(fields.next().unwrap_or_default());
        let deletions = parse_numstat_field(fields.next().unwrap_or_default());
        let path =
            path_buf_from_git_bytes(fields.next().unwrap_or_default(), "git diff --numstat path")?;
        counts.insert(path, (additions, deletions));
    }

    Ok(counts)
}

fn first_parent_commit_id(
    repo: &gix::Repository,
    commit_id: &CommitId,
) -> Result<Option<CommitId>> {
    let Some(object_id) = object_id_from_commit_id(commit_id, repo.object_hash()) else {
        return Err(Error::new(ErrorKind::Backend(format!(
            "invalid commit id '{}'",
            commit_id.as_ref()
        ))));
    };
    let commit = repo
        .find_commit(object_id)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix find commit: {e}"))))?;
    Ok(commit
        .parent_ids()
        .next()
        .map(|id| object_id_to_commit_id(id.detach())))
}

fn gitlink_commit_id_at_revision(
    repo: &gix::Repository,
    revision: &str,
    path: &Path,
) -> Result<Option<CommitId>> {
    let object_id = crate::refs::resolve_required(repo, revision)
        .map(|id| id.detach())
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "resolve revision '{revision}' for submodule '{}': {e}",
                path.display()
            )))
            .with_source(e)
        })?;
    gitlink_commit_id_in_object(repo, object_id, path)
}

#[cfg(test)]
mod tests;
