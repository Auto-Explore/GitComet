use super::*;

fn default_opts() -> MergeOptions {
    MergeOptions::default()
}

fn opts_with_labels(ours: &str, base: &str, theirs: &str) -> MergeOptions {
    MergeOptions {
        labels: MergeLabels {
            ours: Some(ours.to_string()),
            base: Some(base.to_string()),
            theirs: Some(theirs.to_string()),
        },
        ..Default::default()
    }
}

fn opts_with_strategy(strategy: MergeStrategy) -> MergeOptions {
    MergeOptions {
        strategy,
        ..Default::default()
    }
}

fn opts_with_style(style: ConflictStyle) -> MergeOptions {
    MergeOptions {
        style,
        ..Default::default()
    }
}

fn render_merge_plan_uncached_for_test(plan: &MergePlan, options: &MergeOptions) -> MergeResult {
    let mut output = String::new();
    let mut conflict_count = 0usize;
    let final_block_is_manual = plan
        .blocks
        .last()
        .is_some_and(|block| block.manual_content.is_some());
    for block in &plan.blocks {
        if let Some(manual) = block.manual_content.as_deref() {
            output.push_str(manual);
            continue;
        }
        if !block.selection.is_empty() {
            for source in block.selection.iter() {
                for line in plan.block_source_lines(block, source) {
                    output.push_str(line);
                    output.push_str(plan.line_ending);
                }
            }
            continue;
        }

        let ours: Vec<Cow<'_, str>> = plan
            .block_source_lines(block, plan.local_source())
            .into_iter()
            .map(Cow::Borrowed)
            .collect();
        let theirs: Vec<Cow<'_, str>> = plan
            .block_source_lines(block, plan.remote_source())
            .into_iter()
            .map(Cow::Borrowed)
            .collect();
        let base = plan.block_ancestor_lines(block);
        emit_conflict_markers(
            &mut output,
            &ours,
            &theirs,
            &base,
            options,
            plan.line_ending,
        );
        conflict_count += 1;
    }
    if !final_block_is_manual {
        let base_text = plan.base.as_deref().unwrap_or_default();
        let base_lines = split_lines(base_text);
        let local_lines = split_lines(&plan.local);
        let remote_lines = split_lines(&plan.remote);
        apply_trailing_newline_decision(
            &mut output,
            base_text,
            &base_lines,
            &plan.local,
            &local_lines,
            &plan.remote,
            &remote_lines,
        );
    }
    MergeResult {
        output,
        conflict_count,
    }
}

#[test]
fn final_manual_content_controls_its_trailing_newline() {
    let options = MergeOptions::default();
    let mut plan = build_merge_plan("base", "local", "remote", &options);
    let conflict = plan.original_conflict_block_indices()[0];

    assert!(plan.set_manual_content(conflict, "local\n".to_owned()));
    assert_eq!(render_merge_plan(&plan, &options).output, "local\n");

    assert!(plan.set_manual_content(conflict, "local".to_owned()));
    assert_eq!(render_merge_plan(&plan, &options).output, "local");

    assert!(plan.set_manual_content(conflict, "local\r\n".to_owned()));
    assert_eq!(render_merge_plan(&plan, &options).output, "local\r\n");
}

#[test]
fn cached_line_renderer_matches_uncached_many_small_blocks() {
    let mut base = String::new();
    let mut local = String::new();
    let mut remote = String::new();
    for index in 0..256 {
        base.push_str(&format!("context-{index}\nbase-{index}\n"));
        local.push_str(&format!("context-{index}\nlocal-{index}\n"));
        remote.push_str(&format!("context-{index}\nremote-{index}\n"));
    }
    let options = MergeOptions {
        style: ConflictStyle::Diff3,
        ..MergeOptions::default()
    };
    let plan = build_merge_plan(&base, &local, &remote, &options);
    assert!(
        plan.original_conflict_block_indices().len() > 200,
        "fixture should exercise many independently rendered blocks",
    );
    assert_eq!(
        render_merge_plan(&plan, &options),
        render_merge_plan_uncached_for_test(&plan, &options),
    );
}

// -----------------------------------------------------------------------
// Identity and clean merge
// -----------------------------------------------------------------------

#[test]
fn merge_identity() {
    let text = "line1\nline2\nline3\n";
    let result = merge_file(text, text, text, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, text);
}

#[test]
fn merge_nonoverlapping_clean() {
    let base = "line1\nline2\nline3\n";
    let ours = "LINE1\nline2\nline3\n";
    let theirs = "line1\nline2\nLINE3\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "LINE1\nline2\nLINE3\n");
}

