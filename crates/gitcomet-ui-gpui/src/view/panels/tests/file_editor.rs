use super::*;
use crate::view::panes::main::{
    apply_file_editor_pair_highlights, file_editor_blame_line_for_editor_line,
    file_editor_provider_binding_key,
};
use gitcomet_core::services::GitBackend;
use palette::IntoColor;
use std::path::PathBuf;

#[path = "file_editor_scrolling.rs"]
mod scrolling;

/// A repo whose working tree holds `file_rel` with `contents`, already showing
/// that file in the editor.
fn editor_state(
    repo_id: gitcomet_state::model::RepoId,
    workdir: &Path,
    file_rel: &Path,
) -> Arc<AppState> {
    let mut repo = opening_repo_state(repo_id, workdir);
    repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::working_tree(
        file_rel.to_path_buf(),
        gitcomet_core::domain::DiffArea::Unstaged,
    ));
    repo.diff_state.content_preview = true;
    repo.diff_state.edit_mode = true;
    app_state_with_repo(repo, repo_id)
}

fn two_repo_editor_state(
    first: (gitcomet_state::model::RepoId, &Path, &Path),
    second: (gitcomet_state::model::RepoId, &Path, &Path),
    active_repo: gitcomet_state::model::RepoId,
) -> Arc<AppState> {
    let make_repo =
        |(repo_id, workdir, file_rel): (gitcomet_state::model::RepoId, &Path, &Path)| {
            let mut repo = opening_repo_state(repo_id, workdir);
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::working_tree(
                file_rel.to_path_buf(),
                gitcomet_core::domain::DiffArea::Unstaged,
            ));
            repo.diff_state.content_preview = true;
            repo.diff_state.edit_mode = true;
            repo
        };
    Arc::new(AppState {
        repos: vec![make_repo(first), make_repo(second)],
        active_repo: Some(active_repo),
        ..Default::default()
    })
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
fn navigation_during_filesystem_pauses_hides_the_outgoing_buffer(cx: &mut gpui::TestAppContext) {
    navigate_during_filesystem_pause(cx, "original B");
}

#[gpui::test]
fn an_empty_file_loaded_after_a_filesystem_pause_becomes_editable(cx: &mut gpui::TestAppContext) {
    navigate_during_filesystem_pause(cx, "");
}

fn navigate_during_filesystem_pause(cx: &mut gpui::TestAppContext, second_contents: &str) {
    use gitcomet_core::filesystem::{DocumentIdentity, OperationId};
    let _guard = lock_visual_test();
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
    let pane = cx.update(|_, app| view.read(app).main_pane.clone());
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
                    pane.file_editor_key,
                    Some(DocumentIdentity(workdir.join(relative)))
                );
                if relative == first {
                    assert!(!pane.file_editor_loading);
                    assert!(pane.file_editor_dirty);
                    pane.file_editor_input.update(cx, |input, cx| {
                        assert_eq!(input.text(), "unsaved A");
                        input.replace_utf8_range(input.text().len()..input.text().len(), "!", cx);
                        assert_eq!(input.text(), "unsaved A!");
                    });
                } else {
                    assert!(pane.file_editor_loading);
                    assert!(!pane.file_editor_dirty);
                    pane.file_editor_input.update(cx, |input, cx| {
                        assert!(input.text().is_empty());
                        input.replace_utf8_range(0..0, "wrong file edit", cx);
                        assert!(
                            input.text().is_empty(),
                            "the loading input must reject edits"
                        );
                    });
                    pane.save_file_editor_buffer(cx);
                    assert!(pane.file_editor_saves.is_empty());
                }
            });
        });
        cx.run_until_parked();
    }
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            assert!(pane.file_editor_loading);
            pane.filesystem_finish(pauses[0], &[], &Default::default(), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            assert!(
                pane.file_editor_loading,
                "one operation still holds a pause"
            );
            assert!(pane.file_editor_input.read(cx).text().is_empty());
            pane.filesystem_finish(pauses[1], &[], &Default::default(), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            assert!(!pane.file_editor_loading);
            assert!(pane.file_editor_error.is_none());
            assert_eq!(
                pane.file_editor_stash[&DocumentIdentity(workdir.join(first))]
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

#[gpui::test]
fn renaming_the_active_editor_refreshes_its_language_without_replacing_edits(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::filesystem::{DocumentIdentity, Filesystem, Operation, Request};
    let _guard = lock_visual_test();
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
    let pane = cx.update(|_, app| view.read(app).main_pane.clone());
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
            pane.file_editor_saved_fingerprint,
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
                    pane.file_editor_key,
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
                assert!(!pane.file_editor_loading);
                assert!(pane.file_editor_dirty);
                assert_eq!(pane.file_editor_saved_fingerprint, saved);
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

fn rename_during_editor_load(cx: &mut gpui::TestAppContext, parent: bool, overlapping: bool) {
    use gitcomet_core::filesystem::{
        DocumentIdentity, Filesystem, Operation, OperationId, Request,
    };
    let _guard = lock_visual_test();
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
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.auto_save_file_edits = false;
            pane.ensure_file_editor_loaded(cx);
            assert!(pane.file_editor_loading);
            let sequence = pane.file_editor_reread_seq;
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
                pane.file_editor_key,
                Some(DocumentIdentity(renamed.clone()))
            );
            if overlapping {
                assert_eq!(pane.file_editor_reread_seq, sequence);
            } else {
                assert!(pane.file_editor_reread_seq > sequence);
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
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                assert!(pane.file_editor_loading);
                assert!(pane.file_editor_input.read(cx).text().is_empty());
                pane.ensure_file_editor_loaded(cx);
                assert_eq!(pane.file_editor_reread_seq, initial_sequence);
                pane.filesystem_finish(other_pause, &[], &Default::default(), cx);
                assert!(pane.file_editor_reread_seq > initial_sequence);
            })
        });
        cx.run_until_parked();
    }
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.ensure_file_editor_loaded(cx);
            assert!(!pane.file_editor_loading);
            assert!(pane.file_editor_error.is_none());
            assert!(!pane.file_editor_dirty);
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
            .main_pane
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

