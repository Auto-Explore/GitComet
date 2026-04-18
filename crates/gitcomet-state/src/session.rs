use crate::model::{AppState, GitLogTagFetchMode, RepoId};
use gitcomet_core::domain::{HistoryMode, LogScope};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{env, fs, io};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiSession {
    pub open_repos: Vec<PathBuf>,
    pub active_repo: Option<PathBuf>,
    pub recent_repos: Vec<PathBuf>,
    pub repo_sidebar_collapsed_items: BTreeMap<PathBuf, BTreeSet<String>>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub sidebar_width: Option<u32>,
    pub details_width: Option<u32>,
    pub theme_mode: Option<String>,
    pub ui_font_family: Option<String>,
    pub editor_font_family: Option<String>,
    pub use_font_ligatures: Option<bool>,
    pub date_time_format: Option<String>,
    pub timezone: Option<String>,
    pub show_timezone: Option<bool>,
    pub change_tracking_view: Option<String>,
    pub diff_scroll_sync: Option<String>,
    pub change_tracking_height: Option<u32>,
    pub untracked_height: Option<u32>,
    pub history_show_graph: Option<bool>,
    pub history_show_author: Option<bool>,
    pub history_show_date: Option<bool>,
    pub history_show_sha: Option<bool>,
    pub history_show_tags: Option<bool>,
    pub history_tag_fetch_mode: Option<GitLogTagFetchMode>,
    pub default_history_mode: Option<HistoryMode>,
    pub git_executable_path: Option<PathBuf>,
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
struct UiSessionFileV2 {
    version: u32,
    open_repos: Vec<String>,
    active_repo: Option<String>,
    recent_repos: Option<Vec<String>>,
    repo_sidebar_collapsed_items: Option<BTreeMap<String, BTreeSet<String>>>,
    window_width: Option<u32>,
    window_height: Option<u32>,
    sidebar_width: Option<u32>,
    details_width: Option<u32>,
    theme_mode: Option<String>,
    ui_font_family: Option<String>,
    editor_font_family: Option<String>,
    use_font_ligatures: Option<bool>,
    date_time_format: Option<String>,
    timezone: Option<String>,
    show_timezone: Option<bool>,
    change_tracking_view: Option<String>,
    diff_scroll_sync: Option<String>,
    change_tracking_height: Option<u32>,
    untracked_height: Option<u32>,
    history_show_graph: Option<bool>,
    history_show_author: Option<bool>,
    history_show_date: Option<bool>,
    history_show_sha: Option<bool>,
    history_show_tags: Option<bool>,
    history_tag_fetch_mode: Option<GitLogTagFetchMode>,
    default_history_mode: Option<HistoryModeSetting>,
    git_executable_path: Option<String>,
    repo_history_modes: Option<BTreeMap<String, HistoryModeSetting>>,
    repo_history_scopes: Option<BTreeMap<String, HistoryScopeSetting>>,
    repo_fetch_prune_deleted_remote_tracking_branches: Option<BTreeMap<String, bool>>,
}

const SESSION_FILE_VERSION_V1: u32 = 1;
const SESSION_FILE_VERSION_V2: u32 = 2;
const CURRENT_SESSION_FILE_VERSION: u32 = SESSION_FILE_VERSION_V2;
const MAX_RECENT_REPOS: usize = 15;
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
    let Some(file) = load_file_v2(path) else {
        return UiSession::default();
    };

    let (open_repos, active_repo) = parse_repos(file.open_repos, file.active_repo);
    let recent_repos = parse_path_list(file.recent_repos.unwrap_or_default());
    let repo_sidebar_collapsed_items =
        parse_path_keyed_string_sets(file.repo_sidebar_collapsed_items.unwrap_or_default());
    UiSession {
        open_repos,
        active_repo,
        recent_repos,
        repo_sidebar_collapsed_items,
        window_width: file.window_width,
        window_height: file.window_height,
        sidebar_width: file.sidebar_width,
        details_width: file.details_width,
        theme_mode: file.theme_mode,
        ui_font_family: file.ui_font_family,
        editor_font_family: file.editor_font_family,
        use_font_ligatures: file.use_font_ligatures,
        date_time_format: file.date_time_format,
        timezone: file.timezone,
        show_timezone: file.show_timezone,
        change_tracking_view: file.change_tracking_view,
        diff_scroll_sync: file.diff_scroll_sync,
        change_tracking_height: file.change_tracking_height,
        untracked_height: file.untracked_height,
        history_show_graph: file.history_show_graph,
        history_show_author: file.history_show_author,
        history_show_date: file.history_show_date,
        history_show_sha: file.history_show_sha,
        history_show_tags: file.history_show_tags,
        history_tag_fetch_mode: file.history_tag_fetch_mode,
        default_history_mode: file.default_history_mode.map(Into::into),
        git_executable_path: file
            .git_executable_path
            .as_deref()
            .map(path_from_storage_key),
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
    let mut file = load_file_v2(path).unwrap_or_default();
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
}

pub fn persist_recent_repo(workdir: &Path) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    persist_recent_repo_to_path(workdir, &path)
}