#[test]
fn merge_nonoverlapping_additions() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nbbb\nccc\nours_added\n";
    let theirs = "theirs_added\naaa\nbbb\nccc\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "theirs_added\naaa\nbbb\nccc\nours_added\n");
}

// -----------------------------------------------------------------------
// Conflict detection and marker format
// -----------------------------------------------------------------------

#[test]
fn merge_overlapping_conflict() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(!result.is_clean());
    assert_eq!(result.conflict_count, 1);
    assert!(result.output.contains("<<<<<<<"));
    assert!(result.output.contains("======="));
    assert!(result.output.contains(">>>>>>>"));
    assert!(result.output.contains("OURS"));
    assert!(result.output.contains("THEIRS"));
}

#[test]
fn merge_conflict_markers_with_labels() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let opts = opts_with_labels("local", "ancestor", "remote");
    let result = merge_file(base, ours, theirs, &opts);
    assert!(!result.is_clean());
    assert!(result.output.contains("<<<<<<< local"));
    assert!(result.output.contains(">>>>>>> remote"));
}

#[test]
fn merge_delete_vs_modify_conflict() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\n";
    let theirs = "aaa\nBBB\nccc\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(!result.is_clean());
}

// -----------------------------------------------------------------------
// Conflict resolution strategies
// -----------------------------------------------------------------------

#[test]
fn merge_ours_strategy() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let result = merge_file(base, ours, theirs, &opts_with_strategy(MergeStrategy::Ours));
    assert!(result.is_clean());
    assert_eq!(result.output, "aaa\nOURS\nccc\n");
}

#[test]
fn merge_theirs_strategy() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let result = merge_file(
        base,
        ours,
        theirs,
        &opts_with_strategy(MergeStrategy::Theirs),
    );
    assert!(result.is_clean());
    assert_eq!(result.output, "aaa\nTHEIRS\nccc\n");
}

#[test]
fn merge_union_strategy() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let result = merge_file(
        base,
        ours,
        theirs,
        &opts_with_strategy(MergeStrategy::Union),
    );
    assert!(result.is_clean());
    assert!(result.output.contains("OURS"));
    assert!(result.output.contains("THEIRS"));
    // Union: ours comes before theirs.
    let ours_pos = result.output.find("OURS").unwrap();
    let theirs_pos = result.output.find("THEIRS").unwrap();
    assert!(ours_pos < theirs_pos);
}

// -----------------------------------------------------------------------
// Diff3 and zdiff3 conflict styles
// -----------------------------------------------------------------------

#[test]
fn merge_diff3_output() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let result = merge_file(base, ours, theirs, &opts_with_style(ConflictStyle::Diff3));
    assert!(!result.is_clean());
    assert!(result.output.contains("|||||||"));
    assert!(result.output.contains("bbb"));
}

#[test]
fn zdiff3_extracts_common_prefix_suffix() {
    // Both sides share prefix "A" and suffix "E" around the conflict.
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let ours = "1\n2\n3\n4\nA\nB\nC\nD\nE\n7\n8\n9\n";
    let theirs = "1\n2\n3\n4\nA\nX\nC\nY\nE\n7\n8\n9\n";
    let result = merge_file(base, ours, theirs, &opts_with_style(ConflictStyle::Zdiff3));
    assert!(!result.is_clean());
    // "A" should appear before the conflict marker, not inside.
    let marker_start = result.output.find("<<<<<<<").unwrap();
    let a_positions: Vec<_> = result
        .output
        .match_indices("\nA\n")
        .map(|(pos, _)| pos)
        .collect();
    // At least one "A" occurrence should be before the conflict.
    assert!(
        a_positions.iter().any(|&pos| pos < marker_start),
        "Common prefix 'A' should be before conflict markers"
    );
}

#[test]
fn zdiff3_preserves_base_when_sides_share_added_prefix() {
    // Both sides prepend "A" (a line absent from base) before diverging.
    // The zdiff3 base section must still show the real ancestor line "X",
    // not be trimmed away by the sides' common-prefix length.
    let base = "pre\nX\npost\n";
    let ours = "pre\nA\nO\npost\n";
    let theirs = "pre\nA\nT\npost\n";
    let result = merge_file(base, ours, theirs, &opts_with_style(ConflictStyle::Zdiff3));
    assert!(!result.is_clean());
    let base_marker = result.output.find("|||||||").expect("base marker");
    let sep = result.output[base_marker..]
        .find("=======")
        .expect("separator")
        + base_marker;
    let base_section = &result.output[base_marker..sep];
    assert!(
        base_section.contains("\nX\n"),
        "zdiff3 base section should preserve ancestor line X, got: {base_section:?}"
    );
}

