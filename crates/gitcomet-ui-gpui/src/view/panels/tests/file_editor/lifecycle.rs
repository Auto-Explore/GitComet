use super::*;

#[gpui::test]
fn discarding_an_inactive_pending_save_never_restores_empty_edits(cx: &mut gpui::TestAppContext) {
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let workdir = directory.path().canonicalize().unwrap();
    let repo = RepoId(1943);
    let first = Path::new("first.txt");
    let second = Path::new("second.txt");
    for path in [first, second] {
        std::fs::write(workdir.join(path), "original").unwrap();
    }
    let pane = cx.update(|_, app| view.read(app).diff_renderer.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, editor_state(repo, &workdir, first), cx)
        })
    });
    cx.update(|_, app| pane.update(app, |pane, cx| pane.ensure_file_editor_loaded(cx)));
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.auto_save_file_edits = false;
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..input.text().len(), "saved before discard", cx)
            });
        })
    });
    cx.run_until_parked();
    let held = hold_editor_saves(cx.update(|_, app| view.read(app).store.filesystem()));
    cx.update(|_, app| pane.update(app, |pane, cx| pane.save_file_editor_buffer(cx)));
    cx.update(|_, app| {
        view.read(app).ui_model.clone().update(app, |model, cx| {
            model.set_state(editor_state(repo, &workdir, second), cx)
        })
    });
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.ensure_file_editor_loaded(cx);
            pane.discard_file_edits_for(repo, first, cx);
            assert_eq!(pane.editor_documents.pending().len(), 1);
            assert!(pane.unsaved_file_edit_keys().is_empty());
        })
    });
    held.release();
    finish_editor_saves(&view, cx);
    cx.update(|_, app| {
        assert!(
            view.read(app)
                .ui_model
                .read(app)
                .documents
                .unsaved_documents()
                .is_empty()
        );
        view.update(app, |view, cx| {
            push_test_state(view, Arc::new(AppState::test_default()), cx)
        });
    });
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(workdir.join(first)).unwrap(),
        "saved before discard"
    );
    assert_eq!(
        std::fs::read_to_string(workdir.join(second)).unwrap(),
        "original"
    );
}

#[gpui::test]
fn a_save_receipt_before_the_discard_read_cannot_turn_the_placeholder_into_an_edit(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::filesystem::{ItemOutcome, ItemResult, OperationResult};
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let workdir = directory.path().canonicalize().unwrap();
    let path = Path::new("file.txt");
    std::fs::write(workdir.join(path), "original").unwrap();
    let pane = cx.update(|_, app| view.read(app).diff_renderer.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, editor_state(RepoId(1944), &workdir, path), cx)
        })
    });
    cx.update(|_, app| pane.update(app, |pane, cx| pane.ensure_file_editor_loaded(cx)));
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.auto_save_file_edits = true;
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..input.text().len(), "saved revision", cx)
            });
        })
    });
    cx.run_until_parked();
    let held = hold_editor_saves(cx.update(|_, app| view.read(app).store.filesystem()));
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.save_file_editor_buffer(cx);
            let operation = pane.editor_documents.pending()[0]
                .read()
                .saving()
                .as_ref()
                .unwrap()
                .operation;
            pane.discard_file_editor_buffer(cx);
            let generation = pane.file_editor_reread_seq();
            let mut state = (*pane.store.snapshot().expect("window store")).clone();
            state.filesystem.completed.push_back(OperationResult {
                id: operation,
                saved_version: None,
                moved_versions: Default::default(),
                items: vec![ItemResult {
                    source: workdir.join(path),
                    destination: None,
                    outcome: ItemOutcome::Completed,
                }],
                changes: Vec::new(),
                undo_available: false,
                redo_available: false,
            });
            pane.process_file_editor_saves(&state, cx);
            assert!(
                pane.editor_documents
                    .active()
                    .unwrap()
                    .read()
                    .accepts_read(generation)
            );
            pane.on_file_editor_edited(cx);
            assert!(pane.file_editor_loading());
            assert!(!pane.file_editor_dirty());
            assert!(pane.file_editor_autosave.is_none());
        })
    });
    held.release();
    cx.run_until_parked();
    cx.update(|_, app| {
        let pane = pane.read(app);
        assert!(!pane.file_editor_loading());
        assert!(!pane.file_editor_input.read(app).text().is_empty());
        assert!(!pane.file_editor_dirty());
    });
}

