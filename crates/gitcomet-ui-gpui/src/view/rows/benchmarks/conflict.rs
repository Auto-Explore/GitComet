use super::*;
use crate::view::conflict_resolver::{
    self, ConflictBlock, ConflictChoice, ConflictPickSide, ConflictSegment,
    ConflictSplitStyledTextCache, ThreeWayVisibleItem, TwoWayWordHighlights, WordHighlights,
};
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
use gitcomet_core::mergetool_trace::{
    self, MergetoolTraceEvent, MergetoolTraceRenderingMode, MergetoolTraceSideStats,
    MergetoolTraceStage,
};
use gitcomet_state::model::ConflictFile;
use gitcomet_state::msg::RepoPath;
use std::path::PathBuf;
use std::time::Instant;

fn word_ranges_for_line(highlights: &WordHighlights, line_ix: usize) -> &[Range<usize>] {
    highlights
        .get(&line_ix)
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}

fn two_way_word_ranges_for_row(
    highlights: &TwoWayWordHighlights,
    row_ix: usize,
) -> (&[Range<usize>], &[Range<usize>]) {
    highlights
        .get(row_ix)
        .map(|(old, new)| (old.as_slice(), new.as_slice()))
        .unwrap_or((&[], &[]))
}

pub struct ConflictThreeWayScrollFixture {
    base_lines: Vec<SharedString>,
    ours_lines: Vec<SharedString>,
    theirs_lines: Vec<SharedString>,
    base_word_highlights: WordHighlights,
    ours_word_highlights: WordHighlights,
    theirs_word_highlights: WordHighlights,
    line_conflict_maps: [Vec<Option<usize>>; 3],
    visible_map: Vec<ThreeWayVisibleItem>,
    conflict_count: usize,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    theme: AppTheme,
    base_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ours_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    theirs_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
}

impl ConflictThreeWayScrollFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        Self::build(lines, conflict_blocks, false)
    }

    pub fn new_with_prepared_documents(lines: usize, conflict_blocks: usize) -> Self {
        Self::build(lines, conflict_blocks, true)
    }

    fn build(lines: usize, conflict_blocks: usize, prepare_documents: bool) -> Self {
        let theme = AppTheme::gitcomet_dark();
        let segments = build_synthetic_three_way_segments(lines, conflict_blocks);
        let (base_text, ours_text, theirs_text) = materialize_three_way_side_texts(&segments);
        let base_lines = split_lines_shared(&base_text);
        let ours_lines = split_lines_shared(&ours_text);
        let theirs_lines = split_lines_shared(&theirs_text);
        let base_line_starts = line_starts_for_text(&base_text);
        let ours_line_starts = line_starts_for_text(&ours_text);
        let theirs_line_starts = line_starts_for_text(&theirs_text);
        let three_way_len = base_lines
            .len()
            .max(ours_lines.len())
            .max(theirs_lines.len());
        let conflict_maps = conflict_resolver::build_three_way_conflict_maps(
            &segments,
            base_lines.len(),
            ours_lines.len(),
            theirs_lines.len(),
        );
        let visible_map = conflict_resolver::build_three_way_visible_map(
            three_way_len,
            &conflict_maps.conflict_ranges[1],
            &segments,
            false,
        );
        let (base_word_highlights, ours_word_highlights, theirs_word_highlights) =
            conflict_resolver::compute_three_way_word_highlights(
                &base_text,
                &base_line_starts,
                &ours_text,
                &ours_line_starts,
                &theirs_text,
                &theirs_line_starts,
                &segments,
            );
        let language = diff_syntax_language_for_path("src/conflict.rs");

        let (base_document, ours_document, theirs_document) = if prepare_documents {
            let lang = language.unwrap_or(DiffSyntaxLanguage::Rust);
            let budget = DiffSyntaxBudget::default();
            (
                prepare_bench_diff_syntax_document(lang, budget, &base_text, None),
                prepare_bench_diff_syntax_document(lang, budget, &ours_text, None),
                prepare_bench_diff_syntax_document(lang, budget, &theirs_text, None),
            )
        } else {
            (None, None, None)
        };

        Self {
            base_lines,
            ours_lines,
            theirs_lines,
            base_word_highlights,
            ours_word_highlights,
            theirs_word_highlights,
            line_conflict_maps: conflict_maps.line_conflict_maps,
            visible_map,
            conflict_count: conflict_maps.conflict_ranges[1].len(),
            language,
            // The mergetool fallback path now always uses bounded per-line Auto
            // syntax until any background-prepared document is ready.
            syntax_mode: DiffSyntaxMode::Auto,
            theme,
            base_document,
            ours_document,
            theirs_document,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        if self.visible_map.is_empty() || window == 0 {
            return 0;
        }
        let start = start % self.visible_map.len();
        let end = (start + window).min(self.visible_map.len());

        let mut h = FxHasher::default();
        for visible_item in &self.visible_map[start..end] {
            let line_ix = match *visible_item {
                ThreeWayVisibleItem::Line(ix) => ix,
                ThreeWayVisibleItem::CollapsedBlock(conflict_ix) => {
                    conflict_ix.hash(&mut h);
                    continue;
                }
            };

            for map in &self.line_conflict_maps {
                map.get(line_ix).copied().flatten().hash(&mut h);
            }

            for (lines, highlights) in [
                (&self.base_lines, &self.base_word_highlights),
                (&self.ours_lines, &self.ours_word_highlights),
                (&self.theirs_lines, &self.theirs_word_highlights),
            ] {
                if let Some(line) = lines.get(line_ix) {
                    let styled = super::diff_text::build_cached_diff_styled_text(
                        self.theme,
                        line.as_ref(),
                        word_ranges_for_line(highlights, line_ix),
                        "",
                        self.language,
                        self.syntax_mode,
                        None,
                    );
                    styled.text_hash.hash(&mut h);
                    styled.highlights_hash.hash(&mut h);
                }
            }
        }

        h.finish()
    }

    pub fn visible_rows(&self) -> usize {
        self.visible_map.len()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_count
    }

    /// Scroll step using prepared-document syntax rendering for each side.
    /// This exercises the post-background-parse rendering path that the real
    /// conflict resolver uses once tree-sitter documents are ready.
    pub fn run_prepared_scroll_step(&self, start: usize, window: usize) -> u64 {
        if self.visible_map.is_empty() || window == 0 {
            return 0;
        }
        let start = start % self.visible_map.len();
        let end = (start + window).min(self.visible_map.len());

        let syntax_config = super::diff_text::DiffSyntaxConfig {
            language: self.language,
            mode: DiffSyntaxMode::Auto,
        };

        let mut h = FxHasher::default();
        for visible_item in &self.visible_map[start..end] {
            let line_ix = match *visible_item {
                ThreeWayVisibleItem::Line(ix) => ix,
                ThreeWayVisibleItem::CollapsedBlock(conflict_ix) => {
                    conflict_ix.hash(&mut h);
                    continue;
                }
            };

            for map in &self.line_conflict_maps {
                map.get(line_ix).copied().flatten().hash(&mut h);
            }

            for (lines, highlights, document) in [
                (
                    &self.base_lines,
                    &self.base_word_highlights,
                    self.base_document,
                ),
                (
                    &self.ours_lines,
                    &self.ours_word_highlights,
                    self.ours_document,
                ),
                (
                    &self.theirs_lines,
                    &self.theirs_word_highlights,
                    self.theirs_document,
                ),
            ] {
                if let Some(line) = lines.get(line_ix) {
                    let prepared_line =
                        super::diff_text::PreparedDiffSyntaxLine { document, line_ix };
                    let result =
                        super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                            self.theme,
                            line.as_ref(),
                            word_ranges_for_line(highlights, line_ix),
                            "",
                            syntax_config,
                            None,
                            prepared_line,
                        );
                    let (styled, is_pending) = result.into_parts();
                    is_pending.hash(&mut h);
                    styled.text_hash.hash(&mut h);
                    styled.highlights_hash.hash(&mut h);
                }
            }
        }

        h.finish()
    }

    #[cfg(test)]
    pub(super) fn syntax_mode(&self) -> DiffSyntaxMode {
        self.syntax_mode
    }

    #[cfg(test)]
    pub(super) fn has_prepared_documents(&self) -> bool {
        self.base_document.is_some()
            && self.ours_document.is_some()
            && self.theirs_document.is_some()
    }
}

fn hash_three_way_visible_map_items(items: &[ThreeWayVisibleItem]) -> u64 {
    let mut h = FxHasher::default();
    items.len().hash(&mut h);

    let mut hash_item = |item: &ThreeWayVisibleItem| match *item {
        ThreeWayVisibleItem::Line(ix) => {
            0u8.hash(&mut h);
            ix.hash(&mut h);
        }
        ThreeWayVisibleItem::CollapsedBlock(conflict_ix) => {
            1u8.hash(&mut h);
            conflict_ix.hash(&mut h);
        }
    };

    if let Some(first) = items.first() {
        hash_item(first);
    }
    if let Some(mid) = items.get(items.len() / 2) {
        hash_item(mid);
    }
    if let Some(last) = items.last() {
        hash_item(last);
    }

    h.finish()
}

fn build_three_way_visible_map_legacy(
    total_lines: usize,
    conflict_ranges: &[Range<usize>],
    conflict_resolved: &[bool],
    hide_resolved: bool,
) -> Vec<ThreeWayVisibleItem> {
    if !hide_resolved {
        return (0..total_lines).map(ThreeWayVisibleItem::Line).collect();
    }

    let mut visible = Vec::with_capacity(total_lines);
    let mut line = 0usize;
    while line < total_lines {
        if let Some((range_ix, range)) = conflict_ranges
            .iter()
            .enumerate()
            .find(|(_, r)| r.contains(&line))
            .filter(|(ri, _)| conflict_resolved.get(*ri).copied().unwrap_or(false))
        {
            visible.push(ThreeWayVisibleItem::CollapsedBlock(range_ix));
            line = range.end;
            continue;
        }
        visible.push(ThreeWayVisibleItem::Line(line));
        line += 1;
    }
    visible
}

pub struct ConflictThreeWayVisibleMapBuildFixture {
    total_lines: usize,
    conflict_ranges: Vec<Range<usize>>,
    conflict_resolved: Vec<bool>,
    segments: Vec<ConflictSegment>,
    conflict_count: usize,
}

impl ConflictThreeWayVisibleMapBuildFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let segments = build_synthetic_three_way_segments(lines, conflict_blocks);
        let (base_text, ours_text, theirs_text) = materialize_three_way_side_texts(&segments);
        let base_lines = split_lines_shared(&base_text);
        let ours_lines = split_lines_shared(&ours_text);
        let theirs_lines = split_lines_shared(&theirs_text);
        let total_lines = base_lines
            .len()
            .max(ours_lines.len())
            .max(theirs_lines.len());
        let conflict_maps = conflict_resolver::build_three_way_conflict_maps(
            &segments,
            base_lines.len(),
            ours_lines.len(),
            theirs_lines.len(),
        );
        let [_base_ranges, ours_ranges, _theirs_ranges] = conflict_maps.conflict_ranges;
        let conflict_resolved = conflict_maps.conflict_resolved;
        let conflict_count = ours_ranges.len();

        Self {
            total_lines,
            conflict_ranges: ours_ranges,
            conflict_resolved,
            segments,
            conflict_count,
        }
    }

    pub fn run_linear_step(&self) -> u64 {
        let visible_map = conflict_resolver::build_three_way_visible_map(
            self.total_lines,
            &self.conflict_ranges,
            &self.segments,
            true,
        );
        std::hint::black_box(visible_map.as_slice());
        hash_three_way_visible_map_items(&visible_map)
    }

    pub fn run_legacy_step(&self) -> u64 {
        let visible_map = build_three_way_visible_map_legacy(
            self.total_lines,
            &self.conflict_ranges,
            &self.conflict_resolved,
            true,
        );
        std::hint::black_box(visible_map.as_slice());
        hash_three_way_visible_map_items(&visible_map)
    }

    pub fn visible_rows(&self) -> usize {
        self.total_lines
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_count
    }

    #[cfg(test)]
    pub(super) fn build_linear_map(&self) -> Vec<ThreeWayVisibleItem> {
        conflict_resolver::build_three_way_visible_map(
            self.total_lines,
            &self.conflict_ranges,
            &self.segments,
            true,
        )
    }

    #[cfg(test)]
    pub(super) fn build_legacy_map(&self) -> Vec<ThreeWayVisibleItem> {
        build_three_way_visible_map_legacy(
            self.total_lines,
            &self.conflict_ranges,
            &self.conflict_resolved,
            true,
        )
    }
}

