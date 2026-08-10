use crate::model::{AppState, DefaultTagType, GitLogTagFetchMode, RepoId};
use gitcomet_core::domain::{HistoryMode, LogScope};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::Write;
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum HistoryScopeSetting {
    CurrentBranch,
    AllBranches,
}

impl From<LogScope> for HistoryScopeSetting {
    fn from(value: LogScope) -> Self {
        match value {
            HistoryMode::AllBranches => Self::AllBranches,
            HistoryMode::FullReachable
            | HistoryMode::FirstParent
            | HistoryMode::NoMerges
            | HistoryMode::MergesOnly => Self::CurrentBranch,
        }
    }
}

impl From<HistoryScopeSetting> for LogScope {
    fn from(value: HistoryScopeSetting) -> Self {
        match value {
            HistoryScopeSetting::CurrentBranch => Self::CurrentBranch,
            HistoryScopeSetting::AllBranches => Self::AllBranches,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum HistoryModeSetting {
    FullReachable,
    FirstParent,
    NoMerges,
    MergesOnly,
    AllBranches,
}

impl From<HistoryMode> for HistoryModeSetting {
    fn from(value: HistoryMode) -> Self {
        match value {
            HistoryMode::FullReachable => Self::FullReachable,
            HistoryMode::FirstParent => Self::FirstParent,
            HistoryMode::NoMerges => Self::NoMerges,
            HistoryMode::MergesOnly => Self::MergesOnly,
            HistoryMode::AllBranches => Self::AllBranches,
        }
    }
}

impl From<HistoryModeSetting> for HistoryMode {
    fn from(value: HistoryModeSetting) -> Self {
        match value {
            HistoryModeSetting::FullReachable => Self::FullReachable,
            HistoryModeSetting::FirstParent => Self::FirstParent,
            HistoryModeSetting::NoMerges => Self::NoMerges,
            HistoryModeSetting::MergesOnly => Self::MergesOnly,
            HistoryModeSetting::AllBranches => Self::AllBranches,
        }
    }
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
    virtual_branches: Option<BTreeMap<String, VirtualBranchesSessionFile>>,
    survey_prompt: Option<SurveyPromptSession>,
}

/// One persisted virtual branch (prototype workspace): the accounting of which
/// worktree paths belong to the branch plus any parked unified patch.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct VirtualBranchSessionFile {
    pub id: u64,
    pub name: String,
    /// Repo-relative paths assigned to the branch.
    pub paths: Vec<String>,
    /// Whether the branch's changes are present in the worktree. Persisted so
    /// unapplied branches (parked patches) restore as unapplied.
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_patch: Option<String>,
}

/// Per-repo persisted virtual branch workspace: the next id allocator plus the
/// branch list.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct VirtualBranchesSessionFile {
    pub next_id: u64,
    pub branches: Vec<VirtualBranchSessionFile>,
}

impl From<&gitcomet_core::domain::VirtualBranch> for VirtualBranchSessionFile {
    fn from(branch: &gitcomet_core::domain::VirtualBranch) -> Self {
        Self {
            id: branch.id,
            name: branch.name.to_string(),
            paths: branch
                .paths
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            applied: branch.applied,
            stored_patch: branch.stored_patch.as_deref().map(ToOwned::to_owned),
        }
    }
}

impl From<&VirtualBranchSessionFile> for gitcomet_core::domain::VirtualBranch {
    fn from(file: &VirtualBranchSessionFile) -> Self {
        use gitcomet_core::domain::VirtualBranch;
        VirtualBranch {
            id: file.id,
            name: file.name.clone().into(),
            paths: file
                .paths
                .iter()
                .map(std::path::PathBuf::from)
                .collect(),
            applied: file.applied,
            stored_patch: file.stored_patch.clone().map(Into::into),
            // Restored branches are never mid-operation.
            pending: false,
        }
    }
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct SurveyPromptSession {
    survey_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    opened_at_unix_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postponed_until_unix_seconds: Option<u64>,
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
    pub(crate) virtual_branches: BTreeMap<String, VirtualBranchesSessionFile>,
}

pub(crate) fn load_repo_session_preferences() -> RepoSessionPreferences {
    let Some(session_file_path) = default_session_file_path() else {
        return RepoSessionPreferences::default();
    };
    load_repo_session_preferences_from_path(&session_file_path)
}

pub(crate) fn load_repo_session_preferences_from_path(
    session_file_path: &Path,
) -> RepoSessionPreferences {
    let Some(file) = load_file(session_file_path) else {
        return RepoSessionPreferences::default();
    };

    RepoSessionPreferences {
        default_history_mode: file.default_history_mode.map(Into::into),
        repo_history_modes: file
            .repo_history_modes
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect(),
        repo_history_scopes: file
            .repo_history_scopes
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect(),
        repo_history_author_filters: file.repo_history_author_filters.unwrap_or_default(),
        repo_fetch_prune_deleted_remote_tracking_branches: file
            .repo_fetch_prune_deleted_remote_tracking_branches
            .unwrap_or_default(),
        virtual_branches: file.virtual_branches.unwrap_or_default(),
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionReposSnapshot {
    pub open_repos: Arc<[Arc<str>]>,
    pub active_repo_index: Option<usize>,
}

#[derive(Clone, Debug, Default)]
struct CachedSessionReposSnapshot {
    repo_ids: SmallVec<[RepoId; 24]>,
    repo_keys: SmallVec<[Arc<str>; 24]>,
    dedup_indexes_by_repo: SmallVec<[usize; 24]>,
    open_repos: Arc<[Arc<str>]>,
}

thread_local! {
    static SESSION_REPOS_SNAPSHOT_CACHE: RefCell<Option<CachedSessionReposSnapshot>> = const { RefCell::new(None) };
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

fn snapshot_repos_from_cache(state: &AppState) -> Option<SessionReposSnapshot> {
    SESSION_REPOS_SNAPSHOT_CACHE.with(|cache| {
        let cache = cache.borrow();
        let cached = cache.as_ref()?;
        if cached.repo_ids.len() != state.repos.len() {
            return None;
        }

        let mut active_repo_index = None;
        for (repo_ix, repo) in state.repos.iter().enumerate() {
            if cached.repo_ids[repo_ix] != repo.id
                || !Arc::ptr_eq(&cached.repo_keys[repo_ix], repo.session_workdir_key())
            {
                return None;
            }
            if active_repo_index.is_none() && Some(repo.id) == state.active_repo {
                active_repo_index = Some(cached.dedup_indexes_by_repo[repo_ix]);
            }
        }

        Some(SessionReposSnapshot {
            open_repos: Arc::clone(&cached.open_repos),
            active_repo_index,
        })
    })
}

pub fn snapshot_repos_from_state(state: &AppState) -> SessionReposSnapshot {
    if let Some(snapshot) = snapshot_repos_from_cache(state) {
        return snapshot;
    }

    // Repo switches rarely change the open-tab order, so cache the last exact repo sequence and
    // reuse its dedup map on steady-state switches. When the sequence changes, rebuild once with
    // a linear scan over the small user-scale repo list.
    let mut repo_ids = SmallVec::<[RepoId; 24]>::with_capacity(state.repos.len());
    let mut repo_keys = SmallVec::<[Arc<str>; 24]>::with_capacity(state.repos.len());
    let mut unique_keys = SmallVec::<[Arc<str>; 24]>::new();
    let mut dedup_indexes_by_repo = SmallVec::<[usize; 24]>::with_capacity(state.repos.len());
    let active_repo_id = state.active_repo;
    let mut active_repo_index = None;

    for repo in &state.repos {
        repo_ids.push(repo.id);
        let key = repo.session_workdir_key();
        repo_keys.push(Arc::clone(key));

        let unique_ix = if let Some(ix) = unique_keys
            .iter()
            .position(|seen| seen.as_ref() == key.as_ref())
        {
            ix
        } else {
            unique_keys.push(Arc::clone(key));
            unique_keys.len() - 1
        };
        dedup_indexes_by_repo.push(unique_ix);
        if active_repo_index.is_none() && Some(repo.id) == active_repo_id {
            active_repo_index = Some(unique_ix);
        }
    }

    let open_repos: Arc<[Arc<str>]> = unique_keys.into_vec().into();
    SESSION_REPOS_SNAPSHOT_CACHE.with(|cache| {
        *cache.borrow_mut() = Some(CachedSessionReposSnapshot {
            repo_ids,
            repo_keys,
            dedup_indexes_by_repo,
            open_repos: Arc::clone(&open_repos),
        });
    });

    SessionReposSnapshot {
        open_repos,
        active_repo_index,
    }
}

pub fn persist_from_state(state: &AppState) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };

    let snapshot = snapshot_repos_from_state(state);
    persist_repos_snapshot_to_path(&snapshot, &path)
}

pub fn persist_from_state_to_path(state: &AppState, path: &Path) -> io::Result<()> {
    let snapshot = snapshot_repos_from_state(state);
    persist_repos_snapshot_to_path(&snapshot, path)
}

pub fn persist_repos_snapshot(snapshot: &SessionReposSnapshot) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    persist_repos_snapshot_to_path(snapshot, &path)
}

pub fn persist_repos_snapshot_to_path(
    snapshot: &SessionReposSnapshot,
    path: &Path,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        file.open_repos = snapshot
            .open_repos
            .iter()
            .map(|path| path.to_string())
            .collect();
        file.active_repo = snapshot
            .active_repo_index
            .and_then(|ix| snapshot.open_repos.get(ix))
            .map(|path| path.to_string());

        persist_to_path(path, &file)
    })
}

/// Moves `value` to the front of an MRU list, dropping any earlier copy of it
/// and holding the list to [`MAX_RECENT_REPOS`]. The cap lives here alone so
/// the session file and the in-memory caches the UI shows can never disagree
/// about how long the list is.
fn promote_within_recents_cap<T: PartialEq>(list: &mut Vec<T>, value: T) {
    list.retain(|existing| existing != &value);
    list.insert(0, value);
    list.truncate(MAX_RECENT_REPOS);
}

/// [`promote_within_recents_cap`] for a caller holding its own copy of what
/// [`UiSession::recent_repos`] last returned: applies one recents bump to that
/// copy so it still matches the file after [`persist_recent_repo`] writes it.
pub fn promote_recent_repo(recents: &mut Vec<PathBuf>, workdir: &Path) {
    promote_within_recents_cap(recents, workdir.to_path_buf());
}

pub fn persist_recent_repo(workdir: &Path) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    persist_recent_repo_to_path(workdir, &path)
}

/// Storage key for a repository path in the recents list.
///
/// Canonicalized so the key matches the workdir the store holds for an open
/// repository, which is canonicalized on open (see
/// `gitcomet_state::store::canonicalize_path`). The repo picker relies on plain
/// equality between the two to keep a still-open repository out of the
/// "recently closed" section; on macOS, where the temp and home directories are
/// reached through symlinks, an uncanonicalized key would compare unequal to the
/// very same directory and the repository would be listed twice.
///
/// Falls back to the path as given when it cannot be canonicalized, so a
/// repository that has since been deleted or unmounted still round-trips.
///
/// That fallback is one-way: once the directory is gone the canonical form it
/// was stored under can no longer be reconstructed from the path alone. Removal
/// therefore normalizes the *stored* side too rather than relying on this key
/// alone -- see [`remove_recent_repo_to_path`].
fn recent_repo_storage_key(workdir: &Path) -> String {
    path_storage_key(&gitcomet_core::path_utils::canonicalize_or_original(
        workdir.to_path_buf(),
    ))
}

pub fn persist_recent_repo_to_path(workdir: &Path, session_file_path: &Path) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;

        let workdir_key = recent_repo_storage_key(workdir);
        let raw_key = path_storage_key(workdir);
        let recent_repos = file.recent_repos.get_or_insert_with(Vec::new);
        // Blanks go, and a key a hand-edited file padded is normalized in place
        // so the promotion below still recognizes it as the same repository.
        // The uncanonicalized form an older build wrote goes too, so re-opening
        // a repository heals the list instead of duplicating it.
        recent_repos.retain_mut(|path| {
            let trimmed = path.trim();
            if trimmed.is_empty() || trimmed == raw_key {
                return false;
            }
            if trimmed.len() != path.len() {
                *path = trimmed.to_owned();
            }
            true
        });
        promote_within_recents_cap(recent_repos, workdir_key);

        persist_to_path(session_file_path, &file)
    })
}

