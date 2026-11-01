use super::*;

fn vb(name: &str) -> VirtualBranch {
    VirtualBranch::new(name)
}

fn virtual_branches() -> VirtualBranchesState {
    VirtualBranchesState::new("main")
}

#[test]
fn independent_branches_are_ordered_by_declaration() {
    let state = virtual_branches().with_branches(vec![vb("a"), vb("b"), vb("c")]);
    assert_eq!(state.application_order().unwrap().len(), 3);
    let names: Vec<_> = state
        .application_order()
        .unwrap()
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(names, ["a", "b", "c"]);
}

#[test]
fn a_stacked_branch_is_applied_after_its_parent() {
    let state = virtual_branches().with_branches(vec![
        vb("feature/ui").with_parent("feature/api"),
        vb("feature/api"),
    ]);
    let names: Vec<_> = state
        .application_order()
        .unwrap()
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(names, ["feature/api", "feature/ui"]);
}

#[test]
fn unapplied_branches_are_excluded_from_the_application_order() {
    let state = virtual_branches().with_branches(vec![
        vb("a"),
        vb("b").with_applied(BranchApplyState::Unapplied),
    ]);
    let names: Vec<_> = state
        .application_order()
        .unwrap()
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(names, ["a"]);
}

#[test]
fn a_stacked_branch_applies_after_its_unapplied_parent() {
    // The parent contributes nothing, but the child still has to be merged
    // last or its commits have no ancestor in the virtual-branches branch.
    let state = virtual_branches().with_branches(vec![
        vb("parent").with_applied(BranchApplyState::Unapplied),
        vb("child").with_parent("parent"),
    ]);
    let names: Vec<_> = state
        .application_order()
        .unwrap()
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(names, ["child"]);
}

#[test]
fn stacked_branches_group_independent_branches_separately() {
    let state = virtual_branches().with_branches(vec![
        vb("api"),
        vb("ui").with_parent("api"),
        vb("e2e").with_parent("ui"),
        vb("fix"),
    ]);
    let stacked_branches = state.stacked_branches().unwrap();
    assert_eq!(stacked_branches.len(), 2);

    let first = &stacked_branches[0];
    assert_eq!(first.base, "main");
    assert_eq!(
        first
            .branches
            .iter()
            .map(|m| (m.branch.name.as_str(), m.depth))
            .collect::<Vec<_>>(),
        [("api", 0), ("ui", 1), ("e2e", 2)]
    );
    assert_eq!(first.applied_count(), 3);

    let second = &stacked_branches[1];
    assert_eq!(second.base, "main");
    assert_eq!(second.branches[0].branch.name, "fix");
}

#[test]
fn order_controls_sibling_ordering() {
    let state = virtual_branches()
        .with_branches(vec![vb("second").with_order(1), vb("first").with_order(0)]);
    let stacked_branches = state.stacked_branches().unwrap();
    // Two roots are two stacked branches, and `order` is what decides which one
    // comes first: without it the declaration order ("second" first)
    // would win.
    assert_eq!(stacked_branches.len(), 2);
    assert_eq!(stacked_branches[0].branches[0].branch.name, "first");
    assert_eq!(stacked_branches[1].branches[0].branch.name, "second");
}

#[test]
fn a_cycle_is_rejected_by_validate() {
    let state =
        virtual_branches().with_branches(vec![vb("a").with_parent("b"), vb("b").with_parent("a")]);
    assert!(state.validate().is_err());
}

#[test]
fn a_branch_stacked_on_itself_is_rejected() {
    let state = virtual_branches().with_branches(vec![vb("a").with_parent("a")]);
    assert!(state.validate().is_err());
}

#[test]
fn a_branch_stacked_on_a_missing_branch_is_rejected() {
    let state = virtual_branches().with_branches(vec![vb("a").with_parent("ghost")]);
    assert!(state.validate().is_err());
}

#[test]
fn duplicate_branch_names_are_rejected() {
    let state = virtual_branches().with_branches(vec![vb("a"), vb("a")]);
    assert!(state.validate().is_err());
}

#[test]
fn the_virtual_branches_reserved_namespace_is_rejected() {
    let mut state = virtual_branches().with_branches(vec![vb("gitcomet/virtual")]);
    assert!(state.validate().is_err());
    assert!(state.push_branch(vb("gitcomet/other")).is_err());
}

#[test]
fn a_branch_named_like_the_target_is_rejected() {
    let state = virtual_branches().with_branches(vec![vb("main")]);
    assert!(state.validate().is_err());
}

#[test]
fn invalid_branch_names_are_rejected() {
    for name in [
        "",
        "-leading",
        "has space",
        "trailing/",
        "double//slash",
        "dots..here",
        "tilde~1",
        "caret^",
        "colon:name",
        "question?",
        "star*",
        "bracket[",
        "back\\slash",
        ".hidden",
    ] {
        let state = virtual_branches().with_branches(vec![vb(name)]);
        assert!(
            state.validate().is_err(),
            "expected '{name}' to be rejected"
        );
    }
}

