use super::*;
use crate::merge::{ConflictStyle, MergeStrategy, render_merge_plan};

fn row(a: bool, b: bool, c: bool, equal_ab: bool, equal_ac: bool, equal_bc: bool) -> AlignedRow {
    AlignedRow {
        a: a.then_some(0),
        b: b.then_some(0),
        c: c.then_some(0),
        equal_ab,
        equal_ac,
        equal_bc,
        ..AlignedRow::default()
    }
}

#[test]
fn block_fingerprint_preserves_existing_hash_values() {
    // A library change must not change IDs used to restore merge choices.
    assert_eq!(
        block_fingerprint(
            &[row(true, true, false, false, false, false)],
            MergeBlockClassification::BChangedCDeleted,
            &["alpha"],
            &["beta"],
            &[],
        ),
        0xd2d9_2d9e_9178_fb92
    );
}

#[test]
fn builds_plan_for_an_empty_side_against_a_large_base() {
    // The preflight admits an empty side because it shares nothing to
    // require. Planning must then stay linear: a trace-storing search at
    // depth 30_000 would allocate over a gigabyte before the plan exists.
    let base: String = (0..30_000).map(|line| format!("line {line}\n")).collect();
    let mut local = base.clone();
    local.push_str("local tail\n");

    let budget = InteractiveMergePlanBudget::default();
    assert!(interactive_merge_plan_is_practical(
        Some(&base),
        &local,
        "",
        budget
    ));

    let plan = try_build_interactive_merge_plan_with_optional_base(
        Some(&base),
        &local,
        "",
        &MergeOptions::default(),
        budget,
    )
    .expect("empty remote side should still produce a plan");
    assert_eq!(plan.source_lines(MergeSource::C).len(), 0);
    assert_eq!(plan.source_lines(MergeSource::B).len(), 30_001);
}

#[test]
fn classifies_every_three_way_merge_detail() {
    use MergeBlockClassification as Kind;

    let cases = [
        (row(false, false, false, false, false, false), Kind::Default),
        (row(true, true, true, true, true, true), Kind::NoChange),
        (row(true, true, true, false, true, false), Kind::BChanged),
        (row(true, true, true, true, false, false), Kind::CChanged),
        (row(true, true, true, false, false, false), Kind::BCChanged),
        (
            row(true, true, true, false, false, true),
            Kind::BCChangedAndEqual,
        ),
        (row(true, false, true, false, true, false), Kind::BDeleted),
        (row(true, true, false, true, false, false), Kind::CDeleted),
        (
            row(true, false, false, false, false, false),
            Kind::BCDeleted,
        ),
        (
            row(true, true, false, false, false, false),
            Kind::BChangedCDeleted,
        ),
        (
            row(true, false, true, false, false, false),
            Kind::CChangedBDeleted,
        ),
        (row(false, true, false, false, false, false), Kind::BAdded),
        (row(false, false, true, false, false, false), Kind::CAdded),
        (row(false, true, true, false, false, false), Kind::BCAdded),
        (
            row(false, true, true, false, false, true),
            Kind::BCAddedAndEqual,
        ),
    ];

    for (row, expected) in cases {
        assert_eq!(classify_three_way(&row).classification, expected);
    }
}

#[test]
fn manual_alignment_pairs_lines_the_automatic_diff_leaves_unmatched() {
    let (local, remote) = ("x\ny\n", "y\nx\n");
    let options = MergeOptions::default();

    let automatic = build_merge_plan_with_optional_base(None, local, remote, &options);
    assert_eq!(
        automatic
            .rows
            .iter()
            .map(|row| (row.a, row.b))
            .collect::<Vec<_>>(),
        vec![(Some(0), None), (Some(1), Some(0)), (None, Some(1))],
        "the automatic alignment anchors the shared line and strands the rest"
    );

    let mut alignments = ManualAlignmentList::new();
    assert!(alignments.insert(ManualAlignment::two_input(0..1, 0..1)));
    let pinned = build_merge_plan_with_alignments(None, local, remote, &options, &alignments);

    assert_eq!(
        pinned
            .rows
            .iter()
            .map(|row| (row.a, row.b))
            .collect::<Vec<_>>(),
        vec![(Some(0), Some(0)), (Some(1), Some(1))],
        "the pin forces the first lines onto one row, and the rest follows"
    );
    assert!(!pinned.rows[0].equal_ab);
}