pub fn remove_recent_repo(workdir: &Path) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    remove_recent_repo_to_path(workdir, &path)
}

pub fn remove_recent_repo_to_path(workdir: &Path, session_file_path: &Path) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;

        // Must key exactly as `persist_recent_repo_to_path` does, or removal
        // silently misses entries written in the other form.
        let workdir_key = recent_repo_storage_key(workdir);
        let raw_key = path_storage_key(workdir);
        let Some(recent_repos) = file.recent_repos.as_mut() else {
            return Ok(());
        };
        // `raw_key` also clears entries left by older builds, which stored the
        // path uncanonicalized. Entries are normalized on their own side as
        // well, so an entry and a caller that spell the same directory
        // differently -- one through a symlink, one not -- still match: keying
        // off `workdir` alone cannot bridge that once the directory is gone,
        // because `canonicalize` no longer resolves it.
        recent_repos.retain(|path| {
            let path = path.trim();
            if path == workdir_key || path == raw_key {
                return false;
            }
            // Through the storage-key decoder, not `Path::new`: a non-UTF-8
            // workdir is stored hex-encoded, and canonicalizing that encoding
            // as a literal path would quietly never match.
            let decoded = path_from_storage_key(path);
            // Only absolute entries are resolved. A relative one -- which only
            // a hand-edited file can produce -- would canonicalize against the
            // process working directory and could match a repository the user
            // never asked to forget.
            if !decoded.is_absolute() {
                return true;
            }
            let normalized = recent_repo_storage_key(&decoded);
            normalized != workdir_key && normalized != raw_key
        });

        persist_to_path(session_file_path, &file)
    })
}

pub fn persist_pinned_repo(workdir: &Path) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    persist_pinned_repo_to_path(workdir, &path)
}

/// Appends a repository to the pin list. Unlike the recents, pins keep the
/// order the user created them in and are never capped — they leave the list
/// only when the user unpins them. Pinning something already pinned therefore
/// leaves it where it is rather than moving it to the end.
pub fn persist_pinned_repo_to_path(workdir: &Path, session_file_path: &Path) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;

        let workdir_key = path_storage_key(workdir);
        let pinned_repos = file.pinned_repos.get_or_insert_with(Vec::new);
        pinned_repos.retain(|path| !path.trim().is_empty());
        if !pinned_repos.iter().any(|path| path.trim() == workdir_key) {
            pinned_repos.push(workdir_key);
        }

        persist_to_path(session_file_path, &file)
    })
}

pub fn remove_pinned_repo(workdir: &Path) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    remove_pinned_repo_to_path(workdir, &path)
}

pub fn remove_pinned_repo_to_path(workdir: &Path, session_file_path: &Path) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;

        let workdir_key = path_storage_key(workdir);
        let Some(pinned_repos) = file.pinned_repos.as_mut() else {
            return Ok(());
        };
        pinned_repos.retain(|path| path.trim() != workdir_key);

        persist_to_path(session_file_path, &file)
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiSettings {
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub sidebar_width: Option<u32>,
    pub details_width: Option<u32>,
    pub sidebar_collapsed: Option<bool>,
    pub repo_sidebar_collapsed_items: Option<BTreeMap<PathBuf, BTreeSet<String>>>,
    pub repo_sidebar_pinned_branches: Option<BTreeMap<PathBuf, BTreeSet<String>>>,
    pub theme_mode: Option<String>,
    pub ui_scale_percent: Option<u32>,
    pub ui_font_family: Option<String>,
    pub editor_font_family: Option<String>,
    pub use_font_ligatures: Option<bool>,
    pub date_time_format: Option<String>,
    pub timezone: Option<String>,
    pub show_timezone: Option<bool>,
    pub change_tracking_view: Option<String>,
    pub repo_picker_sort: Option<String>,
    /// Whole replacement set — the repository picker owns it and always writes
    /// every collapsed section it knows about.
    pub repo_picker_collapsed_sections: Option<BTreeSet<String>>,
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
    pub git_executable_path: Option<Option<PathBuf>>,
    pub external_code_editor: Option<Option<ExternalCodeEditorSetting>>,
}

pub fn persist_ui_settings(settings: UiSettings) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    persist_ui_settings_to_path(settings, &path)
}

pub fn persist_ui_settings_to_path(settings: UiSettings, path: &Path) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        if settings.window_width.is_some() && settings.window_height.is_some() {
            file.window_width = settings.window_width;
            file.window_height = settings.window_height;
        }
        if let Some(w) = settings.sidebar_width {
            file.sidebar_width = Some(w);
        }
        if let Some(w) = settings.details_width {
            file.details_width = Some(w);
        }
        if let Some(collapsed) = settings.sidebar_collapsed {
            file.sidebar_collapsed = Some(collapsed);
        }
        if let Some(items) = settings.repo_sidebar_collapsed_items {
            let items = path_keyed_string_sets_to_storage(items);
            file.repo_sidebar_collapsed_items = (!items.is_empty()).then_some(items);
        }
        if let Some(items) = settings.repo_sidebar_pinned_branches {
            let items = path_keyed_string_sets_to_storage(items);
            file.repo_sidebar_pinned_branches = (!items.is_empty()).then_some(items);
        }
        if let Some(theme_mode) = settings.theme_mode {
            file.theme_mode = Some(theme_mode);
        }
        if let Some(percent) = settings.ui_scale_percent {
            file.ui_scale_percent = Some(percent);
        }
        if let Some(font_family) = settings.ui_font_family {
            file.ui_font_family = Some(font_family);
        }
        if let Some(font_family) = settings.editor_font_family {
            file.editor_font_family = Some(font_family);
        }
        if let Some(value) = settings.use_font_ligatures {
            file.use_font_ligatures = Some(value);
        }
        if let Some(fmt) = settings.date_time_format {
            file.date_time_format = Some(fmt);
        }
        if let Some(tz) = settings.timezone {
            file.timezone = Some(tz);
        }
        if let Some(value) = settings.show_timezone {
            file.show_timezone = Some(value);
        }
        if let Some(value) = settings.change_tracking_view {
            file.change_tracking_view = Some(value);
        }
        if let Some(value) = settings.repo_picker_sort {
            file.repo_picker_sort = Some(value);
        }
        // Owned by the repository picker (`repo_picker::persist_collapsed_sections`).
        if let Some(value) = settings.repo_picker_collapsed_sections {
            file.repo_picker_collapsed_sections = Some(value);
        }
        if let Some(value) = settings.diff_scroll_sync {
            file.diff_scroll_sync = Some(value);
        }
        if let Some(value) = settings.diff_content_mode {
            file.diff_content_mode = Some(value);
        }
        if let Some(value) = settings.diff_whitespace_mode {
            file.diff_whitespace_mode = Some(value);
        }
        if let Some(value) = settings.diff_view_mode {
            file.diff_view_mode = Some(value);
        }
        if let Some(value) = settings.annotate_enabled {
            file.annotate_enabled = Some(value);
        }
        if let Some(value) = settings.diff_reveal_whitespace_chars {
            file.diff_reveal_whitespace_chars = Some(value);
        }
        if let Some(value) = settings.mergetool_auto_advance {
            file.mergetool_auto_advance = Some(value);
        }
        if let Some(value) = settings.mergetool_collapse_unchanged {
            file.mergetool_collapse_unchanged = Some(value);
        }
        if let Some(value) = settings.mergetool_output_scroll_sync {
            file.mergetool_output_scroll_sync = Some(value);
        }
        if let Some(value) = settings.mergetool_show_line_numbers {
            file.mergetool_show_line_numbers = Some(value);
        }
        if let Some(value) = settings.mergetool_view_three_way {
            file.mergetool_view_three_way = Some(value);
        }
        if let Some(value) = settings.diff_word_wrap {
            file.diff_word_wrap = Some(value);
        }
        if let Some(value) = settings.auto_save_file_edits {
            file.auto_save_file_edits = Some(value);
        }
        if let Some(value) = settings.diff_show_line_numbers {
            file.diff_show_line_numbers = Some(value);
        }
        if let Some(value) = settings.change_tracking_height {
            file.change_tracking_height = Some(value);
        }
        if let Some(value) = settings.untracked_height {
            file.untracked_height = Some(value);
        }
        if let Some(value) = settings.history_show_graph {
            file.history_show_graph = Some(value);
        }
        if let Some(value) = settings.history_show_author {
            file.history_show_author = Some(value);
        }
        if let Some(value) = settings.history_show_date {
            file.history_show_date = Some(value);
        }
        if let Some(value) = settings.history_show_sha {
            file.history_show_sha = Some(value);
        }
        if let Some(value) = settings.terminal_external_mode {
            file.terminal_external_mode = Some(value);
        }
        if let Some(value) = settings.terminal_external_program {
            file.terminal_external_program = Some(value);
        }
        if let Some(value) = settings.terminal_external_args {
            let values = value
                .into_iter()
                .map(|arg| arg.trim().to_string())
                .filter(|arg| !arg.is_empty())
                .collect::<Vec<_>>();
            file.terminal_external_args = Some(values);
        }
        if let Some(value) = settings.terminal_action_bar_target {
            file.terminal_action_bar_target = Some(value);
        }
        if let Some(value) = settings.history_show_tags {
            file.history_show_tags = Some(value);
        }
        if let Some(value) = settings.history_highlight_commit_chain {
            file.history_highlight_commit_chain = Some(value);
        }
        if let Some(value) = settings.history_relative_dates {
            file.history_relative_dates = Some(value);
        }
        if let Some(value) = settings.history_tag_fetch_mode {
            file.history_tag_fetch_mode = Some(value);
        }
        if let Some(value) = settings.default_history_mode {
            file.default_history_mode = Some(value.into());
        }
        if let Some(value) = settings.commit_push_after_enabled {
            file.commit_push_after_enabled = Some(value);
        }
        if let Some(value) = settings.default_tag_type {
            file.default_tag_type = Some(value);
        }
        if let Some(path) = settings.git_executable_path {
            file.git_executable_path = path.map(|path| path_storage_key(&path));
        }
        if let Some(editor) = settings.external_code_editor {
            file.external_code_editor = editor.map(external_code_editor_to_file);
        }

        persist_to_path(path, &file)
    })
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

pub fn load_default_history_mode() -> Option<HistoryMode> {
    let session_file_path = default_session_file_path()?;
    load_default_history_mode_from_path(&session_file_path)
}

pub fn load_default_history_mode_from_path(session_file_path: &Path) -> Option<HistoryMode> {
    let file = load_file(session_file_path)?;
    file.default_history_mode.map(Into::into)
}

pub fn load_repo_history_mode(workdir: &Path) -> Option<HistoryMode> {
    let session_file_path = default_session_file_path()?;
    load_repo_history_mode_from_path(workdir, &session_file_path)
}

pub fn load_repo_history_mode_from_path(
    workdir: &Path,
    session_file_path: &Path,
) -> Option<HistoryMode> {
    let workdir_key = path_storage_key(workdir);
    let file = load_file(session_file_path)?;
    let modes = file.repo_history_modes?;
    modes.get(&workdir_key).copied().map(Into::into)
}

pub fn load_repo_history_modes() -> BTreeMap<String, HistoryMode> {
    let Some(session_file_path) = default_session_file_path() else {
        return BTreeMap::new();
    };
    load_repo_history_modes_from_path(&session_file_path)
}

pub fn load_repo_history_modes_from_path(
    session_file_path: &Path,
) -> BTreeMap<String, HistoryMode> {
    let Some(file) = load_file(session_file_path) else {
        return BTreeMap::new();
    };
    file.repo_history_modes
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.into()))
        .collect()
}

pub fn persist_repo_history_mode(workdir: &Path, mode: HistoryMode) -> io::Result<()> {
    let Some(session_file_path) = default_session_file_path() else {
        return Ok(());
    };
    persist_repo_history_mode_to_path(workdir, mode, &session_file_path)
}

fn repo_history_mode_setting_from_file(
    file: &UiSessionFile,
    workdir: &Path,
) -> Option<HistoryModeSetting> {
    file.repo_history_modes.as_ref().and_then(|modes| {
        workdir
            .to_str()
            .and_then(|path| modes.get(path).copied())
            .or_else(|| {
                let workdir_key = path_storage_key(workdir);
                modes.get(&workdir_key).copied()
            })
    })
}