pub struct ConflictTwoWaySplitScrollFixture {
    diff_rows: Vec<gitcomet_core::file_diff::FileDiffRow>,
    stable_cache: ConflictSplitStyledTextCache,
    diff_row_conflict_map: Vec<Option<usize>>,
    visible_row_indices: Vec<usize>,
    conflict_count: usize,
    #[cfg(test)]
    syntax_mode: DiffSyntaxMode,
}

struct BlockLocalTwoWayBenchmarkRows {
    diff_rows: Vec<gitcomet_core::file_diff::FileDiffRow>,
    diff_word_highlights_split: TwoWayWordHighlights,
    diff_row_conflict_map: Vec<Option<usize>>,
    visible_row_indices: Vec<usize>,
}

fn build_block_local_two_way_benchmark_rows(
    segments: &[ConflictSegment],
) -> BlockLocalTwoWayBenchmarkRows {
    let diff_rows = conflict_resolver::block_local_two_way_diff_rows(segments);
    let inline_rows = conflict_resolver::build_inline_rows(&diff_rows);
    let (diff_row_conflict_map, _) =
        conflict_resolver::map_two_way_rows_to_conflicts(segments, &diff_rows, &inline_rows);
    let visible_row_indices =
        conflict_resolver::build_two_way_visible_indices(&diff_row_conflict_map, segments, false);
    let diff_word_highlights_split = conflict_resolver::compute_two_way_word_highlights(&diff_rows);

    BlockLocalTwoWayBenchmarkRows {
        diff_rows,
        diff_word_highlights_split,
        diff_row_conflict_map,
        visible_row_indices,
    }
}

fn prewarm_two_way_split_stable_cache(
    diff_rows: &[gitcomet_core::file_diff::FileDiffRow],
    diff_word_highlights_split: &TwoWayWordHighlights,
    theme: AppTheme,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
) -> ConflictSplitStyledTextCache {
    let mut stable_cache = ConflictSplitStyledTextCache::with_row_capacity(diff_rows.len());

    for (row_ix, row) in diff_rows.iter().enumerate() {
        let (old_word_ranges, new_word_ranges) =
            two_way_word_ranges_for_row(diff_word_highlights_split, row_ix);

        for (side, text, word_ranges) in [
            (ConflictPickSide::Ours, row.old.as_ref(), old_word_ranges),
            (ConflictPickSide::Theirs, row.new.as_ref(), new_word_ranges),
        ] {
            let Some(text) = text else {
                continue;
            };
            if text.is_empty() || (word_ranges.is_empty() && language.is_none()) {
                continue;
            }

            let key = (row_ix, side);
            if stable_cache.get(&key).is_none() {
                stable_cache.insert(
                    key,
                    super::diff_text::build_cached_diff_styled_text_with_source_identity(
                        theme,
                        text.as_ref(),
                        Some(super::diff_text::DiffTextSourceIdentity::from_str(
                            text.as_ref(),
                        )),
                        word_ranges,
                        "",
                        language,
                        syntax_mode,
                        None,
                    ),
                );
            }
        }
    }

    stable_cache
}

impl ConflictTwoWaySplitScrollFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let theme = AppTheme::gitcomet_dark();
        let segments = build_synthetic_two_way_segments(lines, conflict_blocks);
        let conflict_count = conflict_block_count_for_segments(&segments);
        let BlockLocalTwoWayBenchmarkRows {
            diff_rows,
            diff_word_highlights_split,
            diff_row_conflict_map,
            visible_row_indices,
        } = build_block_local_two_way_benchmark_rows(&segments);
        let language = diff_syntax_language_for_path("src/conflict.rs");
        let syntax_mode = DiffSyntaxMode::Auto;
        let stable_cache = prewarm_two_way_split_stable_cache(
            &diff_rows,
            &diff_word_highlights_split,
            theme,
            language,
            syntax_mode,
        );

        Self {
            diff_rows,
            stable_cache,
            diff_row_conflict_map,
            visible_row_indices,
            conflict_count,
            #[cfg(test)]
            syntax_mode,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        if self.visible_row_indices.is_empty() || window == 0 {
            return 0;
        }
        let start = start % self.visible_row_indices.len();
        let end = (start + window).min(self.visible_row_indices.len());

        let mut h = FxHasher::default();
        for &row_ix in &self.visible_row_indices[start..end] {
            self.diff_row_conflict_map
                .get(row_ix)
                .copied()
                .flatten()
                .hash(&mut h);

            let Some(row) = self.diff_rows.get(row_ix) else {
                continue;
            };
            if row.old.is_some()
                && let Some(styled) = self.stable_cache.get(&(row_ix, ConflictPickSide::Ours))
            {
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }

            if row.new.is_some()
                && let Some(styled) = self.stable_cache.get(&(row_ix, ConflictPickSide::Theirs))
            {
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }
        }
        h.finish()
    }

    pub fn visible_rows(&self) -> usize {
        self.visible_row_indices.len()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_count
    }

    #[cfg(test)]
    pub(super) fn diff_rows(&self) -> usize {
        self.diff_rows.len()
    }

    #[cfg(test)]
    pub(super) fn syntax_mode(&self) -> DiffSyntaxMode {
        self.syntax_mode
    }

    /// Returns first-window metrics for sidecar emission.
    pub fn measure_first_window(&self, window: usize) -> ConflictCompareFirstWindowMetrics {
        let total_visible = self.visible_row_indices.len();
        let rows_rendered = window.min(total_visible);
        ConflictCompareFirstWindowMetrics {
            total_diff_rows: self.diff_rows.len() as u64,
            total_visible_rows: total_visible as u64,
            rows_rendered: rows_rendered as u64,
            conflict_count: self.conflict_count as u64,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConflictCompareFirstWindowMetrics {
    pub total_diff_rows: u64,
    pub total_visible_rows: u64,
    pub rows_rendered: u64,
    pub conflict_count: u64,
}

pub struct ConflictTwoWayDiffBuildFixture {
    segments: Vec<ConflictSegment>,
    ours_text: String,
    theirs_text: String,
    full_diff_rows: Vec<gitcomet_core::file_diff::FileDiffRow>,
    block_local_diff_rows: Vec<gitcomet_core::file_diff::FileDiffRow>,
    conflict_count: usize,
}

impl ConflictTwoWayDiffBuildFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let segments = build_synthetic_two_way_segments(lines, conflict_blocks);
        let (ours_text, theirs_text) = materialize_two_way_side_texts(&segments);
        let full_diff_rows = gitcomet_core::file_diff::side_by_side_rows(&ours_text, &theirs_text);
        let block_local_diff_rows = conflict_resolver::block_local_two_way_diff_rows(&segments);
        let conflict_count = conflict_block_count_for_segments(&segments);

        Self {
            segments,
            ours_text,
            theirs_text,
            full_diff_rows,
            block_local_diff_rows,
            conflict_count,
        }
    }

    pub fn run_full_diff_build_step(&self) -> u64 {
        let diff_rows =
            gitcomet_core::file_diff::side_by_side_rows(&self.ours_text, &self.theirs_text);
        hash_file_diff_rows(&diff_rows)
    }

    pub fn run_block_local_diff_build_step(&self) -> u64 {
        let diff_rows = conflict_resolver::block_local_two_way_diff_rows(&self.segments);
        hash_file_diff_rows(&diff_rows)
    }

    pub fn run_full_word_highlights_step(&self) -> u64 {
        let highlights = conflict_resolver::compute_two_way_word_highlights(&self.full_diff_rows);
        hash_two_way_word_highlights(&highlights)
    }

    pub fn run_block_local_word_highlights_step(&self) -> u64 {
        let highlights =
            conflict_resolver::compute_two_way_word_highlights(&self.block_local_diff_rows);
        hash_two_way_word_highlights(&highlights)
    }

    pub fn full_diff_rows(&self) -> usize {
        self.full_diff_rows.len()
    }

    pub fn block_local_diff_rows(&self) -> usize {
        self.block_local_diff_rows.len()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_count
    }
}

pub struct ConflictLoadDuplicationFixture {
    path: RepoPath,
    session: ConflictSession,
    base_stage: (Option<Arc<[u8]>>, Option<Arc<str>>),
    ours_stage: (Option<Arc<[u8]>>, Option<Arc<str>>),
    theirs_stage: (Option<Arc<[u8]>>, Option<Arc<str>>),
    current_stage: (Option<Arc<[u8]>>, Option<Arc<str>>),
    current_text: Arc<str>,
    current_bytes: Arc<[u8]>,
    line_count: usize,
    conflict_count: usize,
}

impl ConflictLoadDuplicationFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let path = RepoPath::from(std::path::PathBuf::from("fixtures/large_conflict.html"));
        let (base_text, ours_text, theirs_text, current_text) =
            build_synthetic_html_conflict_texts(lines, conflict_blocks);
        let current_text: Arc<str> = current_text.into();
        let current_bytes = Arc::<[u8]>::from(current_text.as_bytes());
        let session = ConflictSession::from_merged_shared_text(
            path.to_path_buf(),
            gitcomet_core::domain::FileConflictKind::BothModified,
            ConflictPayload::Text(base_text.into()),
            ConflictPayload::Text(ours_text.into()),
            ConflictPayload::Text(theirs_text.into()),
            Arc::clone(&current_text),
        );
        let base_stage = stage_loaded_stage_parts_from_payload(&session.base);
        let ours_stage = stage_loaded_stage_parts_from_payload(&session.ours);
        let theirs_stage = stage_loaded_stage_parts_from_payload(&session.theirs);
        let current_stage = (Some(Arc::clone(&current_bytes)), None);
        let conflict_count = session.regions.len();
        let line_count = session
            .ours
            .as_text()
            .map(|text: &str| text.lines().count())
            .unwrap_or_default();

        Self {
            path,
            session,
            base_stage,
            ours_stage,
            theirs_stage,
            current_stage,
            current_text,
            current_bytes,
            line_count,
            conflict_count,
        }
    }

    pub fn run_shared_payload_forwarding_step(&self) -> u64 {
        let file = self.build_shared_conflict_file();
        let mut h = FxHasher::default();
        hash_conflict_file_load(
            &self.session,
            &self.current_text,
            &self.current_bytes,
            &file,
        )
        .hash(&mut h);
        self.line_count.hash(&mut h);
        h.finish()
    }

    pub fn run_duplicated_payload_forwarding_step(&self) -> u64 {
        let file = self.build_duplicated_conflict_file();
        let mut h = FxHasher::default();
        hash_conflict_file_load(
            &self.session,
            &self.current_text,
            &self.current_bytes,
            &file,
        )
        .hash(&mut h);
        self.line_count.hash(&mut h);
        h.finish()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_count
    }

    #[cfg(test)]
    pub(super) fn line_count(&self) -> usize {
        self.line_count
    }

    #[cfg(test)]
    pub(super) fn session(&self) -> &ConflictSession {
        &self.session
    }

    #[cfg(test)]
    pub(super) fn current_text(&self) -> &Arc<str> {
        &self.current_text
    }

    pub(super) fn build_shared_conflict_file(&self) -> ConflictFile {
        ConflictFile::from_shared_conflict_session(self.path.clone(), &self.session)
    }

    pub(super) fn build_duplicated_conflict_file(&self) -> ConflictFile {
        ConflictFile::from_loaded_stage_parts(
            self.path.clone(),
            clone_stage_parts(&self.base_stage),
            clone_stage_parts(&self.ours_stage),
            clone_stage_parts(&self.theirs_stage),
            clone_stage_parts(&self.current_stage),
        )
    }
}

fn stage_loaded_stage_parts_from_payload(
    payload: &ConflictPayload,
) -> (Option<Arc<[u8]>>, Option<Arc<str>>) {
    match payload {
        ConflictPayload::Text(text) => (None, Some(Arc::<str>::from(text.as_ref()))),
        ConflictPayload::Binary(bytes) => (Some(bytes.clone()), None),
        ConflictPayload::Absent => (None, None),
    }
}

fn clone_stage_parts(
    parts: &(Option<Arc<[u8]>>, Option<Arc<str>>),
) -> (Option<Arc<[u8]>>, Option<Arc<str>>) {
    (parts.0.clone(), parts.1.clone())
}

