use super::*;

/// Find the visible index for the first line of a conflict range, or the
/// collapsed block entry. Returns `None` if the range is not visible.
#[cfg(test)]
pub fn visible_index_for_conflict(
    visible_map: &[ThreeWayVisibleItem],
    conflict_ranges: &[std::ops::Range<usize>],
    range_ix: usize,
) -> Option<usize> {
    let range = conflict_ranges.get(range_ix)?;
    visible_map.iter().position(|item| match item {
        ThreeWayVisibleItem::Line(ix) => range.contains(ix),
        ThreeWayVisibleItem::CollapsedBlock(ci) => *ci == range_ix,
        ThreeWayVisibleItem::CollapsedContext { .. } => false,
    })
}

/// When conflict markers use 2-way style (no `|||||||` base section), `block.base`
/// will be `None` even though the git ancestor content (index stage :1:) is available.
/// This function populates `block.base` by using the Text segments as anchors to
/// locate the corresponding base content in the ancestor file.
pub(in crate::view) fn populate_block_bases_from_ancestor_impl(
    segments: &mut [ConflictSegment],
    ancestor_text: &str,
    shared_ancestor_text: Option<&Arc<str>>,
) {
    if ancestor_text.is_empty() {
        return;
    }
    let any_missing = segments
        .iter()
        .any(|s| matches!(s, ConflictSegment::Block(b) if b.base.is_none()));
    if !any_missing {
        return;
    }

    // Find each Text segment's byte position in the ancestor file.
    // Text segments are the non-conflicting parts that exist in all three versions.
    let mut text_byte_ranges: Vec<std::ops::Range<usize>> =
        Vec::with_capacity(segments.len().saturating_add(1) / 2);
    let mut cursor = 0usize;
    for seg in segments.iter() {
        if let ConflictSegment::Text(text) = seg {
            if let Some(rel) = ancestor_text[cursor..].find(text.as_str()) {
                let start = cursor + rel;
                let end = start + text.len();
                text_byte_ranges.push(start..end);
                cursor = end;
            } else {
                // Text not found in ancestor – bail out.
                return;
            }
        }
    }

    // Extract base content for each block from the gaps between text positions.
    let mut text_idx = 0usize;
    let mut prev_end = 0usize;
    for seg in segments.iter_mut() {
        match seg {
            ConflictSegment::Text(_) => {
                prev_end = text_byte_ranges[text_idx].end;
                text_idx += 1;
            }
            ConflictSegment::Block(block) => {
                if block.base.is_some() {
                    continue;
                }
                let next_start = text_byte_ranges
                    .get(text_idx)
                    .map(|r| r.start)
                    .unwrap_or(ancestor_text.len());
                block.base = Some(if let Some(shared_ancestor_text) = shared_ancestor_text {
                    ConflictText::shared_slice(
                        Arc::clone(shared_ancestor_text),
                        prev_end..next_start,
                    )
                } else {
                    ancestor_text[prev_end..next_start].to_string().into()
                });
            }
        }
    }
}

#[cfg(test)]
pub fn populate_block_bases_from_ancestor(segments: &mut [ConflictSegment], ancestor_text: &str) {
    populate_block_bases_from_ancestor_impl(segments, ancestor_text, None);
}

pub fn populate_block_bases_from_shared_ancestor(
    segments: &mut [ConflictSegment],
    ancestor_text: Arc<str>,
) {
    populate_block_bases_from_ancestor_impl(segments, ancestor_text.as_ref(), Some(&ancestor_text));
}

