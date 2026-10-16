use super::*;

#[cfg(unix)]
#[gpui::test]
fn repository_editor_rejects_saves_through_a_symlinked_directory(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("repo");
    let external = directory.path().join("external");
    std::fs::create_dir(&repository).unwrap();
    std::fs::create_dir(&external).unwrap();
    std::fs::write(external.join("file.txt"), "external contents\n").unwrap();
    std::os::unix::fs::symlink(&external, repository.join("historical-folder")).unwrap();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(
                view,
                editor_state(
                    gitcomet_state::model::RepoId(997),
                    &repository,
                    Path::new("historical-folder/file.txt"),
                ),
                cx,
            );
            view.diff_renderer.update(cx, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                assert_eq!(
                    pane.file_editor_input.read(cx).text(),
                    "external contents\n"
                );
                pane.file_editor_input
                    .update(cx, |input, cx| input.replace_utf8_range(0..0, "edit ", cx));
            })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| pane.save_file_editor_buffer(cx))
    });
    finish_editor_saves(&view, cx);
    cx.update(|_, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(pane.file_editor_dirty());
        assert!(pane.file_editor_error().is_some());
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            "edit external contents\n"
        );
    });
    assert_eq!(
        std::fs::read_to_string(external.join("file.txt")).unwrap(),
        "external contents\n"
    );
}

#[gpui::test]
fn failed_autosaves_reach_the_unsaved_edits_prompt_without_retrying_forever(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let directory = tempfile::tempdir().unwrap();
    let repo_id = gitcomet_state::model::RepoId(1941);
    let relative = PathBuf::from("file.txt");
    let path = directory.path().join(&relative);
    std::fs::write(&path, "original").unwrap();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, editor_state(repo_id, directory.path(), &relative), cx);
            view.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.auto_save_file_edits = true;
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..input.text().len(), "unsaved edits", cx);
                });
            })
    });
    cx.run_until_parked();
    // A directory at the save path fails even for privileged test users.
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| pane.save_file_editor_buffer(cx))
    });
    finish_editor_saves(&view, cx);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            assert!(view.diff_renderer.read(cx).file_editor_is_dirty());
            assert!(view.diff_renderer.read(cx).file_editor_error().is_some());
            assert!(view.request_unsaved_file_edits_prompt(UnsavedFileEditsAction::QuitApp, cx));
            assert!(
                view.pending_unsaved_file_edits_prompt.is_some(),
                "a failed autosave must allow Save/Discard"
            );
            assert!(
                view.diff_renderer
                    .read(cx)
                    .editor_documents
                    .pending()
                    .is_empty(),
                "closing must not silently retry the failed write"
            );
            assert_eq!(
                view.diff_renderer
                    .read(cx)
                    .file_editor_input
                    .read(cx)
                    .text(),
                "unsaved edits"
            );
        })
    });
}

#[gpui::test]
fn save_acknowledgments_release_clean_stashes_but_keep_newer_edits(cx: &mut gpui::TestAppContext) {
    save_completion_after_navigation(cx, false, false);
}

#[gpui::test]
fn autosave_completion_saves_newer_edits_after_navigating_away(cx: &mut gpui::TestAppContext) {
    save_completion_after_navigation(cx, true, false);
}

#[gpui::test]
fn stashed_autosaves_resume_after_the_final_filesystem_pause(cx: &mut gpui::TestAppContext) {
    save_completion_after_navigation(cx, true, true);
}

#[gpui::test]
fn releasing_filesystem_pauses_keeps_manual_stashed_edits_unsaved(cx: &mut gpui::TestAppContext) {
    save_completion_after_navigation(cx, false, true);
}