#[gpui::test]
async fn closing_a_repository_retains_its_unsaved_buffer_in_documents(
    cx: &mut gpui::TestAppContext,
) {
    // Regression: a stash entry whose repo tab had closed stayed dirty forever.
    // The quit dialog listed it, "Save all" could not write it (the store drops
    // messages for a repo it no longer has), and the retry raised the dialog
    // again — the window could never be closed.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(956);
    let workdir = unique_workdir("file_editor_closed_repo");
    let file_rel = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file_rel), "fn main() {}\n").expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// edited\n", cx);
            });
        });
    });
    cx.run_until_parked();

    // Leaving the file stashes the edit, then the repo tab closes under it.
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| pane.stash_current_file_editor_buffer(cx));
    });
    cx.update(|_window, app| {
        assert!(
            !diff_renderer
                .read(app)
                .unsaved_file_edit_labels()
                .is_empty(),
            "the stashed edit is unsaved while its repo is still open"
        );
        view.update(app, |this, cx| {
            let mut state = AppState::test_default();
            state.active_repo = None;
            push_test_state(this, Arc::new(state), cx);
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert!(
            pane.unsaved_file_edit_labels().is_empty(),
            "a buffer with no repo left to save it into must not block the quit dialog"
        );
    });

    cx.update(|_window, app| {
        let docs = view.read(app).documents.read(app);
        assert_eq!(
            docs.unsaved_labels(app),
            vec![SharedString::from(
                workdir.join(&file_rel).display().to_string()
            )]
        );
    });
    assert_eq!(
        std::fs::read_to_string(workdir.join(&file_rel)).unwrap(),
        "fn main() {}\n"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn review_regression_confirmed_noop_move_skips_destructive_guards(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = gitcomet_state::model::RepoId(965);
    let workdir = unique_workdir("file_editor_noop_move_target");
    let file = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file), "fn main() {}\n").expect("write fixture");

    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file), cx);
            this.diff_renderer.update(cx, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// must survive\n", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let own_workspace = this.workspace_id.expect("source workspace");
            this.request_move_repo_to_workspace(repo_id, workdir.clone(), Some(own_workspace), cx);
            assert!(
                this.pending_unsaved_file_edits_prompt.is_none(),
                "a move within the same workspace must not offer to discard edits"
            );
            assert_eq!(
                this.diff_renderer
                    .read(cx)
                    .unsaved_file_edit_labels_for_repo(repo_id),
                vec![SharedString::from("main.rs")],
                "the no-op move must leave the dirty buffer untouched"
            );
        });
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn review_regression_followup_saving_before_a_move_only_writes_the_moved_repository(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let first_id = gitcomet_state::model::RepoId(963);
    let second_id = gitcomet_state::model::RepoId(964);
    let first_dir = unique_workdir("file_editor_scoped_move_save_a");
    let second_dir = unique_workdir("file_editor_scoped_move_save_b");
    let first_file = std::path::PathBuf::from("a.rs");
    let second_file = std::path::PathBuf::from("b.rs");
    std::fs::write(first_dir.join(&first_file), "fn a() {}\n").expect("write first fixture");
    std::fs::write(second_dir.join(&second_file), "fn b() {}\n").expect("write second fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                two_repo_editor_state(
                    (first_id, &first_dir, &first_file),
                    (second_id, &second_dir, &second_file),
                    first_id,
                ),
                cx,
            );
            this.diff_renderer.update(cx, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        });
    });
    cx.run_until_parked();
    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// unsaved A\n", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                two_repo_editor_state(
                    (first_id, &first_dir, &first_file),
                    (second_id, &second_dir, &second_file),
                    second_id,
                ),
                cx,
            );
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// unsaved B\n", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        let action = UnsavedFileEditsAction::MoveRepo {
            window_id: window.window_handle().window_id(),
            repo_id: first_id,
            path: first_dir.clone(),
            target_workspace: None,
        };
        view.update(app, |this, cx| {
            assert_eq!(
                this.diff_renderer
                    .read(cx)
                    .unsaved_file_edit_labels_for_repo(first_id)
                    .len(),
                1
            );
            assert_eq!(
                this.diff_renderer
                    .read(cx)
                    .unsaved_file_edit_labels_for_repo(second_id)
                    .len(),
                1
            );
            this.resolve_unsaved_file_edits(action, true, cx);
            assert!(
                !this
                    .diff_renderer
                    .read(cx)
                    .unsaved_file_edit_labels_for_repo(second_id)
                    .is_empty(),
                "saving repository A for a move must leave repository B's dirty buffer untouched"
            );
        });
    });

    let _ = std::fs::remove_dir_all(&first_dir);
    let _ = std::fs::remove_dir_all(&second_dir);
}

