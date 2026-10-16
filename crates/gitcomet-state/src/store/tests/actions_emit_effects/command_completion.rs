use super::*;

#[test]
fn markdown_save_uses_the_callers_disk_baseline_without_a_loaded_conflict() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let original: Arc<[u8]> = Arc::from(b"- [ ] ship it\n".as_slice());
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SaveWorktreeFile {
            repo_id,
            path: PathBuf::from("README.md"),
            contents: "- [x] ship it\n".into(),
            expected_contents: Some(original.clone()),
            stage: false,
            completion: None,
        },
    );
    assert!(
        matches!(effects.as_slice(), [Effect::RepoCommand(RepoCommandEffect::SaveWorktreeFile {
        expected_contents: Some(baseline), contents, stage: false, ..
    })] if baseline == &original && contents == "- [x] ship it\n")
    );
}

#[test]
fn checkout_conflict_base_emits_effect() {
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

    let path = PathBuf::from("conflicted.bin");
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutConflictBase {
            repo_id: RepoId(1),
            path: path.clone(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::CheckoutConflictBase { repo_id: RepoId(1), path: effect_path })] if effect_path == &path
    ));
}

#[test]
fn accept_conflict_deletion_emits_effect() {
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

    let path = PathBuf::from("conflicted.bin");
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcceptConflictDeletion {
            repo_id: RepoId(1),
            path: path.clone(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::AcceptConflictDeletion { repo_id: RepoId(1), path: effect_path })] if effect_path == &path
    ));
}

#[test]
fn apply_file_change_emits_effect_and_keeps_the_diff_target() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);
    let target = DiffTarget::commit_range(
        CommitId("1111111111111111111111111111111111111111".into()),
        Some(CommitId("2222222222222222222222222222222222222222".into())),
        Some(PathBuf::from("src/lib.rs")),
    );
    let apply_target = gitcomet_core::domain::ApplyChangeTarget::from_diff_target(&target)
        .expect("a comparison between commits can be applied");

    for commit in [false, true] {
        state.repos[0].set_diff_target(Some(target.clone()));
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::ApplyFileChange {
                commit_retry: None,
                repo_id,
                target: apply_target.clone(),
                commit,
            },
        );
        assert!(
            matches!(
                effects.as_slice(),
                [Effect::RepoCommand(RepoCommandEffect::ApplyFileChange {
                    commit_retry: None,
                    repo_id: RepoId(1),
                    target: effect_target,
                    commit: effect_commit,
                    auth: None,
                })] if effect_target == &apply_target && *effect_commit == commit
            ),
            "commit={commit}: {effects:?}"
        );
        assert_eq!(state.repos[0].local_actions_in_flight, 1);
        // A committing apply counts as a git operation while it runs, so the
        // worktree write it makes is not taken for an outside edit.
        assert_eq!(
            state.repos[0].sequencer_actions_in_flight,
            u32::from(commit)
        );
        assert_eq!(state.repos[0].git_operation_in_flight(), commit);

        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::ApplyFileChange {
                    commit_retry: None,
                    target: apply_target.clone(),
                    commit,
                },
                result: Ok(CommandOutput::default()),
            }),
        );
        assert_eq!(state.repos[0].local_actions_in_flight, 0);
        assert_eq!(state.repos[0].sequencer_actions_in_flight, 0);
        // The source diff did not change, so the user keeps their place.
        assert_eq!(
            state.repos[0].diff_state.diff_target.as_ref(),
            Some(&target)
        );
    }
}

#[test]
fn apply_pop_and_drop_stash_emit_effects() {
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

    let apply = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyStash {
            repo_id: RepoId(1),
            index: 0,
        },
    );
    assert!(matches!(
        apply.as_slice(),
        [Effect::ApplyStash {
            repo_id: RepoId(1),
            index: 0
        }]
    ));

    let pop = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PopStash {
            repo_id: RepoId(1),
            index: 0,
        },
    );
    assert!(matches!(
        pop.as_slice(),
        [Effect::PopStash {
            repo_id: RepoId(1),
            index: 0
        }]
    ));

    let drop = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DropStash {
            repo_id: RepoId(1),
            index: 0,
        },
    );
    assert!(matches!(
        drop.as_slice(),
        [Effect::DropStash {
            repo_id: RepoId(1),
            index: 0
        }]
    ));
}

