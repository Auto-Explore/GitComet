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
/// Whether `reader` yields a complete conflict block, following the same marker
/// rules as [`parse_conflict_marker_ranges`] but streaming, so the answer comes
/// as soon as the first block closes rather than after reading the whole file.
///
/// `budget_bytes` bounds the read, and with it the memory used: a file with no
/// line breaks in it is one `read_until` call, so the bound has to sit under the
/// reader rather than being checked between lines. Running out of budget having
/// already seen an opening marker answers `true`: for a file the caller already
/// knows git reports as conflicted, markers that do not close within the budget
/// cannot be taken as resolved.
pub fn reader_has_conflict_markers<R: std::io::BufRead>(
    reader: R,
    budget_bytes: u64,
) -> std::io::Result<bool> {
    use std::io::BufRead as _;

    let mut reader = reader.take(budget_bytes);
    // Bytes, not `read_line`: the scan must not fail on a file that is not valid
    // UTF-8, and the markers are ASCII at the start of a line either way.
    let mut line = Vec::new();
    let mut saw_opener = false;
    let mut saw_separator = false;

    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            // Nothing left to read: either the end of the file, where a block
            // left open is malformed and the strict parser treats it as ordinary
            // text, or the budget running out, where an opener that never closed
            // cannot be called resolved.
            return Ok(saw_opener && reader.limit() == 0);
        }

        if line.starts_with(b"<<<<<<<") {
            // Deliberately does not reset `saw_separator`: past the separator the
            // strict parser scans only for a closing marker, so an opener nested
            // in the `theirs` side is content, not the start of a new block.
            saw_opener = true;
        } else if saw_opener && line.starts_with(b"=======") {
            saw_separator = true;
        } else if saw_separator && line.starts_with(b">>>>>>>") {
            return Ok(true);
        }
    }
}

/// Whether `text` still holds at least one complete conflict block, i.e. a merge
/// whose markers were never resolved — or were resolved by hand with the markers
/// left behind. Uses the same conservative parse as the conflict session, so a
/// malformed block reads as ordinary text rather than a false alarm.
pub fn text_has_conflict_markers(text: &str) -> bool {
    parse_conflict_marker_ranges(text)
        .iter()
        .any(|segment| matches!(segment, ParsedConflictSegmentRanges::Conflict(_)))
}

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

#[cfg(test)]
mod marker_detection_tests {
    use super::{reader_has_conflict_markers, text_has_conflict_markers};

    const BUDGET: u64 = 128 * 1024 * 1024;

    fn scan(text: &str) -> bool {
        reader_has_conflict_markers(std::io::BufReader::new(text.as_bytes()), BUDGET).expect("scan")
    }

    #[test]
    fn streamed_scan_agrees_with_the_strict_parser() {
        for text in [
            "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nb\n",
            "a\nours\nb\n",
            "a\n<<<<<<< HEAD\nours\n",
            "a\n<<<<<<< HEAD\nours\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> other\n",
            // Merging a file that already had markers committed to it nests one
            // block inside another's `theirs` side. The strict parser reads the
            // inner opener as content and still closes the outer block, so a
            // scan that restarted on it would miss the whole thing.
            "<<<<<<< a\nours\n=======\ntheirs\n<<<<<<< c\nmore\n>>>>>>> d\n",
            // The same nesting on the `ours` side, which neither scan counts
            // until a separator shows up.
            "<<<<<<< a\n<<<<<<< c\nours\n",
            // A closing marker before any separator closes nothing.
            "<<<<<<< a\nours\n>>>>>>> b\n",
        ] {
            assert_eq!(
                scan(text),
                text_has_conflict_markers(text),
                "streamed and strict scans disagree on {text:?}"
            );
        }
    }

    #[test]
    fn streamed_scan_finds_a_block_spanning_a_large_file() {
        // The conflict of a generated file can span nearly all of it, which is
        // what sizing a file out of the scan used to miss.
        let mut text = String::from("<<<<<<< HEAD\n");
        for i in 0..200_000 {
            text.push_str(&format!("line {i}\n"));
        }
        text.push_str("=======\n");
        for i in 0..200_000 {
            text.push_str(&format!("other {i}\n"));
        }
        text.push_str(">>>>>>> feature\n");
        assert!(text.len() > 4 * 1024 * 1024, "fixture should be multi-MB");
        assert!(scan(&text));
    }

    /// A minified file is a single enormous line, so a budget only checked
    /// between lines would not bound the read at all.
    #[test]
    fn streamed_scan_budget_bounds_a_file_with_no_line_breaks() {
        let mut text = String::from("<<<<<<< HEAD");
        text.push_str(&"x".repeat(4 * 1024 * 1024));

        let mut reader = CountingReader {
            inner: std::io::Cursor::new(text.as_bytes()),
            read: 0,
        };
        // An opener with nothing after it to close it: the caller is warned.
        const BUDGET_BYTES: u64 = 4096;
        const BUFFER_BYTES: usize = 1024;
        assert!(
            reader_has_conflict_markers(
                std::io::BufReader::with_capacity(BUFFER_BYTES, &mut reader),
                BUDGET_BYTES,
            )
            .expect("scan")
        );
        // Bounded by the budget (plus whatever the last buffer refill overshot
        // by), not by the 4MB behind it.
        assert!(
            reader.read <= BUDGET_BYTES as usize + BUFFER_BYTES,
            "the budget must bound the read itself, not just the line count: read {}",
            reader.read
        );
    }

    struct CountingReader<R> {
        inner: R,
        read: usize,
    }

    impl<R: std::io::Read> std::io::Read for CountingReader<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.read += n;
            Ok(n)
        }
    }

    #[test]
    fn streamed_scan_warns_when_the_budget_runs_out_after_an_opener() {
        let text = "<<<<<<< HEAD\nours\nmore\n";
        // Too small to reach a closing marker: an open block cannot be called
        // resolved, so the caller is warned.
        assert!(
            reader_has_conflict_markers(std::io::BufReader::new(text.as_bytes()), 16)
                .expect("scan")
        );
        // Nothing seen at all within the budget stays quiet.
        assert!(
            !reader_has_conflict_markers(std::io::BufReader::new("plain text\n".as_bytes()), 4)
                .expect("scan")
        );
    }

    #[test]
    fn detects_a_complete_conflict_block() {
        assert!(text_has_conflict_markers(
            "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nb\n"
        ));
    }

    #[test]
    fn ignores_resolved_text() {
        assert!(!text_has_conflict_markers("a\nours\nb\n"));
    }

    #[test]
    fn ignores_a_lone_opener() {
        // The parser keeps malformed marker text as context, so an unterminated
        // block must not raise a false alarm.
        assert!(!text_has_conflict_markers("a\n<<<<<<< HEAD\nours\n"));
    }
}
