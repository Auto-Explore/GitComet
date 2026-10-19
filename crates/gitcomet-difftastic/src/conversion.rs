//! Turns difftastic's comparison into the diff pane's structural diff, checking
//! that its lines and ranges fit the caller's texts.

use std::collections::BTreeMap;
use std::ops::Range;

use difftastic::api::{self, ChunkLine, Comparison, Fallback, LineRanges};
use gitcomet_core::diff_engine::{
    AlignedLine, DiffEngineFallback, StructuralDiff, StructuralDiffOutcome,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Side {
    Old,
    New,
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum InvalidComparison {
    #[error("created file has an old source")]
    CreatedWithOldSource,
    #[error("deleted file has a new source")]
    DeletedWithNewSource,
    #[error("{side:?} alignment lists line {line} out of order")]
    AlignmentOutOfOrder { side: Side, line: usize },
    #[error("{side:?} alignment lists line {line} of {count}")]
    AlignmentPastEnd {
        side: Side,
        line: usize,
        count: usize,
    },
    #[error("{side:?} alignment omits source lines")]
    AlignmentIncomplete { side: Side },
    #[error("{side:?} change on missing line {line}")]
    MissingLine { side: Side, line: usize },
    #[error("{side:?} change {range:?} is not a character range of line {line}")]
    RangeNotInLine {
        side: Side,
        line: usize,
        range: Range<usize>,
    },
    #[error("chunk puts old row {old_row} beside new row {new_row}")]
    ChunkRowsDisagree { old_row: usize, new_row: usize },
    #[error("changed comparison contains no changed ranges")]
    NoChangedRanges,
}

pub(crate) fn outcome(
    comparison: Comparison,
    old: &str,
    new: &str,
) -> Result<StructuralDiffOutcome, InvalidComparison> {
    match comparison {
        Comparison::Unchanged => Ok(StructuralDiffOutcome::Unchanged),
        Comparison::Fallback(reason) => Ok(StructuralDiffOutcome::Fallback(fallback(reason))),
        Comparison::Created { .. } if !old.is_empty() => {
            Err(InvalidComparison::CreatedWithOldSource)
        }
        Comparison::Deleted { .. } if !new.is_empty() => {
            Err(InvalidComparison::DeletedWithNewSource)
        }
        Comparison::Created { language } | Comparison::Deleted { language } => Ok(
            StructuralDiffOutcome::Compared(one_sided(language, old, new)),
        ),
        Comparison::Changed(changes) => {
            changed(changes, old, new).map(StructuralDiffOutcome::Compared)
        }
    }
}

fn fallback(reason: Fallback) -> DiffEngineFallback {
    match reason {
        Fallback::UnsupportedLanguage => DiffEngineFallback::UnsupportedLanguage,
        Fallback::ParseErrors => DiffEngineFallback::ParseErrors,
        Fallback::ByteLimit => DiffEngineFallback::ByteLimit,
        Fallback::GraphLimit => DiffEngineFallback::GraphLimit,
        Fallback::NestingLimit => DiffEngineFallback::NestingLimit,
    }
}

fn changed(
    changes: api::Changes,
    old: &str,
    new: &str,
) -> Result<StructuralDiff, InvalidComparison> {
    let old_lines: Vec<_> = old.split_terminator('\n').collect();
    let new_lines: Vec<_> = new.split_terminator('\n').collect();
    let aligned_lines = alignment(&changes.aligned_lines, old_lines.len(), new_lines.len())?;
    let mut old_rows = vec![0; old_lines.len()];
    let mut new_rows = vec![0; new_lines.len()];
    for (row, &(old, new)) in aligned_lines.iter().enumerate() {
        if let Some(line) = old {
            old_rows[line] = row;
        }
        if let Some(line) = new {
            new_rows[line] = row;
        }
    }
    let mut diff = StructuralDiff {
        language: changes.language.to_owned(),
        aligned_lines,
        old_changes: BTreeMap::new(),
        new_changes: BTreeMap::new(),
        blocks: Vec::new(),
    };
    for chunk in changes.chunks {
        let mut bounds: Option<Range<usize>> = None;
        for line in chunk {
            let (old_ranges, new_ranges) = match line {
                ChunkLine::Both { old, new } => (Some(old), Some(new)),
                ChunkLine::Old(old) => (Some(old), None),
                ChunkLine::New(new) => (None, Some(new)),
            };
            let old_row = old_ranges
                .map(|ranges| {
                    add_side(
                        Side::Old,
                        ranges,
                        &old_lines,
                        &old_rows,
                        &mut diff.old_changes,
                    )
                })
                .transpose()?;
            let new_row = new_ranges
                .map(|ranges| {
                    add_side(
                        Side::New,
                        ranges,
                        &new_lines,
                        &new_rows,
                        &mut diff.new_changes,
                    )
                })
                .transpose()?;
            if let (Some(old_row), Some(new_row)) = (old_row, new_row)
                && old_row != new_row
            {
                return Err(InvalidComparison::ChunkRowsDisagree { old_row, new_row });
            }
            for row in old_row.into_iter().chain(new_row) {
                bounds = Some(match bounds {
                    Some(bounds) => bounds.start.min(row)..bounds.end.max(row + 1),
                    None => row..row + 1,
                });
            }
        }
        diff.blocks.extend(bounds);
    }
    if diff.old_changes.is_empty() && diff.new_changes.is_empty() {
        return Err(InvalidComparison::NoChangedRanges);
    }
    diff.blocks.sort_by_key(|block| block.start);
    for ranges in diff
        .old_changes
        .values_mut()
        .chain(diff.new_changes.values_mut())
    {
        ranges.sort_by_key(|range| (range.start, range.end));
        ranges.dedup();
    }
    Ok(diff)
}

fn alignment(
    rows: &[api::AlignedLine],
    old_count: usize,
    new_count: usize,
) -> Result<Vec<AlignedLine>, InvalidComparison> {
    let mut old_next = 0;
    let mut new_next = 0;
    let aligned = rows
        .iter()
        .map(|row| {
            let (old, new) = match *row {
                api::AlignedLine::Both { old, new } => (Some(old), Some(new)),
                api::AlignedLine::Old(old) => (Some(old), None),
                api::AlignedLine::New(new) => (None, Some(new)),
            };
            Ok((
                next_line(Side::Old, old, &mut old_next, old_count)?,
                next_line(Side::New, new, &mut new_next, new_count)?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if old_next < old_count {
        return Err(InvalidComparison::AlignmentIncomplete { side: Side::Old });
    }
    if new_next < new_count {
        return Err(InvalidComparison::AlignmentIncomplete { side: Side::New });
    }
    Ok(aligned)
}

fn next_line(
    side: Side,
    line: Option<u32>,
    next: &mut usize,
    count: usize,
) -> Result<Option<usize>, InvalidComparison> {
    let Some(line) = line else { return Ok(None) };
    let line = line as usize;
    if line != *next {
        return Err(InvalidComparison::AlignmentOutOfOrder { side, line });
    }
    if line >= count {
        return Err(InvalidComparison::AlignmentPastEnd { side, line, count });
    }
    *next += 1;
    Ok(Some(line))
}

fn add_side(
    side: Side,
    ranges: LineRanges,
    lines: &[&str],
    rows: &[usize],
    changes: &mut BTreeMap<usize, Vec<Range<usize>>>,
) -> Result<usize, InvalidComparison> {
    let line = ranges.line as usize;
    let text = lines
        .get(line)
        .ok_or(InvalidComparison::MissingLine { side, line })?;
    for range in ranges.ranges {
        let range = range.start as usize..range.end as usize;
        if text.get(range.clone()).is_none() {
            return Err(InvalidComparison::RangeNotInLine { side, line, range });
        }
        changes.entry(line).or_default().push(range);
    }
    Ok(rows[line])
}

fn one_sided(language: &str, old: &str, new: &str) -> StructuralDiff {
    let old_lines: Vec<_> = old.split_terminator('\n').collect();
    let new_lines: Vec<_> = new.split_terminator('\n').collect();
    let aligned_lines: Vec<_> = (0..old_lines.len())
        .map(|line| (Some(line), None))
        .chain((0..new_lines.len()).map(|line| (None, Some(line))))
        .collect();
    let whole_lines = |lines: &[&str]| -> BTreeMap<usize, Vec<Range<usize>>> {
        lines
            .iter()
            .enumerate()
            .map(|(line, text)| (line, std::iter::once(0..text.len()).collect()))
            .collect()
    };
    StructuralDiff {
        language: language.to_owned(),
        blocks: if aligned_lines.is_empty() {
            Vec::new()
        } else {
            std::iter::once(0..aligned_lines.len()).collect()
        },
        aligned_lines,
        old_changes: whole_lines(&old_lines),
        new_changes: whole_lines(&new_lines),
    }
}

#[cfg(test)]
#[path = "conversion_tests.rs"]
mod tests;