#[test]
fn valid_branch_names_are_accepted() {
    for name in [
        "hotfix",
        "feature/api",
        "feature/api/v2",
        "fix-1.2",
        "release-2024",
    ] {
        let state = virtual_branches().with_branches(vec![vb(name)]);
        assert!(state.validate().is_ok(), "expected '{name}' to be valid");
    }
}

#[test]
fn a_refused_parent_change_leaves_the_state_untouched() {
    let mut state = virtual_branches().with_branches(vec![vb("a"), vb("b")]);
    state.set_parent("a", Some("b")).unwrap();
    let before = state.clone();
    assert!(state.set_parent("b", Some("a")).is_err());
    assert_eq!(state, before);
}

#[test]
fn a_branch_inserted_above_another_is_stacked_on_it() {
    let mut state = virtual_branches().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);
    state.insert_above(vb("ui2"), "ui").unwrap();
    assert_eq!(state.get("ui").unwrap().parent.as_deref(), Some("api"));
    assert_eq!(state.get("ui2").unwrap().parent.as_deref(), Some("ui"));
}

#[test]
fn a_branch_inserted_below_another_takes_its_place() {
    // The opposite of the above case, which is the whole point of having
    // two: `ui2` is pushed *down* to where `ui` was, `ui` now sits on it,
    // and the stacked branch is one deeper without anything above having moved.
    let mut state = virtual_branches().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);
    state.insert_below(vb("ui2"), "ui").unwrap();
    assert_eq!(
        state.get("ui2").unwrap().parent.as_deref(),
        Some("api"),
        "it inherits the anchor's parent"
    );
    assert_eq!(
        state.get("ui").unwrap().parent.as_deref(),
        Some("ui2"),
        "the anchor now sits on the branch that took its place"
    );
}

#[test]
fn inserting_below_a_branch_below_the_target_puts_the_new_one_on_the_target() {
    let mut state = virtual_branches().with_branches(vec![vb("api")]);
    state.insert_below(vb("ui"), "api").unwrap();
    assert_eq!(state.get("ui").unwrap().parent.as_deref(), None);
    assert_eq!(state.get("api").unwrap().parent.as_deref(), Some("ui"));
}

#[test]
fn inserting_below_leaves_the_branches_above_the_anchor_where_they_are() {
    let mut state = virtual_branches().with_branches(vec![
        vb("api"),
        vb("ui").with_parent("api"),
        vb("e2e").with_parent("ui"),
    ]);
    state.insert_below(vb("ui2"), "ui").unwrap();
    assert_eq!(
        state.get("e2e").unwrap().parent.as_deref(),
        Some("ui"),
        "what was stacked on the anchor is still stacked on it"
    );
    assert_eq!(state.application_order().unwrap().len(), 4);
}

#[test]
fn removing_a_branch_reparents_its_children() {
    let mut state = virtual_branches().with_branches(vec![
        vb("api"),
        vb("ui").with_parent("api"),
        vb("e2e").with_parent("ui"),
    ]);
    state.remove_branch("ui").unwrap();
    assert_eq!(state.get("e2e").unwrap().parent.as_deref(), Some("api"));
    assert!(state.get("ui").is_none());
}

#[test]
fn removing_an_unknown_branch_is_an_error() {
    let mut state = virtual_branches();
    assert!(state.remove_branch("ghost").is_err());
}

#[test]
fn reorder_swaps_sibling_positions() {
    let mut state = virtual_branches().with_branches(vec![
        vb("a").with_order(0),
        vb("b").with_order(1),
        vb("c").with_order(2),
    ]);
    state.reorder("a", "c").unwrap();
    assert_eq!(state.get("a").unwrap().order, Some(2));
    assert_eq!(state.get("c").unwrap().order, Some(0));
}

#[test]
fn reorder_across_stacked_branches_is_refused() {
    let mut state = virtual_branches().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);
    assert!(state.reorder("api", "ui").is_err());
}

#[test]
fn reorder_gives_both_branches_a_position_when_neither_has_one() {
    let mut state = virtual_branches().with_branches(vec![vb("a"), vb("b")]);
    state.reorder("a", "b").unwrap();
    let orders: Vec<_> = [state.get("a").unwrap().order, state.get("b").unwrap().order]
        .into_iter()
        .collect();
    assert!(orders.iter().all(Option::is_some));
    assert_ne!(orders[0], orders[1]);
}

#[test]
fn move_to_stacked_branch_places_a_branch_under_the_new_base() {
    let mut state = virtual_branches().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);
    state
        .move_to_stacked_branch("ui", "main", None, false)
        .unwrap();
    assert_eq!(state.get("ui").unwrap().parent, None);
}

#[test]
fn move_to_stacked_branch_below_a_sibling_stacked_branches_on_that_sibling() {
    let mut state = virtual_branches().with_branches(vec![vb("api"), vb("e2e").with_parent("api")]);
    state
        .move_to_stacked_branch("e2e", "main", Some("api"), true)
        .unwrap();
    assert_eq!(state.get("e2e").unwrap().parent.as_deref(), Some("api"));
}

