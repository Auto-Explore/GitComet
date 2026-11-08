//! A status refresh re-reads the submodule list, with its drift and
//! inner-change facts, only when a first-level submodule shows in the status.
use super::*;
use crate::model::RepoState;
use gitcomet_core::domain::{
    CommitId, FileStatus, FileStatusKind, RepoSpec, RepoStatus, Submodule, SubmoduleStatus,
};
use std::path::PathBuf;

fn state() -> (AppState, RepoId) {
    let id = RepoId(1);
    let mut repo = RepoState::new_opening(
        id,
        RepoSpec {
            workdir: PathBuf::from("/w/app"),
        },
    );
    repo.set_submodules(Loadable::Ready(vec![Submodule {
        path: PathBuf::from("vendor/lib"),
        recorded_head: CommitId("a".into()),
        checked_out_head: Some(CommitId("a".into())),
        status: SubmoduleStatus::UpToDate,
        nested: false,
        drift: None,
        has_inner_changes: false,
    }]));
    let mut state = AppState::test_default();
    state.repos.push(repo);
    (state, id)
}

fn status(paths: &[&str]) -> RepoStatus {
    RepoStatus {
        staged: Arc::new(Vec::new()),
        unstaged: Arc::new(
            paths
                .iter()
                .map(|path| FileStatus {
                    path: PathBuf::from(path),
                    kind: FileStatusKind::Modified,
                    conflict: None,
                })
                .collect(),
        ),
    }
}

fn loads_submodules(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::LoadSubmodules { .. }))
}

#[test]
fn a_status_that_lists_no_submodule_leaves_the_facts_alone() {
    let (mut state, id) = state();
    let inputs = super::test_inputs::clock();
    let effects = effects::status_loaded(&inputs, &mut state, id, Ok(status(&["README.md"])));
    assert!(!loads_submodules(&effects));
}

#[test]
fn a_submodule_in_the_status_or_just_gone_from_it_refreshes_the_facts() {
    let (mut state, id) = state();
    let inputs = super::test_inputs::clock();
    let effects = effects::status_loaded(&inputs, &mut state, id, Ok(status(&["vendor/lib"])));
    assert!(
        loads_submodules(&effects),
        "a submodule that moved or changed"
    );
    state.repos[0].loads_in_flight = Default::default();

    let effects = effects::status_loaded(&inputs, &mut state, id, Ok(status(&[])));
    assert!(loads_submodules(&effects), "a submodule back in sync");
    state.repos[0].loads_in_flight = Default::default();

    let effects = effects::status_loaded(
        &inputs,
        &mut state,
        id,
        Err(gitcomet_core::error::Error::new(
            gitcomet_core::error::ErrorKind::Backend("status failed".into()),
        )),
    );
    assert!(!loads_submodules(&effects), "a failed status reads nothing");
}
