//! section 30 mergetool split/join: byte-range surgery on conflict-marker text.
//!
//! Splitting rewrites one `<<<<<<<`/`>>>>>>>` block into 2–3 adjacent blocks
//! at block-local line boundaries; joining merges two neighbouring blocks,
//! absorbing the context between them into every side. Both operate purely on
//! the merged text. Sessions re-parse the edited marker projection and then
//! reconcile its structural blocks with the shared merge plan.

use super::marker_parse::{
    ParsedConflictBlockRanges, ParsedConflictSegmentRanges, parse_conflict_marker_ranges,
};

/// Block-local split boundaries, in line offsets within each side's content.
/// Two boundaries produce up to 3 parts: `[0..b0)`, `[b0..b1)`, `[b1..len)`.
/// Boundaries clamp to each side's line count; `b1` clamps up to `b0`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictRegionSplitBoundaries {
    pub ours: [usize; 2],
    pub theirs: [usize; 2],
    /// `None` for base-less (both-added) blocks; must match the block.
    pub base: Option<[usize; 2]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictRegionEditOutcome {
    pub new_text: String,
    /// Number of marker blocks the edited region became (split: 2..=3, join: 1).
    pub parts: usize,
}

/// Byte offset of the start of line `n` within `side` (`n >= line count`
/// clamps to `side.len()`). Sides are newline-terminated by the marker
/// parser, so line `k` starts right after the `k`-th `\n`.
fn nth_line_start(side: &str, n: usize) -> usize {
    let mut start = 0usize;
    for _ in 0..n {
        match side[start..].find('\n') {
            Some(rel) => start += rel + 1,
            None => return side.len(),
        }
    }
    start
}

fn conflict_blocks(current: &str) -> Vec<ParsedConflictBlockRanges> {
    parse_conflict_marker_ranges(current)
        .into_iter()
        .filter_map(|segment| match segment {
            ParsedConflictSegmentRanges::Text(_) => None,
            ParsedConflictSegmentRanges::Conflict(block) => Some(block),
        })
        .collect()
}

/// The four marker lines of a block, sliced verbatim (labels preserved).
struct MarkerLines<'a> {
    opening: &'a str,
    base_intro: Option<&'a str>,
    separator: &'a str,
    closing: &'a str,
}

fn marker_lines<'a>(current: &'a str, block: &ParsedConflictBlockRanges) -> MarkerLines<'a> {
    let separator_start = block
        .base
        .as_ref()
        .map(|base| base.end)
        .unwrap_or(block.ours.end);
    MarkerLines {
        opening: &current[block.marker_start..block.ours.start],
        base_intro: block
            .base
            .as_ref()
            .map(|base| &current[block.ours.end..base.start]),
        separator: &current[separator_start..block.theirs.start],
        closing: &current[block.theirs.end..block.marker_end],
    }
}

fn render_block(
    out: &mut String,
    markers: &MarkerLines<'_>,
    ours: &str,
    base: Option<&str>,
    theirs: &str,
) {
    debug_assert!(ours.is_empty() || ours.ends_with('\n'));
    debug_assert!(theirs.is_empty() || theirs.ends_with('\n'));
    debug_assert!(base.is_none_or(|b| b.is_empty() || b.ends_with('\n')));
    out.push_str(markers.opening);
    out.push_str(ours);
    if let (Some(intro), Some(base)) = (markers.base_intro, base) {
        out.push_str(intro);
        out.push_str(base);
    }
    out.push_str(markers.separator);
    out.push_str(theirs);
    out.push_str(markers.closing);
}