#[test]
fn repo_operations_emit_effects() {
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

    let stage = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::StagePath {
            repo_id: RepoId(1),
            path: PathBuf::from("a.txt"),
        },
    );
    assert!(matches!(
        stage.as_slice(),
        [Effect::StagePath {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let unstage = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnstagePath {
            repo_id: RepoId(1),
            path: PathBuf::from("a.txt"),
        },
    );
    assert!(matches!(
        unstage.as_slice(),
        [Effect::UnstagePath {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let commit = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Commit {
            repo_id: RepoId(1),
            message: "m".to_string(),
            push_after_commit: false,
        },
    );
    assert!(matches!(
        commit.as_slice(),
        [Effect::Commit {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let pull = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id: RepoId(1),
            mode: PullMode::Rebase,
        },
    );
    assert!(matches!(
        pull.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::Pull {
            repo_id: RepoId(1),
            ..
        })]
    ));

    let prune_branches = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PruneMergedBranches { repo_id: RepoId(1) },
    );
    assert!(matches!(
        prune_branches.as_slice(),
        [Effect::RepoCommand(
            RepoCommandEffect::PruneMergedBranches { repo_id: RepoId(1) }
        )]
    ));

    let prune_tags = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PruneLocalTags { repo_id: RepoId(1) },
    );
    assert!(matches!(
        prune_tags.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::PruneLocalTags {
            repo_id: RepoId(1)
        })]
    ));

    let push = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Push { repo_id: RepoId(1) },
    );
    assert!(matches!(
        push.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::Push {
            repo_id: RepoId(1),
            ..
        })]
    ));

    let force_push = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ForcePush { repo_id: RepoId(1) },
    );
    assert!(matches!(
        force_push.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::ForcePush {
            repo_id: RepoId(1),
            ..
        })]
    ));

    let push_set_upstream = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushSetUpstream {
            repo_id: RepoId(1),
            remote: "origin".to_string(),
            branch: "feature/foo".to_string(),
        },
    );
    assert!(matches!(
        push_set_upstream.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::PushSetUpstream {
            repo_id: RepoId(1),
            ..
        })]
    ));

    let set_upstream_branch = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch: "feature/local".to_string(),
            upstream: upstream_target("origin", "feature/foo"),
        },
    );
    assert!(matches!(
        set_upstream_branch.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream,
        })] if branch == "feature/local" && upstream == &upstream_target("origin", "feature/foo")
    ));

    let unset_upstream = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnsetUpstreamBranch {
            repo_id: RepoId(1),
            branch: "feature/foo".to_string(),
        },
    );
    assert!(matches!(
        unset_upstream.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::UnsetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
        })] if branch == "feature/foo"
    ));

    let stash = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Stash {
            repo_id: RepoId(1),
            message: "wip".to_string(),
            include_untracked: true,
        },
    );
    assert!(matches!(
        stash.as_slice(),
        [Effect::Stash {
            repo_id: RepoId(1),
            ..
        }]
    ));
}

#[test]
fn repo_command_finished_stage_hunk_triggers_diff_reload_effects() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("src/lib.rs"),
        DiffArea::Unstaged,
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::empty_success("git apply --cached")),
        }),
    );

    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert!(state.repos[0].diff_state.diff.is_loading());
    assert!(state.repos[0].diff_state.diff_file.is_loading());
    assert!(matches!(
        state.repos[0].diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiff {
            repo_id: id,
            target: DiffTarget::WorkingTree { path, .. },
        } if *id == repo_id && path == &PathBuf::from("src/lib.rs")
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiffFile {
            repo_id: id,
            target: DiffTarget::WorkingTree { path, .. },
        } if *id == repo_id && path == &PathBuf::from("src/lib.rs")
    )));
}

#[test]
fn repo_command_finished_stage_hunk_invalidates_loaded_blame() {
    // Regression: staging recomputes the diff, and the blame annotation column is
    // derived from the same content. Leaving blame `Ready` would make the view
    // skip a reload (same target, already attempted) and paint stale attribution.
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("src/lib.rs"),
        DiffArea::Unstaged,
    ));
    state.repos[0].diff_state.blame_path = Some(PathBuf::from("src/lib.rs"));
    state.repos[0].diff_state.blame_source = Some(gitcomet_core::domain::BlameSource::WorkingTree(
        DiffArea::Unstaged,
    ));
    state.repos[0].diff_state.blame = ready_working_tree_blame();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::empty_success("git apply --cached")),
        }),
    );

    assert!(
        matches!(state.repos[0].diff_state.blame, Loadable::NotLoaded),
        "blame must be invalidated so the annotation column reloads after staging"
    );
    // The target is preserved so the reload re-blames the same file/source.
    assert_eq!(
        state.repos[0].diff_state.blame_path.as_deref(),
        Some(std::path::Path::new("src/lib.rs"))
    );
    assert_eq!(
        state.repos[0].diff_state.blame_source,
        Some(gitcomet_core::domain::BlameSource::WorkingTree(
            DiffArea::Unstaged
        ))
    );
}

