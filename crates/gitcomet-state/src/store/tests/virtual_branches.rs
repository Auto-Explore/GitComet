use super::*;

fn fixture_repo() -> (HashMap<RepoId, Arc<dyn GitRepository>>, AtomicU64, AppState, RepoId) {
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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

fn move_hunk_patch() -> String {
    "@@ -1,3 +1,4 @@\n fn a() {\n+    let x = 1;\n-    let y = 2;\n }\n".to_string()
}

#[test]
fn move_hunk_dispatches_reverse_apply_effect_and_marks_pending() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "target".into(),
        },
    );
    let path = PathBuf::from("src/lib.rs");
    let patch = move_hunk_patch();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::MoveHunkToVirtualBranch {
            repo_id,
            branch_id: 1,
            patch: patch.clone(),
            path: path.clone(),
        },
    );
    assert_eq!(effects.len(), 1);
    let Effect::MoveHunkToVirtualBranch {
        repo_id: rid,
        branch_id,
        patch: effect_patch,
        path: effect_path,
    } = &effects[0]
    else {
        panic!("expected MoveHunkToVirtualBranch effect");
    };
    assert_eq!(*rid, repo_id);
    assert_eq!(*branch_id, 1);
    assert_eq!(*effect_patch, patch);
    assert_eq!(*effect_path, path);
    let branch = &state.repos[0].virtual_branches[0];
    assert!(branch.pending);
    assert!(branch.applied);
    assert!(branch.stored_patch.is_none());
    assert!(branch.paths.is_empty());
}

#[test]
fn move_hunk_empty_patch_is_noop() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "target".into(),
        },
    );
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::MoveHunkToVirtualBranch {
            repo_id,
            branch_id: 1,
            patch: "   ".to_string(),
            path: PathBuf::from("src/lib.rs"),
        },
    );
    assert!(effects.is_empty());
    let branch = &state.repos[0].virtual_branches[0];
    assert!(!branch.pending);
}

#[test]
fn move_hunk_success_parks_patch_and_assigns_path() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    for name in ["other", "target"] {
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
    // The path is already assigned to branch 1; moving a hunk of it to
    // branch 2 should reassign the file to branch 2.
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
    let patch = move_hunk_patch();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::MoveHunkToVirtualBranch {
            repo_id,
            branch_id: 2,
            patch: patch.clone(),
            path: path.clone(),
        },
    );
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::VirtualBranchHunkMoved {
            repo_id,
            branch_id: 2,
            path: path.clone(),
            result: Ok(patch.clone()),
        }),
    );
    assert!(effects.is_empty());
    let branches = &state.repos[0].virtual_branches;
    let target = &branches[1];
    assert!(!target.pending);
    assert!(!target.applied);
    assert_eq!(target.stored_patch.as_deref(), Some(patch.trim_end()));
    assert_eq!(target.paths, vec![path]);
    assert!(branches[0].paths.is_empty());
}

#[test]
fn move_hunk_appends_to_existing_parked_patch() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "target".into(),
        },
    );
    let first = "@@ -1,1 +1,1 @@\n-a\n+b\n".to_string();
    let second = "@@ -5,1 +5,1 @@\n-c\n+d\n".to_string();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::MoveHunkToVirtualBranch {
            repo_id,
            branch_id: 1,
            patch: first.clone(),
            path: PathBuf::from("src/a.rs"),
        },
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::VirtualBranchHunkMoved {
            repo_id,
            branch_id: 1,
            path: PathBuf::from("src/a.rs"),
            result: Ok(first.clone()),
        }),
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::MoveHunkToVirtualBranch {
            repo_id,
            branch_id: 1,
            patch: second.clone(),
            path: PathBuf::from("src/b.rs"),
        },
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::VirtualBranchHunkMoved {
            repo_id,
            branch_id: 1,
            path: PathBuf::from("src/b.rs"),
            result: Ok(second.clone()),
        }),
    );
    let branch = &state.repos[0].virtual_branches[0];
    assert_eq!(
        branch.stored_patch.as_deref(),
        Some(format!("{}\n{}", first.trim_end(), second.trim_end()).as_str())
    );
    assert_eq!(branch.paths.len(), 2);
}