/// Split conflict block `region_index` (index among conflict blocks, same
/// order as `ConflictSession::regions`) into up to 3 blocks at `boundaries`.
/// Parts that are empty on every side are skipped; fewer than 2 remaining
/// parts (degenerate selection) returns `None`. Marker lines are preserved
/// verbatim, so labels and CRLF content survive byte-for-byte.
pub fn split_conflict_region_text(
    current: &str,
    region_index: usize,
    boundaries: ConflictRegionSplitBoundaries,
) -> Option<ConflictRegionEditOutcome> {
    let blocks = conflict_blocks(current);
    let block = blocks.get(region_index)?;
    if block.base.is_some() != boundaries.base.is_some() {
        return None;
    }

    let markers = marker_lines(current, block);
    let ours = &current[block.ours.clone()];
    let theirs = &current[block.theirs.clone()];
    let base = block.base.as_ref().map(|range| &current[range.clone()]);

    // Per side: three byte-range slices from the two (clamped) boundaries.
    fn side_slices(side: &str, bounds: [usize; 2]) -> [&str; 3] {
        let cut0 = nth_line_start(side, bounds[0]);
        let cut1 = nth_line_start(side, bounds[0].max(bounds[1])).max(cut0);
        [&side[..cut0], &side[cut0..cut1], &side[cut1..]]
    }
    let ours_parts = side_slices(ours, boundaries.ours);
    let theirs_parts = side_slices(theirs, boundaries.theirs);
    let base_parts = match (base, boundaries.base) {
        (Some(base), Some(bounds)) => Some(side_slices(base, bounds)),
        _ => None,
    };

    let emitted: Vec<usize> = (0..3)
        .filter(|&p| {
            !ours_parts[p].is_empty()
                || !theirs_parts[p].is_empty()
                || base_parts.is_some_and(|parts| !parts[p].is_empty())
        })
        .collect();
    if emitted.len() < 2 {
        return None;
    }

    let mut new_text = String::with_capacity(current.len() + emitted.len() * 64);
    new_text.push_str(&current[..block.marker_start]);
    for (emitted_ix, &p) in emitted.iter().enumerate() {
        if emitted_ix > 0 && !markers.closing.ends_with('\n') {
            new_text.push_str(if markers.opening.ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            });
        }
        render_block(
            &mut new_text,
            &markers,
            ours_parts[p],
            base_parts.as_ref().map(|parts| parts[p]),
            theirs_parts[p],
        );
    }
    new_text.push_str(&current[block.marker_end..]);

    Some(ConflictRegionEditOutcome {
        new_text,
        parts: emitted.len(),
    })
}

/// Join conflict blocks `first_region_index` and `first_region_index + 1`
/// into one block, absorbing the context between them into every side.
/// Marker lines come from the first block (closing from the second), so both
/// end labels are preserved. Returns `None` when the neighbour is missing or
/// the intervening context contains marker-looking lines (malformed marker
/// text the parser preserved as context — joining across it would corrupt).
pub fn join_conflict_regions_text(
    current: &str,
    first_region_index: usize,
) -> Option<ConflictRegionEditOutcome> {
    let blocks = conflict_blocks(current);
    let first = blocks.get(first_region_index)?;
    let second = blocks.get(first_region_index + 1)?;

    let ctx = &current[first.marker_end..second.marker_start];
    let ctx_is_markerish = ctx.lines().any(|line| {
        let bytes = line.as_bytes();
        bytes.starts_with(b"<<<<<<<")
            || bytes.starts_with(b"=======")
            || bytes.starts_with(b">>>>>>>")
            || bytes.starts_with(b"|||||||")
    });
    if ctx_is_markerish {
        return None;
    }

    let first_markers = marker_lines(current, first);
    let second_markers = marker_lines(current, second);
    let markers = MarkerLines {
        opening: first_markers.opening,
        // Mixed base presence should not occur within one file, but degrade
        // gracefully: use whichever intro line exists.
        base_intro: first_markers.base_intro.or(second_markers.base_intro),
        separator: first_markers.separator,
        closing: second_markers.closing,
    };

    let joined_side = |a: &str, b: &str| {
        let mut side = String::with_capacity(a.len() + ctx.len() + b.len());
        side.push_str(a);
        side.push_str(ctx);
        side.push_str(b);
        side
    };
    let ours = joined_side(&current[first.ours.clone()], &current[second.ours.clone()]);
    let theirs = joined_side(
        &current[first.theirs.clone()],
        &current[second.theirs.clone()],
    );
    let base = if first.base.is_none() && second.base.is_none() {
        None
    } else {
        Some(joined_side(
            first
                .base
                .as_ref()
                .map(|range| &current[range.clone()])
                .unwrap_or(""),
            second
                .base
                .as_ref()
                .map(|range| &current[range.clone()])
                .unwrap_or(""),
        ))
    };

    let mut new_text = String::with_capacity(current.len());
    new_text.push_str(&current[..first.marker_start]);
    render_block(&mut new_text, &markers, &ours, base.as_deref(), &theirs);
    new_text.push_str(&current[second.marker_end..]);

    Some(ConflictRegionEditOutcome { new_text, parts: 1 })
}

#[cfg(test)]
mod tests {
    use super::super::marker_parse::parse_conflict_marker_segments;
    use super::*;

