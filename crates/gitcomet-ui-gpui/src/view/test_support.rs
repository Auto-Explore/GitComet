use super::*;

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

pub(crate) fn popover_is_open(view: &GitCometView, app: &App) -> bool {
    popover_kind(view, app).is_some()
}

pub(in crate::view) fn popover_kind(view: &GitCometView, app: &App) -> Option<PopoverKind> {
    view.popover_host.read(app).popover_kind_for_tests()
}

pub(crate) fn tooltip_text(view: &GitCometView, app: &App) -> Option<SharedString> {
    view.tooltip_host.read(app).tooltip_text_for_test()
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