pub fn persist_recent_repo_to_path(workdir: &Path, session_file_path: &Path) -> io::Result<()> {
    let mut file = load_file_v2(session_file_path).unwrap_or_default();
    file.version = CURRENT_SESSION_FILE_VERSION;

    let workdir_key = path_storage_key(workdir);
    let recent_repos = file.recent_repos.get_or_insert_with(Vec::new);
    recent_repos.retain(|path| path.trim() != workdir_key);
    recent_repos.retain(|path| !path.trim().is_empty());
    recent_repos.insert(0, workdir_key);
    if recent_repos.len() > MAX_RECENT_REPOS {
        recent_repos.truncate(MAX_RECENT_REPOS);
    }

    persist_to_path(session_file_path, &file)
}

pub fn remove_recent_repo(workdir: &Path) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    remove_recent_repo_to_path(workdir, &path)
}

pub fn remove_recent_repo_to_path(workdir: &Path, session_file_path: &Path) -> io::Result<()> {
    let mut file = load_file_v2(session_file_path).unwrap_or_default();
    file.version = CURRENT_SESSION_FILE_VERSION;

    let workdir_key = path_storage_key(workdir);
    let Some(recent_repos) = file.recent_repos.as_mut() else {
        return Ok(());
    };
    recent_repos.retain(|path| path.trim() != workdir_key);

    persist_to_path(session_file_path, &file)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiSettings {
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub sidebar_width: Option<u32>,
    pub details_width: Option<u32>,
    pub repo_sidebar_collapsed_items: Option<BTreeMap<PathBuf, BTreeSet<String>>>,
    pub theme_mode: Option<String>,
    pub ui_font_family: Option<String>,
    pub editor_font_family: Option<String>,
    pub use_font_ligatures: Option<bool>,
    pub date_time_format: Option<String>,
    pub timezone: Option<String>,
    pub show_timezone: Option<bool>,
    pub change_tracking_view: Option<String>,
    pub diff_scroll_sync: Option<String>,
    pub change_tracking_height: Option<u32>,
    pub untracked_height: Option<u32>,
    pub history_show_graph: Option<bool>,
    pub history_show_author: Option<bool>,
    pub history_show_date: Option<bool>,
    pub history_show_sha: Option<bool>,
    pub history_show_tags: Option<bool>,
    pub history_tag_fetch_mode: Option<GitLogTagFetchMode>,
    pub default_history_mode: Option<HistoryMode>,
    pub git_executable_path: Option<Option<PathBuf>>,
}

pub fn persist_ui_settings(settings: UiSettings) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    persist_ui_settings_to_path(settings, &path)
}