fn replace_active_destination(cx: &mut gpui::TestAppContext, dirty_destination: bool) {
    use gitcomet_core::filesystem::{
        ConflictChoice, ConflictResolution, DiskVersion, DocumentIdentity, Filesystem, Operation,
        Request,
    };
    let _guard = lock_visual_test();
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
                view.main_pane.update(cx, |pane, cx| {
                    pane.auto_save_file_edits = false;
                    pane.ensure_file_editor_loaded(cx);
                });
            })
        });
        cx.run_until_parked();
        if name == "a.txt" || dirty_destination {
            cx.update(|_, app| {
                view.read(app).main_pane.clone().update(app, |pane, cx| {
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
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.filesystem_pause(request.id, cx);
        })
    });
    let result = Filesystem::default().execute(request, |_| {});
    assert!(result.succeeded(), "{:?}", result.items);
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.filesystem_finish(result.id, &result.changes, &result.moved_versions, cx);
            assert_eq!(pane.file_editor_dirty, dirty_destination);
            assert_eq!(
                pane.file_editor_input.read(cx).text(),
                if dirty_destination {
                    "unsaved destination\n"
                } else {
                    "destination\n"
                }
            );
            assert_eq!(
                pane.file_editor_disk_versions
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
    for _ in 0..200 {
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
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cx.update(|_, app| {
        let docs = view.read(app).documents.read(app);
        assert!(docs.saves_drained(app));
        assert!(
            docs.unsaved_labels(app).is_empty(),
            "the recovered source must use its moved disk baseline"
        );
        assert_eq!(
            view.read(app).main_pane.read(app).file_editor_dirty,
            dirty_destination
        );
    });
    assert!(!source.exists());
    assert_eq!(
        std::fs::read_to_string(destination).unwrap(),
        "unsaved source\n"
    );
}

#[cfg(unix)]
#[gpui::test]
fn repository_editor_rejects_saves_through_a_symlinked_directory(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
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
            view.main_pane.update(cx, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
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
            .main_pane
            .clone()
            .update(app, |pane, cx| pane.save_file_editor_buffer(cx))
    });
    finish_editor_saves(&view, cx);
    cx.update(|_, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.file_editor_dirty);
        assert!(pane.file_editor_error.is_some());
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
async fn file_editor_loads_the_working_tree_file_and_starts_clean(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
fn failed_autosaves_reach_the_unsaved_edits_prompt_without_retrying_forever(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
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
            view.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
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
            .main_pane
            .clone()
            .update(app, |pane, cx| pane.save_file_editor_buffer(cx))
    });
    finish_editor_saves(&view, cx);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            assert!(view.main_pane.read(cx).file_editor_is_dirty());
            assert!(view.main_pane.read(cx).file_editor_error.is_some());
            assert!(view.request_unsaved_file_edits_prompt(UnsavedFileEditsAction::QuitApp, cx));
            assert!(
                view.pending_unsaved_file_edits_prompt.is_some(),
                "a failed autosave must allow Save/Discard"
            );
            assert!(
                view.main_pane.read(cx).file_editor_saves.is_empty(),
                "closing must not silently retry the failed write"
            );
            assert_eq!(
                view.main_pane.read(cx).file_editor_input.read(cx).text(),
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

fn save_completion_after_navigation(cx: &mut gpui::TestAppContext, autosave: bool, paused: bool) {
    let _visual_guard = lock_visual_test();
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
    let pane = cx.update(|_, app| view.read(app).main_pane.clone());
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
            let key = pane.file_editor_key.clone().unwrap();
            assert!(
                pane.file_editor_stash.contains_key(&key),
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
            model.set_state(editor_state(repo_id, &workdir, &second), cx);
        });
    });
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.ensure_file_editor_loaded(cx);
            assert_eq!(pane.file_editor_saves.len(), 1);
            assert_eq!(
                pane.file_editor_key.as_ref().unwrap().0,
                workdir.join(&second)
            );
            assert_eq!(pane.file_editor_stash[&key].text.as_ref(), "newer edits");
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
                assert!(pane.file_editor_stash[&key].is_dirty());
                pane.filesystem_finish(pauses[0], &[], &Default::default(), cx);
                assert!(pane.file_editor_saves.is_empty());
                pane.filesystem_finish(pauses[1], &[], &Default::default(), cx);
                assert_eq!(pane.file_editor_saves.len(), usize::from(autosave));
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
            let stashed = &pane.read(app).file_editor_stash[&key];
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
        assert!(pane.read(app).file_editor_stash.is_empty());
        assert!(pane.read(app).unsaved_file_edit_labels().is_empty());
        assert!(pane.read(app).file_editor_error.is_none());
    });
}

#[gpui::test]
fn oversized_repository_editor_files_fail_without_capturing_a_baseline(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
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
            view.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_editor_error
                .as_ref()
                .is_some_and(|error| error.contains("32 MB"))
        );
        assert!(pane.file_editor_disk_versions.is_empty());
        assert!(!pane.file_editor_is_dirty());
    });
}

