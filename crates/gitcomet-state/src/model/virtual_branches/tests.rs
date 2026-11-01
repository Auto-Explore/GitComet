use super::*;

fn state_with(names: &[(&str, Option<&str>, BranchApplyState)]) -> VirtualBranchesState {
    VirtualBranchesState::new("main").with_branches(
        names
            .iter()
            .map(|(name, parent, applied)| {
                let mut branch = VirtualBranch::new(*name);
                if let Some(parent) = parent {
                    branch = branch.with_parent(*parent);
                }
                branch.applied = *applied;
                branch
            })
            .collect(),
    )
}

fn loaded(names: &[(&str, Option<&str>, BranchApplyState)]) -> VirtualBranchesRepoState {
    let mut virtual_branches = VirtualBranchesRepoState::default();
    virtual_branches.set_state(state_with(names));
    virtual_branches
}

#[test]
fn set_state_derives_stacked_branches() {
    let virtual_branches = loaded(&[
        ("api", None, BranchApplyState::Applied),
        ("ui", Some("api"), BranchApplyState::Applied),
    ]);
    assert_eq!(virtual_branches.stacked_branches.len(), 1);
    assert_eq!(virtual_branches.total_count(), 2);
    assert_eq!(virtual_branches.applied_count(), 2);
}

#[test]
fn unapplied_branches_are_counted_separately() {
    let virtual_branches = loaded(&[
        ("api", None, BranchApplyState::Applied),
        ("parked", None, BranchApplyState::Unapplied),
    ]);
    assert_eq!(virtual_branches.total_count(), 2);
    assert_eq!(virtual_branches.applied_count(), 1);
}

#[test]
fn rev_advances_on_every_change() {
    let mut virtual_branches = VirtualBranchesRepoState::default();
    assert_eq!(virtual_branches.rev, 0);
    virtual_branches.set_state(VirtualBranchesState::new("main"));
    assert_eq!(virtual_branches.rev, 1);
    virtual_branches.bump_rev();
    assert_eq!(virtual_branches.rev, 2);
}

