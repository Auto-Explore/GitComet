//! Runs diff-session loads with the same backend readers as the selected
//! diff, under the session's own token (a child of the repository's).

use super::*;
use crate::diff_session::{
    DiffSessionContent, DiffSessionEffect, DiffSessionMsg as Event, DiffSessionWork,
};

pub(super) fn schedule(
    executor: &TaskExecutor,
    repos: &util::RepoMap,
    msg_tx: StoreWorkerSender,
    work: DiffSessionEffect,
    parent: CancellationToken,
) {
    let on_missing = work.clone();
    util::spawn_detached_with_repo_or_else(
        executor,
        "diff-session",
        repos,
        work.repo_id,
        msg_tx,
        move |repo, tx| {
            let DiffSessionEffect {
                repo_id,
                view,
                lifetime,
                generation,
                work,
                cancellation,
            } = work;
            let cancellation = cancellation.with_parent(parent);
            let send = |content| {
                util::send_or_log(
                    &tx,
                    Msg::DiffSession(Event::Loaded {
                        repo_id,
                        view,
                        lifetime,
                        generation,
                        content,
                    }),
                );
            };
            match work {
                DiffSessionWork::Content {
                    target,
                    encoding,
                    patch,
                    file_text,
                    image,
                } => {
                    if let Some(path) = target.file_path() {
                        send(DiffSessionContent::Attributes(repo.text_attributes(path)));
                    }
                    if patch {
                        send(DiffSessionContent::Patch(
                            repo.diff_parsed_with_encoding_cancellable(
                                &target,
                                encoding,
                                &cancellation,
                            ),
                        ));
                    }
                    if file_text && !cancellation.is_cancelled() {
                        send(DiffSessionContent::FileText(
                            repo.diff_file_text_with_encoding_cancellable(
                                &target,
                                encoding,
                                &cancellation,
                            ),
                        ));
                    }
                    if image && !cancellation.is_cancelled() {
                        send(DiffSessionContent::Image(
                            repo.diff_file_image_cancellable(&target, &cancellation),
                        ));
                    }
                }
                DiffSessionWork::Changes { source } => {
                    let result = match source {
                        crate::diff_session::ChangeSource::Worktree {
                            area,
                            include_untracked,
                        } => {
                            let status = match area {
                                gitcomet_core::domain::DiffArea::Staged => {
                                    repo.staged_status_cancellable(&cancellation)
                                }
                                gitcomet_core::domain::DiffArea::Unstaged => {
                                    repo.worktree_status_cancellable(&cancellation)
                                }
                            };
                            status.map(|files| (None, files.into_iter()
                                .filter(|file| include_untracked || file.kind != gitcomet_core::domain::FileStatusKind::Untracked)
                                .map(|file| gitcomet_core::domain::CommitFileChange::new(file.path, file.kind)).collect()))
                        }
                        crate::diff_session::ChangeSource::Commit(id) => repo
                            .commit_details(&id)
                            .map(|details| (details.parent_ids.first().cloned(), details.files)),
                        crate::diff_session::ChangeSource::Comparison { from, to, options } => repo
                            .compare_files(&from, to.as_ref(), &options, &cancellation)
                            .map(|comparison| (Some(comparison.base), comparison.files)),
                    };
                    util::send_or_log(
                        &tx,
                        Msg::DiffSession(Event::ChangesLoaded {
                            repo_id,
                            view,
                            lifetime,
                            generation,
                            result,
                        }),
                    );
                }
                DiffSessionWork::Blame { path, source } => {
                    if !cancellation.is_cancelled() {
                        send(DiffSessionContent::Blame(repo_load::load_blame(
                            repo.as_ref(),
                            &path,
                            &source,
                        )));
                    }
                }
            }
        },
        move |tx| {
            for reply in
                on_missing.failed(Error::new(ErrorKind::Backend("Repository closed".into())))
            {
                util::send_or_log(&tx, Msg::DiffSession(reply));
            }
        },
    );
}
