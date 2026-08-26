use super::*;

/// The whole focus rule for a sidebar worktree click, in one table.
#[test]
fn a_worktree_click_focuses_its_changes_or_the_commit_it_sits_on() {
    let head = CommitId("head-sha".into());

    // This tab's own changes are the pinned row at the top of the log.
    assert_eq!(
        worktree_reveal_target(true, true, Some(false), Some(head.clone())),
        WorktreeRevealTarget::WorkingTreeSummaryRow
    );
    // Clean, so there is no row -- land on what it is checked out at. No
    // fallback scope: the current worktree's HEAD is in scope by definition.
    assert_eq!(
        worktree_reveal_target(true, false, Some(false), Some(head.clone())),
        WorktreeRevealTarget::Commit {
            head: head.clone(),
            fallback_scope: None,
        }
    );
    // A linked worktree's changes live in a row of their own.
    assert_eq!(
        worktree_reveal_target(false, false, Some(true), Some(head.clone())),
        WorktreeRevealTarget::WorktreeRow {
            head: head.clone(),
            fallback_scope: Some(LogScope::AllBranches),
        }
    );
    // Clean linked worktree: its branch may sit outside the current scope.
    assert_eq!(
        worktree_reveal_target(false, false, Some(false), Some(head.clone())),
        WorktreeRevealTarget::Commit {
            head: head.clone(),
            fallback_scope: Some(LogScope::AllBranches),
        }
    );
}

/// The first scan has not replied when a repo opens, and "no answer yet" is
/// not the answer that the worktree is clean. Aiming at the commit on an
/// unknown fixes the reveal against a row set that is about to grow.
#[test]
fn an_unscanned_worktree_is_revealed_as_a_row_not_as_its_commit() {
    let head = CommitId("head-sha".into());
    assert_eq!(
        worktree_reveal_target(false, false, None, Some(head.clone())),
        WorktreeRevealTarget::WorktreeRow {
            head,
            fallback_scope: Some(LogScope::AllBranches),
        }
    );
}

/// The current worktree's own changes never appear as a linked-worktree row,
/// so a dirty *other* worktree must not divert this tab's click.
#[test]
fn the_current_worktree_ignores_other_worktrees_dirt() {
    let head = CommitId("head-sha".into());
    assert_eq!(
        worktree_reveal_target(true, true, Some(true), Some(head)),
        WorktreeRevealTarget::WorkingTreeSummaryRow
    );
}

#[test]
fn a_clean_worktree_with_no_resolvable_head_focuses_nothing() {
    assert_eq!(
        worktree_reveal_target(false, false, Some(false), None),
        WorktreeRevealTarget::Nothing
    );
    // Even a dirty one: its row is anchored by that same HEAD.
    assert_eq!(
        worktree_reveal_target(false, false, Some(true), None),
        WorktreeRevealTarget::Nothing
    );
}

/// Selecting a worktree row also leaves the commit selection empty, which is
/// the state the working-tree row uses to decide it is selected. Claiming
/// index 0 here is what made both rows light up at once.
#[test]
fn a_selected_worktree_row_does_not_claim_the_working_tree_row() {
    let plan = HistoryListPlan::new(true, Vec::new());
    let commits = vec![commit("aaa", &[], "tip")];
    let visible = HistoryVisibleIndices::all(1);

    let working_tree = peek_history_selected_list_index(
        None,
        RepoId(1),
        1,
        1,
        LogScope::AllBranches,
        &plan,
        HistorySelectionRef {
            commit: None,
            worktree_selected: false,
        },
        &visible,
        &commits,
    );
    assert_eq!(
        working_tree,
        Some(0),
        "with nothing else selected the working-tree row owns index 0"
    );

    let worktree = peek_history_selected_list_index(
        None,
        RepoId(1),
        1,
        1,
        LogScope::AllBranches,
        &plan,
        HistorySelectionRef {
            commit: None,
            worktree_selected: true,
        },
        &visible,
        &commits,
    );
    assert_eq!(
        worktree, None,
        "a selected worktree row must not report the working-tree row's index"
    );
}

#[test]
fn resolve_history_selected_list_index_populates_cache_for_commit_selection() {
    let commits = vec![
        commit("a", &["p0"], "a"),
        commit("b", &["a"], "b"),
        commit("c", &["b"], "c"),
    ];
    let selected = CommitId("c".into());
    let mut cache = None;

    let list_ix = resolve_history_selected_list_index(
        &mut cache,
        RepoId(7),
        11,
        13,
        LogScope::AllBranches,
        &HistoryListPlan::new(true, Vec::new()),
        HistorySelectionRef {
            commit: Some(&selected),
            worktree_selected: false,
        },
        &HistoryVisibleIndices::Filtered(vec![0, 2].into()),
        &commits,
    );

    assert_eq!(list_ix, Some(2));
    assert_eq!(
        cache,
        Some(HistorySelectedListIndexCache {
            repo_id: RepoId(7),
            log_rev: 11,
            stashes_rev: 13,
            history_scope: LogScope::AllBranches,
            show_working_tree_summary_row: true,
            plan_fingerprint: HistoryListPlan::new(true, Vec::new()).fingerprint(),
            selected_commit: Some(selected),
            list_ix: 2,
        })
    );
}

