use super::*;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(transparent)]
pub struct WindowGroupId(Uuid);

impl WindowGroupId {
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

impl Default for WindowGroupId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WindowGroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowGroupColor {
    Gray,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
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

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PortableWindowPlacement {
    pub normal_frame: Option<SavedWindowFrame>,
    pub captured_visible_frame: Option<SavedWindowFrame>,
    pub display_id: Option<String>,
    pub state: SavedWindowState,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct WindowGroupLayout {
    pub sidebar_width: Option<u32>,
    pub details_width: Option<u32>,
    pub sidebar_collapsed: bool,
    pub change_tracking_height: Option<u32>,
    pub untracked_height: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SavedWindowGroup {
    pub id: WindowGroupId,
    pub custom_name: Option<String>,
    pub color: Option<WindowGroupColor>,
    pub repositories: Vec<PathBuf>,
    pub active_repository: Option<PathBuf>,
    pub restore_on_launch: bool,
    pub last_activation_order: u64,
    pub layout: WindowGroupLayout,
    pub placement: PortableWindowPlacement,
}

impl SavedWindowGroup {
    pub fn new(repositories: Vec<PathBuf>) -> Self {
        let active_repository = repositories.first().cloned();
        Self {
            id: WindowGroupId::new(),
            custom_name: None,
            color: None,
            repositories,
            active_repository,
            restore_on_launch: true,
            last_activation_order: 0,
            layout: WindowGroupLayout::default(),
            placement: PortableWindowPlacement::default(),
        }
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

        let active = self
            .active_repository
            .as_ref()
            .filter(|active| self.repositories.contains(active))
            .or_else(|| self.repositories.first());
        let base = active
            .and_then(|path| path.file_name())
            .and_then(OsStr::to_str)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "Window Group".to_string());
        let other_count = self.repositories.len().saturating_sub(1);
        if other_count == 0 {
            base
        } else {
            format!("{base} +{other_count}")
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct SavedWindowGroupFile {
    pub(super) id: WindowGroupId,
    pub(super) custom_name: Option<String>,
    pub(super) color: Option<WindowGroupColor>,
    pub(super) repositories: Vec<String>,
    pub(super) active_repository: Option<String>,
    pub(super) restore_on_launch: bool,
    pub(super) last_activation_order: u64,
    pub(super) layout: WindowGroupLayout,
    pub(super) placement: PortableWindowPlacement,
}

pub fn persist_window_groups(groups: &[SavedWindowGroup]) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    persist_window_groups_to_path(groups, &path)
}

pub fn persist_window_groups_to_path(groups: &[SavedWindowGroup], path: &Path) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        let stored_groups = window_groups_to_file(groups);

        // Keep the legacy projection coherent while the UI transition is in
        // progress and for diagnostics/performance tooling that still reads
        // the old single-window fields. Closed groups must not leak into this
        // projection or they would be restored by an older launch path.
        let projected = stored_groups
            .iter()
            .filter(|group| group.restore_on_launch)
            .max_by_key(|group| group.last_activation_order);
        if let Some(group) = projected {
            file.open_repos.clone_from(&group.repositories);
            file.active_repo.clone_from(&group.active_repository);
            file.sidebar_width = group.layout.sidebar_width;
            file.details_width = group.layout.details_width;
            file.sidebar_collapsed = Some(group.layout.sidebar_collapsed);
            file.change_tracking_height = group.layout.change_tracking_height;
            file.untracked_height = group.layout.untracked_height;
            if let Some(frame) = group.placement.normal_frame {
                file.window_width = Some(frame.width);
                file.window_height = Some(frame.height);
            }
        } else {
            file.open_repos.clear();
            file.active_repo = None;
        }
        file.window_groups = Some(stored_groups);

        persist_to_path(path, &file)
    })
}

pub(super) fn parse_window_groups(groups: Vec<SavedWindowGroupFile>) -> Vec<SavedWindowGroup> {
    let mut parsed = Vec::with_capacity(groups.len());
    let mut seen_ids = FxHashSet::default();
    for group in groups {
        if !seen_ids.insert(group.id) {
            continue;
        }
        let repositories = parse_path_list(group.repositories);
        if repositories.is_empty() {
            continue;
        }
        let active_repository = group
            .active_repository
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(path_from_storage_key)
            .filter(|path| repositories.contains(path));
        parsed.push(SavedWindowGroup {
            id: group.id,
            custom_name: group.custom_name.and_then(non_empty_string),
            color: group.color,
            repositories,
            active_repository,
            restore_on_launch: group.restore_on_launch,
            last_activation_order: group.last_activation_order,
            layout: group.layout,
            placement: group.placement,
        });
    }
    parsed
}

pub(super) fn window_groups_to_file(groups: &[SavedWindowGroup]) -> Vec<SavedWindowGroupFile> {
    let mut stored = Vec::with_capacity(groups.len());
    let mut seen_ids = FxHashSet::default();
    for group in groups {
        if !seen_ids.insert(group.id) {
            continue;
        }
        let repositories = parse_path_list(
            group
                .repositories
                .iter()
                .map(|path| path_storage_key(path))
                .collect(),
        );
        if repositories.is_empty() {
            continue;
        }
        let active_repository = group
            .active_repository
            .as_ref()
            .filter(|active| repositories.contains(active))
            .map(|path| path_storage_key(path));
        stored.push(SavedWindowGroupFile {
            id: group.id,
            custom_name: group.custom_name.clone().and_then(non_empty_string),
            color: group.color,
            repositories: repositories
                .iter()
                .map(|path| path_storage_key(path))
                .collect(),
            active_repository,
            restore_on_launch: group.restore_on_launch,
            last_activation_order: group.last_activation_order,
            layout: group.layout.clone(),
            placement: group.placement.clone(),
        });
    }
    stored
}

pub(super) fn legacy_window_group_from_projection(
    file: &UiSessionFile,
) -> Option<SavedWindowGroupFile> {
    if file.open_repos.iter().all(|path| path.trim().is_empty()) {
        return None;
    }
    Some(SavedWindowGroupFile {
        id: LEGACY_WINDOW_GROUP_ID,
        custom_name: None,
        color: None,
        repositories: file.open_repos.clone(),
        active_repository: file.active_repo.clone(),
        restore_on_launch: true,
        last_activation_order: 1,
        layout: WindowGroupLayout {
            sidebar_width: file.sidebar_width,
            details_width: file.details_width,
            sidebar_collapsed: file.sidebar_collapsed.unwrap_or(false),
            change_tracking_height: file.change_tracking_height,
            untracked_height: file.untracked_height,
        },
        placement: PortableWindowPlacement::default(),
    })
}

pub(super) fn sync_legacy_window_group_from_projection(file: &mut UiSessionFile) {
    let replacement = legacy_window_group_from_projection(file);
    let Some(groups) = file.window_groups.as_mut() else {
        file.window_groups = replacement.map(|group| vec![group]);
        return;
    };
    if groups.len() != 1 || groups[0].id != LEGACY_WINDOW_GROUP_ID || !groups[0].restore_on_launch {
        return;
    }
    if let Some(replacement) = replacement {
        groups[0].repositories = replacement.repositories;
        groups[0].active_repository = replacement.active_repository;
    } else {
        groups.clear();
    }
}

pub(super) fn migrate_v3_file(mut file: UiSessionFile) -> UiSessionFile {
    file.version = CURRENT_SESSION_FILE_VERSION;
    if file.window_groups.is_some() {
        return file;
    }

    file.window_groups = legacy_window_group_from_projection(&file).map(|group| vec![group]);
    file
}

pub(super) fn preserve_pre_v4_session_backup(path: &Path, replacement: &[u8]) -> io::Result<()> {
    let replacement_version = serde_json::from_slice::<serde_json::Value>(replacement)
        .ok()
        .and_then(|value| value.get("version").and_then(|version| version.as_u64()));
    if replacement_version != Some(SESSION_FILE_VERSION_V4 as u64) {
        return Ok(());
    }

    let Ok(previous) = fs::read(path) else {
        return Ok(());
    };
    let previous_version = serde_json::from_slice::<serde_json::Value>(&previous)
        .ok()
        .and_then(|value| value.get("version").and_then(|version| version.as_u64()))
        .unwrap_or(SESSION_FILE_VERSION_V1 as u64);
    if previous_version >= SESSION_FILE_VERSION_V4 as u64 {
        return Ok(());
    }

    let mut backup_name = path.as_os_str().to_os_string();
    backup_name.push(".v3.bak");
    let backup_path = PathBuf::from(backup_name);
    if backup_path.exists() {
        return Ok(());
    }
    gitcomet_core::fs_utils::write_private_file(&backup_path, &previous)
}
