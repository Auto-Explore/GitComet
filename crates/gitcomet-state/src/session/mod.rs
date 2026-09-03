use crate::model::{AppState, DefaultTagType, GitLogTagFetchMode, RepoId};
use gitcomet_core::domain::{HistoryMode, LogScope};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::{env, fs, io};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiSession {
    pub open_repos: Vec<PathBuf>,
    pub active_repo: Option<PathBuf>,
    pub recent_repos: Vec<PathBuf>,
    /// Repositories the user pinned in the repository picker, in the order they
    /// were pinned. Independent of `recent_repos`, so a pin outlives the
    /// recents cap.
    pub pinned_repos: Vec<PathBuf>,
    pub repo_picker_sort: Option<String>,
    /// Storage keys of the repository picker sections the user folded away.
    /// Every section defaults to expanded, so this only ever holds deviations.
    pub repo_picker_collapsed_sections: BTreeSet<String>,
    pub repo_sidebar_collapsed_items: BTreeMap<PathBuf, BTreeSet<String>>,
    pub repo_sidebar_pinned_branches: BTreeMap<PathBuf, BTreeSet<String>>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub sidebar_width: Option<u32>,
    pub details_width: Option<u32>,
    pub sidebar_collapsed: Option<bool>,
    pub theme_mode: Option<String>,
    pub ui_scale_percent: Option<u32>,
    pub ui_font_family: Option<String>,
    pub editor_font_family: Option<String>,
    pub use_font_ligatures: Option<bool>,
    pub date_time_format: Option<String>,
    pub timezone: Option<String>,
    pub show_timezone: Option<bool>,
    pub change_tracking_view: Option<String>,
    pub diff_scroll_sync: Option<String>,
    pub diff_content_mode: Option<String>,
    pub diff_whitespace_mode: Option<String>,
    pub diff_view_mode: Option<String>,
    pub annotate_enabled: Option<bool>,
    pub diff_reveal_whitespace_chars: Option<bool>,
    pub diff_word_wrap: Option<bool>,
    pub diff_show_line_numbers: Option<bool>,
    pub auto_save_file_edits: Option<bool>,
    pub mergetool_auto_advance: Option<bool>,
    pub mergetool_collapse_unchanged: Option<bool>,
    pub mergetool_output_scroll_sync: Option<bool>,
    pub mergetool_show_line_numbers: Option<bool>,
    pub mergetool_view_three_way: Option<bool>,
    pub change_tracking_height: Option<u32>,
    pub untracked_height: Option<u32>,
    pub history_show_graph: Option<bool>,
    pub history_show_author: Option<bool>,
    pub history_show_date: Option<bool>,
    pub history_show_sha: Option<bool>,
    pub terminal_external_mode: Option<String>,
    pub terminal_external_program: Option<String>,
    pub terminal_external_args: Option<Vec<String>>,
    pub terminal_action_bar_target: Option<String>,
    pub history_show_tags: Option<bool>,
    pub history_relative_dates: Option<bool>,
    pub history_highlight_commit_chain: Option<bool>,
    pub history_tag_fetch_mode: Option<GitLogTagFetchMode>,
    pub default_history_mode: Option<HistoryMode>,
    pub commit_push_after_enabled: Option<bool>,
    pub default_tag_type: Option<DefaultTagType>,
    pub git_executable_path: Option<PathBuf>,
    pub external_code_editor: Option<ExternalCodeEditorSetting>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExternalCodeEditorSetting {
    Detected {
        id: String,
        path: PathBuf,
    },
    Custom {
        executable: PathBuf,
        arguments: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UiSessionFileV1 {
    version: u32,
    open_repos: Vec<String>,
    active_repo: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct UiSessionFile {
    version: u32,
    open_repos: Vec<String>,
    active_repo: Option<String>,
    recent_repos: Option<Vec<String>>,
    pinned_repos: Option<Vec<String>>,
    repo_picker_sort: Option<String>,
    repo_picker_collapsed_sections: Option<BTreeSet<String>>,
    repo_sidebar_collapsed_items: Option<BTreeMap<String, BTreeSet<String>>>,
    repo_sidebar_pinned_branches: Option<BTreeMap<String, BTreeSet<String>>>,
    window_width: Option<u32>,
    window_height: Option<u32>,
    sidebar_width: Option<u32>,
    details_width: Option<u32>,
    sidebar_collapsed: Option<bool>,
    theme_mode: Option<String>,
    ui_scale_percent: Option<u32>,
    ui_font_family: Option<String>,
    editor_font_family: Option<String>,
    use_font_ligatures: Option<bool>,
    date_time_format: Option<String>,
    timezone: Option<String>,
    show_timezone: Option<bool>,
    change_tracking_view: Option<String>,
    diff_scroll_sync: Option<String>,
    diff_content_mode: Option<String>,
    diff_whitespace_mode: Option<String>,
    diff_view_mode: Option<String>,
    annotate_enabled: Option<bool>,
    diff_reveal_whitespace_chars: Option<bool>,
    diff_word_wrap: Option<bool>,
    diff_show_line_numbers: Option<bool>,
    auto_save_file_edits: Option<bool>,
    mergetool_auto_advance: Option<bool>,
    mergetool_collapse_unchanged: Option<bool>,
    mergetool_output_scroll_sync: Option<bool>,
    mergetool_show_line_numbers: Option<bool>,
    mergetool_view_three_way: Option<bool>,
    change_tracking_height: Option<u32>,
    untracked_height: Option<u32>,
    history_show_graph: Option<bool>,
    history_show_author: Option<bool>,
    history_show_date: Option<bool>,
    history_show_sha: Option<bool>,
    terminal_external_mode: Option<String>,
    terminal_external_program: Option<String>,
    terminal_external_args: Option<Vec<String>>,
    terminal_action_bar_target: Option<String>,
    history_show_tags: Option<bool>,
    history_relative_dates: Option<bool>,
    history_highlight_commit_chain: Option<bool>,
    history_tag_fetch_mode: Option<GitLogTagFetchMode>,
    default_history_mode: Option<HistoryModeSetting>,
    commit_push_after_enabled: Option<bool>,
    default_tag_type: Option<DefaultTagType>,
    git_executable_path: Option<String>,
    external_code_editor: Option<ExternalCodeEditorSettingFile>,
    repo_history_modes: Option<BTreeMap<String, HistoryModeSetting>>,
    repo_history_scopes: Option<BTreeMap<String, HistoryScopeSetting>>,
    repo_history_author_filters: Option<BTreeMap<String, Option<String>>>,
    repo_fetch_prune_deleted_remote_tracking_branches: Option<BTreeMap<String, bool>>,
    survey_prompt: Option<SurveyPromptSession>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ExternalCodeEditorSettingFile {
    Detected {
        id: String,
        path: String,
    },
    Custom {
        executable: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        arguments: Option<String>,
    },
}

const SESSION_FILE_VERSION_V1: u32 = 1;
const SESSION_FILE_VERSION_V2: u32 = 2;
const SESSION_FILE_VERSION_V3: u32 = 3;
const CURRENT_SESSION_FILE_VERSION: u32 = SESSION_FILE_VERSION_V3;
const MAX_RECENT_REPOS: usize = 15;
const DEFAULT_UI_SCALE_PERCENT: u32 = 100;
const MIN_UI_SCALE_PERCENT: u32 = 80;
const MAX_UI_SCALE_PERCENT: u32 = 200;
#[cfg(unix)]
const SESSION_PATH_BYTES_PREFIX: &str = "gitcomet-path-bytes:";
#[cfg(windows)]
const SESSION_PATH_WIDE_PREFIX: &str = "gitcomet-path-utf16le:";

const SESSION_FILE_ENV: &str = "GITCOMET_SESSION_FILE";
const DISABLE_SESSION_PERSIST_ENV: &str = "GITCOMET_DISABLE_SESSION_PERSIST";

pub fn load() -> UiSession {
    let Some(path) = default_session_file_path() else {
        return UiSession::default();
    };

    load_from_path(&path)
}

pub fn load_from_path(path: &Path) -> UiSession {
    let Some(file) = load_file(path) else {
        return UiSession::default();
    };

    let (open_repos, active_repo) = parse_repos(file.open_repos, file.active_repo);
    let recent_repos = parse_path_list(file.recent_repos.unwrap_or_default());
    let pinned_repos = parse_path_list(file.pinned_repos.unwrap_or_default());
    let repo_sidebar_collapsed_items =
        parse_path_keyed_string_sets(file.repo_sidebar_collapsed_items.unwrap_or_default());
    let repo_sidebar_pinned_branches =
        parse_path_keyed_string_sets(file.repo_sidebar_pinned_branches.unwrap_or_default());
    UiSession {
        open_repos,
        active_repo,
        recent_repos,
        pinned_repos,
        repo_picker_sort: file.repo_picker_sort,
        repo_picker_collapsed_sections: file.repo_picker_collapsed_sections.unwrap_or_default(),
        repo_sidebar_collapsed_items,
        repo_sidebar_pinned_branches,
        window_width: file.window_width,
        window_height: file.window_height,
        sidebar_width: file.sidebar_width,
        details_width: file.details_width,
        sidebar_collapsed: file.sidebar_collapsed,
        theme_mode: file.theme_mode,
        ui_scale_percent: file.ui_scale_percent,
        ui_font_family: file.ui_font_family,
        editor_font_family: file.editor_font_family,
        use_font_ligatures: file.use_font_ligatures,
        date_time_format: file.date_time_format,
        timezone: file.timezone,
        show_timezone: file.show_timezone,
        change_tracking_view: file.change_tracking_view,
        diff_scroll_sync: file.diff_scroll_sync,
        diff_content_mode: file.diff_content_mode,
        diff_whitespace_mode: file.diff_whitespace_mode,
        diff_view_mode: file.diff_view_mode,
        annotate_enabled: file.annotate_enabled,
        diff_reveal_whitespace_chars: file.diff_reveal_whitespace_chars,
        diff_word_wrap: file.diff_word_wrap,
        diff_show_line_numbers: file.diff_show_line_numbers,
        auto_save_file_edits: file.auto_save_file_edits,
        mergetool_auto_advance: file.mergetool_auto_advance,
        mergetool_collapse_unchanged: file.mergetool_collapse_unchanged,
        mergetool_output_scroll_sync: file.mergetool_output_scroll_sync,
        mergetool_show_line_numbers: file.mergetool_show_line_numbers,
        mergetool_view_three_way: file.mergetool_view_three_way,
        change_tracking_height: file.change_tracking_height,
        untracked_height: file.untracked_height,
        history_show_graph: file.history_show_graph,
        history_show_author: file.history_show_author,
        history_show_date: file.history_show_date,
        history_show_sha: file.history_show_sha,
        terminal_external_mode: file.terminal_external_mode,
        terminal_external_program: file.terminal_external_program,
        terminal_external_args: file.terminal_external_args,
        terminal_action_bar_target: file.terminal_action_bar_target,
        history_show_tags: file.history_show_tags,
        history_relative_dates: file.history_relative_dates,
        history_highlight_commit_chain: file.history_highlight_commit_chain,
        history_tag_fetch_mode: file.history_tag_fetch_mode,
        default_history_mode: file.default_history_mode.map(Into::into),
        commit_push_after_enabled: file.commit_push_after_enabled,
        default_tag_type: file.default_tag_type,
        git_executable_path: file
            .git_executable_path
            .as_deref()
            .map(path_from_storage_key),
        external_code_editor: external_code_editor_from_file(file.external_code_editor),
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RepoSessionPreferences {
    pub(crate) default_history_mode: Option<HistoryMode>,
    pub(crate) repo_history_modes: BTreeMap<String, HistoryMode>,
    pub(crate) repo_history_scopes: BTreeMap<String, LogScope>,
    pub(crate) repo_history_author_filters: BTreeMap<String, Option<String>>,
    pub(crate) repo_fetch_prune_deleted_remote_tracking_branches: BTreeMap<String, bool>,
}

#[cfg(test)]
thread_local! {
    static TEST_SESSION_FILE_PATH_OVERRIDE: RefCell<Vec<Option<PathBuf>>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) struct TestSessionFilePathGuard;

#[cfg(test)]
pub(crate) fn push_test_session_file_path_override(
    path: impl Into<Option<PathBuf>>,
) -> TestSessionFilePathGuard {
    TEST_SESSION_FILE_PATH_OVERRIDE.with(|stack| stack.borrow_mut().push(path.into()));
    TestSessionFilePathGuard
}

#[cfg(test)]
impl Drop for TestSessionFilePathGuard {
    fn drop(&mut self) {
        TEST_SESSION_FILE_PATH_OVERRIDE.with(|stack| {
            let popped = stack.borrow_mut().pop();
            debug_assert!(popped.is_some(), "session path override stack underflow");
        });
    }
}

#[cfg(test)]
fn test_session_file_path_override() -> Option<Option<PathBuf>> {
    TEST_SESSION_FILE_PATH_OVERRIDE.with(|stack| stack.borrow().last().cloned())
}

static SESSION_FILE_PERSIST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn session_file_persist_lock() -> &'static Mutex<()> {
    SESSION_FILE_PERSIST_LOCK.get_or_init(|| Mutex::new(()))
}

fn with_session_file_persist_lock<T>(persist: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let _guard = session_file_persist_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    persist()
}

fn load_file(path: &Path) -> Option<UiSessionFile> {
    let Ok(contents) = fs::read_to_string(path) else {
        return None;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return None;
    };
    let version = value
        .get("version")
        .and_then(|v| v.as_u64())
        .unwrap_or(SESSION_FILE_VERSION_V1 as u64) as u32;
    match version {
        SESSION_FILE_VERSION_V1 => {
            let file: UiSessionFileV1 = serde_json::from_value(value).ok()?;
            Some(UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: file.open_repos,
                active_repo: file.active_repo,
                ..UiSessionFile::default()
            })
        }
        SESSION_FILE_VERSION_V2 => {
            let file = serde_json::from_value::<UiSessionFile>(value).ok()?;
            Some(migrate_v2_file(file))
        }
        SESSION_FILE_VERSION_V3 => serde_json::from_value::<UiSessionFile>(value).ok(),
        _ => None,
    }
}

fn persist_to_path(path: &Path, session: &impl Serialize) -> io::Result<()> {
    let contents = serde_json::to_vec(session).expect("serializing session file should succeed");
    // Records every open repository path; keep it owner-only.
    gitcomet_core::fs_utils::write_private_file(path, &contents)
}

fn default_session_file_path() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = test_session_file_path_override() {
        return path;
    }

    if let Some(path) = env::var_os(SESSION_FILE_ENV)
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }

    if env::var_os(DISABLE_SESSION_PERSIST_ENV).is_some() {
        return None;
    }

    // Avoid reading/writing user state dir during test binaries (e.g. `cargo test`, `cargo nextest`).
    // `cfg!(test)` only applies to this crate's own unit tests; dependencies built for tests do not
    // have `cfg(test)` set, so we also use a runtime heuristic.
    if cfg!(test) || running_under_test_harness() {
        return None;
    }

    Some(app_state_dir()?.join("session.json"))
}

pub(crate) fn default_session_file_path_for_effect() -> Option<PathBuf> {
    default_session_file_path()
}

fn running_under_test_harness() -> bool {
    let Ok(exe) = env::current_exe() else {
        return false;
    };
    looks_like_test_binary(&exe)
}

fn looks_like_test_binary(exe: &Path) -> bool {
    if exe.components().any(|component| {
        component.as_os_str() == OsStr::new("deps")
            || component.as_os_str() == OsStr::new("nextest")
    }) {
        return true;
    }

    exe.file_stem()
        .is_some_and(looks_like_cargo_test_binary_name)
}

fn looks_like_cargo_test_binary_name(stem: &OsStr) -> bool {
    let Some(stem) = stem.to_str() else {
        return false;
    };
    let Some((_prefix, suffix)) = stem.rsplit_once('-') else {
        return false;
    };
    // Cargo test binaries typically end in a 16-hex-digit hash suffix, e.g. `mycrate-3ad1b0fd3f0c0d3e`.
    suffix.len() == 16 && suffix.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn user_themes_dir() -> Option<PathBuf> {
    if cfg!(test) || running_under_test_harness() {
        return None;
    }

    Some(app_data_dir()?.join("themes"))
}

fn non_empty_path(value: Option<&OsStr>) -> Option<PathBuf> {
    let value = value?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn app_data_dir() -> Option<PathBuf> {
    // Follow XDG on linux; otherwise fall back to platform conventions.
    #[cfg(target_os = "linux")]
    {
        app_data_dir_linux(
            env::var_os("XDG_DATA_HOME").as_deref(),
            env::var_os("HOME").as_deref(),
        )
    }

    #[cfg(target_os = "macos")]
    {
        let home = non_empty_path(env::var_os("HOME").as_deref())?;
        Some(home.join("Library/Application Support/gitcomet"))
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = env::var_os("LOCALAPPDATA").or_else(|| env::var_os("APPDATA"));
        Some(non_empty_path(appdata.as_deref())?.join("gitcomet"))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        non_empty_path(env::var_os("HOME").as_deref()).map(|home| home.join(".gitcomet"))
    }
}

#[cfg(target_os = "linux")]
fn app_data_dir_linux(xdg_data_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(data_home) = non_empty_path(xdg_data_home) {
        return Some(data_home.join("gitcomet"));
    }
    let home = non_empty_path(home)?;
    Some(home.join(".local/share/gitcomet"))
}

fn app_state_dir() -> Option<PathBuf> {
    // Follow XDG on linux; otherwise fall back to platform conventions.
    #[cfg(target_os = "linux")]
    {
        if let Some(state_home) = non_empty_path(env::var_os("XDG_STATE_HOME").as_deref()) {
            return Some(state_home.join("gitcomet"));
        }
        let home = non_empty_path(env::var_os("HOME").as_deref())?;
        Some(home.join(".local/state/gitcomet"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = non_empty_path(env::var_os("HOME").as_deref())?;
        Some(home.join("Library/Application Support/gitcomet"))
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = env::var_os("LOCALAPPDATA").or_else(|| env::var_os("APPDATA"));
        Some(non_empty_path(appdata.as_deref())?.join("gitcomet"))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        non_empty_path(env::var_os("HOME").as_deref()).map(|home| home.join(".gitcomet"))
    }
}

use history_mode::{HistoryModeSetting, HistoryScopeSetting};
use parse::*;
use survey::SurveyPromptSession;

mod history_mode;
mod parse;
mod paths;
mod repos;
mod settings;
mod survey;

pub use history_mode::*;
pub use paths::*;
pub use repos::*;
pub use settings::*;
pub use survey::*;

pub(crate) use history_mode::persist_repo_history_modes_batch_to_path;
pub(crate) use repos::load_repo_session_preferences;
#[cfg(test)]
pub(crate) use repos::load_repo_session_preferences_from_path;

#[cfg(test)]
mod tests;