#[gpui::test]
fn pr530_close_requested_during_move_drain_is_retried(cx: &mut gpui::TestAppContext) {
    check_shutdown_requested_during_move_drain(cx, false);
}

#[gpui::test]
fn pr530_quit_requested_during_move_drain_is_retried(cx: &mut gpui::TestAppContext) {
    check_shutdown_requested_during_move_drain(cx, true);
}

#[gpui::test]
fn review_regression_move_waits_for_dispatched_manual_saves(cx: &mut gpui::TestAppContext) {
    check_move_waits_for_existing_saves(cx, false, true);
}

#[gpui::test]
fn review_regression_failed_dispatched_manual_save_keeps_move_recovery(
    cx: &mut gpui::TestAppContext,
) {
    check_move_waits_for_existing_saves(cx, false, false);
}

#[gpui::test]
fn review_regression_move_waits_for_dispatched_auto_saves(cx: &mut gpui::TestAppContext) {
    check_move_waits_for_existing_saves(cx, true, true);
}

#[gpui::test]
fn review_regression_failed_dispatched_auto_save_keeps_move_recovery(
    cx: &mut gpui::TestAppContext,
) {
    check_move_waits_for_existing_saves(cx, true, false);
}

#[gpui::test]
fn review_regression_discard_after_save_timeout_closes_once_written(cx: &mut gpui::TestAppContext) {
    check_discard_after_save_timeout(cx, |window_id, _, _, _| {
        UnsavedFileEditsAction::CloseWindow(window_id)
    });
}

#[gpui::test]
fn review_regression_discard_after_save_timeout_quits_once_written(cx: &mut gpui::TestAppContext) {
    check_discard_after_save_timeout(cx, |_, _, _, _| UnsavedFileEditsAction::QuitApp);
}

#[gpui::test]
fn review_regression_discard_after_save_timeout_deletes_workspace_once_written(
    cx: &mut gpui::TestAppContext,
) {
    check_discard_after_save_timeout(cx, |window_id, _, _, app| {
        UnsavedFileEditsAction::DeleteWorkspace {
            window_id,
            workspace_id: crate::workspaces::workspace_for_window(app, window_id)
                .expect("the window's workspace")
                .id,
        }
    });
}

#[gpui::test]
fn review_regression_discard_after_save_timeout_still_waits_before_move(
    cx: &mut gpui::TestAppContext,
) {
    check_discard_after_save_timeout(cx, |window_id, repo_id, path, _| {
        UnsavedFileEditsAction::MoveRepo {
            window_id,
            repo_id,
            path: path.to_path_buf(),
            target_workspace: None,
        }
    });
}