#[test]
fn move_to_stacked_branch_above_a_sibling_takes_its_place() {
    let mut state = virtual_branches().with_branches(vec![
        vb("api"),
        vb("e2e").with_parent("api"),
        vb("other"),
    ]);
    state
        .move_to_stacked_branch("other", "main", Some("api"), false)
        .unwrap();
    assert_eq!(state.get("other").unwrap().parent, None);
    assert_eq!(state.get("e2e").unwrap().parent.as_deref(), Some("api"));
}

#[test]
fn a_move_that_would_create_a_cycle_is_refused() {
    let mut state = virtual_branches().with_branches(vec![
        vb("api"),
        vb("ui").with_parent("api"),
        vb("e2e").with_parent("ui"),
    ]);
    assert!(
        state
            .move_to_stacked_branch("api", "main", Some("e2e"), true)
            .is_err()
    );
    assert_eq!(state.get("api").unwrap().parent, None);
}

#[test]
fn a_move_relative_to_a_branch_that_is_not_there_is_refused() {
    // The drag named a branch that was removed in the meantime. Guessing the
    // stacked branch base instead would put the branch somewhere the user never
    // asked for and report the move as done.
    let mut state = virtual_branches().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);
    assert!(
        state
            .move_to_stacked_branch("api", "main", Some("gone"), false)
            .is_err()
    );
    assert_eq!(state.get("api").unwrap().parent, None);
}

#[test]
fn base_branch_follows_the_parent_or_the_target() {
    let stacked = vb("ui").with_parent("api");
    assert_eq!(stacked.base_branch("main"), "api");
    assert_eq!(vb("api").base_branch("main"), "main");
}

#[test]
fn set_applied_toggles_a_branch_without_disturbing_the_stacked_branch() {
    let mut state = virtual_branches().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);
    state
        .set_applied("ui", BranchApplyState::Unapplied)
        .unwrap();
    assert!(!state.get("ui").unwrap().is_applied());
    assert_eq!(state.get("ui").unwrap().parent.as_deref(), Some("api"));
    state.set_applied("ui", BranchApplyState::Applied).unwrap();
    assert!(state.get("ui").unwrap().is_applied());
}

#[test]
fn set_applied_rejects_an_unknown_branch() {
    let mut state = virtual_branches();
    assert!(
        state
            .set_applied("ghost", BranchApplyState::Applied)
            .is_err()
    );
}

#[test]
fn set_target_refuses_an_empty_or_conflicting_target() {
    let mut state = virtual_branches().with_branches(vec![vb("api")]);
    assert!(state.set_target("").is_err());
    assert!(state.set_target("api").is_err());
    state.set_target("develop").unwrap();
    assert_eq!(state.target, "develop");
}

#[test]
fn virtual_branches_state_round_trips_through_serde() {
    let state = virtual_branches().with_branches(vec![
        vb("api"),
        vb("ui").with_parent("api").with_order(1),
        vb("parked").with_applied(BranchApplyState::Unapplied),
    ]);
    let json = serde_json::to_string(&state).unwrap();
    let parsed: VirtualBranchesState = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, state);
}

