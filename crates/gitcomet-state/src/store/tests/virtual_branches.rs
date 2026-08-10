use super::*;

fn fixture_repo() -> (HashMap<RepoId, Arc<dyn GitRepository>>, AtomicU64, AppState, RepoId) {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let repo_id = RepoId(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].set_open(Loadable::Ready(()));
    state.active_repo = Some(repo_id);
    (repos, id_alloc, state, repo_id)
}

fn branch_ids(state: &AppState) -> Vec<u64> {
    state.repos[0].virtual_branches.iter().map(|b| b.id).collect()
}

#[test]
fn create_assigns_incrementing_ids_and_default_name() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: String::new(),
        },
    );
    assert!(effects.is_empty());
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "feature".into(),
        },
    );
    assert!(effects.is_empty());
    let branches = &state.repos[0].virtual_branches;
    assert_eq!(branches.len(), 2);
    assert_eq!(branch_ids(&state), vec![1, 2]);
    assert_eq!(branches[0].name.as_ref(), "Branch 1");
    assert_eq!(branches[1].name.as_ref(), "feature");
    assert!(branches.iter().all(|b| b.applied && !b.pending && b.paths.is_empty()));
}

#[test]
fn assign_path_moves_it_between_branches() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    for name in ["a", "b"] {
        let _ = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::CreateVirtualBranch {
                repo_id,
                name: name.into(),
            },
        );
    }
    let path = PathBuf::from("src/lib.rs");
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AssignPathToVirtualBranch {
            repo_id,
            branch_id: 1,
            path: path.clone(),
        },
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AssignPathToVirtualBranch {
            repo_id,
            branch_id: 2,
            path: path.clone(),
        },
    );
    let branches = &state.repos[0].virtual_branches;
    assert!(branches[0].paths.is_empty());
    assert_eq!(branches[1].paths, vec![path]);
}

#[test]
fn unassign_path_removes_it_from_branch() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "a".into(),
        },
    );
    let path = PathBuf::from("src/lib.rs");
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AssignPathToVirtualBranch {
            repo_id,
            branch_id: 1,
            path: path.clone(),
        },
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnassignPathFromVirtualBranch {
            repo_id,
            branch_id: 1,
            path,
        },
    );
    assert!(state.repos[0].virtual_branches[0].paths.is_empty());
}

#[test]
fn rename_and_delete_virtual_branch() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "old".into(),
        },
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RenameVirtualBranch {
            repo_id,
            branch_id: 1,
            name: "new".into(),
        },
    );
    assert_eq!(state.repos[0].virtual_branches[0].name.as_ref(), "new");
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteVirtualBranch {
            repo_id,
            branch_id: 1,
        },
    );
    assert!(state.repos[0].virtual_branches.is_empty());
}

#[test]
fn unapply_dispatches_effect_with_paths_and_marks_pending() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "a".into(),
        },
    );
    let path = PathBuf::from("src/lib.rs");
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AssignPathToVirtualBranch {
            repo_id,
            branch_id: 1,
            path: path.clone(),
        },
    );
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnapplyVirtualBranch {
            repo_id,
            branch_id: 1,
        },
    );
    assert_eq!(effects.len(), 1);
    let Effect::UnapplyVirtualBranch {
        repo_id: rid,
        branch_id,
        paths: effect_paths,
    } = &effects[0]
    else {
        panic!("expected UnapplyVirtualBranch effect");
    };
    assert_eq!(*rid, repo_id);
    assert_eq!(*branch_id, 1);
    assert_eq!(*effect_paths, vec![path]);
    let branch = &state.repos[0].virtual_branches[0];
    assert!(branch.pending);
    assert!(branch.applied);
}

#[test]
fn unapply_without_paths_is_noop() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "empty".into(),
        },
    );
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnapplyVirtualBranch {
            repo_id,
            branch_id: 1,
        },
    );
    assert!(effects.is_empty());
    let branch = &state.repos[0].virtual_branches[0];
    assert!(branch.applied && !branch.pending);
}

#[test]
fn unapplied_result_stores_patch_and_marks_unapplied() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "a".into(),
        },
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AssignPathToVirtualBranch {
            repo_id,
            branch_id: 1,
            path: PathBuf::from("src/lib.rs"),
        },
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnapplyVirtualBranch {
            repo_id,
            branch_id: 1,
        },
    );
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::VirtualBranchUnapplied {
            repo_id,
            branch_id: 1,
            result: Ok("diff --git a/src/lib.rs b/src/lib.rs".to_string()),
        }),
    );
    assert!(effects.is_empty());
    let branch = &state.repos[0].virtual_branches[0];
    assert!(!branch.pending);
    assert!(!branch.applied);
    assert_eq!(
        branch.stored_patch.as_deref(),
        Some("diff --git a/src/lib.rs b/src/lib.rs")
    );
}

#[test]
fn unapplied_error_keeps_applied_and_reports_diagnostic() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "a".into(),
        },
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AssignPathToVirtualBranch {
            repo_id,
            branch_id: 1,
            path: PathBuf::from("src/lib.rs"),
        },
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnapplyVirtualBranch {
            repo_id,
            branch_id: 1,
        },
    );
    let err = gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Backend(
        "boom".into(),
    ));
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::VirtualBranchUnapplied {
            repo_id,
            branch_id: 1,
            result: Err(err),
        }),
    );
    let branch = &state.repos[0].virtual_branches[0];
    assert!(!branch.pending);
    assert!(branch.applied);
    assert!(branch.stored_patch.is_none());
    assert!(!state.repos[0].diagnostics.is_empty());
}

#[test]
fn apply_dispatches_effect_and_clears_patch_on_success() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "a".into(),
        },
    );
    let patch = "diff --git a/src/lib.rs b/src/lib.rs".to_string();
    state.repos[0].virtual_branches[0].applied = false;
    state.repos[0].virtual_branches[0].stored_patch = Some(patch.clone().into());
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyVirtualBranch {
            repo_id,
            branch_id: 1,
        },
    );
    assert_eq!(effects.len(), 1);
    let Effect::ApplyVirtualBranch {
        repo_id: rid,
        branch_id,
        patch: effect_patch,
    } = &effects[0]
    else {
        panic!("expected ApplyVirtualBranch effect");
    };
    assert_eq!(*rid, repo_id);
    assert_eq!(*branch_id, 1);
    assert_eq!(*effect_patch, patch);
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::VirtualBranchApplied {
            repo_id,
            branch_id: 1,
            result: Ok(()),
        }),
    );
    let branch = &state.repos[0].virtual_branches[0];
    assert!(!branch.pending);
    assert!(branch.applied);
    assert!(branch.stored_patch.is_none());
}

#[test]
fn apply_without_stored_patch_is_noop() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "a".into(),
        },
    );
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyVirtualBranch {
            repo_id,
            branch_id: 1,
        },
    );
    assert!(effects.is_empty());
}