#[test]
fn resolve_history_selected_list_index_reuses_matching_cache() {
    let selected = CommitId("cached".into());
    let mut cache = Some(HistorySelectedListIndexCache {
        repo_id: RepoId(3),
        log_rev: 21,
        stashes_rev: 34,
        history_scope: LogScope::CurrentBranch,
        show_working_tree_summary_row: false,
        plan_fingerprint: HistoryListPlan::new(false, Vec::new()).fingerprint(),
        selected_commit: Some(selected.clone()),
        list_ix: 5,
    });

    let list_ix = resolve_history_selected_list_index(
        &mut cache,
        RepoId(3),
        21,
        34,
        LogScope::CurrentBranch,
        &HistoryListPlan::new(false, Vec::new()),
        HistorySelectionRef {
            commit: Some(&selected),
            worktree_selected: false,
        },
        &HistoryVisibleIndices::all(0),
        &[],
    );

    assert_eq!(list_ix, Some(5));
}

#[test]
fn pending_history_reveal_visible_target_scrolls_and_clears() {
    let commits = vec![
        commit("a", &["p0"], "a"),
        commit("b", &["a"], "b"),
        commit("c", &["b"], "c"),
    ];
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: CommitId("c".into()),
        fallback_scope: Some(LogScope::AllBranches),
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        None,
        11,
        13,
        false,
        Some(&log_page(commits, None)),
        Some(false),
        true,
        Some(&HistoryVisibleIndices::Filtered(vec![0, 2].into())),
        &HistoryListPlan::new(true, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: None,
            select_commit: Some(CommitId("c".into())),
            scroll_to_list_ix: Some(2),
            load_more: false,
            clear_pending: true,
        }
    );
}

#[test]
fn pending_history_reveal_missing_target_requests_load_more() {
    let commits = vec![commit("a", &["p0"], "a"), commit("b", &["a"], "b")];
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: CommitId("c".into()),
        fallback_scope: Some(LogScope::AllBranches),
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        None,
        11,
        13,
        false,
        Some(&log_page(commits, Some("b"))),
        Some(true),
        true,
        Some(&HistoryVisibleIndices::all(2)),
        &HistoryListPlan::new(false, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: None,
            // Selecting is `Msg::RevealCommit`'s job; a target that has not
            // been paged in yet is nobody's cue to touch the selection.
            select_commit: None,
            scroll_to_list_ix: None,
            load_more: true,
            clear_pending: false,
        }
    );
}

#[test]
fn pending_history_reveal_switches_to_fallback_scope_after_exhausting_current_mode() {
    let commits = vec![commit("a", &["p0"], "a"), commit("b", &["a"], "b")];
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: CommitId("c".into()),
        fallback_scope: Some(LogScope::AllBranches),
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        None,
        11,
        13,
        false,
        Some(&log_page(commits, None)),
        Some(false),
        true,
        Some(&HistoryVisibleIndices::all(2)),
        &HistoryListPlan::new(false, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: Some(LogScope::AllBranches),
            select_commit: None,
            scroll_to_list_ix: None,
            load_more: false,
            clear_pending: false,
        }
    );
}

#[test]
fn pending_history_reveal_missing_target_with_exhausted_history_and_no_fallback_clears() {
    let commits = vec![commit("a", &["p0"], "a"), commit("b", &["a"], "b")];
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: CommitId("c".into()),
        fallback_scope: None,
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        None,
        11,
        13,
        false,
        Some(&log_page(commits, None)),
        Some(false),
        true,
        Some(&HistoryVisibleIndices::all(2)),
        &HistoryListPlan::new(false, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: None,
            select_commit: None,
            scroll_to_list_ix: None,
            load_more: false,
            clear_pending: true,
        }
    );
}

#[test]
fn pending_history_reveal_already_selected_commit_still_scrolls() {
    let commits = vec![commit("a", &["p0"], "a"), commit("b", &["a"], "b")];
    let selected = CommitId("b".into());
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: selected.clone(),
        fallback_scope: None,
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        Some(&selected),
        21,
        34,
        false,
        Some(&log_page(commits, None)),
        Some(false),
        true,
        Some(&HistoryVisibleIndices::all(2)),
        &HistoryListPlan::new(false, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: None,
            select_commit: None,
            scroll_to_list_ix: Some(1),
            load_more: false,
            clear_pending: true,
        }
    );
}

#[test]
fn pending_history_reveal_unique_abbreviated_commit_scrolls_and_selects_full_id() {
    let full = "abcdef0123456789abcdef0123456789abcdef01";
    let other = "1234567890abcdef1234567890abcdef12345678";
    let commits = vec![
        commit(other, &["p0"], "other"),
        commit(full, &[other], "target"),
    ];
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: CommitId(full[..8].into()),
        fallback_scope: Some(LogScope::AllBranches),
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        None,
        21,
        34,
        false,
        Some(&log_page(commits, None)),
        Some(false),
        true,
        Some(&HistoryVisibleIndices::all(2)),
        &HistoryListPlan::new(false, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: None,
            select_commit: Some(CommitId(full.into())),
            scroll_to_list_ix: Some(1),
            load_more: false,
            clear_pending: true,
        }
    );
}

