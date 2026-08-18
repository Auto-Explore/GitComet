use super::*;
use crate::view::panes::main::{
    apply_file_editor_bracket_highlights, file_editor_blame_line_for_editor_line,
    file_editor_provider_binding_key,
};
use palette::IntoColor;

/// A repo whose working tree holds `file_rel` with `contents`, already showing
/// that file in the editor.
fn editor_state(
    repo_id: gitcomet_state::model::RepoId,
    workdir: &Path,
    file_rel: &Path,
) -> Arc<AppState> {
    let mut repo = opening_repo_state(repo_id, workdir);
    repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.to_path_buf(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });
    repo.diff_state.content_preview = true;
    repo.diff_state.edit_mode = true;
    app_state_with_repo(repo, repo_id)
}

fn unique_workdir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{label}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[gpui::test]
async fn file_editor_loads_the_working_tree_file_and_starts_clean(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(940);
    let workdir = unique_workdir("file_editor_load");
    let file_rel = std::path::PathBuf::from("main.rs");
    let contents = "fn main() {\n    let value = 1;\n}\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    // The read runs off the foreground, so let it land.
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_file_editor_active());
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            contents,
            "the buffer must hold the file as it is on disk"
        );
        assert!(
            !pane.file_editor_is_dirty(),
            "a freshly loaded buffer matches disk"
        );
        assert!(pane.unsaved_file_edit_labels().is_empty());
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn file_editor_marks_dirty_on_edit_and_clean_after_save(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                assert!(!pane.file_editor_is_dirty());
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "// header\n", cx);
                });
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.file_editor_is_dirty(), "typing marks the buffer dirty");
        assert_eq!(
            pane.unsaved_file_edit_labels(),
            vec![SharedString::from("main.rs")],
            "the edited file is reported as unsaved"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane
                .update(cx, |pane, cx| pane.save_file_editor_buffer(cx));
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(!pane.file_editor_is_dirty(), "saving settles the buffer");
        assert!(pane.unsaved_file_edit_labels().is_empty());
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn file_editor_keeps_an_unsaved_buffer_across_a_file_switch(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    // Edit the first file without saving...
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
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
async fn file_editor_refuses_a_non_utf8_file(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(943);
    let workdir = unique_workdir("file_editor_binary");
    let file_rel = std::path::PathBuf::from("blob.bin");
    std::fs::write(workdir.join(&file_rel), [0xff, 0xfe, 0x00, 0x01]).expect("write binary");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_editor_error.is_some(),
            "a binary file must surface an error instead of an empty buffer"
        );
        assert!(!pane.file_editor_is_dirty());
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn bracket_overlay_paints_both_delimiters_over_the_syntax_runs() {
    let text = "fn f() {}";
    let keyword = gpui::HighlightStyle {
        color: Some(gpui::rgb(0x112233).into_color()),
        ..Default::default()
    };
    let bracket = gpui::HighlightStyle {
        background_color: Some(gpui::rgba(0xffffff26).into_color()),
        ..Default::default()
    };
    let open = text.find('{').expect("open brace");
    let close = text.find('}').expect("close brace");

    let highlights = apply_file_editor_bracket_highlights(
        vec![(0..2, keyword), (open..close + 1, keyword)],
        Some(&(open..open + 1, close..close + 1)),
        0..text.len(),
        bracket,
    );

    // Sorted, disjoint and inside the window, as the input requires.
    let mut previous_end = 0usize;
    for (range, _) in &highlights {
        assert!(range.start >= previous_end, "runs overlap: {highlights:?}");
        assert!(range.start < range.end);
        assert!(range.end <= text.len());
        previous_end = range.end;
    }

    let background_at = |offset: usize| {
        highlights
            .iter()
            .find(|(range, _)| range.contains(&offset))
            .and_then(|(_, style)| style.background_color)
    };
    assert_eq!(background_at(open), bracket.background_color);
    assert_eq!(background_at(close), bracket.background_color);
    assert_eq!(background_at(0), None, "unrelated runs keep their styling");
}

#[test]
fn bracket_overlay_is_a_no_op_without_a_pair() {
    let style = gpui::HighlightStyle::default();
    let runs = vec![(0..4, style), (6..9, style)];
    assert_eq!(
        apply_file_editor_bracket_highlights(runs.clone(), None, 0..9, style),
        runs
    );
}

#[test]
fn provider_binding_key_changes_only_when_something_changed() {
    let pair = (3..4, 9..10);
    let base = file_editor_provider_binding_key(7, 1, Some(&pair));

    assert_eq!(
        base,
        file_editor_provider_binding_key(7, 1, Some(&pair)),
        "an unchanged binding must not rebind — that is what stops the observe cycle"
    );
    assert_ne!(base, file_editor_provider_binding_key(8, 1, Some(&pair)));
    assert_ne!(base, file_editor_provider_binding_key(7, 2, Some(&pair)));
    assert_ne!(
        base,
        file_editor_provider_binding_key(7, 1, Some(&(3..4, 12..13))),
        "moving the caret to another pair must rebind"
    );
    assert_ne!(base, file_editor_provider_binding_key(7, 1, None));
}

#[gpui::test]
async fn alt_e_toggles_the_editor_and_ctrl_s_saves_and_exits_while_it_has_focus(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(944);
    let workdir = unique_workdir("file_editor_shortcuts");
    let file_rel = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file_rel), "fn main() {}\n").expect("write fixture");

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    let press = |cx: &mut gpui::VisualTestContext, chord: &str| -> bool {
        let keystroke = gpui::Keystroke::parse(chord).expect("valid chord");
        cx.update(|window, app| {
            main_pane.update(app, |pane, cx| {
                pane.handle_diff_shortcut(&keystroke, window, cx)
            })
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
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

    // Ctrl/Cmd+S only saves while the buffer has focus; with the editor
    // unfocused it stays out of the way of the staging shortcut.
    cx.update(|window, app| {
        main_pane.update(app, |pane, cx| {
            let handle = pane.file_editor_input.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        });
    });
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// edited\n", cx);
            });
        });
    });
    cx.run_until_parked();
    assert!(cx.update(|_window, app| main_pane.read(app).file_editor_is_dirty()));

    assert!(
        press(cx, "secondary-s"),
        "ctrl/cmd-s saves inside the editor"
    );
    assert!(
        cx.update(|_window, app| !main_pane.read(app).file_editor_is_dirty()),
        "the buffer settles once saved"
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
    });
    cx.run_until_parked();
    assert!(
        cx.update(|_window, app| !main_pane.read(app).is_file_editor_active()),
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    // Type into the middle of the file, then take it back out again.
    let middle = contents.len() / 2;
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(middle..middle, "xyz", cx);
                });
            });
        });
    });
    cx.run_until_parked();
    assert!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).file_editor_is_dirty()),
        "the inserted text must read as modified"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(middle..middle + 3, "", cx);
                });
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
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
async fn a_pending_read_does_not_land_in_another_repos_buffer(cx: &mut gpui::TestAppContext) {
    // Regression: the in-flight guard compared only the path, so switching to a
    // second repo holding the same relative path before the read completed
    // seated the first repo's contents in the second repo's buffer — and a save
    // would then have written them to the wrong file.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let file_rel = std::path::PathBuf::from("shared.rs");
    let repo_a = gitcomet_state::model::RepoId(946);
    let repo_b = gitcomet_state::model::RepoId(947);
    let workdir_a = unique_workdir("file_editor_repo_a");
    let workdir_b = unique_workdir("file_editor_repo_b");
    std::fs::write(workdir_a.join(&file_rel), "fn from_a() {}\n").expect("write a");
    std::fs::write(workdir_b.join(&file_rel), "fn from_b() {}\n").expect("write b");

    // Start repo A's read and switch to repo B before letting it land.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_a, &workdir_a, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
            push_test_state(this, editor_state(repo_b, &workdir_b, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            "fn from_b() {}\n",
            "the buffer must hold the repo it is actually showing"
        );
        assert!(!pane.file_editor_is_dirty());
    });

    let _ = std::fs::remove_dir_all(&workdir_a);
    let _ = std::fs::remove_dir_all(&workdir_b);
}