#[test]
fn repo_command_finished_stage_hunk_with_svg_diff_triggers_text_and_image_reload_effects() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("icon.svg"),
        DiffArea::Unstaged,
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::empty_success("git apply --cached")),
        }),
    );

    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert!(state.repos[0].diff_state.diff.is_loading());
    assert!(state.repos[0].diff_state.diff_file.is_loading());
    assert!(state.repos[0].diff_state.diff_file_image.is_loading());
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiff {
            repo_id: id,
            target: DiffTarget::WorkingTree { path, .. },
        } if *id == repo_id && path == &PathBuf::from("icon.svg")
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiffFileImage {
            repo_id: id,
            target: DiffTarget::WorkingTree { path, .. },
        } if *id == repo_id && path == &PathBuf::from("icon.svg")
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiffFile {
            repo_id: id,
            target: DiffTarget::WorkingTree { path, .. },
        } if *id == repo_id && path == &PathBuf::from("icon.svg")
    )));
}

#[test]
fn additional_routing_messages_emit_effects_and_update_counters() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyWorktreePatch {
            repo_id,
            patch: "@@ -1 +1 @@\n-old\n+new\n".to_string().into(),
            reverse: true,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::ApplyWorktreePatch {
            repo_id: RepoId(1),
            reverse: true,
            ..
        })]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutRemoteBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature".to_string(),
            local_branch: "feature".to_string(),
            mode: gitcomet_core::services::CheckoutRemoteBranchMode::Overwrite,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CheckoutRemoteBranch {
            repo_id: RepoId(1),
            mode: gitcomet_core::services::CheckoutRemoteBranchMode::Overwrite,
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CherryPickCommit {
            repo_id,
            commit_id: CommitId("deadbeef".into()),
            commit: true,
            mainline: Some(2),
            summary: "pick me".into(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::CherryPickCommit {
            repo_id: RepoId(1),
            commit: true,
            mainline: Some(2),
            summary,
            ..
        })] if summary == "pick me"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateBranchAndCheckout {
            repo_id,
            name: "feature/new".to_string(),
            target: "HEAD".to_string(),
            force: false,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CreateBranchAndCheckout {
            repo_id: RepoId(1),
            target,
            force: false,
            ..
        }] if target == "HEAD"
    ));

    // The force flag travels with the message: an overwrite request still
    // reaches the effect layer, just marked.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateBranchAndCheckout {
            repo_id,
            name: "feature/new".to_string(),
            target: "HEAD".to_string(),
            force: true,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CreateBranchAndCheckout {
            repo_id: RepoId(1),
            force: true,
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::StagePaths {
            repo_id,
            paths: vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")].into(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::StagePaths {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnstagePaths {
            repo_id,
            paths: vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")].into(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::UnstagePaths {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DiscardWorktreeChangesPaths {
            repo_id,
            paths: vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")],
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::DiscardWorktreeChangesPaths {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetUpstreamBranch {
            repo_id,
            branch: "feature/current".to_string(),
            upstream: upstream_target("origin", "feature/current"),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream,
        })] if branch == "feature/current" && upstream == &upstream_target("origin", "feature/current")
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnsetUpstreamBranch {
            repo_id,
            branch: "feature/current".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::UnsetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
        })] if branch == "feature/current"
    ));

    assert_eq!(
        state.repos[0].local_actions_in_flight, 10,
        "expected begin_local_action for all routed local-action messages"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExportPatch {
            repo_id,
            commit_id: CommitId("cafebabe".into()),
            dest: PathBuf::from("out.patch"),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::ExportPatch {
            repo_id: RepoId(1),
            ..
        })]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyPatch {
            repo_id,
            patch: PathBuf::from("input.patch"),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::ApplyPatch {
            repo_id: RepoId(1),
            ..
        })]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AddWorktree {
            repo_id,
            path: PathBuf::from("/tmp/worktree"),
            reference: Some("main".to_string()),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::AddWorktree {
            repo_id: RepoId(1),
            ..
        })]
    ));
    assert_eq!(state.repos[0].worktrees_in_flight, 1);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RemoveWorktree {
            repo_id,
            path: PathBuf::from("nested/worktree"),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::RemoveWorktree {
            repo_id: RepoId(1),
            path
        })] if path == &PathBuf::from("/tmp/repo/nested/worktree")
    ));
    assert_eq!(state.repos[0].worktrees_in_flight, 2);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SaveWorktreeFile {
            repo_id,
            path: PathBuf::from("src/lib.rs"),
            contents: "fn main() {}".to_string().into(),
            expected_contents: None,
            stage: true,
            completion: None,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::SaveWorktreeFile {
            repo_id: RepoId(1),
            stage: true,
            ..
        })]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AppendGitignorePatterns {
            repo_id,
            patterns: vec!["/build/out.log".to_string(), "*.tmp".to_string()],
        },
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::RepoCommand(RepoCommandEffect::AppendGitignorePatterns {
                repo_id: RepoId(1),
                patterns,
            })] if patterns.as_slice() == ["/build/out.log", "*.tmp"]
        ),
        "the patterns must reach the effect verbatim: the reducer is not allowed \
         to re-derive or reorder them"
    );

    // The messages above never finished; Continue/Abort wait for the
    // sequencer ones (`continue_and_abort_wait_only_for_sequencer_commands`).
    state.repos[0].sequencer_actions_in_flight = 0;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RebaseContinue { repo_id },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::RebaseContinue {
            repo_id: RepoId(1),
            auth: None,
        })]
    ));
    state.repos[0].sequencer_actions_in_flight = 0;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RebaseAbort { repo_id },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::RebaseAbort {
            repo_id: RepoId(1)
        })]
    ));
    state.repos[0].sequencer_actions_in_flight = 0;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::MergeAbort { repo_id },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::MergeAbort {
            repo_id: RepoId(1)
        })]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AddRemote {
            repo_id,
            name: "origin".to_string(),
            url: "https://example.com/repo.git".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::AddRemote {
            repo_id: RepoId(1),
            ..
        })]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RemoveRemote {
            repo_id,
            name: "origin".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::RemoveRemote {
            repo_id: RepoId(1),
            ..
        })]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetRemoteUrl {
            repo_id,
            name: "origin".to_string(),
            url: "https://example.com/alt.git".to_string(),
            kind: gitcomet_core::services::RemoteUrlKind::Push,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::SetRemoteUrl {
            repo_id: RepoId(1),
            kind: gitcomet_core::services::RemoteUrlKind::Push,
            ..
        })]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetUpstreamBranch {
            repo_id,
            branch: "feature/current".to_string(),
            upstream: upstream_target("origin", "feature/current"),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream,
        })] if branch == "feature/current" && upstream == &upstream_target("origin", "feature/current")
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnsetUpstreamBranch {
            repo_id,
            branch: "feature/current".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::UnsetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
        })] if branch == "feature/current"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutConflictSide {
            repo_id,
            path: PathBuf::from("conflicted.txt"),
            side: gitcomet_core::services::ConflictSide::Theirs,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(
            RepoCommandEffect::CheckoutConflictSide {
                repo_id: RepoId(1),
                side: gitcomet_core::services::ConflictSide::Theirs,
                ..
            }
        )]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LaunchMergetool {
            repo_id,
            path: PathBuf::from("conflicted.txt"),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::LaunchMergetool {
            repo_id: RepoId(1),
            ..
        })]
    ));
}

