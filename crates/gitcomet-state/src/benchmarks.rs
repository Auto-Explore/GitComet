use crate::model::{AppState, RepoId};
use crate::msg::{Effect, Msg};

pub fn dispatch_sync(state: &mut AppState, msg: Msg) -> Vec<Effect> {
    crate::store::dispatch_sync_for_bench(state, msg)
}

pub fn set_active_repo_sync(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    dispatch_sync(state, Msg::SetActiveRepo { repo_id })
}

#[cfg(all(test, feature = "benchmarks"))]
mod tests {
    use super::*;
    use crate::model::{Loadable, RepoState};
    use gitcomet_core::domain::RepoSpec;
    use std::path::PathBuf;

    #[test]
    fn set_active_repo_sync_uses_reducer_path() {
        let mut state = AppState::default();

        let mut repo1 = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-1"),
            },
        );
        repo1.open = Loadable::Ready(());

        let mut repo2 = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-2"),
            },
        );
        repo2.open = Loadable::Ready(());

        state.repos.push(repo1);
        state.repos.push(repo2);
        state.active_repo = Some(RepoId(1));

        let effects = set_active_repo_sync(&mut state, RepoId(2));

        assert_eq!(state.active_repo, Some(RepoId(2)));
        assert!(
            effects.iter().any(
                |effect| matches!(effect, Effect::LoadStatus { repo_id } if *repo_id == RepoId(2))
            ),
            "expected reducer refresh effects for the target repo"
        );
    }
}