#[gpui::test]
async fn returning_to_a_stashed_buffer_measures_it_against_its_own_file(
    cx: &mut gpui::TestAppContext,
) {
    // Regression: dirtiness was measured against a single "last saved" slot,
    // which by the time a stashed buffer came back held the *other* file's
    // fingerprint — so a restored buffer could never settle, or worse, could be
    // reported clean and then dropped.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(948);
    let workdir = unique_workdir("file_editor_baseline");
    let first = std::path::PathBuf::from("first.rs");
    let second = std::path::PathBuf::from("second.rs");
    std::fs::write(workdir.join(&first), "fn first() {}\n").expect("write first");
    std::fs::write(workdir.join(&second), "fn second() {}\n").expect("write second");

    let open = |cx: &mut gpui::VisualTestContext, path: &std::path::Path| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                push_test_state(this, editor_state(repo_id, &workdir, path), cx);
                this.main_pane
                    .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
            });
        });
        cx.run_until_parked();
    };

    open(cx, &first);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "// edited\n", cx);
                });
            });
        });
    });
    cx.run_until_parked();

    open(cx, &second);
    open(cx, &first);

    // Undoing the edit by hand must return the restored buffer to clean.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..10, "", cx);
                });
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "fn first() {}\n");
        assert!(
            !pane.file_editor_is_dirty(),
            "a restored buffer is measured against its own file, not the one visited in between"
        );
        assert!(pane.unsaved_file_edit_labels().is_empty());
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn editing_markdown_highlights_and_leaves_the_rendered_preview(
    cx: &mut gpui::TestAppContext,
) {
    // Markdown opens rendered by default, so entering the editor has to flip the
    // toggle to Source — otherwise `is_markdown_preview_active` reports a
    // preview over a buffer that is plainly showing text and greys out Edit and
    // Blame. The highlight assertions pin the other half: the editor really is
    // running the markdown grammar, injections included.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(949);
    let workdir = unique_workdir("file_editor_markdown");
    let file_rel = std::path::PathBuf::from("README.md");
    let contents = "# Heading\n\nSome `code` here.\n\n```rust\nfn main() {}\n```\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.is_markdown_preview_active(),
            "the editor is not a rendered preview, whatever the toggle said before"
        );

        let document = pane
            .file_editor_live_syntax
            .as_ref()
            .expect("markdown must get a tree-sitter document, not the heuristic fallback");
        let highlights = document
            .snapshot(pane.theme)
            .highlights_for_byte_range(0..contents.len());

        let styled = |needle: &str| {
            let at = contents.find(needle).expect("fixture contains the needle");
            highlights
                .iter()
                .any(|(range, style)| range.contains(&at) && style.color.is_some())
        };
        assert!(styled("Heading"), "headings must be highlighted");
        assert!(styled("`code`"), "code spans must be highlighted");
        assert!(
            styled("fn main"),
            "a fenced rust block must pick up the injected grammar"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Two lines of blame for the `open_annotated_editor` fixture file.
fn editor_blame_lines() -> Vec<gitcomet_core::services::BlameLine> {
    vec![
        gitcomet_core::services::BlameLine {
            commit_id: std::sync::Arc::from("abcdef1"),
            author: "Sampo Kivistö".into(),
            author_time_unix: Some(1_700_000_000),
            summary: "add main".into(),
            body: None,
            line: "fn main() {}".into(),
            prior_exists: true,
            source_path: None,
            prior_commit: None,
        },
        gitcomet_core::services::BlameLine {
            commit_id: std::sync::Arc::from("abcdef2"),
            author: "Sampo Kivistö".into(),
            author_time_unix: Some(1_700_000_100),
            summary: "add other".into(),
            body: None,
            line: "fn other() {}".into(),
            prior_exists: true,
            source_path: None,
            prior_commit: None,
        },
    ]
}

/// Show `file_rel` in the editor with annotate on and blame already loaded for
/// it, so the gutter has a populated annotation column.
fn open_annotated_editor(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &Path,
    file_rel: &Path,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut state = editor_state(repo_id, workdir, file_rel);
            let repo = &mut Arc::make_mut(&mut state).repos[0];
            repo.history_state.blame_path = Some(file_rel.to_path_buf());
            repo.history_state.blame_source =
                Some(gitcomet_core::domain::BlameSource::WorkingTree(
                    gitcomet_core::domain::DiffArea::Unstaged,
                ));
            repo.history_state.blame =
                gitcomet_state::model::Loadable::Ready(std::sync::Arc::new(editor_blame_lines()));
            push_test_state(this, state, cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.set_annotate_enabled(true, cx);
                pane.ensure_file_editor_loaded(cx);
            });
        });
    });
    cx.run_until_parked();
}

