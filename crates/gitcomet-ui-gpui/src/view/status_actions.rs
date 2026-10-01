use gitcomet_core::domain::DiffArea;
use gitcomet_state::model::RepoId;
use gitcomet_state::msg::{Msg, RepoPathList};
use gitcomet_state::store::AppStore;

/// Row selection cleanup belongs to the caller. The store decides whether the
/// view currently on screen is the diff being moved out of this status area.
pub(in crate::view) fn stage_or_unstage_paths(
    store: &impl StatusDispatcher,
    repo_id: RepoId,
    area: DiffArea,
    paths: impl Into<RepoPathList>,
) {
    let paths = paths.into();
    store.dispatch(match area {
        DiffArea::Unstaged => Msg::StagePaths { repo_id, paths },
        DiffArea::Staged => Msg::UnstagePaths { repo_id, paths },
    });
}

pub(in crate::view) trait StatusDispatcher {
    fn dispatch(&self, msg: Msg);
}
impl StatusDispatcher for AppStore {
    fn dispatch(&self, msg: Msg) {
        AppStore::dispatch(self, msg);
    }
}
impl StatusDispatcher for std::sync::Arc<AppStore> {
    fn dispatch(&self, msg: Msg) {
        AppStore::dispatch(self, msg);
    }
}
impl StatusDispatcher for super::pane_store::PaneStore {
    fn dispatch(&self, msg: Msg) {
        super::pane_store::PaneStore::dispatch(self, msg);
    }
}
