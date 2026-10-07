use crate::model::{AppState, DefaultTagType, GitLogTagFetchMode, RepoId};
use gitcomet_core::domain::{HistoryMode, LogScope};
use gitcomet_core::platform::dirs;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::{env, fs, io};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiSession {
    pub workspaces: Vec<Workspace>,
    pub open_repos: Vec<PathBuf>,
    pub active_repo: Option<PathBuf>,
    pub recent_repos: Vec<PathBuf>,
    pub recent_documents: Vec<PathBuf>,
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
    /// The focused mergetool's own size; normal windows use workspace frames.
    pub mergetool_window_width: Option<u32>,
    pub mergetool_window_height: Option<u32>,
    pub sidebar_width: Option<u32>,
    pub details_width: Option<u32>,
    pub sidebar_collapsed: Option<bool>,
    pub theme_mode: Option<String>,
    pub ui_scale_percent: Option<u32>,
    pub window_controls_mode: Option<String>,
    pub browser_open_target: Option<String>,
    pub ui_density: Option<String>,
    pub ui_font_size_px: Option<u32>,
    pub editor_font_size_px: Option<u32>,
    pub markdown_preview_font_size_px: Option<u32>,
    pub ui_font_family: Option<String>,
    pub editor_font_family: Option<String>,
    pub use_font_ligatures: Option<bool>,
    pub date_time_format: Option<String>,
    pub timezone: Option<String>,
    pub show_timezone: Option<bool>,
    pub change_tracking_view: Option<String>,
    pub file_list_layout: Option<String>,
    pub file_list_sort: Option<String>,
    pub diff_scroll_sync: Option<String>,
    pub diff_content_mode: Option<String>,
    pub diff_whitespace_mode: Option<String>,
    pub diff_view_mode: Option<String>,
    pub annotate_enabled: Option<bool>,
    pub diff_reveal_whitespace_chars: Option<bool>,
    pub diff_word_wrap: Option<bool>,
    /// Columns a tab advances to in the text views.
    pub diff_tab_size: Option<u8>,
    pub diff_show_line_numbers: Option<bool>,
    pub remote_markdown_image_policy: Option<String>,
    pub allowed_remote_protocols: Option<BTreeSet<String>>,
    pub check_for_updates_on_startup: Option<bool>,
    pub auto_save_file_edits: Option<bool>,
    pub mergetool_auto_advance: Option<bool>,
    pub mergetool_collapse_unchanged: Option<bool>,
    pub mergetool_output_scroll_sync: Option<bool>,
    pub mergetool_show_line_numbers: Option<bool>,
    pub mergetool_view_three_way: Option<bool>,
    pub change_tracking_height: Option<u32>,
    pub untracked_height: Option<u32>,
    pub history_branch_names: Option<String>,
    pub history_show_graph: Option<bool>,
    pub history_show_author: Option<bool>,
    pub history_show_date: Option<bool>,
    pub history_show_sha: Option<bool>,
    pub terminal_external_mode: Option<String>,
    pub terminal_external_program: Option<String>,
    pub terminal_external_args: Option<Vec<String>>,
    pub terminal_action_bar_target: Option<String>,
    pub history_show_tags: Option<bool>,
    pub history_verify_commit_signatures: Option<bool>,
    pub history_relative_dates: Option<bool>,
    pub history_highlight_commit_chain: Option<bool>,
    pub file_browser_follow_selected_commit: Option<bool>,
    pub annex_hide_bookkeeping_refs: Option<bool>,
    pub annex_pull_push_on_adjusted: Option<bool>,
    pub annex_sync_content: Option<bool>,
    pub history_tag_fetch_mode: Option<GitLogTagFetchMode>,
    pub default_history_mode: Option<HistoryMode>,
    pub commit_push_after_enabled: Option<bool>,
    pub default_tag_type: Option<DefaultTagType>,
    pub fetch_prune_deleted_remote_branches: Option<bool>,
    pub recommend_repo_maintenance: Option<bool>,
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
pub(crate) struct UiSessionFile {
    version: u32,
    // V4 (branch-only) stored workspaces under their old "window groups" name.
    #[serde(
        alias = "window_groups",
        default,
        deserialize_with = "lenient_workspaces"
    )]
    workspaces: Option<Vec<WorkspaceFile>>,
    open_repos: Vec<String>,
    active_repo: Option<String>,
    recent_repos: Option<Vec<String>>,
    recent_documents: Option<Vec<String>>,
    pinned_repos: Option<Vec<String>>,
    repo_picker_sort: Option<String>,
    repo_picker_collapsed_sections: Option<BTreeSet<String>>,
    repo_sidebar_collapsed_items: Option<BTreeMap<String, BTreeSet<String>>>,
    repo_sidebar_pinned_branches: Option<BTreeMap<String, BTreeSet<String>>>,
    window_width: Option<u32>,
    window_height: Option<u32>,
    mergetool_window_width: Option<u32>,
    mergetool_window_height: Option<u32>,
    sidebar_width: Option<u32>,
    details_width: Option<u32>,
    sidebar_collapsed: Option<bool>,
    theme_mode: Option<String>,
    ui_scale_percent: Option<u32>,
    window_controls_mode: Option<String>,
    browser_open_target: Option<String>,
    ui_density: Option<String>,
    ui_font_size_px: Option<u32>,
    editor_font_size_px: Option<u32>,
    markdown_preview_font_size_px: Option<u32>,
    ui_font_family: Option<String>,
    editor_font_family: Option<String>,
    use_font_ligatures: Option<bool>,
    date_time_format: Option<String>,
    timezone: Option<String>,
    show_timezone: Option<bool>,
    change_tracking_view: Option<String>,
    file_list_layout: Option<String>,
    file_list_sort: Option<String>,
    diff_scroll_sync: Option<String>,
    diff_content_mode: Option<String>,
    diff_whitespace_mode: Option<String>,
    diff_view_mode: Option<String>,
    annotate_enabled: Option<bool>,
    diff_reveal_whitespace_chars: Option<bool>,
    diff_word_wrap: Option<bool>,
    diff_tab_size: Option<u8>,
    diff_show_line_numbers: Option<bool>,
    remote_markdown_image_policy: Option<String>,
    allowed_remote_protocols: Option<BTreeSet<String>>,
    check_for_updates_on_startup: Option<bool>,
    auto_save_file_edits: Option<bool>,
    mergetool_auto_advance: Option<bool>,
    mergetool_collapse_unchanged: Option<bool>,
    mergetool_output_scroll_sync: Option<bool>,
    mergetool_show_line_numbers: Option<bool>,
    mergetool_view_three_way: Option<bool>,
    change_tracking_height: Option<u32>,
    untracked_height: Option<u32>,
    history_branch_names: Option<String>,
    history_show_graph: Option<bool>,
    history_show_author: Option<bool>,
    history_show_date: Option<bool>,
    history_show_sha: Option<bool>,
    terminal_external_mode: Option<String>,
    terminal_external_program: Option<String>,
    terminal_external_args: Option<Vec<String>>,
    terminal_action_bar_target: Option<String>,
    history_show_tags: Option<bool>,
    history_verify_commit_signatures: Option<bool>,
    history_verify_commit_signatures_opt_in: Option<bool>,
    history_relative_dates: Option<bool>,
    history_highlight_commit_chain: Option<bool>,
    file_browser_follow_selected_commit: Option<bool>,
    #[serde(default)]
    annex_hide_bookkeeping_refs: Option<bool>,
    #[serde(default)]
    annex_pull_push_on_adjusted: Option<bool>,
    #[serde(default)]
    annex_sync_content: Option<bool>,
    history_tag_fetch_mode: Option<GitLogTagFetchMode>,
    default_history_mode: Option<HistoryModeSetting>,
    commit_push_after_enabled: Option<bool>,
    default_tag_type: Option<DefaultTagType>,
    fetch_prune_deleted_remote_branches: Option<bool>,
    recommend_repo_maintenance: Option<bool>,
    git_executable_path: Option<String>,
    external_code_editor: Option<ExternalCodeEditorSettingFile>,
    #[serde(
        default,
        deserialize_with = "repository_preferences::lenient_preferences"
    )]
    repository_preferences: BTreeMap<String, crate::model::SharedRepositoryPreferences>,
    repo_history_modes: Option<BTreeMap<String, HistoryModeSetting>>,
    repo_history_scopes: Option<BTreeMap<String, HistoryScopeSetting>>,
    repo_history_author_filters: Option<BTreeMap<String, Option<String>>>,
    #[serde(skip_serializing)]
    repo_fetch_prune_deleted_remote_tracking_branches: Option<BTreeMap<String, bool>>,
    survey_prompt: Option<SurveyPromptSession>,
    #[serde(default, deserialize_with = "lenient_repo_maintenance")]
    repo_maintenance: Option<BTreeMap<String, RepoMaintenanceSession>>,
    /// Extension namespaces, kept verbatim: any JSON loads, so a malformed
    /// namespace never makes the rest of the session unreadable.
    #[serde(default, skip_serializing_if = "ExtensionNamespaces::is_absent")]
    extensions: ExtensionNamespaces,
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
const SESSION_FILE_VERSION_V4: u32 = 4;
const SESSION_FILE_VERSION_V5: u32 = 5;
const SESSION_FILE_VERSION_V6: u32 = 6;
const CURRENT_SESSION_FILE_VERSION: u32 = SESSION_FILE_VERSION_V6;
const LEGACY_WORKSPACE_ID: WorkspaceId =
    WorkspaceId::from_u128(0x4749_5443_4f4d_4554_0000_0000_0000_0001);
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
    let Some(mut file) = load_file(path) else {
        return UiSession::default();
    };

    let stored_workspaces = file
        .workspaces
        .take()
        .or_else(|| legacy_workspace_from_projection(&file).map(|workspace| vec![workspace]));
    let workspaces = parse_workspaces(stored_workspaces.unwrap_or_default());
    let (legacy_open_repos, legacy_active_repo) = parse_repos(file.open_repos, file.active_repo);
    // Prefer a workspace with repositories so an empty customized one cannot
    // blank the legacy projection.
    let restorable = || {
        workspaces
            .iter()
            .filter(|workspace| workspace.restore_on_launch)
    };
    let restored_workspace = restorable()
        .filter(|workspace| !workspace.repositories.is_empty())
        .max_by_key(|workspace| workspace.last_activation_order)
        .or_else(|| restorable().max_by_key(|workspace| workspace.last_activation_order));
    let (open_repos, active_repo) = restored_workspace.map_or_else(
        || {
            if workspaces.is_empty() {
                (legacy_open_repos, legacy_active_repo)
            } else {
                (Vec::new(), None)
            }
        },
        |workspace| {
            (
                workspace.repositories.clone(),
                workspace.active_repository.clone(),
            )
        },
    );
    let restored_layout = restored_workspace.map(|workspace| workspace.layout.clone());
    let restored_frame = restored_workspace.and_then(|workspace| workspace.placement.normal_frame);
    // Until the mergetool saves its own size it keeps the shared legacy one.
    let mergetool_window_size = file
        .mergetool_window_width
        .zip(file.mergetool_window_height)
        .or(file.window_width.zip(file.window_height));
    let recent_repos = parse_path_list(file.recent_repos.unwrap_or_default());
    let pinned_repos = parse_path_list(file.pinned_repos.unwrap_or_default());
    let repo_sidebar_collapsed_items =
        parse_path_keyed_string_sets(file.repo_sidebar_collapsed_items.unwrap_or_default());
    let repo_sidebar_pinned_branches =
        parse_path_keyed_string_sets(file.repo_sidebar_pinned_branches.unwrap_or_default());
    UiSession {
        workspaces,
        open_repos,
        active_repo,
        recent_repos,
        recent_documents: parse_path_list(file.recent_documents.unwrap_or_default())
            .into_iter()
            .filter(|p| p.is_absolute())
            .take(50)
            .collect(),
        pinned_repos,
        repo_picker_sort: file.repo_picker_sort,
        repo_picker_collapsed_sections: file.repo_picker_collapsed_sections.unwrap_or_default(),
        repo_sidebar_collapsed_items,
        repo_sidebar_pinned_branches,
        window_width: restored_frame
            .map(|frame| frame.width)
            .or(file.window_width),
        window_height: restored_frame
            .map(|frame| frame.height)
            .or(file.window_height),
        mergetool_window_width: mergetool_window_size.map(|(width, _)| width),
        mergetool_window_height: mergetool_window_size.map(|(_, height)| height),
        sidebar_width: restored_layout
            .as_ref()
            .and_then(|layout| layout.sidebar_width)
            .or(file.sidebar_width),
        details_width: restored_layout
            .as_ref()
            .and_then(|layout| layout.details_width)
            .or(file.details_width),
        sidebar_collapsed: restored_layout
            .as_ref()
            .map(|layout| layout.sidebar_collapsed)
            .or(file.sidebar_collapsed),
        theme_mode: file.theme_mode,
        ui_scale_percent: file.ui_scale_percent,
        window_controls_mode: file.window_controls_mode,
        browser_open_target: file.browser_open_target,
        ui_density: file.ui_density,
        ui_font_size_px: file.ui_font_size_px,
        editor_font_size_px: file.editor_font_size_px,
        markdown_preview_font_size_px: file.markdown_preview_font_size_px,
        ui_font_family: file.ui_font_family,
        editor_font_family: file.editor_font_family,
        use_font_ligatures: file.use_font_ligatures,
        date_time_format: file.date_time_format,
        timezone: file.timezone,
        show_timezone: file.show_timezone,
        change_tracking_view: file.change_tracking_view,
        file_list_layout: file.file_list_layout,
        file_list_sort: file.file_list_sort,
        diff_scroll_sync: file.diff_scroll_sync,
        diff_content_mode: file.diff_content_mode,
        diff_whitespace_mode: file.diff_whitespace_mode,
        diff_view_mode: file.diff_view_mode,
        annotate_enabled: file.annotate_enabled,
        diff_reveal_whitespace_chars: file.diff_reveal_whitespace_chars,
        diff_word_wrap: file.diff_word_wrap,
        diff_tab_size: file.diff_tab_size,
        diff_show_line_numbers: file.diff_show_line_numbers,
        remote_markdown_image_policy: file.remote_markdown_image_policy,
        allowed_remote_protocols: file.allowed_remote_protocols,
        check_for_updates_on_startup: file.check_for_updates_on_startup,
        auto_save_file_edits: file.auto_save_file_edits,
        mergetool_auto_advance: file.mergetool_auto_advance,
        mergetool_collapse_unchanged: file.mergetool_collapse_unchanged,
        mergetool_output_scroll_sync: file.mergetool_output_scroll_sync,
        mergetool_show_line_numbers: file.mergetool_show_line_numbers,
        mergetool_view_three_way: file.mergetool_view_three_way,
        change_tracking_height: restored_layout
            .as_ref()
            .and_then(|layout| layout.change_tracking_height)
            .or(file.change_tracking_height),
        untracked_height: restored_layout
            .as_ref()
            .and_then(|layout| layout.untracked_height)
            .or(file.untracked_height),
        history_branch_names: file.history_branch_names,
        history_show_graph: file.history_show_graph,
        history_show_author: file.history_show_author,
        history_show_date: file.history_show_date,
        history_show_sha: file.history_show_sha,
        terminal_external_mode: file.terminal_external_mode,
        terminal_external_program: file.terminal_external_program,
        terminal_external_args: file.terminal_external_args,
        terminal_action_bar_target: file.terminal_action_bar_target,
        history_show_tags: file.history_show_tags,
        history_verify_commit_signatures: file.history_verify_commit_signatures,
        history_relative_dates: file.history_relative_dates,
        history_highlight_commit_chain: file.history_highlight_commit_chain,
        file_browser_follow_selected_commit: file.file_browser_follow_selected_commit,
        annex_hide_bookkeeping_refs: file.annex_hide_bookkeeping_refs,
        annex_pull_push_on_adjusted: file.annex_pull_push_on_adjusted,
        annex_sync_content: file.annex_sync_content,
        history_tag_fetch_mode: file.history_tag_fetch_mode,
        default_history_mode: file.default_history_mode.map(Into::into),
        commit_push_after_enabled: file.commit_push_after_enabled,
        default_tag_type: file.default_tag_type,
        fetch_prune_deleted_remote_branches: file.fetch_prune_deleted_remote_branches,
        recommend_repo_maintenance: file.recommend_repo_maintenance,
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

/// What a session update decided.
pub(crate) enum SessionUpdate {
    Write,
    Unchanged,
}

/// The session file's one read-modify-write transaction: under the persist
/// lock, load the current file (or an empty one), let `update` change it, stamp
/// the current version, and write it back unless `update` left it unchanged.
///
/// Every writer goes through here, so each one starts from what is on disk
/// now: fields it does not touch, such as extension namespaces, survive every
/// other writer's update, however their calls interleave.
pub(crate) fn update_session_file(
    path: &Path,
    update: impl FnOnce(&mut UiSessionFile) -> SessionUpdate,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(path).unwrap_or_default();
        match update(&mut file) {
            SessionUpdate::Unchanged => Ok(()),
            SessionUpdate::Write => {
                file.version = CURRENT_SESSION_FILE_VERSION;
                persist_to_path(path, &file)
            }
        }
    })
}