#[gpui::test]
async fn unsaved_edits_are_reported_and_discardable_per_file(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(960);
    let workdir = unique_workdir("file_editor_unsaved_paths");
    let first = std::path::PathBuf::from("a.rs");
    let second = std::path::PathBuf::from("b.rs");
    std::fs::write(workdir.join(&first), "fn a() {}\n").expect("write fixture");
    std::fs::write(workdir.join(&second), "fn b() {}\n").expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &first), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        assert!(
            diff_renderer
                .read(app)
                .unsaved_file_edit_paths(repo_id)
                .is_empty(),
            "a clean buffer is not an unsaved edit"
        );
    });

    // Edit `a.rs`, then move to `b.rs` so the first buffer goes to the stash.
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// a\n", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &second), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    // Now edit `b.rs` too: one stashed dirty buffer and one on screen.
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// b\n", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert_eq!(
            pane.unsaved_file_edit_paths(repo_id),
            vec![first.clone(), second.clone()],
            "both the stashed buffer and the one on screen are reported, path-sorted"
        );
        assert!(pane.file_edits_are_unsaved_for(repo_id, &first));
        assert!(pane.file_edits_are_unsaved_for(repo_id, &second));
        assert!(
            !pane.file_edits_are_unsaved_for(gitcomet_state::model::RepoId(961), &first),
            "another repo's tab holding the same relative path is a different file"
        );
    });

    // Discarding the *stashed* one must work without it being on screen.
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.discard_file_edits_for(repo_id, &first, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert_eq!(
            pane.unsaved_file_edit_paths(repo_id),
            vec![second.clone()],
            "the stashed buffer is gone and the on-screen one is untouched"
        );
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            "// b\nfn b() {}\n",
            "discarding another file must not disturb the buffer on screen"
        );
    });

    // And discarding the on-screen one re-reads it from disk.
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.discard_file_edits_for(repo_id, &second, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert!(pane.unsaved_file_edit_paths(repo_id).is_empty());
        assert!(!pane.file_editor_is_dirty());
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            "fn b() {}\n",
            "the file came back from disk"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A long action in another repository must not hold a close whose own
/// write has already landed.
#[gpui::test]
async fn review_regression_unrelated_repo_action_does_not_block_close(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (edited, busy) = (
        gitcomet_state::model::RepoId(971),
        gitcomet_state::model::RepoId(972),
    );
    let edited_dir = unique_workdir("file_editor_close_edited");
    let busy_dir = unique_workdir("file_editor_close_busy");
    let file = std::path::PathBuf::from("main.rs");
    std::fs::write(edited_dir.join(&file), "fn main() {}\n").expect("write fixture");
    std::fs::write(busy_dir.join(&file), "fn main() {}\n").expect("write fixture");
    let mut state = (*two_repo_editor_state(
        (edited, &edited_dir, &file),
        (busy, &busy_dir, &file),
        edited,
    ))
    .clone();
    state.repos[1].local_actions_in_flight = 1;

    let action =
        request_action_with_dirty_auto_saved_buffer(cx, &view, Arc::new(state), |window_id, _| {
            UnsavedFileEditsAction::CloseWindow(window_id)
        });
    let UnsavedFileEditsAction::CloseWindow(window_id) = action else {
        unreachable!()
    };
    finish_editor_saves(&view, cx);
    advance_file_edit_drain(&view, cx, std::time::Duration::from_millis(200));

    assert!(
        cx.cx.update(|app| app
            .windows()
            .iter()
            .all(|window| window.window_id() != window_id)),
        "the close must go through once the edited repository's write landed"
    );

    let _ = std::fs::remove_dir_all(&edited_dir);
    let _ = std::fs::remove_dir_all(&busy_dir);
}

#[gpui::test]
async fn review_regression_confirmed_failed_save_aborts_repository_move(
    cx: &mut gpui::TestAppContext,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(966);
    let workdir = unique_workdir("file_editor_failed_move_save");
    let file = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file), "fn main() {}\n").expect("write fixture");
    let source_window_id = cx.update(|window, app| {
        crate::app::install_app_shortcuts_for_test(app, Arc::clone(&backend));
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file), cx);
            this.diff_renderer.update(cx, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        });
        let _ = window.draw(app);
        window.window_handle().window_id()
    });
    cx.run_until_parked();

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// unsaved\n", cx);
            });
        });
    });
    cx.run_until_parked();

    let action = UnsavedFileEditsAction::MoveRepo {
        window_id: source_window_id,
        repo_id,
        path: workdir.clone(),
        target_workspace: None,
    };
    // The file changes before the worker gets to the save, which refuses it.
    let held = hold_editor_saves(cx.update(|_, app| view.read(app).store.filesystem()));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.resolve_unsaved_file_edits(action, true, cx);
        });
    });
    std::fs::write(workdir.join(&file), "fn main() { theirs(); }\n").expect("external write");
    held.release();
    // Start the spawned drain waiter so its first deterministic timer exists
    // before the test advances GPUI's clock below.
    cx.run_until_parked();
    finish_editor_saves(&view, cx);
    advance_file_edit_drain(&view, cx, std::time::Duration::from_millis(200));

    assert!(
        cx.cx.update(|app| app
            .windows()
            .iter()
            .any(|window| window.window_id() == source_window_id)),
        "a failed editor write must leave the source window and repository intact"
    );
    cx.update(|_window, app| {
        assert!(
            !diff_renderer
                .read(app)
                .unsaved_file_edit_labels_for_repo(repo_id)
                .is_empty(),
            "the refused save must remain visibly unsaved"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Edit with auto-save on and request an action. The flushed save stays
/// queued until the test pumps it with `finish_editor_saves`.
fn request_action_with_dirty_auto_saved_buffer(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    state: Arc<AppState>,
    make_action: impl FnOnce(gpui::WindowId, &gpui::App) -> UnsavedFileEditsAction,
) -> UnsavedFileEditsAction {
    let window_id = cx.update(|window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, state, cx);
            this.diff_renderer.update(cx, |pane, cx| {
                pane.auto_save_file_edits = true;
                pane.ensure_file_editor_loaded(cx);
            });
        });
        let _ = window.draw(app);
        window.window_handle().window_id()
    });
    cx.run_until_parked();
    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// edited\n", cx);
            });
        });
    });
    cx.run_until_parked();
    let action = cx.update(|_window, app| make_action(window_id, app));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(
                this.request_unsaved_file_edits_prompt(action.clone(), cx),
                "the pending auto-save write must hold the action"
            );
            assert!(
                !this
                    .diff_renderer
                    .read(cx)
                    .editor_documents
                    .pending()
                    .is_empty(),
                "the flush queues the write"
            );
        })
    });
    // Start the drain waiter before the clock moves.
    cx.run_until_parked();
    action
}