#[gpui::test]
async fn file_editor_marks_dirty_on_edit_and_clean_after_save(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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

    finish_editor_saves(&view, cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.file_editor_is_dirty(),
            "a confirmed save settles the buffer"
        );
        assert!(pane.unsaved_file_edit_labels().is_empty());
        assert!(
            pane.file_editor_stash.is_empty(),
            "acknowledged clean buffers must be released"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn file_editor_keeps_an_unsaved_buffer_across_a_file_switch(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
async fn file_editor_refuses_a_binary_file(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
fn pair_overlay_paints_both_delimiters_over_the_syntax_runs() {
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

    let highlights = apply_file_editor_pair_highlights(
        vec![(0..2, keyword), (open..close + 1, keyword)],
        Some(&rows::SyntaxPair {
            open: open..open + 1,
            close: close..close + 1,
            kind: rows::SyntaxPairKind::Bracket,
        }),
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
fn whole_tag_pair_overlay_preserves_disjoint_syntax_runs() {
    let punctuation = gpui::HighlightStyle {
        color: Some(gpui::rgb(0x112233).into_color()),
        ..Default::default()
    };
    let tag_name = gpui::HighlightStyle {
        color: Some(gpui::rgb(0x445566).into_color()),
        ..Default::default()
    };
    let attribute = gpui::HighlightStyle {
        color: Some(gpui::rgb(0x778899).into_color()),
        ..Default::default()
    };
    let pair_style = gpui::HighlightStyle {
        background_color: Some(gpui::rgba(0xffffff26).into_color()),
        ..Default::default()
    };
    let runs = vec![
        (0..1, punctuation),
        (1..4, tag_name),
        (5..10, attribute),
        (10..11, punctuation),
        (11..12, punctuation),
        (12..15, tag_name),
        (15..16, punctuation),
    ];

    let highlights = apply_file_editor_pair_highlights(
        runs,
        Some(&rows::SyntaxPair {
            open: 0..11,
            close: 11..16,
            kind: rows::SyntaxPairKind::Tag,
        }),
        0..16,
        pair_style,
    );

    assert_eq!(
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color,))
            .collect::<Vec<_>>(),
        vec![
            (0..1, punctuation.color, pair_style.background_color),
            (1..4, tag_name.color, pair_style.background_color),
            // The grammar left the whitespace unstyled; the overlay fills only
            // that gap instead of adding a run across the whole start tag.
            (4..5, pair_style.color, pair_style.background_color),
            (5..10, attribute.color, pair_style.background_color),
            (10..11, punctuation.color, pair_style.background_color),
            (11..12, punctuation.color, pair_style.background_color),
            (12..15, tag_name.color, pair_style.background_color),
            (15..16, punctuation.color, pair_style.background_color),
        ]
    );

    for adjacent in highlights.windows(2) {
        assert_eq!(
            adjacent[0].0.end, adjacent[1].0.start,
            "whole-tag overlay runs must form a sorted, disjoint tiling"
        );
    }
}

#[test]
fn pair_overlay_is_a_no_op_without_a_pair() {
    let style = gpui::HighlightStyle::default();
    let runs = vec![(0..4, style), (6..9, style)];
    assert_eq!(
        apply_file_editor_pair_highlights(runs.clone(), None, 0..9, style),
        runs
    );
}

#[test]
fn provider_binding_key_changes_only_when_something_changed() {
    let pair = rows::SyntaxPair {
        open: 3..4,
        close: 9..10,
        kind: rows::SyntaxPairKind::Bracket,
    };
    let no_matches: &[std::ops::Range<usize>] = &[];
    let no_occurrences: &[std::ops::Range<usize>] = &[];
    // Two ranges, not one: clippy reads a single-range array literal as a typo.
    let one_match: &[std::ops::Range<usize>] = &[20..25, 40..45];
    let other_match: &[std::ops::Range<usize>] = &[30..35, 40..45];
    let base = file_editor_provider_binding_key(7, 1, Some(&pair), no_matches, no_occurrences);

    assert_eq!(
        base,
        file_editor_provider_binding_key(7, 1, Some(&pair), no_matches, no_occurrences),
        "an unchanged binding must not rebind — that is what stops the observe cycle"
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(8, 1, Some(&pair), no_matches, no_occurrences)
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(7, 2, Some(&pair), no_matches, no_occurrences)
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(
            7,
            1,
            Some(&rows::SyntaxPair {
                open: 3..4,
                close: 12..13,
                kind: rows::SyntaxPairKind::Bracket,
            }),
            no_matches,
            no_occurrences
        ),
        "moving the caret to another pair must rebind"
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(
            7,
            1,
            Some(&rows::SyntaxPair {
                open: 3..4,
                close: 9..10,
                kind: rows::SyntaxPairKind::Quote,
            }),
            no_matches,
            no_occurrences
        ),
        "same span, different kind: the key must still separate them"
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(7, 1, None, no_matches, no_occurrences)
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(7, 1, Some(&pair), one_match, no_occurrences),
        "a search match moves no text and touches no tree, so this key is the \
         only thing that can tell the input its highlights changed"
    );
    assert_ne!(
        file_editor_provider_binding_key(7, 1, Some(&pair), one_match, no_occurrences),
        file_editor_provider_binding_key(7, 1, Some(&pair), other_match, no_occurrences),
        "stepping to the next match changes which hits are washed"
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(7, 1, Some(&pair), no_matches, &[20..25, 40..45]),
        "moving the caret onto a name moves no text and touches no tree, so this \
         key is the only thing that can tell the input its highlights changed"
    );
}

#[gpui::test]
async fn alt_e_toggles_the_editor_and_ctrl_s_saves_and_exits_while_it_has_focus(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    finish_editor_saves(&view, cx);
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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

/// The editor used to apply its 256 KiB caret-scan ceiling before asking what
/// the caret was on, so this crate's own `syntax.rs` parsed and colored normally
/// but no identifier click produced occurrence highlights.
#[gpui::test]
async fn file_editor_click_highlights_names_in_the_actual_syntax_rs(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(965);
    let workdir = unique_workdir("file_editor_actual_syntax_rs_click");
    let file_rel = std::path::PathBuf::from("syntax.rs");
    let source_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/view/rows/diff_text/syntax.rs");
    let mut contents = std::fs::read_to_string(&source_path).expect("read actual syntax.rs");
    for module in [
        "prepared/drop_queue.rs",
        "prepared/document_cache.rs",
        "prepared/document_prepare.rs",
        "prepared/incremental_reparse.rs",
        "prepared/highlight_spec.rs",
        "prepared/query_tokens.rs",
        "prepared/injections.rs",
        "live.rs",
        "heuristic.rs",
    ] {
        let module_path = source_path
            .parent()
            .expect("syntax dir")
            .join("syntax")
            .join(module);
        contents.push_str(
            &std::fs::read_to_string(module_path)
                .expect("read syntax module as part of the large-file fixture"),
        );
    }
    assert!(
        contents.len() > 256 * 1024,
        "the regression needs a large file"
    );
    assert!(
        contents.len() <= rows::OCCURRENCE_MAX_TEXT_BYTES,
        "the actual source must stay inside the interactive scan ceiling"
    );
    std::fs::write(workdir.join(&file_rel), &contents).expect("write syntax.rs editor fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let target = "ensure_tree_sitter_allocator";
    let target_start = contents
        .find(target)
        .expect("actual syntax.rs should retain the allocator funnel");
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            assert!(
                pane.file_editor_live_syntax.is_some(),
                "the large Rust file should have a live syntax tree"
            );
            pane.file_editor_input.update(cx, |input, cx| {
                input.set_cursor_offset(target_start + 3, cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        assert!(
            pane.file_editor_occurrences.iter().any(
                |range| range.start == target_start && range.end == target_start + target.len()
            ),
            "clicking the allocator name should highlight it in the actual half-megabyte file; \
             occurrences={:?}",
            pane.file_editor_occurrences,
        );
        assert!(
            pane.file_editor_occurrences.len() >= 3,
            "the definition and calls should all be highlighted"
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
            repo.diff_state.blame_path = Some(file_rel.to_path_buf());
            repo.diff_state.blame_source = Some(gitcomet_core::domain::BlameSource::WorkingTree(
                gitcomet_core::domain::DiffArea::Unstaged,
            ));
            repo.diff_state.blame =
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
        });
        let _ = window.draw(app);
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
        });
        let _ = window.draw(app);
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    // A second pass: the row counts are maintained during the buffer's prepaint,
    // so the frame that can project them is the one after it has laid out.
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
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
    // Regression: a save is followed by a disk check (the write bumps the
    // repo's revisions), and an earlier version of that follow-up blanked the
    // input before re-reading, so every save re-seated the buffer — resetting
    // the caret to 0 and clearing undo, once per auto-save.
    let _visual_guard = lock_visual_test();
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
    finish_editor_saves(&view, cx);
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
async fn closing_a_repository_retains_its_unsaved_buffer_in_documents(
    cx: &mut gpui::TestAppContext,
) {
    // Regression: a stash entry whose repo tab had closed stayed dirty forever.
    // The quit dialog listed it, "Save all" could not write it (the store drops
    // messages for a repo it no longer has), and the retry raised the dialog
    // again — the window could never be closed.
    let _visual_guard = lock_visual_test();
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
            let mut state = AppState::test_default();
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
async fn review_regression_moving_a_repository_prompts_before_detaching_dirty_editor_buffers(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
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
            this.main_pane.update(cx, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        });
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// unsaved a\n", cx);
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
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
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
async fn review_regression_confirmed_noop_move_skips_destructive_guards(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
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
            this.main_pane.update(cx, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
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
                this.main_pane
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
    let _visual_guard = lock_visual_test();
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
            this.main_pane.update(cx, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        });
    });
    cx.run_until_parked();
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
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
                this.main_pane
                    .read(cx)
                    .unsaved_file_edit_labels_for_repo(first_id)
                    .len(),
                1
            );
            assert_eq!(
                this.main_pane
                    .read(cx)
                    .unsaved_file_edit_labels_for_repo(second_id)
                    .len(),
                1
            );
            this.resolve_unsaved_file_edits(action, true, cx);
            assert!(
                !this
                    .main_pane
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
            this.main_pane.update(cx, |pane, cx| {
                pane.auto_save_file_edits = true;
                pane.ensure_file_editor_loaded(cx);
            });
        });
        let _ = window.draw(app);
        window.window_handle().window_id()
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
    let action = cx.update(|_window, app| make_action(window_id, app));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(
                this.request_unsaved_file_edits_prompt(action.clone(), cx),
                "the pending auto-save write must hold the action"
            );
            assert!(
                !this.main_pane.read(cx).file_editor_saves.is_empty(),
                "the flush queues the write"
            );
        })
    });
    // Start the drain waiter before the clock moves.
    cx.run_until_parked();
    action
}