/// Check whether the given text still contains a complete git conflict-marker
/// block. Marker-looking content on its own (for example a Markdown `=======`
/// Setext underline) is not enough to block Save.
pub fn text_contains_conflict_markers(text: &str) -> bool {
    #[derive(Clone, Copy)]
    enum MarkerState {
        Outside,
        Ours,
        Theirs,
    }

    let mut state = MarkerState::Outside;
    for line in text.lines() {
        if line.starts_with("<<<<<<<") {
            state = MarkerState::Ours;
            continue;
        }
        state = match (state, line) {
            (MarkerState::Ours, line) if line.starts_with("=======") => MarkerState::Theirs,
            (MarkerState::Theirs, line) if line.starts_with(">>>>>>>") => return true,
            (current, _) => current,
        };
    }
    false
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConflictStageSafetyCheck {
    pub has_conflict_markers: bool,
    pub unresolved_blocks: usize,
}

impl ConflictStageSafetyCheck {
    pub fn blocks_save(self) -> bool {
        self.has_conflict_markers || self.unresolved_blocks > 0
    }
}

/// Compute stage-safety status for the current conflict resolver output/state.
///
/// This gate is stricter than marker-only checks: unresolved conflict blocks
/// still block the save even if the current output text no longer contains
/// marker lines.
pub fn conflict_stage_safety_check(
    output_text: &str,
    segments: &[ConflictSegment],
    block_map: &ResolvedOutputBlockMap,
) -> ConflictStageSafetyCheck {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

    // The editor is intentionally not synchronized into session state on
    // every keystroke. Derive the effective resolutions from its current
    // contents so a manual replacement can enable Save, which then performs
    // the actual synchronization.
    let unresolved_blocks =
        derive_region_resolution_updates_from_output(segments, &[], block_map, output_text)
            .map(|updates| {
                updates
                    .iter()
                    .filter(|(_, resolution)| {
                        matches!(resolution, ConflictRegionResolution::Unresolved)
                    })
                    .count()
            })
            .unwrap_or_else(|| {
                // Ownership validation failed. Treat every displayed block as
                // unresolved so Save fails closed instead of guessing from
                // repeated context anchors.
                conflict_count(segments)
            });
    ConflictStageSafetyCheck {
        has_conflict_markers: text_contains_conflict_markers(output_text),
        unresolved_blocks,
    }
}

/// What the resolved-output analysis needs to read from the buffer.
///
/// Implemented for `str` and for [`Rope`], so the same code serves callers that
/// already hold a materialized string (a freshly generated output, a test
/// fixture) and the editable buffer, which must not be flattened just to be
/// scanned. Implementing it on `str` rather than `&str` is what keeps every
/// existing caller compiling unchanged.
pub(in crate::view) trait ResolvedOutputSource {
    fn len(&self) -> usize;
    fn is_char_boundary(&self, offset: usize) -> bool;
    /// Whether the text at `offset` begins with `needle`.
    fn starts_with_at(&self, offset: usize, needle: &str) -> bool;
    fn byte_at(&self, offset: usize) -> Option<u8>;
    fn count_newlines_in(&self, range: Range<usize>) -> usize;
    /// Rows, counting the empty one a trailing newline leaves behind.
    fn row_count(&self) -> usize {
        self.count_newlines_in(0..self.len()).saturating_add(1)
    }

    /// Visit every row as `(byte range including its terminator, text)`.
    ///
    /// One pass over the text, so a scan for marker rows costs the document
    /// once rather than a seek per row.
    fn for_each_row_with_terminator(&self, visit: &mut dyn FnMut(Range<usize>, &str));
}

impl ResolvedOutputSource for str {
    fn len(&self) -> usize {
        str::len(self)
    }

    fn is_char_boundary(&self, offset: usize) -> bool {
        str::is_char_boundary(self, offset)
    }

    fn starts_with_at(&self, offset: usize, needle: &str) -> bool {
        self.get(offset..)
            .is_some_and(|tail| tail.starts_with(needle))
    }

    fn byte_at(&self, offset: usize) -> Option<u8> {
        self.as_bytes().get(offset).copied()
    }

    fn count_newlines_in(&self, range: Range<usize>) -> usize {
        // Clamped rather than `get(range)`, which answers `None` — and so 0 —
        // for a span that runs past the end or starts inside a character. Zero
        // is the dangerous answer: callers feed this into block line ranges, so
        // "no newlines here" silently places conflict markers on the wrong
        // rows. [`Rope`] clamps and counts what is really there; the two
        // implementations are used against the same buffers and have to agree.
        let len = str::len(self);
        let mut start = range.start.min(len);
        let mut end = range.end.min(len).max(start);
        // Widening to character boundaries cannot change the count: a newline
        // is ASCII, so it never sits inside a multi-byte character.
        while start > 0 && !self.is_char_boundary(start) {
            start -= 1;
        }
        while end < len && !self.is_char_boundary(end) {
            end += 1;
        }
        memchr::memchr_iter(b'\n', &self.as_bytes()[start..end]).count()
    }

    fn for_each_row_with_terminator(&self, visit: &mut dyn FnMut(Range<usize>, &str)) {
        let mut start = 0usize;
        for newline in memchr::memchr_iter(b'\n', self.as_bytes()) {
            let end = newline + 1;
            visit(start..end, &self[start..end]);
            start = end;
        }
        if start <= str::len(self) {
            visit(start..str::len(self), &self[start..]);
        }
    }
}

impl ResolvedOutputSource for crate::kit::rope::Rope {
    fn len(&self) -> usize {
        crate::kit::rope::Rope::len(self)
    }

    fn is_char_boundary(&self, offset: usize) -> bool {
        crate::kit::rope::Rope::is_char_boundary(self, offset)
    }

    fn starts_with_at(&self, offset: usize, needle: &str) -> bool {
        let end = offset.saturating_add(needle.len());
        if end > self.len() {
            return false;
        }
        // Compared as bytes, and over a boundary-widened chunk range, because
        // this is asked precisely when the buffer may have diverged from the
        // segments: `offset` or `end` can land inside a multi-byte character
        // the user typed. A `&str` comparison would panic there instead of
        // reporting the mismatch this exists to find.
        let mut rest = needle.as_bytes();
        let mut skip = offset - self.clip_offset(offset, gpui::sum_tree::Bias::Left);
        for chunk in self.chunks_in_range(offset..end) {
            let bytes = chunk.as_bytes();
            let bytes = if skip >= bytes.len() {
                skip -= bytes.len();
                continue;
            } else {
                let tail = &bytes[skip..];
                skip = 0;
                tail
            };
            let take = bytes.len().min(rest.len());
            if rest[..take] != bytes[..take] {
                return false;
            }
            rest = &rest[take..];
            if rest.is_empty() {
                return true;
            }
        }
        rest.is_empty()
    }

    fn byte_at(&self, offset: usize) -> Option<u8> {
        if offset >= self.len() {
            return None;
        }
        // A byte read is well defined at every offset, including inside a
        // multi-byte character — trimming a `\r` off a row end lands there
        // whenever the row's last character is not ASCII. Widen to the
        // enclosing character and index the bytes, rather than slicing a `&str`
        // at `offset`, which would panic.
        let start = self.clip_offset(offset, gpui::sum_tree::Bias::Left);
        self.chunks_in_range(
            start..self.clip_offset(offset.saturating_add(1), gpui::sum_tree::Bias::Right),
        )
        .next()
        .and_then(|chunk| chunk.as_bytes().get(offset - start).copied())
    }

    fn count_newlines_in(&self, range: Range<usize>) -> usize {
        // Two summary descents rather than a scan.
        let start = self.offset_to_point(range.start).row;
        let end = self.offset_to_point(range.end.max(range.start)).row;
        end.saturating_sub(start) as usize
    }

    fn row_count(&self) -> usize {
        crate::kit::rope::Rope::line_count(self) as usize
    }

    fn for_each_row_with_terminator(&self, visit: &mut dyn FnMut(Range<usize>, &str)) {
        // One walk over the chunks. A row that straddles a chunk boundary is
        // assembled into `carry` — at most one per chunk, so this allocates in
        // proportion to the chunk count, not the row count.
        let mut row_start = 0usize;
        let mut base = 0usize;
        let mut carry = String::new();
        for chunk in self.chunks() {
            let mut search = 0usize;
            while let Some(found) = memchr::memchr(b'\n', &chunk.as_bytes()[search..]) {
                let newline = search + found;
                let row_end = base + newline + 1;
                if carry.is_empty() {
                    visit(row_start..row_end, &chunk[search..=newline]);
                } else {
                    carry.push_str(&chunk[search..=newline]);
                    visit(row_start..row_end, &carry);
                    carry.clear();
                }
                row_start = row_end;
                search = newline + 1;
            }
            if search < chunk.len() {
                carry.push_str(&chunk[search..]);
            }
            base += chunk.len();
        }
        visit(row_start..base, &carry);
    }
}

/// Row count for a materialized output. Production reads this off
/// [`ResolvedOutputSource::row_count`] instead, which the rope answers from its
/// summary; this remains for tests and benchmarks that hold a `String`.
#[cfg(any(test, feature = "benchmarks"))]
pub fn resolved_output_outline_line_count(output: &str) -> usize {
    memchr::memchr_iter(b'\n', output.as_bytes())
        .count()
        .saturating_add(1)
}

/// Split resolved output into one logical row per newline for outline rendering.
///
/// Uses `split('\n')` so trailing newlines are preserved as a final empty row.
#[cfg(any(test, feature = "benchmarks"))]
pub fn split_output_lines_for_outline(output: &str) -> Vec<String> {
    let mut lines = Vec::with_capacity(resolved_output_outline_line_count(output));
    lines.extend(output.split('\n').map(str::to_string));
    lines
}

#[cfg(test)]
pub fn append_lines_to_output(output: &str, lines: &[String]) -> String {
    if lines.is_empty() {
        return output.to_string();
    }

    let needs_leading_nl = !output.is_empty() && !output.ends_with('\n');
    let extra_len: usize =
        lines.iter().map(|l| l.len()).sum::<usize>() + lines.len() + usize::from(needs_leading_nl);
    let mut out = String::with_capacity(output.len() + extra_len);
    out.push_str(output);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    out.push('\n');
    out
}

// ---------------------------------------------------------------------------
// Provenance mapping: classify resolved output lines as A/B/C/Manual
// ---------------------------------------------------------------------------

/// Source lines from the three input panes, used for provenance matching.
///
/// In three-way mode: A = Base, B = Ours, C = Theirs.
/// In two-way mode: A = Ours (old), B = Theirs (new), C is empty.
#[cfg(any(test, feature = "benchmarks"))]
pub struct SourceLines<'a> {
    pub a: &'a [gpui::SharedString],
    pub b: &'a [gpui::SharedString],
    pub c: &'a [gpui::SharedString],
}

