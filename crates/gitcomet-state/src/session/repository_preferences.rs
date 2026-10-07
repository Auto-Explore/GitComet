use super::*;
use crate::model::{RepositoryKey, RepositoryPreferenceUpdate, SharedRepositoryPreferences};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::GitRepository;

pub(super) fn lenient_preferences<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, SharedRepositoryPreferences>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            serde_json::from_value(value.clone())
                .ok()
                .map(|prefs| (key.clone(), prefs))
        })
        .collect())
}

/// The listing includes the main worktree first, even when a linked one opens
/// first. Keep unresolved legacy entries; only this new record is authoritative.
/// Return the preferences independently of whether their migration was saved.
pub(crate) fn initialize_repository_preferences(
    path: Option<&Path>,
    key: &RepositoryKey,
    workdir: &Path,
    repo: &dyn GitRepository,
) -> (SharedRepositoryPreferences, io::Result<()>) {
    let Some(path) = path else {
        return (SharedRepositoryPreferences::default(), Ok(()));
    };
    let storage_key = key.storage_key();
    if let Some(prefs) =
        load_file(path).and_then(|file| file.repository_preferences.get(&storage_key).cloned())
    {
        return (prefs, Ok(()));
    }
    let mut workdirs = match key {
        RepositoryKey::CommonDir(_) => repo
            .list_worktrees()
            .ok()
            .map(|worktrees| {
                worktrees
                    .into_iter()
                    .map(|worktree| worktree.path)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        RepositoryKey::Worktree(_) => Vec::new(),
    };
    let main = workdirs.first().cloned();
    workdirs.push(workdir.to_path_buf());
    workdirs = workdirs.into_iter().map(canonicalize_or_original).collect();
    workdirs.sort();
    workdirs.dedup();
    if let Some(main) = main {
        let main = canonicalize_or_original(main);
        workdirs.retain(|path| path != &main);
        workdirs.insert(0, main);
    }
    let mut result = SharedRepositoryPreferences::default();
    let persistence = update_session_file(path, |file| {
        if let Some(prefs) = file.repository_preferences.get(&storage_key) {
            result = prefs.clone();
            return SessionUpdate::Unchanged;
        }
        let workdir_keys = legacy_workdir_keys(file, &workdirs);
        result.history_mode = workdir_keys
            .iter()
            .find_map(|key| {
                file.repo_history_modes
                    .as_ref()
                    .and_then(|modes| modes.get(key).copied())
                    .map(Into::into)
                    .or_else(|| {
                        file.repo_history_scopes
                            .as_ref()
                            .and_then(|scopes| scopes.get(key).copied())
                            .map(Into::into)
                    })
            })
            .or_else(|| file.default_history_mode.map(Into::into))
            .unwrap_or_default();
        for key in &workdir_keys {
            if let Some(pins) = file
                .repo_sidebar_pinned_branches
                .as_ref()
                .and_then(|pins| pins.get(key))
            {
                result.pinned_items.extend(pins.iter().cloned());
            }
        }
        result.collapsed_items = workdir_keys
            .iter()
            .find_map(|key| {
                file.repo_sidebar_collapsed_items
                    .as_ref()
                    .and_then(|items| items.get(key))
                    .cloned()
            })
            .unwrap_or_default();
        file.repository_preferences
            .insert(storage_key.clone(), result.clone());
        SessionUpdate::Write
    });
    (result, persistence)
}

pub(crate) fn persist_repository_preference_updates(
    path: &Path,
    key: &RepositoryKey,
    updates: &[RepositoryPreferenceUpdate],
    preferences: &SharedRepositoryPreferences,
) -> io::Result<()> {
    update_session_file(path, |file| {
        let storage_key = key.storage_key();
        let Some(prefs) = file.repository_preferences.get_mut(&storage_key) else {
            // Initialization or earlier updates may have failed to persist.
            // The first successful write must keep the full in-memory record.
            file.repository_preferences
                .insert(storage_key, preferences.clone());
            return SessionUpdate::Write;
        };
        let old = prefs.clone();
        for update in updates {
            prefs.apply(update);
        }
        if *prefs == old {
            SessionUpdate::Unchanged
        } else {
            SessionUpdate::Write
        }
    })
}

// Legacy maps can contain the path through which a worktree was opened, such
// as a symlink. Match those aliases by filesystem identity, while preferring
// the canonical entry within each worktree's main-first priority.
fn legacy_workdir_keys(file: &UiSessionFile, workdirs: &[PathBuf]) -> Vec<String> {
    let mut aliases = BTreeMap::<PathBuf, BTreeSet<String>>::new();
    let keys = file
        .repo_history_modes
        .iter()
        .flat_map(|map| map.keys())
        .chain(file.repo_history_scopes.iter().flat_map(|map| map.keys()))
        .chain(
            file.repo_sidebar_pinned_branches
                .iter()
                .flat_map(|map| map.keys()),
        )
        .chain(
            file.repo_sidebar_collapsed_items
                .iter()
                .flat_map(|map| map.keys()),
        );
    for key in keys {
        aliases
            .entry(canonicalize_or_original(path_from_storage_key(key)))
            .or_default()
            .insert(key.clone());
    }
    let mut result = Vec::new();
    for workdir in workdirs {
        let canonical = path_storage_key(workdir);
        result.push(canonical.clone());
        if let Some(keys) = aliases.remove(workdir) {
            result.extend(keys.into_iter().filter(|key| *key != canonical));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::HistoryMode;
    use gitcomet_core::test_support::UnconfiguredRepository;

    #[test]
    fn partial_and_malformed_records_do_not_discard_other_session_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "version": 6,
                "open_repos": [],
                "window_width": 1400,
                "repository_preferences": {
                    "common:/good": {"pinned_items": ["local:dev"]},
                    "common:/bad": {"history_mode": "invalid"}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let file = load_file(&path).unwrap();
        assert_eq!(file.window_width, Some(1400));
        assert_eq!(file.repository_preferences.len(), 1);
        let good = &file.repository_preferences["common:/good"];
        assert!(good.show_hidden_files);
        assert!(!good.show_ignored_files);
        assert!(good.pinned_items.contains("local:dev"));
    }

    #[test]
    fn backend_without_common_directory_migrates_only_its_own_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let workdir = dir.path().join("repo");
        let other = dir.path().join("other");
        let key = RepositoryKey::Worktree(workdir.clone());
        let mut file = UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            ..Default::default()
        };
        file.default_history_mode = Some(HistoryMode::MergesOnly.into());
        file.repo_sidebar_pinned_branches = Some(BTreeMap::from([
            (
                path_storage_key(&workdir),
                BTreeSet::from(["local:mine".into()]),
            ),
            (
                path_storage_key(&other),
                BTreeSet::from(["local:other".into()]),
            ),
        ]));
        persist_to_path(&path, &file).unwrap();
        let (prefs, persistence) = initialize_repository_preferences(
            Some(&path),
            &key,
            &workdir,
            &UnconfiguredRepository::new(&workdir),
        );
        persistence.unwrap();
        assert_eq!(prefs.history_mode, HistoryMode::MergesOnly);
        assert_eq!(prefs.pinned_items, BTreeSet::from(["local:mine".into()]));
        assert_ne!(
            key.storage_key(),
            RepositoryKey::CommonDir(workdir).storage_key()
        );
    }

    #[test]
    fn first_persisted_update_preserves_the_full_in_memory_record() {
        use crate::model::{RepositoryFileSort, RepositoryListKind};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let workdir = dir.path().join("repo");
        let key = RepositoryKey::Worktree(workdir.clone());
        let legacy = serde_json::to_vec(&serde_json::json!({
            "version": 5,
            "open_repos": [],
            "window_width": 1400,
            "repo_sidebar_pinned_branches": {path_storage_key(&workdir): ["local:main"]}
        }))
        .unwrap();
        fs::write(&path, &legacy).unwrap();
        // Migration and several updates succeeded only in memory. The next
        // successful save must seed the missing disk record with all of them.
        let preferences = SharedRepositoryPreferences {
            history_mode: HistoryMode::FirstParent,
            pinned_items: BTreeSet::from(["local:main".into(), "local:dev".into()]),
            collapsed_items: BTreeSet::from(["group:local:existing".into()]),
            file_sorts: BTreeMap::from([(
                RepositoryListKind::CommitFiles,
                RepositoryFileSort::PathDescending,
            )]),
            show_hidden_files: false,
            show_ignored_files: true,
        };
        persist_repository_preference_updates(
            &path,
            &key,
            &[RepositoryPreferenceUpdate::Pin {
                key: "local:dev".into(),
                pinned: true,
            }],
            &preferences,
        )
        .unwrap();
        let file = load_file(&path).unwrap();
        assert_eq!(file.repository_preferences[&key.storage_key()], preferences);
        assert_eq!(file.window_width, Some(1400));
        assert_eq!(
            fs::read(path.with_file_name("session.json.v5.bak")).unwrap(),
            legacy
        );
        let (reopened, persistence) = initialize_repository_preferences(
            Some(&path),
            &key,
            &workdir,
            &UnconfiguredRepository::new(&workdir),
        );
        persistence.unwrap();
        assert_eq!(reopened, preferences);
    }

    #[cfg(unix)]
    #[test]
    fn migration_matches_legacy_symlink_paths() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path().join("repo");
        fs::create_dir(&workdir).unwrap();
        let alias = dir.path().join("alias");
        symlink(&workdir, &alias).unwrap();
        let key = RepositoryKey::Worktree(workdir.clone());
        let path = dir.path().join("session.json");
        let mut file = UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            ..Default::default()
        };
        file.repo_history_modes = Some(BTreeMap::from([(
            path_storage_key(&alias),
            HistoryMode::NoMerges.into(),
        )]));
        file.repo_sidebar_pinned_branches = Some(BTreeMap::from([(
            path_storage_key(&alias),
            BTreeSet::from(["local:dev".into()]),
        )]));
        persist_to_path(&path, &file).unwrap();
        let (prefs, persistence) = initialize_repository_preferences(
            Some(&path),
            &key,
            &workdir,
            &UnconfiguredRepository::new(&workdir),
        );
        persistence.unwrap();
        assert_eq!(prefs.history_mode, HistoryMode::NoMerges);
        assert!(prefs.pinned_items.contains("local:dev"));
        assert_eq!(
            load_file(&path).unwrap().repository_preferences[&key.storage_key()],
            prefs
        );
    }

    #[cfg(unix)]
    #[test]
    fn migration_preserves_non_utf8_path_keys_without_a_worktree_on_disk() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let dir = tempfile::tempdir().unwrap();
        // Some Unix filesystems reject these bytes in directory names. A
        // saved worktree can be unavailable, so migrate its encoded key
        // without creating that directory or depending on filesystem support.
        let workdir = dir.path().join(OsString::from_vec(b"repo-\xff".to_vec()));
        let replacement = dir.path().join("repo-�");
        let key = RepositoryKey::Worktree(workdir.clone());
        let path = dir.path().join("session.json");
        let file = UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            repo_history_modes: Some(BTreeMap::from([
                (path_storage_key(&workdir), HistoryMode::NoMerges.into()),
                (
                    path_storage_key(&replacement),
                    HistoryMode::FirstParent.into(),
                ),
            ])),
            repo_sidebar_pinned_branches: Some(BTreeMap::from([
                (
                    path_storage_key(&workdir),
                    BTreeSet::from(["local:dev".into()]),
                ),
                (
                    path_storage_key(&replacement),
                    BTreeSet::from(["local:other".into()]),
                ),
            ])),
            ..Default::default()
        };
        persist_to_path(&path, &file).unwrap();
        let (prefs, persistence) = initialize_repository_preferences(
            Some(&path),
            &key,
            &workdir,
            &UnconfiguredRepository::new(&workdir),
        );
        persistence.unwrap();
        assert_eq!(prefs.history_mode, HistoryMode::NoMerges);
        assert_eq!(prefs.pinned_items, BTreeSet::from(["local:dev".into()]));
        assert_eq!(path_from_storage_key(&path_storage_key(&workdir)), workdir);
        assert_eq!(
            load_file(&path).unwrap().repository_preferences[&key.storage_key()],
            prefs
        );
        assert_ne!(
            key.storage_key(),
            RepositoryKey::Worktree(replacement).storage_key()
        );
    }
}
