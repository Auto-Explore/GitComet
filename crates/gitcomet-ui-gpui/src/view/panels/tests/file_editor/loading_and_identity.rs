use super::*;

#[gpui::test]
fn navigation_during_filesystem_pauses_hides_the_outgoing_buffer(cx: &mut gpui::TestAppContext) {
    navigate_during_filesystem_pause(cx, "original B");
}

#[gpui::test]
fn an_empty_file_loaded_after_a_filesystem_pause_becomes_editable(cx: &mut gpui::TestAppContext) {
    navigate_during_filesystem_pause(cx, "");
}

#[gpui::test]
fn renaming_the_active_editor_refreshes_its_language_without_replacing_edits(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::filesystem::{DocumentIdentity, Filesystem, Operation, Request};
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let repo_id = gitcomet_state::model::RepoId(996);
    let mut relative = PathBuf::from("before.txt");
    std::fs::write(
        workdir.join(&relative),
        "fn original() { let value = 42; }\n",
    )
    .unwrap();
    let pane = cx.update(|_, app| view.read(app).diff_renderer.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, editor_state(repo_id, &workdir, &relative), cx);
        });
    });
    cx.update(|_, app| pane.update(app, |pane, cx| pane.ensure_file_editor_loaded(cx)));
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.auto_save_file_edits = false;
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// retained edit\n", cx);
                input.set_cursor_offset(input.text().find('{').unwrap(), cx);
            });
        });
    });
    cx.run_until_parked();
    let (snapshot, cursor, saved) = cx.update(|_, app| {
        let pane = pane.read(app);
        let input = pane.file_editor_input.read(app);
        (
            input.text_snapshot(),
            input.cursor_offset(),
            pane.file_editor_saved_fingerprint(),
        )
    });
    // Exercise plain text, two grammars, and the heuristic provider. The last
    // two transitions have the same text revision and no live syntax document.
    for name in [
        "file.rs",
        "renamed.rs",
        "file.py",
        "file.txt",
        "file.conf",
        "after.txt",
    ] {
        let language = rows::diff_syntax_language_for_path(Path::new(name));
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                let old_language = pane.file_editor_language;
                let old_version = pane
                    .file_editor_live_syntax
                    .as_ref()
                    .map(|doc| doc.version());
                let request = Request::new(Operation::Rename {
                    source: workdir.join(&relative),
                    name: name.into(),
                });
                pane.filesystem_pause(request.id, cx);
                let result = Filesystem::default().execute(request, |_| {});
                assert!(result.succeeded(), "{:?}", result.items);
                pane.filesystem_finish(result.id, &result.changes, &result.moved_versions, cx);
                assert_eq!(
                    pane.file_editor_key(),
                    Some(DocumentIdentity(workdir.join(name)))
                );
                assert_eq!(pane.file_editor_language, language);
                if language == old_language {
                    assert_eq!(
                        pane.file_editor_live_syntax
                            .as_ref()
                            .map(|doc| doc.version()),
                        old_version
                    );
                }
            });
        });
        relative = PathBuf::from(name);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                push_test_state(view, editor_state(repo_id, &workdir, &relative), cx);
            });
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                pane.ensure_file_editor_loaded(cx);
                assert!(!pane.file_editor_loading());
                assert!(pane.file_editor_dirty());
                assert_eq!(pane.file_editor_saved_fingerprint(), saved);
                let input = pane.file_editor_input.read(cx);
                assert_eq!(input.text_snapshot().model_id(), snapshot.model_id());
                assert_eq!(input.text_snapshot().revision(), snapshot.revision());
                assert_eq!(input.cursor_offset(), cursor);
                let highlights = pane.file_editor_input.update(cx, |input, _| {
                    input.debug_effective_highlights_for_range(0..snapshot.len())
                });
                if let Some(language) = language.filter(|&language| {
                    rows::live_syntax_document_supported(language, snapshot.len())
                }) {
                    let document = pane
                        .file_editor_live_syntax
                        .as_ref()
                        .expect("renamed grammar");
                    assert_eq!(document.language(), language);
                    assert!(!highlights.is_empty());
                } else {
                    assert!(pane.file_editor_live_syntax.is_none());
                    assert!(pane.file_editor_syntax_pair.is_none());
                    assert!(pane.file_editor_occurrences.is_empty());
                    assert_eq!(highlights.is_empty(), language.is_none(), "{name}");
                }
            });
        });
    }
}

#[gpui::test]
fn pending_editor_reads_restart_after_file_renames(cx: &mut gpui::TestAppContext) {
    rename_during_editor_load(cx, false, false);
}

#[gpui::test]
fn pending_editor_reads_restart_after_parent_directory_renames(cx: &mut gpui::TestAppContext) {
    rename_during_editor_load(cx, true, false);
}