/// Let background store work run, then step GPUI's clock by `total`.
fn advance_file_edit_drain(cx: &mut gpui::VisualTestContext, total: std::time::Duration) {
    std::thread::sleep(std::time::Duration::from_millis(175));
    let step = std::time::Duration::from_millis(25);
    let mut elapsed = std::time::Duration::ZERO;
    while elapsed < total {
        cx.executor().advance_clock(step);
        cx.run_until_parked();
        elapsed += step;
    }
}

#[gpui::test]
fn pr530_close_requested_during_move_drain_is_retried(cx: &mut gpui::TestAppContext) {
    check_shutdown_requested_during_move_drain(cx, false);
}

#[gpui::test]
fn pr530_quit_requested_during_move_drain_is_retried(cx: &mut gpui::TestAppContext) {
    check_shutdown_requested_during_move_drain(cx, true);
}

fn check_shutdown_requested_during_move_drain(cx: &mut gpui::TestAppContext, quit: bool) {
    let _guard = lock_visual_test();
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
    advance_file_edit_drain(cx, std::time::Duration::from_millis(100));
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
    let _visual_guard = lock_visual_test();
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
                view.main_pane
                    .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
            })
        });
        cx.run_until_parked();
    };
    for path in [&first, &second] {
        std::fs::write(workdir.join(path), "original\n").unwrap();
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.main_pane
                    .update(cx, |pane, _| pane.auto_save_file_edits = false);
            })
        });
        open(cx, path);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.main_pane.update(cx, |pane, cx| {
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
    let held = hold_editor_saves();
    let dispatch = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.main_pane.update(cx, |pane, cx| {
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
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_saves.len(), 2);
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
    advance_file_edit_drain(cx, std::time::Duration::from_millis(50));
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
            let pane = view.read(app).main_pane.read(app);
            assert!(pane.file_editor_dirty);
            assert_eq!(
                pane.file_editor_input.read(app).text(),
                "edited\noriginal\n"
            );
            assert_eq!(pane.unsaved_file_edit_paths(repo_id), vec![first.clone()]);
        }
    });
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

