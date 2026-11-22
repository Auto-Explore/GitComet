use super::*;

#[test]
fn create_and_delete_tag_emit_effects() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateTag {
            repo_id: RepoId(1),
            name: "v1.0.0".to_string(),
            target: "HEAD".to_string(),
            message: None,
            annotated: false,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::CreateTag { repo_id: RepoId(1), name, target, message: None, annotated: false })] if name == "v1.0.0" && target == "HEAD"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteTag {
            repo_id: RepoId(1),
            name: "v1.0.0".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::DeleteTag { repo_id: RepoId(1), name })] if name == "v1.0.0"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushTag {
            repo_id: RepoId(1),
            remote: "origin".to_string(),
            name: "v1.0.0".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::PushTag { repo_id: RepoId(1), remote, name, .. })] if remote == "origin" && name == "v1.0.0"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteRemoteTag {
            repo_id: RepoId(1),
            remote: "origin".to_string(),
            name: "v1.0.0".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::DeleteRemoteTag { repo_id: RepoId(1), remote, name, .. })] if remote == "origin" && name == "v1.0.0"
    ));
}

/// The sidebar lists these tags. Resetting the list for the length of a reload
/// empties that section and drops its scroll position, so the loaded list
/// stays until the fresh one lands.
fn assert_tags_kept(repo: &RepoState, tags_rev: u64) {
    assert!(
        matches!(&repo.tags, Loadable::Ready(tags) if tags.len() == 1 && tags[0].name == "v1.0.0"),
        "the loaded tags were reset: {:?}",
        repo.tags
    );
    assert_eq!(repo.tags_rev, tags_rev);
}

#[test]
fn fetch_completion_reloads_remote_refs_and_loaded_tag_metadata() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let mut repo_state = repo_state_with_tags_loaded(repo_id);
    repo_state.pull_in_flight = 1;
    repo_state.set_remote_branches(Loadable::Ready(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "deleted".to_string(),
        target: CommitId("abc123".into()),
    }]));
    repo_state.set_remote_tags(Loadable::Ready(vec![gitcomet_core::domain::RemoteTag {
        remote: "origin".to_string(),
        name: "v1.0.0".to_string(),
        target: CommitId("abc123".into()),
    }]));
    let remote_branches_rev = repo_state.remote_branches_rev;
    let tags_rev = repo_state.tags_rev;
    state.repos.push(repo_state);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::FetchAll,
            result: Ok(CommandOutput::empty_success("git fetch --all --prune")),
        }),
    );

    assert!(matches!(
        &state.repos[0].remote_branches,
        Loadable::Ready(branches)
            if branches.len() == 1
                && branches[0].remote == "origin"
                && branches[0].name == "deleted"
    ));
    assert_eq!(state.repos[0].remote_branches_rev, remote_branches_rev);
    assert_tags_kept(&state.repos[0], tags_rev);
    assert!(matches!(state.repos[0].remote_tags, Loadable::Loading));
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::References(ReferenceEffect::LoadRemoteBranches { repo_id: id }) if *id == repo_id)
    ));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::References(ReferenceEffect::LoadTags { repo_id: id }) if *id == repo_id))
    );
    assert!(
        effects.iter().any(
            |effect| matches!(effect, Effect::References(ReferenceEffect::LoadRemoteTags { repo_id: id }) if *id == repo_id)
        )
    );
}

#[test]
fn create_tag_command_finished_reloads_tags() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(repo_state_with_tags_loaded(repo_id));
    let tags_rev = state.repos[0].tags_rev;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CreateTag {
                name: "v2.0.0".to_string(),
                target: "HEAD".to_string(),
                message: None,
                annotated: false,
            },
            result: Ok(CommandOutput::empty_success(
                "git tag -c tag.gpgsign=false -- v2.0.0 HEAD",
            )),
        }),
    );

    assert_tags_kept(&state.repos[0], tags_rev);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::References(ReferenceEffect::LoadTags { repo_id: id }) if *id == repo_id)),
        "expected LoadTags effect after CreateTag"
    );
}

#[test]
fn delete_tag_command_finished_reloads_tags() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(repo_state_with_tags_loaded(repo_id));
    let tags_rev = state.repos[0].tags_rev;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::DeleteTag {
                name: "v1.0.0".to_string(),
            },
            result: Ok(CommandOutput::empty_success("git tag -d v1.0.0")),
        }),
    );

    assert_tags_kept(&state.repos[0], tags_rev);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::References(ReferenceEffect::LoadTags { repo_id: id }) if *id == repo_id)),
        "expected LoadTags effect after DeleteTag"
    );
}

#[test]
fn prune_local_tags_command_finished_reloads_tags() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(repo_state_with_tags_loaded(repo_id));
    state.repos[0].local_actions_in_flight = 0;
    state.repos[0].pull_in_flight = 1;
    let tags_rev = state.repos[0].tags_rev;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PruneLocalTags,
            result: Ok(CommandOutput::empty_success("git tag prune")),
        }),
    );

    assert_tags_kept(&state.repos[0], tags_rev);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::References(ReferenceEffect::LoadTags { repo_id: id }) if *id == repo_id)),
        "expected LoadTags effect after PruneLocalTags"
    );
}

#[test]
fn create_tag_failed_does_not_reload_tags() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(repo_state_with_tags_loaded(repo_id));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CreateTag {
                name: "v2.0.0".to_string(),
                target: "HEAD".to_string(),
                message: None,
                annotated: false,
            },
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("tag already exists".to_string()),
            )),
        }),
    );

    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::References(ReferenceEffect::LoadTags { .. }))),
        "a failed CreateTag changed no tags, so nothing reloads: {effects:?}"
    );
}

#[test]
fn create_tag_with_message_propagates_to_effect() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateTag {
            repo_id: RepoId(1),
            name: "v1.0.0".to_string(),
            target: "HEAD".to_string(),
            message: Some("Release 1.0".to_string()),
            annotated: true,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::CreateTag { repo_id: RepoId(1), name, target, message: Some(msg), annotated: true })]
            if name == "v1.0.0" && target == "HEAD" && msg == "Release 1.0"
    ));
}