#[test]
fn manual_alignment_survives_the_trim_pass() {
    // The automatic alignment keeps the first `x` and drops the second.
    // Pinning the second one makes the first the deleted occurrence, and
    // the trim pass must not quietly pull it back.
    let base = "a\nx\nb\nx\nc\n";
    let local = "a\nx\nb\nc\n";
    let options = MergeOptions::default();

    let automatic = build_merge_plan(base, local, base, &options);
    assert!(
        automatic
            .rows
            .iter()
            .any(|row| row.a == Some(1) && row.b == Some(1)),
        "without help the first occurrence is the one that survives"
    );

    let mut alignments = ManualAlignmentList::new();
    assert!(alignments.insert(ManualAlignment::new(3..4, 1..2, 3..4)));
    let pinned = build_merge_plan_with_alignments(Some(base), local, base, &options, &alignments);

    assert!(
        pinned
            .rows
            .iter()
            .any(|row| row.a == Some(3) && row.b == Some(1) && row.equal_ab),
        "the pinned occurrence shares a row with the local line"
    );
    assert!(
        pinned
            .rows
            .iter()
            .all(|row| row.a != Some(1) || row.b.is_none()),
        "the first occurrence is now the unmatched one"
    );
}

#[test]
fn a_three_way_pin_pulls_every_source_onto_the_pinned_row() {
    // Pinning base 0 against theirs 1 offsets the contributors by a line.
    // The trim pass has to land all three on one row: its cursors must skip
    // the rows the pin pushed into an earlier segment rather than stall on
    // them, which would leave the pinned lines scattered.
    let base = "base one\nbase two\nmiddle\nbase three\n";
    let local = "ours one\nours two\nmiddle\nours three\n";
    let remote = "theirs one\ntheirs two\nmiddle\ntheirs three\n";

    let mut alignments = ManualAlignmentList::new();
    assert!(alignments.insert(ManualAlignment::new(0..1, 0..1, 1..2)));
    let plan = build_merge_plan_with_alignments(
        Some(base),
        local,
        remote,
        &MergeOptions::default(),
        &alignments,
    );

    assert_eq!(
        plan.rows
            .iter()
            .map(|row| (row.a, row.b, row.c))
            .collect::<Vec<_>>(),
        vec![
            (None, None, Some(0)),
            (Some(0), Some(0), Some(1)),
            (Some(1), Some(1), None),
            (Some(2), Some(2), Some(2)),
            (Some(3), Some(3), Some(3)),
        ],
        "the pinned lines share a row and the displaced remote line stands alone"
    );
}