#[test]
fn virtual_branches_state_deserializes_a_minimal_document() {
    // Older or hand-edited files may omit the optional fields entirely.
    let parsed: VirtualBranchesState = serde_json::from_str(r#"{"target":"main"}"#).unwrap();
    assert_eq!(parsed.target, "main");
    assert!(parsed.branches.is_empty());
    assert!(state_defaults_to_applied(&parsed));
}

fn state_defaults_to_applied(state: &VirtualBranchesState) -> bool {
    let mut probe = state.clone();
    probe.push_branch(vb("probe")).unwrap();
    probe.get("probe").unwrap().is_applied()
}

#[test]
fn assignment_index_resolves_only_existing_branches() {
    let mut index = AssignmentIndex::new();
    index.set(PathBuf::from("src/a.rs"), Some("api".into()));
    index.set(PathBuf::from("src/b.rs"), None);
    index.set(PathBuf::from("src/c.rs"), Some("deleted".into()));

    let state = virtual_branches().with_branches(vec![vb("api")]);
    assert_eq!(index.branch_of(Path::new("src/a.rs")), Some("api"));
    assert_eq!(index.branch_of(Path::new("src/b.rs")), None);
    assert_eq!(index.resolve(Path::new("src/a.rs"), &state), Some("api"));
    assert_eq!(index.resolve(Path::new("src/c.rs"), &state), None);
}

#[test]
fn assignment_paths_for_returns_paths_in_path_order() {
    let mut index = AssignmentIndex::new();
    index.set(PathBuf::from("z.rs"), Some("api".into()));
    index.set(PathBuf::from("a.rs"), Some("api".into()));
    index.set(PathBuf::from("m.rs"), Some("ui".into()));
    let paths: Vec<_> = index
        .paths_for("api")
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert_eq!(paths, ["a.rs", "z.rs"]);
}

#[test]
fn retain_drops_assignments_for_deleted_branches_and_unchanged_paths() {
    let mut index = AssignmentIndex::new();
    index.set(PathBuf::from("kept.rs"), Some("api".into()));
    index.set(PathBuf::from("gone.rs"), Some("deleted".into()));
    index.set(PathBuf::from("clean.rs"), Some("api".into()));

    let state = virtual_branches().with_branches(vec![vb("api")]);
    let changed = |path: &Path| path == Path::new("kept.rs");
    index.retain(&state, &changed, None);

    assert_eq!(index.len(), 1);
    assert_eq!(index.branch_of(Path::new("kept.rs")), Some("api"));
}

#[test]
fn a_separate_edit_produces_its_own_hunk() {
    let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\n";
    let working = "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\neleven\n";
    let spans = hunk_spans(base, working);
    assert_eq!(
        spans.len(),
        2,
        "two edits further apart than git's context, two hunks"
    );
    assert_eq!(spans[0].base_range, 1..2, "line two");
    assert_eq!(spans[1].base_range, 9..10, "line ten");
}

#[test]
fn an_insert_anchors_to_the_line_it_precedes() {
    let spans = hunk_spans("one\ntwo\n", "one\nnew\ntwo\n");
    assert_eq!(spans.len(), 1);
    assert!(
        spans[0].base_range.is_empty(),
        "an insertion replaces no base line"
    );
    assert_eq!(
        spans[0].base_range,
        1..1,
        "empty range at the line the new text sits in front of"
    );
    assert_eq!(spans[0].new_range, 1..2);
}

#[test]
fn editing_a_file_above_a_hunk_leaves_it_alone() {
    // This is the property the whole model rests on: a change made at the
    // top of a file must not move the assignment of a change further down.
    let base = "one\ntwo\nthree\n";
    let working = "one\ntwo\nTHREE\n";
    let before = hunk_spans(base, working)[0].fingerprint;

    let extended = "ZERO\none\ntwo\nTHREE\n";
    let after = hunk_spans(base, extended)[0].fingerprint;
    assert_eq!(
        before, after,
        "what a change is does not depend on what sits above it"
    );
}

#[test]
fn an_unchanged_hunk_keeps_its_assignment_across_a_reload() {
    let state = virtual_branches().with_branches(vec![vb("api")]);
    let base = "one\ntwo\nthree\n";
    let working = "ONE\ntwo\nthree\n";
    let fingerprint = hunk_spans(base, working)[0].fingerprint;

    let mut index = AssignmentIndex::new();
    index.set_hunk(PathBuf::from("a.rs"), fingerprint, Some("api".into()));
    index.retain(&state, &|_| true, Some(&|_| hunk_spans(base, working)));

    assert_eq!(
        index.file(Path::new("a.rs")).unwrap().hunks().next(),
        Some((fingerprint, "api")),
        "the assignment follows the hunk rather than being dropped"
    );
}

#[test]
fn a_change_that_no_longer_exists_loses_its_assignment() {
    let state = virtual_branches().with_branches(vec![vb("api")]);
    let fingerprint = hunk_spans("one\ntwo\n", "ONE\ntwo\n")[0].fingerprint;
    let mut index = AssignmentIndex::new();
    index.set_hunk(PathBuf::from("a.rs"), fingerprint, Some("api".into()));
    // The change was reverted, so the file has no hunk to attach it to.
    index.retain(&state, &|_| true, Some(&|_| Vec::new()));
    assert_eq!(
        index.file(Path::new("a.rs")).unwrap().hunks().count(),
        0,
        "an assignment for a change that is gone has nothing to act on"
    );
}

#[test]
fn a_fingerprint_survives_a_different_base() {
    // The property that makes hunk assignment work at all: the view diffs
    // against HEAD and the commit diffs against the branch's base, so the
    // same change has to carry the same identity through both.
    let working = "one\nTWO\nthree\n";
    let from_head = hunk_spans("one\ntwo\nthree\n", working)[0].fingerprint;
    let from_branch = hunk_spans("one\ntwo\nthree\nfour\n", working)[0].fingerprint;
    assert_eq!(
        from_head, from_branch,
        "what the change adds and removes does not depend on what is around it"
    );
}

#[test]
fn a_merged_hunk_still_reproduces_both_answers_to_the_whole_file() {
    // The merge folds context into a span's ranges, so the two degenerate
    // cases have to be re-checked on the merged shape: keeping everything
    // must still be the working tree byte for byte, and keeping nothing
    // must still be the base. If the folded context broke either, the
    // commit for an unsplit file would stop being a no-op.
    let base = "one\ntwo\nthree\nfour\n";
    let working = "ONE\ntwo\nTHREE\nfour\n";
    let spans = hunk_spans(base, working);
    assert_eq!(
        spans.len(),
        1,
        "one hunk, so this really is the merged shape"
    );

    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| true),
        working,
        "keeping every hunk reproduces the working tree"
    );
    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| false),
        base,
        "keeping none reproduces the base"
    );
}

#[test]
fn two_changes_that_add_and_remove_the_same_text_share_a_fingerprint() {
    // A deliberate consequence of keying on content rather than position:
    // the user cannot tell these apart in the file either, so picking one
    // would be a coin flip dressed up as precision. Pinned so that if the
    // scheme ever changes, this is a decision rather than a surprise.
    let base = "MOVE ME\nalpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\nMOVE ME\n";
    let working = "moved\nalpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\nmoved\n";
    let spans = hunk_spans(base, working);
    assert_eq!(spans.len(), 2, "the two edits are far apart");
    assert_eq!(
        spans[0].fingerprint, spans[1].fingerprint,
        "identical changes are the same fingerprint, and both follow the assignment"
    );
}

