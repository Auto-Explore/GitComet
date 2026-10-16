use super::*;
use crate::view::documents::buffers::DocumentBuffer;
use gitcomet_state::model::RepoId;
use gitcomet_state::msg::Msg;

struct OpeningBackend;

impl GitBackend for OpeningBackend {
    fn open(
        &self,
        path: &Path,
    ) -> gitcomet_core::services::Result<Arc<dyn gitcomet_core::services::GitRepository>> {
        Ok(Arc::new(
            gitcomet_core::test_support::UnconfiguredRepository::new(path),
        ))
    }
}

fn await_close_fixture(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    description: &str,
    ready: impl Fn(&GitCometView, &gpui::App) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                crate::view::test_support::sync_store_snapshot(view, cx);
            });
        });
        cx.run_until_parked();
        let complete = cx.update(|_, app| {
            view.read(app)
                .diff_renderer
                .clone()
                .update(app, |pane, cx| pane.ensure_file_editor_loaded(cx));
            ready(view.read(app), app)
        });
        if complete {
            return;
        }
        if std::time::Instant::now() >= deadline {
            let state = cx.update(|_, app| {
                let view = view.read(app);
                let pane = view.diff_renderer.read(app);
                (
                    view.state
                        .repos
                        .iter()
                        .map(|repo| (repo.id, repo.open.clone(), repo.local_actions_in_flight))
                        .collect::<Vec<_>>(),
                    view.documents.read(app).unsaved_labels(app),
                    pane.editor_documents
                        .all()
                        .iter()
                        .map(|buffer| {
                            let state = buffer.read();
                            (
                                buffer.id(),
                                state.identity().clone(),
                                state.repository(),
                                state.dirty(),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            });
            panic!("{description}: {state:?}");
        }
        std::thread::yield_now();
    }
}

fn edit_repository_file(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: RepoId,
) -> DocumentBuffer {
    cx.update(|_, app| {
        view.read(app)
            .store
            .dispatch(Msg::SetActiveRepo { repo_id });
    });
    await_close_fixture(view, cx, "selected repository did not open", |view, _| {
        view.state.active_repo == Some(repo_id)
            && view
                .state
                .repos
                .iter()
                .any(|repo| repo.id == repo_id && matches!(repo.open, Loadable::Ready(())))
    });
    cx.update(|_, app| {
        view.read(app).store.dispatch(Msg::OpenFileEditor {
            repo_id,
            path: PathBuf::from("main.rs"),
        });
    });
    await_close_fixture(view, cx, "repository editor did not load", |view, app| {
        let pane = view.diff_renderer.read(app);
        pane.editor_documents.active().is_some_and(|buffer| {
            buffer.read().repository() == Some(repo_id) && !buffer.read().loading()
        }) && pane.file_editor_input.read(app).text() == "original\n"
    });
    let buffer = cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "edited\n", cx);
                });
                pane.editor_documents.active().unwrap().clone()
            })
    });
    await_close_fixture(view, cx, "edit did not become dirty", |_, _| {
        buffer.read().dirty()
    });
    buffer
}

enum CloseCase {
    Active,
    Stashed,
    Unrelated,
}

