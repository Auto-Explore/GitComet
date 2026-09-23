use super::*;

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
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionReposSnapshot {
    pub open_repos: Arc<[Arc<str>]>,
    pub active_repo_index: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct CachedSessionReposSnapshot {
    repo_ids: SmallVec<[RepoId; 24]>,
    repo_keys: SmallVec<[Arc<str>; 24]>,
    dedup_indexes_by_repo: SmallVec<[usize; 24]>,
    open_repos: Arc<[Arc<str>]>,
}

thread_local! {
    pub(super) static SESSION_REPOS_SNAPSHOT_CACHE: RefCell<Option<CachedSessionReposSnapshot>> = const { RefCell::new(None) };
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

/// Builds the persisted repository list while one or more external-drop
/// candidates are still being validated. Those temporary tabs must not leak
/// into the session through an unrelated save (for example, switching back to
/// another tab while a drop is opening).
fn snapshot_repos_without_provisional_drops(state: &AppState) -> Option<SessionReposSnapshot> {
    if !state
        .repos
        .iter()
        .any(|repo| repo.is_provisional_external_drop_open())
    {
        return None;
    }

    let active_repo_id = state
        .active_repo
        .filter(|active_id| {
            state
                .repos
                .iter()
                .any(|repo| repo.id == *active_id && !repo.is_provisional_external_drop_open())
        })
        .or_else(|| {
            state
                .active_repo
                .and_then(|active_id| state.repos.iter().find(|repo| repo.id == active_id))
                .filter(|repo| repo.is_provisional_external_drop_open())
                .and_then(|repo| repo.external_drop_previous_active_repo())
                .filter(|previous_active| {
                    state.repos.iter().any(|repo| {
                        repo.id == *previous_active && !repo.is_provisional_external_drop_open()
                    })
                })
        })
        .or_else(|| {
            state
                .repos
                .iter()
                .filter(|repo| !repo.is_provisional_external_drop_open())
                .max_by_key(|repo| repo.last_active_at)
                .map(|repo| repo.id)
        });

    let mut unique_keys = SmallVec::<[Arc<str>; 24]>::new();
    let mut active_repo_index = None;
    for repo in state
        .repos
        .iter()
        .filter(|repo| !repo.is_provisional_external_drop_open())
    {
        let key = repo.session_workdir_key();
        let unique_ix = if let Some(ix) = unique_keys
            .iter()
            .position(|seen| seen.as_ref() == key.as_ref())
        {
            ix
        } else {
            unique_keys.push(Arc::clone(key));
            unique_keys.len() - 1
        };
        if active_repo_index.is_none() && Some(repo.id) == active_repo_id {
            active_repo_index = Some(unique_ix);
        }
    }

    Some(SessionReposSnapshot {
        open_repos: unique_keys.into_vec().into(),
        active_repo_index,
    })
}

pub fn snapshot_repos_from_state(state: &AppState) -> SessionReposSnapshot {
    if let Some(snapshot) = snapshot_repos_without_provisional_drops(state) {
        return snapshot;
    }
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