#[test]
fn zdiff3_shows_base_when_prefix_suffix_span_whole_base() {
    // Regression: conflict base region is exactly prefix_len + suffix_len
    // long. The old `len > prefix + suffix` guard emitted an empty base
    // section here; the ancestor lines must be shown.
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let ours = "1\n2\n3\n4\nA\nB\nC\nD\nE\n7\n8\n9\n";
    let theirs = "1\n2\n3\n4\nA\nX\nC\nY\nE\n7\n8\n9\n";
    let result = merge_file(base, ours, theirs, &opts_with_style(ConflictStyle::Zdiff3));
    assert!(!result.is_clean());
    let base_marker = result.output.find("|||||||").expect("base marker");
    let sep = result.output[base_marker..]
        .find("=======")
        .expect("separator")
        + base_marker;
    let base_section = &result.output[base_marker..sep];
    assert!(
        base_section.contains("\n5\n6\n"),
        "zdiff3 base section should show ancestor lines 5,6, got: {base_section:?}"
    );
}

// -----------------------------------------------------------------------
// Marker size
// -----------------------------------------------------------------------

#[test]
fn merge_marker_size_10() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let opts = MergeOptions {
        marker_size: 10,
        ..Default::default()
    };
    let result = merge_file(base, ours, theirs, &opts);
    assert!(result.output.contains("<<<<<<<<<<"));
    assert!(result.output.contains("=========="));
    assert!(result.output.contains(">>>>>>>>>>"));
}

// -----------------------------------------------------------------------
// Trailing newline / EOF edge cases
// -----------------------------------------------------------------------

#[test]
fn merge_preserves_trailing_newline() {
    let base = "aaa\nbbb\n";
    let ours = "aaa\nbbb\n";
    let theirs = "aaa\nBBB\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(result.is_clean());
    assert!(result.output.ends_with('\n'));
}

#[test]
fn merge_no_trailing_newline_when_inputs_lack_it() {
    let base = "aaa";
    let ours = "aaa";
    let theirs = "aaa";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(result.is_clean());
    assert!(!result.output.ends_with('\n'));
}

// -----------------------------------------------------------------------
// CRLF handling
// -----------------------------------------------------------------------

#[test]
fn merge_crlf_conflict_markers() {
    let base = "1\r\n2\r\n3\r\n";
    let ours = "1\r\n2\r\n4\r\n";
    let theirs = "1\r\n2\r\n5\r\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(!result.is_clean());
    // Conflict markers should use CRLF too.
    assert!(result.output.contains("<<<<<<<\r\n"));
    assert!(result.output.contains("=======\r\n"));
    assert!(result.output.contains(">>>>>>>\r\n"));
}

#[test]
fn merge_lf_conflict_markers() {
    let base = "1\n2\n3\n";
    let ours = "1\n2\n4\n";
    let theirs = "1\n2\n5\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(!result.is_clean());
    assert!(result.output.contains("<<<<<<<\n"));
    assert!(result.output.contains("=======\n"));
    assert!(result.output.contains(">>>>>>>\n"));
    // Ensure no CRLF.
    assert!(!result.output.contains("\r\n"));
}

// -----------------------------------------------------------------------
// Multiple conflicts
// -----------------------------------------------------------------------

#[test]
fn merge_multiple_conflicts() {
    let base = "a\nb\nc\nd\ne\n";
    let ours = "A\nb\nC\nd\ne\n";
    let theirs = "X\nb\nY\nd\ne\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert_eq!(result.conflict_count, 2);
}

// -----------------------------------------------------------------------
// Identical changes
// -----------------------------------------------------------------------

#[test]
fn merge_identical_changes_are_clean() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nXXX\nccc\n";
    let theirs = "aaa\nXXX\nccc\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "aaa\nXXX\nccc\n");
}

// -----------------------------------------------------------------------
// Empty inputs
// -----------------------------------------------------------------------

#[test]
fn merge_all_empty() {
    let result = merge_file("", "", "", &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "");
}

#[test]
fn merge_base_empty_both_add_same() {
    let result = merge_file("", "added\n", "added\n", &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "added\n");
}