fn close_repository_and_save_retained_buffer(cx: &mut gpui::TestAppContext, case: CloseCase) {
    cx.skip_drawing();
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["edited", "unrelated"]
        .into_iter()
        .map(|name| {
            let path = directory.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("main.rs"), "original\n").unwrap();
            gitcomet_core::path_utils::canonicalize_or_original(path)
        })
        .collect();
    let (store, events) = AppStore::new_test(Arc::new(OpeningBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    store.dispatch(Msg::RestoreSession {
        open_repos: paths.clone(),
        active_repo: Some(paths[0].clone()),
    });
    await_close_fixture(&view, cx, "repositories did not open", |view, _| {
        view.state.repos.len() == 2
            && view.state.repos.iter().any(|repo| {
                Some(repo.id) == view.state.active_repo && matches!(repo.open, Loadable::Ready(()))
            })
    });
    let ids: Vec<_> = paths
        .iter()
        .map(|path| {
            store
                .snapshot()
                .repos
                .iter()
                .find(|repo| &repo.spec.workdir == path)
                .unwrap()
                .id
        })
        .collect();
    cx.update(|_, app| {
        view.read(app).diff_renderer.clone().update(app, |pane, _| {
            pane.auto_save_file_edits = false;
        });
    });
    let edited = edit_repository_file(&view, cx, ids[0]);
    let edited_id = edited.id();
    let other = if matches!(case, CloseCase::Stashed) {
        Some(edit_repository_file(&view, cx, ids[1]))
    } else {
        None
    };
    let closed = if matches!(case, CloseCase::Unrelated) {
        ids[1]
    } else {
        ids[0]
    };
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.request_close_repos(vec![closed], None, cx)
        });
    });
    await_close_fixture(
        &view,
        cx,
        "repository close did not reach the view",
        |view, app| {
            view.state.repos.len() == 1
                && view.state.repos[0].id != closed
                && (matches!(case, CloseCase::Unrelated)
                    || (edited.read().repository().is_none()
                        && view.documents.read(app).unsaved_labels(app).len() == 1))
        },
    );
    assert_eq!(store.snapshot().repos.len(), 1);
    assert_eq!(edited.id(), edited_id);
    assert!(edited.read().dirty());
    assert_eq!(
        std::fs::read_to_string(paths[0].join("main.rs")).unwrap(),
        "original\n"
    );
    if matches!(case, CloseCase::Unrelated) {
        assert_eq!(edited.read().repository(), Some(ids[0]));
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.clone();
            assert_eq!(
                pane.read(app).editor_documents.active().unwrap().id(),
                edited_id
            );
            assert_eq!(
                pane.read(app).file_editor_input.read(app).text(),
                "edited\noriginal\n"
            );
            pane.update(app, |pane, cx| assert!(pane.save_all_file_edits(cx)));
        });
    } else {
        assert_eq!(edited.read().repository(), None);
        cx.update(|_, app| {
            let docs = view.read(app).documents.clone();
            let retained = docs
                .read(app)
                .document_owner()
                .at_path(&paths[0].join("main.rs"));
            assert_eq!(retained.len(), 1);
            assert_eq!(retained[0].id(), edited_id);
            assert_eq!(retained[0].stashed().unwrap().text, "edited\noriginal\n");
            assert_eq!(docs.read(app).unsaved_labels(app).len(), 1);
            docs.update(app, |docs, cx| docs.save_all(cx));
        });
    }
    await_close_fixture(&view, cx, "retained save did not finish", |view, app| {
        !edited.read().dirty()
            && edited.read().saving().is_none()
            && view.documents.read(app).saves_drained(app)
    });
    assert_eq!(
        std::fs::read_to_string(paths[0].join("main.rs")).unwrap(),
        "edited\noriginal\n"
    );
    assert!(edited.read().save_error().is_none());
    if let Some(other) = other {
        assert!(other.read().dirty());
        assert_eq!(other.read().repository(), Some(ids[1]));
        assert_eq!(
            std::fs::read_to_string(paths[1].join("main.rs")).unwrap(),
            "original\n"
        );
    }
}

#[gpui::test]
fn closing_the_active_repository_retains_the_same_dirty_buffer_and_saves_it(
    cx: &mut gpui::TestAppContext,
) {
    close_repository_and_save_retained_buffer(cx, CloseCase::Active);
}

#[gpui::test]
fn closing_a_repository_retains_its_stashed_buffer_without_saving_another_repository(
    cx: &mut gpui::TestAppContext,
) {
    close_repository_and_save_retained_buffer(cx, CloseCase::Stashed);
}

#[gpui::test]
fn closing_an_unrelated_repository_keeps_the_active_dirty_buffer_saveable(
    cx: &mut gpui::TestAppContext,
) {
    close_repository_and_save_retained_buffer(cx, CloseCase::Unrelated);
}
