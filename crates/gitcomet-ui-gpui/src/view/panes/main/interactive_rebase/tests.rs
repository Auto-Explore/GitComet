use super::*;

fn entry(
    action: InteractiveRebaseAction,
    id: &str,
    new_message: Option<&str>,
) -> InteractiveRebaseEntry {
    InteractiveRebaseEntry {
        action,
        commit_id: id.to_string(),
        summary: format!("summary {id}"),
        message: format!("summary {id}"),
        new_message: new_message.map(|s| s.to_string()),
    }
}

#[test]
fn squash_folds_into_preceding_entry() {
    // Data index 0 is the oldest commit; a squash at a higher index folds
    // into the nearest non-drop entry below it.
    let entries = vec![
        entry(InteractiveRebaseAction::Pick, "a", None),
        entry(InteractiveRebaseAction::Squash, "b", None),
    ];
    assert!(entry_is_squash_target(&entries, 0));
    assert!(!entry_is_squash_target(&entries, 1));
}

#[test]
fn stranded_auto_reword_reverts_to_pick() {
    // A Reword with no user message and nothing squashing into it — the
    // state left behind when a squash is reordered away from its target
    // (finding #5) — reverts to Pick.
    let mut entries = vec![
        entry(InteractiveRebaseAction::Pick, "a", None),
        entry(InteractiveRebaseAction::Reword, "b", None),
    ];
    validate_squash_entries(&mut entries);
    assert_eq!(entries[1].action, InteractiveRebaseAction::Pick);
}

#[test]
fn deliberate_reword_is_preserved() {
    // A Reword the user actually typed a message for is never downgraded.
    let mut entries = vec![
        entry(InteractiveRebaseAction::Pick, "a", None),
        entry(InteractiveRebaseAction::Reword, "b", Some("new subject")),
    ];
    validate_squash_entries(&mut entries);
    assert_eq!(entries[1].action, InteractiveRebaseAction::Reword);
}

#[test]
fn reword_kept_while_still_a_squash_target() {
    // The auto-promoted Reword target stays Reword as long as a squash
    // continues to fold into it.
    let mut entries = vec![
        entry(InteractiveRebaseAction::Reword, "a", None),
        entry(InteractiveRebaseAction::Squash, "b", None),
    ];
    validate_squash_entries(&mut entries);
    assert_eq!(entries[0].action, InteractiveRebaseAction::Reword);
    assert_eq!(entries[1].action, InteractiveRebaseAction::Squash);
}

#[test]
fn squash_without_target_reverts_to_pick() {
    // A squash at the bottom (nothing below to fold into) becomes a pick,
    // and the now-stranded reword above it also reverts.
    let mut entries = vec![
        entry(InteractiveRebaseAction::Squash, "a", None),
        entry(InteractiveRebaseAction::Reword, "b", None),
    ];
    validate_squash_entries(&mut entries);
    assert_eq!(entries[0].action, InteractiveRebaseAction::Pick);
    assert_eq!(entries[1].action, InteractiveRebaseAction::Pick);
}

// Commit with an explicit summary, for auto-squash grouping tests.
// Order is oldest-first (index 0 = oldest), matching the entries vector.
fn sc(id: &str, summary: &str) -> InteractiveRebaseEntry {
    InteractiveRebaseEntry {
        action: InteractiveRebaseAction::Pick,
        commit_id: id.to_string(),
        summary: summary.to_string(),
        message: summary.to_string(),
        new_message: None,
    }
}

#[test]
fn autosquash_to_bottom_folds_into_oldest() {
    // oldest→newest: B "fix", C "wip", D "fix", F "fix"
    let original = vec![
        sc("B", "fix"),
        sc("C", "wip"),
        sc("D", "fix"),
        sc("F", "fix"),
    ];
    let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::ToBottom);
    // Only B (oldest "fix") and C survive.
    let ids: Vec<&str> = collapsed.iter().map(|e| e.commit_id.as_str()).collect();
    assert_eq!(ids, vec!["B", "C"]);
    let into_b = &folded["B"];
    assert_eq!(
        into_b
            .iter()
            .map(|e| e.commit_id.as_str())
            .collect::<Vec<_>>(),
        vec!["D", "F"]
    );
    assert!(
        into_b
            .iter()
            .all(|e| e.action == InteractiveRebaseAction::Fixup)
    );
}