fn hash_conflict_file_load(
    session: &ConflictSession,
    current_text: &Arc<str>,
    current_bytes: &Arc<[u8]>,
    file: &ConflictFile,
) -> u64 {
    let mut h = FxHasher::default();
    file.path.hash(&mut h);
    session.regions.len().hash(&mut h);
    hash_conflict_file_payload(
        &mut h,
        &session.base,
        file.base_bytes.as_ref(),
        file.base.as_ref(),
    );
    hash_conflict_file_payload(
        &mut h,
        &session.ours,
        file.ours_bytes.as_ref(),
        file.ours.as_ref(),
    );
    hash_conflict_file_payload(
        &mut h,
        &session.theirs,
        file.theirs_bytes.as_ref(),
        file.theirs.as_ref(),
    );

    file.current_bytes
        .as_ref()
        .map(|bytes| bytes.len())
        .hash(&mut h);
    file.current.as_ref().map(|text| text.len()).hash(&mut h);
    file.current
        .as_ref()
        .map(|text| Arc::ptr_eq(text, current_text))
        .hash(&mut h);
    file.current_bytes
        .as_ref()
        .map(|bytes| Arc::ptr_eq(bytes, current_bytes))
        .hash(&mut h);
    h.finish()
}

fn hash_conflict_file_payload(
    h: &mut FxHasher,
    payload: &ConflictPayload,
    file_bytes: Option<&Arc<[u8]>>,
    file_text: Option<&Arc<str>>,
) {
    payload.is_binary().hash(h);
    payload.byte_len().hash(h);
    file_bytes.map(|bytes| bytes.len()).hash(h);
    file_text.map(|text| text.len()).hash(h);

    match (payload, file_text) {
        (ConflictPayload::Text(payload_text), Some(file_text)) => {
            Arc::ptr_eq(payload_text, file_text).hash(h);
        }
        _ => false.hash(h),
    }
}

fn hash_file_diff_rows(rows: &[gitcomet_core::file_diff::FileDiffRow]) -> u64 {
    let mut h = FxHasher::default();
    rows.len().hash(&mut h);
    let step = (rows.len() / 128).max(1);
    for row in rows.iter().step_by(step).take(128) {
        std::mem::discriminant(&row.kind).hash(&mut h);
        row.old_line.hash(&mut h);
        row.new_line.hash(&mut h);
        row.old.as_deref().map(str::len).hash(&mut h);
        row.new.as_deref().map(str::len).hash(&mut h);
        row.eof_newline
            .as_ref()
            .map(std::mem::discriminant)
            .hash(&mut h);
    }
    h.finish()
}

fn hash_two_way_word_highlights(highlights: &conflict_resolver::TwoWayWordHighlights) -> u64 {
    let mut h = FxHasher::default();
    highlights.len().hash(&mut h);
    let step = (highlights.len() / 128).max(1);
    for highlight in highlights.iter().step_by(step).take(128) {
        match highlight {
            Some((old_ranges, new_ranges)) => {
                hash_ranges(old_ranges.as_slice(), &mut h);
                hash_ranges(new_ranges.as_slice(), &mut h);
            }
            None => 0usize.hash(&mut h),
        }
    }
    h.finish()
}

fn hash_ranges(ranges: &[Range<usize>], hasher: &mut FxHasher) {
    ranges.len().hash(hasher);
    for range in ranges.iter().take(32) {
        range.start.hash(hasher);
        range.end.hash(hasher);
    }
}