/// An abbreviation used to force loading the *entire* history before it
/// could be trusted as unambiguous. `Msg::RevealCommit` settles ambiguity
/// against the object database instead, so a visible match is taken at once
/// even with pages left to load.
#[test]
fn pending_history_reveal_abbreviated_commit_takes_a_visible_match_with_pages_left() {
    let full = "abcdef0123456789abcdef0123456789abcdef01";
    let other = "1234567890abcdef1234567890abcdef12345678";
    let commits = vec![
        commit(other, &["p0"], "other"),
        commit(full, &[other], "target"),
    ];
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: CommitId(full[..8].into()),
        fallback_scope: Some(LogScope::AllBranches),
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        None,
        21,
        34,
        false,
        Some(&log_page(commits, Some("next"))),
        Some(true),
        true,
        Some(&HistoryVisibleIndices::all(2)),
        &HistoryListPlan::new(false, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: None,
            select_commit: Some(CommitId(full.into())),
            scroll_to_list_ix: Some(1),
            load_more: false,
            clear_pending: true,
        }
    );
}

#[test]
fn pending_history_reveal_abbreviated_commit_waits_for_display_page_before_selecting() {
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: CommitId("abcdef01".into()),
        fallback_scope: Some(LogScope::AllBranches),
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        None,
        21,
        34,
        false,
        None,
        None,
        true,
        None,
        &HistoryListPlan::new(false, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: None,
            select_commit: None,
            scroll_to_list_ix: None,
            load_more: false,
            clear_pending: false,
        }
    );
}

#[test]
fn pending_history_reveal_abbreviated_commit_waits_for_matching_cache_before_selecting() {
    let full = "abcdef0123456789abcdef0123456789abcdef01";
    let commits = vec![commit(full, &["p0"], "target")];
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: CommitId(full[..8].into()),
        fallback_scope: Some(LogScope::AllBranches),
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        None,
        21,
        34,
        false,
        Some(&log_page(commits, None)),
        Some(false),
        false,
        Some(&HistoryVisibleIndices::all(1)),
        &HistoryListPlan::new(false, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: None,
            select_commit: None,
            scroll_to_list_ix: None,
            load_more: false,
            clear_pending: false,
        }
    );
}

#[test]
fn pending_history_reveal_uppercase_abbreviated_commit_scrolls_and_selects_full_id() {
    let full = "abcdef0123456789abcdef0123456789abcdef01";
    let other = "1234567890abcdef1234567890abcdef12345678";
    let commits = vec![
        commit(other, &["p0"], "other"),
        commit(full, &[other], "target"),
    ];
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: CommitId(full[..8].to_ascii_uppercase().into()),
        fallback_scope: Some(LogScope::AllBranches),
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        None,
        21,
        34,
        false,
        Some(&log_page(commits, None)),
        Some(false),
        true,
        Some(&HistoryVisibleIndices::all(2)),
        &HistoryListPlan::new(false, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: None,
            select_commit: Some(CommitId(full.into())),
            scroll_to_list_ix: Some(1),
            load_more: false,
            clear_pending: true,
        }
    );
}

#[test]
fn pending_history_reveal_ambiguous_abbreviated_commit_clears_without_selecting() {
    let first = "abcdef0123456789abcdef0123456789abcdef01";
    let second = "abcdef0123456789abcdef0123456789abcdef02";
    let commits = vec![
        commit(first, &["p0"], "first"),
        commit(second, &["p0"], "second"),
    ];
    let pending = PendingHistoryReveal {
        worktree_path: None,
        repo_id: RepoId(7),
        commit_id: CommitId(first[..8].into()),
        fallback_scope: Some(LogScope::AllBranches),
    };

    let decision = decide_pending_history_reveal(
        &pending,
        Some(RepoId(7)),
        Some(LogScope::CurrentBranch),
        None,
        21,
        34,
        false,
        Some(&log_page(commits, None)),
        Some(false),
        true,
        Some(&HistoryVisibleIndices::all(2)),
        &HistoryListPlan::new(false, Vec::new()),
        None,
    );

    assert_eq!(
        decision,
        PendingHistoryRevealDecision {
            set_scope: None,
            select_commit: None,
            scroll_to_list_ix: None,
            load_more: false,
            clear_pending: true,
        }
    );
}

#[test]
fn display_log_page_uses_retained_page_while_loading() {
    let mut repo = RepoState::new_opening(
        RepoId(9),
        RepoSpec {
            workdir: "/tmp/repo".into(),
        },
    );
    let page = Arc::new(log_page(vec![commit("a", &[], "a")], None));
    repo.log = Loadable::Loading;
    repo.history_state.log = Loadable::Loading;
    repo.history_state.retained_log_while_loading = Some(Arc::clone(&page));

    let display = HistoryView::display_log_page_for_repo(&repo)
        .expect("retained log should remain available while loading");
    assert!(Arc::ptr_eq(&display, &page));
}