pub fn persist_repo_history_mode_to_path(
    workdir: &Path,
    mode: HistoryMode,
    session_file_path: &Path,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        let mode = HistoryModeSetting::from(mode);

        if repo_history_mode_setting_from_file(&file, workdir)
            .is_some_and(|existing| existing == mode)
        {
            return Ok(());
        }

        file.version = CURRENT_SESSION_FILE_VERSION;
        let workdir_key = path_storage_key(workdir);
        file.repo_history_modes
            .get_or_insert_with(BTreeMap::new)
            .insert(workdir_key, mode);

        persist_to_path(session_file_path, &file)
    })
}

pub(crate) fn persist_repo_history_modes_batch_to_path(
    updates: &[(PathBuf, HistoryMode)],
    session_file_path: &Path,
) -> io::Result<()> {
    if updates.is_empty() {
        return Ok(());
    }

    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        let mut changed = false;

        for (workdir, mode) in updates {
            let mode = HistoryModeSetting::from(*mode);
            if repo_history_mode_setting_from_file(&file, workdir)
                .is_some_and(|existing| existing == mode)
            {
                continue;
            }

            let workdir_key = path_storage_key(workdir);
            file.repo_history_modes
                .get_or_insert_with(BTreeMap::new)
                .insert(workdir_key, mode);
            changed = true;
        }

        if !changed {
            return Ok(());
        }

        file.version = CURRENT_SESSION_FILE_VERSION;
        persist_to_path(session_file_path, &file)
    })
}

pub fn load_repo_history_scope(workdir: &Path) -> Option<LogScope> {
    let session_file_path = default_session_file_path()?;
    load_repo_history_scope_from_path(workdir, &session_file_path)
}

pub fn load_repo_history_scope_from_path(
    workdir: &Path,
    session_file_path: &Path,
) -> Option<LogScope> {
    let workdir_key = path_storage_key(workdir);
    let file = load_file(session_file_path)?;
    let scopes = file.repo_history_scopes?;
    scopes.get(&workdir_key).copied().map(Into::into)
}

pub fn load_repo_history_scopes() -> BTreeMap<String, LogScope> {
    let Some(session_file_path) = default_session_file_path() else {
        return BTreeMap::new();
    };
    load_repo_history_scopes_from_path(&session_file_path)
}

pub fn load_repo_history_scopes_from_path(session_file_path: &Path) -> BTreeMap<String, LogScope> {
    let Some(file) = load_file(session_file_path) else {
        return BTreeMap::new();
    };
    file.repo_history_scopes
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.into()))
        .collect()
}

pub fn persist_repo_history_scope(workdir: &Path, scope: LogScope) -> io::Result<()> {
    let Some(session_file_path) = default_session_file_path() else {
        return Ok(());
    };
    persist_repo_history_scope_to_path(workdir, scope, &session_file_path)
}

pub fn persist_repo_history_scope_to_path(
    workdir: &Path,
    scope: LogScope,
    session_file_path: &Path,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        let scope = HistoryScopeSetting::from(scope);

        if let Some(existing_scope) = file.repo_history_scopes.as_ref().and_then(|scopes| {
            workdir
                .to_str()
                .and_then(|path| scopes.get(path).copied())
                .or_else(|| {
                    let workdir_key = path_storage_key(workdir);
                    scopes.get(&workdir_key).copied()
                })
        }) && existing_scope == scope
        {
            return Ok(());
        }

        file.version = CURRENT_SESSION_FILE_VERSION;
        let workdir_key = path_storage_key(workdir);
        file.repo_history_scopes
            .get_or_insert_with(BTreeMap::new)
            .insert(workdir_key, scope);

        persist_to_path(session_file_path, &file)
    })
}

/// Persists the history author filter for `workdir`. `None` clears the stored
/// filter; a `Some(Some(_))` stores the active author.
pub fn persist_repo_history_author_filter_to_path(
    workdir: &Path,
    author: Option<&str>,
    session_file_path: &Path,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        let stored = file
            .repo_history_author_filters
            .get_or_insert_with(BTreeMap::new);
        let workdir_key = path_storage_key(workdir);
        let existing = stored.get(&workdir_key).cloned().flatten();
        if existing == author.map(ToOwned::to_owned) {
            return Ok(());
        }
        if let Some(author) = author {
            stored.insert(workdir_key, Some(author.to_owned()));
        } else {
            stored.remove(&workdir_key);
        }
        file.version = CURRENT_SESSION_FILE_VERSION;
        persist_to_path(session_file_path, &file)
    })
}

/// Persists the virtual branch workspace for `workdir`. An empty branch list
/// removes the stored workspace for the repo (nothing to restore).
pub fn persist_virtual_branches_to_path(
    workdir: &Path,
    data: &VirtualBranchesSessionFile,
    session_file_path: &Path,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        let workdir_key = path_storage_key(workdir);
        let stored = file.virtual_branches.get_or_insert_with(BTreeMap::new);
        let unchanged = stored.get(&workdir_key) == Some(data);
        if unchanged {
            return Ok(());
        }
        if data.branches.is_empty() {
            stored.remove(&workdir_key);
            if stored.is_empty() {
                file.virtual_branches = None;
            }
        } else {
            stored.insert(workdir_key, data.clone());
        }
        file.version = CURRENT_SESSION_FILE_VERSION;
        persist_to_path(session_file_path, &file)
    })
}

pub fn load_repo_fetch_prune_deleted_remote_tracking_branches(workdir: &Path) -> Option<bool> {
    let session_file_path = default_session_file_path()?;
    load_repo_fetch_prune_deleted_remote_tracking_branches_from_path(workdir, &session_file_path)
}

pub fn load_repo_fetch_prune_deleted_remote_tracking_branches_from_path(
    workdir: &Path,
    session_file_path: &Path,
) -> Option<bool> {
    let workdir_key = path_storage_key(workdir);
    let file = load_file(session_file_path)?;
    let settings = file.repo_fetch_prune_deleted_remote_tracking_branches?;
    settings.get(&workdir_key).copied()
}

pub fn load_repo_fetch_prune_deleted_remote_tracking_branches_by_repo() -> BTreeMap<String, bool> {
    let Some(session_file_path) = default_session_file_path() else {
        return BTreeMap::new();
    };
    load_repo_fetch_prune_deleted_remote_tracking_branches_by_repo_from_path(&session_file_path)
}

pub fn load_repo_fetch_prune_deleted_remote_tracking_branches_by_repo_from_path(
    session_file_path: &Path,
) -> BTreeMap<String, bool> {
    let Some(file) = load_file(session_file_path) else {
        return BTreeMap::new();
    };
    file.repo_fetch_prune_deleted_remote_tracking_branches
        .unwrap_or_default()
}

pub fn persist_repo_fetch_prune_deleted_remote_tracking_branches(
    workdir: &Path,
    enabled: bool,
) -> io::Result<()> {
    let Some(session_file_path) = default_session_file_path() else {
        return Ok(());
    };
    persist_repo_fetch_prune_deleted_remote_tracking_branches_to_path(
        workdir,
        enabled,
        &session_file_path,
    )
}

pub fn persist_repo_fetch_prune_deleted_remote_tracking_branches_to_path(
    workdir: &Path,
    enabled: bool,
    session_file_path: &Path,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        let workdir_key = path_storage_key(workdir);
        file.repo_fetch_prune_deleted_remote_tracking_branches
            .get_or_insert_with(BTreeMap::new)
            .insert(workdir_key, enabled);

        persist_to_path(session_file_path, &file)
    })
}

pub fn should_show_survey_prompt(survey_id: &str) -> bool {
    let Some(session_file_path) = default_session_file_path() else {
        return false;
    };
    should_show_survey_prompt_from_path(&session_file_path, survey_id, current_unix_seconds())
}

pub fn should_show_survey_prompt_from_path(
    session_file_path: &Path,
    survey_id: &str,
    now_unix_seconds: u64,
) -> bool {
    let Some(file) = load_file(session_file_path) else {
        return false;
    };
    if !has_recorded_session_repository(&file) {
        return false;
    }

    let Some(prompt) = file.survey_prompt else {
        return true;
    };
    if prompt.survey_id != survey_id {
        return true;
    }
    if prompt.opened_at_unix_seconds.is_some() {
        return false;
    }

    prompt
        .postponed_until_unix_seconds
        .is_none_or(|postponed_until| postponed_until <= now_unix_seconds)
}

pub fn persist_survey_prompt_opened(survey_id: &str) -> io::Result<()> {
    let Some(session_file_path) = default_session_file_path() else {
        return Ok(());
    };
    persist_survey_prompt_opened_to_path(&session_file_path, survey_id, current_unix_seconds())
}

pub fn persist_survey_prompt_opened_to_path(
    session_file_path: &Path,
    survey_id: &str,
    now_unix_seconds: u64,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        file.survey_prompt = Some(SurveyPromptSession {
            survey_id: survey_id.to_string(),
            opened_at_unix_seconds: Some(now_unix_seconds),
            postponed_until_unix_seconds: None,
        });

        persist_to_path(session_file_path, &file)
    })
}

pub fn persist_survey_prompt_postponed(survey_id: &str, postpone_seconds: u64) -> io::Result<()> {
    let Some(session_file_path) = default_session_file_path() else {
        return Ok(());
    };
    persist_survey_prompt_postponed_to_path(
        &session_file_path,
        survey_id,
        postpone_seconds,
        current_unix_seconds(),
    )
}

pub fn persist_survey_prompt_postponed_to_path(
    session_file_path: &Path,
    survey_id: &str,
    postpone_seconds: u64,
    now_unix_seconds: u64,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        file.survey_prompt = Some(SurveyPromptSession {
            survey_id: survey_id.to_string(),
            opened_at_unix_seconds: None,
            postponed_until_unix_seconds: Some(now_unix_seconds.saturating_add(postpone_seconds)),
        });

        persist_to_path(session_file_path, &file)
    })
}

fn current_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// Survey eligibility only needs a usage signal. A recorded repository means the user has used
// GitComet before; it does not need to prove the repository still exists on disk.
fn has_recorded_session_repository(file: &UiSessionFile) -> bool {
    if file.open_repos.iter().any(|path| !path.trim().is_empty()) {
        return true;
    }
    if file
        .active_repo
        .as_deref()
        .is_some_and(|path| !path.trim().is_empty())
    {
        return true;
    }
    if file
        .recent_repos
        .as_ref()
        .is_some_and(|paths| paths.iter().any(|path| !path.trim().is_empty()))
    {
        return true;
    }
    false
}

fn parse_repos(
    open_repos_raw: Vec<String>,
    active_repo_raw: Option<String>,
) -> (Vec<PathBuf>, Option<PathBuf>) {
    let open_repos = parse_path_list(open_repos_raw);
    let seen: FxHashSet<PathBuf> = open_repos.iter().cloned().collect();

    let active_repo = active_repo_raw
        .as_deref()
        .and_then(|p| {
            let p = p.trim();
            if p.is_empty() {
                None
            } else {
                Some(path_from_storage_key(p))
            }
        })
        .filter(|active| seen.contains(active));

    (open_repos, active_repo)
}

fn parse_path_list(paths_raw: Vec<String>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::with_capacity(paths_raw.len());
    let mut seen: FxHashSet<PathBuf> = FxHashSet::default();
    for raw in paths_raw {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let path = path_from_storage_key(raw);
        if !seen.insert(path.clone()) {
            continue;
        }
        paths.push(path);
    }
    paths
}