#[test]
fn merge_base_empty_both_add_different() {
    let result = merge_file("", "ours\n", "theirs\n", &default_opts());
    assert!(!result.is_clean());
}

// -----------------------------------------------------------------------
// Only one side changes
// -----------------------------------------------------------------------

#[test]
fn merge_only_ours_changes() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let result = merge_file(base, ours, base, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "aaa\nOURS\nccc\n");
}

#[test]
fn merge_only_theirs_changes() {
    let base = "aaa\nbbb\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let result = merge_file(base, base, theirs, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "aaa\nTHEIRS\nccc\n");
}

#[test]
fn merge_identical_changes_both_sides_resolves_cleanly() {
    // When ours and theirs make the exact same change, the hunk-level
    // short-circuit avoids reconstructing the theirs side entirely.
    let base = "first\nsecond\nthird\n";
    let both = "first\nreplaced\nthird\n";
    let result = merge_file(base, both, both, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "first\nreplaced\nthird\n");
}

// -----------------------------------------------------------------------
// Three-way alignment (aligned row space)
// -----------------------------------------------------------------------

fn run_lens(runs: &[AlignedRun]) -> (usize, usize, usize) {
    runs.iter().fold((0, 0, 0), |acc, run| {
        (
            acc.0 + run.base.len(),
            acc.1 + run.ours.len(),
            acc.2 + run.theirs.len(),
        )
    })
}

fn assert_partitions(runs: &[AlignedRun], base: &str, ours: &str, theirs: &str) {
    let (b, o, t) = run_lens(runs);
    assert_eq!(b, split_lines(base).len(), "base lines partitioned");
    assert_eq!(o, split_lines(ours).len(), "ours lines partitioned");
    assert_eq!(t, split_lines(theirs).len(), "theirs lines partitioned");
    let mut bp = 0;
    let mut op = 0;
    let mut tp = 0;
    for run in runs {
        assert_eq!(run.base.start, bp, "base ranges contiguous");
        assert_eq!(run.ours.start, op, "ours ranges contiguous");
        assert_eq!(run.theirs.start, tp, "theirs ranges contiguous");
        bp = run.base.end;
        op = run.ours.end;
        tp = run.theirs.end;
    }
}

#[test]
fn align_identity_is_one_unchanged_run() {
    let text = "a\nb\nc\n";
    let runs = align_three_way(text, text, text, DiffAlgorithm::Myers);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].kind, AlignedRunKind::Unchanged);
    assert_eq!(runs[0].visual_rows(), 3);
    assert_partitions(&runs, text, text, text);
}

#[test]
fn align_classifies_side_changes_and_conflicts() {
    let base = "a\nb\nc\nd\ne\n";
    let ours = "a\nB1\nB2\nc\nd\nE-ours\n";
    let theirs = "a\nb\nc\nd\nE-theirs\n";
    let runs = align_three_way(base, ours, theirs, DiffAlgorithm::Myers);
    assert_partitions(&runs, base, ours, theirs);

    // Fine-grained rows: a | b anchored to its replacement + theirs copy |
    // ours' second replacement line alone | c d | e as one modified-line
    // row holding both replacements.
    let kinds: Vec<_> = runs.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![
            AlignedRunKind::Unchanged,
            AlignedRunKind::OursChanged,
            AlignedRunKind::OursChanged,
            AlignedRunKind::Unchanged,
            AlignedRunKind::Conflict,
        ],
    );
    // Row 1: base `b` rides with ours' first replacement and theirs' copy.
    assert_eq!(runs[1].base, 1..2);
    assert_eq!(runs[1].ours, 1..2);
    assert_eq!(runs[1].theirs, 1..2);
    // Row 2: ours' second replacement line, base/theirs padded.
    assert_eq!(runs[2].base, 2..2);
    assert_eq!(runs[2].ours, 2..3);
    assert_eq!(runs[2].theirs.len(), 0);
    // Row 4: `e` modified by both — one row with both replacements.
    assert_eq!(runs[4].base.len(), 1);
    assert_eq!(runs[4].ours.len(), 1);
    assert_eq!(runs[4].theirs.len(), 1);
    assert_eq!(runs[4].visual_rows(), 1);
}