/// A long action in another repository must not hold a close whose own
/// write has already landed.
#[gpui::test]
async fn review_regression_unrelated_repo_action_does_not_block_close(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
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
    advance_file_edit_drain(cx, std::time::Duration::from_millis(200));

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
    let _visual_guard = lock_visual_test();
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
    advance_file_edit_drain(cx, std::time::Duration::from_secs(6));

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
                assert!(view.main_pane.read(cx).unsaved_file_edit_keys().is_empty());
                assert!(
                    !view.main_pane.read(cx).file_editor_saves.is_empty(),
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
        advance_file_edit_drain(cx, std::time::Duration::from_secs(6));
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
    advance_file_edit_drain(cx, std::time::Duration::from_millis(100));
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
async fn review_regression_confirmed_failed_save_aborts_repository_move(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
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
            this.main_pane.update(cx, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        });
        let _ = window.draw(app);
        window.window_handle().window_id()
    });
    cx.run_until_parked();

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
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
    let held = hold_editor_saves();
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
    advance_file_edit_drain(cx, std::time::Duration::from_millis(200));

    assert!(
        cx.cx.update(|app| app
            .windows()
            .iter()
            .any(|window| window.window_id() == source_window_id)),
        "a failed editor write must leave the source window and repository intact"
    );
    cx.update(|_window, app| {
        assert!(
            !main_pane
                .read(app)
                .unsaved_file_edit_labels_for_repo(repo_id)
                .is_empty(),
            "the refused save must remain visibly unsaved"
        );
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    finish_editor_saves(&view, cx);
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
fn assert_editor_renders_the_engines_highlights(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    label: &str,
    file_rel: &str,
    contents: &str,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
        assert_eq!(
            document.language(),
            rows::diff_syntax_language_for_path(&file_rel)
                .unwrap_or_else(|| panic!("{label}: the fixture path must resolve a language")),
            "{label}: the editor must build the grammar selected from the file extension"
        );
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
fn the_editor_renders_rust_highlights_as_the_engine_produced_them(cx: &mut gpui::TestAppContext) {
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
    );
}

#[gpui::test]
fn the_editor_renders_shell_highlights_as_the_engine_produced_them(cx: &mut gpui::TestAppContext) {
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
    );
}

#[gpui::test]
fn the_editor_renders_nunjucks_with_the_jinja_grammar(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    assert_editor_renders_the_engines_highlights(
        cx,
        gitcomet_state::model::RepoId(964),
        "file_editor_nunjucks_highlights",
        "index.njk",
        // Front matter (yaml), markup (html) and a script body (javascript,
        // depth 2) all on one fixture, since each is its own layer.
        concat!(
            "---\n",
            "title: Home\n",
            "---\n",
            "{# navigation #}\n",
            "<nav class=\"menu\">\n",
            "  {% for item in items %}\n",
            "    <a href=\"{{ item.url }}\">{{ item.label | upper }}</a>\n",
            "  {% endfor %}\n",
            "</nav>\n",
            "<script>\nconst open = false;\n</script>\n",
        ),
    );
}

#[gpui::test]
async fn discarding_from_the_toolbar_returns_to_the_read_only_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
            this.store.insert_repo_for_test(
                repo_id,
                Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
                    workdir.clone(),
                )),
            );
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
    // The write runs on the filesystem worker; wait for its confirmation.
    finish_editor_saves(&view, cx);
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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

/// Put `contents` on disk and show it in the editor.
fn seed_editor(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: u64,
    label: &str,
    contents: &str,
) -> std::path::PathBuf {
    let repo_id = gitcomet_state::model::RepoId(repo_id);
    let workdir = unique_workdir(label);
    let file_rel = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();
    draw_and_drain_test_window(cx);
    workdir
}

#[gpui::test]
fn short_file_editor_uses_the_space_below_eof_for_caret_drag_and_menu(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let contents = "alpha\nbeta";
    let workdir = seed_editor(&view, cx, 998, "file_editor_below_eof_selection", contents);

    let viewport = cx
        .debug_bounds("file_editor_scroll")
        .expect("short editor viewport bounds");
    let below_eof = point(viewport.center().x, viewport.bottom() - px(24.0));
    let target = point(viewport.left() + px(28.0), viewport.top() + px(10.0));
    let target_offset = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let input = pane.file_editor_input.read(app);
        assert_eq!(input.offset_for_position(below_eof), contents.len());
        input.offset_for_position(target)
    });
    assert!(target_offset < contents.len());

    cx.simulate_mouse_down(below_eof, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(below_eof, MouseButton::Left, Modifiers::default());
    assert_eq!(
        editor_selected_range(&view, cx),
        contents.len()..contents.len()
    );

    cx.simulate_mouse_down(below_eof, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    assert_eq!(
        editor_selected_range(&view, cx),
        target_offset..contents.len()
    );

    cx.update(|window, app| {
        let pane = view.read(app).main_pane.clone();
        pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.set_selected_range(0..2, false, window, cx)
            });
        });
    });
    cx.simulate_mouse_down(below_eof, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(below_eof, MouseButton::Right, Modifiers::default());
    assert_eq!(
        editor_selected_range(&view, cx),
        contents.len()..contents.len()
    );
    assert!(
        cx.debug_bounds("text_input_context_select_all").is_some(),
        "right-clicking below EOF should open the normal text-input menu"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.file_editor_scroll.max_offset().y,
            px(0.0),
            "filling the hit surface must not make a short file scroll vertically"
        );
    });

    std::fs::remove_dir_all(workdir).expect("cleanup below-EOF editor fixture");
}