#[test]
fn autosquash_fixup_prefix_folds_into_target() {
    // `fixup! add feature` groups with `add feature` even though the exact
    // subjects differ; the unprefixed commit survives (so its clean message
    // is kept) even under ToTop, which would otherwise keep the newest.
    let original = vec![
        sc("A", "add feature"),
        sc("B", "unrelated"),
        sc("C", "fixup! add feature"),
    ];
    let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::ToTop);
    let ids: Vec<&str> = collapsed.iter().map(|e| e.commit_id.as_str()).collect();
    assert_eq!(ids, vec!["A", "B"]);
    assert_eq!(
        folded["A"]
            .iter()
            .map(|e| e.commit_id.as_str())
            .collect::<Vec<_>>(),
        vec!["C"]
    );
    assert!(
        folded["A"]
            .iter()
            .all(|e| e.action == InteractiveRebaseAction::Fixup)
    );
}

#[test]
fn autosquash_neighbor_folds_adjacent_fixup_prefix() {
    let original = vec![
        sc("A", "add feature"),
        sc("B", "fixup! add feature"),
        sc("C", "unrelated"),
    ];
    let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::Neighbor);
    let ids: Vec<&str> = collapsed.iter().map(|e| e.commit_id.as_str()).collect();
    assert_eq!(ids, vec!["A", "C"]);
    assert_eq!(
        folded["A"]
            .iter()
            .map(|e| e.commit_id.as_str())
            .collect::<Vec<_>>(),
        vec!["B"]
    );
}

#[test]
fn autosquash_to_top_folds_into_newest() {
    let original = vec![
        sc("B", "fix"),
        sc("C", "wip"),
        sc("D", "fix"),
        sc("F", "fix"),
    ];
    let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::ToTop);
    // F (newest "fix") and C survive; F keeps its slot (after C).
    let ids: Vec<&str> = collapsed.iter().map(|e| e.commit_id.as_str()).collect();
    assert_eq!(ids, vec!["C", "F"]);
    assert_eq!(
        folded["F"]
            .iter()
            .map(|e| e.commit_id.as_str())
            .collect::<Vec<_>>(),
        vec!["B", "D"]
    );
}

#[test]
fn autosquash_neighbor_only_merges_adjacent() {
    // Two "fix" are adjacent (D,E); a separate "fix" (B) is not.
    let original = vec![
        sc("B", "fix"),
        sc("C", "wip"),
        sc("D", "fix"),
        sc("E", "fix"),
    ];
    let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::Neighbor);
    // B stays (not adjacent to another "fix"); D survives its run, E folds in.
    let ids: Vec<&str> = collapsed.iter().map(|e| e.commit_id.as_str()).collect();
    assert_eq!(ids, vec!["B", "C", "D"]);
    assert_eq!(
        folded["D"]
            .iter()
            .map(|e| e.commit_id.as_str())
            .collect::<Vec<_>>(),
        vec!["E"]
    );
    assert!(!folded.contains_key("B"));
}

#[test]
fn autosquash_no_duplicates_yields_empty_fold() {
    let original = vec![sc("A", "one"), sc("B", "two"), sc("C", "three")];
    let (_, folded) = compute_autosquash(&original, AutosquashMode::ToBottom);
    assert!(folded.is_empty());
}

#[test]
fn expand_folded_reinserts_fixups_after_survivor() {
    let original = vec![sc("B", "fix"), sc("C", "wip"), sc("D", "fix")];
    let (collapsed, folded) = compute_autosquash(&original, AutosquashMode::ToBottom);
    let expanded = expand_folded(&collapsed, &folded);
    let seq: Vec<(&str, InteractiveRebaseAction)> = expanded
        .iter()
        .map(|e| (e.commit_id.as_str(), e.action))
        .collect();
    assert_eq!(
        seq,
        vec![
            ("B", InteractiveRebaseAction::Pick),
            ("D", InteractiveRebaseAction::Fixup),
            ("C", InteractiveRebaseAction::Pick),
        ]
    );
}

#[test]
fn expand_folded_drops_fixups_with_dropped_survivor() {
    let original = vec![sc("B", "fix"), sc("D", "fix")];
    let (mut collapsed, folded) = compute_autosquash(&original, AutosquashMode::ToBottom);
    // Drop the survivor B; its folded commit D is dropped with it — but
    // emitted as a `drop` entry, because the executor revalidates that
    // the plan covers the live range exactly.
    collapsed[0].action = InteractiveRebaseAction::Drop;
    let expanded = expand_folded(&collapsed, &folded);
    let seq: Vec<(&str, InteractiveRebaseAction)> = expanded
        .iter()
        .map(|e| (e.commit_id.as_str(), e.action))
        .collect();
    assert_eq!(
        seq,
        vec![
            ("B", InteractiveRebaseAction::Drop),
            ("D", InteractiveRebaseAction::Drop),
        ]
    );
}

