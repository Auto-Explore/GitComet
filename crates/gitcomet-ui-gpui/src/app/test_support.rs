use super::bindings::{
    bind_app_keys, bind_terminal_keys, bind_text_input_keys, install_app_actions,
    install_global_diff_navigation_actions,
};
use super::launch::GitCometBackendGlobal;
use super::windows::{entry_contains_repo_path, gitcomet_window_entries};
use gitcomet_core::services::GitBackend;
use gpui::App;
use std::{path::Path, sync::Arc};

pub(crate) fn bind_text_input_keys_for_test(cx: &mut App) {
    bind_text_input_keys(cx);
}

pub(crate) fn bind_app_keys_for_test(cx: &mut App) {
    bind_app_keys(cx);
}

pub(crate) fn install_global_diff_navigation_actions_for_test(cx: &mut App) {
    install_global_diff_navigation_actions(cx);
}

pub(crate) fn bind_terminal_keys_for_test(cx: &mut App) {
    bind_terminal_keys(cx);
}

pub(crate) fn install_app_shortcuts_for_test(app: &mut App, backend: Arc<dyn GitBackend>) {
    bind_app_keys(app);
    app.set_global(GitCometBackendGlobal(Arc::clone(&backend)));
    install_app_actions(app, backend);
}

pub(crate) fn windows_owning_repo_for_test(cx: &mut App, path: &Path) -> Vec<gpui::WindowId> {
    gitcomet_window_entries(cx)
        .into_iter()
        .filter(|entry| entry_contains_repo_path(entry, path))
        .map(|entry| entry.handle.window_id())
        .collect()
}
