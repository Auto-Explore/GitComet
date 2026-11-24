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
        DiffArea::Unstaged => Msg::StagePaths {
            repo_id,
            paths: paths.into(),
        },
        DiffArea::Staged => Msg::UnstagePaths { repo_id, paths },
    });
}

pub(in crate::view) fn stage_checked(
    store: &impl StatusDispatcher,
    repo_id: RepoId,
    selection: gitcomet_state::msg::StageSelection,
    guard: Option<gitcomet_state::msg::StageGuard>,
) {
    use gitcomet_state::msg::StageSelection;
    let msg = match (guard, selection) {
        (None, StageSelection::Path(path)) => Msg::StagePath { repo_id, path },
        (guard, selection) => Msg::StagePaths {
            repo_id,
            paths: gitcomet_state::msg::StageRequest { selection, guard },
        },
    };
    store.dispatch(msg);
}

pub(in crate::view) fn toggle_checked_stage(
    store: &impl StatusDispatcher,
    repo_id: RepoId,
    area: DiffArea,
    selection: gitcomet_state::msg::StageSelection,
    guard: Option<gitcomet_state::msg::StageGuard>,
) {
    use gitcomet_state::msg::StageSelection;
    match (area, selection) {
        (DiffArea::Unstaged, selection) => stage_checked(store, repo_id, selection, guard),
        (DiffArea::Staged, StageSelection::Path(path)) => {
            store.dispatch(Msg::UnstagePath { repo_id, path })
        }
        (DiffArea::Staged, StageSelection::Paths(paths)) => {
            store.dispatch(Msg::UnstagePaths { repo_id, paths })
        }
    }
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