/// On-disk version `load_file` last read per path, so the write that follows
/// can skip the backup check's re-read.
static LOADED_SESSION_VERSIONS: OnceLock<Mutex<FxHashMap<PathBuf, u32>>> = OnceLock::new();

fn loaded_session_versions() -> MutexGuard<'static, FxHashMap<PathBuf, u32>> {
    LOADED_SESSION_VERSIONS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

fn loaded_session_version(path: &Path) -> Option<u32> {
    loaded_session_versions().get(path).copied()
}

fn load_file(path: &Path) -> Option<UiSessionFile> {
    let value = fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok());
    let Some(value) = value else {
        loaded_session_versions().remove(path);
        return None;
    };
    let version = value
        .get("version")
        .and_then(|v| v.as_u64())
        .unwrap_or(SESSION_FILE_VERSION_V1 as u64) as u32;
    loaded_session_versions().insert(path.to_path_buf(), version);
    let mut file = match version {
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
        SESSION_FILE_VERSION_V3
        | SESSION_FILE_VERSION_V4
        | SESSION_FILE_VERSION_V5
        | SESSION_FILE_VERSION_V6 => serde_json::from_value::<UiSessionFile>(value).ok(),
        _ => None,
    }?;
    file = migrate_legacy_repo_fetch_prune_setting(file);
    if version < SESSION_FILE_VERSION_V5 {
        file = migrate_pre_v5_default_density(file);
    }
    let enabled = file
        .history_verify_commit_signatures_opt_in
        .unwrap_or(false);
    file.history_verify_commit_signatures = Some(enabled);
    file.history_verify_commit_signatures_opt_in = Some(enabled);
    Some(file)
}