#[gpui::test]
fn pending_editor_reads_wait_for_all_filesystem_pauses_before_restarting(
    cx: &mut gpui::TestAppContext,
) {
    rename_during_editor_load(cx, true, true);
}

#[gpui::test]
async fn file_editor_loads_the_working_tree_file_and_starts_clean(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    // The read runs off the foreground, so let it land.
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
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
fn oversized_repository_editor_files_fail_without_capturing_a_baseline(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let directory = tempfile::tempdir().unwrap();
    let relative = PathBuf::from("oversized.txt");
    std::fs::File::create(directory.path().join(&relative))
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(
                view,
                editor_state(
                    gitcomet_state::model::RepoId(1943),
                    directory.path(),
                    &relative,
                ),
                cx,
            );
            view.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.file_editor_error()
                .as_ref()
                .is_some_and(|error| error.contains("32 MB"))
        );
        assert!(pane.file_editor_disk_versions().is_empty());
        assert!(!pane.file_editor_is_dirty());
    });
}

#[gpui::test]
async fn file_editor_refuses_a_binary_file(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(943);
    let workdir = unique_workdir("file_editor_binary");
    let file_rel = std::path::PathBuf::from("blob.bin");
    // NUL bytes and invalid UTF-8 that is not UTF-16 either: binary in any
    // encoding. (`FF FE ...` would be a UTF-16 byte-order mark, i.e. text.)
    std::fs::write(
        workdir.join(&file_rel),
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\xff\xd8",
    )
    .expect("write binary");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.file_editor_error().is_some(),
            "a binary file must surface an error instead of an empty buffer"
        );
        assert!(!pane.file_editor_is_dirty());
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn a_pending_read_does_not_land_in_another_repos_buffer(cx: &mut gpui::TestAppContext) {
    // Regression: the in-flight guard compared only the path, so switching to a
    // second repo holding the same relative path before the read completed
    // seated the first repo's contents in the second repo's buffer — and a save
    // would then have written them to the wrong file.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
            push_test_state(this, editor_state(repo_b, &workdir_b, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                this.diff_renderer
                    .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
            });
        });
        cx.run_until_parked();
    };

    open(cx, &first);
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

    open(cx, &second);
    open(cx, &first);

    // Undoing the edit by hand must return the restored buffer to clean.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..10, "", cx);
                });
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "fn first() {}\n");
        assert!(
            !pane.file_editor_is_dirty(),
            "a restored buffer is measured against its own file, not the one visited in between"
        );
        assert!(pane.unsaved_file_edit_labels().is_empty());
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(966);
    let workdir = unique_workdir("file_editor_blank_stash");
    let first = std::path::PathBuf::from("first.rs");
    let second = std::path::PathBuf::from("second.rs");
    std::fs::write(workdir.join(&first), "fn first() {}\n").expect("write first");
    std::fs::write(workdir.join(&second), "fn second() {}\n").expect("write second");

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());

    // Open the first file, then switch away *without* letting the read land, so
    // the flush sees the blanked buffer.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &first), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &second), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
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
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            diff_renderer.read(app).file_editor_input.read(app).text(),
            "fn first() {}\n",
            "returning to the file must show the file, not a stashed blank"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