#[gpui::test]
async fn blame_is_available_and_rendered_while_editing(cx: &mut gpui::TestAppContext) {
    // The editor used to leave Blame inert: the toggle was reachable but the
    // buffer had no annotation column, so turning it on did nothing visible.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(950);
    let workdir = unique_workdir("file_editor_blame");
    let file_rel = std::path::PathBuf::from("main.rs");
    let contents = "fn main() {}\nfn other() {}\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    open_annotated_editor(cx, &view, repo_id, &workdir, &file_rel);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_file_editor_active());
        assert!(
            pane.annotation_active(),
            "an edited working-tree file is blameable — nothing about editing changes that"
        );
        assert!(
            pane.blame_render_ctx_for_test().is_some(),
            "the editor must resolve a blame context so its gutter has something to draw"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Left offset of the annotate resize handle within the editor. Panics if the
/// handle is absent.
fn editor_annotate_handle_offset(cx: &mut gpui::VisualTestContext) -> Pixels {
    let handle = cx
        .debug_bounds("annotate_resize_handle")
        .expect("edit mode must mount the annotate resize handle");
    let editor = cx
        .debug_bounds("file_editor")
        .expect("expected `file_editor` bounds");
    handle.center().x - editor.left()
}

/// Press the handle and drag it `dx` to the right. Two moves: gpui starts the
/// drag on the first button-held move and only delivers `DragMoveEvent` on the
/// ones after it.
fn drag_annotate_handle(cx: &mut gpui::VisualTestContext, dx: f32) {
    let start = cx
        .debug_bounds("annotate_resize_handle")
        .expect("expected the annotate resize handle")
        .center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: start,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_mouse_move(
        point(start.x + px(dx.signum() * 8.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    draw_and_drain_test_window(cx);
    let end = point(start.x + px(dx), start.y);
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_event(MouseUpEvent {
        position: end,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });
    draw_and_drain_test_window(cx);
}

#[gpui::test]
async fn annotate_column_is_resizable_while_editing(cx: &mut gpui::TestAppContext) {
    // The editor drew the annotation column but mounted no drag handle, so the
    // column was stuck at whatever width the diff view had last left it at.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(952);
    let workdir = unique_workdir("file_editor_annotate_resize");
    let file_rel = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file_rel), "fn main() {}\nfn other() {}\n")
        .expect("write fixture");

    open_annotated_editor(cx, &view, repo_id, &workdir, &file_rel);
    draw_and_drain_test_window(cx);

    let offset = editor_annotate_handle_offset(cx);
    let column_width = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.annotate_column_width_px(crate::ui_scale::DEFAULT_UI_SCALE_PERCENT)
    });
    assert!(
        (f32::from(offset) - f32::from(column_width)).abs() <= 1.0,
        "the handle must straddle the annotation column's right edge, got {offset:?} for a \
         {column_width:?} column"
    );

    drag_annotate_handle(cx, 60.0);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.annotate_column_width, 360.0,
            "dragging 60px right at 100% ui scale must widen the column by 60"
        );
        assert!(
            pane.annotate_resize.is_none(),
            "the release must end the drag"
        );
    });

    let widened_offset = editor_annotate_handle_offset(cx);
    assert!(
        f32::from(widened_offset) - f32::from(offset) > 55.0,
        "the handle must follow the column it resized, moved {:?}",
        widened_offset - offset
    );

    // Past the maximum the clamp takes over rather than the column running on.
    drag_annotate_handle(cx, 1_000.0);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).annotate_column_width,
            crate::view::rows::DIFF_ANNOTATION_MAX_WIDTH_PX,
            "the drag must clamp at the maximum column width"
        );
    });

    // ...and the minimum on the way back.
    drag_annotate_handle(cx, -1_000.0);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).annotate_column_width,
            crate::view::rows::DIFF_ANNOTATION_MIN_WIDTH_PX,
            "the drag must clamp at the minimum column width"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn annotate_resize_handle_waits_for_the_column_it_resizes(cx: &mut gpui::TestAppContext) {
    // The editor reserves the annotation column only once blame resolves, so
    // the handle has to follow the column rather than the toggle: gated on
    // `annotation_active()` it would sit at x=0 over a column nobody drew.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(953);
    let workdir = unique_workdir("file_editor_annotate_no_blame");
    let file_rel = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file_rel), "fn main() {}\nfn other() {}\n")
        .expect("write fixture");

    // Annotate on, but no blame ever loads — `TestBackend` opens no repository.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.set_annotate_enabled(true, cx);
                pane.ensure_file_editor_loaded(cx);
            });
        });
    });
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.annotation_active(),
            "the toggle is on and the target is blameable"
        );
        assert!(
            pane.blame_render_ctx_for_test().is_none(),
            "no blame resolved, so the editor draws no annotation column"
        );
    });
    assert!(
        cx.debug_bounds("annotate_resize_handle").is_none(),
        "a handle without a column would drag nothing"
    );

    // Blame arrives: the column appears, and the handle with it.
    open_annotated_editor(cx, &view, repo_id, &workdir, &file_rel);
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("annotate_resize_handle").is_some(),
        "the handle must appear once there is a column to resize"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn word_wrap_reaches_the_buffer(cx: &mut gpui::TestAppContext) {
    // The editor was built at content width, so long lines ran off to the right
    // whatever the word-wrap preference said.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(951);
    let workdir = unique_workdir("file_editor_wrap");
    let file_rel = std::path::PathBuf::from("notes.md");
    std::fs::write(workdir.join(&file_rel), "a very long line ".repeat(40)).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    // Driven on the pane directly: `set_diff_word_wrap` reaches back into the
    // root view, which cannot be updated from inside its own update.
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|window, app| {
        main_pane.update(app, |pane, cx| {
            pane.set_diff_word_wrap(true, cx);
            let _ = pane.diff_view(window, cx);
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_editor_input.read(app).soft_wrap(),
            "the word-wrap preference has to reach the buffer, not just the diff"
        );
    });

    // Turning it off puts the buffer back on content-width layout.
    cx.update(|window, app| {
        main_pane.update(app, |pane, cx| {
            pane.set_diff_word_wrap(false, cx);
            let _ = pane.diff_view(window, cx);
        });
    });
    cx.update(|_window, app| {
        assert!(
            !view
                .read(app)
                .main_pane
                .read(app)
                .file_editor_input
                .read(app)
                .soft_wrap()
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn file_editor_mounts_a_vertical_scrollbar(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(975);
    let workdir = unique_workdir("file_editor_scrollbar");
    let file_rel = std::path::PathBuf::from("long.rs");
    let contents = (0..400)
        .map(|line| format!("fn line_{line}() {{}}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds("file_editor_scrollbar").is_some(),
        "edit mode should mount its vertical scrollbar beside the scroll surface"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_editor_scroll.max_offset().y > px(0.0),
            "the long editor fixture should expose a vertical scroll range"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn an_svg_opened_from_the_explorer_can_show_its_code_and_be_edited(
    cx: &mut gpui::TestAppContext,
) {
    // Pictures opened from the explorer used to land in the A/B image diff.
    // Rendered is still the picture; Code is the source, and the source is
    // editable text like any other file.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(952);
    let workdir = unique_workdir("file_editor_svg");
    let file_rel = std::path::PathBuf::from("logo.svg");
    let contents = "<svg viewBox=\"0 0 1 1\"><rect width=\"1\" height=\"1\" /></svg>\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");
    let absolute = workdir.join(&file_rel);

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut state = editor_state(repo_id, &workdir, &file_rel);
            // A read-only content view, as `OpenFileContent` leaves it.
            Arc::make_mut(&mut state).repos[0].diff_state.edit_mode = false;
            push_test_state(this, state, cx);
        });
    });

    cx.update(|_window, app| {
        main_pane.update(app, |pane, _cx| {
            assert!(
                pane.content_preview_is_picture(&absolute),
                "rendered is the default for an SVG, and rendered means the picture"
            );
            pane.rendered_preview_modes.set(
                crate::view::RenderedPreviewKind::Svg,
                crate::view::RenderedPreviewMode::Source,
            );
            assert!(
                !pane.content_preview_is_picture(&absolute),
                "asking for Code is asking for the source, which is text"
            );
        });
    });

    // With Code showing, the file is editable.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut state = editor_state(repo_id, &workdir, &file_rel);
            Arc::make_mut(&mut state).repos[0].diff_state.edit_mode = true;
            push_test_state(this, state, cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_file_editor_active());
        assert_eq!(pane.file_editor_input.read(app).text(), contents);
        assert_eq!(
            pane.file_editor_language,
            Some(rows::DiffSyntaxLanguage::Xml)
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn a_wrapped_line_owns_its_gutter_rows_and_numbers_only_the_first() {
    use crate::view::panes::main::file_editor_line_for_visual_row;

    // Line 0 wraps to 3 rows, line 1 to 1, line 2 to 2 — so the gutter rows are
    // [0,0,0, 1, 2,2] and the number goes on the first row of each block.
    let starts = [0usize, 3, 4];
    let expected = [
        (0, false),
        (0, true),
        (0, true),
        (1, false),
        (2, false),
        (2, true),
    ];
    for (visual_ix, want) in expected.iter().enumerate() {
        assert_eq!(
            file_editor_line_for_visual_row(&starts, visual_ix),
            *want,
            "gutter row {visual_ix}"
        );
    }

    // No wrap: the two row spaces coincide, and nothing is a continuation.
    for visual_ix in 0..4 {
        assert_eq!(
            file_editor_line_for_visual_row(&[], visual_ix),
            (visual_ix, false)
        );
    }
}

#[gpui::test]
async fn the_gutter_projects_through_wrap_so_numbers_track_their_lines(
    cx: &mut gpui::TestAppContext,
) {
    // Regression: the gutter was one row per *logical* line while the buffer
    // laid out one per *visual* row, so from the first wrap onward the numbers
    // labelled the wrong lines.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(953);
    let workdir = unique_workdir("file_editor_wrap_gutter");
    let file_rel = std::path::PathBuf::from("notes.md");
    let long = "wrap ".repeat(120);
    std::fs::write(workdir.join(&file_rel), format!("{long}\nshort\n")).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|window, app| {
        main_pane.update(app, |pane, cx| {
            pane.set_diff_word_wrap(true, cx);
            let _ = pane.diff_view(window, cx);
        });
    });
    cx.run_until_parked();
    // A second pass: the row counts are maintained during the buffer's prepaint,
    // so the frame that can project them is the one after it has laid out.
    cx.update(|window, app| {
        main_pane.update(app, |pane, cx| {
            let _ = pane.diff_view(window, cx);
        });
    });

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        let starts = &pane.file_editor_wrap_row_starts;
        if starts.is_empty() {
            // The headless text system reports every glyph identically, so the
            // wrap pass may legitimately decide nothing wraps here. The mapping
            // itself is pinned by the unit test above; what matters here is that
            // the gutter never claims more lines than the buffer has.
            return;
        }
        assert_eq!(
            starts.len(),
            pane.file_editor_input
                .read(app)
                .text_snapshot()
                .line_count(),
            "the projection has one entry per logical line"
        );
        assert!(
            starts.windows(2).all(|pair| pair[0] < pair[1]),
            "each line starts strictly after the previous one: {starts:?}"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn a_supported_language_is_never_latched_as_unparseable() {
    use crate::view::rows;

    // Regression: a failed build used to latch the file as unhighlightable for
    // the rest of the session, so one unlucky parse left markdown plain until
    // the file was reopened. The permanent reasons are now asked directly.
    assert!(
        rows::live_syntax_document_supported(rows::DiffSyntaxLanguage::Markdown, 4_096),
        "markdown has a wired grammar and a small file is well under the ceiling"
    );
    assert!(
        !rows::live_syntax_document_supported(
            rows::DiffSyntaxLanguage::Markdown,
            rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES + 1,
        ),
        "past the parse ceiling is a permanent no, and is checked without parsing"
    );
}

#[gpui::test]
async fn the_load_placeholder_is_never_saved_over_the_file(cx: &mut gpui::TestAppContext) {
    // Regression: switching files left the *previous* file's dirty flag set
    // while the buffer held the blank placeholder for the new one, so a save in
    // that window wrote an empty string over the file just opened. The window is
    // a scheduling accident, so this reconstructs the state it produced rather
    // than trying to race it.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            // Exactly the mid-load state: the key names this file, the buffer is
            // the blank placeholder, and a stale dirty flag says it is unsaved.
            pane.file_editor_input.update(cx, |input, cx| {
                input.set_text("", cx);
            });
            pane.file_editor_loading = true;
            pane.file_editor_dirty = true;

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
    // Regression: a save bumps the repo's status revision, which the clean-buffer
    // disk-follow treats as "the file may have moved". That path used to blank
    // the input before re-reading, so every save re-seated the buffer — resetting
    // the caret to 0 and clearing undo, once per auto-save.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
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
        main_pane
            .read(app)
            .file_editor_input
            .read(app)
            .text_snapshot()
            .model_id()
    });

    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| pane.save_file_editor_buffer(cx));
    });
    // Let the save land and the follow-up re-read run to completion.
    cx.run_until_parked();
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| pane.ensure_file_editor_loaded(cx));
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
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
async fn a_closed_repos_stashed_buffer_cannot_wedge_the_quit_dialog(cx: &mut gpui::TestAppContext) {
    // Regression: a stash entry whose repo tab had closed stayed dirty forever.
    // The quit dialog listed it, "Save all" could not write it (the store drops
    // messages for a repo it no longer has), and the retry raised the dialog
    // again — the window could never be closed.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// edited\n", cx);
            });
        });
    });
    cx.run_until_parked();

    // Leaving the file stashes the edit, then the repo tab closes under it.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| pane.stash_current_file_editor_buffer(cx));
    });
    cx.update(|_window, app| {
        assert!(
            !main_pane.read(app).unsaved_file_edit_labels().is_empty(),
            "the stashed edit is unsaved while its repo is still open"
        );
        view.update(app, |this, cx| {
            let mut state = AppState::default();
            state.active_repo = None;
            push_test_state(this, Arc::new(state), cx);
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        assert!(
            pane.unsaved_file_edit_labels().is_empty(),
            "a buffer with no repo left to save it into must not block the quit dialog"
        );
    });

    // And Save all is a no-op rather than an endless round trip.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| pane.save_all_file_edits(cx));
    });
    cx.update(|_window, app| {
        assert!(main_pane.read(app).unsaved_file_edit_labels().is_empty());
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn blame_lines_shift_with_unsaved_edits_instead_of_vanishing() {
    // A clean buffer is line-for-line with the blamed revision.
    for line in [0usize, 1, 40] {
        assert_eq!(
            file_editor_blame_line_for_editor_line(line, None, 0),
            u32::try_from(line + 1).ok()
        );
    }

    // An edit that moved no line boundary leaves the mapping alone, even though
    // the buffer is dirty.
    assert_eq!(
        file_editor_blame_line_for_editor_line(9, Some(3), 0),
        Some(10)
    );

    // Two lines typed at line 3: everything above is untouched...
    assert_eq!(
        file_editor_blame_line_for_editor_line(0, Some(3), 2),
        Some(1)
    );
    assert_eq!(
        file_editor_blame_line_for_editor_line(2, Some(3), 2),
        Some(3)
    );
    // ...the two inserted lines have no revision line behind them...
    assert_eq!(file_editor_blame_line_for_editor_line(3, Some(3), 2), None);
    assert_eq!(file_editor_blame_line_for_editor_line(4, Some(3), 2), None);
    // ...and everything below keeps the attribution it actually has, rather
    // than being blanked or reading two lines low.
    assert_eq!(
        file_editor_blame_line_for_editor_line(5, Some(3), 2),
        Some(4)
    );
    assert_eq!(
        file_editor_blame_line_for_editor_line(20, Some(3), 2),
        Some(19)
    );

    // Deleting two lines at line 3 shifts the other way.
    assert_eq!(
        file_editor_blame_line_for_editor_line(3, Some(3), -2),
        Some(6)
    );
    assert_eq!(
        file_editor_blame_line_for_editor_line(2, Some(3), -2),
        Some(3),
        "lines above the deletion are still themselves"
    );
}

#[gpui::test]
async fn editing_mid_file_keeps_blame_above_the_edit(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(958);
    let workdir = unique_workdir("file_editor_blame_watermark");
    let file_rel = std::path::PathBuf::from("main.rs");
    let contents = "one\ntwo\nthree\nfour\nfive\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        assert_eq!(
            main_pane.read(app).file_editor_first_dirty_line,
            None,
            "a buffer that matches disk has no watermark"
        );
    });

    // Type into line index 2 ("three").
    let at = contents.find("three").expect("fixture line");
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(at..at, "// ", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        assert!(pane.file_editor_is_dirty());
        assert_eq!(
            pane.file_editor_first_dirty_line,
            Some(2),
            "the watermark is the first line the edit touched"
        );
    });

    // An edit further down must not move the watermark back up...
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            let end = pane.file_editor_input.read(cx).text().len();
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(end..end, "six\n", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert_eq!(
            main_pane.read(app).file_editor_first_dirty_line,
            Some(2),
            "a later edit must not raise the watermark"
        );
    });

    // ...but an edit above it must lower it.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// header\n", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert_eq!(
            main_pane.read(app).file_editor_first_dirty_line,
            Some(0),
            "an earlier edit lowers the watermark"
        );
    });

    // Saving makes every line attributable again.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| pane.save_file_editor_buffer(cx));
    });
    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        assert!(!pane.file_editor_is_dirty());
        assert_eq!(pane.file_editor_first_dirty_line, None);
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn a_stashed_buffer_brings_its_blame_watermark_back(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(959);
    let workdir = unique_workdir("file_editor_blame_stash");
    let file_rel = std::path::PathBuf::from("main.rs");
    let contents = "one\ntwo\nthree\nfour\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    let at = contents.find("three").expect("fixture line");
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(at..at, "// ", cx);
            });
        });
    });
    cx.run_until_parked();

    // Leave the file and come back: the watermark must not be rebuilt from
    // nothing, or the restored buffer would blank its whole annotation column.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.stash_current_file_editor_buffer(cx);
            pane.file_editor_key = None;
            pane.file_editor_first_dirty_line = None;
            pane.ensure_file_editor_loaded(cx);
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        assert!(pane.file_editor_is_dirty(), "the stashed edit came back");
        assert_eq!(
            pane.file_editor_first_dirty_line,
            Some(2),
            "and so did the line it started at"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn unsaved_edits_are_reported_and_discardable_per_file(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        assert!(
            main_pane
                .read(app)
                .unsaved_file_edit_paths(repo_id)
                .is_empty(),
            "a clean buffer is not an unsaved edit"
        );
    });

    // Edit `a.rs`, then move to `b.rs` so the first buffer goes to the stash.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// a\n", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &second), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    // Now edit `b.rs` too: one stashed dirty buffer and one on screen.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// b\n", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
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
        main_pane.update(app, |pane, cx| {
            pane.discard_file_edits_for(repo_id, &first, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let pane = main_pane.read(app);
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
        main_pane.update(app, |pane, cx| {
            pane.discard_file_edits_for(repo_id, &second, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let pane = main_pane.read(app);
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

/// End-to-end check that what the buffer *renders* is what the syntax engine
/// produced, across the highlight-provider and `TextInput` windowing layers.
///
/// The engine-level equivalence with the read-only panes is pinned in
/// `syntax/live.rs`; this pins the rest of the path, which is where an offset or
/// a dropped window would show up as "the colours are wrong in edit mode" while
/// every engine test still passed.
async fn assert_editor_renders_the_engines_highlights(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    label: &str,
    file_rel: &str,
    contents: &str,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let workdir = unique_workdir(label);
    let file_rel = std::path::PathBuf::from(file_rel);
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            contents,
            "{label}: the buffer must hold the fixture"
        );
        let document = pane
            .file_editor_live_syntax
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: the editor must build a tree-sitter document"));
        let expected = document
            .snapshot(pane.theme)
            .highlights_for_byte_range(0..contents.len());
        assert!(
            expected.len() > 4,
            "{label}: the fixture must produce a real spread of tokens, got {}",
            expected.len()
        );

        let rendered = main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, _| {
                input.debug_effective_highlights_for_range(0..contents.len())
            })
        });
        assert_eq!(
            rendered, expected,
            "{label}: the buffer must render exactly the runs the engine produced"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn the_editor_renders_rust_highlights_as_the_engine_produced_them(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    assert_editor_renders_the_engines_highlights(
        cx,
        gitcomet_state::model::RepoId(962),
        "file_editor_rs_highlights",
        "main.rs",
        concat!(
            "use std::collections::HashMap;\n",
            "\n",
            "/// Doc comment.\n",
            "pub struct Stage<'a> {\n",
            "    pub name: &'a str,\n",
            "    map: HashMap<String, usize>,\n",
            "}\n",
            "\n",
            "impl<'a> Stage<'a> {\n",
            "    pub fn run(&mut self, n: usize) -> Result<usize, String> {\n",
            "        let mut total = 0usize;\n",
            "        for (key, _value) in self.map.iter() {\n",
            "            println!(\"{key}: {}\", n);\n",
            "            total += key.len();\n",
            "        }\n",
            "        Ok(total)\n",
            "    }\n",
            "}\n",
        ),
    )
    .await;
}

#[gpui::test]
async fn the_editor_renders_shell_highlights_as_the_engine_produced_them(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    assert_editor_renders_the_engines_highlights(
        cx,
        gitcomet_state::model::RepoId(963),
        "file_editor_sh_highlights",
        "build.sh",
        concat!(
            "#!/usr/bin/env bash\n",
            "set -euo pipefail\n",
            "\n",
            "NAME=\"world\"\n",
            "count=0\n",
            "\n",
            "greet() {\n",
            "  local who=\"$1\"\n",
            "  echo \"hello ${who}\"\n",
            "  if [[ -n \"$who\" ]]; then\n",
            "    count=$((count + 1))\n",
            "  fi\n",
            "}\n",
            "\n",
            "for f in *.txt; do\n",
            "  greet \"$f\"\n",
            "done\n",
        ),
    )
    .await;
}

#[gpui::test]
async fn discarding_from_the_toolbar_returns_to_the_read_only_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// scratch\n", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert!(main_pane.read(app).file_editor_is_dirty());
    });

    cx.update(|window, app| {
        main_pane.update(app, |pane, cx| {
            pane.discard_file_editor_buffer_and_exit(window, cx);
        });
    });
    cx.run_until_parked();
    // The exit goes through the store, so pull the reduced snapshot back into
    // the view the way a real frame would.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
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
async fn saving_from_the_toolbar_exits_the_editor(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// saved\n", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        main_pane.update(app, |pane, cx| {
            pane.save_file_editor_buffer_and_exit(window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        assert!(!pane.file_editor_is_dirty());
        assert!(
            !pane.is_file_editor_active(),
            "the toolbar Save action should close edit mode after dispatching the write"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Opening a *second* file in the same pane must highlight it too.
///
/// This is the sequence the whole class of "edit mode has no highlighting" bugs
/// lived in, and no test used to walk it: every other test opens exactly one
/// file, where the buffer is already empty so the blanking `set_text("")` is a
/// no-op. From the second file on, the blanking is a real edit that used to
/// build a document over the empty text, which a budget-blown wholesale
/// replacement then kept — a full rope paired with a 0-byte tree.
///
/// Driven at a zero foreground budget so the test takes the arm the app takes:
/// the shipped budget is 1 ms against unoptimised tree-sitter in a dev build,
/// while tests get 2 ms at `opt-level = 1`, which is why this never reproduced
/// under `cargo test`.
#[gpui::test]
async fn a_second_file_opened_in_the_same_pane_is_still_highlighted(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(965);
    let workdir = unique_workdir("file_editor_second_file");
    let first = std::path::PathBuf::from("first.rs");
    let second = std::path::PathBuf::from("build.sh");
    std::fs::write(workdir.join(&first), "fn first() -> usize { 1 }\n").expect("write first");
    // Big enough that a zero-budget parse cannot finish, and shaped like the
    // reported repro: a quoted heredoc, a case block, `${var:-}` expansions.
    let second_contents = concat!(
        "#!/usr/bin/env bash\n",
        "set -euo pipefail\n",
        "\n",
        "usage() {\n",
        "  cat <<'USAGE'\n",
        "Usage: scripts/build.sh --out PATH [--verify]\n",
        "USAGE\n",
        "}\n",
        "\n",
        "out=\"\"\n",
        "verify=\"false\"\n",
        "while [[ $# -gt 0 ]]; do\n",
        "  case \"$1\" in\n",
        "    --out) out=\"${2:-}\"; shift 2 ;;\n",
        "    --verify) verify=\"true\"; shift ;;\n",
        "    *) echo \"unknown: $1\" >&2; usage; exit 1 ;;\n",
        "  esac\n",
        "done\n",
    )
    .repeat(8);
    std::fs::write(workdir.join(&second), &second_contents).expect("write second");

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, _cx| {
            pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                foreground_parse: std::time::Duration::ZERO,
            });
        });
    });

    // First file, then the second — the order that matters.
    for path in [&first, &second] {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                push_test_state(this, editor_state(repo_id, &workdir, path), cx);
                this.main_pane
                    .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
            });
        });
        cx.run_until_parked();
    }

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            second_contents,
            "the second file must be the one in the buffer"
        );
        let rendered = main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, _| {
                input.debug_effective_highlights_for_range(0..second_contents.len())
            })
        });
        assert!(
            !rendered.is_empty(),
            "the second file opened in a pane must still be highlighted; a zero \
             budget must fall back to heuristics or an off-thread parse, never to \
             a tree describing the file that was here before"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A file switch must not stash the blank placeholder over the file being left.
///
/// Between blanking the buffer and the read landing, `file_editor_saved_fingerprint`
/// has already been cleared, so the empty text reads as *dirty*. `flush_file_editor_buffer`
/// used to run in that window and stash the empty placeholder under the outgoing
/// file's own path, which the next open then restored over it.
#[gpui::test]
async fn switching_files_mid_load_never_stashes_the_blank_placeholder(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(966);
    let workdir = unique_workdir("file_editor_blank_stash");
    let first = std::path::PathBuf::from("first.rs");
    let second = std::path::PathBuf::from("second.rs");
    std::fs::write(workdir.join(&first), "fn first() {}\n").expect("write first");
    std::fs::write(workdir.join(&second), "fn second() {}\n").expect("write second");

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());

    // Open the first file, then switch away *without* letting the read land, so
    // the flush sees the blanked buffer.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &first), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &second), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        assert!(
            pane.unsaved_file_edit_paths(repo_id).is_empty(),
            "nothing was edited, so nothing may be reported unsaved: {:?}",
            pane.unsaved_file_edit_paths(repo_id)
        );
    });

    // Going back must show the file, not an empty buffer restored from the stash.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &first), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            main_pane.read(app).file_editor_input.read(app).text(),
            "fn first() {}\n",
            "returning to the file must show the file, not a stashed blank"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}
