use super::*;
use crate::view::test_support::TestBackend;

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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