fn navigate_during_filesystem_pause(cx: &mut gpui::TestAppContext, second_contents: &str) {
    use gitcomet_core::filesystem::{DocumentIdentity, OperationId};
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let repo_id = gitcomet_state::model::RepoId(997);
    let first = Path::new("first.txt");
    let second = Path::new("second.txt");
    std::fs::write(workdir.join(first), "original A").unwrap();
    std::fs::write(workdir.join(second), second_contents).unwrap();
    let pane = cx.update(|_, app| view.read(app).diff_renderer.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, editor_state(repo_id, &workdir, first), cx);
        });
    });
    cx.update(|_, app| pane.update(app, |pane, cx| pane.ensure_file_editor_loaded(cx)));
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.auto_save_file_edits = false;
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..input.text().len(), "unsaved A", cx);
            });
        });
    });
    cx.run_until_parked();
    let pauses = [OperationId::allocate(), OperationId::allocate()];
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            for id in pauses {
                pane.filesystem_pause(id, cx);
            }
        });
    });
    // Going back to A restores its own stash while paused; every visit to B
    // must hide A and refuse input until B's disk read can actually run.
    for relative in [second, first, second] {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                push_test_state(view, editor_state(repo_id, &workdir, relative), cx);
            });
        });
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                pane.ensure_file_editor_loaded(cx);
                assert_eq!(
                    pane.file_editor_key(),
                    Some(DocumentIdentity(workdir.join(relative)))
                );
                if relative == first {
                    assert!(!pane.file_editor_loading());
                    assert!(pane.file_editor_dirty());
                    pane.file_editor_input.update(cx, |input, cx| {
                        assert_eq!(input.text(), "unsaved A");
                        input.replace_utf8_range(input.text().len()..input.text().len(), "!", cx);
                        assert_eq!(input.text(), "unsaved A!");
                    });
                } else {
                    assert!(pane.file_editor_loading());
                    assert!(!pane.file_editor_dirty());
                    pane.file_editor_input.update(cx, |input, cx| {
                        assert!(input.text().is_empty());
                        input.replace_utf8_range(0..0, "wrong file edit", cx);
                        assert!(
                            input.text().is_empty(),
                            "the loading input must reject edits"
                        );
                    });
                    pane.save_file_editor_buffer(cx);
                    assert!(pane.editor_documents.pending().is_empty());
                }
            });
        });
        cx.run_until_parked();
    }
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            assert!(pane.file_editor_loading());
            pane.filesystem_finish(pauses[0], &[], &Default::default(), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            assert!(
                pane.file_editor_loading(),
                "one operation still holds a pause"
            );
            assert!(pane.file_editor_input.read(cx).text().is_empty());
            pane.filesystem_finish(pauses[1], &[], &Default::default(), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            assert!(!pane.file_editor_loading());
            assert!(pane.file_editor_error().is_none());
            assert_eq!(
                pane.file_editor_stash()[&DocumentIdentity(workdir.join(first))]
                    .text
                    .as_ref(),
                "unsaved A!"
            );
            pane.file_editor_input.update(cx, |input, cx| {
                assert_eq!(input.text(), second_contents);
                input.replace_utf8_range(0..input.text().len(), "edited B", cx);
                assert_eq!(input.text(), "edited B");
            });
        });
    });
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(workdir.join(first)).unwrap(),
        "original A"
    );
    cx.update(|_, app| pane.update(app, |pane, cx| pane.save_all_file_edits(cx)));
    finish_editor_saves(&view, cx);
    assert_eq!(
        std::fs::read_to_string(workdir.join(first)).unwrap(),
        "unsaved A!"
    );
    assert_eq!(
        std::fs::read_to_string(workdir.join(second)).unwrap(),
        "edited B"
    );
}

fn rename_during_editor_load(cx: &mut gpui::TestAppContext, parent: bool, overlapping: bool) {
    use gitcomet_core::filesystem::{
        DocumentIdentity, Filesystem, Operation, OperationId, Request,
    };
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let original = workdir.join("before/file.rs");
    let relative = Path::new(if parent {
        "after/file.rs"
    } else {
        "before/renamed.rs"
    });
    let renamed = workdir.join(relative);
    std::fs::create_dir(original.parent().unwrap()).unwrap();
    std::fs::write(&original, "fn renamed_file() {}\n").unwrap();
    let repo_id = gitcomet_state::model::RepoId(998);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(
                view,
                editor_state(repo_id, &workdir, Path::new("before/file.rs")),
                cx,
            );
        })
    });
    let other_pause = OperationId::allocate();
    let initial_sequence = cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
                assert!(pane.file_editor_loading());
                let sequence = pane.file_editor_reread_seq();
                let request = Request::new(Operation::Rename {
                    source: if parent {
                        original.parent().unwrap().to_path_buf()
                    } else {
                        original.clone()
                    },
                    name: if parent { "after" } else { "renamed.rs" }.into(),
                });
                pane.filesystem_pause(request.id, cx);
                if overlapping {
                    pane.filesystem_pause(other_pause, cx);
                }
                let result = Filesystem::default().execute(request, |_| {});
                assert!(result.succeeded(), "{:?}", result.items);
                // Make the obsolete read succeed with different contents. Its result
                // must never replace the restarted read from the renamed identity.
                std::fs::create_dir_all(original.parent().unwrap()).unwrap();
                std::fs::write(&original, "obsolete path contents\n").unwrap();
                pane.filesystem_finish(result.id, &result.changes, &result.moved_versions, cx);
                assert_eq!(
                    pane.file_editor_key(),
                    Some(DocumentIdentity(renamed.clone()))
                );
                if overlapping {
                    assert_eq!(pane.file_editor_reread_seq(), sequence);
                } else {
                    assert!(pane.file_editor_reread_seq() > sequence);
                }
                sequence
            })
    });
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, editor_state(repo_id, &workdir, relative), cx);
        })
    });
    cx.run_until_parked();
    if overlapping {
        cx.update(|_, app| {
            view.read(app)
                .diff_renderer
                .clone()
                .update(app, |pane, cx| {
                    assert!(pane.file_editor_loading());
                    assert!(pane.file_editor_input.read(cx).text().is_empty());
                    pane.ensure_file_editor_loaded(cx);
                    assert_eq!(pane.file_editor_reread_seq(), initial_sequence);
                    pane.filesystem_finish(other_pause, &[], &Default::default(), cx);
                    assert!(pane.file_editor_reread_seq() > initial_sequence);
                })
        });
        cx.run_until_parked();
    }
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.ensure_file_editor_loaded(cx);
                assert!(!pane.file_editor_loading());
                assert!(pane.file_editor_error().is_none());
                assert!(!pane.file_editor_dirty());
                assert_eq!(
                    pane.file_editor_input.read(cx).text(),
                    "fn renamed_file() {}\n"
                );
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "// edit\n", cx)
                });
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
    assert_eq!(
        std::fs::read_to_string(renamed).unwrap(),
        "// edit\nfn renamed_file() {}\n"
    );
    assert_eq!(
        std::fs::read_to_string(original).unwrap(),
        "obsolete path contents\n"
    );
}

