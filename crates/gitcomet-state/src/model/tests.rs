use super::*;
use gitcomet_core::domain::*;
use gitcomet_core::services::{BlameLine, SequencerState};
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::time::SystemTime;

fn summary_with_live_halves(
    mode: gitcomet_core::domain::SubmoduleDiffSummaryMode,
) -> SubmoduleDiffSummary {
    let change = |name: &str| {
        SubmoduleInnerChange::new(PathBuf::from(name), FileStatusKind::Modified)
            .with_line_counts(Some(1), Some(0))
    };
    SubmoduleDiffSummary {
        path: PathBuf::from("vendor/lib"),
        mode,
        status: None,
        checkout_available: true,
        commit_id: None,
        parent_commit_id: None,
        checked_out_head: None,
        ranges: vec![gitcomet_core::domain::SubmoduleDiffRange {
            kind: gitcomet_core::domain::SubmoduleDiffRangeKind::CommitHistory,
            from: Some(CommitId("aaaa".into())),
            to: Some(CommitId("bbbb".into())),
            unavailable_reason: None,
            changes: vec![change("range.rs")],
        }],
        live_staged: vec![change("staged.rs")],
        live_unstaged: vec![change("unstaged.rs")],
    }
}

/// The pane draws live rows only for a worktree summary, and prev/next walks
/// this list by index -- so it must describe exactly the rows on screen.
#[test]
fn only_a_worktree_summary_has_live_inline_entries() {
    use gitcomet_core::domain::SubmoduleDiffSummaryMode;

    let worktree = summary_with_live_halves(SubmoduleDiffSummaryMode::Worktree);
    let paths: Vec<_> = submodule_inline_diff_entries(&worktree)
        .into_iter()
        .map(|entry| entry.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            PathBuf::from("range.rs"),
            PathBuf::from("staged.rs"),
            PathBuf::from("unstaged.rs")
        ]
    );

    let history = summary_with_live_halves(SubmoduleDiffSummaryMode::CommitHistory);
    let paths: Vec<_> = submodule_inline_diff_entries(&history)
        .into_iter()
        .map(|entry| entry.path)
        .collect();
    assert_eq!(paths, vec![PathBuf::from("range.rs")]);
    assert!(
        submodule_inline_diff_target(&history, SubmoduleChangeSection::LiveStaged, 0).is_none(),
        "a commit-history summary has no live half to open"
    );
}

/// A row asks for the target alone; the list a click dispatches asks for the
/// whole entry. Both come from `submodule_change_at`, so they agree.
#[test]
fn a_rows_target_is_the_entry_the_click_selects() {
    use gitcomet_core::domain::SubmoduleDiffSummaryMode;

    let summary = summary_with_live_halves(SubmoduleDiffSummaryMode::Worktree);
    let entries = submodule_inline_diff_entries(&summary);
    for (index, section) in [
        SubmoduleChangeSection::Range(0),
        SubmoduleChangeSection::LiveStaged,
        SubmoduleChangeSection::LiveUnstaged,
    ]
    .into_iter()
    .enumerate()
    {
        let target = submodule_inline_diff_target(&summary, section, 0).expect("a target");
        assert_eq!(target, entries[index].target);
        assert_eq!(
            Some(&entries[index]),
            submodule_inline_diff_entry(&summary, section, 0).as_ref()
        );
    }
    assert!(
        submodule_inline_diff_target(&summary, SubmoduleChangeSection::Range(0), 1).is_none(),
        "past the end of a section there is no file"
    );
}

fn entry(name: &str) -> ViewHistoryEntry {
    ViewHistoryEntry {
        source: FileSource::Commit(CommitId(name.into())),
        path: PathBuf::from("src/lib.rs"),
        old_path: None,
    }
}

#[test]
fn nav_stack_reconcile_seeds_origin_pushes_and_updates_in_place() {
    let history_view = MainViewSnapshot {
        diff_target: None,
        content_preview: false,
        edit_mode: false,
        selected_commit: None,
        range_selection: None,
        worktree_selection: None,
    };
    let commit_view = MainViewSnapshot {
        diff_target: None,
        content_preview: false,
        edit_mode: false,
        selected_commit: Some(CommitId("aaa".into())),
        range_selection: None,
        worktree_selection: None,
    };
    let file_view = MainViewSnapshot {
        diff_target: Some(DiffTarget::commit(
            CommitId("aaa".into()),
            PathBuf::from("src/lib.rs"),
        )),
        edit_mode: false,
        content_preview: false,
        selected_commit: Some(CommitId("aaa".into())),
        range_selection: None,
        worktree_selection: None,
    };

    let mut h: NavStack<MainViewSnapshot> = NavStack::default();
    // Pre-change sync on the first navigation seeds the origin (history log).
    h.reconcile(history_view.clone(), false);
    // Selecting a commit then opening a file each push a distinct step.
    h.reconcile(commit_view.clone(), true);
    h.reconcile(file_view.clone(), true);
    assert_eq!(
        h.entries,
        vec![history_view.clone(), commit_view.clone(), file_view.clone()]
    );
    assert_eq!(h.cursor, 2);

    // Back steps one-by-one: file diff → commit details → history log.
    assert_eq!(h.step(ViewNavDir::Back), Some(commit_view.clone()));
    assert_eq!(h.step(ViewNavDir::Back), Some(history_view.clone()));
    assert!(!h.can_back());
    // Forward reopens them one-by-one.
    assert_eq!(h.step(ViewNavDir::Forward), Some(commit_view.clone()));
    assert_eq!(h.step(ViewNavDir::Forward), Some(file_view.clone()));

    // A non-push (background) change folds into the current entry without
    // adding a step or dropping forward history.
    let reloaded_file_view = MainViewSnapshot {
        content_preview: true,
        edit_mode: false,
        ..file_view.clone()
    };
    h.reconcile(reloaded_file_view.clone(), false);
    assert_eq!(h.entries.len(), 3, "in-place update must not add a step");
    assert_eq!(h.entries[2], reloaded_file_view);
    assert_eq!(h.cursor, 2);
}

#[test]
fn view_history_records_and_navigates() {
    let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
    assert!(!h.can_back());
    assert!(!h.can_forward());

    h.record(entry("a"));
    h.record(entry("b"));
    h.record(entry("c"));
    assert_eq!(h.cursor, 2);
    assert!(h.can_back());
    assert!(!h.can_forward());

    // Step back to b, then a.
    assert_eq!(h.step(ViewNavDir::Back), Some(entry("b")));
    assert_eq!(h.step(ViewNavDir::Back), Some(entry("a")));
    assert!(!h.can_back());
    // Clamped at the start.
    assert_eq!(h.step(ViewNavDir::Back), None);

    // Forward again to b.
    assert_eq!(h.step(ViewNavDir::Forward), Some(entry("b")));
    assert!(h.can_forward());
}