#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) fn build_source_line_lookup<'a>(
    sources: &'a SourceLines<'a>,
) -> FxHashMap<&'a str, (ResolvedLineSource, u32)> {
    let mut lookup = FxHashMap::default();

    // Insert in reverse order so duplicates keep the first line number within a side.
    // Later sides overwrite earlier ones to enforce priority A > B > C.
    for (ix, line) in sources.c.iter().enumerate().rev() {
        lookup.insert(
            line.as_ref(),
            (
                ResolvedLineSource::C,
                u32::try_from(ix + 1).unwrap_or(u32::MAX),
            ),
        );
    }
    for (ix, line) in sources.b.iter().enumerate().rev() {
        lookup.insert(
            line.as_ref(),
            (
                ResolvedLineSource::B,
                u32::try_from(ix + 1).unwrap_or(u32::MAX),
            ),
        );
    }
    for (ix, line) in sources.a.iter().enumerate().rev() {
        lookup.insert(
            line.as_ref(),
            (
                ResolvedLineSource::A,
                u32::try_from(ix + 1).unwrap_or(u32::MAX),
            ),
        );
    }

    lookup
}

pub(in crate::view) fn compute_resolved_line_provenance_from_iter<'a>(
    output_lines: impl Iterator<Item = &'a str>,
    lookup: &FxHashMap<&str, (ResolvedLineSource, u32)>,
) -> Vec<ResolvedLineMeta> {
    let mut result = Vec::new();
    for (out_ix, out_line) in output_lines.enumerate() {
        let (source, input_line) = match lookup.get(out_line).copied() {
            Some((src, line_no)) => (src, Some(line_no)),
            None => (ResolvedLineSource::Manual, None),
        };
        result.push(ResolvedLineMeta {
            output_line: out_ix as u32,
            source,
            input_line,
        });
    }
    result
}

