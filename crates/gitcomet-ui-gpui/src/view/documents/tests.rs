use super::*;
use crate::view::test_support::TestBackend;

// Standalone fixtures must stay outside repository routing even when an
// ancestor of the temporary directory contains a .git entry.
struct StandaloneBackend;

impl gitcomet_core::services::GitBackend for StandaloneBackend {
    fn open(
        &self,
        _workdir: &Path,
    ) -> gitcomet_core::services::Result<Arc<dyn gitcomet_core::services::GitRepository>> {
        Err(gitcomet_core::error::Error::new(
            gitcomet_core::error::ErrorKind::NotARepository,
        ))
    }
}

#[gpui::test]
fn background_document_opens_defer_loading_and_clean_navigation_releases_buffers(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..session::MAX_RECENT_DOCUMENTS + 5)
        .map(|i| directory.path().join(format!("{i}.txt")))
        .collect();
    for path in &paths {
        std::fs::write(path, "document contents").unwrap();
    }
    let docs = cx.update(|_, app| root.read(app).documents.clone());
    cx.update(|_, app| {
        docs.update(app, |docs, cx| {
            for path in &paths {
                docs.open(path.clone(), false, cx);
            }
            assert!(
                docs.buffers.is_empty(),
                "undisplayed documents must not allocate or load buffers"
            );
            assert!(docs.active.is_none());
            assert_eq!(
                docs.recents.read(cx).paths.len(),
                session::MAX_RECENT_DOCUMENTS
            );
        })
    });
    cx.run_until_parked();
    let mut previous: Option<WeakEntity<StandaloneBuffer>> = None;
    for (i, path) in paths.iter().take(5).enumerate() {
        cx.update(|_, app| docs.update(app, |docs, cx| docs.open(path.clone(), true, cx)));
        if i % 2 == 0 {
            // Also navigate away while an initial read is still pending.
            drain(&root, cx);
        }
        cx.update(|_, app| {
            assert_eq!(docs.read(app).buffers.len(), 1);
            if let Some(previous) = &previous {
                assert!(
                    previous.upgrade().is_none(),
                    "eviction must release the entity and its observers"
                );
            }
            let docs = docs.read(app);
            previous = Some(docs.buffers[&docs.active.unwrap()].downgrade());
        });
    }
    drain(&root, cx);
    cx.update(|_, app| {
        let docs = docs.read(app);
        assert_eq!(
            docs.buffers[&docs.active.unwrap()]
                .read(app)
                .input
                .read(app)
                .text(),
            "document contents"
        );
    });
}

#[gpui::test]
fn document_eviction_preserves_dirty_and_saving_buffers_until_acknowledged_clean(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let [first, second, third] =
        ["first.txt", "second.txt", "third.txt"].map(|p| directory.path().join(p));
    let copy = directory.path().join("copy.txt");
    for path in [&first, &second, &third] {
        std::fs::write(path, "original").unwrap();
    }
    let docs = cx.update(|_, app| root.read(app).documents.clone());
    cx.update(|_, app| docs.update(app, |docs, cx| docs.open(first.clone(), true, cx)));
    drain(&root, cx);
    let dirty = cx.update(|_, app| {
        let view = docs.read(app);
        view.buffers[&view.active.unwrap()].clone()
    });
    cx.update(|_, app| {
        dirty.update(app, |buffer, cx| {
            buffer.editing = true;
            buffer.input.update(cx, |input, cx| {
                input.set_read_only(false, cx);
                input.set_text("unsaved edits", cx);
            });
        });
        // Navigate before the input observer has updated the dirty flag.
        docs.update(app, |docs, cx| docs.open(second.clone(), true, cx));
    });
    drain(&root, cx);
    let (saving, saving_id) = cx.update(|_, app| {
        docs.update(app, |docs, cx| {
            update_recent(first.clone(), true, cx);
            for i in 0..session::MAX_RECENT_DOCUMENTS + 1 {
                docs.open(
                    directory.path().join(format!("background-{i}.txt")),
                    false,
                    cx,
                );
            }
            assert!(docs.buffers.contains_key(&dirty.entity_id()));
            assert!(dirty.read(cx).dirty);
            let id = docs.active.unwrap();
            let buffer = docs.buffers[&id].clone();
            buffer.update(cx, |buffer, cx| buffer.save(Some(copy.clone()), false, cx));
            assert!(!buffer.read(cx).dirty && buffer.read(cx).saving.is_some());
            docs.open(third.clone(), true, cx);
            assert!(
                docs.buffers.contains_key(&id),
                "a clean Save As still needs its pending buffer"
            );
            (buffer.downgrade(), id)
        })
    });
    drain(&root, cx);
    assert_eq!(std::fs::read_to_string(copy).unwrap(), "original");
    cx.update(|_, app| {
        docs.update(app, |docs, cx| {
            assert!(!docs.buffers.contains_key(&saving_id));
            assert!(saving.upgrade().is_none());
            let active = docs.active;
            // A picker row rendered before the save acknowledgment may still be clicked.
            docs.activate_buffer(saving_id, cx);
            assert_eq!(docs.active, active);
            assert_eq!(docs.buffers.len(), 2);
        })
    });
    cx.update(|_, app| {
        dirty.update(app, |buffer, cx| {
            buffer.save(None, false, cx);
            buffer
                .input
                .update(cx, |input, cx| input.set_text("newer edits", cx));
        })
    });
    drain(&root, cx);
    cx.update(|_, app| {
        assert!(dirty.read(app).dirty);
        assert!(docs.read(app).buffers.contains_key(&dirty.entity_id()));
        dirty.update(app, |buffer, cx| buffer.save(None, false, cx));
    });
    drain(&root, cx);
    assert_eq!(std::fs::read_to_string(first).unwrap(), "newer edits");
    cx.update(|_, app| assert_eq!(docs.read(app).buffers.len(), 1));
}

