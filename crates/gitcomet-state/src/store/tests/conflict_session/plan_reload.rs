use super::*;
use crate::msg::ConflictPlanOrigin;
use gitcomet_core::merge::{MergeBlockId, MergeSource, OrderedSelection};

fn file() -> ConflictFile {
    ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: None,
        current_bytes: None,
        base: Some("base\nshared\nsame\n".into()),
        ours: Some("ours\nshared\nlocal\n".into()),
        theirs: Some("theirs\nshared\nsame\n".into()),
        current: Some(
            "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nshared\nlocal\n".into(),
        ),
    }
}

fn complete_load(
    state: &mut AppState,
    repos: &mut RepositoryRuntimes,
    id_alloc: &AtomicU64,
    repo_id: RepoId,
    file: ConflictFile,
) {
    let request = current_conflict_request(state, repo_id);
    reduce(
        repos,
        id_alloc,
        state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            request,
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );
}

fn reload(
    state: &mut AppState,
    repos: &mut RepositoryRuntimes,
    id_alloc: &AtomicU64,
    repo_id: RepoId,
) {
    reduce(
        repos,
        id_alloc,
        state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );
}

fn setup() -> (AppState, RepositoryRuntimes, AtomicU64, RepoId) {
    let mut state = AppState::test_default();
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "file.txt",
        FileConflictKind::BothModified,
    );
    reload(&mut state, &mut repos, &id_alloc, repo_id);
    complete_load(&mut state, &mut repos, &id_alloc, repo_id, file());
    (state, repos, id_alloc, repo_id)
}

fn selection(session: &ConflictSession, block_id: MergeBlockId) -> OrderedSelection {
    session
        .merge_plan
        .as_ref()
        .unwrap()
        .blocks
        .iter()
        .find(|block| block.id == block_id)
        .unwrap()
        .selection
        .clone()
}

#[test]
fn a_plan_toggle_during_reload_survives_the_accepted_load() {
    let (mut state, mut repos, id_alloc, repo_id) = setup();
    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    let origin = Arc::new(ConflictPlanOrigin::from_plan(
        session.merge_plan.as_ref().unwrap(),
    ));
    let block_id = session.merge_plan.as_ref().unwrap().blocks[0].id;
    reload(&mut state, &mut repos, &id_alloc, repo_id);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictTogglePlanBlockSource {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            origin,
            block_id,
            source: MergeSource::B,
        },
    );
    assert_eq!(
        selection(
            state.repos[0]
                .conflict_state
                .session_pending_restore
                .as_ref()
                .unwrap(),
            block_id
        ),
        MergeSource::B.into()
    );
    complete_load(&mut state, &mut repos, &id_alloc, repo_id, file());
    assert_eq!(
        selection(
            state.repos[0]
                .conflict_state
                .conflict_session
                .as_ref()
                .unwrap(),
            block_id
        ),
        MergeSource::B.into()
    );
}

#[test]
fn an_automatic_plan_replacement_survives_a_same_plan_reload() {
    let (mut state, mut repos, id_alloc, repo_id) = setup();
    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    let origin = Arc::new(ConflictPlanOrigin::from_plan(
        session.merge_plan.as_ref().unwrap(),
    ));
    let (index, block) = session
        .merge_plan
        .as_ref()
        .unwrap()
        .blocks
        .iter()
        .enumerate()
        .find(|(_, block)| block.is_delta && !block.original_conflict)
        .unwrap();
    assert!(!session.region_plan_blocks.contains(&index));
    let block_id = block.id;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictReplacePlanBlockSelection {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            origin,
            block_id,
            selection: MergeSource::A.into(),
        },
    );
    assert_eq!(
        selection(
            state.repos[0]
                .conflict_state
                .conflict_session
                .as_ref()
                .unwrap(),
            block_id
        ),
        MergeSource::A.into()
    );
    reload(&mut state, &mut repos, &id_alloc, repo_id);
    complete_load(&mut state, &mut repos, &id_alloc, repo_id, file());
    assert_eq!(
        selection(
            state.repos[0]
                .conflict_state
                .conflict_session
                .as_ref()
                .unwrap(),
            block_id
        ),
        MergeSource::A.into()
    );
}