fn advance_file_edit_drain(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    total: std::time::Duration,
) {
    crate::view::test_support::drain_store_worker(view, cx);
    let step = std::time::Duration::from_millis(25);
    let mut elapsed = std::time::Duration::ZERO;
    while elapsed < total {
        cx.executor().advance_clock(step);
        cx.run_until_parked();
        elapsed += step;
    }
}

fn check_shutdown_requested_during_move_drain(cx: &mut gpui::TestAppContext, quit: bool) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_, app| crate::app::install_app_shortcuts_for_test(app, backend));
    let directory = tempfile::tempdir().unwrap();
    let workdir = directory.path().canonicalize().unwrap();
    let file = PathBuf::from("main.rs");
    let repo_id = gitcomet_state::model::RepoId(981);
    std::fs::write(workdir.join(&file), "original\n").unwrap();
    request_action_with_dirty_auto_saved_buffer(
        cx,
        &view,
        editor_state(repo_id, &workdir, &file),
        |window_id, _| UnsavedFileEditsAction::MoveRepo {
            window_id,
            repo_id,
            path: workdir.clone(),
            target_workspace: None,
        },
    );
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            if quit {
                assert!(view.request_quit_unsaved_file_edits_prompt(cx));
            } else {
                assert!(view.request_close_window_or_warn(window.window_handle().window_id(), cx));
            }
        })
    });
    finish_editor_saves(&view, cx);
    advance_file_edit_drain(&view, cx, std::time::Duration::from_millis(100));
    cx.cx.update(|app| {
        if quit {
            // The test platform stubs quit; the retry must leave this window in
            // place rather than moving its repository to a newly opened window.
            assert_eq!(app.windows().len(), 1);
            view.update(app, |view, cx| {
                assert!(view.pending_file_edits_action.is_none());
                assert!(!view.request_quit_unsaved_file_edits_prompt(cx));
                assert_eq!(view.state.repos[0].id, repo_id);
            });
        } else {
            assert!(
                app.windows().is_empty(),
                "the close must supersede the move"
            );
        }
    });
}

