use crate::model::{AppState, RepoId};
use gitcomet_core::domain::LogScope;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiSession {
    pub open_repos: Vec<PathBuf>,
    pub active_repo: Option<PathBuf>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub sidebar_width: Option<u32>,
    pub details_width: Option<u32>,
    pub theme_mode: Option<String>,
    pub date_time_format: Option<String>,
    pub timezone: Option<String>,
    pub show_timezone: Option<bool>,
    pub history_show_author: Option<bool>,
    pub history_show_date: Option<bool>,
    pub history_show_sha: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum HistoryScopeSetting {
    CurrentBranch,
    AllBranches,
}

impl From<LogScope> for HistoryScopeSetting {
    fn from(value: LogScope) -> Self {
        match value {
            LogScope::CurrentBranch => Self::CurrentBranch,
            LogScope::AllBranches => Self::AllBranches,
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
    window_width: Option<u32>,
    window_height: Option<u32>,
    sidebar_width: Option<u32>,
    details_width: Option<u32>,
    theme_mode: Option<String>,
    date_time_format: Option<String>,
    timezone: Option<String>,
    show_timezone: Option<bool>,
    history_show_author: Option<bool>,
    history_show_date: Option<bool>,
    history_show_sha: Option<bool>,
    repo_history_scopes: Option<BTreeMap<String, HistoryScopeSetting>>,
    repo_fetch_prune_deleted_remote_tracking_branches: Option<BTreeMap<String, bool>>,
}

const SESSION_FILE_VERSION_V1: u32 = 1;
const SESSION_FILE_VERSION_V2: u32 = 2;
const CURRENT_SESSION_FILE_VERSION: u32 = SESSION_FILE_VERSION_V2;
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
    UiSession {
        open_repos,
        active_repo,
        window_width: file.window_width,
        window_height: file.window_height,
        sidebar_width: file.sidebar_width,
        details_width: file.details_width,
        theme_mode: file.theme_mode,
        date_time_format: file.date_time_format,
        timezone: file.timezone,
        show_timezone: file.show_timezone,
        history_show_author: file.history_show_author,
        history_show_date: file.history_show_date,
        history_show_sha: file.history_show_sha,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionReposSnapshot {
    pub open_repos: Vec<String>,
    pub active_repo: Option<String>,
}

pub fn snapshot_repos_from_state(state: &AppState) -> SessionReposSnapshot {
    let mut open_repos: Vec<String> = Vec::with_capacity(state.repos.len());
    let mut seen: FxHashSet<&Path> = FxHashSet::default();
    for repo in &state.repos {
        let workdir = repo.spec.workdir.as_path();
        if !seen.insert(workdir) {
            continue;
        }
        open_repos.push(path_storage_key(workdir));
    }

    let active_repo: Option<String> = active_repo_path(state, state.active_repo)
        .filter(|p| seen.contains(*p))
        .map(path_storage_key);

    SessionReposSnapshot {
        open_repos,
        active_repo,
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
    file.open_repos = snapshot.open_repos.clone();
    file.active_repo = snapshot.active_repo.clone();

    persist_to_path(path, &file)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiSettings {
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub sidebar_width: Option<u32>,
    pub details_width: Option<u32>,
    pub theme_mode: Option<String>,
    pub date_time_format: Option<String>,
    pub timezone: Option<String>,
    pub show_timezone: Option<bool>,
    pub history_show_author: Option<bool>,
    pub history_show_date: Option<bool>,
    pub history_show_sha: Option<bool>,
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
    if let Some(theme_mode) = settings.theme_mode {
        file.theme_mode = Some(theme_mode);
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
    if let Some(value) = settings.history_show_author {
        file.history_show_author = Some(value);
    }
    if let Some(value) = settings.history_show_date {
        file.history_show_date = Some(value);
    }
    if let Some(value) = settings.history_show_sha {
        file.history_show_sha = Some(value);
    }

    persist_to_path(path, &file)
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
    file.version = CURRENT_SESSION_FILE_VERSION;
    let workdir_key = path_storage_key(workdir);
    file.repo_history_scopes
        .get_or_insert_with(BTreeMap::new)
        .insert(workdir_key, scope.into());

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
    let mut open_repos: Vec<PathBuf> = Vec::with_capacity(open_repos_raw.len());
    let mut seen: FxHashSet<PathBuf> = FxHashSet::default();
    for repo in open_repos_raw {
        let repo = repo.trim();
        if repo.is_empty() {
            continue;
        }
        let repo = path_from_storage_key(repo);
        if !seen.insert(repo.clone()) {
            continue;
        }
        open_repos.push(repo);
    }

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

fn active_repo_path(state: &AppState, active_repo_id: Option<RepoId>) -> Option<&Path> {
    let active_repo_id = active_repo_id?;
    state
        .repos
        .iter()
        .find(|r| r.id == active_repo_id)
        .map(|r| r.spec.workdir.as_path())
}

pub(crate) fn path_storage_key(path: &Path) -> String {
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

fn path_from_storage_key(raw: &str) -> PathBuf {
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

fn app_state_dir() -> Option<PathBuf> {
    // Follow XDG on linux; otherwise fall back to platform conventions.
    #[cfg(target_os = "linux")]
    {
        if let Some(state_home) = env::var_os("XDG_STATE_HOME") {
            return Some(PathBuf::from(state_home).join("gitcomet"));
        }
        let home = env::var_os("HOME")?;
        Some(PathBuf::from(home).join(".local/state/gitcomet"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = env::var_os("HOME")?;
        return Some(PathBuf::from(home).join("Library/Application Support/gitcomet"));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = env::var_os("LOCALAPPDATA").or_else(|| env::var_os("APPDATA"))?;
        Some(PathBuf::from(appdata).join("gitcomet"))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        env::var_os("HOME").map(|home| PathBuf::from(home).join(".gitcomet"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RepoState;
    use gitcomet_core::domain::LogScope;
    use gitcomet_core::domain::RepoSpec;

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
            "/tmp/target/debug/gitcomet-app"
        )));
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
        assert_eq!(snapshot.open_repos, vec![path_storage_key(&repo_a)]);
        assert_eq!(snapshot.active_repo, None);

        let state = AppState {
            repos: vec![
                RepoState::new_opening(
                    RepoId(1),
                    RepoSpec {
                        workdir: repo_a.clone(),
                    },
                ),
                RepoState::new_opening(RepoId(2), RepoSpec { workdir: repo_b }),
            ],
            active_repo: Some(RepoId(2)),
            ..Default::default()
        };
        let snapshot = snapshot_repos_from_state(&state);
        assert_eq!(
            snapshot.active_repo,
            Some(path_storage_key(Path::new("/tmp/repo-b")))
        );
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
        assert_eq!(loaded.window_width, None);
        assert_eq!(loaded.date_time_format, None);
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
                theme_mode: None,
                date_time_format: Some("ymd_hm_utc".to_string()),
                timezone: None,
                show_timezone: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
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
                theme_mode: None,
                date_time_format: None,
                timezone: None,
                show_timezone: Some(false),
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.show_timezone, Some(false));
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
                theme_mode: Some("dark".to_string()),
                date_time_format: None,
                timezone: None,
                show_timezone: None,
                history_show_author: None,
                history_show_date: None,
                history_show_sha: None,
            },
            &path,
        )
        .expect("persist ui settings");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.theme_mode.as_deref(), Some("dark"));
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
}