#[test]
fn repo_command_finished_error_summaries_cover_additional_labels() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let cases: Vec<(RepoCommandKind, &str)> = vec![
        (
            RepoCommandKind::AddRemote {
                name: "origin".to_string(),
                url: "https://example.com/repo.git".to_string(),
            },
            "Remote",
        ),
        (
            RepoCommandKind::RemoveRemote {
                name: "origin".to_string(),
            },
            "Remote",
        ),
        (
            RepoCommandKind::SetRemoteUrl {
                name: "origin".to_string(),
                url: "https://example.com/push.git".to_string(),
                kind: gitcomet_core::services::RemoteUrlKind::Push,
            },
            "Remote",
        ),
        (
            RepoCommandKind::SetUpstreamBranch {
                branch: "feature/current".to_string(),
                upstream: upstream_target("origin", "feature/current"),
            },
            "Set as tracking upstream",
        ),
        (
            RepoCommandKind::UnsetUpstreamBranch {
                branch: "feature/current".to_string(),
            },
            "Unlink upstream branch",
        ),
        (
            RepoCommandKind::CheckoutConflict {
                path: PathBuf::from("conflicted.txt"),
                side: gitcomet_core::services::ConflictSide::Ours,
            },
            "Checkout ours",
        ),
        (
            RepoCommandKind::CheckoutConflict {
                path: PathBuf::from("conflicted.txt"),
                side: gitcomet_core::services::ConflictSide::Theirs,
            },
            "Checkout theirs",
        ),
        (
            RepoCommandKind::AcceptConflictDeletion {
                path: PathBuf::from("conflicted.txt"),
            },
            "Accept deletion",
        ),
        (
            RepoCommandKind::CheckoutConflictBase {
                path: PathBuf::from("conflicted.txt"),
            },
            "Checkout base",
        ),
        (
            RepoCommandKind::LaunchMergetool {
                path: PathBuf::from("conflicted.txt"),
            },
            "Mergetool",
        ),
        (
            RepoCommandKind::SaveWorktreeFile {
                path: PathBuf::from("a.txt"),
                stage: false,
            },
            "Save file",
        ),
        (
            RepoCommandKind::ExportPatch {
                commit_id: CommitId("deadbeef".into()),
                dest: PathBuf::from("out.patch"),
            },
            "Patch",
        ),
        (
            RepoCommandKind::ApplyPatch {
                patch: PathBuf::from("in.patch"),
            },
            "Patch",
        ),
        (
            RepoCommandKind::AddWorktree {
                path: PathBuf::from("/tmp/worktree"),
                reference: None,
            },
            "Worktree",
        ),
        (
            RepoCommandKind::RemoveWorktree {
                path: PathBuf::from("/tmp/worktree"),
            },
            "Worktree",
        ),
        (
            RepoCommandKind::AddSubmodule {
                url: "https://example.com/sub.git".to_string(),
                path: PathBuf::from("mods/sub"),
                branch: None,
                name: None,
                force: false,
                approved_sources: Vec::new(),
            },
            "Submodule",
        ),
        (
            RepoCommandKind::UpdateSubmodules {
                approved_sources: Vec::new(),
            },
            "Submodule",
        ),
        (
            RepoCommandKind::RemoveSubmodule {
                path: PathBuf::from("mods/sub"),
            },
            "Submodule",
        ),
        (RepoCommandKind::StageHunk, "Hunk"),
        (RepoCommandKind::UnstageHunk, "Hunk"),
        (
            RepoCommandKind::ApplyWorktreePatch { reverse: true },
            "Discard",
        ),
        (
            RepoCommandKind::ApplyWorktreePatch { reverse: false },
            "Patch",
        ),
    ];

    for (command, label) in cases {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Err(Error::new(ErrorKind::Backend("boom".to_string()))),
            }),
        );

        let summary = state.repos[0]
            .feedback
            .command_log
            .last()
            .expect("command log entry")
            .summary
            .clone();
        assert!(
            summary.starts_with(&format!("{label} failed:")),
            "unexpected summary for label {label}: {summary}"
        );
    }
}