fn conflict_block_count_for_segments(segments: &[ConflictSegment]) -> usize {
    segments
        .iter()
        .filter(|segment| matches!(segment, ConflictSegment::Block(_)))
        .count()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConflictSearchQueryStyledSource {
    StableCache,
    QueryCache,
}

pub struct ConflictSearchQueryUpdateFixture {
    diff_rows: Vec<gitcomet_core::file_diff::FileDiffRow>,
    diff_word_highlights_split: conflict_resolver::TwoWayWordHighlights,
    visible_row_indices: Vec<usize>,
    conflict_count: usize,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    theme: AppTheme,
    stable_cache: ConflictSplitStyledTextCache,
    query_cache: ConflictSplitStyledTextCache,
    query_cache_query: SharedString,
}

impl ConflictSearchQueryUpdateFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let theme = AppTheme::gitcomet_dark();
        let segments = build_synthetic_two_way_segments(lines, conflict_blocks);
        let conflict_count = conflict_block_count_for_segments(&segments);
        let BlockLocalTwoWayBenchmarkRows {
            diff_rows,
            diff_word_highlights_split,
            diff_row_conflict_map: _,
            visible_row_indices,
        } = build_block_local_two_way_benchmark_rows(&segments);
        let row_count = diff_rows.len();

        let mut fixture = Self {
            diff_rows,
            diff_word_highlights_split,
            visible_row_indices,
            conflict_count,
            language: diff_syntax_language_for_path("src/conflict.rs"),
            syntax_mode: DiffSyntaxMode::Auto,
            theme,
            stable_cache: ConflictSplitStyledTextCache::with_row_capacity(row_count),
            query_cache: ConflictSplitStyledTextCache::with_row_capacity(row_count),
            query_cache_query: SharedString::default(),
        };
        fixture.prewarm_stable_cache();
        fixture
    }

    fn prewarm_stable_cache(&mut self) {
        for row_ix in 0..self.diff_rows.len() {
            let Some(row) = self.diff_rows.get(row_ix) else {
                continue;
            };
            let (old_word_ranges, new_word_ranges) =
                two_way_word_ranges_for_row(&self.diff_word_highlights_split, row_ix);

            let _ = Self::split_row_styled(
                self.theme,
                &mut self.stable_cache,
                &mut self.query_cache,
                row_ix,
                ConflictPickSide::Ours,
                row.old.as_ref(),
                old_word_ranges,
                "",
                self.language,
                self.syntax_mode,
            );
            let _ = Self::split_row_styled(
                self.theme,
                &mut self.stable_cache,
                &mut self.query_cache,
                row_ix,
                ConflictPickSide::Theirs,
                row.new.as_ref(),
                new_word_ranges,
                "",
                self.language,
                self.syntax_mode,
            );
        }
        self.query_cache.clear();
        self.query_cache_query = SharedString::default();
    }

    fn sync_query_cache(&mut self, query: &str) {
        if self.query_cache_query.as_ref() != query {
            self.query_cache_query = query.to_string().into();
            self.query_cache.clear();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn split_row_styled(
        theme: AppTheme,
        stable_cache: &mut ConflictSplitStyledTextCache,
        query_cache: &mut ConflictSplitStyledTextCache,
        row_ix: usize,
        side: ConflictPickSide,
        text: Option<&gitcomet_core::file_diff::FileDiffLineText>,
        word_ranges: &[Range<usize>],
        query: &str,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
    ) -> Option<ConflictSearchQueryStyledSource> {
        let text = text?;
        if text.is_empty() {
            return None;
        }
        let source_identity = Some(super::diff_text::DiffTextSourceIdentity::from_str(text));
        let text = text.as_ref();

        let query = query.trim();
        let query_active = !query.is_empty();
        let base_has_style = !word_ranges.is_empty() || syntax_lang.is_some();
        let key = (row_ix, side);

        if base_has_style && stable_cache.get(&key).is_none() {
            stable_cache.insert(
                key,
                super::diff_text::build_cached_diff_styled_text_with_source_identity(
                    theme,
                    text,
                    source_identity,
                    word_ranges,
                    "",
                    syntax_lang,
                    syntax_mode,
                    None,
                ),
            );
        }

        if query_active {
            if query_cache.get(&key).is_none() {
                let styled = if let Some(base) = stable_cache.get(&key) {
                    super::diff_text::build_cached_diff_query_overlay_styled_text(
                        theme, base, query,
                    )
                } else {
                    super::diff_text::build_cached_diff_styled_text_with_source_identity(
                        theme,
                        text,
                        source_identity,
                        word_ranges,
                        query,
                        syntax_lang,
                        syntax_mode,
                        None,
                    )
                };
                query_cache.insert(key, styled);
            }
            return Some(ConflictSearchQueryStyledSource::QueryCache);
        }

        if base_has_style {
            Some(ConflictSearchQueryStyledSource::StableCache)
        } else {
            None
        }
    }

    pub fn run_query_update_step(&mut self, query: &str, start: usize, window: usize) -> u64 {
        if self.visible_row_indices.is_empty() || window == 0 {
            return 0;
        }

        self.sync_query_cache(query);
        let start = start % self.visible_row_indices.len();
        let end = (start + window).min(self.visible_row_indices.len());
        let query = self.query_cache_query.as_ref();

        let mut h = FxHasher::default();
        for &row_ix in &self.visible_row_indices[start..end] {
            row_ix.hash(&mut h);
            let Some(row) = self.diff_rows.get(row_ix) else {
                continue;
            };
            let (old_word_ranges, new_word_ranges) =
                two_way_word_ranges_for_row(&self.diff_word_highlights_split, row_ix);

            let old = Self::split_row_styled(
                self.theme,
                &mut self.stable_cache,
                &mut self.query_cache,
                row_ix,
                ConflictPickSide::Ours,
                row.old.as_ref(),
                old_word_ranges,
                query,
                self.language,
                self.syntax_mode,
            );
            if let Some(styled) = old.and_then(|source| match source {
                ConflictSearchQueryStyledSource::StableCache => {
                    self.stable_cache.get(&(row_ix, ConflictPickSide::Ours))
                }
                ConflictSearchQueryStyledSource::QueryCache => {
                    self.query_cache.get(&(row_ix, ConflictPickSide::Ours))
                }
            }) {
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }

            let new = Self::split_row_styled(
                self.theme,
                &mut self.stable_cache,
                &mut self.query_cache,
                row_ix,
                ConflictPickSide::Theirs,
                row.new.as_ref(),
                new_word_ranges,
                query,
                self.language,
                self.syntax_mode,
            );
            if let Some(styled) = new.and_then(|source| match source {
                ConflictSearchQueryStyledSource::StableCache => {
                    self.stable_cache.get(&(row_ix, ConflictPickSide::Theirs))
                }
                ConflictSearchQueryStyledSource::QueryCache => {
                    self.query_cache.get(&(row_ix, ConflictPickSide::Theirs))
                }
            }) {
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }
        }
        self.stable_cache.len().hash(&mut h);
        self.query_cache.len().hash(&mut h);
        h.finish()
    }

    pub fn visible_rows(&self) -> usize {
        self.visible_row_indices.len()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_count
    }

    #[cfg(test)]
    pub(super) fn stable_cache_entries(&self) -> usize {
        self.stable_cache.len()
    }

    #[cfg(test)]
    pub(super) fn query_cache_entries(&self) -> usize {
        self.query_cache.len()
    }

    #[cfg(test)]
    pub(super) fn diff_rows(&self) -> usize {
        self.diff_rows.len()
    }

    #[cfg(test)]
    pub(super) fn syntax_mode(&self) -> DiffSyntaxMode {
        self.syntax_mode
    }
}

pub struct ConflictSplitResizeStepFixture {
    inner: ConflictSearchQueryUpdateFixture,
    split_ratio: f32,
    drag_direction: f32,
    total_width_px: f32,
    drag_step_px: f32,
}

impl ConflictSplitResizeStepFixture {
    const MIN_RATIO: f32 = 0.1;
    const MAX_RATIO: f32 = 0.9;

    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        Self {
            inner: ConflictSearchQueryUpdateFixture::new(lines, conflict_blocks),
            split_ratio: 0.5,
            drag_direction: 1.0,
            total_width_px: 1_200.0,
            drag_step_px: 24.0,
        }
    }

    fn advance_resize_drag_step(&mut self) -> (f32, f32) {
        let available_width = (self.total_width_px - PANE_RESIZE_HANDLE_PX).max(1.0);
        let delta_ratio = (self.drag_step_px * self.drag_direction) / available_width;
        let next_ratio = (self.split_ratio + delta_ratio).clamp(Self::MIN_RATIO, Self::MAX_RATIO);
        self.split_ratio = next_ratio;
        if next_ratio <= Self::MIN_RATIO + f32::EPSILON
            || next_ratio >= Self::MAX_RATIO - f32::EPSILON
        {
            self.drag_direction = -self.drag_direction;
        }

        let left_col_width = (available_width * next_ratio).max(1.0);
        let right_col_width = (available_width - left_col_width).max(1.0);
        (left_col_width, right_col_width)
    }

    pub fn run_resize_step(&mut self, query: &str, start: usize, window: usize) -> u64 {
        let (left_col_width, right_col_width) = self.advance_resize_drag_step();
        let styled_hash = self.inner.run_query_update_step(query, start, window);

        let mut h = FxHasher::default();
        styled_hash.hash(&mut h);
        self.split_ratio.to_bits().hash(&mut h);
        left_col_width.to_bits().hash(&mut h);
        right_col_width.to_bits().hash(&mut h);
        self.drag_direction.to_bits().hash(&mut h);
        h.finish()
    }

    pub fn visible_rows(&self) -> usize {
        self.inner.visible_rows()
    }

    pub fn conflict_count(&self) -> usize {
        self.inner.conflict_count()
    }

    #[cfg(test)]
    pub(super) fn stable_cache_entries(&self) -> usize {
        self.inner.stable_cache_entries()
    }

    #[cfg(test)]
    pub(super) fn query_cache_entries(&self) -> usize {
        self.inner.query_cache_entries()
    }

    #[cfg(test)]
    pub(super) fn split_ratio(&self) -> f32 {
        self.split_ratio
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResolvedOutputGutterMarker {
    conflict_ix: usize,
    is_start: bool,
    is_end: bool,
    unresolved: bool,
}

pub struct ConflictResolvedOutputGutterScrollFixture {
    gutter_rows: Box<[conflict_resolver::ResolvedOutputGutterRow]>,
    active_conflict: usize,
    conflict_count: usize,
}

impl ConflictResolvedOutputGutterScrollFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let segments = build_synthetic_three_way_segments(lines, conflict_blocks);
        let conflict_count = segments
            .iter()
            .filter(|segment| matches!(segment, ConflictSegment::Block(_)))
            .count();

        let (resolved_text, block_ranges) =
            materialize_resolved_output_with_block_ranges(&segments);
        let output_lines = conflict_resolver::split_output_lines_for_outline(&resolved_text);

        let (base_text, ours_text, theirs_text) = materialize_three_way_side_texts(&segments);
        let base_lines = split_lines_shared(&base_text);
        let ours_lines = split_lines_shared(&ours_text);
        let theirs_lines = split_lines_shared(&theirs_text);

        let meta = conflict_resolver::compute_resolved_line_provenance(
            &output_lines,
            &conflict_resolver::SourceLines {
                a: &base_lines,
                b: &ours_lines,
                c: &theirs_lines,
            },
        );
        let line_sources = meta
            .into_iter()
            .map(|entry| entry.source)
            .collect::<Vec<_>>();
        let markers =
            build_synthetic_resolved_output_markers(&segments, &block_ranges, output_lines.len());
        let gutter_rows = line_sources
            .iter()
            .copied()
            .enumerate()
            .map(|(line_ix, source)| {
                let marker = markers.get(line_ix).copied().flatten();
                conflict_resolver::ResolvedOutputGutterRow::new(
                    source,
                    marker.map(|entry| entry.conflict_ix),
                    marker.is_some_and(|entry| entry.is_start),
                    marker.is_some_and(|entry| entry.is_end),
                    marker.is_some_and(|entry| entry.unresolved),
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self {
            gutter_rows,
            active_conflict: conflict_count / 2,
            conflict_count,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        if self.gutter_rows.is_empty() || window == 0 {
            return 0;
        }
        let start = start % self.gutter_rows.len();
        let end = (start + window).min(self.gutter_rows.len());

        let mut h = FxHasher::default();
        for (offset, gutter_row) in self.gutter_rows[start..end].iter().copied().enumerate() {
            let line_ix = start + offset;
            let source = gutter_row.source();
            source.hash(&mut h);
            gutter_row.badge_char().hash(&mut h);
            (line_ix + 1).hash(&mut h);

            gutter_row.manual_without_marker().hash(&mut h);

            if let Some(conflict_ix) = gutter_row.marker_conflict_ix() {
                conflict_ix.hash(&mut h);
                gutter_row.is_start().hash(&mut h);
                gutter_row.is_end().hash(&mut h);
                gutter_row.unresolved().hash(&mut h);
                let lane_state = if gutter_row.unresolved() {
                    0u8
                } else if conflict_ix == self.active_conflict {
                    1u8
                } else {
                    2u8
                };
                lane_state.hash(&mut h);
            } else {
                255u8.hash(&mut h);
            }
        }

        h.finish()
    }

    pub fn visible_rows(&self) -> usize {
        self.gutter_rows.len()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_count
    }
}

pub struct ResolvedOutputRecomputeIncrementalFixture {
    requested_lines: usize,
    base_text: String,
    ours_text: String,
    theirs_text: String,
    base_line_starts: Vec<usize>,
    ours_line_starts: Vec<usize>,
    theirs_line_starts: Vec<usize>,
    block_ranges: Vec<Range<usize>>,
    block_unresolved: Vec<bool>,
    both_choice_blocks: usize,
    output_text: String,
    output_line_starts: Vec<usize>,
    meta: Vec<conflict_resolver::ResolvedLineMeta>,
    markers: Vec<Option<ResolvedOutputGutterMarker>>,
    edit_line_ix: usize,
    edit_nonce: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResolvedOutputRecomputeMetrics {
    pub requested_lines: u64,
    pub conflict_blocks: u64,
    pub unresolved_blocks: u64,
    pub both_choice_blocks: u64,
    pub outline_rows: u64,
    pub marker_rows: u64,
    pub manual_rows: u64,
    pub dirty_rows: u64,
    pub recomputed_rows: u64,
    pub fallback_full_recompute: bool,
}

impl ResolvedOutputRecomputeIncrementalFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let marker_segments = build_synthetic_three_way_segments(lines, conflict_blocks);
        let (output_text, block_ranges) =
            materialize_resolved_output_with_block_ranges(&marker_segments);
        let (base_text, ours_text, theirs_text) =
            materialize_three_way_side_texts(&marker_segments);
        let base_line_starts = line_starts_for_text(&base_text);
        let ours_line_starts = line_starts_for_text(&ours_text);
        let theirs_line_starts = line_starts_for_text(&theirs_text);
        let output_line_starts = line_starts_for_text(&output_text);
        let line_count = conflict_resolver::resolved_output_outline_line_count(&output_text);
        let block_unresolved = marker_segments
            .iter()
            .filter_map(|segment| match segment {
                ConflictSegment::Block(block) => Some(!block.resolved),
                _ => None,
            })
            .collect::<Vec<_>>();
        let both_choice_blocks = marker_segments
            .iter()
            .filter(|segment| {
                matches!(
                    segment,
                    ConflictSegment::Block(ConflictBlock {
                        choice: ConflictChoice::Both,
                        ..
                    })
                )
            })
            .count();

        let mut fixture = Self {
            requested_lines: lines,
            base_text,
            ours_text,
            theirs_text,
            base_line_starts,
            ours_line_starts,
            theirs_line_starts,
            block_ranges,
            block_unresolved,
            both_choice_blocks,
            output_text,
            output_line_starts,
            meta: Vec::new(),
            markers: Vec::new(),
            edit_line_ix: 0,
            edit_nonce: 0,
        };
        fixture.meta = fixture.recompute_meta_full(fixture.output_text.as_str());
        fixture.markers = fixture.rebuild_markers(line_count);
        fixture.edit_line_ix = fixture
            .output_line_starts
            .len()
            .saturating_sub(1)
            .min(lines / 2);
        fixture
    }

    fn recompute_meta_full(&self, output_text: &str) -> Vec<conflict_resolver::ResolvedLineMeta> {
        conflict_resolver::compute_resolved_line_provenance_from_text_with_indexed_sources(
            output_text,
            self.base_text.as_str(),
            self.base_line_starts.as_slice(),
            self.ours_text.as_str(),
            self.ours_line_starts.as_slice(),
            self.theirs_text.as_str(),
            self.theirs_line_starts.as_slice(),
        )
    }

    fn insert_lookup_from_text<'a>(
        lookup: &mut HashMap<&'a str, (conflict_resolver::ResolvedLineSource, Option<u32>)>,
        source: conflict_resolver::ResolvedLineSource,
        text: &'a str,
        line_starts: &[usize],
    ) {
        let line_count = if text.is_empty() {
            0
        } else {
            line_starts.len().max(1)
        };
        for line_ix in (0..line_count).rev() {
            let line = if text.is_empty() {
                ""
            } else {
                let text_len = text.len();
                let start = line_starts.get(line_ix).copied().unwrap_or(text_len);
                let mut end = line_starts
                    .get(line_ix.saturating_add(1))
                    .copied()
                    .unwrap_or(text_len)
                    .min(text_len);
                if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
                    end = end.saturating_sub(1);
                }
                text.get(start..end).unwrap_or("")
            };
            lookup.insert(
                line,
                (
                    source,
                    Some(u32::try_from(line_ix.saturating_add(1)).unwrap_or(u32::MAX)),
                ),
            );
        }
    }

    fn build_source_lookup(
        &self,
    ) -> HashMap<&str, (conflict_resolver::ResolvedLineSource, Option<u32>)> {
        let mut lookup = HashMap::default();
        Self::insert_lookup_from_text(
            &mut lookup,
            conflict_resolver::ResolvedLineSource::C,
            self.theirs_text.as_str(),
            self.theirs_line_starts.as_slice(),
        );
        Self::insert_lookup_from_text(
            &mut lookup,
            conflict_resolver::ResolvedLineSource::B,
            self.ours_text.as_str(),
            self.ours_line_starts.as_slice(),
        );
        Self::insert_lookup_from_text(
            &mut lookup,
            conflict_resolver::ResolvedLineSource::A,
            self.base_text.as_str(),
            self.base_line_starts.as_slice(),
        );
        lookup
    }

    fn rebuild_markers(&self, output_line_count: usize) -> Vec<Option<ResolvedOutputGutterMarker>> {
        let mut markers = vec![None; output_line_count];
        if output_line_count == 0 {
            return markers;
        }
        for (conflict_ix, range) in self.block_ranges.iter().enumerate() {
            let unresolved = self
                .block_unresolved
                .get(conflict_ix)
                .copied()
                .unwrap_or(false);
            if range.start < range.end {
                let end = range.end.min(output_line_count);
                for (line_ix, marker_slot) in
                    markers.iter_mut().enumerate().take(end).skip(range.start)
                {
                    *marker_slot = Some(ResolvedOutputGutterMarker {
                        conflict_ix,
                        is_start: line_ix == range.start,
                        is_end: line_ix + 1 == range.end,
                        unresolved,
                    });
                }
            } else {
                let anchor = range.start.min(output_line_count.saturating_sub(1));
                markers[anchor] = Some(ResolvedOutputGutterMarker {
                    conflict_ix,
                    is_start: true,
                    is_end: true,
                    unresolved,
                });
            }
        }
        markers
    }

    fn line_text<'a>(&self, text: &'a str, line_starts: &[usize], line_ix: usize) -> &'a str {
        if text.is_empty() {
            return "";
        }
        let text_len = text.len();
        let start = line_starts.get(line_ix).copied().unwrap_or(text_len);
        if start >= text_len {
            return "";
        }
        let mut end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text_len)
            .min(text_len);
        if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        text.get(start..end).unwrap_or("")
    }

    fn dirty_line_range(
        line_starts: &[usize],
        text_len: usize,
        byte_range: Range<usize>,
    ) -> Range<usize> {
        let line_count = line_starts.len().max(1);
        let start = line_starts
            .partition_point(|&line_start| line_start <= byte_range.start.min(text_len))
            .saturating_sub(1)
            .min(line_count.saturating_sub(1));
        let end = if byte_range.is_empty() {
            start.saturating_add(1)
        } else {
            line_starts
                .partition_point(|&line_start| {
                    line_start <= byte_range.end.min(text_len).saturating_sub(1)
                })
                .saturating_sub(1)
                .saturating_add(1)
        }
        .min(line_count)
        .max(start.saturating_add(1));
        start..end
    }

    fn next_single_line_edit(&mut self) -> (String, Range<usize>, Range<usize>, usize, isize) {
        self.edit_nonce = self.edit_nonce.wrapping_add(1);
        let line_ix = self
            .edit_line_ix
            .min(self.output_line_starts.len().saturating_sub(1));
        let text_len = self.output_text.len();
        let start = self
            .output_line_starts
            .get(line_ix)
            .copied()
            .unwrap_or(text_len)
            .min(text_len);
        let mut end = self
            .output_line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text_len)
            .min(text_len);
        if end > start && self.output_text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }

        let replacement = format!(
            "let bench_manual_{}_{} = {};",
            line_ix,
            self.edit_nonce,
            self.edit_nonce % 31
        );
        let mut next = String::with_capacity(
            self.output_text
                .len()
                .saturating_sub(end.saturating_sub(start))
                .saturating_add(replacement.len()),
        );
        next.push_str(self.output_text.get(0..start).unwrap_or_default());
        next.push_str(replacement.as_str());
        next.push_str(self.output_text.get(end..).unwrap_or_default());
        let old_range = start..end;
        let new_range = start..start.saturating_add(replacement.len());
        let byte_delta = replacement.len() as isize - end.saturating_sub(start) as isize;
        (next, old_range, new_range, line_ix, byte_delta)
    }

    fn shift_line_starts_after_same_line_edit(
        mut line_starts: Vec<usize>,
        edited_line_ix: usize,
        byte_delta: isize,
    ) -> Vec<usize> {
        if byte_delta == 0 {
            return line_starts;
        }
        for start in line_starts
            .iter_mut()
            .skip(edited_line_ix.saturating_add(1))
        {
            *start = if byte_delta >= 0 {
                start.saturating_add(byte_delta as usize)
            } else {
                start.saturating_sub((-byte_delta) as usize)
            };
        }
        line_starts
    }

    fn hash_outline_state(&self) -> u64 {
        let mut h = FxHasher::default();
        self.meta.len().hash(&mut h);
        self.markers.len().hash(&mut h);
        self.output_line_starts.len().hash(&mut h);
        self.meta
            .iter()
            .take(32)
            .map(|m| (m.output_line, m.source, m.input_line))
            .collect::<Vec<_>>()
            .hash(&mut h);
        h.finish()
    }

    fn capture_metrics(
        &self,
        dirty_rows: usize,
        recomputed_rows: usize,
        fallback_full_recompute: bool,
    ) -> ResolvedOutputRecomputeMetrics {
        let unresolved_blocks = self.block_unresolved.iter().filter(|&&value| value).count();
        let marker_rows = self
            .markers
            .iter()
            .filter(|marker| marker.is_some())
            .count();
        let manual_rows = self
            .meta
            .iter()
            .filter(|meta| meta.source == conflict_resolver::ResolvedLineSource::Manual)
            .count();

        ResolvedOutputRecomputeMetrics {
            requested_lines: bench_counter_u64(self.requested_lines),
            conflict_blocks: bench_counter_u64(self.block_ranges.len()),
            unresolved_blocks: bench_counter_u64(unresolved_blocks),
            both_choice_blocks: bench_counter_u64(self.both_choice_blocks),
            outline_rows: bench_counter_u64(self.meta.len()),
            marker_rows: bench_counter_u64(marker_rows),
            manual_rows: bench_counter_u64(manual_rows),
            dirty_rows: bench_counter_u64(dirty_rows),
            recomputed_rows: bench_counter_u64(recomputed_rows),
            fallback_full_recompute,
        }
    }

    pub fn run_full_recompute_step(&mut self) -> u64 {
        self.run_full_recompute_with_metrics().0
    }

    pub fn run_full_recompute_with_metrics(&mut self) -> (u64, ResolvedOutputRecomputeMetrics) {
        let (next_output, _old_range, _new_range, _line_ix, _byte_delta) =
            self.next_single_line_edit();
        let next_line_starts = line_starts_for_text(&next_output);
        let line_count = conflict_resolver::resolved_output_outline_line_count(&next_output);
        let next_meta = self.recompute_meta_full(next_output.as_str());
        let next_markers = self.rebuild_markers(line_count);

        self.output_text = next_output;
        self.output_line_starts = next_line_starts;
        self.meta = next_meta;
        self.markers = next_markers;

        let hash = self.hash_outline_state();
        let metrics = self.capture_metrics(0, line_count, false);
        (hash, metrics)
    }

    pub fn run_incremental_recompute_step(&mut self) -> u64 {
        self.run_incremental_recompute_with_metrics().0
    }

    pub fn run_incremental_recompute_with_metrics(
        &mut self,
    ) -> (u64, ResolvedOutputRecomputeMetrics) {
        let (next_output, old_byte_range, new_byte_range, edited_line_ix, byte_delta) =
            self.next_single_line_edit();

        let mut old_dirty = Self::dirty_line_range(
            self.output_line_starts.as_slice(),
            self.output_text.len(),
            old_byte_range,
        );
        let next_line_count = self.output_line_starts.len().max(1);
        let next_line_starts = Self::shift_line_starts_after_same_line_edit(
            std::mem::take(&mut self.output_line_starts),
            edited_line_ix,
            byte_delta,
        );
        let mut new_dirty = Self::dirty_line_range(
            next_line_starts.as_slice(),
            next_output.len(),
            new_byte_range,
        );
        old_dirty.start = old_dirty.start.saturating_sub(1);
        old_dirty.end = old_dirty.end.saturating_add(1).min(self.meta.len());
        new_dirty.start = new_dirty.start.saturating_sub(1);
        new_dirty.end = new_dirty.end.saturating_add(1).min(next_line_count);
        let dirty_rows = new_dirty.len();
        let recomputed_rows;
        let mut fallback_full_recompute = false;
        if old_dirty.start != new_dirty.start {
            // Keep this fixture conservative; fall back to full for odd shifts.
            fallback_full_recompute = true;
            self.output_text = next_output;
            self.output_line_starts = next_line_starts;
            self.meta = self.recompute_meta_full(self.output_text.as_str());
            self.markers = self.rebuild_markers(next_line_count);
            recomputed_rows = next_line_count;
            let hash = self.hash_outline_state();
            let metrics =
                self.capture_metrics(dirty_rows, recomputed_rows, fallback_full_recompute);
            return (hash, metrics);
        }

        let line_delta = new_dirty.len() as isize - old_dirty.len() as isize;
        let middle_meta = {
            let source_lookup = self.build_source_lookup();
            let mut middle_meta = Vec::with_capacity(new_dirty.len());
            for line_ix in new_dirty.clone() {
                let line =
                    self.line_text(next_output.as_str(), next_line_starts.as_slice(), line_ix);
                let (source, input_line) = source_lookup
                    .get(line)
                    .copied()
                    .unwrap_or((conflict_resolver::ResolvedLineSource::Manual, None));
                middle_meta.push(conflict_resolver::ResolvedLineMeta {
                    output_line: u32::try_from(line_ix).unwrap_or(u32::MAX),
                    source,
                    input_line,
                });
            }
            middle_meta
        };
        let old_meta = std::mem::take(&mut self.meta);
        let mut next_meta = Vec::with_capacity(next_line_count);
        next_meta.extend(
            old_meta
                .iter()
                .take(old_dirty.start.min(old_meta.len()))
                .cloned(),
        );
        next_meta.extend(middle_meta);
        for meta in old_meta.iter().skip(old_dirty.end.min(old_meta.len())) {
            let mut shifted = meta.clone();
            let shifted_ix = if line_delta >= 0 {
                (meta.output_line as usize).saturating_add(line_delta as usize)
            } else {
                (meta.output_line as usize).saturating_sub((-line_delta) as usize)
            };
            shifted.output_line = u32::try_from(shifted_ix).unwrap_or(u32::MAX);
            next_meta.push(shifted);
        }
        if next_meta.len() != next_line_count {
            fallback_full_recompute = true;
            self.output_text = next_output;
            self.output_line_starts = next_line_starts;
            self.meta = self.recompute_meta_full(self.output_text.as_str());
            self.markers = self.rebuild_markers(next_line_count);
            recomputed_rows = next_line_count;
            let hash = self.hash_outline_state();
            let metrics =
                self.capture_metrics(dirty_rows, recomputed_rows, fallback_full_recompute);
            return (hash, metrics);
        }

        let old_markers = std::mem::take(&mut self.markers);
        let mut next_markers = if old_markers.len() == next_line_count {
            old_markers
        } else {
            self.rebuild_markers(next_line_count)
        };
        for line_ix in new_dirty.clone() {
            if let Some(slot) = next_markers.get_mut(line_ix) {
                *slot = None;
            }
        }
        for (conflict_ix, range) in self.block_ranges.iter().enumerate() {
            if range.start >= range.end || range.end > next_line_count {
                continue;
            }
            if range.start >= new_dirty.end || new_dirty.start >= range.end {
                continue;
            }
            let unresolved = self
                .block_unresolved
                .get(conflict_ix)
                .copied()
                .unwrap_or(false);
            for (line_ix, marker_slot) in next_markers
                .iter_mut()
                .enumerate()
                .take(range.end)
                .skip(range.start)
            {
                *marker_slot = Some(ResolvedOutputGutterMarker {
                    conflict_ix,
                    is_start: line_ix == range.start,
                    is_end: line_ix + 1 == range.end,
                    unresolved,
                });
            }
        }

        self.output_text = next_output;
        self.output_line_starts = next_line_starts;
        self.meta = next_meta;
        self.markers = next_markers;

        recomputed_rows = dirty_rows;

        let hash = self.hash_outline_state();
        let metrics = self.capture_metrics(dirty_rows, recomputed_rows, fallback_full_recompute);
        (hash, metrics)
    }

    pub fn visible_rows(&self) -> usize {
        self.output_line_starts.len().max(1)
    }
}