#[test]
fn move_hunk_error_reports_diagnostic_without_recording() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "target".into(),
        },
    );
    let path = PathBuf::from("src/lib.rs");
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::MoveHunkToVirtualBranch {
            repo_id,
            branch_id: 1,
            patch: move_hunk_patch(),
            path: path.clone(),
        },
    );
    let err = gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Backend(
        "patch does not apply".into(),
    ));
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::VirtualBranchHunkMoved {
            repo_id,
            branch_id: 1,
            path: path.clone(),
            result: Err(err),
        }),
    );
    let branch = &state.repos[0].virtual_branches[0];
    assert!(!branch.pending);
    assert!(branch.applied);
    assert!(branch.stored_patch.is_none());
    assert!(branch.paths.is_empty());
    assert!(!state.repos[0].diagnostics.is_empty());
}

#[test]
fn unapply_after_move_appends_captured_diff_to_parked_patch() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateVirtualBranch {
            repo_id,
            name: "target".into(),
        },
    );
    let parked = "@@ -1,1 +1,1 @@\n-a\n+b\n".to_string();
    // Park a hunk first.
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::VirtualBranchHunkMoved {
            repo_id,
            branch_id: 1,
            path: PathBuf::from("src/lib.rs"),
            result: Ok(parked.clone()),
        }),
    );
    let captured = "diff --git a/src/lib.rs b/src/lib.rs".to_string();
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::VirtualBranchUnapplied {
            repo_id,
            branch_id: 1,
            result: Ok(captured.clone()),
        }),
    );
    let branch = &state.repos[0].virtual_branches[0];
    assert_eq!(
        branch.stored_patch.as_deref(),
        Some(format!("{}\n{captured}", parked.trim_end()).as_str())
    );
}

fn set_status(state: &mut AppState, paths: &[&str]) {
    use gitcomet_core::domain::{FileStatus, FileStatusKind};
    state.repos[0].worktree_status = Loadable::Ready(Arc::new(
        paths
            .iter()
            .map(|p| FileStatus {
                path: PathBuf::from(p),
                kind: FileStatusKind::Modified,
                conflict: None,
            })
            .collect(),
    ));
}

#[test]
fn stale_virtual_branch_ids_ignores_branches_with_changes_or_parked_patches() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    for name in ["changed", "stale", "parked"] {
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
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AssignPathToVirtualBranch {
            repo_id,
            branch_id: 1,
            path: PathBuf::from("src/active.rs"),
        },
    );
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AssignPathToVirtualBranch {
            repo_id,
            branch_id: 2,
            path: PathBuf::from("src/gone.rs"),
        },
    );
    // Branch 3 has a parked patch: it must never be pruned.
    state.repos[0].virtual_branches[2].stored_patch = Some("parked diff".into());
    state.repos[0].virtual_branches[2].applied = false;
    set_status(&mut state, &["src/active.rs"]);

    let stale = crate::model::stale_virtual_branch_ids(&state.repos[0]);
    assert_eq!(stale, vec![2]);
}

#[test]
fn stale_virtual_branch_ids_treats_empty_branches_as_stale() {
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
    set_status(&mut state, &[]);
    let stale = crate::model::stale_virtual_branch_ids(&state.repos[0]);
    assert_eq!(stale, vec![1]);
}

#[test]
fn prune_virtual_branches_removes_only_given_ids_and_persists() {
    let (mut repos, id_alloc, mut state, repo_id) = fixture_repo();
    for _ in 0..3 {
        let _ = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::CreateVirtualBranch {
                repo_id,
                name: String::new(),
            },
        );
    }
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PruneVirtualBranches {
            repo_id,
            branch_ids: vec![1, 3],
        },
    );
    assert!(has_persist_effect(&effects));
    assert_eq!(branch_ids(&state), vec![2]);

    // Pruning nothing is a no-op.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PruneVirtualBranches {
            repo_id,
            branch_ids: vec![999],
        },
    );
    assert!(effects.is_empty());
    assert_eq!(branch_ids(&state), vec![2]);
}
