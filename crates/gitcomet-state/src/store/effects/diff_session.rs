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
                    // Every part asked for is answered, or the session's
                    // pending flags never clear; once cancelled, the readers
                    // are skipped and the answer says so.
                    if file_text {
                        send(DiffSessionContent::FileText(
                            cancellation
                                .check_cancelled()
                                .and_then(|()| {
                                    repo.diff_file_text_with_encoding_cancellable(
                                        &target,
                                        encoding,
                                        &cancellation,
                                    )
                                })
                                .map(|text| text.map(Box::new)),
                        ));
                    }
                    if image {
                        send(DiffSessionContent::Image(
                            cancellation.check_cancelled().and_then(|()| {
                                repo.diff_file_image_cancellable(&target, &cancellation)
                            }),
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
                    send(DiffSessionContent::Blame(
                        cancellation
                            .check_cancelled()
                            .and_then(|()| repo_load::load_blame(repo.as_ref(), &path, &source)),
                    ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_session::DiffViewId;
    use gitcomet_core::domain::{DiffArea, DiffTarget};

    /// Once its repository's loads are cancelled, a session load skips its
    /// readers but still answers every part it was asked for: an unanswered
    /// part would leave the session loading forever.
    #[test]
    fn a_cancelled_load_answers_every_requested_part() {
        let executor = super::super::super::executor::TaskExecutor::new(1);
        let repo_id = RepoId(1);
        let mut repos: util::RepoMap = Default::default();
        repos.insert(
            repo_id,
            Arc::new(crate::store::tests::DummyRepo::new(
                "/tmp/diff-session-cancelled",
            )),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let msg_tx =
            super::super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(tx);
        let parent = CancellationToken::new();
        parent.cancel();
        let work = DiffSessionEffect {
            repo_id,
            view: DiffViewId::next(),
            lifetime: 1,
            generation: 1,
            work: DiffSessionWork::Content {
                target: DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged),
                encoding: None,
                patch: true,
                file_text: true,
                image: true,
            },
            cancellation: CancellationToken::new(),
        };
        schedule(&executor, &repos, msg_tx, work, parent);

        fn cancelled<T>(result: &gitcomet_core::services::Result<T>) -> bool {
            matches!(result, Err(error) if matches!(error.kind(), ErrorKind::Cancelled))
        }
        let mut parts = Vec::new();
        while let Ok(msg) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            let Msg::DiffSession(Event::Loaded { content, .. }) = msg else {
                panic!("not a session reply");
            };
            parts.push(match content {
                DiffSessionContent::Attributes(_) => "attributes",
                DiffSessionContent::Patch(_) => "patch",
                DiffSessionContent::FileText(result) => {
                    assert!(cancelled(&result));
                    "file_text"
                }
                DiffSessionContent::Image(result) => {
                    assert!(cancelled(&result));
                    "image"
                }
                DiffSessionContent::Blame(_) => "blame",
            });
            if parts.len() == 4 {
                break;
            }
        }
        assert_eq!(parts, ["attributes", "patch", "file_text", "image"]);
    }
}