/// Benchmark fixture for streamed/paged conflict provider performance.
///
/// Creates a single whole-file conflict block with realistic mixed content
/// (shared lines, insertions, deletions) to exercise the anchor index and
/// paged row provider at scale.
pub struct ConflictStreamedProviderFixture {
    segments: Vec<ConflictSegment>,
    split_row_index: conflict_resolver::ConflictSplitRowIndex,
    two_way_projection: conflict_resolver::TwoWaySplitProjection,
    ours_line_count: usize,
    theirs_line_count: usize,
}

impl ConflictStreamedProviderFixture {
    pub fn new(lines: usize) -> Self {
        let segments = build_synthetic_whole_file_conflict_segments(lines);
        let split_row_index = conflict_resolver::ConflictSplitRowIndex::new(
            &segments,
            conflict_resolver::BLOCK_LOCAL_DIFF_CONTEXT_LINES,
        );
        let two_way_projection =
            conflict_resolver::TwoWaySplitProjection::new(&split_row_index, &segments, false);
        let (ours_line_count, theirs_line_count) = match &segments[0] {
            ConflictSegment::Block(block) => {
                (block.ours.lines().count(), block.theirs.lines().count())
            }
            _ => (0, 0),
        };

        Self {
            segments,
            split_row_index,
            two_way_projection,
            ours_line_count,
            theirs_line_count,
        }
    }

    /// Benchmark: build the split row index from scratch (includes anchor build).
    pub fn run_index_build_step(&self) -> u64 {
        let index = conflict_resolver::ConflictSplitRowIndex::new(
            &self.segments,
            conflict_resolver::BLOCK_LOCAL_DIFF_CONTEXT_LINES,
        );
        let mut h = FxHasher::default();
        index.total_rows().hash(&mut h);
        h.finish()
    }

    fn hash_visible_window(&self, start: usize, end: usize) -> u64 {
        let mut h = FxHasher::default();
        self.two_way_projection.for_each_chunk_in_visible_range(
            start..end,
            |_, source_range, _conflict_ix| {
                self.split_row_index
                    .for_each_row_range(&self.segments, source_range, |_, row| {
                        std::mem::discriminant(&row.kind).hash(&mut h);
                        row.old.as_deref().map(|s| s.len()).hash(&mut h);
                        row.new.as_deref().map(|s| s.len()).hash(&mut h);
                    });
            },
        );
        h.finish()
    }

    /// Benchmark: generate rows for the first viewport window.
    pub fn run_first_page_step(&self, window: usize) -> u64 {
        self.split_row_index.clear_cached_pages();
        let end = window.min(self.two_way_projection.visible_len());
        self.hash_visible_window(0, end)
    }

    /// Prime the page cache for the first viewport window (call before benchmarking cache hits).
    pub fn prime_first_page_cache(&self, window: usize) {
        let end = window.min(self.two_way_projection.visible_len());
        let _ = self.hash_visible_window(0, end);
    }

    /// Benchmark: re-read the first viewport window from a warm page cache.
    /// Call `prime_first_page_cache` once before entering the timed loop.
    pub fn run_first_page_cache_hit_step(&self, window: usize) -> u64 {
        let end = window.min(self.two_way_projection.visible_len());
        self.hash_visible_window(0, end)
    }

    /// Benchmark: generate rows for a deep-scroll position.
    pub fn run_deep_scroll_step(&self, offset_fraction: f64, window: usize) -> u64 {
        let visible_len = self.two_way_projection.visible_len();
        if visible_len == 0 || window == 0 {
            return 0;
        }
        let start = ((visible_len as f64 * offset_fraction) as usize).min(visible_len - 1);
        let end = (start + window).min(visible_len);
        self.split_row_index.clear_cached_pages();
        self.hash_visible_window(start, end)
    }

    /// Benchmark: search for text in the middle of the giant block.
    pub fn run_search_step(&self, needle: &str) -> u64 {
        let matches = self
            .split_row_index
            .search_text_matching_rows(&self.segments, needle.as_bytes());
        let mut h = FxHasher::default();
        matches.len().hash(&mut h);
        for &row_ix in matches.iter().take(32) {
            row_ix.hash(&mut h);
        }
        h.finish()
    }

    /// Benchmark: build the two-way projection from a pre-built index.
    pub fn run_projection_build_step(&self) -> u64 {
        let proj = conflict_resolver::TwoWaySplitProjection::new(
            &self.split_row_index,
            &self.segments,
            false,
        );
        let mut h = FxHasher::default();
        proj.visible_len().hash(&mut h);
        h.finish()
    }

