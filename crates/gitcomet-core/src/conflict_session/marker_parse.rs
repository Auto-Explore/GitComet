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

/// Reconstruct the contributor files represented by a marker document.
///
/// Ordinary context belongs to both contributors; each conflict contributes
/// its corresponding side. This is useful for proving that provisional marker
/// coordinates still describe the immutable stages loaded later.
pub fn reconstruct_conflict_marker_sides(text: &str) -> (String, String) {
    let mut ours = String::with_capacity(text.len());
    let mut theirs = String::with_capacity(text.len());
    for segment in parse_conflict_marker_ranges(text) {
        match segment {
            ParsedConflictSegmentRanges::Text(range) => {
                let context = text_for_range(text, &range);
                ours.push_str(context);
                theirs.push_str(context);
            }
            ParsedConflictSegmentRanges::Conflict(block) => {
                ours.push_str(text_for_range(text, &block.ours));
                theirs.push_str(text_for_range(text, &block.theirs));
            }
        }
    }
    (ours, theirs)
}

pub(super) fn marker_region_source_ranges(text: &str) -> Vec<super::ConflictRegionSourceRanges> {
    let mut ours_cursor = 0usize;
    let mut theirs_cursor = 0usize;
    let mut base_cursor = 0usize;
    let mut ranges = Vec::new();
    let line_count = |fragment: &str| fragment.lines().count();

    for segment in parse_conflict_marker_ranges(text) {
        match segment {
            ParsedConflictSegmentRanges::Text(range) => {
                let count = line_count(text_for_range(text, &range));
                ours_cursor = ours_cursor.saturating_add(count);
                theirs_cursor = theirs_cursor.saturating_add(count);
                base_cursor = base_cursor.saturating_add(count);
            }
            ParsedConflictSegmentRanges::Conflict(block) => {
                let ours_count = line_count(text_for_range(text, &block.ours));
                let theirs_count = line_count(text_for_range(text, &block.theirs));
                let base_count = block
                    .base
                    .as_ref()
                    .map(|range| line_count(text_for_range(text, range)));
                ranges.push(super::ConflictRegionSourceRanges {
                    base: base_count.map(|count| base_cursor..base_cursor.saturating_add(count)),
                    ours: ours_cursor..ours_cursor.saturating_add(ours_count),
                    theirs: theirs_cursor..theirs_cursor.saturating_add(theirs_count),
                });
                ours_cursor = ours_cursor.saturating_add(ours_count);
                theirs_cursor = theirs_cursor.saturating_add(theirs_count);
                if let Some(base_count) = base_count {
                    base_cursor = base_cursor.saturating_add(base_count);
                }
            }
        }
    }
    ranges
}

fn text_for_range<'a>(text: &'a str, range: &Range<usize>) -> &'a str {
    text.get(range.clone())
        .expect("conflict marker parser produced invalid byte range")
}

struct LineCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> LineCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn next(&mut self) -> Option<(Range<usize>, &'a [u8])> {
        if self.offset >= self.bytes.len() {
            return None;
        }
        let start = self.offset;
        self.offset = memchr::memchr(b'\n', &self.bytes[start..])
            .map_or(self.bytes.len(), |relative| start + relative + 1);
        Some((start..self.offset, &self.bytes[start..self.offset]))
    }
}

/// Git's default `conflict-marker-size`, and the shortest marker read back.
const MIN_MARKER_SIZE: usize = 7;

/// The length of the run of `marker` that starts `line`, when the run is a
/// conflict marker: Git follows one with a space (and label) or ends the line.
pub(crate) fn conflict_marker_run(line: &[u8], marker: u8) -> Option<usize> {
    let run = line.iter().take_while(|&&byte| byte == marker).count();
    (run > 0 && matches!(line.get(run), None | Some(b' ' | b'\t' | b'\r' | b'\n'))).then_some(run)
}