#[test]
fn view_history_new_open_truncates_forward() {
    let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
    h.record(entry("a"));
    h.record(entry("b"));
    h.record(entry("c"));
    // Go back to a, then open a new view: forward (b, c) is dropped.
    h.step(ViewNavDir::Back);
    h.step(ViewNavDir::Back);
    h.record(entry("d"));
    assert_eq!(
        h.entries,
        vec![entry("a"), entry("d")],
        "forward history past the cursor is truncated on a new open"
    );
    assert_eq!(h.cursor, 1);
    assert!(!h.can_forward());
}

#[test]
fn seek_or_record_moves_cursor_to_existing_entry_without_mutating() {
    let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
    h.record(entry("a"));
    h.record(entry("b"));
    h.record(entry("c"));
    // Realign onto an entry already present: only the cursor moves.
    h.seek_or_record(entry("a"));
    assert_eq!(h.cursor, 0);
    assert_eq!(h.entries, vec![entry("a"), entry("b"), entry("c")]);
    assert!(h.can_forward());
}

#[test]
fn seek_or_record_appends_when_entry_absent() {
    let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
    h.record(entry("a"));
    h.record(entry("b"));
    // An entry not in the stack is recorded as a fresh destination.
    h.seek_or_record(entry("z"));
    assert_eq!(h.entries, vec![entry("a"), entry("b"), entry("z")]);
    assert_eq!(h.cursor, 2);
}

#[test]
fn view_history_dedupes_repeat_of_current() {
    let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
    h.record(entry("a"));
    h.record(entry("a"));
    assert_eq!(h.entries, vec![entry("a")]);
    assert_eq!(h.cursor, 0);
}

#[test]
fn view_history_caps_length() {
    let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
    for i in 0..(NAV_HISTORY_CAP + 5) {
        h.record(entry(&format!("c{i}")));
    }
    assert_eq!(h.entries.len(), NAV_HISTORY_CAP);
    assert_eq!(h.cursor, NAV_HISTORY_CAP - 1);
    // Oldest entries were evicted; newest is current.
    assert_eq!(
        h.entries.last(),
        Some(&entry(&format!("c{}", NAV_HISTORY_CAP + 4)))
    );
}

#[test]
fn reconcile_leaves_history_intact_when_parked_mid_stack() {
    let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
    h.record(entry("a"));
    h.record(entry("b"));
    h.record(entry("c"));
    // Navigate back to the middle entry "b".
    assert_eq!(h.step(ViewNavDir::Back), Some(entry("b")));
    assert_eq!(h.cursor, 1);

    // A background (non-push) change to a different snapshot must not rewrite
    // the historical entry nor drop the forward entry "c".
    h.reconcile(entry("x"), false);
    assert_eq!(
        h.entries,
        vec![entry("a"), entry("b"), entry("c")],
        "background change while parked mid-stack must not mutate saved history"
    );
    assert_eq!(h.cursor, 1);
    assert!(h.can_forward());
    assert_eq!(h.step(ViewNavDir::Forward), Some(entry("c")));
}

#[test]
fn reconcile_folds_background_change_into_live_tail() {
    let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
    h.record(entry("a"));
    h.record(entry("b"));
    // At the live tail a background change still folds in place (no new step).
    h.reconcile(entry("b2"), false);
    assert_eq!(h.entries, vec![entry("a"), entry("b2")]);
    assert_eq!(h.cursor, 1);
}

#[test]
fn clear_resets_stack() {
    let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
    h.record(entry("a"));
    h.record(entry("b"));
    h.clear();
    assert!(h.entries.is_empty());
    assert_eq!(h.cursor, 0);
    assert!(!h.can_back());
    assert!(!h.can_forward());
}

#[test]
fn reconcile_fold_collapses_consecutive_duplicate() {
    let history_log = MainViewSnapshot {
        diff_target: None,
        content_preview: false,
        edit_mode: false,
        selected_commit: None,
        range_selection: None,
        worktree_selection: None,
    };
    let commit_view = MainViewSnapshot {
        diff_target: None,
        content_preview: false,
        edit_mode: false,
        selected_commit: Some(CommitId("aaa".into())),
        range_selection: None,
        worktree_selection: None,
    };
    let file_diff = MainViewSnapshot {
        diff_target: Some(DiffTarget::commit(
            CommitId("aaa".into()),
            PathBuf::from("src/lib.rs"),
        )),
        edit_mode: false,
        content_preview: false,
        selected_commit: Some(CommitId("aaa".into())),
        range_selection: None,
        worktree_selection: None,
    };

    let mut h: NavStack<MainViewSnapshot> = NavStack::default();
    h.reconcile(history_log.clone(), false);
    h.reconcile(commit_view.clone(), true);
    h.reconcile(file_diff.clone(), true);
    assert_eq!(h.cursor, 2);
    assert_eq!(h.entries.len(), 3);

    // Folding the cleared-diff state in-place makes it match the
    // previous entry (commit details), so the stack collapses back
    // to that entry instead of creating a consecutive duplicate.
    h.reconcile(commit_view.clone(), false);
    assert_eq!(
        h.entries.len(),
        2,
        "duplicate entries collapsed into original"
    );
    assert_eq!(h.cursor, 1);
    assert_eq!(h.entries[1], commit_view);
}

#[test]
fn reconcile_fold_no_collapse_when_not_adjacent_duplicate() {
    let view_a = MainViewSnapshot {
        diff_target: Some(DiffTarget::working_tree(
            PathBuf::from("a.txt"),
            DiffArea::Unstaged,
        )),
        edit_mode: false,
        content_preview: false,
        selected_commit: None,
        range_selection: None,
        worktree_selection: None,
    };
    let view_b = MainViewSnapshot {
        diff_target: Some(DiffTarget::working_tree(
            PathBuf::from("b.txt"),
            DiffArea::Unstaged,
        )),
        edit_mode: false,
        content_preview: false,
        selected_commit: None,
        range_selection: None,
        worktree_selection: None,
    };

    let mut h: NavStack<MainViewSnapshot> = NavStack::default();
    let empty = MainViewSnapshot {
        diff_target: None,
        content_preview: false,
        edit_mode: false,
        selected_commit: None,
        range_selection: None,
        worktree_selection: None,
    };
    h.reconcile(empty.clone(), false);
    h.reconcile(view_a.clone(), true);
    h.reconcile(view_b.clone(), true);
    assert_eq!(h.entries.len(), 3);

    // Folding into an adjacent entry that is *different* does not
    // collapse — it overwrites in place.
    let changed = MainViewSnapshot {
        content_preview: true,
        edit_mode: false,
        ..view_b.clone()
    };
    h.reconcile(changed.clone(), false);
    assert_eq!(h.entries.len(), 3, "no collapse when adjacent differs");
    assert_eq!(h.entries[2], changed);
    assert_eq!(h.cursor, 2);
}

