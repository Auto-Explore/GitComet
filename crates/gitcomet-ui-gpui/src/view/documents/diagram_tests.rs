use super::*;

#[gpui::test]
fn standalone_diagram_uses_code_while_editing_and_tracks_buffer_changes(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("diagram.mmd");
    std::fs::write(&path, "flowchart LR\nA-->B").unwrap();
    let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
    let (root, model) = {
        let (root, root_cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store.clone(), events, None, window, cx)
        });
        let model = root_cx.update(|_, app| root.read(app).ui_model.clone());
        (root, model)
    };
    let (buffer, cx) = cx.add_window_view(|_, cx| {
        StandaloneBuffer::new(
            path,
            AppTheme::gitcomet_dark(),
            root.downgrade(),
            store.into(),
            model,
            None,
            cx,
        )
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        if cx.update(|_, app| !buffer.read(app).loading) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "standalone file did not load"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("diagram_preview").is_some());
    assert!(cx.debug_bounds("document_diagram_single").is_some());
    assert!(cx.debug_bounds("diagram_source_toggle").is_none());
    let code = cx.debug_bounds("diagram_diff_view_code").unwrap();
    cx.simulate_click(code.center(), gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("diagram_preview").is_none());
    assert!(cx.debug_bounds("document_editor_scroll").is_some());
    let preview = cx.debug_bounds("diagram_diff_view_preview").unwrap();
    cx.simulate_click(preview.center(), gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("diagram_preview").is_some());
    let edit = cx.debug_bounds("document_edit").unwrap();
    cx.simulate_click(edit.center(), gpui::Modifiers::default());
    cx.update(|window, app| {
        assert!(buffer.read(app).editing);
        let _ = window.draw(app);
        buffer.update(app, |buffer, cx| {
            buffer
                .input
                .update(cx, |input, cx| input.set_text("flowchart LR\nA-->C", cx))
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("diagram_preview").is_none());
    cx.update(|_, app| {
        let buffer = buffer.read(app);
        assert!(buffer.dirty);
        let super::super::diagram_preview::DiagramInput::Text { text, .. } =
            buffer.diagram_input.as_ref().unwrap()
        else {
            panic!("buffer source expected")
        };
        assert_eq!(text.as_ref(), "flowchart LR\nA-->C");
    });
    cx.update(|_, app| {
        buffer.update(app, |buffer, cx| {
            buffer.input.update(cx, |input, cx| {
                input.set_text("a".repeat(gitcomet_diagrams::MAX_SOURCE_BYTES + 1), cx)
            });
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_, app| {
        assert!(matches!(
            buffer.read(app).diagram_input,
            Some(super::super::diagram_preview::DiagramInput::TooLarge { .. })
        ));
        assert!(buffer.read(app).dirty);
    });
}