#[test]
fn an_automatic_plan_replacement_during_reload_survives_the_accepted_load() {
    let (mut state, mut repos, id_alloc, repo_id) = setup();
    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    let origin = Arc::new(ConflictPlanOrigin::from_plan(
        session.merge_plan.as_ref().unwrap(),
    ));
    let (index, block) = session
        .merge_plan
        .as_ref()
        .unwrap()
        .blocks
        .iter()
        .enumerate()
        .find(|(_, block)| block.is_delta && !block.original_conflict)
        .unwrap();
    assert!(!session.region_plan_blocks.contains(&index));
    let block_id = block.id;
    reload(&mut state, &mut repos, &id_alloc, repo_id);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictReplacePlanBlockSelection {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            origin,
            block_id,
            selection: MergeSource::A.into(),
        },
    );
    assert_eq!(
        selection(
            state.repos[0]
                .conflict_state
                .session_pending_restore
                .as_ref()
                .unwrap(),
            block_id
        ),
        MergeSource::A.into()
    );
    complete_load(&mut state, &mut repos, &id_alloc, repo_id, file());
    assert_eq!(
        selection(
            state.repos[0]
                .conflict_state
                .conflict_session
                .as_ref()
                .unwrap(),
            block_id
        ),
        MergeSource::A.into()
    );
}

fn repeated_file(count: usize) -> ConflictFile {
    ConflictFile {
        base: Some("base\nshared\n".repeat(count).into()),
        ours: Some("ours\nshared\n".repeat(count).into()),
        theirs: Some("theirs\nshared\n".repeat(count).into()),
        current: Some(
            "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nshared\n"
                .repeat(count)
                .into(),
        ),
        ..file()
    }
}

fn reload_with(
    state: &mut AppState,
    repos: &mut RepositoryRuntimes,
    id_alloc: &AtomicU64,
    repo_id: RepoId,
    file: ConflictFile,
) {
    reload(state, repos, id_alloc, repo_id);
    complete_load(state, repos, id_alloc, repo_id, file);
}

#[test]
fn a_stale_plan_source_basis_is_refused_without_changing_any_decision() {
    for pending in [false, true] {
        let (mut state, mut repos, id_alloc, repo_id) = setup();
        let plan = state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .merge_plan
            .as_ref()
            .unwrap();
        let origin = Arc::new(ConflictPlanOrigin::from_plan(plan));
        let block_id = plan.blocks[0].id;
        let mut changed = file();
        changed.base = Some("base\nshared\nexternal\n".into());
        reload_with(&mut state, &mut repos, &id_alloc, repo_id, changed);
        let before = state.repos[0]
            .conflict_state
            .conflict_session
            .clone()
            .unwrap();
        assert!(
            before
                .merge_plan
                .as_ref()
                .unwrap()
                .blocks
                .iter()
                .any(|block| block.id == block_id)
        );
        if pending {
            reload(&mut state, &mut repos, &id_alloc, repo_id);
        }
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::ConflictTogglePlanBlockSource {
                repo_id,
                path: PathBuf::from("file.txt").into(),
                origin,
                block_id,
                source: MergeSource::B,
            },
        );
        let conflict = &state.repos[0].conflict_state;
        let after = conflict
            .conflict_session
            .as_ref()
            .or(conflict.session_pending_restore.as_ref())
            .unwrap();
        assert_eq!(after.merge_plan, before.merge_plan);
        assert_eq!(after.regions, before.regions);
        assert!(
            state.repos[0]
                .feedback
                .last_error
                .as_ref()
                .unwrap()
                .contains("Review the refreshed conflict")
        );
    }
}