/// The marker size of the conflicts in `text`: the longest marker length that
/// closes a complete block.
///
/// Git writes markers of exactly `conflict-marker-size` characters (7 unless
/// configured). Content can hold marker-like lines of other lengths, such as
/// a Markdown `=======` underline or a decorative `<<<<<<<<<<`, so a block is
/// only read from lines of one length. GitComet's own marker projections use
/// a size longer than any run in the content, which makes them win here.
///
/// One pass with a state per length, so no input makes this quadratic.
fn conflict_marker_size(text: &[u8]) -> Option<usize> {
    #[derive(Clone, Copy, PartialEq)]
    enum Phase {
        Opened,
        Separated,
    }
    let mut phases = rustc_hash::FxHashMap::<usize, Phase>::default();
    let mut longest = None;
    let mut it = LineCursor::new(text);
    while let Some((_, line)) = it.next() {
        match line.first() {
            Some(b'<') => {
                if let Some(size) =
                    conflict_marker_run(line, b'<').filter(|size| *size >= MIN_MARKER_SIZE)
                {
                    // An opener inside an open block is content, so keep its phase.
                    phases.entry(size).or_insert(Phase::Opened);
                }
            }
            Some(b'=') => {
                if let Some(phase) =
                    conflict_marker_run(line, b'=').and_then(|size| phases.get_mut(&size))
                    && *phase == Phase::Opened
                {
                    *phase = Phase::Separated;
                }
            }
            Some(b'>') => {
                if let Some(size) = conflict_marker_run(line, b'>')
                    && phases.get(&size) == Some(&Phase::Separated)
                {
                    phases.remove(&size);
                    longest = longest.max(Some(size));
                }
            }
            _ => {}
        }
    }
    longest
}

/// Parse merged text into alternating context byte ranges and conflict block
/// byte ranges.
///
/// Parsing is intentionally conservative. If a marker block is malformed, all
/// consumed marker text is preserved as context and parsing continues.
/// Marker lines are exactly as long as the text's marker size; see
/// `conflict_marker_size`.
pub fn parse_conflict_marker_ranges(text: &str) -> Vec<ParsedConflictSegmentRanges> {
    parse_conflict_marker_ranges_bytes(text.as_bytes())
}