/// First kdiff3-parity case from manual testing: ours inserts two lines
/// and keeps the base line; theirs replaces it with four lines. The kept
/// base line must anchor to its identical ours copy on one row, with
/// theirs' replacement lines flowing 1:1 through all rows.
#[test]
fn align_anchors_kept_line_below_paired_insertions() {
    let base = "ctx\nreturn SLA\n";
    let ours = "ctx\nif critical:\n    return 2x\nreturn SLA\n";
    let theirs = "ctx\nthreshold = SLA\nif blocked:\n    half\nreturn threshold\n";
    let runs = align_three_way(base, ours, theirs, DiffAlgorithm::Myers);
    assert_partitions(&runs, base, ours, theirs);

    // ctx | [o1|t1] [o2|t2] | [base ~ ours copy ~ t3] | [t4]
    let rows: Vec<(usize, usize, usize)> = runs
        .iter()
        .map(|r| (r.base.len(), r.ours.len(), r.theirs.len()))
        .collect();
    assert_eq!(
        rows,
        vec![(1, 1, 1), (0, 2, 2), (1, 1, 1), (0, 0, 1)],
        "kept base line must share a row with its ours copy and theirs' third line",
    );
    // The anchored row pairs base line 1 with ours line 3 (its copy).
    assert_eq!(runs[2].base, 1..2);
    assert_eq!(runs[2].ours, 3..4);
    assert_eq!(runs[2].theirs, 3..4);
}

/// Third kdiff3-parity case from manual testing: a block where every
/// line was modified on both sides aligns 1:1 — seven clean rows, no
/// padding and no stranded base lines.
#[test]
fn align_pairs_fully_rewritten_blocks_line_by_line() {
    let make =
        |suffix: &str| -> String { (0..7).map(|i| format!("const_{i} = {suffix}\n")).collect() };
    let base = format!("ctx\n{}rest\n", make("base"));
    let ours = format!("ctx\n{}rest\n", make("ours"));
    let theirs = format!("ctx\n{}rest\n", make("theirs"));
    let runs = align_three_way(&base, &ours, &theirs, DiffAlgorithm::Myers);
    assert_partitions(&runs, &base, &ours, &theirs);

    let total_rows: usize = runs.iter().map(AlignedRun::visual_rows).sum();
    assert_eq!(total_rows, 9, "ctx + 7 paired rows + rest, no padding");
    for run in &runs {
        assert_eq!(
            (run.base.len(), run.ours.len(), run.theirs.len()),
            (run.visual_rows(), run.visual_rows(), run.visual_rows()),
            "every row pairs one line from each side",
        );
    }
}

/// Second kdiff3-parity case: ours replaces two base lines with two new
/// ones; theirs keeps one base line mid-region among three new lines.
/// The theirs-kept base line anchors to its copy, and ours' second
/// replacement rides on that anchor row.
#[test]
fn align_anchors_theirs_kept_line_with_ours_replacement() {
    let base = "ctx\ndone = len\nreturn round(done)\n";
    let ours = "ctx\nfinished = len\nreturn round(finished)\n";
    let theirs = "ctx\nactive = [t]\ndone = len\ndenom = len or 1\nreturn round(denom)\n";
    let runs = align_three_way(base, ours, theirs, DiffAlgorithm::Myers);
    assert_partitions(&runs, base, ours, theirs);

    // ctx | [o1|t1] | [base done ~ o2 ~ theirs copy] | [base return ~ t3]
    // | [t4]
    let rows: Vec<(usize, usize, usize)> = runs
        .iter()
        .map(|r| (r.base.len(), r.ours.len(), r.theirs.len()))
        .collect();
    assert_eq!(
        rows,
        vec![(1, 1, 1), (0, 1, 1), (1, 1, 1), (1, 0, 1), (0, 0, 1)],
    );
    // Anchor row: base `done = len` with theirs' identical copy.
    assert_eq!(runs[2].base, 1..2);
    assert_eq!(runs[2].theirs, 2..3);
}

#[test]
fn align_marks_identical_changes_as_both_same() {
    let base = "a\nb\nc\n";
    let both = "a\nX\nc\n";
    let runs = align_three_way(base, both, both, DiffAlgorithm::Myers);
    assert_partitions(&runs, base, both, both);
    assert!(runs.iter().any(|r| r.kind == AlignedRunKind::BothSame));
    assert!(!runs.iter().any(|r| r.kind == AlignedRunKind::Conflict));
}

#[test]
fn align_handles_empty_and_added_files() {
    let runs = align_three_way("", "", "", DiffAlgorithm::Myers);
    assert!(runs.is_empty());

    let runs = align_three_way("", "added\n", "other\n", DiffAlgorithm::Myers);
    assert_partitions(&runs, "", "added\n", "other\n");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].kind, AlignedRunKind::Conflict);
    assert_eq!(runs[0].visual_rows(), 1);
}