#[test]
fn two_different_changes_do_not_share_a_fingerprint() {
    // Far enough apart that git prints them as two hunks.
    let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    let working = "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n";
    let spans = hunk_spans(base, working);
    assert_eq!(spans.len(), 2);
    assert_ne!(spans[0].fingerprint, spans[1].fingerprint);
}

#[test]
fn changes_within_gits_context_are_one_hunk() {
    // Two edits three lines apart are a single `@@` to the user, so they have
    // to be a single span here too: an assignment is keyed by the hunk the
    // view showed, and a fingerprint matching neither span would assign the
    // change to nothing at all.
    let base = "one\ntwo\nthree\nfour\n";
    let working = "ONE\ntwo\nTHREE\nfour\n";
    let spans = hunk_spans(base, working);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].base_range, 0..3);
    assert_eq!(spans[0].new_range, 0..3);
}

#[test]
fn a_merged_hunk_is_fingerprinted_by_the_lines_it_changes_only() {
    // The view can only read the `+` lines out of a git hunk, so the context
    // a merge folds in must stay out of the hash — otherwise the same change
    // would hash differently depending on what was edited nearby.
    let base = "one\ntwo\nthree\nfour\n";
    let working = "ONE\ntwo\nTHREE\nfour\n";
    let spans = hunk_spans(base, working);
    assert_eq!(
        spans[0].fingerprint,
        HunkFingerprint::of(&["ONE\n", "THREE\n"], 2),
        "the hash covers what the hunk adds and removes, not the two lines between"
    );
}

#[test]
fn a_change_beyond_gits_context_stays_its_own_hunk() {
    // xdiff merges change groups while at most two contexts separate them;
    // this pins the boundary at 6 lines, the value `git diff` was observed
    // to use at its default context.
    let base: String = (1..=20).map(|n| format!("line{n}\n")).collect();
    let close = base
        .replacen("line1\n", "LINE1\n", 1)
        .replacen("line8\n", "LINE8\n", 1);
    let spans = hunk_spans(&base, &close);
    assert_eq!(spans.len(), 1, "6 unchanged lines between them is one hunk");

    let far = base
        .replacen("line1\n", "LINE1\n", 1)
        .replacen("line9\n", "LINE9\n", 1);
    let spans = hunk_spans(&base, &far);
    assert_eq!(
        spans.len(),
        2,
        "7 unchanged lines between them is two hunks"
    );
}

#[test]
fn assigning_a_hunk_splits_the_file_and_leaves_the_rest_unassigned() {
    let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    let working = "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n";
    let spans = hunk_spans(base, working);
    let mut index = AssignmentIndex::new();
    index.set(PathBuf::from("a.rs"), Some("api".into()));
    index.set_hunk(
        PathBuf::from("a.rs"),
        spans[0].fingerprint,
        Some("ui".into()),
    );

    let file = index.file(Path::new("a.rs")).unwrap();
    assert_eq!(file.branch(), None, "the file is split, not whole");
    let assigned: Vec<_> = file.hunks().collect();
    assert_eq!(
        assigned,
        [(spans[0].fingerprint, "ui")],
        "only the hunk that was named; the other is nobody's until it is named"
    );
}

#[test]
fn a_branch_commit_takes_only_its_own_hunks() {
    let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    let working = "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n";
    let spans = hunk_spans(base, working);

    let ui = synthesize_for_branch(base, working, &spans, &|fingerprint| {
        fingerprint == spans[1].fingerprint
    });
    assert_eq!(
        ui, "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
        "the hunk that is not this branch's is reverted to the base"
    );

    let api = synthesize_for_branch(base, working, &spans, &|_| true);
    assert_eq!(
        api, working,
        "keeping every hunk reproduces the working tree"
    );

    let neither = synthesize_for_branch(base, working, &spans, &|_| false);
    assert_eq!(neither, base, "keeping none reproduces the base");
}

#[test]
fn an_unsplit_file_commits_unchanged() {
    // The overwhelmingly common case must not be perturbed by any of this.
    let base = "one\ntwo\n";
    let working = "one\nTWO\n";
    let spans = hunk_spans(base, working);
    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| true),
        working
    );
}

#[test]
fn an_insertion_kept_for_one_branch_is_reverted_for_the_other() {
    let base = "one\ntwo\n";
    let working = "one\nnew\ntwo\n";
    let spans = hunk_spans(base, working);
    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| false),
        base,
        "an insertion that is not this branch's must not be left behind"
    );
    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| true),
        working
    );
}