#[gpui::test]
async fn file_editor_marks_dirty_on_edit_and_clean_after_save(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(941);
    let workdir = unique_workdir("file_editor_dirty");
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

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                assert!(!pane.file_editor_is_dirty());
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "// header\n", cx);
                });
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(pane.file_editor_is_dirty(), "typing marks the buffer dirty");
        assert_eq!(
            pane.unsaved_file_edit_labels(),
            vec![SharedString::from("main.rs")],
            "the edited file is reported as unsaved"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer
                .update(cx, |pane, cx| pane.save_file_editor_buffer(cx));
        });
    });

    finish_editor_saves(&view, cx);
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            !pane.file_editor_is_dirty(),
            "a confirmed save settles the buffer"
        );
        assert!(pane.unsaved_file_edit_labels().is_empty());
        assert!(
            pane.file_editor_stash().is_empty(),
            "acknowledged clean buffers must be released"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn file_editor_keeps_an_unsaved_buffer_across_a_file_switch(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(942);
    let workdir = unique_workdir("file_editor_stash");
    let first = std::path::PathBuf::from("first.rs");
    let second = std::path::PathBuf::from("second.rs");
    std::fs::write(workdir.join(&first), "fn first() {}\n").expect("write first");
    std::fs::write(workdir.join(&second), "fn second() {}\n").expect("write second");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &first), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    // Edit the first file without saving...
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "// edited\n", cx);
                });
            });
        });
    });
    cx.run_until_parked();

    // ...move to the second...
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &second), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "fn second() {}\n");
        assert_eq!(
            pane.unsaved_file_edit_labels(),
            vec![SharedString::from("first.rs")],
            "the edit left behind is still tracked as unsaved"
        );
    });

    // ...and back: the edit is still there.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &first), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            "// edited\nfn first() {}\n",
            "returning to a file restores the unsaved buffer, not the file on disk"
        );
        assert!(pane.file_editor_is_dirty());
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn alt_e_toggles_the_editor_and_ctrl_s_saves_and_exits_while_it_has_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(944);
    let workdir = unique_workdir("file_editor_shortcuts");
    let file_rel = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file_rel), "fn main() {}\n").expect("write fixture");

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    let press = |cx: &mut gpui::VisualTestContext, chord: &str| -> bool {
        let keystroke = gpui::Keystroke::parse(chord).expect("valid chord");
        cx.update(|window, app| {
            diff_renderer.update(app, |pane, cx| {
                pane.handle_diff_shortcut(&keystroke, window, cx)
            })
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    // Alt+E is routed to the pane at all…
    assert!(
        crate::view::is_diff_shortcut_candidate(
            &gpui::Keystroke::parse("alt-e").expect("valid chord")
        ),
        "alt-e must reach the diff shortcut table"
    );

    // …and leaves the editor when it is already up.
    assert!(press(cx, "alt-e"));

    // Ctrl/Cmd+S saves from the focused buffer; status_staging covers the
    // other focus places, where it saves instead of staging.
    cx.update(|window, app| {
        diff_renderer.update(app, |pane, cx| {
            let handle = pane.file_editor_input.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        });
    });
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// edited\n", cx);
            });
        });
    });
    cx.run_until_parked();
    assert!(cx.update(|_window, app| diff_renderer.read(app).file_editor_is_dirty()));

    assert!(
        press(cx, "secondary-s"),
        "ctrl/cmd-s saves inside the editor"
    );
    finish_editor_saves(&view, cx);
    assert!(
        cx.update(|_window, app| !diff_renderer.read(app).file_editor_is_dirty()),
        "the buffer settles once saved"
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
    });
    cx.run_until_parked();
    assert!(
        cx.update(|_window, app| !diff_renderer.read(app).is_file_editor_active()),
        "saving from the editor returns to the view that opened it"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn typing_and_undoing_returns_the_buffer_to_clean(cx: &mut gpui::TestAppContext) {
    // Regression: the fingerprint used to be hashed chunk-by-chunk, and the
    // rope's chunk boundaries depend on edit history — so text that was once
    // byte-identical to disk hashed differently and the buffer could never
    // settle. It has to be big enough to span several chunks (512 bytes each).
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(945);
    let workdir = unique_workdir("file_editor_fingerprint");
    let file_rel = std::path::PathBuf::from("big.rs");
    let contents = "fn line() { let value = 1; }\n".repeat(120);
    assert!(contents.len() > 2048, "must span several rope chunks");
    std::fs::write(workdir.join(&file_rel), &contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    // Type into the middle of the file, then take it back out again.
    let middle = contents.len() / 2;
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(middle..middle, "xyz", cx);
                });
            });
        });
    });
    cx.run_until_parked();
    assert!(
        cx.update(|_window, app| view
            .read(app)
            .diff_renderer
            .read(app)
            .file_editor_is_dirty()),
        "the inserted text must read as modified"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(middle..middle + 3, "", cx);
                });
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), contents);
        assert!(
            !pane.file_editor_is_dirty(),
            "text identical to disk must read as clean whatever the rope's chunking"
        );
        assert!(pane.unsaved_file_edit_labels().is_empty());
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn the_load_placeholder_is_never_saved_over_the_file(cx: &mut gpui::TestAppContext) {
    // Regression: switching files left the *previous* file's dirty flag set
    // while the buffer held the blank placeholder for the new one, so a save in
    // that window wrote an empty string over the file just opened. The window is
    // a scheduling accident, so this reconstructs the state it produced rather
    // than trying to race it.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(954);
    let workdir = unique_workdir("file_editor_placeholder");
    let file_rel = std::path::PathBuf::from("target.rs");
    let contents = "fn target() {}\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

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
            // Exactly the mid-load state: the key names this file, the buffer is
            // the blank placeholder, and a stale dirty flag says it is unsaved.
            pane.file_editor_input.update(cx, |input, cx| {
                input.set_text("", cx);
            });
            let document = pane.editor_documents.active().unwrap();
            document.seat_text(1, Some(2), cx);
            document.begin_read(true, cx);

            assert!(
                pane.unsaved_file_edit_labels()
                    .iter()
                    .all(|label| label.as_ref() != "target.rs"),
                "a placeholder must not be offered to the user as an unsaved edit"
            );

            // Every save entry point must refuse while the read is in flight.
            pane.save_file_editor_buffer(cx);
            pane.save_all_file_edits(cx);
        });
    });
    cx.run_until_parked();

    assert_eq!(
        std::fs::read_to_string(workdir.join(&file_rel)).expect("read fixture"),
        contents,
        "the file must be untouched by a save that landed during its load"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn saving_keeps_the_caret_and_the_undo_stack(cx: &mut gpui::TestAppContext) {
    // Regression: a save is followed by a disk check (the write bumps the
    // repo's revisions), and an earlier version of that follow-up blanked the
    // input before re-reading, so every save re-seated the buffer — resetting
    // the caret to 0 and clearing undo, once per auto-save.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(955);
    let workdir = unique_workdir("file_editor_save_caret");
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
                input.replace_utf8_range(0..0, "// header\n", cx);
                input.set_cursor_offset(5, cx);
            });
        });
    });
    cx.run_until_parked();

    // `set_text` mints a fresh model id, so an unchanged id is proof the buffer
    // was never re-seated — and therefore that the undo stack, which lives on
    // the widget and is cleared by a re-seat, survived too.
    let model_id_before = cx.update(|_window, app| {
        diff_renderer
            .read(app)
            .file_editor_input
            .read(app)
            .text_snapshot()
            .model_id()
    });

    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| pane.save_file_editor_buffer(cx));
    });
    finish_editor_saves(&view, cx);
    // Let the save land and the follow-up re-read run to completion.
    cx.run_until_parked();
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| pane.ensure_file_editor_loaded(cx));
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert_eq!(
            pane.file_editor_input.read(app).cursor_offset(),
            5,
            "the caret must not jump on save"
        );
        assert!(!pane.file_editor_is_dirty());
        assert_eq!(
            pane.file_editor_input.read(app).text_snapshot().model_id(),
            model_id_before,
            "the buffer must not be re-seated by a save — that clears undo"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn review_regression_moving_a_repository_prompts_before_detaching_dirty_editor_buffers(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(962);
    let workdir = unique_workdir("file_editor_move_dirty_repo");
    let first = std::path::PathBuf::from("a.rs");
    let second = std::path::PathBuf::from("b.rs");
    std::fs::write(workdir.join(&first), "fn a() {}\n").expect("write fixture");
    std::fs::write(workdir.join(&second), "fn b() {}\n").expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &first), cx);
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
                input.replace_utf8_range(0..0, "// unsaved a\n", cx);
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
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// unsaved b\n", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.request_move_repo_to_workspace(repo_id, workdir.clone(), None, cx);
            let prompt = this
                .pending_unsaved_file_edits_prompt
                .as_ref()
                .expect("moving a dirty repository must queue the unsaved-edits prompt");
            assert_eq!(
                prompt.files.len(),
                2,
                "both the stashed and current dirty buffers must be protected"
            );
        });
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn discarding_from_the_toolbar_returns_to_the_read_only_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(964);
    let workdir = unique_workdir("file_editor_discard_exits");
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
                input.replace_utf8_range(0..0, "// scratch\n", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert!(diff_renderer.read(app).file_editor_is_dirty());
    });

    cx.update(|window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.discard_file_editor_buffer_and_exit(window, cx);
        });
    });
    cx.run_until_parked();
    // The exit goes through the store, so wait for its worker to reduce it and
    // pull that snapshot back into the view the way a real frame would.
    crate::view::test_support::drain_store_worker(&view, cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert!(
            !pane.file_editor_is_dirty(),
            "the buffer is thrown away, not stashed"
        );
        assert!(
            pane.unsaved_file_edit_paths(repo_id).is_empty(),
            "and nothing is left behind for the explorer to pin"
        );
        assert!(
            !pane.is_file_editor_active(),
            "discarding leaves the editor for the read-only view"
        );
        assert_eq!(
            std::fs::read_to_string(workdir.join(&file_rel)).expect("file still on disk"),
            "fn main() {}\n",
            "and the file on disk was never written"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn file_editor_saves_stay_paused_until_rename_retargets_the_unsaved_buffer(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::filesystem::{DocumentIdentity, Filesystem, Operation, Request};
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let original = workdir.join("before.rs");
    let renamed = workdir.join("after.rs");
    std::fs::write(&original, "fn original() {}\n").unwrap();
    let repo_id = gitcomet_state::model::RepoId(995);
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                editor_state(repo_id, &workdir, Path::new("before.rs")),
                cx,
            );
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();
    let request = Request::new(Operation::Rename {
        source: original.clone(),
        name: "after.rs".into(),
    });
    cx.update(|_, app| {
        let pane = view.read(app).diff_renderer.clone();
        pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// retained edit\n", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                assert!(pane.file_editor_dirty());
                pane.filesystem_pause(request.id, cx);
                pane.save_file_editor_buffer(cx);
                assert!(pane.editor_documents.pending().is_empty());
            });
    });
    let result = Filesystem::default().execute(request, |_| {});
    assert!(result.succeeded(), "{:?}", result.items);
    assert!(!original.exists());
    assert_eq!(
        std::fs::read_to_string(&renamed).unwrap(),
        "fn original() {}\n"
    );
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.filesystem_finish(result.id, &result.changes, &result.moved_versions, cx);
                assert_eq!(
                    pane.file_editor_key(),
                    Some(DocumentIdentity(renamed.clone()))
                );
                assert!(pane.file_editor_dirty());
                assert_eq!(
                    pane.file_editor_input.read(cx).text(),
                    "// retained edit\nfn original() {}\n"
                );
            });
            push_test_state(
                this,
                editor_state(repo_id, &workdir, Path::new("after.rs")),
                cx,
            );
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| pane.save_file_editor_buffer(cx));
    });
    finish_editor_saves(&view, cx);
    assert!(
        !original.exists(),
        "saving after a rename must never recreate the old path"
    );
    assert_eq!(
        std::fs::read_to_string(&renamed).unwrap(),
        "// retained edit\nfn original() {}\n"
    );
}

