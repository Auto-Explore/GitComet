use std::collections::BTreeMap;

use line_numbers::LineNumber;

use crate::display::context::{all_matched_lines_filled, opposite_positions};
use crate::display::hunks::{matched_lines_indexes_for_hunk, matched_pos_to_hunks, merge_adjacent};
use crate::display::side_by_side::lines_with_novel;
use crate::hash::DftHashMap;
use crate::lines::MaxLine;
use crate::parse::syntax::MatchedPos;
use crate::summary::DiffResult;

#[derive(Debug, PartialEq)]
pub(crate) enum Status {
    Unchanged,
    Changed,
    Created,
    Deleted,
}

#[derive(Debug)]
pub(crate) struct File {
    pub(crate) aligned_lines: Vec<(Option<u32>, Option<u32>)>,
    pub(crate) chunks: Vec<Vec<Line>>,
    pub(crate) status: Status,
}

impl File {
    fn with_sections(
        aligned_lines: Vec<(Option<u32>, Option<u32>)>,
        chunks: Vec<Vec<Line>>,
    ) -> Self {
        File {
            aligned_lines,
            chunks,
            status: Status::Changed,
        }
    }

    fn with_status(status: Status) -> Self {
        File {
            aligned_lines: Vec::new(),
            chunks: Vec::new(),
            status,
        }
    }
}

impl From<&DiffResult> for File {
    fn from(summary: &DiffResult) -> Self {
        let lhs_src = &summary.lhs_src;
        let rhs_src = &summary.rhs_src;
        let opposite_to_lhs = opposite_positions(&summary.lhs_positions);
        let opposite_to_rhs = opposite_positions(&summary.rhs_positions);

        let hunks = matched_pos_to_hunks(&summary.lhs_positions, &summary.rhs_positions);
        let hunks = merge_adjacent(
            &hunks,
            &opposite_to_lhs,
            &opposite_to_rhs,
            lhs_src.max_line(),
            rhs_src.max_line(),
            0,
        );

        if hunks.is_empty() {
            return File::with_status(Status::Unchanged);
        }

        if lhs_src.is_empty() {
            return File::with_status(Status::Created);
        }
        if rhs_src.is_empty() {
            return File::with_status(Status::Deleted);
        }

        let lhs_lines = lhs_src.split('\n').collect::<Vec<&str>>();
        let rhs_lines = rhs_src.split('\n').collect::<Vec<&str>>();

        let (lhs_lines_with_novel, rhs_lines_with_novel) =
            lines_with_novel(&summary.lhs_positions, &summary.rhs_positions);

        let matched_lines = all_matched_lines_filled(
            &summary.lhs_positions,
            &summary.rhs_positions,
            &lhs_lines,
            &rhs_lines,
        );

        let aligned_lines: Vec<(Option<u32>, Option<u32>)> = matched_lines
            .iter()
            .map(|(lhs, rhs)| (lhs.map(|l| l.0), rhs.map(|l| l.0)))
            .collect();

        let mut matched_lines = &matched_lines[..];
        let lhs_novel_by_line = novel_by_line(&summary.lhs_positions);
        let rhs_novel_by_line = novel_by_line(&summary.rhs_positions);

        let mut chunks = Vec::with_capacity(hunks.len());
        for hunk in &hunks {
            let mut lines = BTreeMap::new();

            let (start_i, end_i) = matched_lines_indexes_for_hunk(matched_lines, hunk, 0);
            let aligned_lines = &matched_lines[start_i..end_i];
            matched_lines = &matched_lines[start_i..];

            for (lhs_line_num, rhs_line_num) in aligned_lines {
                if !lhs_lines_with_novel.contains(&lhs_line_num.unwrap_or(LineNumber(0)))
                    && !rhs_lines_with_novel.contains(&rhs_line_num.unwrap_or(LineNumber(0)))
                {
                    continue;
                }

                let line = lines
                    .entry((lhs_line_num.map(|l| l.0), rhs_line_num.map(|l| l.0)))
                    .or_insert_with(|| {
                        Line::new(lhs_line_num.map(|l| l.0), rhs_line_num.map(|l| l.0))
                    });

                if let Some(line_num) = lhs_line_num {
                    add_changes_to_side(line.lhs.as_mut().unwrap(), *line_num, &lhs_novel_by_line);
                }
                if let Some(line_num) = rhs_line_num {
                    add_changes_to_side(line.rhs.as_mut().unwrap(), *line_num, &rhs_novel_by_line);
                }
            }

            chunks.push(lines.into_values().collect());
        }

        File::with_sections(aligned_lines, chunks)
    }
}

#[derive(Debug)]
pub(crate) struct Line {
    pub(crate) lhs: Option<Side>,
    pub(crate) rhs: Option<Side>,
}

impl Line {
    fn new(lhs_number: Option<u32>, rhs_number: Option<u32>) -> Self {
        Line {
            lhs: lhs_number.map(Side::new),
            rhs: rhs_number.map(Side::new),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Side {
    pub(crate) line_number: u32,
    pub(crate) changes: Vec<Change>,
}

impl Side {
    fn new(line_number: u32) -> Self {
        Side {
            line_number,
            changes: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Change {
    pub(crate) start: u32,
    pub(crate) end: u32,
}

fn add_changes_to_side(
    side: &mut Side,
    line_num: LineNumber,
    novel_by_line: &DftHashMap<LineNumber, Vec<&MatchedPos>>,
) {
    for m in novel_by_line.get(&line_num).into_iter().flatten() {
        side.changes.push(Change {
            start: m.pos.start_col,
            end: m.pos.end_col,
        })
    }
}

// GitComet: one pass over the positions, rather than one per changed line.
fn novel_by_line(matches: &[MatchedPos]) -> DftHashMap<LineNumber, Vec<&MatchedPos>> {
    let mut by_line: DftHashMap<LineNumber, Vec<&MatchedPos>> = DftHashMap::default();
    for m in matches.iter().filter(|m| m.kind.is_novel()) {
        by_line.entry(m.pos.line).or_default().push(m);
    }
    by_line
}
