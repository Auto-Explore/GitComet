//! One preference authority for the stores belonging to a running app.

use crate::model::{
    RepositoryKey, RepositoryPreferenceUpdate, RepositoryPreferencesSnapshot,
    SharedRepositoryPreferences,
};
use gitcomet_core::services::GitRepository;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

type Subscriber = Box<dyn Fn(RepositoryPreferencesSnapshot) -> bool + Send + Sync>;

#[derive(Default)]
struct State {
    records: HashMap<RepositoryKey, RepositoryPreferencesSnapshot>,
    subscribers: HashMap<u64, Subscriber>,
    unpersisted_updates: HashMap<RepositoryKey, Vec<RepositoryPreferenceUpdate>>,
}

pub(super) struct PreferenceHub {
    pub(super) session_path: Option<PathBuf>,
    state: Mutex<State>,
}

impl PreferenceHub {
    pub(super) fn new(session_path: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            session_path,
            state: Mutex::new(State::default()),
        })
    }

    pub(super) fn shared() -> Arc<Self> {
        static HUBS: OnceLock<Mutex<HashMap<Option<PathBuf>, Arc<PreferenceHub>>>> =
            OnceLock::new();
        let path = crate::session::default_session_file_path_for_effect();
        let mut hubs = HUBS
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Arc::clone(hubs.entry(path.clone()).or_insert_with(|| Self::new(path)))
    }

    pub(super) fn subscribe(&self, id: u64, subscriber: Subscriber) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .subscribers
            .insert(id, subscriber);
    }

    pub(super) fn unsubscribe(&self, id: u64) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .subscribers
            .remove(&id);
    }

    pub(super) fn latest(&self, key: &RepositoryKey) -> Option<RepositoryPreferencesSnapshot> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .records
            .get(key)
            .cloned()
    }

    pub(super) fn initialize(
        &self,
        key: RepositoryKey,
        workdir: &Path,
        repo: &dyn GitRepository,
    ) -> (RepositoryPreferencesSnapshot, std::io::Result<()>) {
        if let Some(snapshot) = self.latest(&key) {
            return (snapshot, Ok(()));
        }
        let (preferences, persistence) = crate::session::initialize_repository_preferences(
            self.session_path.as_deref(),
            &key,
            workdir,
            repo,
        );
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let snapshot = state
            .records
            .entry(key.clone())
            .or_insert_with(|| RepositoryPreferencesSnapshot {
                key,
                revision: 1,
                preferences: Arc::new(preferences),
            })
            .clone();
        (snapshot, persistence)
    }

    /// Called on the single shared session executor, so publication and disk
    /// writes have the same order across windows. Subscribers never rebroadcast.
    pub(super) fn update(
        &self,
        key: RepositoryKey,
        update: &RepositoryPreferenceUpdate,
    ) -> std::io::Result<()> {
        let (preferences, updates) =
            {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let current = state.records.entry(key.clone()).or_insert_with(|| {
                    RepositoryPreferencesSnapshot {
                        key: key.clone(),
                        revision: 0,
                        preferences: Arc::new(SharedRepositoryPreferences::default()),
                    }
                });
                let mut next_preferences = (*current.preferences).clone();
                next_preferences.apply(update);
                let changed = next_preferences != *current.preferences || current.revision == 0;
                if changed {
                    current.preferences = Arc::new(next_preferences);
                    current.revision += 1;
                }
                let preferences = Arc::clone(&current.preferences);
                if changed {
                    let snapshot = current.clone();
                    state
                        .subscribers
                        .retain(|_, subscriber| subscriber(snapshot.clone()));
                }
                let updates = if self.session_path.is_some() {
                    let pending = state.unpersisted_updates.entry(key.clone()).or_default();
                    pending.push(update.clone());
                    pending.clone()
                } else {
                    Vec::new()
                };
                (preferences, updates)
            };
        if let Some(path) = &self.session_path {
            // Retain failed deltas until a write succeeds. Replaying them in
            // order also preserves unrelated preferences already on disk.
            crate::session::persist_repository_preference_updates(
                path,
                &key,
                &updates,
                &preferences,
            )?;
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .unpersisted_updates
                .remove(&key);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RepositoryFileSort, RepositoryListKind};
    use gitcomet_core::test_support::UnconfiguredRepository;
    use std::collections::BTreeSet;
    use std::fs;

    #[test]
    fn failed_writes_retry_ordered_deltas_and_preserve_unrelated_disk_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let saved = dir.path().join("saved.json");
        let workdir = dir.path().join("repo");
        let key = RepositoryKey::Worktree(workdir.clone());
        let repo = UnconfiguredRepository::new(&workdir);
        let hub = PreferenceHub::new(Some(path.clone()));
        hub.initialize(key.clone(), &workdir, &repo).1.unwrap();
        let pin = |name: &str, pinned| RepositoryPreferenceUpdate::Pin {
            key: format!("local:{name}"),
            pinned,
        };
        hub.update(key.clone(), &pin("remove", true)).unwrap();

        // A directory at the destination makes replacement fail even when
        // running as root. Restore the existing disk record after the outage.
        fs::rename(&path, &saved).unwrap();
        fs::create_dir(&path).unwrap();
        for update in [
            pin("a", true),
            pin("remove", false),
            RepositoryPreferenceUpdate::FileSort {
                list: RepositoryListKind::CommitFiles,
                sort: RepositoryFileSort::PathDescending,
            },
            RepositoryPreferenceUpdate::FileSort {
                list: RepositoryListKind::CommitFiles,
                sort: RepositoryFileSort::FileTypeAscending,
            },
        ] {
            assert!(hub.update(key.clone(), &update).is_err());
        }
        let expected = (*hub.latest(&key).unwrap().preferences).clone();
        assert_eq!(expected.pinned_items, BTreeSet::from(["local:a".into()]));
        fs::remove_dir(&path).unwrap();
        fs::rename(&saved, &path).unwrap();
        let mut disk: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        disk["window_width"] = 1400.into();
        disk["repository_preferences"][key.storage_key()]["file_sorts"]["range_files"] =
            "edits".into();
        fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();

        hub.update(key.clone(), &pin("b", true)).unwrap();
        let reopened = PreferenceHub::new(Some(path.clone()));
        let (snapshot, persistence) = reopened.initialize(key.clone(), &workdir, &repo);
        persistence.unwrap();
        let mut expected = expected;
        expected.pinned_items.insert("local:b".into());
        expected
            .file_sorts
            .insert(RepositoryListKind::RangeFiles, RepositoryFileSort::Edits);
        assert_eq!(*snapshot.preferences, expected);
        let mut disk: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["window_width"], 1400);

        // Successfully saved deltas must be retired: a later write should
        // preserve an independent removal instead of replaying the old pin.
        disk["repository_preferences"][key.storage_key()]["pinned_items"] =
            serde_json::json!(["local:b"]);
        fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();
        hub.update(key.clone(), &pin("c", true)).unwrap();
        let reopened = PreferenceHub::new(Some(path));
        let (snapshot, persistence) = reopened.initialize(key, &workdir, &repo);
        persistence.unwrap();
        assert_eq!(
            snapshot.preferences.pinned_items,
            BTreeSet::from(["local:b".into(), "local:c".into()])
        );
    }

    #[test]
    fn duplicate_updates_retry_failed_persistence_without_another_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let saved = dir.path().join("saved.json");
        let workdir = dir.path().join("repo");
        let key = RepositoryKey::Worktree(workdir.clone());
        let repo = UnconfiguredRepository::new(&workdir);
        let hub = PreferenceHub::new(Some(path.clone()));
        hub.initialize(key.clone(), &workdir, &repo).1.unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        hub.subscribe(1, Box::new(move |snapshot| tx.send(snapshot).is_ok()));
        fs::rename(&path, &saved).unwrap();
        fs::create_dir(&path).unwrap();
        let pin = RepositoryPreferenceUpdate::Pin {
            key: "local:a".into(),
            pinned: true,
        };
        assert!(hub.update(key.clone(), &pin).is_err());
        let changed = rx.try_recv().unwrap();
        fs::remove_dir(&path).unwrap();
        fs::rename(&saved, &path).unwrap();
        hub.update(key.clone(), &pin).unwrap();
        assert!(rx.try_recv().is_err());
        assert_eq!(hub.latest(&key).unwrap().revision, changed.revision);
        let reopened = PreferenceHub::new(Some(path));
        let (snapshot, persistence) = reopened.initialize(key, &workdir, &repo);
        persistence.unwrap();
        assert_eq!(*snapshot.preferences, *changed.preferences);
    }

    #[test]
    fn opening_again_uses_latest_revision_and_duplicate_updates_do_not_echo() {
        let hub = PreferenceHub::new(None);
        let key = RepositoryKey::Worktree(PathBuf::from("/repo"));
        let repo = UnconfiguredRepository::new("/repo");
        let (initial, persistence) = hub.initialize(key.clone(), Path::new("/repo"), &repo);
        persistence.unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        hub.subscribe(1, Box::new(move |snapshot| tx.send(snapshot).is_ok()));
        let pin = RepositoryPreferenceUpdate::Pin {
            key: "local:dev".into(),
            pinned: true,
        };
        hub.update(key.clone(), &pin).unwrap();
        let changed = rx.try_recv().unwrap();
        assert!(changed.revision > initial.revision);
        assert!(changed.preferences.pinned_items.contains("local:dev"));
        hub.update(key.clone(), &pin).unwrap();
        assert!(rx.try_recv().is_err());
        let (reopened, persistence) = hub.initialize(key.clone(), Path::new("/repo"), &repo);
        persistence.unwrap();
        assert_eq!(reopened.revision, changed.revision);
        assert_eq!(*reopened.preferences, *changed.preferences);
        hub.unsubscribe(1);
        hub.update(
            key,
            &RepositoryPreferenceUpdate::Pin {
                key: "local:dev".into(),
                pinned: false,
            },
        )
        .unwrap();
        assert!(rx.try_recv().is_err());
    }
}
