use super::*;
use crate::msg::{ConflictRegionOrigin, ConflictRegionResolutionUpdate};
use gitcomet_core::conflict_session::{
    ConflictRegionResolution as Resolution, reconstruct_conflict_marker_sides,
};
use gitcomet_core::merge::{MergeSource, OrderedSelection};

fn markers(ours: &str, theirs: &str) -> String {
    format!("<<<<<<< ours\n{ours}=======\n{theirs}>>>>>>> theirs\n")
}

fn session(current: &str, provisional: bool) -> ConflictSession {
    let (ours, theirs) = reconstruct_conflict_marker_sides(current);
    let (base, ours, theirs) = if provisional {
        (
            ConflictPayload::Absent,
            ConflictPayload::Absent,
            ConflictPayload::Absent,
        )
    } else {
        (
            ConflictPayload::Text("base\n".into()),
            ConflictPayload::Text(ours.into()),
            ConflictPayload::Text(theirs.into()),
        )
    };
    ConflictSession::from_merged_text(
        "file.txt".into(),
        FileConflictKind::BothModified,
        base,
        ours,
        theirs,
        current,
    )
}

fn apply(
    session: &ConflictSession,
    command: impl FnOnce(RepoId) -> Msg,
) -> crate::model::RepoState {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "file.txt",
        FileConflictKind::BothModified,
    );
    state.repos[0].conflict_state.conflict_session = Some(session.clone());
    reduce(&mut repos, &id_alloc, &mut state, command(repo_id));
    state.repos.remove(0)
}

fn sync(
    session: &ConflictSession,
    origin: &Arc<ConflictRegionOrigin>,
    updates: &[(usize, Resolution)],
) -> crate::model::RepoState {
    apply(session, |repo_id| Msg::ConflictSyncRegionResolutions {
        repo_id,
        path: PathBuf::from("file.txt").into(),
        origin: origin.clone(),
        updates: updates
            .iter()
            .map(
                |(region_index, resolution)| ConflictRegionResolutionUpdate {
                    region_index: *region_index,
                    resolution: resolution.clone(),
                },
            )
            .collect(),
    })
}

#[test]
fn stale_region_choice_maps_a_split_without_changing_an_unselected_region() {
    let provisional_text = format!(
        "{}separator\n{}",
        markers(
            "ours one\nshared\nours two\n",
            "theirs one\nshared\ntheirs two\n"
        ),
        markers("ours three\n", "theirs three\n"),
    );
    let full_text = format!(
        "{}shared\n{}separator\n{}",
        markers("ours one\n", "theirs one\n"),
        markers("ours two\n", "theirs two\n"),
        markers("ours three\n", "theirs three\n"),
    );
    let origin = Arc::new(ConflictRegionOrigin::from_session(&session(
        &provisional_text,
        true,
    )));
    let mut full = session(&full_text, false);
    full.regions[2].resolution = Resolution::PickTheirs;
    let repo = apply(&full, |repo_id| Msg::ConflictReplaceRegionSelection {
        repo_id,
        path: PathBuf::from("file.txt").into(),
        origin,
        region_index: 0,
        selection: MergeSource::A.into(),
    });
    assert_eq!(repo.feedback.last_error, None);
    let actual = repo.conflict_state.conflict_session.unwrap();
    assert_eq!(actual.regions.len(), 3);
    assert_eq!(
        actual.regions[0].resolution,
        Resolution::Sources(MergeSource::B.into())
    );
    assert_eq!(
        actual.regions[1].resolution,
        Resolution::Sources(MergeSource::B.into())
    );
    assert_eq!(actual.regions[2].resolution, Resolution::PickTheirs);
}