#[test]
fn a_duplicate_block_occurrence_is_valid_only_in_its_unchanged_plan() {
    for changed in [false, true] {
        let (mut state, mut repos, id_alloc, repo_id) = setup();
        reload_with(&mut state, &mut repos, &id_alloc, repo_id, repeated_file(2));
        let plan = state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .merge_plan
            .as_ref()
            .unwrap();
        let original: Vec<_> = plan
            .blocks
            .iter()
            .filter(|block| block.original_conflict)
            .collect();
        assert_eq!(original.len(), 2);
        assert_eq!(original[0].id.fingerprint, original[1].id.fingerprint);
        assert_ne!(original[0].id.occurrence, original[1].id.occurrence);
        let block_id = original[1].id;
        let origin = Arc::new(ConflictPlanOrigin::from_plan(plan));
        if changed {
            reload_with(&mut state, &mut repos, &id_alloc, repo_id, repeated_file(3));
        }
        let before = state.repos[0]
            .conflict_state
            .conflict_session
            .clone()
            .unwrap();
        assert!(
            before
                .merge_plan
                .as_ref()
                .unwrap()
                .blocks
                .iter()
                .any(|block| block.id == block_id)
        );
        reload(&mut state, &mut repos, &id_alloc, repo_id);
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::ConflictReplacePlanBlockSelection {
                repo_id,
                path: PathBuf::from("file.txt").into(),
                origin,
                block_id,
                selection: MergeSource::B.into(),
            },
        );
        let after = state.repos[0]
            .conflict_state
            .session_pending_restore
            .as_ref()
            .unwrap();
        if changed {
            assert_eq!(after.merge_plan, before.merge_plan);
            assert_eq!(after.regions, before.regions);
            assert!(state.repos[0].feedback.last_error.is_some());
        } else {
            assert_eq!(selection(after, block_id), MergeSource::B.into());
            assert_eq!(after.regions[0], before.regions[0]);
            assert_eq!(state.repos[0].feedback.last_error, None);
        }
    }
}

#[test]
fn a_plan_choice_keeps_newer_decisions_on_other_blocks() {
    let (mut state, mut repos, id_alloc, repo_id) = setup();
    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_mut()
        .unwrap();
    let plan = session.merge_plan.as_ref().unwrap();
    let origin = Arc::new(ConflictPlanOrigin::from_plan(plan));
    let block_id = plan.blocks[0].id;
    let automatic = plan
        .blocks
        .iter()
        .find(|block| block.is_delta && !block.original_conflict)
        .unwrap()
        .id;
    assert!(session.replace_plan_block_selection(automatic, MergeSource::A.into()));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictTogglePlanBlockSource {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            origin,
            block_id,
            source: MergeSource::B,
        },
    );
    let after = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(selection(after, block_id), MergeSource::B.into());
    assert_eq!(selection(after, automatic), MergeSource::A.into());
    assert_eq!(state.repos[0].feedback.last_error, None);
}

#[test]
fn a_changed_block_partition_refuses_a_stale_choice_with_unchanged_stage_texts() {
    let (mut state, mut repos, id_alloc, repo_id) = setup();
    let split_file = ConflictFile {
        base: Some("base one\nbase two\nshared\nsame\n".into()),
        ours: Some("ours one\nours two\nshared\nlocal\n".into()),
        theirs: Some("theirs one\ntheirs two\nshared\nsame\n".into()),
        current: Some(
            "<<<<<<< ours\nours one\nours two\n=======\ntheirs one\ntheirs two\n>>>>>>> theirs\nshared\nlocal\n".into(),
        ),
        ..file()
    };
    reload_with(&mut state, &mut repos, &id_alloc, repo_id, split_file);
    let original = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap()
        .merge_plan
        .clone()
        .unwrap();
    let origin = Arc::new(ConflictPlanOrigin::from_plan(&original));
    let block_id = original
        .blocks
        .iter()
        .find(|block| block.is_delta && !block.original_conflict)
        .unwrap()
        .id;
    let expected_conflict_rev = state.repos[0].conflict_state.conflict_rev;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSplitRegion {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            boundaries: gitcomet_core::conflict_session::ConflictRegionSplitBoundaries {
                ours: [1, 1],
                theirs: [1, 1],
                base: Some([1, 1]),
            },
            expected_conflict_rev,
        },
    );
    let before = state.repos[0]
        .conflict_state
        .conflict_session
        .clone()
        .unwrap();
    let plan = before.merge_plan.as_ref().unwrap();
    assert_eq!(plan.base, original.base);
    assert_eq!(plan.local, original.local);
    assert_eq!(plan.remote, original.remote);
    assert_eq!(plan.blocks.len(), original.blocks.len() + 1);
    assert!(plan.blocks.iter().any(|block| block.id == block_id));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictReplacePlanBlockSelection {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            origin,
            block_id,
            selection: MergeSource::A.into(),
        },
    );
    let after = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(after.merge_plan, before.merge_plan);
    assert_eq!(after.regions, before.regions);
    assert!(state.repos[0].feedback.last_error.is_some());
}
