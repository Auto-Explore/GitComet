use gitcomet_core::services::GitRepository;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, RwLock, Weak};

type Gates = HashMap<PathBuf, Weak<RwLock<()>>>;
static GATES: LazyLock<Mutex<Gates>> = LazyLock::new(Mutex::default);

pub(super) fn for_repo(repo: &dyn GitRepository) -> Arc<RwLock<()>> {
    let path = repo
        .common_dir()
        .unwrap_or_else(|| repo.spec().workdir.clone());
    let path = gitcomet_core::path_utils::canonicalize_or_original(path);
    let mut gates = GATES.lock().unwrap_or_else(|error| error.into_inner());
    gates.retain(|_, gate| gate.strong_count() > 0);
    if let Some(gate) = gates.get(&path).and_then(Weak::upgrade) {
        return gate;
    }
    let gate = Arc::new(RwLock::new(()));
    gates.insert(path, Arc::downgrade(&gate));
    gate
}
