use super::*;

#[test]
fn lane_branch_attribution_flows_down_from_the_branch_head() {
    // Only `feature` and `main` carry a ref; the commits below them inherit
    // the branch through their lane.
    let labels = lane_branch_labels(
        vec![
            commit("f2", &["f1"], "feature work"),
            commit("f1", &["base"], "feature start"),
            commit("m1", &["base"], "main work"),
            commit("base", &[], "base"),
        ],
        &[branch("feature", "f2"), branch("main", "m1")],
        &[],
        None,
    );

    assert_eq!(labels[0].as_deref(), Some("feature"));
    assert_eq!(labels[1].as_deref(), Some("feature"));
    assert_eq!(labels[2].as_deref(), Some("main"));
}

#[test]
fn a_feature_branch_parked_on_dev_does_not_claim_dev_s_history() {
    // The reported case: a freshly cut feature branch and `dev` point at the
    // very same commit, so nothing in the graph separates them. Attribution
    // has to prefer `dev`, or the whole history below is labelled with a
    // branch that has not added a single commit yet.
    let ref_items: Vec<HistoryRefListItem> = vec![
        HistoryRefListItem {
            text: HistoryTextVm::new("HEAD -> feat/thing".into()),
            kind: HistoryRefListItemKind::AttachedHead {
                branch: "feat/thing".to_string(),
            },
        },
        HistoryRefListItem {
            text: HistoryTextVm::new("dev".into()),
            kind: HistoryRefListItemKind::LocalBranch {
                name: "dev".to_string(),
            },
        },
    ];
    let tracked = FxHashSet::from_iter(["dev"]);
    assert_eq!(
        history_row_attribution_branch(&ref_items, &tracked),
        Some("dev")
    );

    // ...and it must still hold when the feature branch has been pushed, so
    // "is tracked" alone cannot separate them.
    let tracked = FxHashSet::from_iter(["dev", "feat/thing"]);
    assert_eq!(
        history_row_attribution_branch(&ref_items, &tracked),
        Some("dev")
    );
}

#[test]
fn attribution_prefers_a_pushed_branch_over_a_local_only_one() {
    // Neither is a conventional integration name, so the tie falls to the
    // branch whose history is actually shared.
    let ref_items: Vec<HistoryRefListItem> = vec![
        HistoryRefListItem {
            text: HistoryTextVm::new("scratch".into()),
            kind: HistoryRefListItemKind::LocalBranch {
                name: "scratch".to_string(),
            },
        },
        HistoryRefListItem {
            text: HistoryTextVm::new("release/24".into()),
            kind: HistoryRefListItemKind::LocalBranch {
                name: "release/24".to_string(),
            },
        },
    ];
    let tracked = FxHashSet::from_iter(["release/24"]);
    assert_eq!(
        history_row_attribution_branch(&ref_items, &tracked),
        Some("release/24")
    );

    // With nothing to separate them, the rendered order decides.
    let tracked = FxHashSet::default();
    assert_eq!(
        history_row_attribution_branch(&ref_items, &tracked),
        Some("scratch")
    );
}

#[test]
fn attribution_reads_origin_prefixed_remotes_as_their_branch() {
    let ref_items: Vec<HistoryRefListItem> = vec![
        HistoryRefListItem {
            text: HistoryTextVm::new("feat/thing".into()),
            kind: HistoryRefListItemKind::LocalBranch {
                name: "feat/thing".to_string(),
            },
        },
        HistoryRefListItem {
            text: HistoryTextVm::new("origin/dev".into()),
            kind: HistoryRefListItemKind::RemoteBranch {
                name: "origin/dev".to_string(),
                remote: "origin".to_string(),
                branch: "dev".to_string(),
            },
        },
    ];
    assert_eq!(
        history_row_attribution_branch(&ref_items, &FxHashSet::default()),
        Some("origin/dev")
    );
}

#[test]
fn dev_keeps_its_commits_however_the_feature_lane_is_drawn() {
    // The reported case: `feature` has diverged and its tip sits above dev's
    // in the log, so the lane that reaches the fork first is the feature's.
    // Containment has to win regardless -- every commit below the fork is
    // still in `dev`.
    let labels = lane_branch_labels(
        vec![
            commit("f2", &["f1"], "feature work"),
            commit("f1", &["base"], "feature start"),
            commit("d2", &["d1"], "dev work"),
            commit("d1", &["base"], "dev start"),
            commit("base", &["root"], "shared base"),
            commit("root", &[], "root"),
        ],
        &[branch("feature", "f2"), branch("dev", "d2")],
        &[],
        Some("feature"),
    );

    assert_eq!(labels[0].as_deref(), Some("feature"), "feature-only commit");
    assert_eq!(labels[1].as_deref(), Some("feature"), "feature-only commit");
    assert_eq!(labels[2].as_deref(), Some("dev"));
    assert_eq!(labels[3].as_deref(), Some("dev"));
    assert_eq!(labels[4].as_deref(), Some("dev"), "the fork point is dev's");
    assert_eq!(labels[5].as_deref(), Some("dev"), "and so is the root");
}

