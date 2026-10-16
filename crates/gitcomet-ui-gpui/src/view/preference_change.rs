//! What the settings window asks every main window to apply. The settings
//! window sends these values instead of calling into the root view.

use super::{
    ChangeTrackingView, DefaultTagType, DiffContentMode, DiffScrollSync, DiffViewMode,
    DiffWhitespaceMode, FileListLayout, HistoryBranchNamesMode, RemoteMarkdownImagePolicy,
    RemoteUrlPolicy, TerminalPreferences, ThemeMode, components,
};
use crate::view::date_time::{DateTimeFormat, Timezone};

/// A preference change, or a settings-driven refresh, for one main window.
#[derive(Clone, Debug)]
pub(in crate::view) enum PreferenceChange {
    Terminal(TerminalPreferences),
    Toast(components::ToastKind, String),
    ThemeMode(ThemeMode),
    /// Font roles or sizes changed in the shared appearance.
    Fonts,
    DateTimeFormat(DateTimeFormat),
    Timezone(Timezone),
    ShowTimezone(bool),
    ChangeTrackingView(ChangeTrackingView),
    FileListLayout(FileListLayout),
    FileListSort(crate::view::rows::CommitFileSort),
    DiffScrollSync(DiffScrollSync),
    DiffContentMode(DiffContentMode),
    DiffWhitespaceMode(DiffWhitespaceMode),
    DiffViewMode(DiffViewMode),
    DiffRevealWhitespaceChars(bool),
    DiffWordWrap(bool),
    DiffTabSize(u8),
    DiffShowLineNumbers(bool),
    AutoSaveFileEdits(bool),
    RemoteMarkdownImagePolicy(RemoteMarkdownImagePolicy),
    RemoteUrlPolicy(RemoteUrlPolicy),
    CheckForUpdatesOnStartup(bool),
    HistoryBranchNames(HistoryBranchNamesMode),
    HistoryColumns {
        graph: bool,
        author: bool,
        date: bool,
        sha: bool,
    },
    HistoryHighlightCommitChain(bool),
    HistoryShowGraphGradient(bool),
    FilesFollowSelectedCommit(bool),
    HistoryRelativeDates(bool),
    HistoryTags {
        show: bool,
        fetch_mode: gitcomet_state::model::GitLogTagFetchMode,
    },
    VerifyCommitSignatures(bool),
    DefaultHistoryMode(gitcomet_core::domain::HistoryMode),
    DefaultTagType(DefaultTagType),
    LargeFileSettings(gitcomet_state::model::LargeFileSettings),
    RemotePrune(bool),
    MaintenanceRecommendations(bool),
    ResetHistoryColumnWidths,
    /// A new Git executable was probed; the window's signing probe restarts.
    GitRuntime(gitcomet_core::process::GitRuntimeState),
    LargeFileTools(gitcomet_core::large_file_tools::LargeFileToolsState),
}