fn check_move_waits_for_existing_saves(
    cx: &mut gpui::TestAppContext,
    auto_save: bool,
    succeeds: bool,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(976);
    let directory = tempfile::tempdir().unwrap();
    let workdir = directory.path().canonicalize().unwrap();
    let first = PathBuf::from("first.rs");
    let second = PathBuf::from("second.rs");
    let window_id = cx.update(|window, app| {
        crate::app::install_app_shortcuts_for_test(app, backend);
        window.window_handle().window_id()
    });
    let open = |cx: &mut gpui::VisualTestContext, path: &Path| {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                push_test_state(view, editor_state(repo_id, &workdir, path), cx);
                view.diff_renderer
                    .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
            })
        });
        cx.run_until_parked();
    };
    for path in [&first, &second] {
        std::fs::write(workdir.join(path), "original\n").unwrap();
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.diff_renderer
                    .update(cx, |pane, _| pane.auto_save_file_edits = false);
            })
        });
        open(cx, path);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.diff_renderer.update(cx, |pane, cx| {
                    pane.file_editor_input.update(cx, |input, cx| {
                        input.replace_utf8_range(0..0, "edited\n", cx);
                    });
                });
            })
        });
        cx.run_until_parked();
    }

    // Both writes stay queued while held. Pushing test state replaces the
    // store's results too, so every push happens before the writes can land.
    let held = hold_editor_saves(cx.update(|_, app| view.read(app).store.filesystem()));
    let dispatch = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.diff_renderer.update(cx, |pane, cx| {
                    if auto_save {
                        pane.auto_save_file_edits = true;
                        assert!(pane.flush_file_editor_buffer(cx));
                    } else {
                        assert!(pane.save_all_file_edits(cx));
                    }
                })
            })
        });
    };
    dispatch(cx);
    open(cx, &first);
    if auto_save {
        dispatch(cx);
    }
    cx.update(|_, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.editor_documents.pending().len(), 2);
        assert_eq!(
            pane.unsaved_file_edit_keys().len(),
            2,
            "unconfirmed saves stay unsaved"
        );
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            "edited\noriginal\n",
            "reopening a pending save must not reload old disk contents"
        );
    });
    if !succeeds {
        // The first file changes before the worker gets to it, so its save is
        // refused as a conflict.
        std::fs::write(workdir.join(&first), "changed elsewhere\n").unwrap();
    }
    held.release();

    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.request_move_repo_to_workspace(repo_id, workdir.clone(), None, cx);
            assert!(view.pending_unsaved_file_edits_flush.is_some());
        })
    });
    cx.run_until_parked();
    cx.cx.update(|app| {
        assert_eq!(app.windows().len(), 1, "queued saves must block the move");
    });

    finish_editor_saves(&view, cx);
    advance_file_edit_drain(&view, cx, std::time::Duration::from_millis(50));
    cx.cx.update(|app| {
        let source_exists = app
            .windows()
            .iter()
            .any(|window| window.window_id() == window_id);
        assert_eq!(source_exists, !succeeds);
        let owners = crate::app::windows_owning_repo_for_test(app, &workdir);
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0] == window_id, !succeeds);
        if !succeeds {
            let pane = view.read(app).diff_renderer.read(app);
            assert!(pane.file_editor_dirty());
            assert_eq!(
                pane.file_editor_input.read(app).text(),
                "edited\noriginal\n"
            );
            assert_eq!(pane.unsaved_file_edit_paths(repo_id), vec![first.clone()]);
        }
    });
}