#[test]
fn repo_command_finished_success_summaries_cover_additional_commands() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let cases: Vec<(RepoCommandKind, &str)> = vec![
        (
            RepoCommandKind::SaveWorktreeFile {
                path: PathBuf::from("a.txt"),
                stage: true,
            },
            "Saved and staged → a.txt",
        ),
        (
            RepoCommandKind::SaveWorktreeFile {
                path: PathBuf::from("a.txt"),
                stage: false,
            },
            "Saved → a.txt",
        ),
        (
            RepoCommandKind::ExportPatch {
                commit_id: CommitId("deadbeef".into()),
                dest: PathBuf::from("out.patch"),
            },
            "Patch exported → out.patch",
        ),
        (
            RepoCommandKind::ApplyPatch {
                patch: PathBuf::from("in.patch"),
            },
            "Patch applied → in.patch",
        ),
        (
            RepoCommandKind::AddWorktree {
                path: PathBuf::from("../wt"),
                reference: Some("main".to_string()),
            },
            "Worktree added → ../wt (main)",
        ),
        (
            RepoCommandKind::AddWorktree {
                path: PathBuf::from("../wt"),
                reference: None,
            },
            "Worktree added → ../wt",
        ),
        (
            RepoCommandKind::RemoveWorktree {
                path: PathBuf::from("../wt"),
            },
            "Worktree removed → ../wt",
        ),
        (
            RepoCommandKind::AddSubmodule {
                url: "https://example.com/sub.git".to_string(),
                path: PathBuf::from("mods/sub"),
                branch: None,
                name: None,
                force: false,
                approved_sources: Vec::new(),
            },
            "Submodule added → mods/sub",
        ),
        (
            RepoCommandKind::UpdateSubmodules {
                approved_sources: Vec::new(),
            },
            "Submodules: Updated",
        ),
        (
            RepoCommandKind::RemoveSubmodule {
                path: PathBuf::from("mods/sub"),
            },
            "Submodule removed → mods/sub",
        ),
        (RepoCommandKind::StageHunk, "Hunk staged"),
        (RepoCommandKind::UnstageHunk, "Hunk unstaged"),
        (
            RepoCommandKind::ApplyWorktreePatch { reverse: true },
            "Changes discarded",
        ),
        (
            RepoCommandKind::ApplyWorktreePatch { reverse: false },
            "Patch applied",
        ),
    ];

    for (command, expected_summary) in cases {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Ok(CommandOutput::empty_success("git command")),
            }),
        );

        let summary = state.repos[0]
            .feedback
            .command_log
            .last()
            .expect("command log entry")
            .summary
            .clone();
        assert_eq!(summary, expected_summary);
    }
}