#[test]
fn browsing_commit_reflects_file_browser_source() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    assert_eq!(repo.browsing_commit(), None);

    repo.file_browser.source = FileSource::Commit(CommitId("abc123".into()));
    assert_eq!(repo.browsing_commit(), Some(&CommitId("abc123".into())));

    repo.file_browser.source = FileSource::WorkingDirectory;
    assert_eq!(repo.browsing_commit(), None);
}

#[test]
fn app_state_clone_shares_heavy_repo_fields_via_arc() {
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let repo = &mut state.repos[0];
    repo.status = Loadable::Ready(Arc::new(RepoStatus::default()));
    repo.history_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: None,
    }));
    repo.history_state.file_history = Loadable::Ready(Arc::new(LogPage {
        commits: Vec::new(),
        next_cursor: None,
    }));
    repo.diff_state.blame = Loadable::Ready(Arc::new(vec![BlameLine {
        commit_id: "c1".into(),
        author: "a".into(),
        author_time_unix: None,
        summary: "s1".into(),
        body: None,
        line: "line".to_string(),
        prior_exists: true,
        source_path: None,
        prior_commit: None,
    }]));
    repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id: CommitId("c1".into()),
        message: "m".to_string(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: "t".to_string(),
        committed_at_unix: 0,
        parent_ids: Vec::new(),
        files: Vec::new(),
    }));
    repo.diff_state.diff = Loadable::Ready(Arc::new(Diff {
        target: DiffTarget::commit(CommitId("c1".into()), "a.txt".into()),
        lines: Vec::new(),
    }));

    let cloned = state.clone();

    let repo1 = &state.repos[0];
    let repo2 = &cloned.repos[0];

    let Loadable::Ready(status1) = &repo1.status else {
        panic!("expected status ready");
    };
    let Loadable::Ready(status2) = &repo2.status else {
        panic!("expected status ready");
    };
    assert!(Arc::ptr_eq(status1, status2));
    assert_eq!(Arc::strong_count(status1), 2);

    let Loadable::Ready(log1) = &repo1.history_state.log else {
        panic!("expected log ready");
    };
    let Loadable::Ready(log2) = &repo2.history_state.log else {
        panic!("expected log ready");
    };
    assert!(Arc::ptr_eq(log1, log2));
    assert_eq!(Arc::strong_count(log1), 2);

    let Loadable::Ready(diff1) = &repo1.diff_state.diff else {
        panic!("expected diff ready");
    };
    let Loadable::Ready(diff2) = &repo2.diff_state.diff else {
        panic!("expected diff ready");
    };
    assert!(Arc::ptr_eq(diff1, diff2));
    assert_eq!(Arc::strong_count(diff1), 2);
}

fn new_repo() -> RepoState {
    RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    )
}

fn file_status(path: &str, kind: FileStatusKind) -> FileStatus {
    FileStatus {
        path: PathBuf::from(path),
        kind,
        conflict: None,
    }
}

fn log_request(scope: LogScope, author: Option<&str>, cursor: Option<LogCursor>) -> PendingLogLoad {
    PendingLogLoad {
        scope,
        author: author.map(str::to_owned),
        limit: 20,
        cursor,
    }
}

fn test_log_request() -> PendingLogLoad {
    log_request(LogScope::FullReachable, None, None)
}

fn test_cursor(id: &str) -> LogCursor {
    LogCursor {
        last_seen: CommitId(id.into()),
        resume_from: None,
        resume_token: None,
    }
}

#[test]
fn request_primary_refresh_batch_marks_all_primary_loads_when_idle() {
    let mut loads = RepoLoadsInFlight::default();

    assert!(
        loads
            .request_primary_refresh_batch(test_log_request())
            .is_some()
    );
    assert!(loads.is_in_flight(RepoLoadsInFlight::HEAD_BRANCH));
    assert!(loads.is_in_flight(RepoLoadsInFlight::UPSTREAM_DIVERGENCE));
    assert!(loads.is_in_flight(RepoLoadsInFlight::REBASE_STATE));
    assert!(loads.is_in_flight(RepoLoadsInFlight::MERGE_COMMIT_MESSAGE));
    assert!(loads.is_in_flight(RepoLoadsInFlight::WORKTREE_STATUS));
    assert!(loads.is_in_flight(RepoLoadsInFlight::STAGED_STATUS));
    assert!(loads.is_in_flight(RepoLoadsInFlight::LOG));
}

#[test]
fn request_primary_refresh_batch_skips_when_any_load_is_already_in_flight() {
    let mut loads = RepoLoadsInFlight::default();
    assert!(loads.request(RepoLoadsInFlight::WORKTREE_STATUS));

    assert!(
        loads
            .request_primary_refresh_batch(test_log_request())
            .is_none()
    );
    assert!(!loads.is_in_flight(RepoLoadsInFlight::HEAD_BRANCH));
    assert!(loads.is_in_flight(RepoLoadsInFlight::WORKTREE_STATUS));
    assert!(!loads.is_in_flight(RepoLoadsInFlight::LOG));
}

/// A scope change supersedes the walk in flight rather than queueing behind
/// it: on a large repository that walk runs for tens of seconds, and the
/// effects layer cancels it as the replacement is dispatched.
#[test]
fn request_log_scope_change_starts_immediately() {
    let mut loads = RepoLoadsInFlight::default();
    let first = loads
        .request_log(log_request(LogScope::FullReachable, None, None))
        .expect("first request starts");

    assert!(
        loads
            .request_log(log_request(
                LogScope::AllBranches,
                None,
                Some(test_cursor("older")),
            ))
            .is_some()
    );
    let latest = loads
        .request_log(log_request(LogScope::NoMerges, None, None))
        .expect("a scope change starts at once");

    // The superseded walks' replies are no longer the active one.
    assert!(!loads.is_active_log_reply(first));
    assert!(loads.is_active_log_reply(latest));
    // Nothing is left queued: the newest request is the one running.
    assert_eq!(loads.finish_log(|_| {}), None);
}