/// The same ASCII marker grammar over original bytes, before text decoding.
pub fn parse_conflict_marker_ranges_bytes(text: &[u8]) -> Vec<ParsedConflictSegmentRanges> {
    // With no complete block, the first opener still marks where the
    // malformed text begins, as a segment of its own.
    let first_opener_size = || {
        let mut it = LineCursor::new(text);
        std::iter::from_fn(|| it.next()).find_map(|(_, line)| {
            conflict_marker_run(line, b'<').filter(|size| *size >= MIN_MARKER_SIZE)
        })
    };
    let Some(size) = conflict_marker_size(text).or_else(first_opener_size) else {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![ParsedConflictSegmentRanges::Text(0..text.len())]
        };
    };
    let is_marker = |line: &[u8], marker: u8| conflict_marker_run(line, marker) == Some(size);
    let mut segments = Vec::new();
    let mut context_start = 0usize;
    let mut it = LineCursor::new(text);

    while let Some((line_range, line)) = it.next() {
        if !is_marker(line, b'<') {
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
            if is_marker(next_line, b'=') {
                separator_range = Some(next_range.clone());
                ours_end = next_range.start;
                break;
            }

            if is_marker(next_line, b'|') {
                ours_end = next_range.start;
                let base_start = next_range.end;
                let mut base_end = base_start;

                while let Some((base_line_range, base_line)) = it.next() {
                    if is_marker(base_line, b'=') {
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
            if is_marker(theirs_line, b'>') {
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
    // Per marker length, as in `conflict_marker_size`: `true` once separated.
    let mut separated = rustc_hash::FxHashMap::<usize, bool>::default();

    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            // Nothing left to read: either the end of the file, where a block
            // left open is malformed and the strict parser treats it as ordinary
            // text, or the budget running out, where an opener that never closed
            // cannot be called resolved.
            return Ok(!separated.is_empty() && reader.limit() == 0);
        }

        if let Some(size) = conflict_marker_run(&line, b'<').filter(|size| *size >= MIN_MARKER_SIZE)
        {
            // Deliberately does not reset a separated block: past the separator
            // the strict parser scans only for a closing marker, so an opener
            // nested in the `theirs` side is content, not the start of a block.
            separated.entry(size).or_insert(false);
        } else if let Some(state) =
            conflict_marker_run(&line, b'=').and_then(|size| separated.get_mut(&size))
        {
            *state = true;
        } else if conflict_marker_run(&line, b'>')
            .is_some_and(|size| separated.get(&size) == Some(&true))
        {
            return Ok(true);
        }
    }
}

pub fn worktree_file_has_conflict_markers(path: &std::path::Path) -> std::io::Result<bool> {
    const MAX_SCANNED_BYTES: u64 = 16 * 1024 * 1024;
    const BUFFER_BYTES: usize = 64 * 1024;
    match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    let opened = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(std::fs::File::from)
    .map_err(std::io::Error::from);
    #[cfg(not(unix))]
    let opened = std::fs::File::open(path);
    let file = match opened {
        Ok(file) if file.metadata()?.is_file() => file,
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    reader_has_conflict_markers(
        std::io::BufReader::with_capacity(BUFFER_BYTES, file),
        MAX_SCANNED_BYTES,
    )
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
            // Marker lines are exactly as long as their opener.
            "<<<<<<< a\nx\n========\ny\n=======\nz\n>>>>>>> b\n",
            "<<<<<<<<<< decoration\ntext\n<<<<<<< a\nx\n=======\ny\n>>>>>>> b\n",
            "<<<<<<<<<< a\nx\n=======\ny\n>>>>>>> b\n",
            "<<<<<<<<<< a\nx\n==========\ny\n>>>>>>>>>> b\n",
        ] {
            assert_eq!(
                scan(text),
                text_has_conflict_markers(text),
                "streamed and strict scans disagree on {text:?}"
            );
        }
    }

    #[test]
    fn marker_lines_must_be_as_long_as_their_opener() {
        let text = "<<<<<<< a\nx\n========\ny\n=======\nz\n>>>>>>> b\n";
        let segments = super::parse_conflict_marker_segments(text);
        let [super::ParsedConflictSegment::Conflict(block)] = segments.as_slice() else {
            panic!("expected one block: {segments:?}");
        };
        assert_eq!(block.ours, "x\n========\ny\n");
        assert_eq!(block.theirs, "z\n");
    }

    #[test]
    fn an_unclosed_longer_opener_does_not_hide_a_later_conflict() {
        let text = "<<<<<<<<<< decoration\ntext\n<<<<<<< a\nx\n=======\ny\n>>>>>>> b\n";
        let segments = super::parse_conflict_marker_segments(text);
        assert_eq!(
            segments,
            vec![
                super::ParsedConflictSegment::Text("<<<<<<<<<< decoration\ntext\n".into()),
                super::ParsedConflictSegment::Conflict(super::ParsedConflictBlock {
                    base: None,
                    ours: "x\n".into(),
                    theirs: "y\n".into(),
                }),
            ]
        );
    }

    /// `conflict-marker-size=10` in `.gitattributes`: Git writes ten.
    #[test]
    fn a_configured_marker_size_is_read_from_the_markers() {
        let text = "<<<<<<<<<< ours\na\n=======\n==========\nb\n>>>>>>>>>> theirs\n";
        let segments = super::parse_conflict_marker_segments(text);
        let [super::ParsedConflictSegment::Conflict(block)] = segments.as_slice() else {
            panic!("expected one block: {segments:?}");
        };
        assert_eq!(block.ours, "a\n=======\n");
        assert_eq!(block.theirs, "b\n");
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