#[test]
fn a_hunk_keeps_the_bases_line_endings() {
    // A CRLF file where one hunk belongs to this branch and another does
    // not. If the working tree's endings leaked into the branch's copy,
    // committing one hunk would rewrite every untouched line of the file —
    // and the diff would show the whole file as changed.
    let base = "a\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\nh\r\ni\r\nj\r\n";
    let working = "A\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\nh\r\ni\r\nJ\r\n";
    let spans = hunk_spans(base, working);
    assert_eq!(
        spans.len(),
        2,
        "further apart than git's context, so two hunks"
    );

    let one = synthesize_for_branch(base, working, &spans, &|f| f == spans[0].fingerprint);
    assert_eq!(one, "A\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\nh\r\ni\r\nj\r\n");
    assert_eq!(
        one.bytes().filter(|b| *b == b'\r').count(),
        10,
        "every line still ends CRLF, including the one taken from the working tree: {one:?}"
    );

    let other = synthesize_for_branch(base, working, &spans, &|_| false);
    assert_eq!(
        other, base,
        "and reverting a hunk gives the base byte for byte"
    );
}

#[test]
fn a_hunk_that_deletes_every_line_of_a_span_is_handled() {
    // A pure removal produces no new lines at all, so its new range is
    // empty. That is the one span shape where `new_lines` is zero, and it
    // is the one a file deletion or a removed block goes through.
    let base = "a\nb\nc\n";
    let working = "a\nc\n";
    let spans = hunk_spans(base, working);
    assert_eq!(spans.len(), 1);
    assert!(
        spans[0].new_range.is_empty(),
        "a deletion produces no new lines to put in a branch's copy"
    );

    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| true),
        working
    );
    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| false),
        base,
        "a deletion another branch owns must not be carried into this one's commit"
    );
}

#[test]
fn a_brand_new_file_splits_against_an_empty_base() {
    // A file the branch introduces has nothing in its base, so every line is
    // an addition and the "base range" is empty.
    let base = "";
    let working = "one\ntwo\n";
    let spans = hunk_spans(base, working);
    assert_eq!(spans.len(), 1);
    assert!(
        spans[0].base_range.is_empty(),
        "there was nothing to replace"
    );

    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| true),
        working
    );
    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| false),
        "",
        "a file another branch introduced must not appear in this one's commit"
    );
}

#[test]
fn a_file_with_no_trailing_newline_keeps_that() {
    // Git marks this with `\ No newline at end of file`, and a synthesized
    // version that added one back would show as a change to the last line.
    let base = "one\ntwo";
    let working = "ONE\ntwo";
    let spans = hunk_spans(base, working);
    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| true),
        working
    );
    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| false),
        base
    );
}

#[test]
fn a_fingerprint_distinguishes_lines_from_one_string() {
    // The reason each line's length is hashed before its bytes: these two
    // produce identical bytes but are different changes, and collapsing
    // them would move an unrelated hunk when the user assigned this one.
    let split = HunkFingerprint::of(&["X", "\nY"], 1);
    let whole = HunkFingerprint::of(&["X\nY"], 1);
    assert_ne!(split, whole);
}

#[test]
fn a_fingerprint_counts_how_much_it_removes_as_well_as_what_it_adds() {
    // Replacing one line with `X` is not the same change as replacing two,
    // even though both add exactly `X`. Keying on the added text alone
    // would have the first drag the second along.
    assert_ne!(
        HunkFingerprint::of(&["X\n"], 1),
        HunkFingerprint::of(&["X\n"], 2)
    );
    assert_ne!(
        HunkFingerprint::of(&["X\n"], 1),
        HunkFingerprint::of(&["X\n", "Y\n"], 1)
    );
}

/// `count` lines named `l1..`, each terminated.
fn numbered_lines(count: usize) -> String {
    (1..=count).map(|n| format!("l{n}\n")).collect()
}

/// The same, with Windows line endings.
fn crlf_lines(count: usize) -> String {
    (1..=count).map(|n| format!("l{n}\r\n")).collect()
}

/// Keeping every hunk must reproduce the working tree and keeping none must
/// reproduce the base, byte for byte. Checked on every awkward shape
/// because `synthesize_for_branch` walks ranges rather than lines, and an
/// off-by-one only shows up where a range is empty — an insertion or a
/// deletion — which is exactly where a plain diff never looks.
fn assert_round_trips(base: &str, working: &str, expected_hunks: usize) {
    let spans = hunk_spans(base, working);
    assert_eq!(
        spans.len(),
        expected_hunks,
        "{spans:#?}\n`git diff -U3` prints {expected_hunks} hunk(s) for this"
    );
    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| true),
        working,
        "keeping every hunk must reproduce the working tree"
    );
    assert_eq!(
        synthesize_for_branch(base, working, &spans, &|_| false),
        base,
        "keeping no hunk must reproduce the base"
    );
}

#[test]
fn an_insertion_at_the_end_of_the_file_is_one_hunk() {
    // An insertion has no base lines, which is the one range shape the walk
    // has to get right on its own.
    assert_round_trips("a\nb\nc\n", "a\nb\nc\nd\n", 1);
}

#[test]
fn deleting_the_first_line_of_the_file_is_one_hunk() {
    assert_round_trips("a\nb\nc\n", "b\nc\n", 1);
}

#[test]
fn a_file_the_user_emptied_is_one_hunk_that_removes_everything() {
    // The branch keeping the deletion commits an empty file; the branch not
    // keeping it commits the file as it was. Neither may end up with a
    // stray blank line where the content used to be.
    assert_round_trips("a\nb\n", "", 1);
}