#[test]
fn request_log_same_scope_refresh_replaces_stale_pending_pagination() {
    let mut loads = RepoLoadsInFlight::default();
    let cursor = test_cursor("page-1");

    assert!(
        loads
            .request_log(log_request(LogScope::MergesOnly, None, None))
            .is_some()
    );
    assert!(
        loads
            .request_log(log_request(
                LogScope::MergesOnly,
                None,
                Some(cursor.clone())
            ))
            .is_none()
    );
    assert!(
        loads
            .request_log(log_request(LogScope::MergesOnly, None, None))
            .is_none()
    );

    assert_eq!(
        loads.finish_log(|_| {}).map(|(_, next)| next),
        Some(log_request(LogScope::MergesOnly, None, None))
    );
}

#[test]
fn promoted_log_bookkeeping_matches_the_prepared_effect_request() {
    let mut loads = RepoLoadsInFlight::default();
    loads.request_log(test_log_request()).unwrap();
    assert!(loads.request_log(test_log_request()).is_none());
    let (seq, request) = loads.finish_log(|next| next.limit = 800).unwrap();
    assert_eq!(request.limit, 800);
    assert_eq!(loads.active_log.as_ref(), Some(&(seq, request)));
}

#[test]
fn request_log_author_change_starts_immediately_and_drops_pending_pagination() {
    let mut loads = RepoLoadsInFlight::default();

    assert!(
        loads
            .request_log(log_request(LogScope::MergesOnly, None, None))
            .is_some()
    );
    // A pagination request for the same scope+author is kept pending.
    assert!(
        loads
            .request_log(log_request(
                LogScope::MergesOnly,
                None,
                Some(test_cursor("page-1"))
            ))
            .is_none()
    );
    // Switching the author starts at once and drops that pagination, which
    // belonged to the previous filter.
    let latest = loads
        .request_log(log_request(LogScope::MergesOnly, Some("alice"), None))
        .expect("an author change starts at once");

    assert!(loads.is_active_log_reply(latest));
    assert_eq!(loads.finish_log(|_| {}), None);
}

/// Replies are matched against the walk that is actually running, and by
/// identity rather than by what it asked for: switching a filter away and
/// back leaves the second request looking exactly like the first.
#[test]
fn superseded_reply_is_not_the_active_one_even_when_it_asked_for_the_same_thing() {
    let mut loads = RepoLoadsInFlight::default();

    let first = loads
        .request_log(log_request(LogScope::NoMerges, None, None))
        .expect("first request starts");
    assert!(loads.is_active_log_reply(first));

    let alice = loads
        .request_log(log_request(LogScope::NoMerges, Some("alice"), None))
        .expect("an author change starts at once");
    assert!(!loads.is_active_log_reply(first));

    // Back to no filter: same scope, same author, same cursor as `first`.
    let again = loads
        .request_log(log_request(LogScope::NoMerges, None, None))
        .expect("clearing the filter starts at once");

    assert_ne!(first, again);
    assert!(!loads.is_active_log_reply(first));
    assert!(!loads.is_active_log_reply(alice));
    assert!(loads.is_active_log_reply(again));
}

#[test]
fn set_spec_refreshes_session_workdir_key() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo-a"),
        },
    );
    assert_eq!(repo.session_workdir_key().as_ref(), "/tmp/repo-a");

    repo.set_spec(RepoSpec {
        workdir: PathBuf::from("/tmp/repo-b"),
    });

    assert_eq!(repo.spec.workdir, PathBuf::from("/tmp/repo-b"));
    assert_eq!(repo.session_workdir_key().as_ref(), "/tmp/repo-b");
}

// --- Setter rev-bump tests ---

#[test]
fn set_status_bumps_status_rev() {
    let mut repo = new_repo();
    let before = repo.status_rev;
    repo.set_status(Loadable::Loading);
    assert_eq!(repo.status_rev, before + 1);
    repo.set_status(Loadable::Ready(Arc::new(RepoStatus::default())));
    assert_eq!(repo.status_rev, before + 2);
}

#[test]
fn split_status_setters_do_not_bump_legacy_status_rev() {
    let mut repo = new_repo();
    repo.set_status(Loadable::Loading);
    let status_rev = repo.status_rev;

    repo.set_worktree_status(Loadable::Ready(vec![file_status(
        "src/lib.rs",
        FileStatusKind::Modified,
    )]));
    assert_eq!(repo.status_rev, status_rev);

    repo.set_staged_status(Loadable::Ready(vec![file_status(
        "src/lib.rs",
        FileStatusKind::Added,
    )]));
    assert_eq!(repo.status_rev, status_rev);
}

#[test]
fn status_entry_for_path_prefers_split_lane_entries() {
    let mut repo = new_repo();
    repo.status = Loadable::Ready(Arc::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![file_status("legacy.rs", FileStatusKind::Modified)]),
        staged: std::sync::Arc::new(vec![file_status("legacy-stage.rs", FileStatusKind::Added)]),
    }));
    repo.status_rev = 1;
    repo.set_worktree_status(Loadable::Ready(vec![file_status(
        "split.rs",
        FileStatusKind::Deleted,
    )]));

    let entry = repo
        .status_entry_for_path(DiffArea::Unstaged, std::path::Path::new("split.rs"))
        .expect("split lane entry");
    assert_eq!(entry.kind, FileStatusKind::Deleted);
    assert!(
        repo.status_entry_for_path(DiffArea::Unstaged, std::path::Path::new("legacy.rs"))
            .is_none()
    );
}

#[test]
fn nothing_to_commit_depends_on_staged_entries_not_unstaged_work() {
    let mut repo = new_repo();
    repo.set_staged_status(Loadable::Ready(vec![]));
    repo.set_worktree_status(Loadable::Ready(vec![file_status(
        "unstaged.rs",
        FileStatusKind::Modified,
    )]));

    assert!(repo.nothing_to_commit());

    repo.set_staged_status(Loadable::Ready(vec![file_status(
        "staged.rs",
        FileStatusKind::Added,
    )]));
    assert!(!repo.nothing_to_commit());
}