/// Open the search box over the editor and put `query` in it, the way Ctrl+F
/// followed by typing does. Returns once the reveal has been drawn.
fn search_the_editor_for(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    query: &str,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                assert!(
                    pane.open_search_for_active_view(window, cx),
                    "the editor is showing a working-tree file, so search must open over it"
                );
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query, cx));
            });
        });
    });
    cx.run_until_parked();
    // Typing schedules a debounced recompute; run it now rather than on a timer.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });
    draw_and_drain_test_window(cx);
}

fn editor_search_matches(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Vec<std::ops::Range<usize>> {
    cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .file_editor_search_matches
            .clone()
    })
}

fn editor_selected_range(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> std::ops::Range<usize> {
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input
                .update(cx, |input, _| input.selected_range())
        })
    })
}

fn set_editor_text(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    text: &str,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_input
                    .update(cx, |input, cx| input.set_text(text, cx));
            });
        });
    });
    cx.run_until_parked();
}

/// The editor counts occurrences, not rows. Every other view counts matching
/// rows, and reusing that here would make Enter skip repeats within a line.
#[gpui::test]
async fn file_editor_search_counts_every_occurrence_in_the_live_buffer(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let contents = "let needle = needle.needle();\nfn other() {}\nlast needle\n";
    let workdir = seed_editor(&view, cx, 981, "file_editor_search_count", contents);

    search_the_editor_for(&view, cx, "needle");

    let matches = editor_search_matches(&view, cx);
    assert_eq!(
        matches.len(),
        4,
        "three hits on the first line and one on the last are four stops"
    );
    for range in &matches {
        assert_eq!(&contents[range.clone()], "needle");
    }
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_matches,
            vec![0, 0, 0, 2],
            "the list the `n/N` label reads holds one entry per occurrence, \
             carrying the line that occurrence sits on"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The regression this change exists for: edit mode leaves `content_preview`
/// set, so the search used to take the file preview's branch and count the
/// *pre-edit* preview text — a plausible number against a document that is not
/// on screen.
#[gpui::test]
async fn file_editor_search_reads_the_buffer_not_the_stale_preview(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let workdir = seed_editor(
        &view,
        cx,
        982,
        "file_editor_search_live",
        "fn main() {\n    let value = 1;\n}\n",
    );

    // A needle that exists nowhere on disk.
    set_editor_text(&view, cx, "fn main() {\n    let needle = needle;\n}\n");
    search_the_editor_for(&view, cx, "needle");
    assert_eq!(
        editor_search_matches(&view, cx).len(),
        2,
        "the search must read the buffer, including edits that are not on disk"
    );

    set_editor_text(&view, cx, "needle needle needle\n");
    assert_eq!(
        editor_search_matches(&view, cx).len(),
        3,
        "editing the buffer with the search open must recount against it"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Every match wears the search wash except the current one, which the
/// selection marks instead. Selection quads are painted *before* highlight
/// backgrounds, so a wash over the current match would cover it.
#[gpui::test]
async fn file_editor_search_selects_the_current_match_and_washes_the_rest(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let contents = "alpha needle beta\nneedle gamma\n";
    let workdir = seed_editor(&view, cx, 983, "file_editor_search_paint", contents);

    search_the_editor_for(&view, cx, "needle");

    let matches = editor_search_matches(&view, cx);
    assert_eq!(matches.len(), 2);
    let (first, second) = (matches[0].clone(), matches[1].clone());

    assert_eq!(
        editor_selected_range(&view, cx),
        first,
        "activating the search lands on the first match and selects it"
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let wash = main_pane
            .read(app)
            .theme
            .colors
            .editor
            .search_match_background
            .into_color();
        let highlights = main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, _| {
                input.debug_effective_highlights_for_range(0..contents.len())
            })
        });

        assert!(
            !highlights
                .iter()
                .any(|(range, style)| *range == first && style.background_color == Some(wash)),
            "the current match is left to the selection, not washed over it: {highlights:?}"
        );
        assert!(
            highlights
                .iter()
                .any(|(range, style)| *range == second && style.background_color == Some(wash)),
            "every other match wears the search wash: {highlights:?}"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Stepping visits repeats within a line, then wraps.
#[gpui::test]
async fn file_editor_search_next_match_steps_within_a_line(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let workdir = seed_editor(
        &view,
        cx,
        984,
        "file_editor_search_step",
        "needle needle needle\n",
    );

    search_the_editor_for(&view, cx, "needle");
    let matches = editor_search_matches(&view, cx);
    assert_eq!(matches.len(), 3);

    let mut walked = Vec::new();
    for _ in 0..4 {
        walked.push(editor_selected_range(&view, cx));
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane
                    .update(cx, |pane, _cx| pane.diff_search_next_match());
            });
        });
        draw_and_drain_test_window(cx);
    }

    assert_eq!(
        walked,
        vec![
            matches[0].clone(),
            matches[1].clone(),
            matches[2].clone(),
            matches[0].clone(),
        ],
        "three same-line hits are three stops, and the walk wraps"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn file_editor_search_keeps_navigation_relative_to_the_match_before_edits(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let contents = "needle\nneedle\nneedle\n";
    let workdir = seed_editor(
        &view,
        cx,
        999,
        "file_editor_search_pending_navigation",
        contents,
    );
    search_the_editor_for(&view, cx, "needle");

    for (case, (current, edits, next, expected)) in [
        (1, 1, true, 2),
        (1, 2, true, 2),
        (1, 2, false, 0),
        (0, 1, false, 2),
        (2, 1, true, 0),
    ]
    .into_iter()
    .enumerate()
    {
        cx.update(|_, app| {
            let pane = view.read(app).main_pane.clone();
            pane.update(app, |pane, cx| {
                pane.diff_search_match_ix = Some(current);
                for edit in 0..edits {
                    pane.file_editor_input.update(cx, |input, cx| {
                        input.set_text(format!("{contents}case {case} edit {edit}"), cx);
                    });
                    pane.on_file_editor_edited(cx);
                }
                assert!(pane.diff_search_worker_running);
                assert!(pane.diff_search_matches.is_empty());
                if next {
                    pane.diff_search_next_match();
                } else {
                    pane.diff_search_prev_match();
                }
            });
        });
        draw_and_drain_test_window(cx);
        cx.update(|_, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(pane.diff_search_matches, vec![0, 1, 2]);
            assert_eq!(pane.diff_search_match_ix, Some(expected), "case {case}");
            assert!(!pane.diff_search_worker_running);
            assert!(pane.diff_search_pending_previous_query.is_none());
        });
    }

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A match below the fold has to bring the buffer with it. The editor is a
/// `TextInput` over a plain `ScrollHandle`, so there is no deferred
/// `scroll_to_item` to inherit.
#[gpui::test]
async fn file_editor_search_scrolls_a_distant_match_into_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let mut lines: Vec<String> = (0..400)
        .map(|line| format!("fn line_{line}() {{}}"))
        .collect();
    lines.push("fn needle() {}".to_string());
    let contents = format!("{}\n", lines.join("\n"));
    let workdir = seed_editor(&view, cx, 985, "file_editor_search_scroll", &contents);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_editor_scroll.max_offset().y > px(0.0),
            "the fixture must be taller than the viewport for this to mean anything"
        );
        assert_eq!(pane.file_editor_scroll.offset().y, px(0.0));
    });

    search_the_editor_for(&view, cx, "needle");
    // The reveal is applied by the render pass, and the input's own caret
    // autoscroll settles on the frame after that.
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_search_matches.len(), 1);
        // Asserting movement, not a pixel value: `#[gpui::test]` measures every
        // glyph identically, so exact offsets say nothing about the real app.
        assert!(
            pane.file_editor_scroll.offset().y < px(0.0),
            "a match 400 lines down must scroll the buffer, got {:?}",
            pane.file_editor_scroll.offset()
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A match far along a long line has to be scrolled to sideways too.
///
/// A multiline `TextInput` leaves horizontal scrolling to its container, and its
/// own caret autoscroll only handles the vertical axis, so without the reveal
/// the line comes into view with the hit still off the right edge.
#[gpui::test]
async fn file_editor_search_scrolls_sideways_to_a_match_far_along_a_line(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let mut lines: Vec<String> = (0..20)
        .map(|line| format!("fn line_{line}() {{}}"))
        .collect();
    // The needle sits well past any plausible viewport width.
    lines.push(format!("// {}needle", "pad ".repeat(200)));
    let contents = format!("{}\n", lines.join("\n"));
    let workdir = seed_editor(&view, cx, 986, "file_editor_search_hscroll", &contents);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                // Wrapping would fold the long line into the pane and leave
                // nothing to scroll sideways to.
                pane.diff_word_wrap = false;
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_editor_scroll.max_offset().x > px(0.0),
            "the fixture must overflow sideways for this to mean anything; max={:?}",
            pane.file_editor_scroll.max_offset()
        );
        assert_eq!(pane.file_editor_scroll.offset().x, px(0.0));
    });

    search_the_editor_for(&view, cx, "needle");
    // One frame places the caret, the next measures it and scrolls to it.
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_search_matches.len(), 1);
        assert!(
            pane.file_editor_scroll.offset().x < px(0.0),
            "a match far along the line must scroll the editor sideways, got {:?}",
            pane.file_editor_scroll.offset()
        );
        assert!(
            !pane.file_editor_search_reveal_x_pending,
            "the sideways reveal should be claimed once, not retried every frame"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The whole loop, driven by the keys the user actually presses.
///
/// Ctrl+F reaches the search over a focused editor only because its binding
/// carries no context predicate — `handle_diff_shortcut` hands every other
/// keystroke to the buffer.
#[gpui::test]
async fn ctrl_f_and_escape_walk_in_and_out_of_the_editor(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let contents = "alpha needle beta\nneedle gamma\n";
    let workdir = seed_editor(&view, cx, 986, "file_editor_search_keys", contents);

    cx.update(|window, app| {
        crate::app::bind_app_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let focus = pane.file_editor_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_search_active,
            "ctrl+f must open the search even with the buffer focused"
        );
        assert!(
            pane.diff_search_input
                .read(app)
                .focus_handle()
                .is_focused(window),
            "and put the caret in the search box, not the buffer"
        );
    });

    cx.simulate_keystrokes("n e e d l e");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });
    draw_and_drain_test_window(cx);

    let matches = editor_search_matches(&view, cx);
    assert_eq!(matches.len(), 2, "the buffer holds two needles");
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .main_pane
                .read(app)
                .file_editor_input
                .read(app)
                .text(),
            contents,
            "the query must land in the search box, never in the file"
        );
    });

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(!pane.diff_search_active, "escape closes the search");
        assert!(
            pane.file_editor_search_matches.is_empty(),
            "and takes the wash with it"
        );
        assert!(
            pane.is_file_editor_active(),
            "escape from the search must not also exit edit mode"
        );
        assert!(
            pane.file_editor_input
                .read(app)
                .focus_handle()
                .is_focused(window),
            "the buffer gets focus back, so the next keystroke edits the file"
        );
    });
    assert_eq!(
        editor_selected_range(&view, cx),
        matches[0],
        "the caret is left on the match the search was showing"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn recent_repository_shortcut_does_not_select_the_file_editor(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let contents = "alpha beta gamma\n";
    let workdir = seed_editor(&view, cx, 987, "file_editor_recent_repo_shortcut", contents);
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::install_app_shortcuts_for_test(app, Arc::new(TestBackend));
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.set_caret(6, cx);
                    let focus = input.focus_handle();
                    window.focus(&focus, cx);
                });
            });
        });
        let _ = window.draw(app);
        window.activate();
    });
    assert_eq!(editor_selected_range(&view, cx), 6..6);

    cx.simulate_keystrokes("secondary-shift-a");
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("app_popover").is_some(),
        "Ctrl/Cmd+Shift+A must open the recent-repositories picker from the editor"
    );
    assert_eq!(
        editor_selected_range(&view, cx),
        6..6,
        "the recent-repositories shortcut must not also select the editor buffer"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The occurrence set must depend on where the caret *is*, not on how it got
/// there.
///
/// Two name tokens can abut, and the guard that decided whether the cached set
/// still applied was an inclusive range test -- so the offset where one name
/// ends and the next begins satisfied both. Walking right out of `$foo` into
/// `$bar` kept `$foo` lit for one keypress, while arriving at the same offset
/// from the right lit `$bar`. Perl is the in-tree case: `scalar_variable` is a
/// sigil plus a name with no separator between two of them.
#[gpui::test]
async fn file_editor_occurrences_do_not_depend_on_the_direction_of_approach(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(966);
    let workdir = unique_workdir("file_editor_abutting_names");
    let file_rel = std::path::PathBuf::from("abutting.pl");
    // `$foo` is 6..10 and `$bar` is 10..14, so offset 10 is the boundary. Each
    // name occurs again below, so the two sets are told apart by more than the
    // clicked token alone.
    let contents = "print $foo$bar;\nprint $foo;\nprint $bar;\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write abutting-name fixture");

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
        assert!(
            main_pane.read(app).file_editor_live_syntax.is_some(),
            "the Perl fixture should have a live syntax tree"
        );
    });

    let set_cursor = |cx: &mut gpui::VisualTestContext, offset: usize| {
        cx.update(|_window, app| {
            main_pane.update(app, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.set_cursor_offset(offset, cx);
                });
            });
        });
        cx.run_until_parked();
        cx.update(|_window, app| main_pane.read(app).file_editor_occurrences.clone())
    };

    let foo: Vec<std::ops::Range<usize>> = vec![6..10, 22..26];
    let bar: Vec<std::ops::Range<usize>> = vec![10..14, 34..38];

    assert_eq!(
        set_cursor(cx, 8),
        foo,
        "the caret inside `$foo` lights `$foo`"
    );
    // Walk right, one offset at a time, across the boundary.
    let _ = set_cursor(cx, 9);
    assert_eq!(
        set_cursor(cx, 10),
        bar,
        "offset 10 is the first byte of `$bar`, whichever side the caret came from"
    );

    assert_eq!(
        set_cursor(cx, 12),
        bar,
        "the caret inside `$bar` lights `$bar`"
    );
    let _ = set_cursor(cx, 11);
    assert_eq!(
        set_cursor(cx, 10),
        bar,
        "and arriving from the right agrees with arriving from the left"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn file_editor_saves_stay_paused_until_rename_retargets_the_unsaved_buffer(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::filesystem::{DocumentIdentity, Filesystem, Operation, Request};
    let _guard = lock_visual_test();
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
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();
    let request = Request::new(Operation::Rename {
        source: original.clone(),
        name: "after.rs".into(),
    });
    cx.update(|_, app| {
        let pane = view.read(app).main_pane.clone();
        pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// retained edit\n", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            assert!(pane.file_editor_dirty);
            pane.filesystem_pause(request.id, cx);
            pane.save_file_editor_buffer(cx);
            assert!(pane.file_editor_saves.is_empty());
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
            this.main_pane.update(cx, |pane, cx| {
                pane.filesystem_finish(result.id, &result.changes, &result.moved_versions, cx);
                assert_eq!(
                    pane.file_editor_key,
                    Some(DocumentIdentity(renamed.clone()))
                );
                assert!(pane.file_editor_dirty);
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
            .main_pane
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

/// Closing a repository's tab must not drop its editor's unsaved edits: the
/// tab-close path should ask like closing the window does. It does not yet
/// (`prune_orphaned_file_editor_stash` documents the gap); the fix needs the
/// unsaved-edits action scoped to several repositories.
#[gpui::test]
#[ignore = "known gap: closing a repository tab discards its unsaved edits"]
async fn closing_a_repository_tab_prompts_for_its_unsaved_edits(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(963);
    let workdir = unique_workdir("file_editor_close_dirty_repo");
    let file = std::path::PathBuf::from("a.rs");
    std::fs::write(workdir.join(&file), "fn a() {}\n").expect("write fixture");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file), cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        });
    });
    cx.run_until_parked();
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// unsaved\n", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.request_close_repos(vec![repo_id], None, cx);
            let prompt = this
                .pending_unsaved_file_edits_prompt
                .as_ref()
                .expect("closing a dirty repository's tab must queue the unsaved-edits prompt");
            assert_eq!(prompt.files.len(), 1);
        });
    });
    let _ = std::fs::remove_dir_all(&workdir);
}