#[test]
fn dropping_a_squash_target_promotes_the_new_target() {
    let mut entries = vec![sc("X", "base"), sc("A", "target"), sc("B", "fold")];
    entries[1].action = InteractiveRebaseAction::Reword; // auto-promoted target
    entries[2].action = InteractiveRebaseAction::Squash;
    // The user drops A; the squash retargets to X, which must take over
    // the Reword promotion.
    entries[1].action = InteractiveRebaseAction::Drop;
    validate_squash_entries(&mut entries);
    assert_eq!(entries[0].action, InteractiveRebaseAction::Reword);
}

#[test]
fn undropping_a_target_repromotes_it_and_reverts_the_stand_in() {
    let mut entries = vec![sc("X", "base"), sc("A", "target"), sc("B", "fold")];
    entries[0].action = InteractiveRebaseAction::Reword; // promoted while A was dropped
    entries[1].action = InteractiveRebaseAction::Drop;
    entries[2].action = InteractiveRebaseAction::Squash;
    // The user picks A back: it becomes the run's target again and X's
    // auto-promotion (no typed message) reverts.
    entries[1].action = InteractiveRebaseAction::Pick;
    validate_squash_entries(&mut entries);
    assert_eq!(entries[1].action, InteractiveRebaseAction::Reword);
    assert_eq!(entries[0].action, InteractiveRebaseAction::Pick);
}

#[test]
fn squash_after_fixup_promotes_the_runs_fold_target() {
    // [Pick A, Fixup B, Squash C]: the run folds into A, so A (not the
    // fixup) must be auto-promoted to Reword and highlighted as target.
    let entries = vec![
        sc("A", "base"),
        {
            let mut e = sc("B", "fix");
            e.action = InteractiveRebaseAction::Fixup;
            e
        },
        {
            let mut e = sc("C", "more");
            e.action = InteractiveRebaseAction::Squash;
            e
        },
    ];
    assert_eq!(squash_target(&entries, 2), Some(0));
    assert!(entry_is_squash_target(&entries, 0));
    assert!(!entry_is_squash_target(&entries, 1));
}

#[test]
fn stale_reword_edit_cleared_when_squash_joins_the_run() {
    // A reworded target gains a new squash member: the saved combined
    // message no longer matches the run and must be dropped.
    let mut entries = vec![sc("A", "base"), sc("B", "second"), sc("C", "third")];
    entries[0].action = InteractiveRebaseAction::Reword;
    entries[0].new_message = Some("edited".to_string());
    entries[1].action = InteractiveRebaseAction::Squash;
    let before = reword_edit_signatures(&entries);

    entries[2].action = InteractiveRebaseAction::Squash;
    clear_stale_reword_edits(&before, &mut entries);
    assert_eq!(entries[0].new_message, None);
}

#[test]
fn stale_reword_edit_cleared_when_squash_leaves_the_run() {
    let mut entries = vec![sc("A", "base"), sc("B", "second")];
    entries[0].action = InteractiveRebaseAction::Reword;
    entries[0].new_message = Some("combined A+B".to_string());
    entries[1].action = InteractiveRebaseAction::Squash;
    let before = reword_edit_signatures(&entries);

    entries[1].action = InteractiveRebaseAction::Pick;
    clear_stale_reword_edits(&before, &mut entries);
    assert_eq!(entries[0].new_message, None);
}

#[test]
fn plain_reword_edit_survives_unrelated_changes() {
    let mut entries = vec![sc("A", "base"), sc("B", "second"), sc("C", "third")];
    entries[0].action = InteractiveRebaseAction::Reword;
    entries[0].new_message = Some("edited".to_string());
    let before = reword_edit_signatures(&entries);

    // A fixup contributes no message; the edit stays valid.
    entries[1].action = InteractiveRebaseAction::Fixup;
    clear_stale_reword_edits(&before, &mut entries);
    assert_eq!(entries[0].new_message.as_deref(), Some("edited"));

    // Dropping an entry outside the run does not touch it either.
    let before = reword_edit_signatures(&entries);
    entries[2].action = InteractiveRebaseAction::Drop;
    clear_stale_reword_edits(&before, &mut entries);
    assert_eq!(entries[0].new_message.as_deref(), Some("edited"));
}
