use super::*;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(transparent)]
pub struct WorkspaceId(Uuid);

impl WorkspaceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub const fn from_u128(value: u128) -> Self {
        Self(Uuid::from_u128(value))
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for WorkspaceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceColor {
    Gray,
    Brown,
    Red,
    Orange,
    Yellow,
    Lime,
    Green,
    Teal,
    Cyan,
    Blue,
    Indigo,
    Purple,
    Magenta,
    Pink,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedWindowState {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SavedWindowFrame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Window edges a tiling window manager had snapped to when last captured.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default)]
pub struct SavedWindowTiling {
    pub top: bool,
    pub left: bool,
    pub right: bool,
    pub bottom: bool,
}

// Session parsing is lenient: a bad value here resets only that value.
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default)]
pub struct PortableWindowPlacement {
    #[serde(deserialize_with = "lenient")]
    pub normal_frame: Option<SavedWindowFrame>,
    #[serde(deserialize_with = "lenient")]
    pub captured_visible_frame: Option<SavedWindowFrame>,
    #[serde(deserialize_with = "lenient")]
    pub display_id: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub state: SavedWindowState,
    /// Diagnostic only: tiling and snapping belong to the window manager and
    /// cannot be requested back, so restore uses the last frame instead.
    #[serde(deserialize_with = "lenient")]
    pub tiled: Option<SavedWindowTiling>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default)]
pub struct WorkspaceLayout {
    pub sidebar_width: Option<u32>,
    pub details_width: Option<u32>,
    pub sidebar_collapsed: bool,
    pub change_tracking_height: Option<u32>,
    pub untracked_height: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub custom_name: Option<String>,
    pub color: Option<WorkspaceColor>,
    pub repositories: Vec<PathBuf>,
    pub active_repository: Option<PathBuf>,
    pub restore_on_launch: bool,
    pub last_activation_order: u64,
    pub layout: WorkspaceLayout,
    pub placement: PortableWindowPlacement,
    /// Theme override in the global `theme_mode` key space; None follows the app.
    pub theme_mode: Option<String>,
    /// Unix seconds.
    pub created_at: Option<u64>,
    /// Unix seconds; bumped whenever the workspace's window gains focus.
    pub last_opened_at: Option<u64>,
}

pub fn unix_time_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn is_customized_parts(
    custom_name: Option<&str>,
    color: Option<WorkspaceColor>,
    theme_mode: Option<&str>,
) -> bool {
    custom_name.is_some_and(|name| !name.trim().is_empty())
        || color.is_some()
        || theme_mode.is_some_and(|mode| !mode.trim().is_empty())
}

impl Workspace {
    pub fn new(repositories: Vec<PathBuf>) -> Self {
        let active_repository = repositories.first().cloned();
        Self {
            id: WorkspaceId::new(),
            custom_name: None,
            color: None,
            repositories,
            active_repository,
            restore_on_launch: true,
            last_activation_order: 0,
            layout: WorkspaceLayout::default(),
            placement: PortableWindowPlacement::default(),
            theme_mode: None,
            created_at: Some(unix_time_now()),
            last_opened_at: None,
        }
    }

    /// Personalized workspaces outlive their last repository; anonymous ones
    /// are deleted with it.
    pub fn is_customized(&self) -> bool {
        is_customized_parts(
            self.custom_name.as_deref(),
            self.color,
            self.theme_mode.as_deref(),
        )
    }

    pub fn display_name(&self) -> String {
        if let Some(name) = self
            .custom_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            return name.to_string();
        }

        // The first tab in the strip, not the active one: following the
        // active tab would rename the workspace (and resize its title-bar
        // chip, shifting the tabs) every time the user switches repository.
        let base = self
            .repositories
            .first()
            .and_then(|path| path.file_name())
            .and_then(OsStr::to_str)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "Workspace".to_string());
        let other_count = self.repositories.len().saturating_sub(1);
        if other_count == 0 {
            base
        } else {
            format!("{base} +{other_count}")
        }
    }
}