#[test]
fn files_are_grouped_by_their_branch() {
    let mut virtual_branches = loaded(&[("api", None, BranchApplyState::Applied)]);
    let mut index = AssignmentIndex::new();
    index.set(PathBuf::from("src/a.rs"), Some("api".into()));
    index.set(PathBuf::from("src/b.rs"), None);
    virtual_branches.assignments = Arc::new(index);

    let files =
        virtual_branches.files_by_branch(&[PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")]);
    assert_eq!(files[0].branch.as_deref(), Some("api"));
    assert_eq!(files[1].branch, None);
}

#[test]
fn a_file_assigned_to_a_deleted_branch_reads_as_unassigned() {
    let mut virtual_branches = loaded(&[("api", None, BranchApplyState::Applied)]);
    let mut index = AssignmentIndex::new();
    index.set(PathBuf::from("src/a.rs"), Some("gone".into()));
    virtual_branches.assignments = Arc::new(index);

    let files = virtual_branches.files_by_branch(&[PathBuf::from("src/a.rs")]);
    assert_eq!(files[0].branch, None);
}

#[test]
fn summary_counts_cover_assigned_and_unassigned_files() {
    let mut virtual_branches = loaded(&[
        ("api", None, BranchApplyState::Applied),
        ("ui", None, BranchApplyState::Applied),
    ]);
    let mut index = AssignmentIndex::new();
    index.set(PathBuf::from("a.rs"), Some("api".into()));
    index.set(PathBuf::from("b.rs"), Some("api".into()));
    index.set(PathBuf::from("c.rs"), Some("ui".into()));
    index.set(PathBuf::from("d.rs"), None);
    virtual_branches.assignments = Arc::new(index);

    let summary = virtual_branches.summary_counts(&[
        PathBuf::from("a.rs"),
        PathBuf::from("b.rs"),
        PathBuf::from("c.rs"),
        PathBuf::from("d.rs"),
    ]);
    assert_eq!(summary.per_branch.get("api"), Some(&2));
    assert_eq!(summary.per_branch.get("ui"), Some(&1));
    assert_eq!(summary.unassigned, 1);
    assert_eq!(summary.applied, 2);
}

#[test]
fn a_split_file_reads_as_belonging_to_every_branch_that_has_part_of_it() {
    // A split file has no single branch, so `branch` is `None` — but
    // reporting that as unassigned would send the user looking for a file
    // that two branches are already committing.
    let mut virtual_branches = loaded(&[
        ("api", None, BranchApplyState::Applied),
        ("ui", None, BranchApplyState::Applied),
    ]);
    let mut index = AssignmentIndex::new();
    let first = gitcomet_core::virtual_branches::HunkFingerprint::of(&["api\n"], 1);
    let second = gitcomet_core::virtual_branches::HunkFingerprint::of(&["ui\n"], 1);
    index.set_hunk(PathBuf::from("a.rs"), first, Some("api".into()));
    index.set_hunk(PathBuf::from("a.rs"), second, Some("ui".into()));
    virtual_branches.assignments = Arc::new(index);

    let files = virtual_branches.files_by_branch(&[PathBuf::from("a.rs")]);
    assert_eq!(files[0].branch, None, "it is on no single branch");
    assert_eq!(files[0].split, ["api", "ui"], "it is on both");
    assert!(files[0].is_assigned());

    let summary = virtual_branches.summary_counts(&[PathBuf::from("a.rs")]);
    assert_eq!(summary.per_branch.get("api"), Some(&1));
    assert_eq!(summary.per_branch.get("ui"), Some(&1));
    assert_eq!(summary.unassigned, 0, "nothing here will fail to commit");
}

#[test]
fn a_hunk_on_a_deleted_branch_does_not_keep_a_file_assigned() {
    let mut virtual_branches = loaded(&[("api", None, BranchApplyState::Applied)]);
    let mut index = AssignmentIndex::new();
    index.set_hunk(
        PathBuf::from("a.rs"),
        gitcomet_core::virtual_branches::HunkFingerprint::of(&["new\n"], 1),
        Some("gone".into()),
    );
    virtual_branches.assignments = Arc::new(index);

    let files = virtual_branches.files_by_branch(&[PathBuf::from("a.rs")]);
    assert!(files[0].split.is_empty());
    assert!(!files[0].is_assigned());
}

#[test]
fn switching_the_working_directory_counts_as_busy() {
    // Entering the virtual branches moves HEAD under the user. Nothing else may
    // offer to run while that is in flight.
    let mut virtual_branches = VirtualBranchesRepoState::default();
    virtual_branches.busy.switching = true;
    assert!(virtual_branches.busy.any());
    assert!(!virtual_branches.can_commit(1));
}

#[test]
fn clearing_forgets_that_the_working_directory_was_in_the_virtual_branches() {
    let mut virtual_branches = VirtualBranchesRepoState {
        active: true,
        checkout_base: Some("main".into()),
        ..Default::default()
    };
    virtual_branches.clear();
    assert!(!virtual_branches.active);
    assert!(
        virtual_branches.checkout_base.is_none(),
        "a remembered branch must not outlive the session that remembered it"
    );
}

#[test]
fn can_commit_requires_changes_applied_branches_and_idle_state() {
    let virtual_branches = loaded(&[("api", None, BranchApplyState::Applied)]);
    assert!(virtual_branches.can_commit(1));
    assert!(!virtual_branches.can_commit(0));

    let mut busy = virtual_branches.clone();
    busy.busy.mutating = true;
    assert!(!busy.can_commit(1));

    let nothing_applied = loaded(&[("api", None, BranchApplyState::Unapplied)]);
    assert!(!nothing_applied.can_commit(1));
}

#[test]
fn clearing_resets_every_field() {
    let mut virtual_branches = loaded(&[("api", None, BranchApplyState::Applied)]);
    virtual_branches.busy.applying = true;
    virtual_branches.clear();
    assert!(virtual_branches.virtual_branches().is_none());
    assert!(virtual_branches.stacked_branches.is_empty());
    assert!(virtual_branches.assignments.is_empty());
    assert!(!virtual_branches.busy.any());
}

#[test]
fn a_corrupt_virtual_branches_still_renders_empty_stacked_branches() {
    let mut virtual_branches = VirtualBranchesRepoState::default();
    // A cycle cannot produce stacked branches, but it must not panic the view.
    let cyclic = VirtualBranchesState::new("main").with_branches(vec![
        VirtualBranch::new("a").with_parent("b"),
        VirtualBranch::new("b").with_parent("a"),
    ]);
    virtual_branches.set_state(cyclic);
    assert!(virtual_branches.stacked_branches.is_empty());
    assert!(virtual_branches.virtual_branches().is_some());
}

#[test]
fn edits_report_which_branch_they_act_on() {
    assert_eq!(
        VirtualBranchesEdit::Create { name: "a".into() }.branch_name(),
        Some("a")
    );
    assert_eq!(
        VirtualBranchesEdit::AssignFile {
            path: PathBuf::from("a.rs"),
            branch: Some("api".into()),
        }
        .branch_name(),
        Some("api")
    );
    assert_eq!(
        VirtualBranchesEdit::SetTarget { target: "x".into() }.branch_name(),
        None
    );
}

#[test]
fn edits_that_move_the_applied_set_are_marked() {
    assert!(
        VirtualBranchesEdit::SetApplied {
            name: "a".into(),
            applied: BranchApplyState::Applied,
        }
        .changes_applied_set()
    );
    assert!(VirtualBranchesEdit::SetTarget { target: "x".into() }.changes_applied_set());
    // A new branch is applied by default, and a removed one stops
    // contributing, so both have to be replayed into the virtual-branches branch.
    assert!(VirtualBranchesEdit::Create { name: "a".into() }.changes_applied_set());
    assert!(VirtualBranchesEdit::Remove { name: "a".into() }.changes_applied_set());
    assert!(
        !VirtualBranchesEdit::Reorder {
            first: "a".into(),
            second: "b".into(),
        }
        .changes_applied_set()
    );
    assert!(
        !VirtualBranchesEdit::AssignFile {
            path: PathBuf::from("a.rs"),
            branch: Some("a".into()),
        }
        .changes_applied_set()
    );
}

#[test]
fn only_base_moves_rewrite_history() {
    assert!(
        VirtualBranchesEdit::SetParent {
            name: "a".into(),
            parent: None,
        }
        .rewrites_history()
    );
    assert!(!VirtualBranchesEdit::Create { name: "a".into() }.rewrites_history());
    assert!(
        !VirtualBranchesEdit::AssignFile {
            path: PathBuf::from("a.rs"),
            branch: None,
        }
        .rewrites_history()
    );
}

#[test]
fn only_inserting_below_replays_the_anchor() {
    let insert = |below| VirtualBranchesEdit::InsertRelativeTo {
        name: "b".into(),
        anchor: "a".into(),
        below,
    };
    // Above stacked branches on the anchor: nothing that existed changes base.
    assert!(!insert(false).rewrites_history());
    // Below takes the anchor's place, so the anchor is replayed on it.
    assert!(insert(true).rewrites_history());
}

#[test]
fn inserting_a_branch_moves_the_applied_set() {
    // A new branch is applied by default, like any other creation.
    assert!(
        VirtualBranchesEdit::InsertRelativeTo {
            name: "b".into(),
            anchor: "a".into(),
            below: true,
        }
        .changes_applied_set()
    );
    assert_eq!(
        VirtualBranchesEdit::InsertRelativeTo {
            name: "b".into(),
            anchor: "a".into(),
            below: true,
        }
        .branch_name(),
        Some("b"),
        "the created branch is the one to report progress on"
    );
}

#[test]
fn only_committing_produces_a_commit() {
    assert!(
        VirtualBranchesEdit::CommitPaths {
            name: "a".into(),
            message: "m".into(),
            paths: vec![PathBuf::from("a.rs")],
        }
        .produces_commit()
    );
    assert!(
        !VirtualBranchesEdit::SetApplied {
            name: "a".into(),
            applied: BranchApplyState::Applied,
        }
        .produces_commit()
    );
    assert_eq!(
        VirtualBranchesEdit::CommitPaths {
            name: "a".into(),
            message: "m".into(),
            paths: Vec::new(),
        }
        .branch_name(),
        Some("a")
    );
}

#[test]
fn a_conflict_summary_names_both_branches() {
    let conflict = VirtualBranchesConflict {
        branch: "feature/ui".into(),
        against: Some("feature/api".into()),
        paths: Arc::new(Vec::new()),
        message: "conflict".into(),
    };
    assert_eq!(conflict.summary(), "feature/ui conflicts with feature/api");

    let alone = VirtualBranchesConflict {
        branch: "feature/ui".into(),
        against: None,
        paths: Arc::new(Vec::new()),
        message: "conflict".into(),
    };
    assert_eq!(alone.summary(), "feature/ui could not be applied");
}