#[test]
fn a_pin_survives_the_contributor_alignment_pass() {
    // The contributor pass hoists B lines toward the head of the list, which
    // used to rip a pinned local line off its pinned row. The pass is kept
    // only when the pins survive it, so the pin wins either way.
    let base = "base one\nbase two\nmiddle\nbase three\n";
    let local = "ours one\nours two\nmiddle\nours three\n";
    let remote = "theirs one\ntheirs two\nmiddle\ntheirs three\n";

    let mut alignments = ManualAlignmentList::new();
    assert!(alignments.insert(ManualAlignment::new(0..1, 0..1, 1..2)));

    for align_contributors in [false, true] {
        let options = MergeOptions {
            align_contributors,
            ..MergeOptions::default()
        };
        let plan =
            build_merge_plan_with_alignments(Some(base), local, remote, &options, &alignments);
        assert!(
            plan.rows
                .iter()
                .any(|row| row.a == Some(0) && row.b == Some(0) && row.c == Some(1)),
            "pinned base/local/remote lines must share a row \
                 (align_contributors={align_contributors}): {:?}",
            plan.rows
                .iter()
                .map(|row| (row.a, row.b, row.c))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn an_empty_alignment_list_leaves_the_plan_untouched() {
    let (base, local, remote) = (
        "one\ntwo\nthree\n",
        "one\nlocal\nthree\n",
        "one\ntwo\nremote\n",
    );
    let options = MergeOptions {
        align_contributors: true,
        ..MergeOptions::default()
    };

    assert_eq!(
        build_merge_plan_with_alignments(
            Some(base),
            local,
            remote,
            &options,
            &ManualAlignmentList::new(),
        )
        .rows,
        build_merge_plan(base, local, remote, &options).rows
    );
}

#[test]
fn a_stale_alignment_entry_constrains_nothing() {
    let (base, local, remote) = ("one\ntwo\n", "one\nlocal\n", "one\nremote\n");
    let options = MergeOptions::default();

    let mut alignments = ManualAlignmentList::new();
    assert!(alignments.insert(ManualAlignment::new(40..41, 40..41, 40..41)));

    assert_eq!(
        build_merge_plan_with_alignments(Some(base), local, remote, &options, &alignments).rows,
        build_merge_plan(base, local, remote, &options).rows,
        "an entry recorded against text that has since changed is ignored"
    );
}

#[test]
fn the_alignment_list_orders_entries_and_rejects_conflicting_ones() {
    let mut alignments = ManualAlignmentList::new();

    assert!(alignments.insert(ManualAlignment::new(10..12, 10..11, 10..14)));
    assert!(
        alignments.insert(ManualAlignment::new(2..3, 2..3, 2..3)),
        "an entry entirely ahead of the first is accepted"
    );
    assert_eq!(
        alignments
            .iter()
            .map(|entry| entry.base.start)
            .collect::<Vec<_>>(),
        vec![2, 10],
        "entries stay in source order regardless of insertion order"
    );

    assert!(
        !alignments.insert(ManualAlignment::new(11..13, 20..21, 20..21)),
        "an entry overlapping an existing one in any source is rejected"
    );
    assert!(
        !alignments.insert(ManualAlignment::new(20..21, 0..1, 20..21)),
        "an entry that interleaves with an existing one is rejected"
    );
    assert!(
        !alignments.insert(ManualAlignment::new(5..5, 5..5, 5..5)),
        "an entry pinning nothing is rejected"
    );
    assert_eq!(alignments.len(), 2);

    assert!(alignments.remove_at(MergeSource::B, true, 10));
    assert_eq!(alignments.len(), 1);
    assert!(!alignments.remove_at(MergeSource::B, true, 10));
}

#[test]
fn an_empty_pinned_range_aligns_lines_against_nothing() {
    let (local, remote) = ("keep\ndrop\n", "keep\n");
    let options = MergeOptions::default();

    let mut alignments = ManualAlignmentList::new();
    assert!(alignments.insert(ManualAlignment::two_input(0..1, 0..0)));
    let plan = build_merge_plan_with_alignments(None, local, remote, &options, &alignments);

    assert_eq!(
        plan.rows
            .iter()
            .map(|row| (row.a, row.b))
            .collect::<Vec<_>>(),
        vec![(Some(0), None), (Some(1), Some(0))],
        "pinning `keep` against an empty range forces it off the shared row"
    );
}

#[test]
fn contributor_alignment_anchors_identical_changed_lines() {
    let base = "start\nbase one\nbase two\nend\n";
    let local = "start\nlocal\nshared\nend\n";
    let remote = "start\nshared\nremote\nend\n";

    let without = build_merge_plan(
        base,
        local,
        remote,
        &MergeOptions {
            align_contributors: false,
            ..MergeOptions::default()
        },
    );
    let with = build_merge_plan(
        base,
        local,
        remote,
        &MergeOptions {
            align_contributors: true,
            ..MergeOptions::default()
        },
    );

    let shared_aligned = |plan: &MergePlan| {
        plan.rows
            .iter()
            .any(|row| row.b == Some(2) && row.c == Some(1) && row.equal_bc)
    };
    assert!(!shared_aligned(&without));
    assert!(shared_aligned(&with));
}

#[test]
fn contributor_alignment_scales_for_large_mostly_equal_inputs() {
    const LINE_COUNT: usize = 20_001;
    let base_lines = (0..LINE_COUNT)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>();
    let mut local_lines = base_lines.clone();
    let mut remote_lines = base_lines.clone();
    local_lines[LINE_COUNT / 2] = "local change".to_owned();
    remote_lines[LINE_COUNT / 2] = "remote change".to_owned();

    let base = base_lines.join("\n");
    let local = local_lines.join("\n");
    let remote = remote_lines.join("\n");
    let plan = build_merge_plan(
        &base,
        &local,
        &remote,
        &MergeOptions {
            align_contributors: true,
            ..MergeOptions::default()
        },
    );

    assert_eq!(plan.rows.len(), LINE_COUNT);
    assert_eq!(plan.unresolved_count(), 1);
}

#[test]
fn contributor_alignment_cannot_discard_a_one_sided_edit() {
    let base = "a\na\nb\n";
    let local = "b\na\n";
    let remote = base;

    for options in [
        MergeOptions::default(),
        MergeOptions {
            align_contributors: true,
            ..MergeOptions::default()
        },
    ] {
        let plan = build_merge_plan(base, local, remote, &options);
        let result = render_merge_plan(&plan, &options);
        assert_eq!(result.output, local);
        assert_eq!(result.conflict_count, 0);
    }
}

#[test]
fn interactive_budget_rejects_large_unrelated_inputs_before_planning() {
    let base = (0..10_000)
        .map(|line| format!("base {line}\n"))
        .collect::<String>();
    let local = (0..10_000)
        .map(|line| format!("local {line}\n"))
        .collect::<String>();
    let remote = (0..10_000)
        .map(|line| format!("remote {line}\n"))
        .collect::<String>();

    assert!(
        try_build_interactive_merge_plan_with_optional_base(
            Some(&base),
            &local,
            &remote,
            &MergeOptions::default(),
            InteractiveMergePlanBudget::default(),
        )
        .is_none()
    );
}

#[test]
fn ordered_selections_toggle_render_order_and_replace_manual_content() {
    let options = MergeOptions::default();
    let mut plan = build_merge_plan("base\n", "local\n", "remote\n", &options);
    let conflict = plan.original_conflict_block_indices()[0];
    assert_eq!(plan.unresolved_block_indices(), &[conflict]);

    assert!(plan.toggle_source(conflict, MergeSource::B));
    assert!(plan.toggle_source(conflict, MergeSource::C));
    assert_eq!(render_merge_plan(&plan, &options).output, "local\nremote\n");

    assert!(plan.replace_selection(
        conflict,
        OrderedSelection::from_sources([MergeSource::C, MergeSource::B]),
    ));
    assert_eq!(render_merge_plan(&plan, &options).output, "remote\nlocal\n");

    assert!(plan.set_manual_content(conflict, "manual\n".to_owned()));
    assert_eq!(render_merge_plan(&plan, &options).output, "manual\n");
    assert!(plan.toggle_source(conflict, MergeSource::B));
    assert!(plan.blocks[conflict].manual_content.is_none());
    assert_eq!(render_merge_plan(&plan, &options).output, "local\n");

    assert!(plan.toggle_source(conflict, MergeSource::B));
    assert!(plan.blocks[conflict].selection.is_empty());
    assert_eq!(plan.unresolved_block_indices(), &[conflict]);
}

#[test]
fn structural_split_keeps_independent_selections_and_join_resets_once() {
    let options = MergeOptions::default();
    let mut plan = build_merge_plan(
        "base one\nbase two\n",
        "local one\nlocal two\n",
        "remote one\nremote two\n",
        &options,
    );
    let conflict = plan.original_conflict_block_indices()[0];
    plan.replace_selection(conflict, MergeSource::B.into());

    assert_eq!(
        plan.split_block_by_source_line_counts(conflict, &[[1, 1, 1], [1, 1, 1]]),
        Some(2),
    );
    assert_eq!(
        plan.blocks[conflict].selection.as_slice(),
        &[MergeSource::B]
    );
    assert_eq!(
        plan.blocks[conflict + 1].selection.as_slice(),
        &[MergeSource::B]
    );

    plan.toggle_source(conflict, MergeSource::C);
    assert_eq!(
        plan.blocks[conflict].selection.as_slice(),
        &[MergeSource::B, MergeSource::C]
    );
    assert_eq!(
        plan.blocks[conflict + 1].selection.as_slice(),
        &[MergeSource::B]
    );

    assert_eq!(plan.join_block_range(conflict, conflict + 1), Some(1));
    assert!(plan.blocks[conflict].selection.is_empty());
    assert_eq!(plan.unresolved_block_indices(), &[conflict]);
}

#[test]
fn optional_base_uses_true_two_input_markers_and_source_mapping() {
    let options = MergeOptions {
        style: ConflictStyle::Diff3,
        strategy: MergeStrategy::Normal,
        ..MergeOptions::default()
    };
    let mut plan = build_merge_plan_with_optional_base(None, "local\n", "remote\n", &options);
    assert!(!plan.has_base());
    assert_eq!(plan.local_source(), MergeSource::A);
    assert_eq!(plan.remote_source(), MergeSource::B);
    let conflict = plan.original_conflict_block_indices()[0];
    assert!(!plan.toggle_source(conflict, MergeSource::C));
    assert!(!plan.replace_selection(
        conflict,
        OrderedSelection::from_sources([MergeSource::A, MergeSource::C]),
    ));

    let output = render_merge_plan(&plan, &options).output;
    assert!(output.contains("<<<<<<<"));
    assert!(output.contains("======="));
    assert!(output.contains(">>>>>>>"));
    assert!(!output.contains("|||||||"));
}

#[test]
fn decisions_restore_by_stable_block_identity() {
    let options = MergeOptions::default();
    let mut previous =
        build_merge_plan("a\nbase\nz\n", "a\nlocal\nz\n", "a\nremote\nz\n", &options);
    let previous_conflict = previous.original_conflict_block_indices()[0];
    previous.replace_selection(
        previous_conflict,
        OrderedSelection::from_sources([MergeSource::C, MergeSource::B]),
    );

    let mut refreshed = build_merge_plan(
        "prefix\na\nbase\nz\n",
        "prefix\na\nlocal\nz\n",
        "prefix\na\nremote\nz\n",
        &options,
    );
    refreshed.restore_decisions_from(&previous);
    let refreshed_conflict = refreshed.original_conflict_block_indices()[0];
    assert_eq!(
        refreshed.blocks[refreshed_conflict].selection.as_slice(),
        &[MergeSource::C, MergeSource::B]
    );
}

#[test]
fn whitespace_metadata_marks_exact_and_equivalent_lines_separately() {
    let plan = build_merge_plan(
        "value = 1\n",
        "value=1\n",
        "value  =  1\n",
        &MergeOptions::default(),
    );
    let row = plan.rows.first().expect("one aligned row");
    assert!(!row.equal_ab);
    assert!(!row.equal_ac);
    assert!(!row.equal_bc);
    assert!(row.whitespace_equal_ab);
    assert!(row.whitespace_equal_ac);
    assert!(row.whitespace_equal_bc);
    assert!(plan.blocks.iter().any(|block| block.whitespace_conflict));
}

#[test]
fn navigation_sets_are_distinct_and_resolution_sensitive() {
    let mut plan = build_merge_plan(
        "one\ntwo\nthree\n",
        "ONE\ntwo\nlocal\n",
        "one\nTWO\nremote\n",
        &MergeOptions::default(),
    );
    let deltas = plan.delta_block_indices();
    let original = plan.original_conflict_block_indices();
    assert!(deltas.len() > original.len());
    assert_eq!(plan.unresolved_block_indices(), original.as_slice());

    let conflict = original[0];
    plan.replace_selection(conflict, MergeSource::B.into());
    assert_eq!(plan.original_conflict_block_indices(), original);
    assert!(plan.unresolved_block_indices().is_empty());
}
