use super::*;
use crate::view::test_support::TestBackend;

#[gpui::test]
fn transfer_completion_releases_all_windows_and_native_receipts_without_rendering(
    cx: &mut gpui::TestAppContext,
) {
    transfer_without_rendering(cx, TransferOutcome::Completed);
}

#[gpui::test]
fn failed_transfer_releases_all_windows_and_native_receipts_without_rendering(
    cx: &mut gpui::TestAppContext,
) {
    transfer_without_rendering(cx, TransferOutcome::Failed);
}

#[gpui::test]
fn cancelled_transfer_releases_its_native_receipt_without_rendering(cx: &mut gpui::TestAppContext) {
    transfer_without_rendering(cx, TransferOutcome::Cancelled);
}

#[gpui::test]
fn transfer_conflicts_release_editors_before_rendering_and_cancel_releases_the_receipt(
    cx: &mut gpui::TestAppContext,
) {
    transfer_without_rendering(cx, TransferOutcome::Conflict);
}

#[derive(Clone, Copy, PartialEq)]
enum TransferOutcome {
    Completed,
    Failed,
    Cancelled,
    Conflict,
}

fn transfer_without_rendering(cx: &mut gpui::TestAppContext, outcome: TransferOutcome) {
    use gitcomet_core::filesystem::TransferIntent;
    let _guard = crate::test_support::lock_visual_test();
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (other, _) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let workdir = std::fs::canonicalize(directory.path()).unwrap();
    let source = workdir.join("source.txt");
    let other_file = workdir.join("other.txt");
    let destination = workdir.join("destination");
    std::fs::write(&source, "source contents").unwrap();
    std::fs::write(&other_file, "other contents").unwrap();
    std::fs::create_dir(&destination).unwrap();
    if outcome == TransferOutcome::Conflict {
        std::fs::write(destination.join("source.txt"), "existing destination").unwrap();
    }
    for (view, path) in [(&root, &source), (&other, &other_file)] {
        cx.update(|_, app| {
            view.read(app)
                .documents
                .clone()
                .update(app, |docs, cx| docs.open(path.clone(), true, cx))
        });
        drain(view, cx);
    }
    let other_buffer = cx.update(|_, app| {
        let docs = other.read(app).documents.read(app);
        docs.buffers[&docs.active.unwrap()].clone()
    });
    cx.update(|_, app| {
        other_buffer.update(app, |buffer, cx| {
            buffer.editing = true;
            buffer.input.update(cx, |input, cx| {
                input.set_read_only(false, cx);
                input.set_text("unsaved edits in another window", cx);
            });
        })
    });
    cx.run_until_parked();
    let request = Request::new(Operation::Transfer {
        sources: vec![source.clone()],
        destination: if outcome == TransferOutcome::Failed {
            destination.join("missing/parent")
        } else {
            destination.clone()
        },
        intent: TransferIntent::Move,
    });
    let id = request.id;
    if outcome == TransferOutcome::Cancelled {
        request.cancellation.cancel();
    }
    let completions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let completed = completions.clone();
    cx.update(|window, app| {
        root.update(app, |root, cx| {
            root.submit_filesystem_drop(
                request,
                gpui::FileDropTransfer {
                    operation: gpui::FileTransferOperation::Move,
                    source_owns_move: false,
                    completion: gpui::FilePaste::new(move |operation| {
                        completed.lock().unwrap().push(operation)
                    }),
                },
                TransferIntent::Move,
                window,
                cx,
            );
            if outcome != TransferOutcome::Cancelled {
                for view in [&root.main_pane, &other.read(cx).main_pane] {
                    assert!(view.read(cx).filesystem_pauses.contains(&id));
                }
                other_buffer.update(cx, |buffer, cx| {
                    assert!(buffer.dirty && buffer.pauses.contains(&id));
                    buffer.save(None, false, cx);
                    assert!(buffer.saving.is_none(), "saves must wait for the transfer");
                });
            }
        })
    });
    // Deliver only model notifications. An occluded Wayland window may never
    // get a compositor frame, so neither draw nor render may finish the move.
    for _ in 0..400 {
        cx.run_until_parked();
        let released = cx.update(|_, app| {
            root.update(app, |root, cx| {
                crate::view::test_support::sync_store_snapshot(root, cx)
            });
            root.read(app)
                .main_pane
                .read(app)
                .filesystem_pauses
                .is_empty()
        });
        if released {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    cx.update(|_, app| {
        for view in [&root, &other] {
            let view = view.read(app);
            assert!(view.main_pane.read(app).filesystem_pauses.is_empty());
            let docs = view.documents.read(app);
            assert!(
                docs.buffers
                    .values()
                    .all(|buffer| buffer.read(app).pauses.is_empty())
            );
        }
        let docs = root.read(app).documents.read(app);
        let buffer = docs.buffers[&docs.active.unwrap()].read(app);
        assert_eq!(
            buffer.identity.0,
            if outcome == TransferOutcome::Completed {
                destination.join("source.txt")
            } else {
                source.clone()
            }
        );
    });
    if outcome == TransferOutcome::Conflict {
        assert!(
            completions.lock().unwrap().is_empty(),
            "the native receipt must wait for a conflict decision"
        );
        assert!(
            !cx.has_pending_prompt(),
            "only rendering should display the conflict prompt"
        );
        cx.update(|_, app| root.update(app, |root, cx| root.cancel_filesystem_operations(cx)));
        assert_eq!(
            std::fs::read_to_string(destination.join("source.txt")).unwrap(),
            "existing destination"
        );
    }
    cx.update(|_, app| assert!(!root.read(app).file_operations.has_pending()));
    assert_eq!(
        *completions.lock().unwrap(),
        vec![(outcome == TransferOutcome::Completed).then_some(gpui::FileTransferOperation::Move)]
    );
    assert_eq!(source.exists(), outcome != TransferOutcome::Completed);
    cx.update(|_, app| {
        other_buffer.update(app, |buffer, cx| {
            buffer.save(None, false, cx);
            assert!(
                buffer.saving.is_some(),
                "the other window must be able to save without rendering the origin"
            );
        })
    });
    drain(&other, cx);
    assert_eq!(
        std::fs::read_to_string(other_file).unwrap(),
        "unsaved edits in another window"
    );
}

#[gpui::test]
fn reopening_documents_refreshes_clean_buffers_and_retries_failed_reads(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    let docs = cx.update(|_, app| root.read(app).documents.clone());
    cx.update(|_, app| docs.update(app, |docs, cx| docs.open(path.clone(), true, cx)));
    drain(&root, cx);
    let buffer = cx.update(|_, app| {
        let docs = docs.read(app);
        let buffer = docs.buffers[&docs.active.unwrap()].clone();
        assert!(buffer.read(app).error.is_some());
        buffer
    });
    for contents in ["file is now available", "changed by another application"] {
        std::fs::write(&path, contents).unwrap();
        cx.update(|_, app| docs.update(app, |docs, cx| docs.open(path.clone(), true, cx)));
        drain(&root, cx);
        cx.update(|_, app| {
            assert_eq!(docs.read(app).buffers.len(), 1);
            assert_eq!(docs.read(app).active, Some(buffer.entity_id()));
            assert_eq!(buffer.read(app).input.read(app).text(), contents);
            assert!(buffer.read(app).error.is_none());
            assert!(!buffer.read(app).dirty);
        });
    }
    cx.update(|_, app| {
        buffer.update(app, |b, cx| {
            b.editing = true;
            b.input.update(cx, |input, cx| {
                input.set_read_only(false, cx);
                input.set_text("unsaved edits", cx);
            });
        })
    });
    cx.run_until_parked();
    std::fs::write(&path, "another external edit").unwrap();
    cx.update(|_, app| docs.update(app, |docs, cx| docs.open(path.clone(), true, cx)));
    drain(&root, cx);
    cx.update(|_, app| {
        assert_eq!(buffer.read(app).input.read(app).text(), "unsaved edits");
        assert!(buffer.read(app).dirty);
    });
}

#[gpui::test]
fn reopening_a_clean_document_during_save_as_preserves_the_pending_buffer(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    let destination = directory.path().join("copy.txt");
    std::fs::write(&source, "original").unwrap();
    let docs = cx.update(|_, app| root.read(app).documents.clone());
    cx.update(|_, app| docs.update(app, |docs, cx| docs.open(source.clone(), true, cx)));
    drain(&root, cx);
    cx.update(|_, app| {
        docs.update(app, |docs, cx| {
            let buffer = docs.buffers[&docs.active.unwrap()].clone();
            let generation = buffer.read(cx).load_generation;
            buffer.update(cx, |b, cx| b.save(Some(destination.clone()), false, cx));
            assert!(buffer.read(cx).saving.is_some());
            assert!(!buffer.read(cx).dirty);
            docs.open(source.clone(), true, cx);
            assert_eq!(buffer.read(cx).load_generation, generation);
            assert!(!buffer.read(cx).loading);
        })
    });
    drain(&root, cx);
    assert_eq!(std::fs::read_to_string(destination).unwrap(), "original");
}

#[gpui::test]
fn clean_save_as_offers_replacement_and_preserves_source(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    let destination = directory.path().join("existing.txt");
    std::fs::write(&source, "source contents").unwrap();
    std::fs::write(&destination, "destination contents").unwrap();
    cx.update(|_, app| {
        root.update(app, |root, cx| {
            root.documents_active = true;
            root.documents
                .update(cx, |docs, cx| docs.open(source.clone(), true, cx));
            cx.notify();
        })
    });
    drain(&root, cx);
    let buffer = cx.update(|_, app| {
        let docs = root.read(app).documents.read(app);
        docs.buffers[&docs.active.unwrap()].clone()
    });
    cx.update(|_, app| buffer.update(app, |b, cx| b.save(Some(destination.clone()), false, cx)));
    drain(&root, cx);
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "destination contents"
    );
    cx.update(|_, app| {
        let b = buffer.read(app);
        assert!(!b.dirty);
        assert!(b.error.is_some());
        assert_eq!(b.failed_destination.as_ref(), Some(&destination));
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let replace = cx
        .debug_bounds("document_replace_disk")
        .expect("a clean Save As failure must offer replacement")
        .center();
    cx.simulate_mouse_down(replace, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(replace, gpui::MouseButton::Left, gpui::Modifiers::default());
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Replace");
    drain(&root, cx);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "source contents");
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "source contents"
    );
    cx.update(|_, app| {
        let b = buffer.read(app);
        assert_eq!(b.identity.0, destination);
        assert!(!b.dirty);
        assert!(b.error.is_none() && b.failed_destination.is_none());
    });
}

#[gpui::test]
fn repository_document_routes_record_absolute_paths_for_foreground_and_background_opens(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().to_path_buf();
    let first = repository.join("first.txt");
    let second = repository.join("second.txt");
    for path in [&first, &second] {
        std::fs::write(path, "text").unwrap();
    }
    cx.update(|_, app| {
        root.update(app, |root, cx| {
            let mut state = (*root.state).clone();
            let repo = gitcomet_state::model::RepoState::new_opening(
                RepoId(1901),
                gitcomet_core::domain::RepoSpec {
                    workdir: repository.clone(),
                },
            );
            state.repos.push(repo);
            root.state = Arc::new(state);
            root.queue_repository_document(repository.clone(), first.clone(), true, cx);
            root.queue_repository_document(repository.clone(), second.clone(), false, cx);
            assert!(!shared_recents(cx).read(cx).paths.contains(&first));
            let mut state = (*root.state).clone();
            state.repos.last_mut().unwrap().open = Loadable::Ready(());
            root.state = Arc::new(state);
            root.finish_document_routing(cx);
            let recents = shared_recents(cx);
            assert!(recents.read(cx).paths.contains(&first));
            assert!(recents.read(cx).paths.contains(&second));
            assert!(root.document_routing.pending.is_empty());
            update_recent(first.clone(), true, cx);
            root.queue_repository_document(repository.clone(), first.clone(), false, cx);
            assert_eq!(recents.read(cx).paths.first(), Some(&first));
        })
    });
}

#[gpui::test]
fn oversized_documents_show_the_size_error_without_loading_a_baseline(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    for name in ["large.txt", "large.bin", "large.png"] {
        let path = directory.path().join(name);
        std::fs::File::create(&path)
            .unwrap()
            .set_len(32 * 1024 * 1024 + 1)
            .unwrap();
        cx.update(|_, app| {
            root.read(app)
                .documents
                .clone()
                .update(app, |docs, cx| docs.open(path, true, cx));
        });
        drain(&root, cx);
        cx.update(|_, app| {
            let docs = root.read(app).documents.read(app);
            let b = docs.buffers[&docs.active.unwrap()].read(app);
            assert!(b.error.as_ref().is_some_and(|e| e.contains("32 MB")));
            assert!(b.version.is_none());
            assert!(!b.dirty);
        });
    }
}

fn drain(root: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) {
    for _ in 0..400 {
        cx.run_until_parked();
        let done = cx.update(|_, app| {
            root.update(app, |root, cx| {
                crate::view::test_support::sync_store_snapshot(root, cx)
            });
            let docs = root.read(app).documents.read(app);
            docs.buffers
                .values()
                .all(|b| !b.read(app).loading && b.read(app).saving.is_none())
        });
        if done {
            cx.run_until_parked();
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("document worker did not finish");
}

#[gpui::test]
fn standalone_edits_detect_other_writes_and_save_as_preserves_both_files(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("standalone.txt");
    std::fs::write(&path, "original").unwrap();
    cx.update(|_, app| {
        let docs = root.read(app).documents.clone();
        docs.update(app, |docs, cx| docs.open(path.clone(), true, cx));
    });
    drain(&root, cx);
    let buffer = cx.update(|_, app| {
        let docs = root.read(app).documents.read(app);
        docs.buffers[&docs.active.unwrap()].clone()
    });
    cx.update(|_, app| {
        buffer.update(app, |b, cx| {
            assert_eq!(b.input.read(cx).text(), "original");
            assert_eq!(b.path_input.read(cx).text(), path.display().to_string());
            assert!(!b.editing);
            b.editing = true;
            b.input.update(cx, |input, cx| {
                input.set_read_only(false, cx);
                input.set_text("my edits", cx);
            });
        })
    });
    cx.run_until_parked();
    std::fs::write(&path, "other window").unwrap();
    cx.update(|_, app| buffer.update(app, |b, cx| b.save(None, false, cx)));
    drain(&root, cx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "other window");
    cx.update(|_, app| {
        let b = buffer.read(app);
        assert!(b.dirty && b.error.is_some());
        assert_eq!(b.input.read(app).text(), "my edits");
    });
    let destination = directory.path().join("saved as.txt");
    cx.update(|_, app| buffer.update(app, |b, cx| b.save(Some(destination.clone()), false, cx)));
    drain(&root, cx);
    assert_eq!(std::fs::read_to_string(&destination).unwrap(), "my edits");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "other window");
    cx.update(|_, app| {
        assert!(!buffer.read(app).dirty);
        assert_eq!(buffer.read(app).identity.0, destination);
    });
}

#[gpui::test]
fn detached_buffers_survive_missing_files_history_removal_and_path_collisions(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing.txt");
    cx.update(|_, app| {
        let docs = root.read(app).documents.clone();
        docs.update(app, |docs, cx| {
            for text in ["first buffer", "second buffer"] {
                docs.adopt(
                    DocumentIdentity(missing.clone()),
                    StashedFileEdit {
                        text: text.into(),
                        cursor: 0,
                        text_fingerprint: 1,
                        saved_fingerprint: 2,
                        first_dirty_line: None,
                        disk: Default::default(),
                    },
                    None,
                    cx,
                );
            }
            update_recent(missing.clone(), true, cx);
            assert_eq!(docs.buffers.len(), 2);
            assert_eq!(docs.unsaved_labels(cx).len(), 2);
            let ids: Vec<_> = docs.buffers.keys().copied().collect();
            docs.active = Some(ids[0]);
            docs.picker = false;
            assert_eq!(
                docs.buffers[&ids[0]].read(cx).input.read(cx).text(),
                "first buffer"
            );
            docs.active = Some(ids[1]);
            assert_eq!(
                docs.buffers[&ids[1]].read(cx).input.read(cx).text(),
                "second buffer"
            );
        });
    });
    assert!(!missing.exists());
}

#[gpui::test]
fn document_history_is_shared_and_opening_another_view_retains_edits(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("shared.txt");
    std::fs::write(&file, "saved").unwrap();
    cx.update(|_, app| {
        let docs = root.read(app).documents.clone();
        let second = app.new(|cx| {
            DocumentsView::new(
                docs.read(cx).theme,
                root.downgrade(),
                docs.read(cx).store.clone(),
                docs.read(cx).ui_model.clone(),
                cx,
            )
        });
        docs.update(app, |docs, cx| {
            update_recent(file.clone(), false, cx);
            docs.adopt(
                DocumentIdentity(file.clone()),
                StashedFileEdit {
                    text: "unsaved".into(),
                    cursor: 0,
                    text_fingerprint: 1,
                    saved_fingerprint: 2,
                    first_dirty_line: None,
                    disk: Default::default(),
                },
                None,
                cx,
            );
        });
        assert_eq!(second.read(app).recents, docs.read(app).recents);
        assert_eq!(
            second.read(app).recents.read(app).paths.first(),
            Some(&file)
        );
        root.update(app, |root, cx| root.show_repository_canvas(cx));
        assert_eq!(docs.read(app).unsaved_labels(app).len(), 1);
        second.update(app, |_, cx| update_recent(file.clone(), true, cx));
        assert!(!docs.read(app).recents.read(app).paths.contains(&file));
        assert_eq!(docs.read(app).unsaved_labels(app).len(), 1);
    });
}

#[gpui::test]
fn unsaved_svg_is_editable_and_discarded_missing_files_stay_clean(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing.svg");
    let buffer = cx.update(|_, app| {
        let docs = root.read(app).documents.clone();
        docs.update(app, |docs, cx| {
            docs.adopt(
                DocumentIdentity(path.clone()),
                StashedFileEdit {
                    text: "<svg/>".into(),
                    cursor: 0,
                    text_fingerprint: 1,
                    saved_fingerprint: 2,
                    first_dirty_line: None,
                    disk: Default::default(),
                },
                None,
                cx,
            );
            docs.buffers.values().next().unwrap().clone()
        })
    });
    cx.update(|_, app| {
        buffer.update(app, |b, cx| {
            assert!(b.dirty && b.editing);
            assert!(!b.image, "an adopted SVG buffer contains editable text");
            b.discard(cx);
        })
    });
    drain(&root, cx);
    cx.update(|_, app| {
        let b = buffer.read(app);
        assert!(!b.dirty && !b.editing);
        assert!(b.error.is_some());
    });
    assert!(!path.exists());
}