#[test]
fn nothing_to_commit_keeps_pending_merge_commit_available() {
    let mut repo = new_repo();
    repo.set_staged_status(Loadable::Ready(vec![]));
    repo.merge_commit_message = Loadable::Ready(Some("Merge branch 'topic'".to_string()));

    assert!(!repo.nothing_to_commit());
}

#[test]
fn history_rewrite_busy_tracks_every_blocking_operation() {
    let repo = new_repo();
    assert!(!repo.history_rewrite_busy());

    let mut repo = new_repo();
    repo.local_actions_in_flight = 1;
    assert!(repo.history_rewrite_busy());

    for state in [
        SequencerState::CherryPick,
        SequencerState::RebaseOrApply,
        SequencerState::Revert,
    ] {
        let mut repo = new_repo();
        repo.sequencer_state = Loadable::Ready(state);
        assert!(repo.history_rewrite_busy(), "sequencer {state:?}");
    }
    let mut repo = new_repo();
    repo.sequencer_state = Loadable::Ready(SequencerState::None);
    assert!(!repo.history_rewrite_busy());

    let mut repo = new_repo();
    repo.rebase_in_progress = Loadable::Ready(true);
    assert!(repo.history_rewrite_busy());

    let mut repo = new_repo();
    repo.merge_commit_message = Loadable::Ready(Some("Merge branch 'topic'".to_string()));
    assert!(repo.history_rewrite_busy());
    repo.merge_commit_message = Loadable::Ready(None);
    assert!(!repo.history_rewrite_busy());
}

#[test]
fn status_cache_rev_changes_with_split_lane_revisions() {
    let mut repo = new_repo();
    let initial = repo.status_cache_rev();
    repo.set_worktree_status(Loadable::Loading);
    let after_worktree = repo.status_cache_rev();
    assert_ne!(after_worktree, initial);

    repo.set_staged_status(Loadable::Loading);
    assert_ne!(repo.status_cache_rev(), after_worktree);
}

#[test]
fn set_log_bumps_log_rev() {
    let mut repo = new_repo();
    let before = (repo.log_rev, repo.history_state.log_rev);
    repo.set_log(Loadable::Loading);
    assert_eq!(repo.log_rev, before.0 + 1);
    assert_eq!(repo.history_state.log_rev, before.1 + 1);
}

#[test]
fn retain_log_while_loading_keeps_ready_log_alive_until_next_log_state() {
    let mut repo = new_repo();
    let page = Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: None,
    });
    repo.set_log(Loadable::Ready(Arc::clone(&page)));

    repo.retain_log_while_loading();
    repo.set_log(Loadable::Loading);

    let retained = repo
        .history_state
        .retained_log_while_loading
        .as_ref()
        .expect("ready log should stay retained while loading");
    assert!(Arc::ptr_eq(retained, &page));

    repo.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: Vec::new(),
        next_cursor: None,
    })));

    assert!(repo.history_state.retained_log_while_loading.is_none());
}

#[test]
fn set_log_loading_more_bumps_log_rev() {
    let mut repo = new_repo();
    let before = (repo.log_rev, repo.history_state.log_rev);
    repo.set_log_loading_more(true);
    assert_eq!(repo.log_rev, before.0 + 1);
    assert_eq!(repo.history_state.log_rev, before.1 + 1);
    repo.set_log_loading_more(false);
    assert_eq!(repo.log_rev, before.0 + 2);
    assert_eq!(repo.history_state.log_rev, before.1 + 2);
}

#[test]
fn set_log_scope_bumps_log_rev() {
    let mut repo = new_repo();
    let before = (repo.log_rev, repo.history_state.log_rev);
    repo.set_log_scope(LogScope::AllBranches);
    assert_eq!(repo.log_rev, before.0 + 1);
    assert_eq!(repo.history_state.log_rev, before.1 + 1);
}

#[test]
fn set_selected_commit_bumps_selected_commit_rev() {
    let mut repo = new_repo();
    let before = repo.history_state.selected_commit_rev;
    repo.set_selected_commit(Some(CommitId("abc".into())));
    assert_eq!(repo.history_state.selected_commit_rev, before + 1);
    repo.set_selected_commit(None);
    assert_eq!(repo.history_state.selected_commit_rev, before + 2);
}

#[test]
fn set_commit_details_bumps_commit_details_rev() {
    let mut repo = new_repo();
    let before = repo.history_state.commit_details_rev;
    repo.set_commit_details(Loadable::Loading);
    assert_eq!(repo.history_state.commit_details_rev, before + 1);
}

#[test]
fn set_merge_commit_message_bumps_merge_message_rev() {
    let mut repo = new_repo();
    let before = repo.merge_message_rev;
    repo.set_merge_commit_message(Loadable::Ready(Some("merge".to_string())));
    assert_eq!(repo.merge_message_rev, before + 1);
}

#[test]
fn set_rebase_in_progress_bumps_merge_message_rev() {
    let mut repo = new_repo();
    let before = repo.merge_message_rev;
    repo.set_rebase_in_progress(Loadable::Ready(true));
    assert_eq!(repo.merge_message_rev, before + 1);
}

#[test]
fn merge_message_and_rebase_share_same_rev_counter() {
    let mut repo = new_repo();
    let before = repo.merge_message_rev;
    repo.set_merge_commit_message(Loadable::Ready(None));
    repo.set_rebase_in_progress(Loadable::Ready(false));
    assert_eq!(repo.merge_message_rev, before + 2);
}

#[test]
fn set_upstream_divergence_bumps_upstream_divergence_rev() {
    let mut repo = new_repo();
    let before = repo.upstream_divergence_rev;
    repo.set_upstream_divergence(Loadable::Loading);
    assert_eq!(repo.upstream_divergence_rev, before + 1);
}

#[test]
fn set_open_bumps_open_rev() {
    let mut repo = new_repo();
    let before = repo.open_rev;
    repo.set_open(Loadable::Ready(()));
    assert_eq!(repo.open_rev, before + 1);
}

#[test]
fn set_conflict_file_path_bumps_conflict_rev() {
    let mut repo = new_repo();
    let before = repo.conflict_state.conflict_rev;
    repo.set_conflict_file_path(Some(PathBuf::from("file.rs")));
    assert_eq!(repo.conflict_state.conflict_rev, before + 1);
}

#[test]
fn set_conflict_file_bumps_conflict_rev() {
    let mut repo = new_repo();
    let before = repo.conflict_state.conflict_rev;
    repo.set_conflict_file(Loadable::Loading);
    assert_eq!(repo.conflict_state.conflict_rev, before + 1);
}

