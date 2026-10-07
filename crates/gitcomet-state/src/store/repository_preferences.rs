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
        let preferences =
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
                preferences
            };
        if let Some(path) = &self.session_path {
            crate::session::persist_repository_preference_update(path, &key, update, &preferences)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::test_support::UnconfiguredRepository;

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
