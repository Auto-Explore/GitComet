use super::*;
use std::time::Duration;

pub(crate) fn push_test_state(
    view: &GitCometView,
    state: Arc<AppState>,
    cx: &mut impl gpui::AppContext,
) {
    view._ui_model.update(cx, |model, cx| {
        model.set_state(state, cx);
    });
}

pub(crate) fn sync_store_snapshot(view: &GitCometView, cx: &mut impl gpui::AppContext) {
    push_test_state(view, view.store.snapshot(), cx);
}

pub(crate) fn set_sidebar_width_for_test(
    view: &mut GitCometView,
    width: gpui::Pixels,
    cx: &mut gpui::Context<GitCometView>,
) {
    view.set_sidebar_width_from_pixels(width);
    view.sidebar_render_width = width;
    view.sidebar_width_anim_seq = view.sidebar_width_anim_seq.wrapping_add(1);
    view.sidebar_width_animating = false;
    cx.notify();
}

pub(crate) fn popover_is_open(view: &GitCometView, app: &App) -> bool {
    popover_kind(view, app).is_some()
}

/// `(scrolled, max_scroll)` of the repository tab strip, in pixels.
pub(crate) fn repo_tab_scroll(view: &GitCometView, app: &App) -> (Pixels, Pixels) {
    view.repo_tabs_bar.read(app).tab_scroll_for_tests()
}

/// Window-space bounds of the scrollable repository tab strip.
pub(crate) fn repo_tab_strip_viewport(view: &GitCometView, app: &App) -> gpui::Bounds<Pixels> {
    view.repo_tabs_bar.read(app).tab_strip_viewport_for_tests()
}

pub(crate) fn add_repo_menu_is_open(view: &GitCometView, app: &App) -> bool {
    matches!(popover_kind(view, app), Some(PopoverKind::AddRepoMenu))
}

pub(in crate::view) fn history_refs_hover_is_open(view: &GitCometView, app: &App) -> bool {
    view.history_refs_hover_host.read(app).is_open_for_tests()
}

pub(in crate::view) fn history_refs_hover_source_bounds(
    view: &GitCometView,
    app: &App,
) -> Option<Bounds<Pixels>> {
    view.history_refs_hover_host
        .read(app)
        .source_bounds_for_tests()
}

pub(in crate::view) fn history_refs_hover_pinned_item_ix(
    view: &GitCometView,
    app: &App,
) -> Option<usize> {
    view.history_refs_hover_host
        .read(app)
        .pinned_item_ix_for_tests()
}

pub(in crate::view) fn history_refs_hover_pinned_item_text(
    view: &GitCometView,
    app: &App,
) -> Option<SharedString> {
    view.history_refs_hover_host
        .read(app)
        .pinned_item_text_for_tests()
}

pub(in crate::view) fn popover_kind(view: &GitCometView, app: &App) -> Option<PopoverKind> {
    view.popover_host.read(app).popover_kind_for_tests()
}

pub(crate) fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

pub(crate) fn wait_for_native_tooltip(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    redraw(cx);
}

pub(crate) fn tooltip_text(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> Option<SharedString> {
    redraw(cx);
    cx.update(|_window, app| view.read(app).tooltip_text_for_test(app))
}

pub(crate) fn open_repo_panel_visible(view: &GitCometView) -> bool {
    view.open_repo_panel
}

pub(crate) fn show_timezone(view: &GitCometView) -> bool {
    view.show_timezone
}

pub(in crate::view) fn change_tracking_view(view: &GitCometView) -> ChangeTrackingView {
    view.change_tracking_view
}

pub(in crate::view) fn diff_scroll_sync(view: &GitCometView) -> DiffScrollSync {
    view.diff_scroll_sync
}

pub(in crate::view) fn diff_content_mode(view: &GitCometView) -> DiffContentMode {
    view.diff_content_mode
}

pub(in crate::view) fn diff_whitespace_mode(view: &GitCometView) -> DiffWhitespaceMode {
    view.diff_whitespace_mode
}

pub(in crate::view) fn diff_reveal_whitespace_chars(view: &GitCometView) -> bool {
    view.diff_reveal_whitespace_chars
}

pub(in crate::view) fn diff_word_wrap(view: &GitCometView) -> bool {
    view.diff_word_wrap
}

pub(in crate::view) fn diff_show_line_numbers(view: &GitCometView) -> bool {
    view.diff_show_line_numbers
}