#[test]
fn checkout_branch_and_submodule_messages_emit_effects() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == RepoId(1)) {
        repo.set_detached_head_commit(Some(CommitId("deadbeef".into())));
    }
    state.active_repo = Some(RepoId(1));

    let checkout = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutBranch {
            repo_id: RepoId(1),
            name: "feature/x".to_string(),
        },
    );
    assert!(matches!(
        checkout.as_slice(),
        [Effect::CheckoutBranch {
            repo_id: RepoId(1),
            name
        }] if name == "feature/x"
    ));
    let repo = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(1))
        .expect("repo should exist");
    assert!(repo.detached_head_commit.is_none());

    let add_submodule = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AddSubmodule {
            repo_id: RepoId(1),
            url: "https://example.com/sub.git".to_string(),
            path: PathBuf::from("mods/sub"),
            branch: Some("feature".to_string()),
            name: None,
            force: false,
        },
    );
    assert!(matches!(
        add_submodule.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::CheckSubmoduleAddTrust {
            repo_id: RepoId(1),
            url,
            path,
            branch,
            ..
        })] if url == "https://example.com/sub.git"
            && path == &PathBuf::from("mods/sub")
            && branch.as_deref() == Some("feature")
    ));

    let update_submodules = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UpdateSubmodules { repo_id: RepoId(1) },
    );
    assert!(matches!(
        update_submodules.as_slice(),
        [Effect::RepoCommand(
            RepoCommandEffect::CheckSubmoduleUpdateTrust {
                repo_id: RepoId(1),
                ..
            }
        )]
    ));

    let remove_submodule = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RemoveSubmodule {
            repo_id: RepoId(1),
            path: PathBuf::from("mods/sub"),
        },
    );
    assert!(matches!(
        remove_submodule.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::RemoveSubmodule {
            repo_id: RepoId(1),
            path
        })] if path == &PathBuf::from("mods/sub")
    ));
}

#[test]
fn report_error_lands_on_its_repo_or_as_an_app_notification() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let report = |repo_id, message: &str| Msg::ReportError {
        repo_id,
        message: message.to_string(),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        report(Some(repo_id), "repo error"),
    );
    // A closed repo's error still reaches the user.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        report(Some(RepoId(9)), "closed repo"),
    );
    reduce(&mut repos, &id_alloc, &mut state, report(None, "app error"));
    reduce(&mut repos, &id_alloc, &mut state, report(None, "  "));

    let diagnostics = &state.repos[0].feedback.diagnostics;
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "repo error");
    let notifications = state
        .notifications
        .iter()
        .map(|notification| (notification.kind, notification.message.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        notifications,
        vec![
            (crate::model::AppNotificationKind::Error, "closed repo"),
            (crate::model::AppNotificationKind::Error, "app error"),
        ]
    );
}