/// Compute per-line provenance metadata for the resolved output.
///
/// Each output line is compared (exact text equality) against every source line
/// in A, B, C. The first match found (priority: A, B, C) wins; if none match
/// the line is labeled `Manual`.
#[cfg(any(test, feature = "benchmarks"))]
pub fn compute_resolved_line_provenance(
    output_lines: &[String],
    sources: &SourceLines<'_>,
) -> Vec<ResolvedLineMeta> {
    let lookup = build_source_line_lookup(sources);
    compute_resolved_line_provenance_from_iter(output_lines.iter().map(String::as_str), &lookup)
}

pub(in crate::view) fn insert_indexed_source_lines<'a>(
    lookup: &mut FxHashMap<&'a str, (ResolvedLineSource, u32)>,
    source: ResolvedLineSource,
    text: &'a str,
    line_starts: &[usize],
) {
    let line_count = indexed_line_count(text, line_starts);
    for line_ix in (0..line_count).rev() {
        if let Some(line) = indexed_line_text(text, line_starts, line_ix) {
            lookup.insert(
                line,
                (
                    source,
                    u32::try_from(line_ix.saturating_add(1)).unwrap_or(u32::MAX),
                ),
            );
        }
    }
}

pub fn compute_resolved_line_provenance_from_text_with_indexed_sources(
    output_text: &str,
    a_text: &str,
    a_line_starts: &[usize],
    b_text: &str,
    b_line_starts: &[usize],
    c_text: &str,
    c_line_starts: &[usize],
) -> Vec<ResolvedLineMeta> {
    let mut lookup = FxHashMap::default();
    insert_indexed_source_lines(&mut lookup, ResolvedLineSource::C, c_text, c_line_starts);
    insert_indexed_source_lines(&mut lookup, ResolvedLineSource::B, b_text, b_line_starts);
    insert_indexed_source_lines(&mut lookup, ResolvedLineSource::A, a_text, a_line_starts);
    compute_resolved_line_provenance_from_iter(output_text.split('\n'), &lookup)
}