    fn two_sided(ours: &[&str], theirs: &[&str]) -> String {
        let mut text = String::from("ctx head\n<<<<<<< HEAD\n");
        for line in ours {
            text.push_str(line);
            text.push('\n');
        }
        text.push_str("=======\n");
        for line in theirs {
            text.push_str(line);
            text.push('\n');
        }
        text.push_str(">>>>>>> feature/x\nctx tail\n");
        text
    }

    fn block_count(text: &str) -> usize {
        parse_conflict_marker_segments(text)
            .iter()
            .filter(|segment| {
                matches!(
                    segment,
                    super::super::marker_parse::ParsedConflictSegment::Conflict(_)
                )
            })
            .count()
    }

    #[test]
    fn split_two_sided_into_three_parts() {
        let text = two_sided(&["o1", "o2", "o3"], &["t1", "t2", "t3"]);
        let outcome = split_conflict_region_text(
            &text,
            0,
            ConflictRegionSplitBoundaries {
                ours: [1, 2],
                theirs: [1, 2],
                base: None,
            },
        )
        .expect("split");
        assert_eq!(outcome.parts, 3);
        assert_eq!(block_count(&outcome.new_text), 3);
        assert!(outcome.new_text.starts_with(
            "ctx head\n<<<<<<< HEAD\no1\n=======\nt1\n>>>>>>> feature/x\n<<<<<<< HEAD\no2\n"
        ));
        assert!(outcome.new_text.ends_with(">>>>>>> feature/x\nctx tail\n"));
    }

    #[test]
    fn split_at_edges_yields_two_parts_and_skips_empty() {
        let text = two_sided(&["o1", "o2"], &["t1", "t2"]);
        // First boundary at 0: "before" part empty on both sides -> skipped.
        let outcome = split_conflict_region_text(
            &text,
            0,
            ConflictRegionSplitBoundaries {
                ours: [0, 1],
                theirs: [0, 1],
                base: None,
            },
        )
        .expect("split");
        assert_eq!(outcome.parts, 2);
        assert_eq!(block_count(&outcome.new_text), 2);
    }

    #[test]
    fn split_preserves_no_final_newline_without_concatenating_markers() {
        let text = "<<<<<<< HEAD\no1\no2\n=======\nt1\nt2\n>>>>>>> feature/x";
        let outcome = split_conflict_region_text(
            text,
            0,
            ConflictRegionSplitBoundaries {
                ours: [1, 2],
                theirs: [1, 2],
                base: None,
            },
        )
        .expect("split");

        assert_eq!(outcome.parts, 2);
        assert_eq!(block_count(&outcome.new_text), 2);
        assert!(outcome.new_text.contains(">>>>>>> feature/x\n<<<<<<< HEAD"));
        assert!(!outcome.new_text.ends_with('\n'));
    }

    #[test]
    fn degenerate_and_out_of_range_splits_are_rejected() {
        let text = two_sided(&["o1", "o2"], &["t1", "t2"]);
        // Whole block in one part.
        assert!(
            split_conflict_region_text(
                &text,
                0,
                ConflictRegionSplitBoundaries {
                    ours: [0, 2],
                    theirs: [0, 2],
                    base: None,
                },
            )
            .is_none()
        );
        // Region index out of range.
        assert!(
            split_conflict_region_text(
                &text,
                1,
                ConflictRegionSplitBoundaries {
                    ours: [1, 1],
                    theirs: [1, 1],
                    base: None,
                },
            )
            .is_none()
        );
        // Base boundaries against a base-less block.
        assert!(
            split_conflict_region_text(
                &text,
                0,
                ConflictRegionSplitBoundaries {
                    ours: [1, 1],
                    theirs: [1, 1],
                    base: Some([1, 1]),
                },
            )
            .is_none()
        );
    }