#[test]
fn conflict_file_path_and_file_share_same_rev_counter() {
    let mut repo = new_repo();
    let before = repo.conflict_state.conflict_rev;
    repo.set_conflict_file_path(Some(PathBuf::from("a.rs")));
    repo.set_conflict_file(Loadable::Loading);
    assert_eq!(repo.conflict_state.conflict_rev, before + 2);
}

#[test]
fn set_conflict_file_load_mode_bumps_conflict_rev_only_on_change() {
    let mut repo = new_repo();
    let before = repo.conflict_state.conflict_rev;
    repo.set_conflict_file_load_mode(ConflictFileLoadMode::Full);
    assert_eq!(
        repo.conflict_state.conflict_file_load_mode,
        ConflictFileLoadMode::Full
    );
    assert_eq!(repo.conflict_state.conflict_rev, before + 1);

    repo.set_conflict_file_load_mode(ConflictFileLoadMode::Full);
    assert_eq!(repo.conflict_state.conflict_rev, before + 1);
}

#[test]
fn set_conflict_hide_resolved_bumps_conflict_rev_only_on_change() {
    let mut repo = new_repo();
    let before = repo.conflict_state.conflict_rev;
    repo.set_conflict_hide_resolved(true);
    assert!(repo.conflict_state.conflict_hide_resolved);
    assert_eq!(repo.conflict_state.conflict_rev, before + 1);
    repo.set_conflict_hide_resolved(true);
    assert_eq!(repo.conflict_state.conflict_rev, before + 1);
    repo.set_conflict_hide_resolved(false);
    assert!(!repo.conflict_state.conflict_hide_resolved);
    assert_eq!(repo.conflict_state.conflict_rev, before + 2);
}

#[test]
fn bump_diff_state_rev_increments() {
    let mut repo = new_repo();
    let before = repo.diff_state.diff_state_rev;
    repo.bump_diff_state_rev();
    assert_eq!(repo.diff_state.diff_state_rev, before + 1);
    repo.bump_diff_state_rev();
    assert_eq!(repo.diff_state.diff_state_rev, before + 2);
}

#[test]
fn set_diff_target_bumps_target_rev_only_on_change() {
    let mut repo = new_repo();
    let target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);

    repo.set_diff_target(Some(target.clone()));
    assert_eq!(repo.diff_state.diff_target, Some(target.clone()));
    assert_eq!(repo.diff_state.diff_target_rev, 1);

    repo.set_diff_target(Some(target));
    assert_eq!(repo.diff_state.diff_target_rev, 1);

    repo.set_diff_target(None);
    assert!(repo.diff_state.diff_target.is_none());
    assert_eq!(repo.diff_state.diff_target_rev, 2);
}

#[test]
fn bump_ops_rev_increments() {
    let mut repo = new_repo();
    let before = repo.ops_rev;
    repo.bump_ops_rev();
    assert_eq!(repo.ops_rev, before + 1);
    repo.bump_ops_rev();
    assert_eq!(repo.ops_rev, before + 2);
}

#[test]
fn git_operation_in_flight_counts_commands_that_can_write_the_worktree() {
    let mut repo = new_repo();
    assert!(!repo.git_operation_in_flight());
    for set in [
        |repo: &mut RepoState| repo.sequencer_actions_in_flight = 1,
        |repo: &mut RepoState| repo.worktree_pull_in_flight = 1,
        |repo: &mut RepoState| repo.commit_in_flight = 1,
    ] {
        let mut repo = new_repo();
        set(&mut repo);
        assert!(repo.git_operation_in_flight());
    }
    // A fetch, a push, staging, an editor save: none writes the checkout
    // behind the user's back.
    repo.pull_in_flight = 1;
    repo.push_in_flight = 1;
    repo.worktrees_in_flight = 1;
    repo.local_actions_in_flight = 1;
    assert!(!repo.git_operation_in_flight());
}

// --- Equality-guard tests: setters that skip rev bump on no-change ---

#[test]
fn set_head_branch_skips_rev_bump_when_unchanged() {
    let mut repo = new_repo();
    repo.set_head_branch(Loadable::Ready("main".to_string()));
    let rev_after_first = repo.head_branch_rev;
    repo.set_head_branch(Loadable::Ready("main".to_string()));
    assert_eq!(
        repo.head_branch_rev, rev_after_first,
        "rev should not bump for same value"
    );
}

#[test]
fn set_head_branch_bumps_rev_when_changed() {
    let mut repo = new_repo();
    repo.set_head_branch(Loadable::Ready("main".to_string()));
    let rev_after_first = repo.head_branch_rev;
    repo.set_head_branch(Loadable::Ready("develop".to_string()));
    assert_eq!(repo.head_branch_rev, rev_after_first + 1);
}

#[test]
fn branch_sidebar_cache_rev_falls_back_to_component_revisions() {
    let mut repo = new_repo();
    let initial = repo.branch_sidebar_cache_rev();
    repo.branches_rev = 1;
    assert_ne!(repo.branch_sidebar_cache_rev(), initial);
}

#[test]
fn branch_sidebar_cache_rev_bumps_only_for_relevant_changes() {
    let mut repo = new_repo();
    let initial = repo.branch_sidebar_cache_rev();

    repo.set_head_branch(Loadable::Ready("main".to_string()));
    let after_head = repo.branch_sidebar_cache_rev();
    assert_ne!(after_head, initial);

    repo.set_head_branch(Loadable::Ready("main".to_string()));
    assert_eq!(repo.branch_sidebar_cache_rev(), after_head);

    repo.set_worktrees(Loadable::Loading);
    assert_ne!(repo.branch_sidebar_cache_rev(), after_head);
}

#[test]
fn set_detached_head_commit_updates_only_on_change() {
    let mut repo = new_repo();
    let head = CommitId("abc123".into());
    repo.set_detached_head_commit(Some(head.clone()));
    assert_eq!(repo.detached_head_commit, Some(head.clone()));

    repo.set_detached_head_commit(Some(head.clone()));
    assert_eq!(repo.detached_head_commit, Some(head));

    repo.set_detached_head_commit(None);
    assert!(repo.detached_head_commit.is_none());
}

#[test]
fn set_branches_skips_rev_bump_when_unchanged() {
    let mut repo = new_repo();
    repo.set_branches(Loadable::NotLoaded);
    let rev = repo.branches_rev;
    repo.set_branches(Loadable::NotLoaded);
    assert_eq!(
        repo.branches_rev, rev,
        "rev should not bump for same Loadable variant"
    );
}

