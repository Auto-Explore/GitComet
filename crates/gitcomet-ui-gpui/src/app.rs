use crate::assets::GitCometAssets;
use crate::launch_guard::{UiLaunchError, run_with_panic_guard};
use crate::ui_scale;
use crate::view::{
    DiffNextFile, DiffNextSearchMatchOrChange, DiffPrevFile, DiffPrevSearchMatchOrChange,
    FocusedMergetoolLabels, FocusedMergetoolViewConfig, GitCometView, GitCometViewConfig,
    GitCometViewMode, HistoryFindPrevious, InitialRepositoryLaunchMode, LocateFileInExplorer,
    MainPaneView, OpenActiveViewSearch, OpenRemoteInBrowser, PopoverPromptDismiss,
    PopoverPromptTabNext, PopoverPromptTabPrev, PushUpstreamRemoteClose, PushUpstreamRemoteNext,
    PushUpstreamRemoteOpenOrSelect, PushUpstreamRemotePrev, SettingsWindowView, StartupCrashReport,
    TerminalCopy, TerminalPaste, TerminalSelectAll, TextInputCommitSubmit, TextInputDiffNextChange,
    TextInputDiffNextFile, TextInputDiffNextSearchMatchOrChange, TextInputDiffPrevChange,
    TextInputDiffPrevFile, TextInputDiffPrevSearchMatchOrChange, ToggleCommandPalette,
    WorkspaceBootstrap, is_diff_shortcut_candidate,
};
use gitcomet_core::identity::{self, WindowKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::GitBackend;
use gitcomet_state::session;
use gitcomet_state::store::AppStore;

use gpui::{
    Action, AnyWindowHandle, App, AppContext, BorrowAppContext, Bounds, DisplayId, KeyBinding,
    Pixels, Point, Size, TitlebarOptions, Unbind, Window, WindowBounds, WindowDecorations,
    WindowId, WindowOptions, actions, point, px, size,
};
#[cfg(target_os = "macos")]
use gpui::{Menu, MenuItem, OsAction, SystemMenuType};
#[cfg(target_os = "windows")]
use raw_window_handle::RawWindowHandle;
use rustc_hash::{FxHashMap, FxHashSet};
#[cfg(target_os = "macos")]
use schemars::JsonSchema;
#[cfg(target_os = "macos")]
use serde::Deserialize;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

pub(super) const WINDOW_MIN_WIDTH_PX: f32 = 820.0;
pub(super) const WINDOW_MIN_HEIGHT_PX: f32 = 560.0;
pub(super) const WINDOW_DEFAULT_WIDTH_PX: f32 = 1280.0;
pub(super) const WINDOW_DEFAULT_HEIGHT_PX: f32 = 800.0;
pub(super) const FOCUSED_MERGETOOL_EXIT_CANCELED: i32 = 1;
#[cfg(test)]
pub(super) const FOCUSED_MERGETOOL_EXIT_SUCCESS: i32 = 0;
pub(super) const FOCUSED_MERGETOOL_EXIT_ERROR: i32 = 2;

mod bindings;
mod chrome;
mod launch;
mod menus;
mod placement;
mod routing;
mod windows;

pub(crate) use bindings::*;
pub(crate) use chrome::*;
pub use launch::*;
#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
pub(crate) use menus::*;
pub(crate) use placement::*;
pub(crate) use routing::*;
pub(crate) use windows::*;

actions!(
    app_menu,
    [
        NewWindow,
        OpenSettings,
        OpenInCodeEditor,
        OpenRepository,
        CloneRepository,
        InitializeRepository,
        SwitchRepository,
        OpenWorkspace,
        ApplyPatch,
        CheckForUpdates,
        ShowReflog,
        Close,
        CloseWindow,
        PreviousRepository,
        NextRepository,
        MinimizeWindow,
        ZoomWindow,
        ToggleFullScreen,
        IncreaseUiScale,
        DecreaseUiScale,
        ResetUiScale,
        Hide,
        HideOthers,
        ShowAll,
        Quit,
    ]
);

#[cfg(test)]
pub(crate) fn bind_text_input_keys_for_test(cx: &mut App) {
    bind_text_input_keys(cx);
}

#[cfg(test)]
pub(crate) fn bind_app_keys_for_test(cx: &mut App) {
    bind_app_keys(cx);
}

#[cfg(test)]
pub(crate) fn bind_terminal_keys_for_test(cx: &mut App) {
    bind_terminal_keys(cx);
}

#[cfg(test)]
pub(crate) fn install_app_shortcuts_for_test(app: &mut App, backend: Arc<dyn GitBackend>) {
    bind_app_keys(app);
    app.set_global(GitCometBackendGlobal(Arc::clone(&backend)));
    install_app_actions(app, backend);
}

#[cfg(test)]
pub(crate) fn windows_owning_repo_for_test(cx: &mut App, path: &Path) -> Vec<gpui::WindowId> {
    gitcomet_window_entries(cx)
        .into_iter()
        .filter(|entry| entry_contains_repo_path(entry, path))
        .map(|entry| entry.handle.window_id())
        .collect()
}

#[cfg(test)]
mod tests;