#[test]
fn a_last_line_with_no_newline_is_still_a_line() {
    // No trailing newline means the last line is a line: adding one back
    // would put the whole file in the diff, and an assignment keyed on the
    // line count would match nothing.
    assert_round_trips("a\nb", "a\nB", 1);
    assert_round_trips("a\nb", "a\nb\n", 1);
}

#[test]
fn changes_on_neighbouring_lines_are_one_hunk() {
    // No unchanged line between them, so they are one run — and one thing to
    // assign, which is what the user sees.
    assert_round_trips(&numbered_lines(5), "l1\nL2\nL3\nl4\nl5\n", 1);
}

#[test]
fn a_crlf_file_is_split_without_rewriting_its_line_endings() {
    // The promise the docs make: line endings come from the base, so a
    // commit that moves one hunk does not rewrite every untouched line of a
    // Windows file. If they came from the working tree instead, the whole
    // file would show as changed.
    let base = crlf_lines(10);
    let working = base
        .replacen("l1\r\n", "API\r\n", 1)
        .replacen("l10\r\n", "L10\r\n", 1);
    let spans = hunk_spans(&base, &working);
    assert_eq!(spans.len(), 2, "`git diff -U3` prints two hunks here");

    let api = synthesize_for_branch(&base, &working, &spans, &|fingerprint| {
        fingerprint == spans[0].fingerprint
    });
    assert_eq!(
        api,
        "API\r\nl2\r\nl3\r\nl4\r\nl5\r\nl6\r\nl7\r\nl8\r\nl9\r\nl10\r\n"
    );
    assert_eq!(
        api.matches('\r').count(),
        10,
        "every line has to keep its carriage return"
    );

    let ui = synthesize_for_branch(&base, &working, &spans, &|fingerprint| {
        fingerprint == spans[1].fingerprint
    });
    assert_eq!(
        ui,
        "l1\r\nl2\r\nl3\r\nl4\r\nl5\r\nl6\r\nl7\r\nl8\r\nl9\r\nL10\r\n"
    );
    assert_eq!(ui.matches('\r').count(), 10);
}

#[test]
fn a_branch_that_keeps_a_deletion_commits_nothing_where_the_file_used_to_be() {
    // The other half of the deletion case, on a file with content after the
    // deleted run: the lines after it must survive untouched rather than
    // being swallowed by the walk.
    assert_round_trips("a\nb\nc\nd\n", "a\nd\n", 1);
}

#[test]
fn three_hunks_are_three_units_and_each_branch_keeps_only_its_own() {
    // Two branches were the most the tests had; three checks that a middle
    // hunk's fingerprint is not disturbed by the ones on either side of it,
    // which is what stops its assignment dragging the others along.
    let base = numbered_lines(30);
    let working = base
        .replacen("l1\n", "ONE\n", 1)
        .replacen("l10\n", "TEN\n", 1)
        .replacen("l25\n", "TWENTYFIVE\n", 1);
    let spans = hunk_spans(&base, &working);
    assert_eq!(spans.len(), 3, "`git diff -U3` prints three hunks here");

    let only = |index: usize| {
        synthesize_for_branch(&base, &working, &spans, &|fingerprint| {
            fingerprint == spans[index].fingerprint
        })
    };

    assert_eq!(
        only(0),
        working
            .replacen("TEN\n", "l10\n", 1)
            .replacen("TWENTYFIVE\n", "l25\n", 1)
    );
    assert_eq!(
        only(1),
        working
            .replacen("ONE\n", "l1\n", 1)
            .replacen("TWENTYFIVE\n", "l25\n", 1)
    );
    assert_eq!(
        only(2),
        working
            .replacen("ONE\n", "l1\n", 1)
            .replacen("TEN\n", "l10\n", 1)
    );
}

#[test]
fn removing_a_path_forgets_it_entirely() {
    // Forget, not "assign to nothing": a path with no entry is one Git no
    // longer reports as changed, and keeping a tombstone for it would show
    // the user a file they cannot find.
    let mut index = AssignmentIndex::new();
    index.set(PathBuf::from("a.rs"), Some("api".into()));
    index.set(PathBuf::from("b.rs"), Some("ui".into()));
    assert_eq!(index.len(), 2);

    index.remove(Path::new("a.rs"));
    assert_eq!(index.len(), 1);
    assert!(index.branch_of(Path::new("a.rs")).is_none());
    assert!(!index.is_empty());
    index.remove(Path::new("b.rs"));
    assert!(index.is_empty());
}

#[test]
fn pairs_keep_the_paths_in_order_regardless_of_input_order() {
    // The list is sorted by path so the UI does not sort on every frame and
    // so the file on disk stays diffable; the constructor has to be the
    // place that guarantees it, since a map built any other way would not.
    let index = AssignmentIndex::from_pairs(vec![
        (PathBuf::from("z.rs"), Some("api".to_string())),
        (PathBuf::from("a.rs"), Some("ui".to_string())),
        (PathBuf::from("m.rs"), None),
    ]);
    let paths: Vec<_> = index.paths().map(|(path, _)| path.to_path_buf()).collect();
    assert_eq!(
        paths,
        [
            PathBuf::from("a.rs"),
            PathBuf::from("m.rs"),
            PathBuf::from("z.rs")
        ]
    );
    assert_eq!(index.branch_of(Path::new("m.rs")), None);
}