/// Only `id` is required; an entry that still fails to parse is dropped alone
/// (see [`lenient_workspaces`]).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct WorkspaceFile {
    pub(super) id: WorkspaceId,
    #[serde(default)]
    pub(super) custom_name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub(super) color: Option<WorkspaceColor>,
    #[serde(default)]
    pub(super) repositories: Vec<String>,
    #[serde(default)]
    pub(super) active_repository: Option<String>,
    #[serde(default = "restore_on_launch_default")]
    pub(super) restore_on_launch: bool,
    #[serde(default)]
    pub(super) last_activation_order: u64,
    #[serde(default, deserialize_with = "lenient")]
    pub(super) layout: WorkspaceLayout,
    #[serde(default, deserialize_with = "lenient")]
    pub(super) placement: PortableWindowPlacement,
    #[serde(default)]
    pub(super) theme_mode: Option<String>,
    #[serde(default)]
    pub(super) created_at: Option<u64>,
    #[serde(default)]
    pub(super) last_opened_at: Option<u64>,
}

const fn restore_on_launch_default() -> bool {
    true
}

/// An unknown or malformed value falls back to its default.
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Drops workspace entries that fail to parse instead of failing the file.
pub(super) fn lenient_workspaces<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<WorkspaceFile>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let serde_json::Value::Array(entries) = serde_json::Value::deserialize(deserializer)? else {
        return Ok(None);
    };
    Ok(Some(
        entries
            .into_iter()
            .filter_map(|entry| serde_json::from_value(entry).ok())
            .collect(),
    ))
}

impl WorkspaceFile {
    fn is_customized(&self) -> bool {
        is_customized_parts(
            self.custom_name.as_deref(),
            self.color,
            self.theme_mode.as_deref(),
        )
    }
}

pub fn persist_workspaces(workspaces: &[Workspace]) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    persist_workspaces_to_path(workspaces, &path)
}

pub fn persist_workspaces_to_path(workspaces: &[Workspace], path: &Path) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        let stored_workspaces = workspaces_to_file(workspaces);

        // Keep the legacy projection coherent while the UI transition is in
        // progress and for diagnostics/performance tooling that still reads
        // the old single-window fields. Closed workspaces must not leak into this
        // projection or they would be restored by an older launch path.
        // An empty (customized) workspace must not blank the projection.
        let projected = stored_workspaces
            .iter()
            .filter(|workspace| workspace.restore_on_launch && !workspace.repositories.is_empty())
            .max_by_key(|workspace| workspace.last_activation_order);
        if let Some(workspace) = projected {
            file.open_repos.clone_from(&workspace.repositories);
            file.active_repo.clone_from(&workspace.active_repository);
            file.sidebar_width = workspace.layout.sidebar_width;
            file.details_width = workspace.layout.details_width;
            file.sidebar_collapsed = Some(workspace.layout.sidebar_collapsed);
            file.change_tracking_height = workspace.layout.change_tracking_height;
            file.untracked_height = workspace.layout.untracked_height;
            if let Some(frame) = workspace.placement.normal_frame {
                file.window_width = Some(frame.width);
                file.window_height = Some(frame.height);
            }
        } else {
            file.open_repos.clear();
            file.active_repo = None;
        }
        file.workspaces = Some(stored_workspaces);

        persist_to_path(path, &file)
    })
}

pub(super) fn parse_workspaces(workspaces: Vec<WorkspaceFile>) -> Vec<Workspace> {
    let mut parsed = Vec::with_capacity(workspaces.len());
    let mut seen_ids = FxHashSet::default();
    for workspace in workspaces {
        if !seen_ids.insert(workspace.id) {
            continue;
        }
        let customized = workspace.is_customized();
        let repositories = parse_path_list(workspace.repositories);
        if repositories.is_empty() && !customized {
            continue;
        }
        let active_repository = workspace
            .active_repository
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(path_from_storage_key)
            .filter(|path| repositories.contains(path));
        parsed.push(Workspace {
            id: workspace.id,
            custom_name: workspace.custom_name.and_then(non_empty_string),
            color: workspace.color,
            repositories,
            active_repository,
            restore_on_launch: workspace.restore_on_launch,
            last_activation_order: workspace.last_activation_order,
            layout: workspace.layout,
            placement: workspace.placement,
            theme_mode: workspace.theme_mode.and_then(non_empty_string),
            created_at: workspace.created_at,
            last_opened_at: workspace.last_opened_at,
        });
    }
    parsed
}

