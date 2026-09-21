use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum HistoryScopeSetting {
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
pub(super) enum HistoryModeSetting {
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