#[test]
fn merging_regions_requires_consistent_choices_for_every_overlapping_origin() {
    let provisional_text = format!(
        "{}{}",
        markers("ours one\n", "theirs one\n"),
        markers("ours two\n", "theirs two\n"),
    );
    let full_text = markers("ours one\nours two\n", "theirs one\ntheirs two\n");
    let origin = Arc::new(ConflictRegionOrigin::from_session(&session(
        &provisional_text,
        true,
    )));
    let full = session(&full_text, false);
    for updates in [
        vec![(0, Resolution::PickOurs)],
        vec![(0, Resolution::PickOurs), (1, Resolution::PickTheirs)],
    ] {
        let repo = sync(&full, &origin, &updates);
        assert!(repo.feedback.last_error.is_some());
        assert_eq!(
            repo.conflict_state.conflict_session.unwrap().regions,
            full.regions
        );
    }
    let repo = sync(
        &full,
        &origin,
        &[(0, Resolution::PickOurs), (1, Resolution::PickOurs)],
    );
    assert_eq!(repo.feedback.last_error, None);
    assert_eq!(
        repo.conflict_state.conflict_session.unwrap().regions[0].resolution,
        Resolution::PickOurs
    );
}

#[test]
fn an_unmapped_region_rejects_the_whole_update_before_changing_other_choices() {
    let original = session(
        &format!(
            "{}middle\n{}",
            markers("ours one\n", "theirs one\n"),
            markers("ours two\n", "theirs two\n"),
        ),
        false,
    );
    let origin = Arc::new(ConflictRegionOrigin::from_session(&original));
    let mut changed = original.clone();
    changed.regions[1].ours = "a new choice\n".into();
    let repo = sync(
        &changed,
        &origin,
        &[(0, Resolution::PickTheirs), (1, Resolution::PickOurs)],
    );
    assert!(repo.feedback.last_error.is_some());
    assert_eq!(
        repo.conflict_state.conflict_session.unwrap().regions,
        changed.regions
    );
}

#[test]
fn identical_regions_keep_positions_only_when_the_complete_basis_matches() {
    let block = markers("ours\n", "theirs\n");
    let original = session(&format!("{block}middle\n{block}"), false);
    let origin = Arc::new(ConflictRegionOrigin::from_session(&original));
    let repo = sync(&original, &origin, &[(1, Resolution::PickOurs)]);
    assert_eq!(repo.feedback.last_error, None);
    let updated = repo.conflict_state.conflict_session.unwrap();
    assert_eq!(updated.regions[0].resolution, Resolution::Unresolved);
    assert_eq!(updated.regions[1].resolution, Resolution::PickOurs);

    let changed = session(&block, false);
    let repo = sync(&changed, &origin, &[(1, Resolution::PickOurs)]);
    assert!(repo.feedback.last_error.is_some());
    assert_eq!(
        repo.conflict_state.conflict_session.unwrap().regions,
        changed.regions
    );
}

#[test]
fn a_pending_session_accepts_replacement_and_manual_output_edits() {
    let original = session(&markers("ours\n", "theirs\n"), true);
    for resolution in [
        Resolution::Sources(OrderedSelection::from_sources([
            MergeSource::B,
            MergeSource::A,
        ])),
        Resolution::ManualEdit("manually resolved\n".into()),
    ] {
        let mut repos = RepositoryRuntimes::default();
        let id_alloc = AtomicU64::new(1);
        let mut state = AppState::test_default();
        let repo_id = setup_repo_with_conflict(
            &mut state,
            &mut repos,
            &id_alloc,
            "file.txt",
            FileConflictKind::BothModified,
        );
        state.repos[0].conflict_state.conflict_file = Loadable::Loading;
        state.repos[0].conflict_state.session_pending_restore = Some(original.clone());
        let origin = Arc::new(ConflictRegionOrigin::from_session(&original));
        let message = match &resolution {
            Resolution::Sources(selection) => Msg::ConflictReplaceRegionSelection {
                repo_id,
                path: PathBuf::from("file.txt").into(),
                origin,
                region_index: 0,
                selection: selection.clone(),
            },
            _ => Msg::ConflictSyncRegionResolutions {
                repo_id,
                path: PathBuf::from("file.txt").into(),
                origin,
                updates: vec![ConflictRegionResolutionUpdate {
                    region_index: 0,
                    resolution: resolution.clone(),
                }],
            },
        };
        reduce(&mut repos, &id_alloc, &mut state, message);
        let conflict = &state.repos[0].conflict_state;
        assert!(conflict.conflict_session.is_none());
        assert_eq!(
            conflict.session_pending_restore.as_ref().unwrap().regions[0].resolution,
            resolution
        );
    }
}
