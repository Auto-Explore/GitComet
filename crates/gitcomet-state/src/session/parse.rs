use super::*;

pub(super) fn has_recorded_session_repository(file: &UiSessionFile) -> bool {
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

pub(super) fn parse_repos(
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

pub(super) fn parse_path_list(paths_raw: Vec<String>) -> Vec<PathBuf> {
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

pub(super) fn parse_path_keyed_string_sets(
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

pub(super) fn path_keyed_string_sets_to_storage(
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

pub(super) fn non_empty_string(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

pub(super) fn external_code_editor_from_file(
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

pub(super) fn external_code_editor_to_file(
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

pub(super) fn sanitize_ui_scale_percent(percent: Option<u32>) -> u32 {
    percent
        .unwrap_or(DEFAULT_UI_SCALE_PERCENT)
        .clamp(MIN_UI_SCALE_PERCENT, MAX_UI_SCALE_PERCENT)
}

pub(super) fn migrate_scaled_dimension_to_design_units(
    value: Option<u32>,
    ui_scale_percent: Option<u32>,
) -> Option<u32> {
    let value = value? as f32;
    let factor =
        sanitize_ui_scale_percent(ui_scale_percent) as f32 / DEFAULT_UI_SCALE_PERCENT as f32;
    let design_units = (value / factor).round();
    (design_units.is_finite() && design_units >= 1.0).then_some(design_units as u32)
}

pub(super) fn migrate_legacy_repo_fetch_prune_setting(mut file: UiSessionFile) -> UiSessionFile {
    if file.fetch_prune_deleted_remote_branches.is_none() {
        file.fetch_prune_deleted_remote_branches = file
            .repo_fetch_prune_deleted_remote_tracking_branches
            .as_ref()
            .filter(|settings| !settings.is_empty())
            // The setting is global now, so preserve every prior opt-out.
            .map(|settings| settings.values().all(|enabled| *enabled));
    }
    file.repo_fetch_prune_deleted_remote_tracking_branches = None;
    file
}

pub(super) fn migrate_v2_file(mut file: UiSessionFile) -> UiSessionFile {
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
