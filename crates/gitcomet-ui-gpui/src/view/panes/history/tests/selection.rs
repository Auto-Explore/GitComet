use super::*;

#[test]
fn graph_branch_heads_are_hidden_for_current_branch_scope() {
    let branches = vec![branch("main", "local-head")];
    let remote_branches = vec![remote_branch("origin", "feature/x", "remote-head")];

    let mut current_branch_heads =
        graph_branch_heads(LogScope::CurrentBranch, &branches, &remote_branches);
    assert!(current_branch_heads.next().is_none());

    let all_branch_heads =
        graph_branch_heads(LogScope::AllBranches, &branches, &remote_branches).collect::<Vec<_>>();
    assert_eq!(all_branch_heads.len(), 2);
    assert!(all_branch_heads.contains(&"local-head"));
    assert!(all_branch_heads.contains(&"remote-head"));
}

#[test]
fn selected_branch_for_history_row_carries_branch_identity() {
    let selected_branch = SelectedBranch {
        repo_id: RepoId(7),
        section: BranchSection::Local,
        name: "main".into(),
    };

    assert_eq!(
        selected_branch_for_history_row(Some(&selected_branch), RepoId(7), true),
        Some(SelectedHistoryBranch {
            section: BranchSection::Local,
            name: "main".into(),
        })
    );
}

#[test]
fn selected_branch_for_history_row_keeps_the_remote_section() {
    let selected_branch = SelectedBranch {
        repo_id: RepoId(7),
        section: BranchSection::Remote,
        name: "origin/feature/topic".into(),
    };

    assert_eq!(
        selected_branch_for_history_row(Some(&selected_branch), RepoId(7), true),
        Some(SelectedHistoryBranch {
            section: BranchSection::Remote,
            name: "origin/feature/topic".into(),
        })
    );
}

#[test]
fn selected_branch_for_history_row_requires_selected_row_and_matching_repo() {
    let selected_branch = SelectedBranch {
        repo_id: RepoId(7),
        section: BranchSection::Local,
        name: "main".into(),
    };

    assert_eq!(
        selected_branch_for_history_row(Some(&selected_branch), RepoId(8), true),
        None
    );
    assert_eq!(
        selected_branch_for_history_row(Some(&selected_branch), RepoId(7), false),
        None
    );
}