#[test]
fn feedback_waits_unbounded_until_acknowledged_and_keeps_newer_entries() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let lifetime = state.repos[0].lifetime();
    for ix in 0..250 {
        for msg in [
            Msg::ReportError {
                repo_id: None,
                message: format!("app {ix}"),
            },
            Msg::ReportError {
                repo_id: Some(repo_id),
                message: format!("repo {ix}"),
            },
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::FetchAll,
                result: Err(Error::new(ErrorKind::Backend(format!("offline {ix}")))),
            }),
        ] {
            reduce(&mut repos, &id_alloc, &mut state, msg);
        }
    }
    let feedback = &state.repos[0].feedback;
    assert_eq!(state.notifications.len(), 250);
    assert_eq!(feedback.diagnostics.len(), 250);
    assert_eq!(feedback.command_log.len(), 250);

    // The window showed the first 240 of each; the rest arrived while its
    // acknowledgements were queued.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcknowledgeNotifications { through_seq: 240 },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcknowledgeRepoFeedback {
            repo_id,
            lifetime,
            diagnostics_through_seq: 240,
            command_log_through_seq: 240,
        },
    );
    let feedback = &state.repos[0].feedback;
    assert_eq!(state.notifications[0].message, "app 240");
    assert_eq!(state.notifications.len(), 10);
    assert_eq!(feedback.diagnostics[0].message, "repo 240");
    assert_eq!(feedback.diagnostics.len(), 10);
    assert!(feedback.command_log[0].stderr.contains("offline 240"));
    assert_eq!(feedback.command_log.len(), 10);

    // Another lifetime of the same repository id releases nothing here.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcknowledgeRepoFeedback {
            repo_id,
            lifetime: lifetime + 1,
            diagnostics_through_seq: 250,
            command_log_through_seq: 250,
        },
    );
    assert_eq!(state.repos[0].feedback.diagnostics.len(), 10);
    assert_eq!(state.repos[0].feedback.command_log.len(), 10);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcknowledgeNotifications { through_seq: 250 },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcknowledgeRepoFeedback {
            repo_id,
            lifetime,
            diagnostics_through_seq: 250,
            command_log_through_seq: 250,
        },
    );
    let feedback = &state.repos[0].feedback;
    assert!(state.notifications.is_empty());
    assert!(feedback.diagnostics.is_empty());
    assert!(feedback.command_log.is_empty());
    assert_eq!(
        (
            state.notifications_seq,
            feedback.diagnostics_seq,
            feedback.command_log_seq
        ),
        (250, 250, 250)
    );
}

#[test]
fn hunk_staging_is_logged_without_announcing_success() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    for command in [RepoCommandKind::StageHunk, RepoCommandKind::UnstageHunk] {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Ok(CommandOutput::default()),
            }),
        );
    }

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(
        repo_state.feedback.command_log.len(),
        2,
        "staging a hunk still belongs in the command log"
    );
    assert!(
        repo_state
            .feedback
            .command_log
            .iter()
            .all(|entry| !entry.announce_success),
        "a staged hunk shows itself in the diff; it must not raise a toast"
    );

    // A failure is still surfaced.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("patch does not apply".into()),
            )),
        }),
    );
    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let last = repo_state.feedback.command_log.last().unwrap();
    assert!(!last.ok, "the failure must be recorded as such");
}

#[test]
fn stage_hunk_command_finished_reloads_commit_png_image_preview_only() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let target = DiffTarget::commit(CommitId("abc123".into()), PathBuf::from("assets/icon.png"));
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.diff_state.diff_target = Some(target.clone());
    state.repos.push(repo_state);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::empty_success("git apply --cached")),
        }),
    );

    let repo = state.repos.first().expect("repo");
    assert!(repo.diff_state.diff.is_loading());
    assert!(matches!(repo.diff_state.diff_file, Loadable::NotLoaded));
    assert!(repo.diff_state.diff_file_image.is_loading());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadDiff {
            repo_id: RepoId(1),
            target: diff_target
        } if diff_target == &target
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadDiffFileImage {
            repo_id: RepoId(1),
            target: diff_target
        } if diff_target == &target
    )));
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::LoadDiffFile { .. })),
        "png reload should not request text diff"
    );
}
