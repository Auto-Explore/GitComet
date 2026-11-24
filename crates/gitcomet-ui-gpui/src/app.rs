use crate::assets::GitCometAssets;
use crate::kit::search_keys::{
    DiffNextSearchMatchOrChange, DiffPrevSearchMatchOrChange, OpenActiveViewSearch,
    TextInputDiffNextSearchMatchOrChange, TextInputDiffPrevSearchMatchOrChange,
};
use crate::launch_guard::{UiLaunchError, run_with_panic_guard};
use crate::ui_scale;
use crate::view::{
    DiffNextChange, DiffNextFile, DiffPrevChange, DiffPrevFile, DiffRenderer,
    FocusedMergetoolLabels, FocusedMergetoolViewConfig, GitCometView, GitCometViewConfig,
    GitCometViewMode, HistoryFindPrevious, InitialRepositoryLaunchMode, LocateFileInExplorer,
    OpenRemoteInBrowser, PopoverPromptDismiss, PushUpstreamRemoteClose, PushUpstreamRemoteNext,
    PushUpstreamRemoteOpenOrSelect, PushUpstreamRemotePrev, SettingsWindowView, StartupCrashReport,
    TerminalCopy, TerminalPaste, TerminalSelectAll, TextInputCommitAndPushSubmit,
    TextInputCommitSubmit, TextInputDiffNextChange, TextInputDiffNextFile, TextInputDiffPrevChange,
    TextInputDiffPrevFile, ToggleCommandPalette, WorkspaceBootstrap, is_diff_shortcut_candidate,
};
use gitcomet_core::identity::{self, WindowKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::GitBackend;
use gitcomet_state::session;
use gitcomet_state::store::AppStore;

use gpui::{
    Action, AnyWindowHandle, App, AppContext, BorrowAppContext, Bounds, DisplayId, KeyBinding,
    Pixels, Size, TitlebarOptions, Unbind, Window, WindowBounds, WindowDecorations, WindowId,
    WindowOptions, actions, point, px, size,
};
#[cfg(target_os = "macos")]
use gpui::{Menu, MenuItem, OsAction, SystemMenuType};
use rustc_hash::{FxHashMap, FxHashSet};
#[cfg(target_os = "macos")]
use schemars::JsonSchema;
#[cfg(target_os = "macos")]
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

pub(super) const WINDOW_MIN_WIDTH_PX: f32 = 820.0;
pub(super) const WINDOW_MIN_HEIGHT_PX: f32 = 560.0;
pub(super) const WINDOW_DEFAULT_WIDTH_PX: f32 = 1280.0;
pub(super) const WINDOW_DEFAULT_HEIGHT_PX: f32 = 800.0;
pub(super) const FOCUSED_MERGETOOL_EXIT_CANCELED: i32 = 1;
#[cfg(test)]
pub(super) const FOCUSED_MERGETOOL_EXIT_SUCCESS: i32 = 0;
pub(super) const FOCUSED_MERGETOOL_EXIT_ERROR: i32 = 2;

mod bindings;
mod launch;
mod menus;
mod placement;
mod readiness;
mod routing;
mod windows;

pub(crate) use bindings::*;
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
mod test_support;
#[cfg(test)]
pub(crate) use test_support::{
    bind_app_keys_for_test, bind_terminal_keys_for_test, bind_text_input_keys_for_test,
    install_app_shortcuts_for_test, install_global_diff_navigation_actions_for_test,
    windows_owning_repo_for_test,
};
#[cfg(test)]
mod tests;
