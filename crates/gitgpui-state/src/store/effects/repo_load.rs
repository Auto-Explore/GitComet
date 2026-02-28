use crate::msg::Msg;
use gitgpui_core::domain::{Diff, DiffArea, DiffTarget, LogCursor, LogScope};
use gitgpui_core::error::ErrorKind;
use gitgpui_core::services::decode_utf8_optional;
use std::path::PathBuf;
use std::sync::mpsc;

use super::super::{RepoId, executor::TaskExecutor};
use super::util::{RepoMap, spawn_with_repo};

pub(super) fn schedule_load_branches(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::BranchesLoaded {
            repo_id,
            result: repo.list_branches(),
        });
    });
}

pub(super) fn schedule_load_remotes(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RemotesLoaded {
            repo_id,
            result: repo.list_remotes(),
        });
    });
}

pub(super) fn schedule_load_remote_branches(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RemoteBranchesLoaded {
            repo_id,
            result: repo.list_remote_branches(),
        });
    });
}

pub(super) fn schedule_load_status(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::StatusLoaded {
            repo_id,
            result: repo.status(),
        });
    });
}

pub(super) fn schedule_load_head_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::HeadBranchLoaded {
            repo_id,
            result: repo.current_branch(),
        });
    });
}

pub(super) fn schedule_load_upstream_divergence(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::UpstreamDivergenceLoaded {
            repo_id,
            result: repo.upstream_divergence(),
        });
    });
}

pub(super) fn schedule_load_log(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    scope: LogScope,
    limit: usize,
    cursor: Option<LogCursor>,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = {
            let cursor_ref = cursor.as_ref();
            match scope {
                LogScope::CurrentBranch => repo.log_head_page(limit, cursor_ref),
                LogScope::AllBranches => repo.log_all_branches_page(limit, cursor_ref),
            }
        };
        let _ = msg_tx.send(Msg::LogLoaded {
            repo_id,
            scope,
            cursor,
            result,
        });
    });
}

pub(super) fn schedule_load_tags(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::TagsLoaded {
            repo_id,
            result: repo.list_tags(),
        });
    });
}

pub(super) fn schedule_load_stashes(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    limit: usize,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let mut entries = repo.stash_list();
        if let Ok(v) = &mut entries {
            v.truncate(limit);
        }
        let _ = msg_tx.send(Msg::StashesLoaded {
            repo_id,
            result: entries,
        });
    });
}

pub(super) fn schedule_load_conflict_file(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let conflict_session = repo.conflict_session(&path).ok().flatten();

        let stages = match repo.conflict_file_stages(&path) {
            Ok(v) => Ok(v),
            Err(e) if matches!(e.kind(), ErrorKind::Unsupported(_)) => repo
                .diff_file_text(&DiffTarget::WorkingTree {
                    path: path.clone(),
                    area: DiffArea::Unstaged,
                })
                .map(|opt| {
                    opt.map(|d| {
                        let ours_bytes = d.old.as_ref().map(|text| text.as_bytes().to_vec());
                        let theirs_bytes = d.new.as_ref().map(|text| text.as_bytes().to_vec());
                        gitgpui_core::services::ConflictFileStages {
                            path: d.path,
                            base_bytes: None,
                            ours_bytes,
                            theirs_bytes,
                            base: None,
                            ours: d.old,
                            theirs: d.new,
                        }
                    })
                }),
            Err(e) => Err(e),
        };

        let current_bytes = std::fs::read(repo.spec().workdir.join(&path)).ok();
        let current = decode_utf8_optional(current_bytes.as_deref());

        let result = stages.map(|opt| {
            opt.map(|d| {
                let gitgpui_core::services::ConflictFileStages {
                    path,
                    base_bytes,
                    ours_bytes,
                    theirs_bytes,
                    base,
                    ours,
                    theirs,
                } = d;

                crate::model::ConflictFile {
                    path,
                    base: base.or_else(|| decode_utf8_optional(base_bytes.as_deref())),
                    ours: ours.or_else(|| decode_utf8_optional(ours_bytes.as_deref())),
                    theirs: theirs.or_else(|| decode_utf8_optional(theirs_bytes.as_deref())),
                    base_bytes,
                    ours_bytes,
                    theirs_bytes,
                    current_bytes,
                    current,
                }
            })
        });

        let _ = msg_tx.send(Msg::ConflictFileLoaded {
            repo_id,
            path,
            result: Box::new(result),
            conflict_session,
        });
    });
}

pub(super) fn schedule_load_reflog(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    limit: usize,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::ReflogLoaded {
            repo_id,
            result: repo.reflog_head(limit),
        });
    });
}

pub(super) fn schedule_load_file_history(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
    limit: usize,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::FileHistoryLoaded {
            repo_id,
            path: path.clone(),
            result: repo.log_file_page(&path, limit, None),
        });
    });
}

pub(super) fn schedule_load_blame(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
    rev: Option<String>,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.blame_file(&path, rev.as_deref());
        let _ = msg_tx.send(Msg::BlameLoaded {
            repo_id,
            path: path.clone(),
            rev: rev.clone(),
            result,
        });
    });
}

pub(super) fn schedule_load_worktrees(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::WorktreesLoaded {
            repo_id,
            result: repo.list_worktrees(),
        });
    });
}

pub(super) fn schedule_load_submodules(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::SubmodulesLoaded {
            repo_id,
            result: repo.list_submodules(),
        });
    });
}

pub(super) fn schedule_load_rebase_state(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RebaseStateLoaded {
            repo_id,
            result: repo.rebase_in_progress(),
        });
    });
}

pub(super) fn schedule_load_merge_commit_message(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::MergeCommitMessageLoaded {
            repo_id,
            result: repo.merge_commit_message(),
        });
    });
}

pub(super) fn schedule_load_commit_details(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    commit_id: gitgpui_core::domain::CommitId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::CommitDetailsLoaded {
            repo_id,
            commit_id: commit_id.clone(),
            result: repo.commit_details(&commit_id),
        });
    });
}

pub(super) fn schedule_load_diff(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    target: DiffTarget,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo
            .diff_unified(&target)
            .map(|text| Diff::from_unified(target.clone(), &text));
        let _ = msg_tx.send(Msg::DiffLoaded {
            repo_id,
            target,
            result,
        });
    });
}

pub(super) fn schedule_load_diff_file(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    target: DiffTarget,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.diff_file_text(&target);
        let _ = msg_tx.send(Msg::DiffFileLoaded {
            repo_id,
            target,
            result,
        });
    });
}

pub(super) fn schedule_load_diff_file_image(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    target: DiffTarget,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.diff_file_image(&target);
        let _ = msg_tx.send(Msg::DiffFileImageLoaded {
            repo_id,
            target,
            result,
        });
    });
}