pub fn compute_resolved_line_provenance_from_text_two_way_indexed_sources(
    output_text: &str,
    ours_text: &str,
    ours_line_starts: &[usize],
    theirs_text: &str,
    theirs_line_starts: &[usize],
) -> Vec<ResolvedLineMeta> {
    let mut lookup = FxHashMap::default();
    insert_indexed_source_lines(
        &mut lookup,
        ResolvedLineSource::B,
        theirs_text,
        theirs_line_starts,
    );
    insert_indexed_source_lines(
        &mut lookup,
        ResolvedLineSource::A,
        ours_text,
        ours_line_starts,
    );
    compute_resolved_line_provenance_from_iter(output_text.split('\n'), &lookup)
}

// ---------------------------------------------------------------------------
// Dedupe key index: tracks which source lines are present in resolved output
// ---------------------------------------------------------------------------

/// Build the set of `SourceLineKey`s currently represented in the resolved output.
///
/// Used to gate the plus-icon: a source row's plus-icon is hidden when its key
/// is already in this set (preventing duplicate insertion).
#[cfg(test)]
pub fn build_resolved_output_line_sources_index(
    meta: &[ResolvedLineMeta],
    output_lines: &[String],
    view_mode: ConflictResolverViewMode,
) -> FxHashSet<SourceLineKey> {
    let mut index = FxHashSet::with_capacity_and_hasher(meta.len(), Default::default());
    for m in meta {
        if m.source == ResolvedLineSource::Manual {
            continue;
        }
        let Some(line_no) = m.input_line else {
            continue;
        };
        let content = output_lines
            .get(m.output_line as usize)
            .map(|s| s.as_str())
            .unwrap_or("");
        index.insert(SourceLineKey::new(view_mode, m.source, line_no, content));
    }
    index
}

pub fn build_resolved_output_line_sources_index_from_text(
    meta: &[ResolvedLineMeta],
    output_text: &str,
    view_mode: ConflictResolverViewMode,
) -> FxHashSet<SourceLineKey> {
    let mut index = FxHashSet::with_capacity_and_hasher(meta.len(), Default::default());
    for (ix, line) in output_text.split('\n').enumerate() {
        let Some(m) = meta.get(ix) else {
            break;
        };
        if m.source == ResolvedLineSource::Manual {
            continue;
        }
        let Some(line_no) = m.input_line else {
            continue;
        };
        index.insert(SourceLineKey::new(view_mode, m.source, line_no, line));
    }
    index
}

/// Check whether a given source line is already present in the resolved output.
///
/// Returns `true` if the source line's key is in the dedupe index — meaning
/// the plus-icon for that row should be hidden.
#[cfg(test)]
pub fn is_source_line_in_output(
    index: &FxHashSet<SourceLineKey>,
    view_mode: ConflictResolverViewMode,
    side: ResolvedLineSource,
    line_no: u32,
    content: &str,
) -> bool {
    let key = SourceLineKey::new(view_mode, side, line_no, content);
    index.contains(&key)
}

/// Extract a single line from text using pre-computed line starts.
pub(in crate::view) fn line_text_from_starts<'a>(
    text: &'a str,
    line_starts: &[usize],
    line_ix: usize,
) -> &'a str {
    let text_len = text.len();
    let start = line_starts
        .get(line_ix)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    let end = line_starts
        .get(line_ix + 1)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if start >= end {
        return "";
    }
    let slice = text.get(start..end).unwrap_or("");
    slice.strip_suffix('\n').unwrap_or(slice)
}
