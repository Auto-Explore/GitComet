use gitcomet_core::domain::{DiffArea, DiffTarget};
use gitcomet_state::model::Loadable;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub(super) fn hash_diff_target<H: Hasher>(target: &DiffTarget, hasher: &mut H) {
    match target {
        DiffTarget::WorkingTree { path, area } => {
            0u8.hash(hasher);
            path.hash(hasher);
            match area {
                DiffArea::Staged => 0u8.hash(hasher),
                DiffArea::Unstaged => 1u8.hash(hasher),
            }
        }
        DiffTarget::Commit { commit_id, path } => {
            1u8.hash(hasher);
            commit_id.hash(hasher);
            path.hash(hasher);
        }
    }
}

pub(super) fn hash_loadable_kind<T, H: Hasher>(value: &Loadable<T>, hasher: &mut H) {
    match value {
        Loadable::NotLoaded => 0u8.hash(hasher),
        Loadable::Loading => 1u8.hash(hasher),
        Loadable::Ready(_) => 2u8.hash(hasher),
        Loadable::Error(err) => {
            3u8.hash(hasher);
            err.hash(hasher);
        }
    }
}

pub(super) fn hash_loadable_arc<T, H: Hasher>(value: &Loadable<Arc<T>>, hasher: &mut H) {
    match value {
        Loadable::NotLoaded => 0u8.hash(hasher),
        Loadable::Loading => 1u8.hash(hasher),
        Loadable::Ready(shared) => {
            2u8.hash(hasher);
            (Arc::as_ptr(shared) as usize).hash(hasher);
        }
        Loadable::Error(err) => {
            3u8.hash(hasher);
            err.hash(hasher);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LoadableArcIdentity {
    NotLoaded,
    Loading,
    Ready(usize),
    Error(u64),
}

#[inline]
fn loadable_error_identity(err: &str) -> u64 {
    let mut hasher = FxHasher::default();
    err.hash(&mut hasher);
    hasher.finish()
}

#[inline]
pub(super) fn loadable_arc_identity<T>(value: &Loadable<Arc<T>>) -> LoadableArcIdentity {
    match value {
        Loadable::NotLoaded => LoadableArcIdentity::NotLoaded,
        Loadable::Loading => LoadableArcIdentity::Loading,
        Loadable::Ready(shared) => LoadableArcIdentity::Ready(Arc::as_ptr(shared) as usize),
        Loadable::Error(err) => LoadableArcIdentity::Error(loadable_error_identity(err)),
    }
}
