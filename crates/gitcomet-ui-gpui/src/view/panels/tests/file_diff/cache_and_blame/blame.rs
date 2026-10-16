use super::*;

/// Regression: enabling blame used to force Split → Inline (and restore it on
/// toggle-off). Blame is an annotation column, not a view mode — the split left
/// column renders it just as the inline view does — so the selected mode must
/// survive the toggle in both directions.
#[gpui::test]
fn blame_toggle_keeps_split_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(281);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "blame_toggle_split",
        DiffViewMode::Split,
    );

    click_blame_toggle(cx);
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.diff_renderer.read(app);
        assert!(pane.annotate_enabled, "clicking blame should enable it");
        assert_eq!(
            pane.diff_view,
            DiffViewMode::Split,
            "enabling blame must not switch the diff view to Inline"
        );
        assert_eq!(root.diff_view_mode, DiffViewMode::Split);
    });

    click_blame_toggle(cx);
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.diff_renderer.read(app);
        assert!(!pane.annotate_enabled);
        assert_eq!(
            pane.diff_view,
            DiffViewMode::Split,
            "disabling blame must not change the diff view either"
        );
        assert_eq!(root.diff_view_mode, DiffViewMode::Split);
    });
}

#[gpui::test]
fn blame_toggle_keeps_inline_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(282);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "blame_toggle_inline",
        DiffViewMode::Inline,
    );

    click_blame_toggle(cx);
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.diff_renderer.read(app);
        assert!(pane.annotate_enabled);
        assert_eq!(pane.diff_view, DiffViewMode::Inline);
        assert_eq!(root.diff_view_mode, DiffViewMode::Inline);
    });
}

/// The annotation column narrows the left split column, so the shared split
/// wrap width must shrink when blame is on — the guarantee that made forcing
/// Inline unnecessary in the first place.
#[gpui::test]
fn split_annotate_reserves_the_annotation_column(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(283);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "blame_split_columns",
        DiffViewMode::Split,
    );

    let (_, split_without_blame) = cx.update(|window, app| {
        let diff_renderer = view.read(app).diff_renderer.clone();
        diff_renderer.update(app, |pane, cx| pane.diff_wrap_columns(window, cx))
    });

    click_blame_toggle(cx);

    let (_, split_with_blame) = cx.update(|window, app| {
        let diff_renderer = view.read(app).diff_renderer.clone();
        diff_renderer.update(app, |pane, cx| {
            assert!(
                pane.annotation_active(),
                "an unstaged working-tree diff supports blame, so the column is active"
            );
            pane.diff_wrap_columns(window, cx)
        })
    });

    assert!(
        split_with_blame < split_without_blame,
        "the annotation column must narrow the split wrap width \
         (with blame: {split_with_blame}, without: {split_without_blame})"
    );
}

fn click_blame_toggle(cx: &mut gpui::VisualTestContext) {
    let bounds = cx
        .debug_bounds("diff_annotate")
        .expect("the diff toolbar should render the blame toggle");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    draw_and_drain_test_window(cx);
}
