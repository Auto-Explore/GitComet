use super::{ConflictRegion, ConflictRegionResolution, ConflictRegionText};
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedConflictBlock {
    pub base: Option<String>,
    pub ours: String,
    pub theirs: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedConflictBlockRanges {
    pub marker_start: usize,
    pub marker_end: usize,
    pub base: Option<Range<usize>>,
    pub ours: Range<usize>,
    pub theirs: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParsedConflictSegment {
    Text(String),
    Conflict(ParsedConflictBlock),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParsedConflictSegmentRanges {
    Text(Range<usize>),
    Conflict(ParsedConflictBlockRanges),
}

fn text_for_range<'a>(text: &'a str, range: &Range<usize>) -> &'a str {
    text.get(range.clone())
        .expect("conflict marker parser produced invalid byte range")
}

struct LineCursor<'a> {
    text: &'a str,
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> LineCursor<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            offset: 0,
        }
    }

    fn next(&mut self) -> Option<(Range<usize>, &'a str)> {
        if self.offset >= self.bytes.len() {
            return None;
        }

        let start = self.offset;
        self.offset = self.text[start..]
            .find('\n')
            .map(|rel| start.saturating_add(rel).saturating_add(1))
            .unwrap_or(self.bytes.len());

        let end = self.offset;
        Some((
            start..end,
            self.text
                .get(start..end)
                .expect("line cursor produced invalid byte range"),
        ))
    }
}

/// Parse merged text into alternating context byte ranges and conflict block
/// byte ranges.
///
/// Parsing is intentionally conservative. If a marker block is malformed, all
/// consumed marker text is preserved as context and parsing continues.
pub fn parse_conflict_marker_ranges(text: &str) -> Vec<ParsedConflictSegmentRanges> {
    let mut segments = Vec::new();
    let mut context_start = 0usize;
    let mut it = LineCursor::new(text);

    while let Some((line_range, line)) = it.next() {
        if !line.as_bytes().starts_with(b"<<<<<<<") {
            continue;
        }

        if context_start < line_range.start {
            segments.push(ParsedConflictSegmentRanges::Text(
                context_start..line_range.start,
            ));
        }

        let marker_start = line_range.start;
        let ours_start = line_range.end;
        let mut ours_end = ours_start;
        let mut separator_range: Option<Range<usize>> = None;
        let mut base_range: Option<Range<usize>> = None;

        while let Some((next_range, next_line)) = it.next() {
            if next_line.as_bytes().starts_with(b"=======") {
                separator_range = Some(next_range.clone());
                ours_end = next_range.start;
                break;
            }

            if next_line.as_bytes().starts_with(b"|||||||") {
                ours_end = next_range.start;
                let base_start = next_range.end;
                let mut base_end = base_start;

                while let Some((base_line_range, base_line)) = it.next() {
                    if base_line.as_bytes().starts_with(b"=======") {
                        separator_range = Some(base_line_range.clone());
                        base_end = base_line_range.start;
                        break;
                    }
                    base_end = base_line_range.end;
                }

                base_range = Some(base_start..base_end);
                break;
            }

            ours_end = next_range.end;
        }

        let Some(separator_range) = separator_range else {
            context_start = marker_start;
            continue;
        };

        let theirs_start = separator_range.end;
        let mut theirs_end = theirs_start;
        let mut marker_end: Option<usize> = None;

        while let Some((theirs_line_range, theirs_line)) = it.next() {
            if theirs_line.as_bytes().starts_with(b">>>>>>>") {
                marker_end = Some(theirs_line_range.end);
                theirs_end = theirs_line_range.start;
                break;
            }
            theirs_end = theirs_line_range.end;
        }

        let Some(marker_end) = marker_end else {
            context_start = marker_start;
            continue;
        };

        segments.push(ParsedConflictSegmentRanges::Conflict(
            ParsedConflictBlockRanges {
                marker_start,
                marker_end,
                base: base_range,
                ours: ours_start..ours_end,
                theirs: theirs_start..theirs_end,
            },
        ));
        context_start = marker_end;
    }

    if context_start < text.len() {
        segments.push(ParsedConflictSegmentRanges::Text(context_start..text.len()));
    }

    segments
}

/// Parse merged text into alternating context text and conflict blocks.
///
/// Parsing is intentionally conservative. If a marker block is malformed,
/// all consumed marker text is preserved as context and parsing continues.
pub fn parse_conflict_marker_segments(text: &str) -> Vec<ParsedConflictSegment> {
    parse_conflict_marker_ranges(text)
        .into_iter()
        .map(|segment| match segment {
            ParsedConflictSegmentRanges::Text(range) => {
                ParsedConflictSegment::Text(text_for_range(text, &range).to_string())
            }
            ParsedConflictSegmentRanges::Conflict(block) => {
                ParsedConflictSegment::Conflict(ParsedConflictBlock {
                    base: block
                        .base
                        .as_ref()
                        .map(|range| text_for_range(text, range).to_string()),
                    ours: text_for_range(text, &block.ours).to_string(),
                    theirs: text_for_range(text, &block.theirs).to_string(),
                })
            }
        })
        .collect()
}

/// Parse conflict marker blocks from merged text into conflict regions.
///
/// This is a thin wrapper over [`parse_conflict_marker_segments`] that
/// discards context text and keeps only conflict blocks.
#[cfg(test)]
pub(super) fn parse_conflict_regions_from_markers(text: &str) -> Vec<ConflictRegion> {
    parse_conflict_regions_from_shared_text(Arc::<str>::from(text))
}

pub(super) fn parse_conflict_regions_from_shared_text(text: Arc<str>) -> Vec<ConflictRegion> {
    parse_conflict_marker_ranges(text.as_ref())
        .into_iter()
        .filter_map(|segment| match segment {
            ParsedConflictSegmentRanges::Text(_) => None,
            ParsedConflictSegmentRanges::Conflict(block) => Some(ConflictRegion {
                base: block
                    .base
                    .map(|range| ConflictRegionText::shared_slice(Arc::clone(&text), range)),
                ours: ConflictRegionText::shared_slice(Arc::clone(&text), block.ours),
                theirs: ConflictRegionText::shared_slice(Arc::clone(&text), block.theirs),
                resolution: ConflictRegionResolution::Unresolved,
            }),
        })
        .collect()
}