#[gpui::test]
async fn saving_from_the_toolbar_exits_the_editor(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(976);
    let workdir = unique_workdir("file_editor_save_exits");
    let file_rel = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file_rel), "fn main() {}\n").expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.store.insert_repo_for_test(
                repo_id,
                Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
                    workdir.clone(),
                )),
            );
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// saved\n", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.save_file_editor_buffer_and_exit(window, cx);
        });
    });
    cx.run_until_parked();
    // The write runs on the filesystem worker; wait for its confirmation.
    finish_editor_saves(&view, cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert!(!pane.file_editor_is_dirty());
        assert!(
            !pane.is_file_editor_active(),
            "the toolbar Save action should close edit mode after dispatching the write"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

fn save_completion_after_navigation(cx: &mut gpui::TestAppContext, autosave: bool, paused: bool) {
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let repo_id = gitcomet_state::model::RepoId(1942);
    let first = PathBuf::from("first.txt");
    let second = PathBuf::from("second.txt");
    for relative in [&first, &second] {
        std::fs::write(workdir.join(relative), "original").unwrap();
    }
    let pane = cx.update(|_, app| view.read(app).diff_renderer.clone());
    let pauses = [
        gitcomet_core::filesystem::OperationId::allocate(),
        gitcomet_core::filesystem::OperationId::allocate(),
    ];
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, editor_state(repo_id, &workdir, &first), cx);
        })
    });
    cx.update(|_, app| pane.update(app, |pane, cx| pane.ensure_file_editor_loaded(cx)));
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.auto_save_file_edits = autosave;
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..input.text().len(), "first save", cx);
            });
        })
    });
    cx.run_until_parked();
    let key = cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.save_file_editor_buffer(cx);
            let key = pane.file_editor_key().clone().unwrap();
            assert!(
                pane.file_editor_stash().contains_key(&key),
                "in-flight writes retain recovery text"
            );
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..input.text().len(), "newer edits", cx);
            });
            key
        })
    });
    // Publish navigation without a save acknowledgment or replacing the
    // worker's snapshot. The navigation flush still sees the first write pending.
    cx.update(|_, app| {
        view.read(app).ui_model.clone().update(app, |model, cx| {
            let mut state = (*model.state).clone();
            state.repos[0].diff_state.diff_target =
                Some(gitcomet_core::domain::DiffTarget::working_tree(
                    second.clone(),
                    gitcomet_core::domain::DiffArea::Unstaged,
                ));
            model.set_state(Arc::new(state), cx);
        });
    });
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.ensure_file_editor_loaded(cx);
            assert_eq!(pane.editor_documents.pending().len(), 1);
            assert_eq!(
                pane.file_editor_key().as_ref().unwrap().0,
                workdir.join(&second)
            );
            assert_eq!(pane.file_editor_stash()[&key].text.as_ref(), "newer edits");
            if paused {
                for id in pauses {
                    pane.filesystem_pause(id, cx);
                }
            }
        })
    });
    finish_editor_saves(&view, cx);
    if paused {
        assert_eq!(
            std::fs::read_to_string(workdir.join(&first)).unwrap(),
            "first save"
        );
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                assert!(pane.file_editor_stash()[&key].is_dirty());
                pane.filesystem_finish(pauses[0], &[], &Default::default(), cx);
                assert!(pane.editor_documents.pending().is_empty());
                pane.filesystem_finish(pauses[1], &[], &Default::default(), cx);
                assert_eq!(pane.editor_documents.pending().len(), usize::from(autosave));
            });
        });
        finish_editor_saves(&view, cx);
    }
    if !autosave {
        assert_eq!(
            std::fs::read_to_string(workdir.join(&first)).unwrap(),
            "first save"
        );
        cx.update(|_, app| {
            let stashed = &pane.read(app).file_editor_stash()[&key];
            assert_eq!(stashed.text.as_ref(), "newer edits");
            assert!(stashed.is_dirty());
        });
        cx.update(|_, app| pane.update(app, |pane, cx| pane.save_all_file_edits(cx)));
        finish_editor_saves(&view, cx);
    }
    assert_eq!(
        std::fs::read_to_string(workdir.join(&first)).unwrap(),
        "newer edits"
    );
    assert_eq!(
        std::fs::read_to_string(workdir.join(&second)).unwrap(),
        "original"
    );
    cx.update(|_, app| {
        assert!(pane.read(app).file_editor_stash().is_empty());
        assert!(pane.read(app).unsaved_file_edit_labels().is_empty());
        assert!(pane.read(app).file_editor_error().is_none());
    });
}
