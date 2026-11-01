use super::branch::{create_tracking_store, wait_until};
use super::*;

#[gpui::test]
fn file_history_picker_loads_file_browser_with_current_source(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("file-history-picker-load");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let source_before = store
        .snapshot()
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .map(|r| r.file_browser.source.clone())
        .expect("repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command("file-history", Some(window), cx);
        });
    });
    wait_until("file browser to start loading", || {
        store
            .snapshot()
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .is_some_and(|r| !matches!(r.file_browser.entries, Loadable::NotLoaded))
    });
    let source_after = store
        .snapshot()
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .map(|r| r.file_browser.source.clone())
        .expect("repo");
    assert_eq!(source_before, source_after);
}