#[test]
fn align_matches_merge_conflict_detection() {
    // Alignment and merge_file must agree on what is a conflict.
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let ours = "1\nO2\n3\n4\n5\nO6\n7\n8\n9\n";
    let theirs = "1\nT2\n3\n4\n5\n6\n7\nT8\n9\n";
    let runs = align_three_way(base, ours, theirs, DiffAlgorithm::Myers);
    assert_partitions(&runs, base, ours, theirs);
    let conflict_runs = runs
        .iter()
        .filter(|r| r.kind == AlignedRunKind::Conflict)
        .count();
    let merged = merge_file(base, ours, theirs, &MergeOptions::default());
    assert_eq!(conflict_runs, merged.conflict_count);
}

// -----------------------------------------------------------------------
// Two-way alignment (aligned row space, no-base fallback)
// -----------------------------------------------------------------------

fn assert_two_way_partitions(runs: &[AlignedRun], ours: &str, theirs: &str) {
    let (b, o, t) = run_lens(runs);
    assert_eq!(b, 0, "two-way runs carry no base lines");
    assert_eq!(o, split_lines(ours).len(), "ours lines partitioned");
    assert_eq!(t, split_lines(theirs).len(), "theirs lines partitioned");
    let mut op = 0;
    let mut tp = 0;
    for run in runs {
        assert_eq!(run.ours.start, op, "ours ranges contiguous");
        assert_eq!(run.theirs.start, tp, "theirs ranges contiguous");
        op = run.ours.end;
        tp = run.theirs.end;
    }
}

#[test]
fn align_two_way_identity_is_one_unchanged_run() {
    let text = "a\nb\nc\n";
    let runs = align_two_way(text, text, DiffAlgorithm::Myers);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].kind, AlignedRunKind::Unchanged);
    assert_eq!(runs[0].visual_rows(), 3);
    assert_two_way_partitions(&runs, text, text);
}

#[test]
fn align_two_way_classifies_inserts_deletes_and_replacements() {
    let ours = "a\nours-only\nb\nreplaced-o\nc\n";
    let theirs = "a\nb\nreplaced-t\nc\ntheirs-only\n";
    let runs = align_two_way(ours, theirs, DiffAlgorithm::Myers);
    assert_two_way_partitions(&runs, ours, theirs);

    let kinds: Vec<_> = runs.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![
            AlignedRunKind::Unchanged,
            AlignedRunKind::OursChanged,
            AlignedRunKind::Unchanged,
            AlignedRunKind::Conflict,
            AlignedRunKind::Unchanged,
            AlignedRunKind::TheirsChanged,
        ],
    );
    // The replaced line pairs 1:1 on a single visual row.
    assert_eq!(runs[3].visual_rows(), 1);
    // Deletion pads theirs; insertion pads ours.
    assert_eq!(runs[1].theirs.len(), 0);
    assert_eq!(runs[5].ours.len(), 0);
}

#[test]
fn align_two_way_pairs_uneven_replacement_top_aligned() {
    let ours = "ctx\no1\no2\nrest\n";
    let theirs = "ctx\nt1\nt2\nt3\nt4\nrest\n";
    let runs = align_two_way(ours, theirs, DiffAlgorithm::Myers);
    assert_two_way_partitions(&runs, ours, theirs);

    let conflict = runs
        .iter()
        .find(|r| r.kind == AlignedRunKind::Conflict)
        .expect("replacement region");
    assert_eq!(conflict.ours.len(), 2);
    assert_eq!(conflict.theirs.len(), 4);
    assert_eq!(conflict.visual_rows(), 4, "shorter side padded below");
}

#[test]
fn align_two_way_handles_empty_sides() {
    assert!(align_two_way("", "", DiffAlgorithm::Myers).is_empty());

    let runs = align_two_way("", "added\n", DiffAlgorithm::Myers);
    assert_two_way_partitions(&runs, "", "added\n");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].kind, AlignedRunKind::TheirsChanged);

    let runs = align_two_way("gone\n", "", DiffAlgorithm::Myers);
    assert_two_way_partitions(&runs, "gone\n", "");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].kind, AlignedRunKind::OursChanged);
}

#[test]
fn merge_identical_multi_hunk_changes_resolves_cleanly() {
    let base = "a\nb\nc\nd\ne\n";
    let both = "a\nX\nc\nY\ne\n";
    let result = merge_file(base, both, both, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "a\nX\nc\nY\ne\n");
}