pub fn persist_ui_settings_to_path(settings: UiSettings, path: &Path) -> io::Result<()> {
    let mut file = load_file_v2(path).unwrap_or_default();
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
    if let Some(items) = settings.repo_sidebar_collapsed_items {
        let items = path_keyed_string_sets_to_storage(items);
        file.repo_sidebar_collapsed_items = (!items.is_empty()).then_some(items);
    }
    if let Some(theme_mode) = settings.theme_mode {
        file.theme_mode = Some(theme_mode);
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
    if let Some(value) = settings.diff_scroll_sync {
        file.diff_scroll_sync = Some(value);
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
    if let Some(value) = settings.history_show_tags {
        file.history_show_tags = Some(value);
    }
    if let Some(value) = settings.history_tag_fetch_mode {
        file.history_tag_fetch_mode = Some(value);
    }
    if let Some(value) = settings.default_history_mode {
        file.default_history_mode = Some(value.into());
    }
    if let Some(path) = settings.git_executable_path {
        file.git_executable_path = path.map(|path| path_storage_key(&path));
    }

    persist_to_path(path, &file)
}

pub fn load_default_history_mode() -> Option<HistoryMode> {
    let session_file_path = default_session_file_path()?;
    load_default_history_mode_from_path(&session_file_path)
}

pub fn load_default_history_mode_from_path(session_file_path: &Path) -> Option<HistoryMode> {
    let file = load_file_v2(session_file_path)?;
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
    let file = load_file_v2(session_file_path)?;
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
    let Some(file) = load_file_v2(session_file_path) else {
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

pub fn persist_repo_history_mode_to_path(
    workdir: &Path,
    mode: HistoryMode,
    session_file_path: &Path,
) -> io::Result<()> {
    let mut file = load_file_v2(session_file_path).unwrap_or_default();
    let mode = HistoryModeSetting::from(mode);

    if let Some(existing_mode) = file.repo_history_modes.as_ref().and_then(|modes| {
        workdir
            .to_str()
            .and_then(|path| modes.get(path).copied())
            .or_else(|| {
                let workdir_key = path_storage_key(workdir);
                modes.get(&workdir_key).copied()
            })
    }) && existing_mode == mode
    {
        return Ok(());
    }

    file.version = CURRENT_SESSION_FILE_VERSION;
    let workdir_key = path_storage_key(workdir);
    file.repo_history_modes
        .get_or_insert_with(BTreeMap::new)
        .insert(workdir_key, mode);

    persist_to_path(session_file_path, &file)
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
    let file = load_file_v2(session_file_path)?;
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
    let Some(file) = load_file_v2(session_file_path) else {
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
    let mut file = load_file_v2(session_file_path).unwrap_or_default();
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
    let file = load_file_v2(session_file_path)?;
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
    let Some(file) = load_file_v2(session_file_path) else {
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
    let mut file = load_file_v2(session_file_path).unwrap_or_default();
    file.version = CURRENT_SESSION_FILE_VERSION;
    let workdir_key = path_storage_key(workdir);
    file.repo_fetch_prune_deleted_remote_tracking_branches
        .get_or_insert_with(BTreeMap::new)
        .insert(workdir_key, enabled);

    persist_to_path(session_file_path, &file)
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

fn load_file_v2(path: &Path) -> Option<UiSessionFileV2> {
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
            Some(UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: file.open_repos,
                active_repo: file.active_repo,
                ..UiSessionFileV2::default()
            })
        }
        SESSION_FILE_VERSION_V2 => serde_json::from_value::<UiSessionFileV2>(value).ok(),
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
    use gitcomet_core::domain::LogScope;
    use gitcomet_core::domain::RepoSpec;

    fn clear_session_repos_snapshot_cache() {
        SESSION_REPOS_SNAPSHOT_CACHE.with(|cache| {
            cache.borrow_mut().take();
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
            },
        )
        .expect("seed session file");

        persist_recent_repo_to_path(&repo_a, &path).expect("persist first repo");
        persist_recent_repo_to_path(&repo_b, &path).expect("persist second repo");
        persist_recent_repo_to_path(&repo_a, &path).expect("move repo to front");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.recent_repos, vec![repo_a, repo_b]);
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                recent_repos: Some(vec![path_storage_key(&repo_a), path_storage_key(&repo_b)]),
                ..UiSessionFileV2::default()
            },
        )
        .expect("seed session file");

        remove_recent_repo_to_path(&repo_b, &path).expect("remove invalid recent repo");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.recent_repos, vec![repo_a]);
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                recent_repos: Some(recent_repos),
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
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
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
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
            &UiSessionFileV2 {
                version: CURRENT_SESSION_FILE_VERSION,
                open_repos: Vec::new(),
                active_repo: None,
                ..UiSessionFileV2::default()
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