/// A save that has not reported back brings the prompt back after the
/// timeout. Discarding cannot cancel the queued write, so every action still
/// waits for it to land.
fn check_discard_after_save_timeout(
    cx: &mut gpui::TestAppContext,
    make_action: impl FnOnce(
        gpui::WindowId,
        gitcomet_state::model::RepoId,
        &Path,
        &gpui::App,
    ) -> UnsavedFileEditsAction,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(973);
    let directory = tempfile::tempdir().unwrap();
    let workdir = directory.path().canonicalize().unwrap();
    let file = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file), "fn main() {}\n").expect("write fixture");

    let window_id = cx.update(|window, app| {
        crate::app::install_app_shortcuts_for_test(app, backend);
        window.window_handle().window_id()
    });
    let action = request_action_with_dirty_auto_saved_buffer(
        cx,
        &view,
        editor_state(repo_id, &workdir, &file),
        |window_id, app| make_action(window_id, repo_id, &workdir, app),
    );
    advance_file_edit_drain(&view, cx, std::time::Duration::from_secs(6));

    assert!(
        cx.cx.update(|app| app
            .windows()
            .iter()
            .any(|window| window.window_id() == window_id)),
        "an unconfirmed write must not close the window"
    );
    cx.update(|_window, app| {
        // Render moves a queued prompt into the popover host.
        let view = view.read(app);
        let queued = view.pending_unsaved_file_edits_prompt.clone();
        let shown = match view.popover_host.read(app).popover_kind_for_tests() {
            Some(PopoverKind::UnsavedFileEditsConfirm(prompt)) => Some(prompt),
            _ => None,
        };
        let prompt = queued.or(shown).expect("timeout prompt");
        assert_eq!(prompt.action, action);
        assert_eq!(
            prompt.files,
            vec![gpui::SharedString::from("main.rs")],
            "the timeout must hand the decision back to the user"
        );
    });

    cx.update(|_window, app| {
        let host = view.read(app).popover_host.clone();
        // Match the discard button's ordering, including the deferred clear.
        host.update(app, |host, cx| {
            view.update(cx, |view, cx| {
                view.resolve_unsaved_file_edits(action.clone(), false, cx);
                assert!(
                    view.diff_renderer
                        .read(cx)
                        .unsaved_file_edit_keys()
                        .is_empty()
                );
                assert!(
                    !view
                        .diff_renderer
                        .read(cx)
                        .editor_documents
                        .pending()
                        .is_empty(),
                    "discard cannot cancel the queued write"
                );
            });
            host.close_popover(cx);
        });
    });
    cx.run_until_parked();
    cx.cx.update(|app| {
        assert!(
            app.windows()
                .iter()
                .any(|window| window.window_id() == window_id)
        );
        assert!(
            view.read(app).pending_unsaved_file_edits_flush.is_some(),
            "the retried action waits for the queued write"
        );
    });
    if matches!(action, UnsavedFileEditsAction::MoveRepo { .. }) {
        advance_file_edit_drain(&view, cx, std::time::Duration::from_secs(6));
        cx.update(|_, app| {
            let view = view.read(app);
            let queued = view.pending_unsaved_file_edits_prompt.clone();
            let shown = match view.popover_host.read(app).popover_kind_for_tests() {
                Some(PopoverKind::UnsavedFileEditsConfirm(prompt)) => Some(prompt),
                _ => None,
            };
            assert!(
                queued
                    .or(shown)
                    .is_some_and(|prompt| prompt.waiting_for_writes),
                "a discarded write that still blocks a move must show a timeout prompt"
            );
        });
        cx.update(|_, app| {
            let host = view.read(app).popover_host.clone();
            host.update(app, |host, cx| {
                view.update(cx, |view, cx| {
                    view.resolve_unsaved_file_edits(action.clone(), false, cx)
                });
                host.close_popover(cx);
            });
        });
        cx.run_until_parked();
    }

    finish_editor_saves(&view, cx);
    advance_file_edit_drain(&view, cx, std::time::Duration::from_millis(100));
    cx.cx.update(|app| match action {
        UnsavedFileEditsAction::CloseWindow(_) => {
            assert!(
                app.windows()
                    .iter()
                    .all(|window| window.window_id() != window_id),
                "the close goes through once the write lands"
            );
        }
        UnsavedFileEditsAction::DeleteWorkspace { workspace_id, .. } => {
            // The only window returns to Home instead of closing.
            assert!(
                app.windows()
                    .iter()
                    .any(|window| window.window_id() == window_id)
            );
            assert!(
                crate::workspaces::workspace(app, workspace_id).is_none(),
                "the delete goes through once the write lands"
            );
            assert_eq!(view.read(app).workspace_id, None);
        }
        UnsavedFileEditsAction::QuitApp => {
            // GPUI's test platform stubs quit, so verify the quit guard lets
            // the retried action through without another drain or prompt.
            view.update(app, |view, cx| {
                assert!(view.pending_unsaved_file_edits_flush.is_none());
                assert!(!view.request_quit_unsaved_file_edits_prompt(cx));
            });
        }
        UnsavedFileEditsAction::MoveRepo { .. } => {
            assert!(
                app.windows()
                    .iter()
                    .all(|window| window.window_id() != window_id)
            );
            let owners = crate::app::windows_owning_repo_for_test(app, &workdir);
            assert_eq!(owners.len(), 1);
            assert_ne!(owners[0], window_id);
        }
    });
}