    pub fn total_rows(&self) -> usize {
        self.split_row_index.total_rows()
    }

    pub fn visible_rows(&self) -> usize {
        self.two_way_projection.visible_len()
    }

    pub fn ours_line_count(&self) -> usize {
        self.ours_line_count
    }

    pub fn theirs_line_count(&self) -> usize {
        self.theirs_line_count
    }

    #[cfg(test)]
    pub(super) fn cached_page_count(&self) -> usize {
        self.split_row_index.cached_page_count()
    }

    /// Total metadata bytes: split row index + two-way projection (excludes page cache
    /// and source text, which are shared).
    #[cfg(test)]
    pub(super) fn metadata_byte_size(&self) -> usize {
        self.split_row_index.metadata_byte_size() + self.two_way_projection.metadata_byte_size()
    }
}

/// Benchmark fixture for streamed resolved-output projection performance.
///
/// Uses many synthetic three-way conflict blocks so the output projection has
/// to track real conflict-line ranges without materializing a whole output text.
pub struct ConflictStreamedResolvedOutputFixture {
    segments: Vec<ConflictSegment>,
    projection: conflict_resolver::ResolvedOutputProjection,
}

impl ConflictStreamedResolvedOutputFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let segments = build_synthetic_three_way_segments(lines, conflict_blocks);
        let projection = conflict_resolver::ResolvedOutputProjection::from_segments(&segments);
        Self {
            segments,
            projection,
        }
    }

    /// Benchmark: build the streamed resolved-output projection from scratch.
    pub fn run_projection_build_step(&self) -> u64 {
        let projection = conflict_resolver::ResolvedOutputProjection::from_segments(&self.segments);
        let mut h = FxHasher::default();
        projection.len().hash(&mut h);
        projection.output_hash().hash(&mut h);
        h.finish()
    }

    fn hash_visible_window(&self, start: usize, end: usize) -> u64 {
        let mut h = FxHasher::default();
        self.projection
            .for_each_line_text_in_range(&self.segments, start..end, |_, line| {
                line.len().hash(&mut h);
                line.as_bytes().first().copied().hash(&mut h);
                line.as_bytes().last().copied().hash(&mut h);
            });
        h.finish()
    }

    /// Benchmark: resolve the first viewport window of streamed output lines.
    pub fn run_window_step(&self, window: usize) -> u64 {
        let end = window.min(self.visible_rows());
        self.hash_visible_window(0, end)
    }

    /// Benchmark: resolve a deep-scroll window of streamed output lines.
    pub fn run_deep_window_step(&self, offset_fraction: f64, window: usize) -> u64 {
        let visible_len = self.visible_rows();
        if visible_len == 0 || window == 0 {
            return 0;
        }
        let start = ((visible_len as f64 * offset_fraction) as usize).min(visible_len - 1);
        let end = (start + window).min(visible_len);
        self.hash_visible_window(start, end)
    }

    pub fn visible_rows(&self) -> usize {
        self.projection.len()
    }

    #[cfg(test)]
    pub(super) fn metadata_byte_size(&self) -> usize {
        self.projection.metadata_byte_size()
    }

    #[cfg(test)]
    pub(super) fn materialized_output_len(&self) -> usize {
        conflict_resolver::generate_resolved_text(&self.segments).len()
    }
}

pub struct MergeOpenBootstrapFixture {
    path: PathBuf,
    base_text: Arc<str>,
    ours_text: Arc<str>,
    theirs_text: Arc<str>,
    current_text: Arc<str>,
    line_count: usize,
    conflict_block_count: usize,
    syntax_language: Option<DiffSyntaxLanguage>,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct MergeOpenBootstrapMetrics {
    pub trace_event_count: u64,
    pub conflict_block_count: u64,
    pub diff_row_count: u64,
    pub inline_row_count: u64,
    pub resolved_output_line_count: u64,
    pub two_way_visible_rows: u64,
    pub three_way_visible_rows: u64,
    pub rendering_mode_streamed: u64,
    pub full_output_generated: u64,
    pub full_syntax_parse_requested: u64,
    pub whole_block_diff_ran: u64,
    pub rss_kib: u64,
    pub parse_conflict_markers_ms: f64,
    pub generate_resolved_text_ms: f64,
    pub side_by_side_rows_ms: f64,
    pub build_three_way_conflict_maps_ms: f64,
    pub compute_three_way_word_highlights_ms: f64,
    pub compute_two_way_word_highlights_ms: f64,
    pub bootstrap_total_ms: f64,
}

struct MergeOpenBootstrapRunResult {
    hash: u64,
    two_way_visible_rows: usize,
    three_way_visible_rows: usize,
}

struct MergeOpenBootstrapTraceContext {
    path: PathBuf,
    base: MergetoolTraceSideStats,
    ours: MergetoolTraceSideStats,
    theirs: MergetoolTraceSideStats,
    current: MergetoolTraceSideStats,
}

impl MergeOpenBootstrapTraceContext {
    fn new(
        path: PathBuf,
        base_text: &str,
        ours_text: &str,
        theirs_text: &str,
        current_text: &str,
    ) -> Self {
        if !mergetool_trace::is_enabled() {
            return Self {
                path,
                base: MergetoolTraceSideStats::default(),
                ours: MergetoolTraceSideStats::default(),
                theirs: MergetoolTraceSideStats::default(),
                current: MergetoolTraceSideStats::default(),
            };
        }

        Self {
            path,
            base: MergetoolTraceSideStats::from_text(Some(base_text)),
            ours: MergetoolTraceSideStats::from_text(Some(ours_text)),
            theirs: MergetoolTraceSideStats::from_text(Some(theirs_text)),
            current: MergetoolTraceSideStats::from_text(Some(current_text)),
        }
    }

    fn event(&self, stage: MergetoolTraceStage, started: Instant) -> MergetoolTraceEvent {
        MergetoolTraceEvent::new(stage, Some(self.path.clone()), started.elapsed())
            .with_base(self.base)
            .with_ours(self.ours)
            .with_theirs(self.theirs)
            .with_current(self.current)
    }