fn persist_to_path(path: &Path, session: &impl Serialize) -> io::Result<()> {
    let contents = serde_json::to_vec(session).expect("serializing session file should succeed");
    preserve_previous_version_session_backup(path, &contents)?;
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

    Some(dirs::state_dir()?.join("session.json"))
}

/// Per-user rendezvous file used by the browser-process broker. Test binaries
/// intentionally receive no path so they cannot touch a developer's live app
/// state; broker unit tests inject an explicit temporary path instead.
pub fn browser_instance_file_path() -> Option<PathBuf> {
    if cfg!(test) || running_under_test_harness() {
        return None;
    }
    Some(dirs::state_dir()?.join("browser-instance.json"))
}

/// Where the filesystem service keeps undo and staging areas, beside the
/// session rather than in a worktree. Test binaries get none, like the session.
pub fn journal_storage_dir() -> Option<PathBuf> {
    if cfg!(test) || running_under_test_harness() {
        return None;
    }
    Some(dirs::state_dir()?.join("journal"))
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

    Some(dirs::data_dir()?.join("themes"))
}

pub use extensions::{
    ExtensionNamespaceError, ExtensionNamespaces, MAX_EXTENSION_NAMESPACE_BYTES,
    check_extension_namespace_size, extension_namespace, extension_namespace_from_path,
    persist_extension_namespace, persist_extension_namespace_to_path,
};
use history_mode::{HistoryModeSetting, HistoryScopeSetting};
use maintenance::{RepoMaintenanceSession, lenient_repo_maintenance};
use parse::*;
use survey::{SurveyPromptSession, current_unix_seconds};
use workspaces::*;

mod documents;
mod extensions;
mod history_mode;
mod maintenance;
mod parse;
mod paths;
mod repos;
mod settings;
mod survey;
mod workspaces;

pub use documents::*;
pub use history_mode::*;
pub use maintenance::{
    MAINTENANCE_CHECK_INTERVAL_SECONDS, MAINTENANCE_SNOOZE_SECONDS, claim_repo_maintenance_check,
    claim_repo_maintenance_check_to_path, persist_repo_maintenance_snooze,
    persist_repo_maintenance_snooze_to_path,
};
pub use paths::*;
pub use repos::*;
pub use settings::*;
pub use survey::*;
pub use workspaces::*;

pub(crate) use history_mode::persist_repo_history_modes_batch_to_path;
pub(crate) use repos::load_repo_session_preferences;
#[cfg(test)]
pub(crate) use repos::load_repo_session_preferences_from_path;

#[cfg(test)]
mod tests;

mod repository_preferences;
pub(crate) use repository_preferences::{
    initialize_repository_preferences, persist_repository_preference_updates,
};