#[test]
fn a_hunk_on_a_deleted_branch_is_dropped_but_its_siblings_stay() {
    // The file is still changed, so it survives — but a hunk naming a branch
    // that no longer exists is dead weight, and one branch leaving does not
    // unassign the others' hunks.
    let state = virtual_branches().with_branches(vec![vb("api"), vb("ui")]);
    let mut index = AssignmentIndex::new();
    let kept = HunkFingerprint::of(&["api\n"], 1);
    let orphan = HunkFingerprint::of(&["ui\n"], 1);
    index.set_hunk(PathBuf::from("a.rs"), kept, Some("api".into()));
    index.set_hunk(PathBuf::from("a.rs"), orphan, Some("gone".into()));

    index.retain(&state, &|_| true, None);

    let file = index
        .file(Path::new("a.rs"))
        .expect("the file is still changed");
    assert_eq!(file.hunk_branch(kept), Some("api"));
    assert!(
        file.hunk_branch(orphan).is_none(),
        "there is no branch left for this hunk to belong to"
    );
}

#[test]
fn a_split_file_survives_only_while_a_live_branch_has_part_of_it() {
    // A file whose every hunk belonged to branches that are gone is not
    // "changed and unassigned" any more — there is nothing left to commit,
    // so the row has to disappear with the branches.
    let state = virtual_branches().with_branches(vec![vb("api")]);
    let mut index = AssignmentIndex::new();
    index.set_hunk(
        PathBuf::from("gone.rs"),
        HunkFingerprint::of(&["x\n"], 1),
        Some("vanished".into()),
    );
    index.set_hunk(
        PathBuf::from("kept.rs"),
        HunkFingerprint::of(&["y\n"], 1),
        Some("api".into()),
    );

    index.retain(&state, &|_| true, None);

    assert!(index.file(Path::new("gone.rs")).is_none());
    assert!(index.file(Path::new("kept.rs")).is_some());
}

#[test]
fn a_virtual_branches_reports_which_of_its_branches_it_has() {
    // The create prompt asks this before it will use a name: creating a
    // branch whose name is spoken for fails on the backend with a Git error
    // the user can do nothing about.
    let state = virtual_branches().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);

    assert!(state.contains("api"));
    assert!(
        state.contains("ui"),
        "a stacked branch is still one of the virtual branches's own branches"
    );
    assert!(!state.contains("uii"));
    assert!(!state.contains(""));
}

#[test]
fn the_target_is_not_one_of_the_virtual_branches_branches() {
    // It is the base every independent branch is built on, and `validate`
    // refuses a virtual branch with the same name, so it is never something
    // `get` can return and `contains` must not pretend otherwise.
    let state = virtual_branches().with_branches(vec![vb("api")]);
    assert_eq!(state.target, "main");
    assert!(!state.contains("main"));
    assert!(state.get("main").is_none());
}

#[test]
fn a_branch_reports_whether_anything_is_stacked_on_it() {
    let state =
        virtual_branches().with_branches(vec![vb("api"), vb("ui").with_parent("api"), vb("docs")]);
    let stacked_branches = state.stacked_branches().unwrap();
    assert!(stacked_branches[0].contains("api"));
    assert!(
        stacked_branches[0].contains("ui"),
        "a child belongs to its parent's stacked branch"
    );
    assert!(
        !stacked_branches[0].contains("docs"),
        "an independent branch starts its own"
    );
    assert!(
        virtual_branches().stacked_branches().unwrap().is_empty(),
        "nothing to stacked branch"
    );
}

#[test]
fn moving_a_branch_changes_only_what_the_move_names() {
    // Used to render the shape a drag will produce without applying it, so
    // the preview and the result have to be computed the same way — and
    // *below* means stacked on the named branch, not pushed down by it.
    let state = virtual_branches().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);

    assert_eq!(
        state.parent_after_move("ui", Some("docs"), true),
        Some("docs".to_string()),
        "below stacked branches on the branch it is dropped under"
    );
    assert_eq!(
        state.parent_after_move("ui", Some("docs"), false),
        Some("main".to_string()),
        "above takes the target's place, so it inherits the target's parent"
    );
    assert_eq!(
        state.parent_after_move("ui", None, true),
        None,
        "no relative target means the branch becomes independent"
    );
    assert_eq!(
        state.parent_after_move("ui", Some("ui"), true),
        Some("api".to_string()),
        "a branch relative to itself does not move"
    );
    assert_eq!(state.parent_after_move("gone", Some("api"), true), None);
}

#[test]
fn the_target_moves_without_touching_the_branches_built_on_it() {
    // The names are the user's; only their base moves, and rebasing them is
    // the backend's job — this only has to say what the base now is.
    let mut state = virtual_branches().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);
    state
        .set_target("release/2.3")
        .expect("a target nobody has claimed");

    assert_eq!(state.target, "release/2.3");
    assert_eq!(
        state.get("api").unwrap().base_branch(&state.target),
        "release/2.3"
    );
    assert_eq!(state.get("ui").unwrap().base_branch(&state.target), "api");
}