pub(super) fn workspaces_to_file(workspaces: &[Workspace]) -> Vec<WorkspaceFile> {
    let mut stored = Vec::with_capacity(workspaces.len());
    let mut seen_ids = FxHashSet::default();
    for workspace in workspaces {
        if !seen_ids.insert(workspace.id) {
            continue;
        }
        let repositories = parse_path_list(
            workspace
                .repositories
                .iter()
                .map(|path| path_storage_key(path))
                .collect(),
        );
        if repositories.is_empty() && !workspace.is_customized() {
            continue;
        }
        let active_repository = workspace
            .active_repository
            .as_ref()
            .filter(|active| repositories.contains(active))
            .map(|path| path_storage_key(path));
        stored.push(WorkspaceFile {
            id: workspace.id,
            custom_name: workspace.custom_name.clone().and_then(non_empty_string),
            color: workspace.color,
            repositories: repositories
                .iter()
                .map(|path| path_storage_key(path))
                .collect(),
            active_repository,
            restore_on_launch: workspace.restore_on_launch,
            last_activation_order: workspace.last_activation_order,
            layout: workspace.layout.clone(),
            placement: workspace.placement.clone(),
            theme_mode: workspace.theme_mode.clone().and_then(non_empty_string),
            created_at: workspace.created_at,
            last_opened_at: workspace.last_opened_at,
        });
    }
    stored
}

/// The one workspace a file without a `workspaces` array implies. Derived on
/// read only: that array appears once the UI saves workspaces, and from then
/// on store snapshots (other windows, a mergetool process) cannot rewrite it.
pub(super) fn legacy_workspace_from_projection(file: &UiSessionFile) -> Option<WorkspaceFile> {
    if file.open_repos.iter().all(|path| path.trim().is_empty()) {
        return None;
    }
    Some(WorkspaceFile {
        id: LEGACY_WORKSPACE_ID,
        custom_name: None,
        color: None,
        repositories: file.open_repos.clone(),
        active_repository: file.active_repo.clone(),
        restore_on_launch: true,
        last_activation_order: 1,
        layout: WorkspaceLayout {
            sidebar_width: file.sidebar_width,
            details_width: file.details_width,
            sidebar_collapsed: file.sidebar_collapsed.unwrap_or(false),
            change_tracking_height: file.change_tracking_height,
            untracked_height: file.untracked_height,
        },
        placement: PortableWindowPlacement::default(),
        theme_mode: None,
        created_at: None,
        last_opened_at: None,
    })
}

/// Keep one copy of a session file written by an older format version, so a
/// downgrade can recover it (older builds drop files with a newer version).
pub(super) fn preserve_previous_version_session_backup(
    path: &Path,
    replacement: &[u8],
) -> io::Result<()> {
    // Writers load the file first; a file already current needs no re-read.
    if loaded_session_version(path).is_some_and(|version| version >= CURRENT_SESSION_FILE_VERSION) {
        return Ok(());
    }
    let replacement_version = serde_json::from_slice::<serde_json::Value>(replacement)
        .ok()
        .and_then(|value| value.get("version").and_then(|version| version.as_u64()));
    if replacement_version != Some(CURRENT_SESSION_FILE_VERSION as u64) {
        return Ok(());
    }

    let Ok(previous) = fs::read(path) else {
        return Ok(());
    };
    let previous_version = serde_json::from_slice::<serde_json::Value>(&previous)
        .ok()
        .and_then(|value| value.get("version").and_then(|version| version.as_u64()))
        .unwrap_or(SESSION_FILE_VERSION_V1 as u64);
    if previous_version >= CURRENT_SESSION_FILE_VERSION as u64 {
        return Ok(());
    }

    let mut backup_name = path.as_os_str().to_os_string();
    backup_name.push(format!(".v{previous_version}.bak"));
    let backup_path = PathBuf::from(backup_name);
    if backup_path.exists() {
        return Ok(());
    }
    gitcomet_core::fs_utils::write_private_file(&backup_path, &previous)
}