#[test]
fn set_tags_skips_rev_bump_when_unchanged() {
    let mut repo = new_repo();
    repo.set_tags(Loadable::NotLoaded);
    let rev = repo.tags_rev;
    repo.set_tags(Loadable::NotLoaded);
    assert_eq!(repo.tags_rev, rev);
}

#[test]
fn set_remotes_skips_rev_bump_when_unchanged() {
    let mut repo = new_repo();
    repo.set_remotes(Loadable::Loading);
    let rev = repo.remotes_rev;
    repo.set_remotes(Loadable::Loading);
    assert_eq!(repo.remotes_rev, rev);
}

#[test]
fn set_stashes_skips_rev_bump_when_unchanged() {
    let mut repo = new_repo();
    repo.set_stashes(Loadable::Loading);
    let rev = repo.stashes_rev;
    repo.set_stashes(Loadable::Loading);
    assert_eq!(repo.stashes_rev, rev);
}

/// The reflog panel keys its filtered-row cache on `reflog_rev`, so the rev
/// has to bump on every content change and stay put otherwise — a spurious
/// bump rebuilds the row list on every poll, a missing one shows stale rows.
#[test]
fn set_reflog_bumps_rev_only_when_the_entries_change() {
    let mut repo = new_repo();
    let before = repo.reflog_rev;

    repo.set_reflog(Loadable::Loading);
    assert_eq!(repo.reflog_rev, before + 1);
    repo.set_reflog(Loadable::Loading);
    assert_eq!(repo.reflog_rev, before + 1);

    let entry = ReflogEntry {
        index: 0,
        new_id: CommitId("abc".into()),
        message: "commit: initial".into(),
        time: None,
        selector: "HEAD@{0}".into(),
        author: "Jane Doe".into(),
    };
    repo.set_reflog(Loadable::Ready(vec![entry.clone()]));
    let ready_rev = repo.reflog_rev;
    assert_eq!(ready_rev, before + 2);

    // Same entries arriving again (a poll that found nothing new) must not
    // invalidate the panel's cache.
    repo.set_reflog(Loadable::Ready(vec![entry]));
    assert_eq!(repo.reflog_rev, ready_rev);
}

#[test]
fn set_ref_metadata_bumps_rev_when_changed_and_not_otherwise() {
    let mut repo = new_repo();
    let before = repo.ref_metadata_rev;
    repo.set_ref_metadata(Loadable::Loading);
    assert_eq!(repo.ref_metadata_rev, before + 1);
    repo.set_ref_metadata(Loadable::Loading);
    assert_eq!(
        repo.ref_metadata_rev,
        before + 1,
        "rev should not bump for an unchanged value"
    );
    repo.set_ref_metadata(Loadable::Ready(Arc::new(FxHashMap::default())));
    assert_eq!(repo.ref_metadata_rev, before + 2);
}

#[test]
fn set_branches_invalidates_cached_ref_metadata() {
    let mut repo = new_repo();
    repo.set_ref_metadata(Loadable::Ready(Arc::new(FxHashMap::from_iter([(
        "main".to_string(),
        RefMetadata {
            author: "Ada".to_string(),
            committed_at: 1,
            summary: "first".to_string(),
        },
    )]))));
    assert!(matches!(repo.ref_metadata, Loadable::Ready(_)));

    repo.set_branches(Loadable::Ready(vec![]));

    assert!(
        matches!(repo.ref_metadata, Loadable::NotLoaded),
        "metadata must not outlive the ref list it describes"
    );
}

#[test]
fn set_remote_branches_invalidates_cached_ref_metadata() {
    let mut repo = new_repo();
    repo.set_ref_metadata(Loadable::Ready(Arc::new(FxHashMap::default())));

    repo.set_remote_branches(Loadable::Ready(vec![]));

    assert!(matches!(repo.ref_metadata, Loadable::NotLoaded));
}

#[test]
fn unchanged_branches_do_not_invalidate_ref_metadata() {
    // The branch setters early-return when nothing changed, so a background
    // refresh that finds the same refs must leave the cache alone.
    let mut repo = new_repo();
    repo.set_branches(Loadable::Ready(vec![]));
    repo.set_ref_metadata(Loadable::Ready(Arc::new(FxHashMap::default())));
    let rev = repo.ref_metadata_rev;

    repo.set_branches(Loadable::Ready(vec![]));

    assert!(matches!(repo.ref_metadata, Loadable::Ready(_)));
    assert_eq!(repo.ref_metadata_rev, rev);
}

#[test]
fn set_worktrees_bumps_rev_when_changed() {
    let mut repo = new_repo();
    let before = repo.worktrees_rev;
    repo.set_worktrees(Loadable::Loading);
    assert_eq!(repo.worktrees_rev, before + 1);
    repo.set_worktrees(Loadable::Ready(vec![]));
    assert_eq!(repo.worktrees_rev, before + 2);
}

#[test]
fn set_submodules_skips_rev_bump_when_unchanged() {
    let mut repo = new_repo();
    repo.set_submodules(Loadable::Loading);
    let rev = repo.submodules_rev;
    repo.set_submodules(Loadable::Loading);
    assert_eq!(repo.submodules_rev, rev);
}

#[test]
fn set_status_skips_rev_bump_when_unchanged() {
    let mut repo = new_repo();
    repo.set_status(Loadable::Loading);
    let rev = repo.status_rev;
    repo.set_status(Loadable::Loading);
    assert_eq!(repo.status_rev, rev);
}

#[test]
fn set_log_skips_rev_bump_when_unchanged() {
    let mut repo = new_repo();
    repo.set_log(Loadable::Loading);
    let rev = (repo.log_rev, repo.history_state.log_rev);
    repo.set_log(Loadable::Loading);
    assert_eq!(repo.log_rev, rev.0);
    assert_eq!(repo.history_state.log_rev, rev.1);
}

#[test]
fn set_log_loading_more_skips_rev_bump_when_unchanged() {
    let mut repo = new_repo();
    repo.set_log_loading_more(true);
    let rev = (repo.log_rev, repo.history_state.log_rev);
    repo.set_log_loading_more(true);
    assert_eq!(repo.log_rev, rev.0);
    assert_eq!(repo.history_state.log_rev, rev.1);
}