fn replace_active_destination(cx: &mut gpui::TestAppContext, dirty_destination: bool) {
    use gitcomet_core::filesystem::{
        ConflictChoice, ConflictResolution, DiskVersion, DocumentIdentity, Filesystem, Operation,
        Request,
    };
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let source = workdir.join("a.txt");
    let destination = workdir.join("b.txt");
    std::fs::write(&source, "source\n").unwrap();
    std::fs::write(&destination, "destination\n").unwrap();
    let destination_version = DiskVersion::read(&destination).unwrap();
    let repo_id = gitcomet_state::model::RepoId(996);
    for name in ["a.txt", "b.txt"] {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                push_test_state(view, editor_state(repo_id, &workdir, Path::new(name)), cx);
                view.diff_renderer.update(cx, |pane, cx| {
                    pane.auto_save_file_edits = false;
                    pane.ensure_file_editor_loaded(cx);
                });
            })
        });
        cx.run_until_parked();
        if name == "a.txt" || dirty_destination {
            cx.update(|_, app| {
                view.read(app)
                    .diff_renderer
                    .clone()
                    .update(app, |pane, cx| {
                        pane.file_editor_input.update(cx, |input, cx| {
                            input.replace_utf8_range(0..0, "unsaved ", cx);
                        });
                    })
            });
            cx.run_until_parked();
        }
    }
    let mut request = Request::new(Operation::Rename {
        source: source.clone(),
        name: "b.txt".into(),
    });
    request.resolutions.insert(
        destination.clone(),
        ConflictResolution {
            expected: destination_version.clone(),
            choice: ConflictChoice::Replace,
        },
    );
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.filesystem_pause(request.id, cx);
            })
    });
    let result = Filesystem::default().execute(request, |_| {});
    assert!(result.succeeded(), "{:?}", result.items);
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.filesystem_finish(result.id, &result.changes, &result.moved_versions, cx);
                assert_eq!(pane.file_editor_dirty(), dirty_destination);
                assert_eq!(
                    pane.file_editor_input.read(cx).text(),
                    if dirty_destination {
                        "unsaved destination\n"
                    } else {
                        "destination\n"
                    }
                );
                assert_eq!(
                    pane.file_editor_disk_versions()
                        .get(&DocumentIdentity(destination.clone())),
                    Some(&destination_version),
                    "the active destination must keep its own baseline"
                );
            })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app).documents.clone().update(app, |docs, cx| {
            assert_eq!(
                docs.unsaved_labels(cx),
                vec![SharedString::from(destination.display().to_string())]
            );
            docs.save_all(cx);
        })
    });
    let deadline = std::time::Instant::now() + DEFAULT_MAIN_PANE_WAIT_TIMEOUT;
    loop {
        cx.run_until_parked();
        let drained = cx.update(|_, app| {
            view.update(app, |view, cx| {
                crate::view::test_support::sync_store_snapshot(view, cx)
            });
            view.read(app).documents.read(app).saves_drained(app)
        });
        if drained {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "document saves did not finish within {DEFAULT_MAIN_PANE_WAIT_TIMEOUT:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cx.update(|_, app| {
        let docs = view.read(app).documents.read(app);
        assert!(
            docs.unsaved_labels(app).is_empty(),
            "the recovered source must use its moved disk baseline"
        );
        assert_eq!(
            view.read(app).diff_renderer.read(app).file_editor_dirty(),
            dirty_destination
        );
    });
    assert!(!source.exists());
    assert_eq!(
        std::fs::read_to_string(destination).unwrap(),
        "unsaved source\n"
    );
}

#[gpui::test]
fn replacing_a_clean_active_destination_preserves_dirty_source_edits(
    cx: &mut gpui::TestAppContext,
) {
    replace_active_destination(cx, false);
}

#[gpui::test]
fn replacing_a_dirty_active_destination_preserves_both_buffers(cx: &mut gpui::TestAppContext) {
    replace_active_destination(cx, true);
}