#[gpui::test]
fn loaded_documents_keep_their_line_endings_when_enter_is_pressed(cx: &mut gpui::TestAppContext) {
    check_document_line_endings(cx, false);
}

#[gpui::test]
fn adopted_documents_keep_their_line_endings_when_enter_is_pressed(cx: &mut gpui::TestAppContext) {
    check_document_line_endings(cx, true);
}

fn check_document_line_endings(cx: &mut gpui::TestAppContext, adopted: bool) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) = cx.add_window_view(|window, cx| {
        // Text input only accepts typing in the active window.
        window.activate();
        GitCometView::new(store, events, None, window, cx)
    });
    let directory = tempfile::tempdir().unwrap();
    cx.update(|_, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        root.update(app, |root, cx| {
            root.documents_active = true;
            cx.notify();
        });
    });
    for (name, ending) in [("crlf.txt", "\r\n"), ("lf.txt", "\n")] {
        let path = directory.path().join(name);
        let text = format!("alpha{ending}beta");
        std::fs::write(&path, &text).unwrap();
        let buffer = cx.update(|_, app| {
            root.read(app).documents.clone().update(app, |docs, cx| {
                if adopted {
                    docs.adopt(
                        DocumentIdentity(path.clone()),
                        StashedFileEdit {
                            text: text.clone().into(),
                            cursor: text.len(),
                            text_fingerprint: 1,
                            saved_fingerprint: 2,
                            first_dirty_line: None,
                            disk: Default::default(),
                            text_format: None,
                            source_text_format: None,
                        },
                        Some(DiskVersion::read(&path).unwrap()),
                        cx,
                    );
                    let id = docs
                        .buffers
                        .iter()
                        .find(|(_, buffer)| buffer.read(cx).identity.0 == path)
                        .unwrap()
                        .0;
                    docs.activate_buffer(*id, cx);
                } else {
                    docs.open(path.clone(), true, cx);
                }
                docs.buffers[&docs.active.unwrap()].clone()
            })
        });
        drain(&root, cx);
        cx.update(|window, app| {
            buffer.update(app, |buffer, cx| {
                buffer.editing = true;
                buffer.input.update(cx, |input, cx| {
                    input.set_read_only(false, cx);
                    input.set_cursor_offset(text.len(), cx);
                });
                window.focus(&buffer.input.read(cx).focus_handle(), cx);
            });
            let _ = window.draw(app);
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let expected = format!("{text}{ending}");
        cx.update(|_, app| {
            buffer.update(app, |buffer, cx| {
                assert_eq!(buffer.input.read(cx).text(), expected);
                assert!(buffer.dirty);
                buffer.save(None, false, cx);
            })
        });
        drain(&root, cx);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    }
}

#[gpui::test]
fn pending_document_reads_restart_after_file_renames(cx: &mut gpui::TestAppContext) {
    rename_during_document_load(cx, false, false);
}

#[gpui::test]
fn pending_document_reads_restart_after_parent_directory_renames(cx: &mut gpui::TestAppContext) {
    rename_during_document_load(cx, true, false);
}

#[gpui::test]
fn pending_document_reads_wait_for_all_filesystem_pauses_before_restarting(
    cx: &mut gpui::TestAppContext,
) {
    rename_during_document_load(cx, true, true);
}

fn rename_during_document_load(cx: &mut gpui::TestAppContext, parent: bool, overlapping: bool) {
    let _guard = crate::test_support::lock_visual_test();
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let original = workdir.join("before/file.txt");
    let renamed = workdir.join(if parent {
        "after/file.txt"
    } else {
        "before/renamed.txt"
    });
    std::fs::create_dir(original.parent().unwrap()).unwrap();
    std::fs::write(&original, "document contents").unwrap();
    let docs = cx.update(|_, app| root.read(app).documents.clone());
    let other_pause = OperationId::allocate();
    let buffer = cx.update(|_, app| {
        docs.update(app, |docs, cx| {
            docs.open(original.clone(), true, cx);
            let buffer = docs.buffers[&docs.active.unwrap()].clone();
            assert!(buffer.read(cx).loading);
            let request = Request::new(Operation::Rename {
                source: if parent {
                    original.parent().unwrap().into()
                } else {
                    original.clone()
                },
                name: if parent { "after" } else { "renamed.txt" }.into(),
            });
            docs.filesystem_pause(request.id, cx);
            if overlapping {
                docs.filesystem_pause(other_pause, cx);
            }
            let result = gitcomet_core::filesystem::Filesystem::default().execute(request, |_| {});
            assert!(result.succeeded(), "{:?}", result.items);
            // A successful read of the obsolete path must also be rejected.
            std::fs::create_dir_all(original.parent().unwrap()).unwrap();
            std::fs::write(&original, "obsolete contents").unwrap();
            docs.filesystem_finish(result.id, &result.changes, &result.moved_versions, cx);
            assert_eq!(buffer.read(cx).identity.0, renamed);
            buffer
        })
    });
    cx.run_until_parked();
    if overlapping {
        cx.update(|_, app| {
            assert!(buffer.read(app).loading);
            assert!(buffer.read(app).input.read(app).text().is_empty());
            docs.update(app, |docs, cx| {
                docs.filesystem_finish(other_pause, &[], &BTreeMap::new(), cx);
            });
        });
    }
    drain(&root, cx);
    cx.update(|_, app| {
        buffer.update(app, |b, cx| {
            assert!(!b.loading && !b.dirty && b.error.is_none());
            assert_eq!(b.input.read(cx).text(), "document contents");
            b.editing = true;
            b.input.update(cx, |input, cx| {
                input.set_read_only(false, cx);
                input.set_text("edited document", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| buffer.update(app, |b, cx| b.save(None, false, cx)));
    drain(&root, cx);
    assert_eq!(std::fs::read_to_string(renamed).unwrap(), "edited document");
    assert_eq!(
        std::fs::read_to_string(original).unwrap(),
        "obsolete contents"
    );
}

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
fn cancelling_before_a_conflict_is_delivered_releases_the_receipt_without_prompting(
    cx: &mut gpui::TestAppContext,
) {
    transfer_without_rendering(cx, TransferOutcome::CancelledAfterConflict);
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
    CancelledAfterConflict,
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
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let source = workdir.join("source.txt");
    let other_file = workdir.join("other.txt");
    let destination = workdir.join("destination");
    std::fs::write(&source, "source contents").unwrap();
    std::fs::write(&other_file, "other contents").unwrap();
    std::fs::create_dir(&destination).unwrap();
    if matches!(
        outcome,
        TransferOutcome::Conflict | TransferOutcome::CancelledAfterConflict
    ) {
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
    if outcome == TransferOutcome::CancelledAfterConflict {
        // Hold the completed conflict on the store side, then cancel before
        // its notification reaches the UI. No render or prompt is involved.
        let mut reached_conflict = false;
        for _ in 0..400 {
            reached_conflict = cx.update(|_, app| {
                root.read(app)
                    .store
                    .snapshot()
                    .filesystem
                    .completed
                    .iter()
                    .any(|result| {
                        result.id == id
                            && result.items.iter().any(|item| {
                                matches!(
                                    item.outcome,
                                    gitcomet_core::filesystem::ItemOutcome::Conflict(_)
                                )
                            })
                    })
            });
            if reached_conflict {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(reached_conflict);
        cx.update(|_, app| root.update(app, |root, cx| root.cancel_filesystem_operations(cx)));
    }
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
            cx.update(
                |_, app| crate::view::test_support::popover_kind(root.read(app), app).is_none()
            ),
            "only rendering should display the conflict dialog"
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
fn replace_asks_about_unsaved_edits_in_a_themed_dialog(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::filesystem::TransferIntent;
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::install_app_shortcuts_for_test(app, Arc::new(TestBackend));
        let _ = window.draw(app);
        window.activate();
    });
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let source = workdir.join("a.txt");
    let destination = workdir.join("dest");
    let target = destination.join("a.txt");
    std::fs::write(&source, "new contents").unwrap();
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(&target, "old contents").unwrap();
    cx.update(|_, app| {
        root.read(app)
            .documents
            .clone()
            .update(app, |docs, cx| docs.open(target.clone(), true, cx))
    });
    drain(&root, cx);
    let buffer = cx.update(|_, app| {
        let docs = root.read(app).documents.read(app);
        docs.buffers[&docs.active.unwrap()].clone()
    });
    cx.update(|_, app| {
        buffer.update(app, |buffer, cx| {
            buffer.editing = true;
            buffer.input.update(cx, |input, cx| {
                input.set_read_only(false, cx);
                input.set_text("unsaved edits", cx);
            });
        })
    });
    cx.run_until_parked();

    // Feeds store results to the view and renders until `done` holds.
    let pump_until = |cx: &mut gpui::VisualTestContext,
                      done: &dyn Fn(&GitCometView, &gpui::App) -> bool| {
        for _ in 0..400 {
            cx.update(|_, app| {
                root.update(app, |root, cx| {
                    crate::view::test_support::sync_store_snapshot(root, cx)
                })
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
            if cx.update(|_, app| done(root.read(app), app)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    };
    let popover =
        |root: &GitCometView, app: &gpui::App| crate::view::test_support::popover_kind(root, app);
    let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        let center = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} must be drawn"))
            .center();
        cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.run_until_parked();
    };

    let request = Request::new(Operation::Transfer {
        sources: vec![source.clone()],
        destination: destination.clone(),
        intent: TransferIntent::Copy,
    });
    cx.update(|window, app| {
        root.update(app, |root, cx| {
            root.submit_filesystem_operation(request, None, window, cx)
        })
    });
    assert!(pump_until(cx, &|root, app| matches!(
        popover(root, app),
        Some(PopoverKind::FilesystemConflict(_))
    )));
    click(cx, "filesystem_conflict_replace");
    assert!(
        pump_until(cx, &|root, app| matches!(
            popover(root, app),
            Some(PopoverKind::FilesystemUnsavedEditsConfirm(prompt))
                if prompt.files == vec![SharedString::from("a.txt")]
        )),
        "replacing a dirty buffer asks first"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "old contents");

    click(cx, "filesystem_unsaved_edits_discard");
    assert!(
        pump_until(cx, &|root, app| popover(root, app).is_none()
            && !root.file_operations.has_pending()
            && std::fs::read_to_string(&target)
                .is_ok_and(|text| text == "new contents")),
        "discarding lets the replace run"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new contents");
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "new contents");
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
    let repository =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let first = repository.join("first.txt");
    let second = repository.join("second.txt");
    for path in [&first, &second] {
        std::fs::write(path, "text").unwrap();
    }
    // Filesystem identities must route into the backend's workdir spelling,
    // even when canonicalize supplies a verbatim-prefixed Windows path.
    let resolved_first =
        DocumentIdentity::resolve(&std::fs::canonicalize(&first).unwrap()).unwrap();
    let resolved_second =
        DocumentIdentity::resolve(&std::fs::canonicalize(&second).unwrap()).unwrap();
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
            root.queue_repository_document(repository.clone(), resolved_first.0, true, cx);
            root.queue_repository_document(repository.clone(), resolved_second.0, false, cx);
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
                        text_format: None,
                        source_text_format: None,
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
                    text_format: None,
                    source_text_format: None,
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
                    text_format: None,
                    source_text_format: None,
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

fn click_status_bar_documents_button(cx: &mut gpui::VisualTestContext) {
    crate::view::test_support::redraw(cx);
    let button = cx
        .debug_bounds("bottom_documents")
        .expect("the status bar shows the Documents button")
        .center();
    cx.simulate_mouse_move(button, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(button, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(button, gpui::MouseButton::Left, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
}

fn document_picker_open(root: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> bool {
    cx.update(|_, app| root.read(app).document_picker_open(app))
}

#[gpui::test]
fn status_bar_documents_button_opens_a_picker_that_opens_typed_paths(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(StandaloneBackend));
    let (root, cx) = cx.add_window_view(|window, cx| {
        // Text input only accepts typing in the active window.
        window.activate();
        GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_, app| crate::app::bind_text_input_keys_for_test(app));
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    // Exercise repository probing without relying on the host's temp layout.
    std::fs::create_dir(workdir.join(".git")).unwrap();
    let path = workdir.join("notes.md");
    std::fs::write(&path, "# Notes").unwrap();

    // The button once updated the bar from inside the bar's own update.
    click_status_bar_documents_button(cx);
    assert!(document_picker_open(&root, cx));
    assert!(cx.debug_bounds("documents_picker").is_some());
    assert!(
        !cx.update(|_, app| root.read(app).documents_active),
        "opening the picker must not replace the canvas"
    );

    cx.simulate_input(&path.display().to_string());
    cx.simulate_keystrokes("enter");
    for _ in 0..400 {
        cx.run_until_parked();
        if cx.update(|_, app| {
            let root = root.read(app);
            root.documents_active && root.documents.read(app).active.is_some()
        }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    drain(&root, cx);
    assert!(!document_picker_open(&root, cx));
    cx.update(|_, app| {
        let docs = root.read(app).documents.read(app);
        let buffer = docs.buffers[&docs.active.expect("Enter opens the typed path")].read(app);
        assert_eq!(buffer.identity.0, path);
        assert_eq!(buffer.input.read(app).text(), "# Notes");
        assert!(buffer.wrap, "prose wraps by default");
    });
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("document_header").is_some());
    assert!(cx.debug_bounds("document_gutter").is_some());

    click_status_bar_documents_button(cx);
    assert!(document_picker_open(&root, cx));
    cx.simulate_keystrokes("escape");
    assert!(!document_picker_open(&root, cx));
    assert!(cx.update(|_, app| root.read(app).documents_active));
}

#[gpui::test]
fn repository_windows_keep_their_panes_around_documents_until_the_repository_navigates(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) = cx.add_window_view({
        let store = store.clone();
        |window, cx| GitCometView::new(store, events, None, window, cx)
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("standalone.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let repo_id = RepoId(1);
    let mut repo = gitcomet_state::model::RepoState::new_opening(
        repo_id,
        gitcomet_core::domain::RepoSpec {
            workdir: directory.path().join("repo"),
        },
    );
    repo.open = Loadable::Ready(());
    let mut state = gitcomet_state::model::AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..gitcomet_state::model::AppState::test_default()
    };
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    cx.update(|_, app| {
        root.update(app, |root, cx| {
            root.documents
                .update(cx, |docs, cx| docs.open(path.clone(), true, cx));
            root.show_documents_canvas(cx);
        })
    });
    drain(&root, cx);
    crate::view::test_support::redraw(cx);
    let canvas = cx
        .debug_bounds("documents_canvas")
        .expect("the viewer is on screen");
    let sidebar = cx
        .debug_bounds("sidebar_pane")
        .expect("the sidebar stays beside the viewer");
    let details = cx
        .debug_bounds("details_pane")
        .expect("the details pane stays beside the viewer");
    assert!(canvas.left() >= sidebar.right() && canvas.right() <= details.left());
    assert!(!cx.update(|_, app| root.read(app).documents.read(app).buffers.is_empty()));

    let push = |state: &gitcomet_state::model::AppState, cx: &mut gpui::VisualTestContext| {
        store.replace_snapshot_for_test(Arc::new(state.clone()));
        cx.update(|_, app| {
            root.update(app, |root, cx| {
                crate::view::test_support::sync_store_snapshot(root, cx)
            })
        });
        cx.run_until_parked();
    };
    // Refreshes clear selections; that is not the user going somewhere.
    let repo = &mut state.repos[0];
    repo.history_state.selected_commit = None;
    repo.history_state.selected_commit_rev += 1;
    push(&state, cx);
    assert!(cx.update(|_, app| root.read(app).documents_active));

    let repo = &mut state.repos[0];
    repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::working_tree(
        std::path::PathBuf::from("a.txt"),
        gitcomet_core::domain::DiffArea::Unstaged,
    ));
    repo.diff_state.diff_target_rev += 1;
    push(&state, cx);
    assert!(
        !cx.update(|_, app| root.read(app).documents_active),
        "opening a file in the repository takes the main slot back"
    );
}