    #[test]
    fn split_uneven_sides_clamps_boundaries() {
        let text = two_sided(&["o1", "o2", "o3", "o4", "o5"], &["t1"]);
        let outcome = split_conflict_region_text(
            &text,
            0,
            ConflictRegionSplitBoundaries {
                ours: [2, 4],
                theirs: [1, 9],
                base: None,
            },
        )
        .expect("split");
        assert_eq!(outcome.parts, 3);
        // theirs cut0 == len, so parts 2 and 3 have empty theirs but keep ours.
        assert_eq!(block_count(&outcome.new_text), 3);
        let segments = parse_conflict_marker_segments(&outcome.new_text);
        let blocks: Vec<_> = segments
            .iter()
            .filter_map(|segment| match segment {
                super::super::marker_parse::ParsedConflictSegment::Conflict(block) => Some(block),
                _ => None,
            })
            .collect();
        assert_eq!(blocks[0].ours, "o1\no2\n");
        assert_eq!(blocks[0].theirs, "t1\n");
        assert_eq!(blocks[1].ours, "o3\no4\n");
        assert_eq!(blocks[1].theirs, "");
        assert_eq!(blocks[2].ours, "o5\n");
        assert_eq!(blocks[2].theirs, "");
    }

    #[test]
    fn split_diff3_block_splits_base_and_preserves_labels() {
        let text = "head\n<<<<<<< HEAD\no1\no2\n||||||| merged common ancestors\nb1\nb2\n=======\nt1\nt2\n>>>>>>> feature/x\ntail\n";
        let outcome = split_conflict_region_text(
            text,
            0,
            ConflictRegionSplitBoundaries {
                ours: [1, 1],
                theirs: [1, 1],
                base: Some([1, 1]),
            },
        )
        .expect("split");
        assert_eq!(outcome.parts, 2);
        assert_eq!(
            outcome.new_text,
            "head\n<<<<<<< HEAD\no1\n||||||| merged common ancestors\nb1\n=======\nt1\n>>>>>>> feature/x\n<<<<<<< HEAD\no2\n||||||| merged common ancestors\nb2\n=======\nt2\n>>>>>>> feature/x\ntail\n"
        );
    }

    #[test]
    fn split_preserves_crlf_content() {
        let text = "<<<<<<< HEAD\no1\r\no2\r\n=======\nt1\r\nt2\r\n>>>>>>> theirs\n";
        let outcome = split_conflict_region_text(
            text,
            0,
            ConflictRegionSplitBoundaries {
                ours: [1, 1],
                theirs: [1, 1],
                base: None,
            },
        )
        .expect("split");
        assert_eq!(
            outcome.new_text,
            "<<<<<<< HEAD\no1\r\n=======\nt1\r\n>>>>>>> theirs\n<<<<<<< HEAD\no2\r\n=======\nt2\r\n>>>>>>> theirs\n"
        );
    }

    #[test]
    fn join_of_two_part_split_round_trips_exactly() {
        let text = two_sided(&["o1", "o2", "o3"], &["t1", "t2"]);
        let split = split_conflict_region_text(
            &text,
            0,
            ConflictRegionSplitBoundaries {
                ours: [2, 3],
                theirs: [1, 2],
                base: None,
            },
        )
        .expect("split");
        assert_eq!(split.parts, 2);
        let joined = join_conflict_regions_text(&split.new_text, 0).expect("join");
        assert_eq!(joined.new_text, text, "join(split_2part(text)) == text");
        assert_eq!(joined.parts, 1);
    }

    #[test]
    fn join_absorbs_context_into_all_sides() {
        let text = "<<<<<<< HEAD\no1\n||||||| base\nb1\n=======\nt1\n>>>>>>> x\nctx1\nctx2\n<<<<<<< HEAD\no2\n||||||| base\nb2\n=======\nt2\n>>>>>>> x\n";
        let joined = join_conflict_regions_text(text, 0).expect("join");
        assert_eq!(
            joined.new_text,
            "<<<<<<< HEAD\no1\nctx1\nctx2\no2\n||||||| base\nb1\nctx1\nctx2\nb2\n=======\nt1\nctx1\nctx2\nt2\n>>>>>>> x\n"
        );
        assert_eq!(block_count(&joined.new_text), 1);
    }

    #[test]
    fn join_rejects_missing_neighbour_and_markerish_context() {
        let single = two_sided(&["o1"], &["t1"]);
        assert!(join_conflict_regions_text(&single, 0).is_none());

        // A lone separator line survives parsing as plain context, but once
        // absorbed into the joined ours it would terminate the ours scan
        // early on re-parse — the join must refuse.
        let markerish = "<<<<<<< HEAD\no1\n=======\nt1\n>>>>>>> x\n=======\n<<<<<<< HEAD\no2\n=======\nt2\n>>>>>>> x\n";
        assert_eq!(block_count(markerish), 2);
        assert!(join_conflict_regions_text(markerish, 0).is_none());
    }
}
