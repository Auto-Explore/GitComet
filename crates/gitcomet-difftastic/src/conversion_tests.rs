use difftastic::api::{AlignedLine as Row, Changes};

use super::*;

fn ranges(line: u32, ranges: impl IntoIterator<Item = Range<u32>>) -> LineRanges {
    LineRanges {
        line,
        ranges: ranges.into_iter().collect(),
    }
}

fn rust_changes(aligned_lines: Vec<Row>, chunks: Vec<Vec<ChunkLine>>) -> Comparison {
    Comparison::Changed(Changes {
        language: "Rust",
        aligned_lines,
        chunks,
    })
}

fn compared(outcome: Result<StructuralDiffOutcome, InvalidComparison>) -> StructuralDiff {
    match outcome {
        Ok(StructuralDiffOutcome::Compared(diff)) => diff,
        other => panic!("expected a structural comparison, got {other:?}"),
    }
}

#[test]
fn changes_keep_alignment_and_utf8_byte_ranges() {
    let diff = compared(outcome(
        rust_changes(
            vec![Row::Both { old: 0, new: 0 }],
            vec![vec![ChunkLine::Both {
                old: ranges(0, []),
                new: ranges(0, [12..17, 18..19]),
            }]],
        ),
        "let café = 1;\n",
        "let café = wrap(1);\n",
    ));
    assert_eq!(diff.language, "Rust");
    assert_eq!(diff.aligned_lines, vec![(Some(0), Some(0))]);
    assert!(diff.old_changes.is_empty());
    assert_eq!(diff.new_changes[&0], vec![12..17, 18..19]);
    assert_eq!(diff.blocks, vec![0..1]);
}

#[test]
fn a_range_inside_a_utf8_character_is_rejected() {
    let comparison = rust_changes(
        vec![Row::Both { old: 0, new: 0 }],
        vec![vec![ChunkLine::New(ranges(0, std::iter::once(1..2)))]],
    );
    assert_eq!(
        outcome(comparison, "a", "é"),
        Err(InvalidComparison::RangeNotInLine {
            side: Side::New,
            line: 0,
            range: 1..2,
        })
    );
}

#[test]
fn a_range_past_the_line_end_is_rejected() {
    let comparison = rust_changes(
        vec![Row::Both { old: 0, new: 0 }],
        vec![vec![ChunkLine::New(ranges(0, std::iter::once(0..2)))]],
    );
    assert_eq!(
        outcome(comparison, "a\n", "b\n"),
        Err(InvalidComparison::RangeNotInLine {
            side: Side::New,
            line: 0,
            range: 0..2,
        })
    );
}

#[test]
fn alignment_must_list_every_source_line_once_in_order() {
    for (rows, error) in [
        (
            vec![Row::Both { old: 0, new: 0 }],
            InvalidComparison::AlignmentIncomplete { side: Side::Old },
        ),
        (
            vec![Row::Both { old: 0, new: 0 }, Row::Both { old: 0, new: 1 }],
            InvalidComparison::AlignmentOutOfOrder {
                side: Side::Old,
                line: 0,
            },
        ),
        (
            vec![Row::Both { old: 1, new: 0 }, Row::Both { old: 0, new: 1 }],
            InvalidComparison::AlignmentOutOfOrder {
                side: Side::Old,
                line: 1,
            },
        ),
        (
            vec![
                Row::Both { old: 0, new: 0 },
                Row::Both { old: 1, new: 1 },
                Row::New(2),
            ],
            InvalidComparison::AlignmentPastEnd {
                side: Side::New,
                line: 2,
                count: 2,
            },
        ),
    ] {
        let comparison = rust_changes(
            rows,
            vec![vec![ChunkLine::New(ranges(0, std::iter::once(0..1)))]],
        );
        assert_eq!(outcome(comparison, "a\nb\n", "c\nd\n"), Err(error));
    }
}

#[test]
fn chunk_sides_must_share_their_aligned_row() {
    let comparison = rust_changes(
        vec![Row::Old(0), Row::New(0)],
        vec![vec![ChunkLine::Both {
            old: ranges(0, std::iter::once(0..1)),
            new: ranges(0, std::iter::once(0..1)),
        }]],
    );
    assert_eq!(
        outcome(comparison, "a\n", "b\n"),
        Err(InvalidComparison::ChunkRowsDisagree {
            old_row: 0,
            new_row: 1,
        })
    );
}

#[test]
fn a_change_without_ranges_is_rejected() {
    let comparison = rust_changes(
        vec![Row::Both { old: 0, new: 0 }],
        vec![vec![ChunkLine::New(ranges(0, []))]],
    );
    assert_eq!(
        outcome(comparison, "a\n", "b\n"),
        Err(InvalidComparison::NoChangedRanges)
    );
}

#[test]
fn carriage_returns_and_missing_final_newlines_keep_line_numbers() {
    let diff = compared(outcome(
        rust_changes(
            vec![Row::Both { old: 0, new: 0 }],
            vec![vec![ChunkLine::New(ranges(0, std::iter::once(0..1)))]],
        ),
        "a\r\n",
        "b\r",
    ));
    assert_eq!(diff.aligned_lines, vec![(Some(0), Some(0))]);
    assert_eq!(diff.new_changes[&0], vec![0..1]);
}

#[test]
fn a_created_file_marks_every_line_including_empty_ones() {
    let diff = compared(outcome(
        Comparison::Created { language: "Rust" },
        "",
        "a\n\n",
    ));
    assert_eq!(diff.aligned_lines, vec![(None, Some(0)), (None, Some(1))]);
    assert_eq!(diff.new_changes[&0], vec![0..1]);
    assert_eq!(diff.new_changes[&1], vec![0..0]);
    assert_eq!(diff.blocks, vec![0..2]);
}

#[test]
fn a_created_file_with_an_old_source_is_rejected() {
    assert_eq!(
        outcome(Comparison::Created { language: "Rust" }, "a\n", "b\n"),
        Err(InvalidComparison::CreatedWithOldSource)
    );
}

#[test]
fn a_deleted_file_with_a_new_source_is_rejected() {
    assert_eq!(
        outcome(Comparison::Deleted { language: "Rust" }, "a\n", "b\n"),
        Err(InvalidComparison::DeletedWithNewSource)
    );
}

#[test]
fn a_change_on_a_missing_line_is_rejected() {
    let comparison = rust_changes(
        vec![Row::Both { old: 0, new: 0 }],
        vec![vec![ChunkLine::New(ranges(1, std::iter::once(0..1)))]],
    );
    assert_eq!(
        outcome(comparison, "a\n", "b\n"),
        Err(InvalidComparison::MissingLine {
            side: Side::New,
            line: 1,
        })
    );
}

#[test]
fn fallback_reasons_map_one_to_one() {
    for (reason, expected) in [
        (
            Fallback::UnsupportedLanguage,
            DiffEngineFallback::UnsupportedLanguage,
        ),
        (Fallback::ParseErrors, DiffEngineFallback::ParseErrors),
        (Fallback::ByteLimit, DiffEngineFallback::ByteLimit),
        (Fallback::GraphLimit, DiffEngineFallback::GraphLimit),
        (Fallback::NestingLimit, DiffEngineFallback::NestingLimit),
    ] {
        assert_eq!(
            outcome(Comparison::Fallback(reason), "a", "b"),
            Ok(StructuralDiffOutcome::Fallback(expected))
        );
    }
}