#[test]
fn dev_wins_even_when_its_tip_is_the_lower_row() {
    // The mirror ordering, which the previous "nearest branch head above"
    // rule got backwards.
    let labels = lane_branch_labels(
        vec![
            commit("d2", &["d1"], "dev work"),
            commit("d1", &["base"], "dev start"),
            commit("f2", &["f1"], "feature work"),
            commit("f1", &["base"], "feature start"),
            commit("base", &["root"], "shared base"),
            commit("root", &[], "root"),
        ],
        &[branch("feature", "f2"), branch("dev", "d2")],
        &[],
        Some("feature"),
    );

    assert_eq!(labels[2].as_deref(), Some("feature"));
    assert_eq!(labels[3].as_deref(), Some("feature"));
    assert_eq!(labels[4].as_deref(), Some("dev"), "the fork point is dev's");
    assert_eq!(labels[5].as_deref(), Some("dev"));
}

#[test]
fn shared_history_below_a_fork_is_attributed_to_the_base_branch() {
    // The reported shape: `feature` cut from `dev`, `dev` has moved on. Both
    // branches contain `base` and everything under it, and labelling those
    // rows with the checked-out feature branch reads as wrong -- they are
    // dev's history, which feature merely sits on top of.
    let labels = lane_branch_labels(
        vec![
            commit("f2", &["f1"], "feature work"),
            commit("f1", &["base"], "feature start"),
            commit("d2", &["d1"], "dev work"),
            commit("d1", &["base"], "dev start"),
            commit("base", &["root"], "shared base"),
            commit("root", &[], "root"),
        ],
        &[branch("feature", "f2"), branch("dev", "d2")],
        &[],
        Some("feature"),
    );

    assert_eq!(labels[0].as_deref(), Some("feature"));
    assert_eq!(labels[1].as_deref(), Some("feature"));
    assert_eq!(labels[2].as_deref(), Some("dev"));
    assert_eq!(labels[3].as_deref(), Some("dev"));
    // The fork point and everything below it belong to dev, not feature.
    assert_eq!(labels[4].as_deref(), Some("dev"));
    assert_eq!(labels[5].as_deref(), Some("dev"));
}

#[test]
fn lane_branch_attribution_reads_remote_branches_too() {
    let labels = lane_branch_labels(
        vec![
            commit("r2", &["r1"], "remote work"),
            commit("r1", &[], "remote start"),
        ],
        &[],
        &[remote_branch("origin", "topic", "r2")],
        None,
    );

    assert_eq!(labels[0].as_deref(), Some("origin/topic"));
    assert_eq!(labels[1].as_deref(), Some("origin/topic"));
}

#[test]
fn lane_branch_attribution_is_absent_without_any_branch_ref() {
    let labels = lane_branch_labels(
        vec![commit("c1", &["c0"], "one"), commit("c0", &[], "zero")],
        &[],
        &[],
        None,
    );

    assert!(labels.iter().all(Option::is_none));
}

#[test]
fn stash_tip_detection_requires_stash_like_message_and_multiple_parents() {
    assert!(is_probable_stash_tip(&commit(
        "s",
        &["p0", "p1"],
        "On main: quick stash"
    )));
    assert!(is_probable_stash_tip(&commit(
        "s",
        &["p0", "p1"],
        "WIP on main: quick stash"
    )));
    assert!(!is_probable_stash_tip(&commit(
        "c",
        &["p0"],
        "On main: normal commit"
    )));
    assert!(!is_probable_stash_tip(&commit(
        "c",
        &["p0", "p1"],
        "Regular summary"
    )));
}

#[test]
fn stash_summary_parser_extracts_tail_after_prefix() {
    assert_eq!(
        stash_summary_from_log_summary("On feature/x: savepoint"),
        Some("savepoint")
    );
    assert_eq!(
        stash_summary_from_log_summary("WIP on main: keep this"),
        Some("keep this")
    );
    assert_eq!(stash_summary_from_log_summary("no delimiter"), None);
}