#[test]
fn set_log_scope_skips_rev_bump_when_unchanged() {
    let mut repo = new_repo();
    repo.set_log_scope(LogScope::AllBranches);
    let rev = (repo.log_rev, repo.history_state.log_rev);
    repo.set_log_scope(LogScope::AllBranches);
    assert_eq!(repo.log_rev, rev.0);
    assert_eq!(repo.history_state.log_rev, rev.1);
}

// --- Isolation tests: one setter does not bump another's rev ---

#[test]
fn setters_only_bump_their_own_rev_counter() {
    let mut repo = new_repo();
    let snap = (
        repo.status_rev,
        repo.log_rev,
        repo.history_state.log_rev,
        repo.history_state.selected_commit_rev,
        repo.history_state.commit_details_rev,
        repo.merge_message_rev,
        repo.upstream_divergence_rev,
        repo.open_rev,
        repo.conflict_state.conflict_rev,
        repo.diff_state.diff_target_rev,
        repo.diff_state.diff_state_rev,
        repo.ops_rev,
    );

    repo.set_status(Loadable::Loading);
    assert_eq!(repo.status_rev, snap.0 + 1);
    assert_eq!(repo.log_rev, snap.1);
    assert_eq!(repo.history_state.log_rev, snap.2);
    assert_eq!(repo.history_state.selected_commit_rev, snap.3);
    assert_eq!(repo.history_state.commit_details_rev, snap.4);
    assert_eq!(repo.merge_message_rev, snap.5);
    assert_eq!(repo.upstream_divergence_rev, snap.6);
    assert_eq!(repo.open_rev, snap.7);
    assert_eq!(repo.conflict_state.conflict_rev, snap.8);
    assert_eq!(repo.diff_state.diff_target_rev, snap.9);
    assert_eq!(repo.diff_state.diff_state_rev, snap.10);
    assert_eq!(repo.ops_rev, snap.11);
}

#[test]
fn all_rev_counters_start_at_zero() {
    let repo = new_repo();
    assert_eq!(repo.status_rev, 0);
    assert_eq!(repo.log_rev, 0);
    assert_eq!(repo.history_state.log_rev, 0);
    assert_eq!(repo.history_state.selected_commit_rev, 0);
    assert_eq!(repo.history_state.commit_details_rev, 0);
    assert_eq!(repo.merge_message_rev, 0);
    assert_eq!(repo.upstream_divergence_rev, 0);
    assert_eq!(repo.open_rev, 0);
    assert_eq!(repo.conflict_state.conflict_rev, 0);
    assert_eq!(repo.diff_state.diff_target_rev, 0);
    assert_eq!(repo.diff_state.diff_state_rev, 0);
    assert_eq!(repo.ops_rev, 0);
    assert_eq!(repo.head_branch_rev, 0);
    assert_eq!(repo.branches_rev, 0);
    assert_eq!(repo.tags_rev, 0);
    assert_eq!(repo.remotes_rev, 0);
    assert_eq!(repo.remote_branches_rev, 0);
    assert_eq!(repo.stashes_rev, 0);
    assert_eq!(repo.worktrees_rev, 0);
    assert_eq!(repo.submodules_rev, 0);
    assert_eq!(repo.branch_sidebar_rev, 0);
}

#[test]
fn grouped_state_defaults_are_initialized() {
    let repo = new_repo();
    assert_eq!(repo.history_state.history_scope, LogScope::FullReachable);
    assert!(matches!(repo.history_state.log, Loadable::NotLoaded));
    assert!(matches!(
        repo.history_state.file_history,
        Loadable::NotLoaded
    ));
    assert!(matches!(repo.diff_state.blame, Loadable::NotLoaded));

    assert!(repo.diff_state.diff_target.is_none());
    assert!(matches!(repo.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(repo.diff_state.diff_file, Loadable::NotLoaded));
    assert!(matches!(
        repo.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));

    assert!(repo.conflict_state.conflict_file_path.is_none());
    assert!(matches!(
        repo.conflict_state.conflict_file,
        Loadable::NotLoaded
    ));
    assert!(repo.conflict_state.conflict_session.is_none());
    assert!(!repo.conflict_state.conflict_hide_resolved);
    assert!(repo.detached_head_commit.is_none());
    assert_eq!(repo.sidebar_data_request, SidebarDataRequest::default());
}

#[test]
fn loadable_ready_exposes_only_the_loaded_arm() {
    assert_eq!(Loadable::Ready(vec![1, 2, 3]).ready(), Some(&vec![1, 2, 3]));
    assert_eq!(Loadable::<Vec<u8>>::NotLoaded.ready(), None);
    assert_eq!(Loadable::<Vec<u8>>::Loading.ready(), None);
    assert_eq!(Loadable::<Vec<u8>>::Error("boom".into()).ready(), None);
}

/// Rows cache on these revs: an unchanged rescan must not bump them, and a
/// real change must.
#[test]
fn line_stats_revs_move_per_lane_only_when_that_lane_changes() {
    use gitcomet_core::domain::{LineStats, UncommittedLineStats};

    fn stats(staged: &[(&str, u32)], unstaged: &[(&str, u32)]) -> UncommittedLineStats {
        let build = |entries: &[(&str, u32)]| {
            entries
                .iter()
                .map(|(path, additions)| {
                    (
                        PathBuf::from(path),
                        LineStats::from((Some(*additions), Some(0))),
                    )
                })
                .collect()
        };
        UncommittedLineStats {
            staged: build(staged),
            unstaged: build(unstaged),
        }
    }

    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/line-stats"),
        },
    );
    repo.set_uncommitted_line_stats(Loadable::Ready(Arc::new(stats(&[("a", 1)], &[("b", 2)]))));
    let (staged_rev, unstaged_rev) = (repo.staged_line_stats_rev, repo.unstaged_line_stats_rev);

    repo.set_uncommitted_line_stats(Loadable::Ready(Arc::new(stats(&[("a", 1)], &[("b", 2)]))));
    assert_eq!(repo.staged_line_stats_rev, staged_rev, "unchanged rescan");
    assert_eq!(
        repo.unstaged_line_stats_rev, unstaged_rev,
        "unchanged rescan"
    );

    repo.set_uncommitted_line_stats(Loadable::Ready(Arc::new(stats(&[("a", 9)], &[("b", 2)]))));
    assert_ne!(
        repo.staged_line_stats_rev, staged_rev,
        "staged lane changed"
    );
    assert_eq!(
        repo.unstaged_line_stats_rev, unstaged_rev,
        "the untouched lane keeps its rev so its rows are not rebuilt"
    );
}
