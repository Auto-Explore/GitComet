use gitcomet_core::diff_engine::{
    AlignedLine, DiffEngineFallback, StructuralDiff, StructuralDiffOutcome,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Debug, thiserror::Error)]
pub enum DifftasticError {
    #[error("invalid Difftastic output: {0}")]
    InvalidOutput(String),
    #[error("Difftastic input/output failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Difftastic comparison was cancelled")]
    Cancelled,
    #[error("Difftastic exceeded its time limit")]
    Timeout,
    #[error("Difftastic exceeded its output limit")]
    OutputLimit,
    #[error("Difftastic exited unsuccessfully ({status:?}): {stderr}")]
    Exit { status: Option<i32>, stderr: String },
    #[error("the bundled Difftastic helper is unavailable")]
    Unavailable,
    #[error("the bundled Difftastic version is incompatible")]
    Version,
}

impl DifftasticError {
    pub fn fallback(&self) -> DiffEngineFallback {
        match self {
            Self::Timeout => DiffEngineFallback::Timeout,
            Self::Unavailable | Self::Version => DiffEngineFallback::Unavailable,
            Self::InvalidOutput(_) | Self::OutputLimit => DiffEngineFallback::InvalidOutput,
            Self::Io(_) | Self::Cancelled | Self::Exit { .. } => DiffEngineFallback::Failed,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Changed,
    Unchanged,
    Created,
    Deleted,
}

#[derive(Deserialize)]
struct File {
    language: String,
    status: Status,
    #[serde(default)]
    aligned_lines: Vec<AlignedLine>,
    #[serde(default)]
    chunks: Vec<Vec<Line>>,
}

#[derive(Deserialize)]
struct Line {
    lhs: Option<Side>,
    rhs: Option<Side>,
}

#[derive(Deserialize)]
struct Side {
    line_number: usize,
    changes: Vec<Change>,
}

#[derive(Deserialize)]
struct Change {
    start: usize,
    end: usize,
    content: String,
}

fn invalid(message: &str) -> DifftasticError {
    DifftasticError::InvalidOutput(message.into())
}

pub fn decode(
    output: &[u8],
    old: &str,
    new: &str,
) -> Result<StructuralDiffOutcome, DifftasticError> {
    let file: File = serde_json::from_slice(output)
        .map_err(|error| DifftasticError::InvalidOutput(error.to_string()))?;
    if let Some(reason) = language_fallback(&file.language) {
        return Ok(StructuralDiffOutcome::Fallback(reason));
    }
    match file.status {
        Status::Unchanged => return Ok(StructuralDiffOutcome::Unchanged),
        Status::Created if !old.is_empty() => {
            return Err(invalid("created file has an old source"));
        }
        Status::Deleted if !new.is_empty() => return Err(invalid("deleted file has a new source")),
        Status::Created | Status::Deleted => {
            return Ok(StructuralDiffOutcome::Compared(one_sided(
                file.language,
                old,
                new,
            )));
        }
        Status::Changed => {}
    }
    let old_lines: Vec<_> = old.split_terminator('\n').collect();
    let new_lines: Vec<_> = new.split_terminator('\n').collect();
    let aligned_lines = validate_alignment(file.aligned_lines, old_lines.len(), new_lines.len())?;
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
        language: file.language,
        aligned_lines,
        old_changes: BTreeMap::new(),
        new_changes: BTreeMap::new(),
        blocks: Vec::new(),
    };
    for chunk in file.chunks {
        let mut bounds: Option<Range<usize>> = None;
        for line in chunk {
            let left_row = add_side(line.lhs, &old_lines, &old_rows, &mut diff.old_changes)?;
            let right_row = add_side(line.rhs, &new_lines, &new_rows, &mut diff.new_changes)?;
            if let (Some(left), Some(right)) = (left_row, right_row)
                && left != right
            {
                return Err(invalid("chunk sides disagree with line alignment"));
            }
            for row in left_row.into_iter().chain(right_row) {
                match &mut bounds {
                    Some(bounds) => {
                        bounds.start = bounds.start.min(row);
                        bounds.end = bounds.end.max(row + 1);
                    }
                    None => bounds = Some(row..row + 1),
                }
            }
        }
        if let Some(bounds) = bounds {
            diff.blocks.push(bounds);
        }
    }
    if diff.old_changes.is_empty() && diff.new_changes.is_empty() {
        return Err(invalid("changed comparison contains no changed ranges"));
    }
    diff.blocks.sort_by_key(|block| block.start);
    for changes in diff
        .old_changes
        .values_mut()
        .chain(diff.new_changes.values_mut())
    {
        changes.sort_by_key(|range| (range.start, range.end));
        changes.dedup();
    }
    Ok(StructuralDiffOutcome::Compared(diff))
}

fn language_fallback(language: &str) -> Option<DiffEngineFallback> {
    match language {
        "Text" => Some(DiffEngineFallback::UnsupportedLanguage),
        "Binary" => Some(DiffEngineFallback::Binary),
        value if value.starts_with("Text (") => {
            let reason = value.to_ascii_lowercase();
            Some(if reason.contains("graph") {
                DiffEngineFallback::GraphLimit
            } else if reason.contains("byte") || reason.contains("size") {
                DiffEngineFallback::ByteLimit
            } else if reason.contains("parse") || reason.contains("syntax") {
                DiffEngineFallback::ParseErrors
            } else {
                DiffEngineFallback::Failed
            })
        }
        _ => None,
    }
}

fn validate_alignment(
    alignment: Vec<AlignedLine>,
    old_count: usize,
    new_count: usize,
) -> Result<Vec<AlignedLine>, DifftasticError> {
    let mut old_cursor = 0;
    let mut new_cursor = 0;
    let mut result = Vec::with_capacity(alignment.len());
    for (old, new) in alignment {
        if old.is_none() && new.is_none() {
            return Err(invalid("alignment row has no source"));
        }
        let old = validate_line(old, &mut old_cursor, old_count)?;
        let new = validate_line(new, &mut new_cursor, new_count)?;
        if old.is_some() || new.is_some() {
            result.push((old, new));
        }
    }
    if old_cursor < old_count || new_cursor < new_count {
        return Err(invalid("alignment omits source lines"));
    }
    Ok(result)
}

fn validate_line(
    line: Option<usize>,
    cursor: &mut usize,
    count: usize,
) -> Result<Option<usize>, DifftasticError> {
    let Some(line) = line else { return Ok(None) };
    if line != *cursor || line > count {
        return Err(invalid("alignment source lines are not consecutive"));
    }
    *cursor += 1;
    Ok((line < count).then_some(line))
}

fn add_side(
    side: Option<Side>,
    lines: &[&str],
    rows: &[usize],
    changes: &mut BTreeMap<usize, Vec<Range<usize>>>,
) -> Result<Option<usize>, DifftasticError> {
    let Some(side) = side else { return Ok(None) };
    let text = lines
        .get(side.line_number)
        .ok_or_else(|| invalid("change references a missing line"))?;
    for change in side.changes {
        if text.get(change.start..change.end) != Some(change.content.as_str()) {
            return Err(invalid("change range does not match source bytes"));
        }
        changes
            .entry(side.line_number)
            .or_default()
            .push(change.start..change.end);
    }
    Ok(Some(rows[side.line_number]))
}

fn one_sided(language: String, old: &str, new: &str) -> StructuralDiff {
    let old_lines: Vec<_> = old.split_terminator('\n').collect();
    let new_lines: Vec<_> = new.split_terminator('\n').collect();
    let aligned_lines: Vec<_> = (0..old_lines.len())
        .map(|line| (Some(line), None))
        .chain((0..new_lines.len()).map(|line| (None, Some(line))))
        .collect();
    let blocks = if aligned_lines.is_empty() {
        Vec::new()
    } else {
        std::iter::once(0..aligned_lines.len()).collect()
    };
    StructuralDiff {
        language,
        aligned_lines,
        old_changes: old_lines
            .iter()
            .enumerate()
            .map(|(line, text)| (line, std::iter::once(0..text.len()).collect()))
            .collect(),
        new_changes: new_lines
            .iter()
            .enumerate()
            .map(|(line, text)| (line, std::iter::once(0..text.len()).collect()))
            .collect(),
        blocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_preserves_structural_alignment_and_utf8_ranges() {
        let old = "let café = 1;\n";
        let new = "let café = wrap(1);\n";
        let output = br#"{"language":"Rust","status":"changed","aligned_lines":[[0,0],[1,1]],"chunks":[[{"lhs":{"line_number":0,"changes":[]},"rhs":{"line_number":0,"changes":[{"start":12,"end":17,"content":"wrap("},{"start":18,"end":19,"content":")"}]}}]]}"#;
        let StructuralDiffOutcome::Compared(diff) = decode(output, old, new).unwrap() else {
            panic!("expected structural comparison");
        };
        assert_eq!(diff.aligned_lines, vec![(Some(0), Some(0))]);
        assert!(diff.old_changes.is_empty());
        assert_eq!(diff.new_changes[&0], vec![12..17, 18..19]);
        assert_eq!(diff.blocks, vec![0..1]);
    }

    #[test]
    fn decode_rejects_spans_inside_utf8_characters() {
        let output = br#"{"language":"Rust","status":"changed","aligned_lines":[[0,0]],"chunks":[[{"rhs":{"line_number":0,"changes":[{"start":1,"end":2,"content":"x"}]}}]]}"#;
        assert!(decode(output, "a", "é").is_err());
    }

    #[test]
    fn decode_rejects_incomplete_and_repeated_source_alignment() {
        for alignment in [
            "[[0,0]]",
            "[[0,0],[0,1]]",
            "[[1,0],[0,1]]",
            "[[0,0],[null,null],[1,1]]",
        ] {
            let output = format!(
                r#"{{"language":"Rust","status":"changed","aligned_lines":{alignment},"chunks":[[{{"rhs":{{"line_number":0,"changes":[{{"start":0,"end":1,"content":"c"}}]}}}}]]}}"#
            );
            assert!(
                decode(output.as_bytes(), "a\nb\n", "c\nd\n").is_err(),
                "{alignment}"
            );
        }
    }

    #[test]
    fn decode_keeps_formatting_only_changes_distinct_from_byte_equality() {
        let output = br#"{"language":"Rust","status":"unchanged"}"#;
        assert_eq!(
            decode(output, "f(a);", "f( a );\n").unwrap(),
            StructuralDiffOutcome::Unchanged
        );
    }

    #[test]
    fn decode_preserves_carriage_returns_and_missing_newline_coordinates() {
        let output = br#"{"language":"Rust","status":"changed","aligned_lines":[[0,0],[1,1]],"chunks":[[{"rhs":{"line_number":0,"changes":[{"start":0,"end":1,"content":"b"}]}}]]}"#;
        let StructuralDiffOutcome::Compared(diff) = decode(output, "a\r\n", "b\r").unwrap() else {
            panic!("expected structural comparison")
        };
        assert_eq!(diff.aligned_lines, vec![(Some(0), Some(0))]);
        assert_eq!(diff.new_changes[&0], vec![0..1]);
    }

    #[test]
    fn decode_created_files_retains_empty_lines() {
        let StructuralDiffOutcome::Compared(diff) =
            decode(br#"{"language":"Rust","status":"created"}"#, "", "a\n\n").unwrap()
        else {
            panic!("expected structural comparison")
        };
        assert_eq!(diff.aligned_lines, vec![(None, Some(0)), (None, Some(1))]);
        assert_eq!(diff.new_changes[&1], vec![0..0]);
        assert_eq!(diff.blocks, vec![0..2]);
    }

    #[test]
    fn decode_reports_upstream_fallback_reasons() {
        for (language, expected) in [
            ("Text", DiffEngineFallback::UnsupportedLanguage),
            ("Binary", DiffEngineFallback::Binary),
            (
                "Text (exceeded DFT_GRAPH_LIMIT)",
                DiffEngineFallback::GraphLimit,
            ),
            (
                "Text (1 MB exceeded DFT_BYTE_LIMIT)",
                DiffEngineFallback::ByteLimit,
            ),
            (
                "Text (1 Rust parse error, exceeded DFT_PARSE_ERROR_LIMIT)",
                DiffEngineFallback::ParseErrors,
            ),
        ] {
            let output = format!(r#"{{"language":"{language}","status":"changed"}}"#);
            assert_eq!(
                decode(output.as_bytes(), "a", "b").unwrap(),
                StructuralDiffOutcome::Fallback(expected)
            );
        }
    }
}
