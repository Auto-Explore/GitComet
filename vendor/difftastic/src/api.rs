use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

use crate::display::json::{self, Status};
use crate::interrupt;
pub use crate::interrupt::Interrupted;
use crate::options::{DiffOptions, DEFAULT_BYTE_LIMIT};
use crate::parse::guess_language::language_name;
use crate::summary::{FileFormat, TextFallbackReason};

pub const BYTE_LIMIT: usize = DEFAULT_BYTE_LIMIT;

#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    pub path: &'a Path,
    pub old: &'a str,
    pub new: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Comparison {
    Unchanged,
    Created { language: &'static str },
    Deleted { language: &'static str },
    Changed(Changes),
    Fallback(Fallback),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fallback {
    UnsupportedLanguage,
    ParseErrors,
    ByteLimit,
    GraphLimit,
    NestingLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Changes {
    pub language: &'static str,
    pub aligned_lines: Vec<AlignedLine>,
    pub chunks: Vec<Vec<ChunkLine>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignedLine {
    Both { old: u32, new: u32 },
    Old(u32),
    New(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChunkLine {
    Both { old: LineRanges, new: LineRanges },
    Old(LineRanges),
    New(LineRanges),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineRanges {
    pub line: u32,
    pub ranges: Vec<Range<u32>>,
}

/// Whether `compare` would parse `input`; when false it returns
/// `Fallback::UnsupportedLanguage` without parsing.
pub fn supports(input: Input<'_>) -> bool {
    let old = with_final_newline(input.old);
    let new = with_final_newline(input.new);
    crate::guess_language_for(input.path, &old, &new).is_some()
}

/// Falls back to `NestingLimit` when either side's tree is deeper than
/// `nesting_limit` tree-sitter levels; an embedded language that would take its
/// host past the limit is diffed with the host grammar instead. The diff
/// recurses up to twice the limit, as a node can gain a wrapper list.
/// `should_stop` is polled during the work and must not panic: tree-sitter
/// calls it from `extern "C"`, where a panic aborts.
pub fn compare(
    input: Input<'_>,
    nesting_limit: usize,
    should_stop: &dyn Fn() -> bool,
) -> Result<Comparison, Interrupted> {
    interrupt::check(should_stop)?;
    let old = with_final_newline(input.old);
    let new = with_final_newline(input.new);
    let options = DiffOptions {
        nesting_limit,
        should_stop,
        ..DiffOptions::default()
    };
    let result = crate::diff_file_content(input.path, &old, &new, &options)?;
    let language = match result.file_format {
        FileFormat::SupportedLanguage(language) => language_name(language),
        FileFormat::PlainText => return Ok(Comparison::Fallback(Fallback::UnsupportedLanguage)),
        FileFormat::TextFallback { reason } => return Ok(Comparison::Fallback(fallback(reason))),
    };
    interrupt::check(should_stop)?;
    let file = json::File::from(&result);
    let lines = LineCounts {
        old: line_count(input.old),
        new: line_count(input.new),
    };
    Ok(match file.status {
        Status::Unchanged => Comparison::Unchanged,
        Status::Created => Comparison::Created { language },
        Status::Deleted => Comparison::Deleted { language },
        Status::Changed => Comparison::Changed(Changes {
            language,
            aligned_lines: file
                .aligned_lines
                .into_iter()
                .filter_map(|(old, new)| lines.aligned_line(old, new))
                .collect(),
            chunks: file
                .chunks
                .into_iter()
                .map(|chunk| -> Vec<ChunkLine> {
                    chunk
                        .into_iter()
                        .filter_map(|line| lines.chunk_line(line))
                        .collect()
                })
                .filter(|chunk| !chunk.is_empty())
                .collect(),
        }),
    })
}

// Upstream counts the empty line after a final newline; the caller's lines stop
// before it.
struct LineCounts {
    old: usize,
    new: usize,
}

impl LineCounts {
    fn aligned_line(&self, old: Option<u32>, new: Option<u32>) -> Option<AlignedLine> {
        let old = old.filter(|line| (*line as usize) < self.old);
        let new = new.filter(|line| (*line as usize) < self.new);
        match (old, new) {
            (Some(old), Some(new)) => Some(AlignedLine::Both { old, new }),
            (Some(old), None) => Some(AlignedLine::Old(old)),
            (None, Some(new)) => Some(AlignedLine::New(new)),
            (None, None) => None,
        }
    }

    fn chunk_line(&self, line: json::Line) -> Option<ChunkLine> {
        let old = line
            .lhs
            .filter(|side| (side.line_number as usize) < self.old);
        let new = line
            .rhs
            .filter(|side| (side.line_number as usize) < self.new);
        match (old, new) {
            (Some(old), Some(new)) => Some(ChunkLine::Both {
                old: line_ranges(old),
                new: line_ranges(new),
            }),
            (Some(old), None) => Some(ChunkLine::Old(line_ranges(old))),
            (None, Some(new)) => Some(ChunkLine::New(line_ranges(new))),
            (None, None) => None,
        }
    }
}

fn line_count(source: &str) -> usize {
    source.split_terminator('\n').count()
}

// Upstream's CLI adds the newline before diffing, so "foo" and "foo\n" compare
// as unchanged.
fn with_final_newline(source: &str) -> Cow<'_, str> {
    if source.is_empty() || source.ends_with('\n') {
        return Cow::Borrowed(source);
    }
    Cow::Owned(format!("{source}\n"))
}

fn fallback(reason: TextFallbackReason) -> Fallback {
    match reason {
        TextFallbackReason::ByteLimit => Fallback::ByteLimit,
        TextFallbackReason::ParseErrors => Fallback::ParseErrors,
        TextFallbackReason::GraphLimit => Fallback::GraphLimit,
        TextFallbackReason::NestingLimit => Fallback::NestingLimit,
    }
}

fn line_ranges(side: json::Side) -> LineRanges {
    LineRanges {
        line: side.line_number,
        ranges: side
            .changes
            .into_iter()
            .map(|change| change.start..change.end)
            .collect(),
    }
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