    fn bootstrap_event(
        &self,
        stage: MergetoolTraceStage,
        started: Instant,
        decisions: MergeOpenBootstrapTraceDecisions,
    ) -> MergetoolTraceEvent {
        decisions.apply_to_event(self.event(stage, started))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct MergeOpenBootstrapTraceDecisions {
    rendering_mode: Option<MergetoolTraceRenderingMode>,
    whole_block_diff_ran: Option<bool>,
    full_output_generated: Option<bool>,
    full_syntax_parse_requested: Option<bool>,
}

impl MergeOpenBootstrapTraceDecisions {
    fn apply_to_event(self, event: MergetoolTraceEvent) -> MergetoolTraceEvent {
        event
            .with_rendering_mode(self.rendering_mode)
            .with_whole_block_diff_ran(self.whole_block_diff_ran)
            .with_full_output_generated(self.full_output_generated)
            .with_full_syntax_parse_requested(self.full_syntax_parse_requested)
    }
}

impl MergeOpenBootstrapFixture {
    /// Small eager bootstrap path: no conflict markers, so the resolver stays
    /// on the non-streamed path while still building the surrounding resolver
    /// state for a normal text file.
    pub fn small(lines: usize) -> Self {
        Self::new(lines.max(1), 0)
    }

    pub fn large_streamed(lines: usize, conflict_blocks: usize) -> Self {
        let lines = lines.max(conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1);
        Self::new(lines, conflict_blocks.max(1))
    }

    /// Many conflict blocks in a moderate-size file.
    ///
    /// Tests how bootstrap cost scales with conflict-block count rather than
    /// file size.  Uses `10 * conflict_blocks + 100` total lines so each
    /// block gets a small amount of surrounding context.
    pub fn many_conflicts(conflict_blocks: usize) -> Self {
        let conflict_blocks = conflict_blocks.max(2);
        let lines = 10 * conflict_blocks + 100;
        Self::new(lines, conflict_blocks)
    }

    /// Large file with many conflict blocks — the extreme scale case.
    ///
    /// Combines a high line count (forced above the streamed threshold) with
    /// a large number of conflict blocks to stress both dimensions
    /// simultaneously.
    pub fn large_many_conflicts(lines: usize, conflict_blocks: usize) -> Self {
        let lines = lines.max(conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1);
        Self::new(lines, conflict_blocks.max(2))
    }

    fn new(lines: usize, conflict_blocks: usize) -> Self {
        let path = PathBuf::from("fixtures/merge_open_bootstrap.html");
        let (base_text, ours_text, theirs_text, current_text) =
            build_synthetic_html_conflict_texts(lines, conflict_blocks);
        let line_count = trace_line_count(&base_text);

        Self {
            path,
            base_text: Arc::<str>::from(base_text),
            ours_text: Arc::<str>::from(ours_text),
            theirs_text: Arc::<str>::from(theirs_text),
            current_text: Arc::<str>::from(current_text),
            line_count,
            conflict_block_count: conflict_blocks,
            syntax_language: diff_syntax_language_for_path("fixtures/merge_open_bootstrap.html"),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal().hash
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, MergeOpenBootstrapMetrics) {
        let _trace = mergetool_trace::capture();
        let result = self.run_internal();
        let trace = mergetool_trace::snapshot();
        let metrics = MergeOpenBootstrapMetrics::from_trace(&trace, &result);
        (result.hash, metrics)
    }

    pub fn line_count(&self) -> usize {
        self.line_count
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_block_count
    }

    fn run_internal(&self) -> MergeOpenBootstrapRunResult {
        let bootstrap_started = Instant::now();
        let base_text = self.base_text.as_ref();
        let ours_text = self.ours_text.as_ref();
        let theirs_text = self.theirs_text.as_ref();
        let current_text = self.current_text.as_ref();
        let trace_ctx = MergeOpenBootstrapTraceContext::new(
            self.path.clone(),
            base_text,
            ours_text,
            theirs_text,
            current_text,
        );

        let three_way_base_len = trace_line_count(base_text);
        let three_way_ours_len = trace_line_count(ours_text);
        let three_way_theirs_len = trace_line_count(theirs_text);
        let three_way_len = three_way_base_len
            .max(three_way_ours_len)
            .max(three_way_theirs_len);

        let marker_parse_started = Instant::now();
        let mut marker_segments = conflict_resolver::parse_conflict_markers_shared_nonempty(
            Arc::clone(&self.current_text),
        );
        let rendering_mode =
            conflict_resolver::select_conflict_rendering_mode(&marker_segments, three_way_len);
        let expected_streamed = self.conflict_block_count > 0;
        assert_eq!(
            rendering_mode.is_streamed_large_file(),
            expected_streamed,
            "merge_open_bootstrap fixture rendering mode mismatch (expected_streamed={expected_streamed}, actual_conflicts={}, three-way lines={three_way_len})",
            conflict_resolver::conflict_count(&marker_segments),
        );

        let mut trace_decisions = MergeOpenBootstrapTraceDecisions {
            rendering_mode: Some(trace_rendering_mode(rendering_mode)),
            full_output_generated: Some(!rendering_mode.is_streamed_large_file()),
            full_syntax_parse_requested: Some(
                self.syntax_language.is_some()
                    && [base_text, ours_text, theirs_text]
                        .into_iter()
                        .any(|text| !text.is_empty()),
            ),
            ..Default::default()
        };
        mergetool_trace::record_with(|| {
            trace_ctx
                .bootstrap_event(
                    MergetoolTraceStage::ParseConflictMarkers,
                    marker_parse_started,
                    trace_decisions,
                )
                .with_conflict_block_count(Some(conflict_resolver::conflict_count(
                    &marker_segments,
                )))
        });

        if !base_text.is_empty() {
            conflict_resolver::populate_block_bases_from_shared_ancestor(
                &mut marker_segments,
                Arc::clone(&self.base_text),
            );
        }
        let conflict_region_indices =
            conflict_resolver::sequential_conflict_region_indices(&marker_segments);
        let conflict_block_count = conflict_resolver::conflict_count(&marker_segments);

        let resolved_started = Instant::now();
        let resolved_output = if rendering_mode.is_streamed_large_file() {
            MergeOpenBootstrapResolvedOutput::Projection(
                conflict_resolver::ResolvedOutputProjection::from_segments(&marker_segments),
            )
        } else {
            MergeOpenBootstrapResolvedOutput::Text(
                conflict_resolver::bootstrap_resolved_output_text(
                    &marker_segments,
                    Some(&self.current_text),
                    Some(&self.ours_text),
                    Some(&self.theirs_text),
                ),
            )
        };
        let resolved_output_line_count = resolved_output.line_count();
        mergetool_trace::record_with(|| {
            trace_ctx
                .bootstrap_event(
                    MergetoolTraceStage::GenerateResolvedText,
                    resolved_started,
                    trace_decisions,
                )
                .with_conflict_block_count(Some(conflict_block_count))
                .with_resolved_output_line_count(Some(resolved_output_line_count))
        });

        let diff_rows_started = Instant::now();
        let split_row_index = conflict_resolver::ConflictSplitRowIndex::new(
            &marker_segments,
            conflict_resolver::BLOCK_LOCAL_DIFF_CONTEXT_LINES,
        );
        trace_decisions.whole_block_diff_ran = Some(false);
        let diff_row_count = split_row_index.total_rows();
        mergetool_trace::record_with(|| {
            trace_ctx
                .bootstrap_event(
                    MergetoolTraceStage::SideBySideRows,
                    diff_rows_started,
                    trace_decisions,
                )
                .with_conflict_block_count(Some(conflict_block_count))
                .with_diff_row_count(Some(diff_row_count))
        });

        let three_way_word_highlights_started = Instant::now();
        mergetool_trace::record_with(|| {
            trace_ctx
                .bootstrap_event(
                    MergetoolTraceStage::ComputeThreeWayWordHighlights,
                    three_way_word_highlights_started,
                    trace_decisions,
                )
                .with_conflict_block_count(Some(conflict_block_count))
        });

        let two_way_word_highlights_started = Instant::now();
        mergetool_trace::record_with(|| {
            trace_ctx
                .bootstrap_event(
                    MergetoolTraceStage::ComputeTwoWayWordHighlights,
                    two_way_word_highlights_started,
                    trace_decisions,
                )
                .with_conflict_block_count(Some(conflict_block_count))
                .with_diff_row_count(Some(diff_row_count))
        });

        let three_way_rebuild_started = Instant::now();
        let maps = conflict_resolver::build_three_way_conflict_maps_without_line_maps(
            &marker_segments,
            three_way_base_len,
            three_way_ours_len,
            three_way_theirs_len,
        );
        let three_way_visible_projection = conflict_resolver::build_three_way_visible_projection(
            three_way_len,
            &maps.conflict_ranges[1],
            &marker_segments,
            false,
        );
        mergetool_trace::record_with(|| {
            trace_ctx
                .bootstrap_event(
                    MergetoolTraceStage::BuildThreeWayConflictMaps,
                    three_way_rebuild_started,
                    trace_decisions,
                )
                .with_conflict_block_count(Some(conflict_block_count))
        });

        let two_way_split_projection = conflict_resolver::TwoWaySplitProjection::new(
            &split_row_index,
            &marker_segments,
            false,
        );
        let inline_row_count = 0usize;

        mergetool_trace::record_with(|| {
            trace_ctx
                .bootstrap_event(
                    MergetoolTraceStage::ConflictResolverBootstrapTotal,
                    bootstrap_started,
                    trace_decisions,
                )
                .with_conflict_block_count(Some(conflict_block_count))
                .with_diff_row_count(Some(diff_row_count))
                .with_inline_row_count(Some(inline_row_count))
                .with_resolved_output_line_count(Some(resolved_output_line_count))
        });

        let hash = hash_merge_open_bootstrap_state(
            &marker_segments,
            &conflict_region_indices,
            &maps,
            &split_row_index,
            &two_way_split_projection,
            &three_way_visible_projection,
            &resolved_output,
        );

        MergeOpenBootstrapRunResult {
            hash,
            two_way_visible_rows: two_way_split_projection.visible_len(),
            three_way_visible_rows: three_way_visible_projection.len(),
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
impl MergeOpenBootstrapMetrics {
    fn from_trace(
        trace: &mergetool_trace::MergetoolTraceSnapshot,
        result: &MergeOpenBootstrapRunResult,
    ) -> Self {
        let bootstrap =
            find_trace_stage(trace, MergetoolTraceStage::ConflictResolverBootstrapTotal);
        Self {
            trace_event_count: trace.events.len() as u64,
            conflict_block_count: bootstrap
                .and_then(|event| event.conflict_block_count)
                .unwrap_or_default() as u64,
            diff_row_count: bootstrap
                .and_then(|event| event.diff_row_count)
                .unwrap_or_default() as u64,
            inline_row_count: bootstrap
                .and_then(|event| event.inline_row_count)
                .unwrap_or_default() as u64,
            resolved_output_line_count: bootstrap
                .and_then(|event| event.resolved_output_line_count)
                .unwrap_or_default() as u64,
            two_way_visible_rows: result.two_way_visible_rows as u64,
            three_way_visible_rows: result.three_way_visible_rows as u64,
            rendering_mode_streamed: bootstrap
                .map(|event| {
                    u64::from(
                        event.rendering_mode
                            == Some(MergetoolTraceRenderingMode::StreamedLargeFile),
                    )
                })
                .unwrap_or_default(),
            full_output_generated: bootstrap
                .map(|event| u64::from(event.full_output_generated.unwrap_or(false)))
                .unwrap_or_default(),
            full_syntax_parse_requested: bootstrap
                .map(|event| u64::from(event.full_syntax_parse_requested.unwrap_or(false)))
                .unwrap_or_default(),
            whole_block_diff_ran: bootstrap
                .map(|event| u64::from(event.whole_block_diff_ran.unwrap_or(false)))
                .unwrap_or_default(),
            rss_kib: bootstrap
                .and_then(|event| event.rss_kib)
                .unwrap_or_default(),
            parse_conflict_markers_ms: trace_stage_ms(
                trace,
                MergetoolTraceStage::ParseConflictMarkers,
            ),
            generate_resolved_text_ms: trace_stage_ms(
                trace,
                MergetoolTraceStage::GenerateResolvedText,
            ),
            side_by_side_rows_ms: trace_stage_ms(trace, MergetoolTraceStage::SideBySideRows),
            build_three_way_conflict_maps_ms: trace_stage_ms(
                trace,
                MergetoolTraceStage::BuildThreeWayConflictMaps,
            ),
            compute_three_way_word_highlights_ms: trace_stage_ms(
                trace,
                MergetoolTraceStage::ComputeThreeWayWordHighlights,
            ),
            compute_two_way_word_highlights_ms: trace_stage_ms(
                trace,
                MergetoolTraceStage::ComputeTwoWayWordHighlights,
            ),
            bootstrap_total_ms: trace_stage_ms(
                trace,
                MergetoolTraceStage::ConflictResolverBootstrapTotal,
            ),
        }
    }
}

fn trace_rendering_mode(
    mode: conflict_resolver::ConflictRenderingMode,
) -> MergetoolTraceRenderingMode {
    match mode {
        conflict_resolver::ConflictRenderingMode::EagerSmallFile => {
            MergetoolTraceRenderingMode::EagerSmallFile
        }
        conflict_resolver::ConflictRenderingMode::StreamedLargeFile => {
            MergetoolTraceRenderingMode::StreamedLargeFile
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn find_trace_stage(
    trace: &mergetool_trace::MergetoolTraceSnapshot,
    stage: MergetoolTraceStage,
) -> Option<&MergetoolTraceEvent> {
    trace.events.iter().rev().find(|event| event.stage == stage)
}

#[cfg(any(test, feature = "benchmarks"))]
fn trace_stage_ms(
    trace: &mergetool_trace::MergetoolTraceSnapshot,
    stage: MergetoolTraceStage,
) -> f64 {
    find_trace_stage(trace, stage)
        .map(|event| event.elapsed.as_secs_f64() * 1_000.0)
        .unwrap_or_default()
}

enum MergeOpenBootstrapResolvedOutput {
    Projection(conflict_resolver::ResolvedOutputProjection),
    Text(conflict_resolver::ResolvedOutputText),
}

impl MergeOpenBootstrapResolvedOutput {
    fn line_count(&self) -> usize {
        match self {
            Self::Projection(projection) => projection.len(),
            Self::Text(text) => text.line_count(),
        }
    }
}

fn hash_merge_open_bootstrap_state(
    segments: &[ConflictSegment],
    conflict_region_indices: &[usize],
    maps: &conflict_resolver::ThreeWayConflictMaps,
    split_row_index: &conflict_resolver::ConflictSplitRowIndex,
    two_way_split_projection: &conflict_resolver::TwoWaySplitProjection,
    three_way_visible_projection: &conflict_resolver::ThreeWayVisibleProjection,
    resolved_output: &MergeOpenBootstrapResolvedOutput,
) -> u64 {
    let mut h = FxHasher::default();
    segments.len().hash(&mut h);
    conflict_region_indices.len().hash(&mut h);
    maps.conflict_has_base.len().hash(&mut h);
    split_row_index.total_rows().hash(&mut h);
    two_way_split_projection.visible_len().hash(&mut h);
    three_way_visible_projection.len().hash(&mut h);

    match resolved_output {
        MergeOpenBootstrapResolvedOutput::Projection(projection) => {
            projection.len().hash(&mut h);
            projection.output_hash().hash(&mut h);
        }
        MergeOpenBootstrapResolvedOutput::Text(text) => {
            let bytes = text.as_str().as_bytes();
            text.line_count().hash(&mut h);
            bytes.len().hash(&mut h);
            for byte_ix in sampled_indices(bytes.len()) {
                bytes.get(byte_ix).copied().hash(&mut h);
            }
        }
    }

    for visible_ix in sampled_indices(two_way_split_projection.visible_len()) {
        if let Some((source_ix, conflict_ix)) = two_way_split_projection.get(visible_ix) {
            source_ix.hash(&mut h);
            conflict_ix.hash(&mut h);
            if let Some(row) = split_row_index.row_at(segments, source_ix) {
                std::mem::discriminant(&row.kind).hash(&mut h);
                row.old.as_deref().map(str::len).hash(&mut h);
                row.new.as_deref().map(str::len).hash(&mut h);
            }
        }
    }

    for visible_ix in sampled_indices(three_way_visible_projection.len()) {
        if let Some(item) = three_way_visible_projection.get(visible_ix) {
            match item {
                ThreeWayVisibleItem::Line(line_ix) => {
                    0u8.hash(&mut h);
                    line_ix.hash(&mut h);
                }
                ThreeWayVisibleItem::CollapsedBlock(conflict_ix) => {
                    1u8.hash(&mut h);
                    conflict_ix.hash(&mut h);
                }
            }
        }
    }

    if let MergeOpenBootstrapResolvedOutput::Projection(projection) = resolved_output {
        for line_ix in sampled_indices(projection.len()) {
            if let Some(line) = projection.line_text(segments, line_ix) {
                line_ix.hash(&mut h);
                line.len().hash(&mut h);
                line.as_bytes().first().copied().hash(&mut h);
                line.as_bytes().last().copied().hash(&mut h);
            }
        }
    }

    h.finish()
}

fn sampled_indices(len: usize) -> [usize; 3] {
    if len == 0 {
        return [0, 0, 0];
    }
    [0, len / 2, len - 1]
}

fn trace_line_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let bytes = text.as_bytes();
    let newline_count = memchr::memchr_iter(b'\n', bytes).count();
    if bytes.last() == Some(&b'\n') {
        newline_count
    } else {
        newline_count + 1
    }
}

fn build_synthetic_html_conflict_texts(
    total_lines: usize,
    requested_conflict_blocks: usize,
) -> (String, String, String, String) {
    let header_lines = [
        "<!doctype html>",
        "<html lang=\"en\">",
        "<body class=\"fixture-root\">",
    ];
    let total_lines = total_lines.max(header_lines.len().saturating_add(1));
    let mut base_lines = header_lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut ours_lines = base_lines.clone();
    let mut theirs_lines = base_lines.clone();
    let mut current_lines = base_lines.clone();
    let mut next_context_row = 0usize;

    if requested_conflict_blocks == 0 {
        append_synthetic_html_conflict_context(
            &mut base_lines,
            &mut ours_lines,
            &mut theirs_lines,
            &mut current_lines,
            &mut next_context_row,
            total_lines.saturating_sub(header_lines.len()),
        );
        return (
            base_lines.join("\n"),
            ours_lines.join("\n"),
            theirs_lines.join("\n"),
            current_lines.join("\n"),
        );
    }

    let max_conflicts = total_lines.saturating_sub(header_lines.len()).max(1);
    let conflict_blocks = requested_conflict_blocks.max(1).min(max_conflicts);
    let context_lines = total_lines
        .saturating_sub(header_lines.len())
        .saturating_sub(conflict_blocks);
    let context_slots = conflict_blocks;
    let context_per_slot = context_lines / context_slots;
    let context_remainder = context_lines % context_slots;

    for conflict_ix in 0..conflict_blocks {
        let base_line = format!(
            r#"<main id="choice-{conflict_ix}" data-side="base"><section class="panel panel-base">base {conflict_ix}</section></main>"#
        );
        let ours_line = format!(
            r#"<main id="choice-{conflict_ix}" data-side="ours"><section class="panel panel-ours">ours {conflict_ix}</section></main>"#
        );
        let theirs_line = format!(
            r#"<main id="choice-{conflict_ix}" data-side="theirs"><section class="panel panel-theirs">theirs {conflict_ix}</section></main>"#
        );

        base_lines.push(base_line);
        ours_lines.push(ours_line.clone());
        theirs_lines.push(theirs_line.clone());
        current_lines.push("<<<<<<< ours".to_string());
        current_lines.push(ours_line);
        current_lines.push("=======".to_string());
        current_lines.push(theirs_line);
        current_lines.push(">>>>>>> theirs".to_string());

        let slot_lines = context_per_slot + usize::from(conflict_ix < context_remainder);
        append_synthetic_html_conflict_context(
            &mut base_lines,
            &mut ours_lines,
            &mut theirs_lines,
            &mut current_lines,
            &mut next_context_row,
            slot_lines,
        );
    }

    assert_eq!(base_lines.len(), total_lines);
    assert_eq!(ours_lines.len(), total_lines);
    assert_eq!(theirs_lines.len(), total_lines);

    (
        base_lines.join("\n"),
        ours_lines.join("\n"),
        theirs_lines.join("\n"),
        current_lines.join("\n"),
    )
}

fn append_synthetic_html_conflict_context(
    base_lines: &mut Vec<String>,
    ours_lines: &mut Vec<String>,
    theirs_lines: &mut Vec<String>,
    current_lines: &mut Vec<String>,
    next_context_row: &mut usize,
    count: usize,
) {
    for _ in 0..count {
        let row = *next_context_row;
        let line = format!(
            r#"<section id="panel-{row}" data-row="{row}"><div class="copy">row {row}</div><span class="hint">shared html benchmark content</span></section>"#
        );
        base_lines.push(line.clone());
        ours_lines.push(line.clone());
        theirs_lines.push(line.clone());
        current_lines.push(line);
        *next_context_row = next_context_row.saturating_add(1);
    }
}

fn build_synthetic_three_way_segments(
    total_lines: usize,
    requested_conflict_blocks: usize,
) -> Vec<ConflictSegment> {
    let total_lines = total_lines.max(1);
    let conflict_blocks = requested_conflict_blocks.max(1).min(total_lines);
    let context_lines = total_lines.saturating_sub(conflict_blocks);
    let context_slots = conflict_blocks.saturating_add(1);
    let context_per_slot = context_lines / context_slots;
    let context_remainder = context_lines % context_slots;

    let mut segments: Vec<ConflictSegment> = Vec::with_capacity(conflict_blocks * 2 + 1);
    for slot_ix in 0..context_slots {
        let slot_lines = context_per_slot + usize::from(slot_ix < context_remainder);
        if slot_lines > 0 {
            let mut text = String::with_capacity(slot_lines * 64);
            for line_ix in 0..slot_lines {
                let seed = slot_ix * 1_000 + line_ix;
                let line = match seed % 5 {
                    0 => {
                        format!(
                            "fn ctx_{slot_ix}_{line_ix}(value: usize) -> usize {{ value + {seed} }}"
                        )
                    }
                    1 => format!("let ctx_{slot_ix}_{line_ix} = \"context line {seed}\";"),
                    2 => {
                        format!("if ctx_{slot_ix}_{line_ix}.len() > 3 {{ println!(\"{seed}\"); }}")
                    }
                    3 => format!("match opt_{slot_ix}_{line_ix} {{ Some(v) => v, None => 0 }}"),
                    _ => format!("// context {seed} repeated words for highlight coverage"),
                };
                text.push_str(&line);
                text.push('\n');
            }
            segments.push(ConflictSegment::Text(text.into()));
        }

        if slot_ix < conflict_blocks {
            let choice = match slot_ix % 4 {
                0 => ConflictChoice::Base,
                1 => ConflictChoice::Ours,
                2 => ConflictChoice::Theirs,
                _ => ConflictChoice::Both,
            };
            segments.push(ConflictSegment::Block(ConflictBlock {
                base: Some(format!("let shared_{slot_ix} = compute_base({slot_ix});\n").into()),
                ours: format!("let shared_{slot_ix} = compute_local({slot_ix});\n").into(),
                theirs: format!("let shared_{slot_ix} = compute_remote({slot_ix});\n").into(),
                choice,
                resolved: slot_ix % 5 == 0,
            }));
        }
    }

    segments
}

fn build_synthetic_two_way_segments(
    total_lines: usize,
    requested_conflict_blocks: usize,
) -> Vec<ConflictSegment> {
    let total_lines = total_lines.max(1);
    let conflict_blocks = requested_conflict_blocks.max(1).min(total_lines);
    let context_lines = total_lines.saturating_sub(conflict_blocks);
    let context_slots = conflict_blocks.saturating_add(1);
    let context_per_slot = context_lines / context_slots;
    let context_remainder = context_lines % context_slots;

    let mut segments: Vec<ConflictSegment> = Vec::with_capacity(conflict_blocks * 2 + 1);
    for slot_ix in 0..context_slots {
        let slot_lines = context_per_slot + usize::from(slot_ix < context_remainder);
        if slot_lines > 0 {
            let mut text = String::with_capacity(slot_lines * 64);
            for line_ix in 0..slot_lines {
                let seed = slot_ix * 1_000 + line_ix;
                let line = match seed % 5 {
                    0 => format!("fn ctx_{slot_ix}_{line_ix}() -> usize {{ {seed} }}"),
                    1 => format!("let ctx_{slot_ix}_{line_ix} = \"context line {seed}\";"),
                    2 => format!("if guard_{seed} {{ println!(\"{seed}\"); }}"),
                    3 => format!("match opt_{seed} {{ Some(v) => v, None => 0 }}"),
                    _ => format!("// context {seed} repeated words for highlight coverage"),
                };
                text.push_str(&line);
                text.push('\n');
            }
            segments.push(ConflictSegment::Text(text.into()));
        }

        if slot_ix < conflict_blocks {
            let (ours, theirs) = match slot_ix % 6 {
                0 => (
                    format!(
                        "let shared_{slot_ix} = compute_local({slot_ix});\nlet shared_{slot_ix}_tail = {slot_ix} + 1;\n"
                    ),
                    format!("let shared_{slot_ix} = compute_remote({slot_ix});\n"),
                ),
                1 => (
                    format!("let shared_{slot_ix} = compute_local({slot_ix});\n"),
                    format!(
                        "let shared_{slot_ix} = compute_remote({slot_ix});\nlet shared_{slot_ix}_tail = {slot_ix} + 2;\n"
                    ),
                ),
                _ => (
                    format!("let shared_{slot_ix} = compute_local({slot_ix});\n"),
                    format!("let shared_{slot_ix} = compute_remote({slot_ix});\n"),
                ),
            };
            let choice = match slot_ix % 3 {
                0 => ConflictChoice::Ours,
                1 => ConflictChoice::Theirs,
                _ => ConflictChoice::Both,
            };
            segments.push(ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: ours.into(),
                theirs: theirs.into(),
                choice,
                resolved: slot_ix % 7 == 0,
            }));
        }
    }

    segments
}

fn materialize_three_way_side_texts(segments: &[ConflictSegment]) -> (String, String, String) {
    let mut base = String::new();
    let mut ours = String::new();
    let mut theirs = String::new();
    for segment in segments {
        match segment {
            ConflictSegment::Text(text) => {
                base.push_str(text);
                ours.push_str(text);
                theirs.push_str(text);
            }
            ConflictSegment::Block(block) => {
                base.push_str(block.base.as_deref().unwrap_or_default());
                ours.push_str(&block.ours);
                theirs.push_str(&block.theirs);
            }
        }
    }
    (base, ours, theirs)
}

fn materialize_two_way_side_texts(segments: &[ConflictSegment]) -> (String, String) {
    let mut ours = String::new();
    let mut theirs = String::new();
    for segment in segments {
        match segment {
            ConflictSegment::Text(text) => {
                ours.push_str(text);
                theirs.push_str(text);
            }
            ConflictSegment::Block(block) => {
                ours.push_str(&block.ours);
                theirs.push_str(&block.theirs);
            }
        }
    }
    (ours, theirs)
}

fn materialize_resolved_output_with_block_ranges(
    segments: &[ConflictSegment],
) -> (String, Vec<Range<usize>>) {
    let mut output = String::new();
    let mut block_byte_ranges = Vec::new();

    for segment in segments {
        let start = output.len();
        match segment {
            ConflictSegment::Text(text) => output.push_str(text),
            ConflictSegment::Block(block) => {
                let rendered =
                    conflict_resolver::generate_resolved_text(&[ConflictSegment::Block(
                        block.clone(),
                    )]);
                output.push_str(&rendered);
                block_byte_ranges.push(start..output.len());
            }
        }
    }

    let block_ranges = block_byte_ranges
        .into_iter()
        .map(|byte_range| {
            let start_line = output[..byte_range.start]
                .bytes()
                .filter(|&byte| byte == b'\n')
                .count();
            let line_count = conflict_resolver::split_output_lines_for_outline(
                &output[byte_range.start..byte_range.end],
            )
            .len();
            start_line..start_line.saturating_add(line_count)
        })
        .collect();

    (output, block_ranges)
}

fn build_synthetic_resolved_output_markers(
    segments: &[ConflictSegment],
    block_ranges: &[Range<usize>],
    output_line_count: usize,
) -> Vec<Option<ResolvedOutputGutterMarker>> {
    let mut markers = vec![None; output_line_count];
    if output_line_count == 0 {
        return markers;
    }

    let mut block_ix = 0usize;
    for segment in segments {
        let ConflictSegment::Block(block) = segment else {
            continue;
        };
        let Some(range) = block_ranges.get(block_ix) else {
            break;
        };
        if range.start < range.end {
            let start = range.start.min(output_line_count);
            let end = range.end.min(output_line_count);
            for (line_ix, marker_slot) in markers.iter_mut().enumerate().take(end).skip(start) {
                *marker_slot = Some(ResolvedOutputGutterMarker {
                    conflict_ix: block_ix,
                    is_start: line_ix == range.start,
                    is_end: line_ix + 1 == range.end,
                    unresolved: !block.resolved,
                });
            }
        } else {
            let anchor = range.start.min(output_line_count.saturating_sub(1));
            markers[anchor] = Some(ResolvedOutputGutterMarker {
                conflict_ix: block_ix,
                is_start: true,
                is_end: true,
                unresolved: !block.resolved,
            });
        }
        block_ix = block_ix.saturating_add(1);
    }

    markers
}

fn split_lines_shared(text: &str) -> Vec<SharedString> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(text.as_bytes().iter().filter(|&&b| b == b'\n').count() + 1);
    out.extend(text.lines().map(|line| line.to_string().into()));
    out
}

/// Build a single whole-file conflict block with mixed content patterns.
///
/// Ours and theirs share ~60% of lines (anchors), with ~20% insertions in ours
/// and ~20% insertions in theirs. This gives the anchor index meaningful work.
fn build_synthetic_whole_file_conflict_segments(total_lines: usize) -> Vec<ConflictSegment> {
    let total_lines = total_lines.max(10);
    let mut ours = String::with_capacity(total_lines * 80);
    let mut theirs = String::with_capacity(total_lines * 80);

    // Generate shared base lines, with periodic ours-only and theirs-only insertions.
    let mut shared_ix = 0usize;
    let mut line_ix = 0usize;
    while line_ix < total_lines {
        let phase = shared_ix % 10;
        match phase {
            // Shared lines (6 out of 10 phases = ~60% shared)
            0 | 1 | 3 | 5 | 7 | 9 => {
                let line =
                    format!("fn shared_{shared_ix}(x: usize) -> usize {{ x + {shared_ix} }}\n");
                ours.push_str(&line);
                theirs.push_str(&line);
                line_ix += 1;
            }
            // Ours-only insertion (2 out of 10 = ~20%)
            2 | 6 => {
                let line = format!("let ours_only_{shared_ix} = compute_local({shared_ix});\n");
                ours.push_str(&line);
                line_ix += 1;
            }
            // Theirs-only insertion (2 out of 10 = ~20%)
            4 | 8 => {
                let line = format!("let theirs_only_{shared_ix} = compute_remote({shared_ix});\n");
                theirs.push_str(&line);
                line_ix += 1;
            }
            _ => unreachable!(),
        }
        shared_ix += 1;
    }

    vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: ours.into(),
        theirs: theirs.into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })]
}