fn parse_path_keyed_string_sets(
    paths_raw: BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<PathBuf, BTreeSet<String>> {
    let mut paths: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for (raw_path, values) in paths_raw {
        let raw_path = raw_path.trim();
        if raw_path.is_empty() {
            continue;
        }
        let path = path_from_storage_key(raw_path);
        let entry = paths.entry(path).or_default();
        for value in values {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            entry.insert(value.to_string());
        }
    }
    paths.retain(|_, values| !values.is_empty());
    paths
}

fn path_keyed_string_sets_to_storage(
    paths: BTreeMap<PathBuf, BTreeSet<String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut stored = BTreeMap::new();
    for (path, values) in paths {
        let mut normalized = BTreeSet::new();
        for value in values {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            normalized.insert(value.to_string());
        }
        if normalized.is_empty() {
            continue;
        }
        stored.insert(path_storage_key(&path), normalized);
    }
    stored
}

fn non_empty_string(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn external_code_editor_from_file(
    setting: Option<ExternalCodeEditorSettingFile>,
) -> Option<ExternalCodeEditorSetting> {
    match setting? {
        ExternalCodeEditorSettingFile::Detected { id, path } => {
            let path = path.trim();
            if path.is_empty() {
                return None;
            }
            Some(ExternalCodeEditorSetting::Detected {
                id: non_empty_string(id)?,
                path: path_from_storage_key(path),
            })
        }
        ExternalCodeEditorSettingFile::Custom {
            executable,
            arguments,
        } => Some(ExternalCodeEditorSetting::Custom {
            executable: path_from_storage_key(executable.trim()),
            arguments: arguments.and_then(non_empty_string),
        }),
    }
}

fn external_code_editor_to_file(
    setting: ExternalCodeEditorSetting,
) -> ExternalCodeEditorSettingFile {
    match setting {
        ExternalCodeEditorSetting::Detected { id, path } => {
            ExternalCodeEditorSettingFile::Detected {
                id,
                path: path_storage_key(&path),
            }
        }
        ExternalCodeEditorSetting::Custom {
            executable,
            arguments,
        } => ExternalCodeEditorSettingFile::Custom {
            executable: path_storage_key(&executable),
            arguments: arguments.and_then(non_empty_string),
        },
    }
}

fn sanitize_ui_scale_percent(percent: Option<u32>) -> u32 {
    percent
        .unwrap_or(DEFAULT_UI_SCALE_PERCENT)
        .clamp(MIN_UI_SCALE_PERCENT, MAX_UI_SCALE_PERCENT)
}

fn migrate_scaled_dimension_to_design_units(
    value: Option<u32>,
    ui_scale_percent: Option<u32>,
) -> Option<u32> {
    let value = value? as f32;
    let factor =
        sanitize_ui_scale_percent(ui_scale_percent) as f32 / DEFAULT_UI_SCALE_PERCENT as f32;
    let design_units = (value / factor).round();
    (design_units.is_finite() && design_units >= 1.0).then_some(design_units as u32)
}

fn migrate_v2_file(mut file: UiSessionFile) -> UiSessionFile {
    let ui_scale_percent = file.ui_scale_percent;
    file.version = CURRENT_SESSION_FILE_VERSION;
    file.sidebar_width =
        migrate_scaled_dimension_to_design_units(file.sidebar_width, ui_scale_percent);
    file.details_width =
        migrate_scaled_dimension_to_design_units(file.details_width, ui_scale_percent);
    file.change_tracking_height =
        migrate_scaled_dimension_to_design_units(file.change_tracking_height, ui_scale_percent);
    file.untracked_height =
        migrate_scaled_dimension_to_design_units(file.untracked_height, ui_scale_percent);
    file
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

pub fn path_storage_key(path: &Path) -> String {
    if let Some(text) = path.to_str() {
        return text.to_string();
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        let bytes = path.as_os_str().as_bytes();
        let mut out = String::with_capacity(SESSION_PATH_BYTES_PREFIX.len() + bytes.len() * 2);
        out.push_str(SESSION_PATH_BYTES_PREFIX);
        out.push_str(&hex_encode(bytes));
        out
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let mut raw = Vec::new();
        for unit in path.as_os_str().encode_wide() {
            raw.extend_from_slice(&unit.to_le_bytes());
        }
        let mut out = String::with_capacity(SESSION_PATH_WIDE_PREFIX.len() + raw.len() * 2);
        out.push_str(SESSION_PATH_WIDE_PREFIX);
        out.push_str(&hex_encode(&raw));
        out
    }

    #[cfg(not(any(unix, windows)))]
    {
        path.display().to_string()
    }
}

pub fn path_storage_key_shared(path: &Path) -> Arc<str> {
    if let Some(text) = path.to_str() {
        return Arc::from(text);
    }

    Arc::from(path_storage_key(path))
}

pub fn path_from_storage_key(raw: &str) -> PathBuf {
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        if let Some(hex) = raw.strip_prefix(SESSION_PATH_BYTES_PREFIX)
            && let Some(bytes) = hex_decode(hex)
        {
            return PathBuf::from(OsString::from_vec(bytes));
        }
    }

    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt as _;

        if let Some(hex) = raw.strip_prefix(SESSION_PATH_WIDE_PREFIX)
            && let Some(bytes) = hex_decode(hex)
            && bytes.len() % 2 == 0
        {
            let mut wide = Vec::with_capacity(bytes.len() / 2);
            for chunk in bytes.chunks_exact(2) {
                wide.push(u16::from_le_bytes([chunk[0], chunk[1]]));
            }
            return PathBuf::from(OsString::from_wide(&wide));
        }
    }

    PathBuf::from(raw)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let high = hex_value(pair[0])?;
        let low = hex_value(pair[1])?;
        out.push((high << 4) | low);
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn persist_to_path(path: &Path, session: &impl Serialize) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let contents = serde_json::to_vec(session).expect("serializing session file should succeed");

    let mut tmp_file = tempfile::NamedTempFile::new_in(parent)?;
    tmp_file.write_all(&contents)?;
    tmp_file.flush()?;

    tmp_file.persist(path).map(|_| ()).map_err(|err| err.error)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RepoId, RepoState};
    use gitcomet_core::domain::{HistoryMode, LogScope, RepoSpec};

    fn clear_session_repos_snapshot_cache() {
        SESSION_REPOS_SNAPSHOT_CACHE.with(|cache| {
            cache.borrow_mut().take();
        });
    }

    fn unique_session_test_dir(label: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "gitcomet-session-unit-test-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn assert_session_writer_waits_for_shared_lock(
        label: &str,
        persist: impl FnOnce(PathBuf) -> io::Result<()> + Send + 'static,
    ) {
        let path = unique_session_test_dir(label).join("session.json");
        let guard = session_file_persist_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();

        let handle = std::thread::spawn(move || {
            started_tx.send(()).expect("send writer started");
            let result = persist(path);
            done_tx.send(result).expect("send writer result");
        });

        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("writer thread started");
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "{label} writer finished while the session persist lock was held"
        );
        drop(guard);

        done_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("writer finished after lock release")
            .expect("writer persist succeeds");
        handle.join().expect("writer thread joins");
    }

    #[test]
    fn session_file_persist_lock_is_shared_by_session_writers() {
        assert_session_writer_waits_for_shared_lock("persist-repos-snapshot", |path| {
            let repo = path.with_file_name("repo-snapshot");
            let repo_text = repo.to_string_lossy().into_owned();
            let open_repos: Arc<[Arc<str>]> =
                Arc::from(vec![Arc::<str>::from(repo_text)].into_boxed_slice());
            let snapshot = SessionReposSnapshot {
                open_repos,
                active_repo_index: Some(0),
            };
            persist_repos_snapshot_to_path(&snapshot, &path)
        });
        assert_session_writer_waits_for_shared_lock("persist-recent-repo", |path| {
            let repo = path.with_file_name("recent-repo");
            fs::create_dir_all(&repo)?;
            persist_recent_repo_to_path(&repo, &path)
        });
        assert_session_writer_waits_for_shared_lock("remove-recent-repo", |path| {
            let repo = path.with_file_name("remove-recent-repo");
            persist_to_path(
                &path,
                &UiSessionFile {
                    version: CURRENT_SESSION_FILE_VERSION,
                    recent_repos: Some(vec![path_storage_key(&repo)]),
                    ..UiSessionFile::default()
                },
            )?;
            remove_recent_repo_to_path(&repo, &path)
        });
        assert_session_writer_waits_for_shared_lock("persist-ui-settings", |path| {
            persist_ui_settings_to_path(
                UiSettings {
                    external_code_editor: Some(Some(ExternalCodeEditorSetting::Custom {
                        executable: PathBuf::from("/usr/bin/editor"),
                        arguments: Some("--reuse-window".to_string()),
                    })),
                    ..UiSettings::default()
                },
                &path,
            )
        });
        assert_session_writer_waits_for_shared_lock("persist-history-mode", |path| {
            let repo = path.with_file_name("history-mode-repo");
            persist_repo_history_mode_to_path(&repo, HistoryMode::NoMerges, &path)
        });
        assert_session_writer_waits_for_shared_lock("persist-history-mode-batch", |path| {
            let repo = path.with_file_name("history-mode-batch-repo");
            persist_repo_history_modes_batch_to_path(&[(repo, HistoryMode::FirstParent)], &path)
        });
        assert_session_writer_waits_for_shared_lock("persist-history-scope", |path| {
            let repo = path.with_file_name("history-scope-repo");
            persist_repo_history_scope_to_path(&repo, LogScope::AllBranches, &path)
        });
        assert_session_writer_waits_for_shared_lock("persist-fetch-prune", |path| {
            let repo = path.with_file_name("fetch-prune-repo");
            persist_repo_fetch_prune_deleted_remote_tracking_branches_to_path(&repo, true, &path)
        });
        assert_session_writer_waits_for_shared_lock("persist-survey-opened", |path| {
            persist_survey_prompt_opened_to_path(&path, "survey", 123)
        });
        assert_session_writer_waits_for_shared_lock("persist-survey-postponed", |path| {
            persist_survey_prompt_postponed_to_path(&path, "survey", 60, 123)
        });
    }

    #[test]
    fn session_file_round_trips() {
        let dir = env::temp_dir().join(format!("gitcomet-session-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        let file = UiSessionFileV1 {
            version: SESSION_FILE_VERSION_V1,
            open_repos: vec!["/a".into(), "/b".into()],
            active_repo: Some("/b".into()),
        };
        persist_to_path(&path, &file).expect("persist succeeds");

        let contents = fs::read_to_string(&path).expect("read succeeds");
        let loaded: UiSessionFileV1 = serde_json::from_str(&contents).expect("json parses");
        assert_eq!(loaded.version, SESSION_FILE_VERSION_V1);
        assert_eq!(loaded.open_repos, vec!["/a".to_string(), "/b".to_string()]);
        assert_eq!(loaded.active_repo.as_deref(), Some("/b"));
    }

    #[test]
    fn path_storage_key_keeps_utf8_plain_text() {
        let path = Path::new("/tmp/gitcomet-repo");
        let key = path_storage_key(path);
        assert_eq!(key, "/tmp/gitcomet-repo");
        assert_eq!(path_from_storage_key(&key), path);
    }

    #[cfg(unix)]
    #[test]
    fn path_storage_key_round_trips_non_utf8_unix_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        let path = Path::new(OsStr::from_bytes(b"/tmp/gitcomet-\xff"));
        let key = path_storage_key(path);
        assert!(key.starts_with(SESSION_PATH_BYTES_PREFIX), "{key}");
        let restored = path_from_storage_key(&key);
        assert_eq!(restored.as_os_str().as_bytes(), path.as_os_str().as_bytes());
    }

    #[test]
    fn load_repo_session_preferences_collects_current_and_legacy_history_settings() {
        let dir = unique_session_test_dir("repo-session-preferences");
        let session_file = dir.join("session.json");
        let repo_mode = dir.join("repo-mode");
        let repo_legacy = dir.join("repo-legacy");
        let repo_fetch = dir.join("repo-fetch");
        let _ = fs::create_dir_all(&repo_mode);
        let _ = fs::create_dir_all(&repo_legacy);
        let _ = fs::create_dir_all(&repo_fetch);

        assert_eq!(
            load_repo_session_preferences_from_path(&dir.join("missing.json")),
            RepoSessionPreferences::default()
        );

        persist_ui_settings_to_path(
            UiSettings {
                default_history_mode: Some(HistoryMode::MergesOnly),
                ..UiSettings::default()
            },
            &session_file,
        )
        .expect("persist default history mode");
        persist_repo_history_mode_to_path(&repo_mode, HistoryMode::NoMerges, &session_file)
            .expect("persist explicit history mode");
        persist_repo_history_scope_to_path(&repo_legacy, LogScope::CurrentBranch, &session_file)
            .expect("persist legacy history scope");
        persist_repo_fetch_prune_deleted_remote_tracking_branches_to_path(
            &repo_fetch,
            true,
            &session_file,
        )
        .expect("persist fetch-prune setting");

        let loaded = load_repo_session_preferences_from_path(&session_file);
        assert_eq!(loaded.default_history_mode, Some(HistoryMode::MergesOnly));
        assert_eq!(
            loaded.repo_history_modes.get(&path_storage_key(&repo_mode)),
            Some(&HistoryMode::NoMerges)
        );
        assert_eq!(
            loaded
                .repo_history_scopes
                .get(&path_storage_key(&repo_legacy)),
            Some(&HistoryMode::FirstParent)
        );
        assert_eq!(
            loaded
                .repo_fetch_prune_deleted_remote_tracking_branches
                .get(&path_storage_key(&repo_fetch)),
            Some(&true)
        );
    }

    #[test]
    fn persist_virtual_branches_round_trips_workspace() {
        let dir = unique_session_test_dir("virtual-branches");
        let session_file = dir.join("session.json");
        let repo = dir.join("repo");
        let _ = fs::create_dir_all(&repo);
        let repo_key = path_storage_key(&repo);

        let data = VirtualBranchesSessionFile {
            next_id: 3,
            branches: vec![
                VirtualBranchSessionFile {
                    id: 1,
                    name: "feature".into(),
                    paths: vec!["src/lib.rs".into()],
                    applied: false,
                    stored_patch: Some(
                        "diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,1 +1,1 @@\n-foo\n+bar\n"
                            .into(),
                    ),
                },
                VirtualBranchSessionFile {
                    id: 2,
                    name: "fix".into(),
                    paths: vec![],
                    applied: true,
                    stored_patch: None,
                },
            ],
        };
        persist_virtual_branches_to_path(&repo, &data, &session_file).expect("persist workspace");

        let loaded = load_repo_session_preferences_from_path(&session_file);
        assert_eq!(loaded.virtual_branches.get(&repo_key), Some(&data));

        // An empty branch list removes the stored workspace for the repo.
        persist_virtual_branches_to_path(
            &repo,
            &VirtualBranchesSessionFile::default(),
            &session_file,
        )
        .expect("clear workspace");
        let loaded = load_repo_session_preferences_from_path(&session_file);
        assert!(loaded.virtual_branches.is_empty());
    }

    #[test]
    fn virtual_branch_session_conversion_round_trips_domain() {
        let file = VirtualBranchSessionFile {
            id: 7,
            name: "feature".into(),
            paths: vec!["src/lib.rs".into(), "README.md".into()],
            applied: false,
            stored_patch: Some("patch text".into()),
        };
        let branch: gitcomet_core::domain::VirtualBranch = (&file).into();
        assert_eq!(branch.id, 7);
        assert_eq!(branch.name.as_ref(), "feature");
        assert_eq!(
            branch.paths,
            vec![PathBuf::from("src/lib.rs"), PathBuf::from("README.md")]
        );
        assert!(!branch.applied);
        assert_eq!(branch.stored_patch.as_deref(), Some("patch text"));
        assert!(!branch.pending);

        let back = VirtualBranchSessionFile::from(&branch);
        assert_eq!(back, file);
    }

    #[test]
    fn persist_ui_settings_round_trips_sidebar_collapsed() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-session-sidebar-collapsed-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let session_file = dir.join("session.json");

        // Default (unset) leaves the field absent.
        assert_eq!(load_from_path(&session_file).sidebar_collapsed, None);

        persist_ui_settings_to_path(
            UiSettings {
                sidebar_collapsed: Some(true),
                ..UiSettings::default()
            },
            &session_file,
        )
        .expect("persist collapsed");
        assert_eq!(load_from_path(&session_file).sidebar_collapsed, Some(true));

        // A later settings write that doesn't touch the field preserves it.
        persist_ui_settings_to_path(
            UiSettings {
                theme_mode: Some("dark".to_string()),
                ..UiSettings::default()
            },
            &session_file,
        )
        .expect("persist theme");
        assert_eq!(load_from_path(&session_file).sidebar_collapsed, Some(true));

        persist_ui_settings_to_path(
            UiSettings {
                sidebar_collapsed: Some(false),
                ..UiSettings::default()
            },
            &session_file,
        )
        .expect("persist expanded");
        assert_eq!(load_from_path(&session_file).sidebar_collapsed, Some(false));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_repo_history_modes_batch_skips_empty_and_unchanged_updates() {
        let dir = unique_session_test_dir("repo-history-mode-batch");
        let session_file = dir.join("session.json");
        let missing_file = dir.join("missing.json");
        let repo_a = dir.join("repo-a");
        let repo_b = dir.join("repo-b");
        let repo_c = dir.join("repo-c");
        let _ = fs::create_dir_all(&repo_a);
        let _ = fs::create_dir_all(&repo_b);
        let _ = fs::create_dir_all(&repo_c);

        persist_repo_history_modes_batch_to_path(&[], &missing_file)
            .expect("empty updates should succeed");
        assert!(
            !missing_file.exists(),
            "empty batch updates should not create a session file"
        );

        persist_ui_settings_to_path(
            UiSettings {
                default_history_mode: Some(HistoryMode::MergesOnly),
                ..UiSettings::default()
            },
            &session_file,
        )
        .expect("persist default history mode");
        persist_repo_history_scope_to_path(&repo_b, LogScope::CurrentBranch, &session_file)
            .expect("persist legacy history scope");
        persist_repo_fetch_prune_deleted_remote_tracking_branches_to_path(
            &repo_c,
            true,
            &session_file,
        )
        .expect("persist fetch-prune setting");
        persist_repo_history_mode_to_path(&repo_a, HistoryMode::FirstParent, &session_file)
            .expect("persist repo_a history mode");

        let before = fs::read_to_string(&session_file).expect("read session file");

        persist_repo_history_modes_batch_to_path(&[], &session_file)
            .expect("empty updates should not rewrite the file");
        assert_eq!(
            fs::read_to_string(&session_file).expect("read session file after empty batch"),
            before
        );

        persist_repo_history_modes_batch_to_path(
            &[(repo_a.clone(), HistoryMode::FirstParent)],
            &session_file,
        )
        .expect("unchanged updates should not rewrite the file");
        assert_eq!(
            fs::read_to_string(&session_file).expect("read session file after unchanged batch"),
            before
        );

        persist_repo_history_modes_batch_to_path(
            &[
                (repo_b.clone(), HistoryMode::AllBranches),
                (repo_c.clone(), HistoryMode::NoMerges),
            ],
            &session_file,
        )
        .expect("persist changed batch updates");

        let loaded = load_repo_session_preferences_from_path(&session_file);
        assert_eq!(loaded.default_history_mode, Some(HistoryMode::MergesOnly));
        assert_eq!(
            loaded.repo_history_modes.get(&path_storage_key(&repo_a)),
            Some(&HistoryMode::FirstParent)
        );
        assert_eq!(
            loaded.repo_history_modes.get(&path_storage_key(&repo_b)),
            Some(&HistoryMode::AllBranches)
        );
        assert_eq!(
            loaded.repo_history_modes.get(&path_storage_key(&repo_c)),
            Some(&HistoryMode::NoMerges)
        );
        assert_eq!(
            loaded.repo_history_scopes.get(&path_storage_key(&repo_b)),
            Some(&HistoryMode::FirstParent)
        );
        assert_eq!(
            loaded
                .repo_fetch_prune_deleted_remote_tracking_branches
                .get(&path_storage_key(&repo_c)),
            Some(&true)
        );
    }

    #[test]
    fn survey_prompt_requires_recorded_repository() {
        const SURVEY_ID: &str = "gitcomet_user_survey_2026_04";
        let dir = unique_session_test_dir("survey-empty-session");
        let session_file = dir.join("session.json");

        assert!(!should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100
        ));

        fs::write(&session_file, b"{not-json").expect("write malformed session");
        assert!(!should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100
        ));

        fs::write(&session_file, br#"{"version":3}"#).expect("write version-only session");
        assert!(!should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100
        ));

        persist_to_path(
            &session_file,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                survey_prompt: Some(SurveyPromptSession {
                    survey_id: SURVEY_ID.to_string(),
                    opened_at_unix_seconds: None,
                    postponed_until_unix_seconds: Some(50),
                }),
                ..UiSessionFile::default()
            },
        )
        .expect("persist survey-only session");
        assert!(!should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100
        ));

        persist_to_path(
            &session_file,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                window_width: Some(1200),
                window_height: Some(800),
                theme_mode: Some("dark".to_string()),
                repo_history_scopes: Some(BTreeMap::from([(
                    "/tmp/repo".to_string(),
                    HistoryScopeSetting::AllBranches,
                )])),
                ..UiSessionFile::default()
            },
        )
        .expect("persist non-repo session data");
        assert!(!should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100
        ));
    }

    #[test]
    fn survey_prompt_accepts_recorded_repository_sources() {
        const SURVEY_ID: &str = "gitcomet_user_survey_2026_04";
        let dir = unique_session_test_dir("survey-repository-sources");
        let session_file = dir.join("session.json");

        persist_to_path(
            &session_file,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: vec![" /tmp/open-repo ".to_string()],
                ..UiSessionFile::default()
            },
        )
        .expect("persist open repo session");
        assert!(should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100
        ));

        persist_to_path(
            &session_file,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                active_repo: Some("/tmp/active-repo".to_string()),
                ..UiSessionFile::default()
            },
        )
        .expect("persist active repo session");
        assert!(should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100
        ));

        persist_to_path(
            &session_file,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                recent_repos: Some(vec!["\t/tmp/recent-repo\n".to_string()]),
                ..UiSessionFile::default()
            },
        )
        .expect("persist recent repo session");
        assert!(should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100
        ));
    }

    #[test]
    fn survey_prompt_respects_id_opened_and_postponed_state() {
        const SURVEY_ID: &str = "gitcomet_user_survey_2026_04";
        const NEXT_SURVEY_ID: &str = "gitcomet_user_survey_2026_05";
        const POSTPONE_SECONDS: u64 = 60 * 60 * 24 * 7;
        let dir = unique_session_test_dir("survey-prompt-state");
        let session_file = dir.join("session.json");

        persist_to_path(
            &session_file,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: vec!["/tmp/repo".to_string()],
                ..UiSessionFile::default()
            },
        )
        .expect("persist eligible session");
        assert!(should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100
        ));

        persist_survey_prompt_postponed_to_path(&session_file, SURVEY_ID, POSTPONE_SECONDS, 100)
            .expect("persist postponed survey");
        let postponed_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&session_file).expect("read postponed session"))
                .expect("postponed session json parses");
        assert_eq!(
            postponed_json
                .pointer("/survey_prompt/survey_id")
                .and_then(|value| value.as_str()),
            Some(SURVEY_ID)
        );
        assert_eq!(
            postponed_json
                .pointer("/survey_prompt/postponed_until_unix_seconds")
                .and_then(|value| value.as_u64()),
            Some(100 + POSTPONE_SECONDS)
        );
        assert!(
            postponed_json
                .pointer("/survey_prompt/opened_at_unix_seconds")
                .is_none()
        );
        assert!(!should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100 + POSTPONE_SECONDS - 1
        ));
        assert!(should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            100 + POSTPONE_SECONDS
        ));
        assert!(should_show_survey_prompt_from_path(
            &session_file,
            NEXT_SURVEY_ID,
            100
        ));

        persist_survey_prompt_opened_to_path(&session_file, SURVEY_ID, 200)
            .expect("persist opened survey");
        let opened_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&session_file).expect("read opened session"))
                .expect("opened session json parses");
        assert_eq!(
            opened_json
                .pointer("/survey_prompt/survey_id")
                .and_then(|value| value.as_str()),
            Some(SURVEY_ID)
        );
        assert_eq!(
            opened_json
                .pointer("/survey_prompt/opened_at_unix_seconds")
                .and_then(|value| value.as_u64()),
            Some(200)
        );
        assert!(
            opened_json
                .pointer("/survey_prompt/postponed_until_unix_seconds")
                .is_none()
        );
        assert!(!should_show_survey_prompt_from_path(
            &session_file,
            SURVEY_ID,
            300
        ));
        assert!(should_show_survey_prompt_from_path(
            &session_file,
            NEXT_SURVEY_ID,
            300
        ));
    }

    #[test]
    fn survey_prompt_persistence_preserves_existing_session_fields() {
        const SURVEY_ID: &str = "gitcomet_user_survey_2026_04";
        let dir = unique_session_test_dir("survey-preserves-session");
        let session_file = dir.join("session.json");
        let repo = dir.join("repo");

        persist_to_path(
            &session_file,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: vec![path_storage_key(&repo)],
                active_repo: Some(path_storage_key(&repo)),
                recent_repos: Some(vec![path_storage_key(&repo)]),
                theme_mode: Some("dark".to_string()),
                repo_history_scopes: Some(BTreeMap::from([(
                    path_storage_key(&repo),
                    HistoryScopeSetting::AllBranches,
                )])),
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_survey_prompt_opened_to_path(&session_file, SURVEY_ID, 123)
            .expect("persist survey opened");

        let file = load_file(&session_file).expect("load session file");
        assert_eq!(file.open_repos, vec![path_storage_key(&repo)]);
        assert_eq!(
            file.active_repo.as_deref(),
            Some(path_storage_key(&repo).as_str())
        );
        assert_eq!(file.theme_mode.as_deref(), Some("dark"));
        assert_eq!(
            file.repo_history_scopes
                .as_ref()
                .and_then(|scopes| scopes.get(&path_storage_key(&repo))),
            Some(&HistoryScopeSetting::AllBranches)
        );
        assert_eq!(
            file.survey_prompt,
            Some(SurveyPromptSession {
                survey_id: SURVEY_ID.to_string(),
                opened_at_unix_seconds: Some(123),
                postponed_until_unix_seconds: None,
            })
        );
    }

    #[test]
    fn detects_test_harness_executable_paths() {
        // `cargo test` / nextest binaries are typically located under a `deps` directory.
        assert!(looks_like_test_binary(Path::new(
            "/tmp/target/debug/deps/foo"
        )));
        assert!(!looks_like_test_binary(Path::new("/tmp/target/debug/foo")));

        // nextest uses a separate target subdir.
        assert!(looks_like_test_binary(Path::new(
            "/tmp/target/nextest/default/foo"
        )));

        // Cargo test binaries also have a hash suffix.
        assert!(looks_like_test_binary(Path::new(
            "/tmp/target/debug/gitcomet_ui_gpui-3ad1b0fd3f0c0d3e"
        )));
        assert!(!looks_like_test_binary(Path::new(
            "/tmp/target/debug/gitcomet"
        )));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn app_data_dir_prefers_xdg_data_home() {
        assert_eq!(
            app_data_dir_linux(
                Some(OsStr::new("/tmp/gitcomet-data")),
                Some(OsStr::new("/home/alice"))
            ),
            Some(PathBuf::from("/tmp/gitcomet-data/gitcomet"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn app_data_dir_falls_back_to_local_share() {
        assert_eq!(
            app_data_dir_linux(None, Some(OsStr::new("/home/alice"))),
            Some(PathBuf::from("/home/alice/.local/share/gitcomet"))
        );
    }

    #[test]
    fn persist_from_state_and_load_from_path_round_trip() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-session-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        let repo_a = dir.join("repo-a");
        let repo_b = dir.join("repo-b");
        let _ = fs::create_dir_all(&repo_a);
        let _ = fs::create_dir_all(&repo_b);

        let state = AppState {
            repos: vec![
                RepoState::new_opening(
                    RepoId(1),
                    RepoSpec {
                        workdir: repo_a.clone(),
                    },
                ),
                RepoState::new_opening(
                    RepoId(2),
                    RepoSpec {
                        workdir: repo_b.clone(),
                    },
                ),
            ],
            active_repo: Some(RepoId(2)),
            ..Default::default()
        };

        persist_from_state_to_path(&state, &path).expect("persist succeeds");
        let loaded = load_from_path(&path);
        assert_eq!(loaded.open_repos, vec![repo_a, repo_b.clone()]);
        assert_eq!(loaded.active_repo, Some(repo_b));
    }

    #[test]
    fn snapshot_repos_from_state_dedups_and_filters_inactive_selection() {
        let repo_a = PathBuf::from("/tmp/repo-a");
        let repo_b = PathBuf::from("/tmp/repo-b");
        let state = AppState {
            repos: vec![
                RepoState::new_opening(
                    RepoId(1),
                    RepoSpec {
                        workdir: repo_a.clone(),
                    },
                ),
                RepoState::new_opening(
                    RepoId(2),
                    RepoSpec {
                        workdir: repo_a.clone(),
                    },
                ),
            ],
            active_repo: Some(RepoId(999)),
            ..Default::default()
        };

        let snapshot = snapshot_repos_from_state(&state);
        assert_eq!(
            snapshot.open_repos.as_ref(),
            &[path_storage_key_shared(&repo_a)]
        );
        assert_eq!(snapshot.active_repo_index, None);

        let state = AppState {
            repos: vec![
                RepoState::new_opening(
                    RepoId(1),
                    RepoSpec {
                        workdir: repo_a.clone(),
                    },
                ),
                RepoState::new_opening(
                    RepoId(2),
                    RepoSpec {
                        workdir: repo_b.clone(),
                    },
                ),
            ],
            active_repo: Some(RepoId(2)),
            ..Default::default()
        };
        let snapshot = snapshot_repos_from_state(&state);
        assert_eq!(snapshot.active_repo_index, Some(1));
        assert_eq!(snapshot.open_repos[1].as_ref(), "/tmp/repo-b");
    }

    #[test]
    fn snapshot_repos_from_state_reuses_cached_open_repo_slice_for_same_repo_list() {
        let state = AppState {
            repos: vec![
                RepoState::new_opening(
                    RepoId(1),
                    RepoSpec {
                        workdir: PathBuf::from("/tmp/repo-a"),
                    },
                ),
                RepoState::new_opening(
                    RepoId(2),
                    RepoSpec {
                        workdir: PathBuf::from("/tmp/repo-b"),
                    },
                ),
            ],
            active_repo: Some(RepoId(2)),
            ..Default::default()
        };

        let first = snapshot_repos_from_state(&state);
        let second = snapshot_repos_from_state(&state);

        assert!(Arc::ptr_eq(&first.open_repos, &second.open_repos));
    }

    #[test]
    fn snapshot_repos_from_state_cache_keeps_dedup_index_for_duplicate_workdirs() {
        let repo_a = PathBuf::from("/tmp/repo-a");
        let mut state = AppState {
            repos: vec![
                RepoState::new_opening(
                    RepoId(1),
                    RepoSpec {
                        workdir: repo_a.clone(),
                    },
                ),
                RepoState::new_opening(RepoId(2), RepoSpec { workdir: repo_a }),
            ],
            active_repo: Some(RepoId(1)),
            ..Default::default()
        };

        let first = snapshot_repos_from_state(&state);
        state.active_repo = Some(RepoId(2));
        let second = snapshot_repos_from_state(&state);

        assert!(Arc::ptr_eq(&first.open_repos, &second.open_repos));
        assert_eq!(second.active_repo_index, Some(0));
    }

    #[test]
    fn snapshot_repos_from_state_preserves_first_seen_order_for_repeated_workdirs() {
        clear_session_repos_snapshot_cache();

        let repo_a = PathBuf::from("/tmp/repo-a");
        let repo_b = PathBuf::from("/tmp/repo-b");
        let state = AppState {
            repos: vec![
                RepoState::new_opening(
                    RepoId(1),
                    RepoSpec {
                        workdir: repo_a.clone(),
                    },
                ),
                RepoState::new_opening(
                    RepoId(2),
                    RepoSpec {
                        workdir: repo_b.clone(),
                    },
                ),
                RepoState::new_opening(
                    RepoId(3),
                    RepoSpec {
                        workdir: repo_a.clone(),
                    },
                ),
            ],
            active_repo: Some(RepoId(3)),
            ..Default::default()
        };

        let snapshot = snapshot_repos_from_state(&state);
        assert_eq!(
            snapshot.open_repos.as_ref(),
            &[
                path_storage_key_shared(&repo_a),
                path_storage_key_shared(&repo_b)
            ]
        );
        assert_eq!(snapshot.active_repo_index, Some(0));
    }

    #[test]
    fn snapshot_repos_from_state_cache_invalidates_when_repo_order_changes() {
        clear_session_repos_snapshot_cache();

        let repo_a = PathBuf::from("/tmp/repo-a");
        let repo_b = PathBuf::from("/tmp/repo-b");
        let mut state = AppState {
            repos: vec![
                RepoState::new_opening(
                    RepoId(1),
                    RepoSpec {
                        workdir: repo_a.clone(),
                    },
                ),
                RepoState::new_opening(
                    RepoId(2),
                    RepoSpec {
                        workdir: repo_b.clone(),
                    },
                ),
            ],
            active_repo: Some(RepoId(1)),
            ..Default::default()
        };

        let first = snapshot_repos_from_state(&state);
        state.repos.swap(0, 1);
        let second = snapshot_repos_from_state(&state);

        assert!(
            !Arc::ptr_eq(&first.open_repos, &second.open_repos),
            "reordering repos should invalidate the cached open-repo slice"
        );
        assert_eq!(
            second.open_repos.as_ref(),
            &[
                path_storage_key_shared(&repo_b),
                path_storage_key_shared(&repo_a)
            ]
        );
        assert_eq!(second.active_repo_index, Some(1));
    }

    #[test]
    fn snapshot_repos_from_state_cache_invalidates_when_repo_spec_changes() {
        clear_session_repos_snapshot_cache();

        let repo_a = PathBuf::from("/tmp/repo-a");
        let repo_b = PathBuf::from("/tmp/repo-b");
        let mut state = AppState {
            repos: vec![RepoState::new_opening(
                RepoId(1),
                RepoSpec {
                    workdir: repo_a.clone(),
                },
            )],
            active_repo: Some(RepoId(1)),
            ..Default::default()
        };

        let first = snapshot_repos_from_state(&state);
        state.repos[0].set_spec(RepoSpec {
            workdir: repo_b.clone(),
        });
        let second = snapshot_repos_from_state(&state);

        assert!(
            !Arc::ptr_eq(&first.open_repos, &second.open_repos),
            "changing the repo spec should invalidate the cached open-repo slice"
        );
        assert_eq!(
            second.open_repos.as_ref(),
            &[path_storage_key_shared(&repo_b)]
        );
        assert_eq!(second.active_repo_index, Some(0));
    }

    #[test]
    fn load_from_path_migrates_v1_files() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-session-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        let repo_a = dir.join("repo-a");
        let repo_b = dir.join("repo-b");
        let _ = fs::create_dir_all(&repo_a);
        let _ = fs::create_dir_all(&repo_b);

        persist_to_path(
            &path,
            &UiSessionFileV1 {
                version: SESSION_FILE_VERSION_V1,
                open_repos: vec![path_storage_key(&repo_a), path_storage_key(&repo_b)],
                active_repo: Some(path_storage_key(&repo_b)),
            },
        )
        .expect("persist succeeds");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.open_repos, vec![repo_a, repo_b.clone()]);
        assert_eq!(loaded.active_repo, Some(repo_b));
        assert!(loaded.recent_repos.is_empty());
        assert_eq!(loaded.window_width, None);
        assert_eq!(loaded.date_time_format, None);
    }

    #[test]
    fn load_from_path_migrates_v2_scaled_dimensions_to_design_units() {
        let cases = [
            (100, 280, 420, 222, 111),
            (125, 350, 525, 278, 139),
            (200, 560, 840, 444, 222),
        ];

        for (percent, sidebar_width, details_width, change_tracking_height, untracked_height) in
            cases
        {
            let dir = env::temp_dir().join(format!(
                "gitcomet-session-v2-migration-test-{}-{}-{percent}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            let _ = fs::create_dir_all(&dir);
            let path = dir.join("session.json");

            persist_to_path(
                &path,
                &UiSessionFile {
                    version: SESSION_FILE_VERSION_V2,
                    open_repos: Vec::new(),
                    active_repo: None,
                    sidebar_width: Some(sidebar_width),
                    details_width: Some(details_width),
                    ui_scale_percent: Some(percent),
                    change_tracking_height: Some(change_tracking_height),
                    untracked_height: Some(untracked_height),
                    ..UiSessionFile::default()
                },
            )
            .expect("persist succeeds");

            let loaded = load_from_path(&path);
            assert_eq!(loaded.ui_scale_percent, Some(percent));
            assert_eq!(loaded.sidebar_width, Some(280));
            assert_eq!(loaded.details_width, Some(420));
            assert_eq!(loaded.change_tracking_height, Some(222));
            assert_eq!(loaded.untracked_height, Some(111));
        }
    }

    #[test]
    fn load_from_path_migrates_v2_scaled_dimensions_without_saved_zoom_as_100_percent() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-session-v2-migration-default-scale-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: SESSION_FILE_VERSION_V2,
                open_repos: Vec::new(),
                active_repo: None,
                sidebar_width: Some(280),
                details_width: Some(420),
                change_tracking_height: Some(222),
                untracked_height: Some(111),
                ..UiSessionFile::default()
            },
        )
        .expect("persist succeeds");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.sidebar_width, Some(280));
        assert_eq!(loaded.details_width, Some(420));
        assert_eq!(loaded.change_tracking_height, Some(222));
        assert_eq!(loaded.untracked_height, Some(111));
    }

    #[test]
    fn persist_recent_repo_round_trips_dedup_and_reorders() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-recent-repos-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        let repo_a = dir.join("repo-a");
        let repo_b = dir.join("repo-b");
        let _ = fs::create_dir_all(&repo_a);
        let _ = fs::create_dir_all(&repo_b);

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_recent_repo_to_path(&repo_a, &path).expect("persist first repo");
        persist_recent_repo_to_path(&repo_b, &path).expect("persist second repo");
        persist_recent_repo_to_path(&repo_a, &path).expect("move repo to front");

        // Recents are stored canonicalized so they compare equal to the workdir
        // an open repository carries.
        let canonical = |path: &std::path::Path| {
            gitcomet_core::path_utils::canonicalize_or_original(path.to_path_buf())
        };
        let loaded = load_from_path(&path);
        assert_eq!(
            loaded.recent_repos,
            vec![canonical(&repo_a), canonical(&repo_b)]
        );
    }

    #[test]
    fn history_highlight_commit_chain_round_trips_and_defaults_to_unset() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-highlight-chain-setting-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let session_file = dir.join("session.json");

        // Absent from the file, so the UI applies its own default rather than
        // the setting silently reading as "off".
        assert_eq!(
            load_from_path(&session_file).history_highlight_commit_chain,
            None
        );

        persist_ui_settings_to_path(
            UiSettings {
                history_highlight_commit_chain: Some(false),
                ..UiSettings::default()
            },
            &session_file,
        )
        .expect("persist highlight setting");
        assert_eq!(
            load_from_path(&session_file).history_highlight_commit_chain,
            Some(false)
        );

        persist_ui_settings_to_path(
            UiSettings {
                history_highlight_commit_chain: Some(true),
                ..UiSettings::default()
            },
            &session_file,
        )
        .expect("re-enable highlight setting");
        assert_eq!(
            load_from_path(&session_file).history_highlight_commit_chain,
            Some(true)
        );
    }

    #[test]
    fn remove_recent_repo_drops_matching_entry() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-remove-recent-repo-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        let repo_a = dir.join("repo-a");
        let repo_b = dir.join("repo-b");
        let _ = fs::create_dir_all(&repo_a);
        let _ = fs::create_dir_all(&repo_b);

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                recent_repos: Some(vec![path_storage_key(&repo_a), path_storage_key(&repo_b)]),
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        remove_recent_repo_to_path(&repo_b, &path).expect("remove invalid recent repo");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.recent_repos, vec![repo_a]);
    }

    #[test]
    fn persist_pinned_repo_appends_in_pin_order_and_dedupes() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-pinned-repos-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        let repo_a = dir.join("repo-a");
        let repo_b = dir.join("repo-b");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_pinned_repo_to_path(&repo_a, &path).expect("pin first repo");
        persist_pinned_repo_to_path(&repo_b, &path).expect("pin second repo");
        // Re-pinning keeps the original position rather than moving the repo,
        // unlike the recents, which are an MRU list.
        persist_pinned_repo_to_path(&repo_a, &path).expect("re-pin first repo");

        let loaded = load_from_path(&path);
        assert_eq!(
            loaded.pinned_repos,
            vec![repo_a.clone(), repo_b.clone()],
            "re-pinning must not reorder the pin list"
        );

        remove_pinned_repo_to_path(&repo_b, &path).expect("unpin second repo");
        assert_eq!(load_from_path(&path).pinned_repos, vec![repo_a]);
    }

    /// The stored keys are written trimmed, but a hand-edited file can pad them.
    /// A padded copy is still the same repository, so it must not survive the
    /// promotion as a second entry.
    #[test]
    fn persist_recent_repo_collapses_a_padded_duplicate() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-recent-padded-dupe-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");
        let repo = dir.join("repo-padded");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                recent_repos: Some(vec![
                    format!("  {}  ", path_storage_key(&repo)),
                    "   ".to_owned(),
                ]),
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_recent_repo_to_path(&repo, &path).expect("record repo as recent");

        assert_eq!(
            load_from_path(&path).recent_repos,
            vec![repo],
            "the padded entry and the blank should both be gone"
        );
    }

    #[test]
    fn pinned_repos_survive_the_recent_repository_cap() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-pinned-repo-cap-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");
        let pinned = dir.join("repo-pinned");

        persist_pinned_repo_to_path(&pinned, &path).expect("pin repo");
        persist_recent_repo_to_path(&pinned, &path).expect("record repo as recent");
        for ix in 0..MAX_RECENT_REPOS {
            persist_recent_repo_to_path(&dir.join(format!("repo-{ix}")), &path)
                .expect("push the pinned repo off the recents tail");
        }

        let loaded = load_from_path(&path);
        assert!(
            !loaded.recent_repos.contains(&pinned),
            "the pinned repository should have fallen off the capped recents list"
        );
        assert_eq!(
            loaded.pinned_repos,
            vec![pinned],
            "pins are a separate, uncapped list, so the repository is still reachable"
        );
    }

    /// A UI holding its own copy of the recents has to be able to apply a bump
    /// without re-reading the file, and the copy has to still match the file
    /// afterwards — including at the cap, where the file drops its tail.
    #[test]
    fn promote_recent_repo_matches_the_file_at_the_cap() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-promote-recent-cap-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        // One more repository than the list can hold, so the cap is in play.
        let repos: Vec<PathBuf> = (0..=MAX_RECENT_REPOS)
            .map(|ix| dir.join(format!("repo-{ix}")))
            .collect();
        for repo in &repos {
            persist_recent_repo_to_path(repo, &path).expect("record repo as recent");
        }

        let mut cached = load_from_path(&path).recent_repos;
        assert_eq!(cached.len(), MAX_RECENT_REPOS);

        // The one the cap pushed off comes back to the front, on both sides.
        let evicted = repos[0].clone();
        assert!(!cached.contains(&evicted));
        promote_recent_repo(&mut cached, &evicted);
        persist_recent_repo_to_path(&evicted, &path).expect("re-record the evicted repo");

        assert_eq!(
            cached.len(),
            MAX_RECENT_REPOS,
            "the in-memory list has to honour the same cap the file does"
        );
        assert_eq!(
            cached,
            load_from_path(&path).recent_repos,
            "a promoted cache must match what the next load returns"
        );

        // Re-promoting something already listed moves it without growing the list.
        let already_listed = cached[3].clone();
        promote_recent_repo(&mut cached, &already_listed);
        assert_eq!(cached.len(), MAX_RECENT_REPOS);
        assert_eq!(cached.first(), Some(&already_listed));
        assert_eq!(
            cached
                .iter()
                .filter(|path| **path == already_listed)
                .count(),
            1
        );
    }

    #[test]
    fn persist_ui_settings_round_trips_repo_picker_collapsed_sections() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-picker-collapse-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        assert!(
            load_from_path(&path)
                .repo_picker_collapsed_sections
                .is_empty()
        );

        let collapsed = BTreeSet::from(["open".to_string(), "recently_closed".to_string()]);
        persist_ui_settings_to_path(
            UiSettings {
                repo_picker_collapsed_sections: Some(collapsed.clone()),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist collapsed sections");
        assert_eq!(
            load_from_path(&path).repo_picker_collapsed_sections,
            collapsed
        );

        // An unrelated write must leave the collapse state alone.
        persist_ui_settings_to_path(
            UiSettings {
                repo_picker_sort: Some("name".to_string()),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist unrelated setting");
        assert_eq!(
            load_from_path(&path).repo_picker_collapsed_sections,
            collapsed
        );
    }

    #[test]
    fn remove_recent_repo_drops_entries_written_uncanonicalized() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-remove-recent-legacy-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        let repo = dir.join("repo-a");
        let _ = fs::create_dir_all(&repo);
        let canonical = gitcomet_core::path_utils::canonicalize_or_original(repo.clone());
        // Only meaningful where the temp directory is reached through a symlink
        // (macOS /var -> /private/var); elsewhere the two forms coincide.
        if canonical == repo {
            return;
        }

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                // The uncanonicalized form an older build would have written.
                recent_repos: Some(vec![path_storage_key(&repo)]),
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        remove_recent_repo_to_path(&repo, &path).expect("remove legacy recent repo");

        let loaded = load_from_path(&path);
        assert!(
            loaded.recent_repos.is_empty(),
            "legacy uncanonicalized entry should have been removed, got {:?}",
            loaded.recent_repos
        );
    }

    /// The mirror of the case above: the caller spells the repository one way
    /// and the stored entry spells it another. Matching on the caller's key
    /// alone misses it, so removal normalizes the stored side too.
    #[test]
    fn remove_recent_repo_matches_an_entry_spelled_through_a_symlink() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-remove-recent-normalized-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        let repo = dir.join("repo-a");
        let _ = fs::create_dir_all(&repo);
        let canonical = gitcomet_core::path_utils::canonicalize_or_original(repo.clone());
        // Only meaningful where the temp directory is reached through a symlink
        // (macOS /var -> /private/var); elsewhere the two forms coincide.
        if canonical == repo {
            return;
        }

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                // Stored uncanonicalized, while the caller below passes the
                // canonical form -- so neither of the two keys built from the
                // caller's path matches this string.
                recent_repos: Some(vec![path_storage_key(&repo)]),
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        remove_recent_repo_to_path(&canonical, &path).expect("remove recent repo");

        let loaded = load_from_path(&path);
        assert!(
            loaded.recent_repos.is_empty(),
            "an entry that resolves to the same directory should have been removed, got {:?}",
            loaded.recent_repos
        );
    }

    /// Storage keys are not paths: a non-UTF-8 workdir is stored hex-encoded
    /// behind [`SESSION_PATH_BYTES_PREFIX`], so normalizing an entry has to run
    /// it back through [`path_from_storage_key`] first. Reading the encoded key
    /// as a literal path makes it look relative, and the entry is skipped.
    ///
    /// Exercised with an encoded key for a *UTF-8* path, which the encoder
    /// itself never produces but a hand-edited or older file can hold: APFS
    /// rejects the invalid bytes outright, so a genuinely non-UTF-8 directory
    /// cannot be created to test against.
    #[cfg(unix)]
    #[test]
    fn remove_recent_repo_normalizes_an_encoded_entry_through_its_decoder() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-remove-recent-encoded-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        let repo = dir.join("repo-a");
        let _ = fs::create_dir_all(&repo);
        let canonical = gitcomet_core::path_utils::canonicalize_or_original(repo.clone());
        // Only meaningful where the two spellings differ (macOS /var -> /private/var):
        // the entry has to need normalizing, not just decoding.
        if canonical == repo {
            return;
        }

        let encoded = format!(
            "{SESSION_PATH_BYTES_PREFIX}{}",
            hex_encode(repo.as_os_str().as_encoded_bytes())
        );
        assert_eq!(
            path_from_storage_key(&encoded),
            repo,
            "the fixture has to decode back to the repository it names"
        );

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                recent_repos: Some(vec![encoded]),
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        remove_recent_repo_to_path(&canonical, &path).expect("remove recent repo");

        let loaded = load_from_path(&path);
        assert!(
            loaded.recent_repos.is_empty(),
            "an encoded entry naming the same directory should have been removed, got {:?}",
            loaded.recent_repos
        );
    }

    /// Entries a hand-edited file can hold that must never be resolved against
    /// the process working directory.
    #[test]
    fn remove_recent_repo_leaves_relative_entries_alone() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-remove-recent-relative-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                recent_repos: Some(vec![".".to_string(), "../elsewhere".to_string()]),
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        // `.` resolves to whatever directory the test process happens to be in.
        // Removing some unrelated repository must not take it with it.
        remove_recent_repo_to_path(&dir.join("repo-a"), &path).expect("remove recent repo");

        let loaded = load_from_path(&path);
        assert_eq!(
            loaded.recent_repos.len(),
            2,
            "relative entries must survive an unrelated removal, got {:?}",
            loaded.recent_repos
        );
    }

    #[test]
    fn persist_recent_repo_truncates_to_max_entries_and_skips_blank_values() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-recent-repo-truncate-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");
        let repo_new = dir.join("repo-new");

        let mut recent_repos = vec!["   ".to_string()];
        recent_repos.extend(
            (0..MAX_RECENT_REPOS).map(|ix| path_storage_key(&dir.join(format!("repo-{ix}")))),
        );

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                recent_repos: Some(recent_repos),
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_recent_repo_to_path(&repo_new, &path).expect("persist latest repo");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.recent_repos.len(), MAX_RECENT_REPOS);
        assert_eq!(loaded.recent_repos.first(), Some(&repo_new));
        assert_eq!(
            loaded.recent_repos.last(),
            Some(&dir.join(format!("repo-{}", MAX_RECENT_REPOS - 2)))
        );
        assert!(
            !loaded
                .recent_repos
                .contains(&dir.join(format!("repo-{}", MAX_RECENT_REPOS - 1)))
        );
        assert!(
            !loaded
                .recent_repos
                .iter()
                .any(|path| path.as_os_str().is_empty())
        );
    }

    #[test]
    fn load_from_path_filters_blank_and_duplicate_recent_repos() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-recent-repo-load-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        let repo_a = dir.join("repo-a");
        let repo_b = dir.join("repo-b");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                recent_repos: Some(vec![
                    "   ".to_string(),
                    path_storage_key(&repo_a),
                    path_storage_key(&repo_a),
                    path_storage_key(&repo_b),
                    "".to_string(),
                ]),
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.recent_repos, vec![repo_a, repo_b]);
    }

    #[test]
    fn persist_ui_settings_round_trips_repo_sidebar_collapsed_items() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");
        let repo_a = dir.join("repo-a");
        let repo_b = dir.join("repo-b");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        let mut repo_sidebar_collapsed_items = BTreeMap::new();
        repo_sidebar_collapsed_items.insert(
            repo_a.clone(),
            BTreeSet::from([
                "section:branches".to_string(),
                "group:local:feature".to_string(),
            ]),
        );
        repo_sidebar_collapsed_items.insert(
            repo_b.clone(),
            BTreeSet::from(["section:worktrees".to_string()]),
        );

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: Some(repo_sidebar_collapsed_items.clone()),
                theme_mode: None,
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: None,
                date_time_format: None,
                timezone: None,
                show_timezone: None,
                change_tracking_view: None,
                diff_scroll_sync: None,
                change_tracking_height: None,
                untracked_height: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: None,
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(
            loaded.repo_sidebar_collapsed_items,
            repo_sidebar_collapsed_items
        );
    }

    #[test]
    fn persist_ui_settings_round_trips_repo_sidebar_pinned_branches() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");
        let repo_a = dir.join("repo-a");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        let mut repo_sidebar_pinned_branches = BTreeMap::new();
        repo_sidebar_pinned_branches.insert(
            repo_a.clone(),
            BTreeSet::from(["local:main".to_string(), "remote:origin/main".to_string()]),
        );

        persist_ui_settings_to_path(
            UiSettings {
                repo_sidebar_pinned_branches: Some(repo_sidebar_pinned_branches.clone()),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(
            loaded.repo_sidebar_pinned_branches,
            repo_sidebar_pinned_branches
        );
    }

    #[test]
    fn persist_ui_settings_round_trips_date_time_format() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: None,
                theme_mode: None,
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: None,
                date_time_format: Some("ymd_hm_utc".to_string()),
                timezone: None,
                show_timezone: None,
                change_tracking_view: None,
                diff_scroll_sync: None,
                change_tracking_height: None,
                untracked_height: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: None,
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.date_time_format.as_deref(), Some("ymd_hm_utc"));
    }

    #[test]
    fn persist_ui_settings_round_trips_show_timezone() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: None,
                theme_mode: None,
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: Some(false),
                date_time_format: None,
                timezone: None,
                show_timezone: Some(false),
                change_tracking_view: None,
                diff_scroll_sync: None,
                change_tracking_height: None,
                untracked_height: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: None,
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.show_timezone, Some(false));
    }

    #[test]
    fn persist_ui_settings_round_trips_font_ligatures() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: None,
                theme_mode: None,
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: Some(true),
                date_time_format: None,
                timezone: None,
                show_timezone: None,
                change_tracking_view: None,
                diff_scroll_sync: None,
                change_tracking_height: None,
                untracked_height: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: None,
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.use_font_ligatures, Some(true));
    }

    #[test]
    fn persist_ui_settings_round_trips_change_tracking_view() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: None,
                theme_mode: None,
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: None,
                date_time_format: None,
                timezone: None,
                show_timezone: None,
                change_tracking_view: Some("split_untracked".to_string()),
                diff_scroll_sync: None,
                change_tracking_height: None,
                untracked_height: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: None,
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(
            loaded.change_tracking_view.as_deref(),
            Some("split_untracked")
        );
    }

    #[test]
    fn persist_ui_settings_round_trips_diff_scroll_sync() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: None,
                theme_mode: None,
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: None,
                date_time_format: None,
                timezone: None,
                show_timezone: None,
                change_tracking_view: None,
                diff_scroll_sync: Some("horizontal".to_string()),
                change_tracking_height: None,
                untracked_height: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: None,
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.diff_scroll_sync.as_deref(), Some("horizontal"));
    }

    #[test]
    fn persist_ui_settings_round_trips_diff_content_mode() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: None,
                theme_mode: None,
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: None,
                date_time_format: None,
                timezone: None,
                show_timezone: None,
                change_tracking_view: None,
                diff_scroll_sync: None,
                diff_content_mode: Some("changed_lines_only".to_string()),
                change_tracking_height: None,
                untracked_height: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: None,
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(
            loaded.diff_content_mode.as_deref(),
            Some("changed_lines_only")
        );
    }

    #[test]
    fn persist_ui_settings_round_trips_diff_whitespace_mode() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: None,
                theme_mode: None,
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: None,
                date_time_format: None,
                timezone: None,
                show_timezone: None,
                change_tracking_view: None,
                diff_scroll_sync: None,
                diff_content_mode: None,
                diff_whitespace_mode: Some("ignore".to_string()),
                change_tracking_height: None,
                untracked_height: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: None,
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.diff_whitespace_mode.as_deref(), Some("ignore"));
    }

    #[test]
    fn persist_ui_settings_round_trips_diff_render_settings() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                diff_reveal_whitespace_chars: Some(true),
                diff_word_wrap: Some(true),
                diff_show_line_numbers: Some(false),
                mergetool_show_line_numbers: Some(false),
                mergetool_view_three_way: Some(false),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.diff_reveal_whitespace_chars, Some(true));
        assert_eq!(loaded.diff_word_wrap, Some(true));
        assert_eq!(loaded.diff_show_line_numbers, Some(false));
        assert_eq!(loaded.mergetool_show_line_numbers, Some(false));
        assert_eq!(loaded.mergetool_view_three_way, Some(false));
    }

    #[test]
    fn persist_ui_settings_round_trips_auto_save_file_edits() {
        let dir = unique_session_test_dir("auto-save-file-edits");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        // Absent from the file means "not chosen yet", which the UI reads as off.
        assert_eq!(load_from_path(&path).auto_save_file_edits, None);

        persist_ui_settings_to_path(
            UiSettings {
                auto_save_file_edits: Some(true),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");
        assert_eq!(load_from_path(&path).auto_save_file_edits, Some(true));

        // A later write that says nothing about the toggle must not clear it.
        persist_ui_settings_to_path(
            UiSettings {
                diff_word_wrap: Some(true),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist unrelated ui settings");
        assert_eq!(load_from_path(&path).auto_save_file_edits, Some(true));

        persist_ui_settings_to_path(
            UiSettings {
                auto_save_file_edits: Some(false),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");
        assert_eq!(load_from_path(&path).auto_save_file_edits, Some(false));
    }

    #[test]
    fn persist_ui_settings_round_trips_change_tracking_heights() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: None,
                theme_mode: None,
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: None,
                date_time_format: None,
                timezone: None,
                show_timezone: None,
                change_tracking_view: None,
                diff_scroll_sync: None,
                change_tracking_height: Some(222),
                untracked_height: Some(111),
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: None,
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.change_tracking_height, Some(222));
        assert_eq!(loaded.untracked_height, Some(111));
    }

    #[test]
    fn persist_ui_settings_round_trips_theme_mode() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: None,
                theme_mode: Some("dark".to_string()),
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: None,
                date_time_format: None,
                timezone: None,
                show_timezone: None,
                change_tracking_view: None,
                diff_scroll_sync: None,
                change_tracking_height: None,
                untracked_height: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: None,
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.theme_mode.as_deref(), Some("dark"));
    }

    #[test]
    fn persist_ui_settings_round_trips_terminal_preferences() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                terminal_external_mode: Some("custom_program".to_string()),
                terminal_external_program: Some("wezterm".to_string()),
                terminal_external_args: Some(vec![
                    "start".to_string(),
                    "--cwd".to_string(),
                    "{cwd}".to_string(),
                ]),
                terminal_action_bar_target: Some("external".to_string()),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(
            loaded.terminal_external_mode.as_deref(),
            Some("custom_program")
        );
        assert_eq!(loaded.terminal_external_program.as_deref(), Some("wezterm"));
        assert_eq!(
            loaded.terminal_external_args,
            Some(vec![
                "start".to_string(),
                "--cwd".to_string(),
                "{cwd}".to_string()
            ])
        );
        assert_eq!(
            loaded.terminal_action_bar_target.as_deref(),
            Some("external")
        );
    }

    #[test]
    fn persist_ui_settings_round_trips_ui_scale_percent() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                ui_scale_percent: Some(125),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.ui_scale_percent, Some(125));
    }

    #[test]
    fn persist_ui_settings_round_trips_empty_custom_git_executable_path() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                window_width: None,
                window_height: None,
                sidebar_width: None,
                details_width: None,
                repo_sidebar_collapsed_items: None,
                theme_mode: None,
                ui_font_family: None,
                editor_font_family: None,
                use_font_ligatures: None,
                date_time_format: None,
                timezone: None,
                show_timezone: None,
                change_tracking_view: None,
                diff_scroll_sync: None,
                change_tracking_height: None,
                untracked_height: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
                git_executable_path: Some(Some(PathBuf::new())),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.git_executable_path, Some(PathBuf::new()));
    }

    #[test]
    fn persist_ui_settings_round_trips_commit_push_after_enabled() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-ui-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_ui_settings_to_path(
            UiSettings {
                commit_push_after_enabled: Some(true),
                ..UiSettings::default()
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.commit_push_after_enabled, Some(true));
    }

    #[test]
    fn persist_repo_history_scope_round_trips() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-repo-history-scope-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let session_path = dir.join("session.json");

        let repo_a = dir.join("repo-a");
        let _ = fs::create_dir_all(&repo_a);

        persist_to_path(
            &session_path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFile::default()
            },
        )
        .expect("seed session file");

        persist_repo_history_scope_to_path(&repo_a, LogScope::AllBranches, &session_path)
            .expect("persist repo history scope");

        let loaded = load_repo_history_scope_from_path(&repo_a, &session_path);
        assert_eq!(loaded, Some(LogScope::AllBranches));
    }

    #[test]
    fn persist_repo_history_scope_skips_rewriting_unchanged_value() {
        let dir = env::temp_dir().join(format!(
            "gitcomet-repo-history-scope-noop-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let session_path = dir.join("session.json");
        let repo_a = dir.join("repo-a");
        let _ = fs::create_dir_all(&repo_a);

        persist_repo_history_scope_to_path(&repo_a, LogScope::AllBranches, &session_path)
            .expect("persist repo history scope");

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;

            let metadata_before = fs::metadata(&session_path).expect("session metadata before");
            let inode_before = metadata_before.ino();

            persist_repo_history_scope_to_path(&repo_a, LogScope::AllBranches, &session_path)
                .expect("persist unchanged repo history scope");

            let metadata_after = fs::metadata(&session_path).expect("session metadata after");
            assert_eq!(
                metadata_after.ino(),
                inode_before,
                "unchanged history scope should not rewrite the session file"
            );
        }

        #[cfg(not(unix))]
        {
            let contents_before = fs::read(&session_path).expect("session bytes before");

            persist_repo_history_scope_to_path(&repo_a, LogScope::AllBranches, &session_path)
                .expect("persist unchanged repo history scope");

            let contents_after = fs::read(&session_path).expect("session bytes after");
            assert_eq!(
                contents_after, contents_before,
                "unchanged history scope should not rewrite the session file"
            );
        }
    }
}
