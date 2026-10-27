use super::repository_runtime::RepositoryRuntimes;
use super::worker_channel::StoreWorkerCommand;
use crate::model::AppState;
use gitcomet_core::services::GitRepository;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak, mpsc};

struct RepositoryTarget {
    root: PathBuf,
    repository: Option<Weak<dyn GitRepository>>,
}

struct Subscriber {
    alive: Weak<AtomicBool>,
    sender: mpsc::Sender<StoreWorkerCommand>,
    repositories: Vec<RepositoryTarget>,
}

static STORES: LazyLock<Mutex<HashMap<u64, Subscriber>>> = LazyLock::new(Mutex::default);

pub(super) fn subscribe(
    id: u64,
    alive: Weak<AtomicBool>,
    sender: mpsc::Sender<StoreWorkerCommand>,
) {
    STORES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(
            id,
            Subscriber {
                alive,
                sender,
                repositories: Vec::new(),
            },
        );
}

pub(super) fn unsubscribe(id: u64) {
    STORES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&id);
}

fn retain_live(stores: &mut HashMap<u64, Subscriber>) {
    stores.retain(|_, subscriber| {
        subscriber
            .alive
            .upgrade()
            .is_some_and(|alive| alive.load(Ordering::Acquire))
    });
}

pub(super) fn subscribers() -> Vec<(u64, mpsc::Sender<StoreWorkerCommand>)> {
    let mut stores = STORES.lock().unwrap_or_else(|error| error.into_inner());
    retain_live(&mut stores);
    stores
        .iter()
        .map(|(id, subscriber)| (*id, subscriber.sender.clone()))
        .collect()
}

pub(super) fn update_repositories(id: u64, state: &AppState, repos: &RepositoryRuntimes) {
    let mut stores = STORES.lock().unwrap_or_else(|error| error.into_inner());
    let Some(subscriber) = stores.get_mut(&id) else {
        return;
    };
    subscriber.repositories = state
        .repos
        .iter()
        .map(|repo| RepositoryTarget {
            root: repo.spec.workdir.clone(),
            repository: repos.repository(&repo.id).map(Arc::downgrade),
        })
        .collect();
}

pub(super) fn repository_targets() -> Vec<(PathBuf, Option<Arc<dyn GitRepository>>)> {
    let mut stores = STORES.lock().unwrap_or_else(|error| error.into_inner());
    retain_live(&mut stores);
    stores
        .values()
        .flat_map(|subscriber| &subscriber.repositories)
        .map(|target| {
            (
                target.root.clone(),
                target.repository.as_ref().and_then(Weak::upgrade),
            )
        })
        .collect()
}
