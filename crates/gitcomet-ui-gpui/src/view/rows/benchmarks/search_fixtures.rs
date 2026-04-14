use super::*;

pub struct ScrollbarDragStepFixture {
    /// Height of the visible viewport in pixels.
    viewport_h: f32,
    /// Maximum scroll offset (content_height - viewport_height).
    max_offset: f32,
    /// Current vertical scroll offset.
    scroll_y: f32,
    /// Track top Y position.
    track_top: f32,
    /// Track height (viewport_h - 2*margin).
    track_h: f32,
    /// Pixel step per drag event along the track.
    drag_step_px: f32,
    /// Current direction (+1.0 = down, -1.0 = up).
    drag_direction: f32,
    /// Number of drag steps per benchmark iteration.
    steps: usize,
}

/// Sidecar metrics for scrollbar drag step benchmarks.
pub struct ScrollbarDragStepMetrics {
    pub steps: u64,
    pub thumb_metric_recomputes: u64,
    pub scroll_offset_recomputes: u64,
    pub viewport_h: f64,
    pub max_offset: f64,
    pub min_scroll_y: f64,
    pub max_scroll_y: f64,
    pub min_thumb_offset_px: f64,
    pub max_thumb_offset_px: f64,
    pub min_thumb_length_px: f64,
    pub max_thumb_length_px: f64,
    pub clamp_at_top_count: u64,
    pub clamp_at_bottom_count: u64,
}

impl ScrollbarDragStepFixture {
    /// Create a fixture simulating 200 scrollbar-drag steps in a realistic
    /// viewport over a long list.
    ///
    /// Viewport: 800 px (≈33 rows at 24 px each).
    /// Content: 10,000 rows × 24 px = 240,000 px.
    /// Track: 800 − 2×4 margin = 792 px.
    /// Drag step: 12 px → 200 steps = 2,400 px ≈ 3× track traversals,
    /// guaranteeing multiple oscillation reversals at both ends.
    pub fn window_200() -> Self {
        let row_height = 24.0_f32;
        let total_rows = 10_000;
        let viewport_h = 800.0_f32;
        let content_h = row_height * total_rows as f32;
        let max_offset = content_h - viewport_h;
        let margin = 4.0_f32;
        let track_h = viewport_h - margin * 2.0;

        Self {
            viewport_h,
            max_offset,
            scroll_y: 0.0,
            track_top: margin,
            track_h,
            drag_step_px: 12.0,
            drag_direction: 1.0,
            steps: 200,
        }
    }

    pub fn run(&mut self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&mut self) -> (u64, ScrollbarDragStepMetrics) {
        use crate::kit::{compute_vertical_click_offset, vertical_thumb_metrics};
        use gpui::{Bounds, point, px, size};

        let mut h = FxHasher::default();
        let mut min_scroll_y = f64::MAX;
        let mut max_scroll_y = f64::MIN;
        let mut min_thumb_offset = f64::MAX;
        let mut max_thumb_offset = f64::MIN;
        let mut min_thumb_length = f64::MAX;
        let mut max_thumb_length = f64::MIN;
        let mut clamp_at_top: u64 = 0;
        let mut clamp_at_bottom: u64 = 0;
        let mut thumb_metric_recomputes: u64 = 0;
        let mut scroll_offset_recomputes: u64 = 0;

        // Build a synthetic track bounds matching the scrollbar layout:
        // track starts at (0, margin) with width=16 and height=track_h.
        let track_bounds = Bounds::new(
            point(px(0.0), px(self.track_top)),
            size(px(16.0), px(self.track_h)),
        );

        // Start with the current mouse Y at the thumb centre.
        let initial_thumb =
            vertical_thumb_metrics(px(self.viewport_h), px(self.max_offset), px(self.scroll_y));
        let mut mouse_y = match initial_thumb {
            Some(tm) => {
                let off: f32 = tm.offset.into();
                let len: f32 = tm.length.into();
                off + len / 2.0
            }
            None => self.track_top + self.track_h / 2.0,
        };

        for _ in 0..self.steps {
            // 1) Compute thumb metrics at the current scroll position.
            let thumb =
                vertical_thumb_metrics(px(self.viewport_h), px(self.max_offset), px(self.scroll_y));
            thumb_metric_recomputes = thumb_metric_recomputes.saturating_add(1);

            let (thumb_size, thumb_length_f, thumb_offset_f) = match thumb {
                Some(tm) => {
                    let len: f32 = tm.length.into();
                    let off: f32 = tm.offset.into();
                    (tm.length, len, off)
                }
                None => (px(24.0), 24.0_f32, self.track_top),
            };

            min_thumb_offset = min_thumb_offset.min(thumb_offset_f as f64);
            max_thumb_offset = max_thumb_offset.max(thumb_offset_f as f64);
            min_thumb_length = min_thumb_length.min(thumb_length_f as f64);
            max_thumb_length = max_thumb_length.max(thumb_length_f as f64);

            // 2) Advance simulated mouse position.
            mouse_y += self.drag_step_px * self.drag_direction;

            // Clamp to track bounds.
            let track_bottom = self.track_top + self.track_h;
            if mouse_y <= self.track_top {
                mouse_y = self.track_top;
                clamp_at_top += 1;
                self.drag_direction = -self.drag_direction;
            } else if mouse_y >= track_bottom {
                mouse_y = track_bottom;
                clamp_at_bottom += 1;
                self.drag_direction = -self.drag_direction;
            }

            // 3) Compute new scroll offset using the production offset function.
            //    Use thumb_size/2 as the grab offset (simulating grab at thumb centre).
            let new_offset = compute_vertical_click_offset(
                px(mouse_y),
                track_bounds,
                thumb_size,
                thumb_size / 2.0,
                px(self.max_offset),
                -1, // negative sign matches the default GPUI scroll direction
            );
            scroll_offset_recomputes = scroll_offset_recomputes.saturating_add(1);

            // The function returns a negative offset for sign=-1, take abs.
            let new_scroll: f32 = (-new_offset).into();
            self.scroll_y = new_scroll.max(0.0).min(self.max_offset);

            min_scroll_y = min_scroll_y.min(self.scroll_y as f64);
            max_scroll_y = max_scroll_y.max(self.scroll_y as f64);

            // Hash to prevent dead-code elimination.
            self.scroll_y.to_bits().hash(&mut h);
            thumb_offset_f.to_bits().hash(&mut h);
            thumb_length_f.to_bits().hash(&mut h);
            self.drag_direction.to_bits().hash(&mut h);
        }

        let metrics = ScrollbarDragStepMetrics {
            steps: self.steps as u64,
            thumb_metric_recomputes,
            scroll_offset_recomputes,
            viewport_h: self.viewport_h as f64,
            max_offset: self.max_offset as f64,
            min_scroll_y,
            max_scroll_y,
            min_thumb_offset_px: min_thumb_offset,
            max_thumb_offset_px: max_thumb_offset,
            min_thumb_length_px: min_thumb_length,
            max_thumb_length_px: max_thumb_length,
            clamp_at_top_count: clamp_at_top,
            clamp_at_bottom_count: clamp_at_bottom,
        };

        (h.finish(), metrics)
    }

    #[cfg(test)]
    pub(super) fn current_scroll_y(&self) -> f32 {
        self.scroll_y
    }
}

// ---------------------------------------------------------------------------
// Search / commit filter benchmarks (Phase 4)
// ---------------------------------------------------------------------------

/// Metrics emitted as sidecar JSON for commit search/filter benchmarks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CommitSearchFilterMetrics {
    pub total_commits: u64,
    pub query_len: u64,
    pub matches_found: u64,
    /// Cost of incremental refinement: filtering the already-matched subset
    /// with one additional character appended to the query.
    pub incremental_matches: u64,
}

/// Benchmark fixture for filtering a large synthetic commit list by author
/// or message substring. Exercises the core scan that any future commit
/// search UI would perform.
pub struct CommitSearchFilterFixture {
    commits: Vec<Commit>,
    /// Pre-lowercased author strings for the author-filter path.
    authors_lower: Vec<String>,
    /// Pre-lowercased summaries for the message-filter path.
    summaries_lower: Vec<String>,
}

impl CommitSearchFilterFixture {
    /// Build a fixture with `count` synthetic commits distributed across
    /// 100 distinct authors and varied commit messages.
    pub fn new(count: usize) -> Self {
        let commits = build_synthetic_commits_for_search(count);
        let authors_lower: Vec<String> = commits.iter().map(|c| c.author.to_lowercase()).collect();
        let summaries_lower: Vec<String> =
            commits.iter().map(|c| c.summary.to_lowercase()).collect();
        Self {
            commits,
            authors_lower,
            summaries_lower,
        }
    }

    /// Filter commits whose author field contains `query` (case-insensitive).
    /// Returns a hash to prevent dead-code elimination.
    pub fn run_filter_by_author(&self, query: &str) -> u64 {
        let query_lower = query.to_lowercase();
        let finder = memchr::memmem::Finder::new(query_lower.as_bytes());
        let mut h = FxHasher::default();
        let mut count = 0u64;
        for author in &self.authors_lower {
            if finder.find(author.as_bytes()).is_some() {
                count += 1;
            }
        }
        count.hash(&mut h);
        h.finish()
    }

    /// Filter commits whose summary field contains `query` (case-insensitive).
    /// Returns a hash to prevent dead-code elimination.
    pub fn run_filter_by_message(&self, query: &str) -> u64 {
        let query_lower = query.to_lowercase();
        let finder = memchr::memmem::Finder::new(query_lower.as_bytes());
        let mut h = FxHasher::default();
        let mut count = 0u64;
        for summary in &self.summaries_lower {
            if finder.find(summary.as_bytes()).is_some() {
                count += 1;
            }
        }
        count.hash(&mut h);
        h.finish()
    }

    /// Filter by author and collect full metrics including incremental
    /// refinement (appending one character to the query).
    pub fn run_filter_by_author_with_metrics(
        &self,
        query: &str,
    ) -> (u64, CommitSearchFilterMetrics) {
        let query_lower = query.to_lowercase();
        let finder = memchr::memmem::Finder::new(query_lower.as_bytes());
        let mut h = FxHasher::default();
        let mut matches = Vec::new();
        for (ix, author) in self.authors_lower.iter().enumerate() {
            if finder.find(author.as_bytes()).is_some() {
                matches.push(ix);
            }
        }
        let matches_found = matches.len() as u64;
        matches_found.hash(&mut h);

        // Incremental refinement: append 'x' and re-filter only the matched subset.
        let refined_query = format!("{query_lower}x");
        let refined_finder = memchr::memmem::Finder::new(refined_query.as_bytes());
        let mut incremental_matches = 0u64;
        for &ix in &matches {
            if refined_finder
                .find(self.authors_lower[ix].as_bytes())
                .is_some()
            {
                incremental_matches += 1;
            }
        }
        incremental_matches.hash(&mut h);

        (
            h.finish(),
            CommitSearchFilterMetrics {
                total_commits: self.commits.len() as u64,
                query_len: query.len() as u64,
                matches_found,
                incremental_matches,
            },
        )
    }

    /// Filter by message and collect full metrics including incremental
    /// refinement (appending one character to the query).
    pub fn run_filter_by_message_with_metrics(
        &self,
        query: &str,
    ) -> (u64, CommitSearchFilterMetrics) {
        let query_lower = query.to_lowercase();
        let finder = memchr::memmem::Finder::new(query_lower.as_bytes());
        let mut h = FxHasher::default();
        let mut matches = Vec::new();
        for (ix, summary) in self.summaries_lower.iter().enumerate() {
            if finder.find(summary.as_bytes()).is_some() {
                matches.push(ix);
            }
        }
        let matches_found = matches.len() as u64;
        matches_found.hash(&mut h);

        // Incremental refinement: append 'x' and re-filter only the matched subset.
        let refined_query = format!("{query_lower}x");
        let refined_finder = memchr::memmem::Finder::new(refined_query.as_bytes());
        let mut incremental_matches = 0u64;
        for &ix in &matches {
            if refined_finder
                .find(self.summaries_lower[ix].as_bytes())
                .is_some()
            {
                incremental_matches += 1;
            }
        }
        incremental_matches.hash(&mut h);

        (
            h.finish(),
            CommitSearchFilterMetrics {
                total_commits: self.commits.len() as u64,
                query_len: query.len() as u64,
                matches_found,
                incremental_matches,
            },
        )
    }

    /// Number of commits in the fixture.
    #[cfg(test)]
    pub fn commit_count(&self) -> usize {
        self.commits.len()
    }

    /// Number of distinct authors in the fixture.
    #[cfg(test)]
    pub fn distinct_authors(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        for author in &self.authors_lower {
            seen.insert(author.as_str());
        }
        seen.len()
    }

    #[cfg(test)]
    pub fn distinct_message_trigrams(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        for summary in &self.summaries_lower {
            for trigram in summary.as_bytes().windows(3) {
                seen.insert(<[u8; 3]>::try_from(trigram).expect("3-byte trigram"));
            }
        }
        seen.len()
    }
}

/// Metrics emitted as sidecar JSON for in-diff text search benchmarks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InDiffTextSearchMetrics {
    pub total_lines: u64,
    pub visible_rows_scanned: u64,
    pub query_len: u64,
    pub matches_found: u64,
    /// Prior broad-query matches when measuring a refined follow-up query.
    pub prior_matches: u64,
}

/// Benchmark fixture for scanning a large synthetic unified diff with the same
/// ASCII-case-insensitive substring semantics as the production diff search.
///
/// The broad query (`render_cache`) matches both context rows and modified
/// rows, while the refined query (`render_cache_hot_path`) narrows to a smaller
/// subset of modified rows without changing the overall scan cost.
pub struct InDiffTextSearchFixture {
    diff: Arc<Diff>,
    visible_line_indices: Box<[usize]>,
    total_lines: usize,
    visible_rows: usize,
}

impl InDiffTextSearchFixture {
    pub fn new(lines: usize) -> Self {
        let total_lines = lines.max(1);
        let target = DiffTarget::WorkingTree {
            path: std::path::PathBuf::from("src/lib.rs"),
            area: DiffArea::Unstaged,
        };
        let text = build_synthetic_diff_search_unified_patch(total_lines);
        let diff = Arc::new(Diff::from_unified(target, text.as_str()));
        let visible_line_indices = diff
            .lines
            .iter()
            .enumerate()
            .filter_map(|(ix, line)| {
                (!should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()))
                    .then_some(ix)
            })
            .collect::<Vec<_>>();
        let visible_rows = visible_line_indices.len();

        Self {
            diff,
            visible_line_indices: visible_line_indices.into_boxed_slice(),
            total_lines,
            visible_rows,
        }
    }

    pub fn run_search(&self, query: &str) -> u64 {
        self.scan_matches(query).0
    }

    pub fn prepare_matches(&self, query: &str) -> Vec<usize> {
        let Some(query) = AsciiCaseInsensitiveNeedle::new(query.trim()) else {
            return Vec::new();
        };

        let mut matches = Vec::with_capacity((self.visible_rows / 16).max(1));
        for (visible_ix, &line_ix) in self.visible_line_indices.iter().enumerate() {
            if query.is_match(self.diff.lines[line_ix].text.as_ref()) {
                matches.push(visible_ix);
            }
        }
        matches
    }

    pub fn run_refinement_from_matches(&self, query: &str, prior_matches: &[usize]) -> u64 {
        self.scan_candidate_matches(query, prior_matches).0
    }

    pub fn run_search_with_metrics(&self, query: &str) -> (u64, InDiffTextSearchMetrics) {
        let (hash, matches_found) = self.scan_matches(query);
        (
            hash,
            InDiffTextSearchMetrics {
                total_lines: bench_counter_u64(self.total_lines),
                visible_rows_scanned: bench_counter_u64(self.visible_rows),
                query_len: query.trim().len() as u64,
                matches_found,
                prior_matches: 0,
            },
        )
    }

    pub fn run_refinement_from_matches_with_metrics(
        &self,
        query: &str,
        prior_matches: &[usize],
    ) -> (u64, InDiffTextSearchMetrics) {
        let (hash, matches_found) = self.scan_candidate_matches(query, prior_matches);
        (
            hash,
            InDiffTextSearchMetrics {
                total_lines: bench_counter_u64(self.total_lines),
                visible_rows_scanned: bench_counter_u64(self.visible_rows),
                query_len: query.trim().len() as u64,
                matches_found,
                prior_matches: bench_counter_u64(prior_matches.len()),
            },
        )
    }

    pub fn run_refinement_with_metrics(
        &self,
        broad_query: &str,
        refined_query: &str,
    ) -> (u64, InDiffTextSearchMetrics) {
        let prior_matches = self.prepare_matches(broad_query);
        self.run_refinement_from_matches_with_metrics(refined_query, &prior_matches)
    }

    fn scan_matches(&self, query: &str) -> (u64, u64) {
        let Some(query) = AsciiCaseInsensitiveNeedle::new(query.trim()) else {
            return (0, 0);
        };

        let mut h = FxHasher::default();
        let mut matches_found = 0u64;

        for (visible_ix, &line_ix) in self.visible_line_indices.iter().enumerate() {
            let line = &self.diff.lines[line_ix];
            if query.is_match(line.text.as_ref()) {
                visible_ix.hash(&mut h);
                line.text.len().hash(&mut h);
                matches_found = matches_found.saturating_add(1);
            }
        }

        matches_found.hash(&mut h);
        self.visible_rows.hash(&mut h);
        (h.finish(), matches_found)
    }

    fn scan_candidate_matches(&self, query: &str, prior_matches: &[usize]) -> (u64, u64) {
        let Some(query) = AsciiCaseInsensitiveNeedle::new(query.trim()) else {
            return (0, 0);
        };

        let mut h = FxHasher::default();
        let mut matches_found = 0u64;

        for &visible_ix in prior_matches {
            let Some(&line_ix) = self.visible_line_indices.get(visible_ix) else {
                continue;
            };
            let line = self.diff.lines[line_ix].text.as_ref();
            if query.is_match(line) {
                visible_ix.hash(&mut h);
                line.len().hash(&mut h);
                matches_found = matches_found.saturating_add(1);
            }
        }

        matches_found.hash(&mut h);
        self.visible_rows.hash(&mut h);
        (h.finish(), matches_found)
    }

    #[cfg(test)]
    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    #[cfg(test)]
    pub fn visible_rows(&self) -> usize {
        self.visible_rows
    }
}

/// Metrics emitted as sidecar JSON for file-preview `Ctrl+F` search
/// benchmarks. This follows the production path that scans reconstructed
/// preview source text line by line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FilePreviewTextSearchMetrics {
    pub total_lines: u64,
    pub source_bytes: u64,
    pub query_len: u64,
    pub matches_found: u64,
    pub prior_matches: u64,
}

/// Benchmark fixture for the file-preview `Ctrl+F` search path in
/// `diff_search_recompute_matches_for_current_view()`.
pub struct FilePreviewTextSearchFixture {
    source_text: SharedString,
    line_starts: Arc<[usize]>,
    total_lines: usize,
}

impl FilePreviewTextSearchFixture {
    pub fn new(lines: usize) -> Self {
        let total_lines = lines.max(1);
        let preview_lines = build_synthetic_file_preview_search_lines(total_lines);
        let source_len = preview_lines
            .iter()
            .map(String::len)
            .sum::<usize>()
            .saturating_add(preview_lines.len().saturating_sub(1));
        let (source_text, line_starts) =
            crate::view::panes::main::preview_source_text_and_line_starts_from_lines(
                &preview_lines,
                source_len,
            );

        Self {
            source_text,
            line_starts,
            total_lines,
        }
    }

    pub fn run_search(&self, query: &str) -> u64 {
        self.scan_matches(query).0
    }

    pub fn run_search_with_metrics(&self, query: &str) -> (u64, FilePreviewTextSearchMetrics) {
        let (hash, matches_found) = self.scan_matches(query);
        (
            hash,
            FilePreviewTextSearchMetrics {
                total_lines: bench_counter_u64(self.total_lines),
                source_bytes: bench_counter_u64(self.source_text.len()),
                query_len: query.trim().len() as u64,
                matches_found,
                prior_matches: 0,
            },
        )
    }

    pub fn run_refinement_with_metrics(
        &self,
        broad_query: &str,
        refined_query: &str,
    ) -> (u64, FilePreviewTextSearchMetrics) {
        let (_, prior_matches) = self.scan_matches(broad_query);
        let (hash, matches_found) = self.scan_matches(refined_query);
        (
            hash,
            FilePreviewTextSearchMetrics {
                total_lines: bench_counter_u64(self.total_lines),
                source_bytes: bench_counter_u64(self.source_text.len()),
                query_len: refined_query.trim().len() as u64,
                matches_found,
                prior_matches,
            },
        )
    }

    fn scan_matches(&self, query: &str) -> (u64, u64) {
        let Some(query) = AsciiCaseInsensitiveNeedle::new(query.trim()) else {
            return (0, 0);
        };

        let mut h = FxHasher::default();
        let mut matches_found = 0u64;

        for line_ix in 0..self.total_lines {
            let line = super::diff_text::resolved_output_line_text(
                self.source_text.as_ref(),
                self.line_starts.as_ref(),
                line_ix,
            );
            if query.is_match(line) {
                line_ix.hash(&mut h);
                line.len().hash(&mut h);
                matches_found = matches_found.saturating_add(1);
            }
        }

        matches_found.hash(&mut h);
        self.total_lines.hash(&mut h);
        self.source_text.len().hash(&mut h);
        (h.finish(), matches_found)
    }

    #[cfg(test)]
    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    #[cfg(test)]
    pub fn source_bytes(&self) -> usize {
        self.source_text.len()
    }
}

/// Metrics emitted as sidecar JSON for the split file-diff `Ctrl+F` search path.
///
/// This models the user-visible sequence in the large file-diff view:
/// 1. open the search input with `Ctrl+F`
/// 2. type a query one character at a time
/// 3. reuse prior matches on refinements instead of rescanning every row
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileDiffCtrlFOpenTypeMetrics {
    pub total_lines: u64,
    pub total_rows: u64,
    pub visible_window_rows: u64,
    pub search_opened: u64,
    pub typed_chars: u64,
    pub query_steps: u64,
    pub final_query_len: u64,
    pub rows_scanned: u64,
    pub full_rescans: u64,
    pub refinement_steps: u64,
    pub final_matches: u64,
}

/// Benchmark fixture for the large split file-diff `Ctrl+F` search path.
///
/// It mirrors `activate_diff_search()` plus repeated
/// `diff_search_recompute_matches_for_query_change()` updates on the split
/// file-diff view. The first visible window is prewarmed to match a diff that
/// is already open before the user hits `Ctrl+F`.
pub struct FileDiffCtrlFOpenTypeFixture {
    split: Arc<PagedFileDiffRows>,
    total_lines: usize,
    visible_window_rows: usize,
}

impl FileDiffCtrlFOpenTypeFixture {
    pub fn new(lines: usize, visible_window_rows: usize) -> Self {
        let total_lines = lines.max(1);
        let visible_window_rows = visible_window_rows.max(1);
        let (old_text, new_text) = build_synthetic_file_diff_search_texts(total_lines);
        #[cfg(feature = "benchmarks")]
        let (split, _inline) = build_bench_file_diff_rebuild_from_text(
            "src/bench_file_diff_search.rs",
            &old_text,
            &new_text,
        );
        #[cfg(not(feature = "benchmarks"))]
        let (split, _inline) =
            unreachable!("FileDiffCtrlFOpenTypeFixture requires benchmarks feature");

        let warm_rows = visible_window_rows.min(split.len_hint());
        let _ = split.slice(0, warm_rows).take(warm_rows).count();

        Self {
            split,
            total_lines,
            visible_window_rows: warm_rows,
        }
    }

    pub fn run_open_and_type(&self, final_query: &str) -> u64 {
        self.run_open_and_type_with_metrics(final_query).0
    }

    pub fn run_open_and_type_with_metrics(
        &self,
        final_query: &str,
    ) -> (u64, FileDiffCtrlFOpenTypeMetrics) {
        let final_query = final_query.trim();
        let total_rows = self.split.len_hint();
        let typed_chars = final_query.chars().count();
        let mut current_query = String::with_capacity(final_query.len());
        let mut previous_query = String::new();
        let mut matches: Vec<usize> = Vec::new();
        let mut rows_scanned = 0u64;
        let mut full_rescans = 0u64;
        let mut refinement_steps = 0u64;

        let mut h = FxHasher::default();
        true.hash(&mut h);
        total_rows.hash(&mut h);

        for ch in final_query.chars() {
            current_query.push(ch);
            let Some(query) = AsciiCaseInsensitiveNeedle::new(current_query.as_str()) else {
                continue;
            };

            match diff_search_query_reuse(previous_query.as_str(), current_query.as_str()) {
                DiffSearchQueryReuse::SameSemantics => {}
                DiffSearchQueryReuse::Refinement => {
                    refinement_steps = refinement_steps.saturating_add(1);
                    let mut next_matches = Vec::with_capacity(matches.len());
                    for &row_ix in &matches {
                        rows_scanned = rows_scanned.saturating_add(1);
                        if self.row_matches_query(row_ix, query) {
                            next_matches.push(row_ix);
                        }
                    }
                    matches = next_matches;
                }
                DiffSearchQueryReuse::None => {
                    full_rescans = full_rescans.saturating_add(1);
                    matches.clear();
                    matches.reserve((total_rows / 16).max(1));
                    for row_ix in 0..total_rows {
                        rows_scanned = rows_scanned.saturating_add(1);
                        if self.row_matches_query(row_ix, query) {
                            matches.push(row_ix);
                        }
                    }
                }
            }

            current_query.len().hash(&mut h);
            matches.len().hash(&mut h);
            matches.first().hash(&mut h);
            matches.last().hash(&mut h);

            previous_query.clear();
            previous_query.push_str(&current_query);
        }

        (
            h.finish(),
            FileDiffCtrlFOpenTypeMetrics {
                total_lines: bench_counter_u64(self.total_lines),
                total_rows: bench_counter_u64(total_rows),
                visible_window_rows: bench_counter_u64(self.visible_window_rows),
                search_opened: 1,
                typed_chars: bench_counter_u64(typed_chars),
                query_steps: bench_counter_u64(typed_chars),
                final_query_len: final_query.len() as u64,
                rows_scanned,
                full_rescans,
                refinement_steps,
                final_matches: bench_counter_u64(matches.len()),
            },
        )
    }

    fn row_matches_query(&self, row_ix: usize, query: AsciiCaseInsensitiveNeedle<'_>) -> bool {
        let Some(row) = self.split.row(row_ix) else {
            return false;
        };
        row.old.as_deref().is_some_and(|text| query.is_match(text))
            || row.new.as_deref().is_some_and(|text| query.is_match(text))
    }

    #[cfg(test)]
    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    #[cfg(test)]
    pub fn total_rows(&self) -> usize {
        self.split.len_hint()
    }

    #[cfg(test)]
    pub fn visible_window_rows(&self) -> usize {
        self.visible_window_rows
    }
}

// ---------------------------------------------------------------------------
// file_fuzzy_find — file-picker fuzzy search over large path corpora
// ---------------------------------------------------------------------------

/// Sidecar metrics emitted by `FileFuzzyFindFixture`.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileFuzzyFindMetrics {
    /// Total number of file paths in the corpus.
    pub total_files: u64,
    /// Length of the query string.
    pub query_len: u64,
    /// Number of paths that matched the query.
    pub matches_found: u64,
    /// Number of matches from a prior (shorter) query — used for incremental keystroke.
    pub prior_matches: u64,
    /// Total number of candidate paths scanned across the measured search pass(es).
    pub files_scanned: u64,
}

struct FileFuzzyFindPath {
    len: usize,
    lowercase_bytes: Box<[u8]>,
    /// Bitmap of which lowercase ASCII letters appear in the path.
    /// Bit `i` is set if byte `b'a' + i` is present (0 ≤ i < 26).
    char_bitmap: u32,
}

impl FileFuzzyFindPath {
    fn new(path: String) -> Self {
        let len = path.len();
        let mut lowercase_bytes = path.into_bytes();
        lowercase_bytes.make_ascii_lowercase();
        let mut char_bitmap = 0u32;
        for &b in lowercase_bytes.iter() {
            if b.is_ascii_lowercase() {
                char_bitmap |= 1 << (b - b'a');
            }
        }
        Self {
            len,
            lowercase_bytes: lowercase_bytes.into_boxed_slice(),
            char_bitmap,
        }
    }
}

#[derive(Clone, Copy)]
struct FileFuzzyFindMatchCandidate {
    index: usize,
    next_start: usize,
}

struct AsciiCaseInsensitiveSubsequenceNeedle {
    lowercase_bytes: Box<[u8]>,
    /// Bitmap of required lowercase ASCII letters.
    /// Bit `i` is set if byte `b'a' + i` appears in the needle (0 ≤ i < 26).
    required_bitmap: u32,
}

impl AsciiCaseInsensitiveSubsequenceNeedle {
    #[inline]
    fn new(needle: &str) -> Option<Self> {
        let bytes = needle.as_bytes();
        let _ = bytes.first()?;

        let mut lowercase_bytes = Vec::with_capacity(bytes.len());
        let mut required_bitmap = 0u32;
        for &byte in bytes {
            let lower = byte.to_ascii_lowercase();
            lowercase_bytes.push(lower);
            if lower.is_ascii_lowercase() {
                required_bitmap |= 1 << (lower - b'a');
            }
        }
        Some(Self {
            lowercase_bytes: lowercase_bytes.into_boxed_slice(),
            required_bitmap,
        })
    }

    #[inline]
    fn is_match(&self, haystack: &[u8]) -> bool {
        self.match_end(haystack).is_some()
    }

    #[inline]
    fn match_end(&self, haystack: &[u8]) -> Option<usize> {
        lowercase_subsequence_match_end(haystack, &self.lowercase_bytes)
    }

    #[inline]
    fn as_bytes(&self) -> &[u8] {
        &self.lowercase_bytes
    }

    #[inline]
    fn is_strict_extension_of(&self, prefix: &[u8]) -> bool {
        self.lowercase_bytes.len() > prefix.len() && self.lowercase_bytes.starts_with(prefix)
    }
}

#[inline]
fn lowercase_subsequence_match_end(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }

    let mut offset = 0usize;
    for &needle_byte in needle {
        let remaining = &haystack[offset..];
        match memchr::memchr(needle_byte, remaining) {
            Some(pos) => offset += pos + 1,
            None => return None,
        }
    }

    Some(offset)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FileFuzzyFindRunResult {
    hash: u64,
    matches_found: u64,
    files_scanned: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FileFuzzyFindIncrementalRunResult {
    hash: u64,
    matches_found: u64,
    prior_matches: u64,
    files_scanned: u64,
}

/// Benchmark fixture for fuzzy-finding file paths in a large synthetic corpus.
///
/// Simulates the production file-picker workflow: the user types a query and
/// the UI filters a flat list of file paths using subsequence matching (each
/// character of the query must appear in order in the candidate path,
/// case-insensitively). The corpus is built once with deterministic,
/// realistic-looking paths covering varied directory depths and extensions.
pub struct FileFuzzyFindFixture {
    paths: Vec<FileFuzzyFindPath>,
    match_candidates_scratch: RefCell<Vec<FileFuzzyFindMatchCandidate>>,
    total_files: usize,
}

impl FileFuzzyFindFixture {
    pub fn new(file_count: usize) -> Self {
        let total_files = file_count.max(1);
        let paths = build_synthetic_file_path_corpus(total_files)
            .into_iter()
            .map(FileFuzzyFindPath::new)
            .collect();
        Self {
            paths,
            match_candidates_scratch: RefCell::new(Vec::with_capacity((total_files / 3).max(1))),
            total_files,
        }
    }

    pub fn run_find(&self, query: &str) -> u64 {
        self.scan_matches(query).hash
    }

    pub fn run_incremental(&self, short_query: &str, long_query: &str) -> u64 {
        self.scan_incremental_matches(short_query, long_query).hash
    }

    pub fn run_find_with_metrics(&self, query: &str) -> (u64, FileFuzzyFindMetrics) {
        let query = query.trim();
        let run = self.scan_matches(query);
        (
            run.hash,
            FileFuzzyFindMetrics {
                total_files: bench_counter_u64(self.total_files),
                query_len: bench_counter_u64(query.len()),
                matches_found: run.matches_found,
                prior_matches: 0,
                files_scanned: run.files_scanned,
            },
        )
    }

    pub fn run_incremental_with_metrics(
        &self,
        short_query: &str,
        long_query: &str,
    ) -> (u64, FileFuzzyFindMetrics) {
        let long_query = long_query.trim();
        let run = self.scan_incremental_matches(short_query, long_query);
        (
            run.hash,
            FileFuzzyFindMetrics {
                total_files: bench_counter_u64(self.total_files),
                query_len: bench_counter_u64(long_query.len()),
                matches_found: run.matches_found,
                prior_matches: run.prior_matches,
                files_scanned: run.files_scanned,
            },
        )
    }

    #[cfg(test)]
    pub fn run_find_without_ordered_pair_prefilter(&self, query: &str) -> u64 {
        self.run_find(query)
    }

    fn scan_matches(&self, query: &str) -> FileFuzzyFindRunResult {
        let Some(query) = AsciiCaseInsensitiveSubsequenceNeedle::new(query.trim()) else {
            return FileFuzzyFindRunResult {
                hash: 0,
                matches_found: bench_counter_u64(self.paths.len()),
                files_scanned: 0,
            };
        };

        self.scan_all_matches(&query)
    }

    fn scan_incremental_matches(
        &self,
        short_query: &str,
        long_query: &str,
    ) -> FileFuzzyFindIncrementalRunResult {
        let short_query = short_query.trim();
        let long_query = long_query.trim();
        let Some(short_needle) = AsciiCaseInsensitiveSubsequenceNeedle::new(short_query) else {
            let run = self.scan_matches(long_query);
            return FileFuzzyFindIncrementalRunResult {
                hash: run.hash,
                matches_found: run.matches_found,
                prior_matches: bench_counter_u64(self.paths.len()),
                files_scanned: run.files_scanned,
            };
        };

        let mut prior_match_candidates = self.match_candidates_scratch.borrow_mut();
        let prior_matches =
            self.collect_match_candidates(&short_needle, &mut prior_match_candidates);
        let refined_run = match AsciiCaseInsensitiveSubsequenceNeedle::new(long_query) {
            Some(long_needle) if long_needle.is_strict_extension_of(short_needle.as_bytes()) => {
                self.scan_extended_candidate_matches(
                    &long_needle,
                    short_needle.as_bytes().len(),
                    prior_match_candidates.as_slice(),
                )
            }
            Some(long_needle) => self.scan_all_matches(&long_needle),
            None => FileFuzzyFindRunResult {
                hash: 0,
                matches_found: bench_counter_u64(self.paths.len()),
                files_scanned: 0,
            },
        };

        FileFuzzyFindIncrementalRunResult {
            hash: refined_run.hash,
            matches_found: refined_run.matches_found,
            prior_matches,
            files_scanned: bench_counter_u64(self.total_files)
                .saturating_add(refined_run.files_scanned),
        }
    }

    fn scan_all_matches(
        &self,
        query: &AsciiCaseInsensitiveSubsequenceNeedle,
    ) -> FileFuzzyFindRunResult {
        let mut h = FxHasher::default();
        let mut matches_found = 0u64;
        let req = query.required_bitmap;

        for (ix, path) in self.paths.iter().enumerate() {
            if path.char_bitmap & req != req {
                continue;
            }
            if query.is_match(path.lowercase_bytes.as_ref()) {
                ix.hash(&mut h);
                path.len.hash(&mut h);
                matches_found = matches_found.saturating_add(1);
            }
        }

        matches_found.hash(&mut h);
        self.total_files.hash(&mut h);
        FileFuzzyFindRunResult {
            hash: h.finish(),
            matches_found,
            files_scanned: bench_counter_u64(self.total_files),
        }
    }

    fn scan_extended_candidate_matches(
        &self,
        query: &AsciiCaseInsensitiveSubsequenceNeedle,
        prefix_len: usize,
        candidate_matches: &[FileFuzzyFindMatchCandidate],
    ) -> FileFuzzyFindRunResult {
        let mut h = FxHasher::default();
        let mut matches_found = 0u64;
        let suffix = &query.as_bytes()[prefix_len..];

        for candidate in candidate_matches {
            let path = &self.paths[candidate.index];
            let matches = suffix.is_empty()
                || lowercase_subsequence_match_end(
                    &path.lowercase_bytes[candidate.next_start..],
                    suffix,
                )
                .is_some();
            if matches {
                candidate.index.hash(&mut h);
                path.len.hash(&mut h);
                matches_found = matches_found.saturating_add(1);
            }
        }

        matches_found.hash(&mut h);
        self.total_files.hash(&mut h);
        FileFuzzyFindRunResult {
            hash: h.finish(),
            matches_found,
            files_scanned: bench_counter_u64(candidate_matches.len()),
        }
    }

    fn collect_match_candidates(
        &self,
        query: &AsciiCaseInsensitiveSubsequenceNeedle,
        out: &mut Vec<FileFuzzyFindMatchCandidate>,
    ) -> u64 {
        out.clear();
        let req = query.required_bitmap;
        for (ix, path) in self.paths.iter().enumerate() {
            if path.char_bitmap & req != req {
                continue;
            }
            if let Some(next_start) = query.match_end(path.lowercase_bytes.as_ref()) {
                out.push(FileFuzzyFindMatchCandidate {
                    index: ix,
                    next_start,
                });
            }
        }
        bench_counter_u64(out.len())
    }

    #[cfg(test)]
    pub fn total_files(&self) -> usize {
        self.total_files
    }
}

/// Build a deterministic corpus of `count` synthetic file paths with realistic
/// directory depths (1–6 segments), varied extensions, and reproducible names.
fn build_synthetic_file_path_corpus(count: usize) -> Vec<String> {
    let dirs_l0 = [
        "src", "crates", "lib", "tests", "benches", "docs", "tools", "scripts", "config", "assets",
    ];
    let dirs_l1 = [
        "core", "ui", "model", "view", "utils", "cache", "render", "state", "events", "layout",
    ];
    let dirs_l2 = [
        "rows",
        "panels",
        "panes",
        "widgets",
        "handlers",
        "traits",
        "builders",
        "parsers",
        "formatters",
        "providers",
    ];
    let stems = [
        "main",
        "app",
        "config",
        "history",
        "diff_cache",
        "branch",
        "commit",
        "status",
        "merge",
        "conflict",
        "search",
        "render",
        "layout",
        "sidebar",
        "toolbar",
        "popover",
        "dialog",
        "input",
        "scroll",
        "resize",
    ];
    let exts = [
        "rs", "ts", "tsx", "js", "json", "toml", "yaml", "md", "css", "html",
    ];

    let mut paths = Vec::with_capacity(count);
    for ix in 0..count {
        let d0 = dirs_l0[ix % dirs_l0.len()];
        let d1 = dirs_l1[(ix / dirs_l0.len()) % dirs_l1.len()];
        let depth = (ix % 6) + 1;
        let stem = stems[(ix / 7) % stems.len()];
        let ext = exts[(ix / 3) % exts.len()];
        let suffix = ix;

        let path = match depth {
            1 => format!("{d0}/{stem}_{suffix}.{ext}"),
            2 => format!("{d0}/{d1}/{stem}_{suffix}.{ext}"),
            3 => {
                let d2 = dirs_l2[(ix / 100) % dirs_l2.len()];
                format!("{d0}/{d1}/{d2}/{stem}_{suffix}.{ext}")
            }
            4 => {
                let d2 = dirs_l2[(ix / 100) % dirs_l2.len()];
                let sub = format!("sub_{}", ix % 50);
                format!("{d0}/{d1}/{d2}/{sub}/{stem}_{suffix}.{ext}")
            }
            5 => {
                let d2 = dirs_l2[(ix / 100) % dirs_l2.len()];
                let sub = format!("sub_{}", ix % 50);
                let deep = format!("deep_{}", ix % 20);
                format!("{d0}/{d1}/{d2}/{sub}/{deep}/{stem}_{suffix}.{ext}")
            }
            _ => {
                let d2 = dirs_l2[(ix / 100) % dirs_l2.len()];
                let sub = format!("sub_{}", ix % 50);
                let deep = format!("deep_{}", ix % 20);
                let leaf = format!("leaf_{}", ix % 10);
                format!("{d0}/{d1}/{d2}/{sub}/{deep}/{leaf}/{stem}_{suffix}.{ext}")
            }
        };
        paths.push(path);
    }
    paths
}

/// Build synthetic commits with richer author and message diversity for
/// search benchmarks. Uses 100 distinct authors and varied message prefixes
/// so that substring queries have realistic selectivity.
fn build_synthetic_commits_for_search(count: usize) -> Vec<Commit> {
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let prefixes = [
        "fix", "feat", "refactor", "chore", "docs", "test", "perf", "ci", "style", "build",
    ];
    let areas = [
        "history view",
        "diff cache",
        "branch sidebar",
        "merge tool",
        "status panel",
        "commit details",
        "repo tabs",
        "settings",
        "theme engine",
        "search",
    ];
    let mut commits = Vec::with_capacity(count);
    for ix in 0..count {
        let id = CommitId(format!("{:040x}", ix).into());
        let mut parent_ids = Vec::new();
        if ix > 0 {
            parent_ids.push(CommitId(format!("{:040x}", ix - 1).into()));
        }

        // 100 distinct authors: "Alice Anderson", "Bob Baker", ..., cycling
        // through 10 first names × 10 last names.
        let first_names = [
            "Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Hank", "Ivy", "Jack",
        ];
        let last_names = [
            "Anderson", "Baker", "Chen", "Davis", "Evans", "Foster", "Garcia", "Hill", "Ito",
            "Jones",
        ];
        let author = format!("{} {}", first_names[ix % 10], last_names[(ix / 10) % 10]);

        let prefix = prefixes[ix % prefixes.len()];
        let area = areas[(ix / prefixes.len()) % areas.len()];
        let summary: Arc<str> = format!("{prefix}: update {area} for commit {ix}").into();

        commits.push(Commit {
            id,
            parent_ids: parent_ids.into(),
            summary,
            author: author.into(),
            time: base + Duration::from_secs(ix as u64),
        });
    }
    commits
}

fn build_synthetic_diff_search_unified_patch(line_count: usize) -> String {
    let line_count = line_count.max(1);
    let mut out = String::new();
    out.push_str("diff --git a/src/lib.rs b/src/lib.rs\n");
    out.push_str("index 3333333..4444444 100644\n");
    out.push_str("--- a/src/lib.rs\n");
    out.push_str("+++ b/src/lib.rs\n");
    out.push_str(&format!(
        "@@ -1,{line_count} +1,{line_count} @@ fn synthetic_diff_search_fixture() {{\n"
    ));

    for ix in 0..line_count {
        if ix % 64 == 0 {
            out.push_str(&format!(
                "-let render_cache_old_{ix} = old_cache_lookup({ix});\n"
            ));
            out.push_str(&format!(
                "+let render_cache_hot_path_{ix} = hot_cache_lookup({ix});\n"
            ));
        } else if ix % 16 == 0 {
            out.push_str(&format!(
                " let render_cache_probe_{ix} = inspect_cache({ix});\n"
            ));
        } else if ix % 7 == 0 {
            out.push_str(&format!("-let old_{ix} = old_call({ix});\n"));
            out.push_str(&format!("+let new_{ix} = new_call({ix});\n"));
        } else {
            out.push_str(&format!(" let stable_line_{ix} = keep({ix});\n"));
        }
    }

    out
}

fn build_synthetic_file_preview_search_lines(line_count: usize) -> Vec<String> {
    let line_count = line_count.max(1);
    let mut lines = Vec::with_capacity(line_count);
    for ix in 0..line_count {
        let line = if ix % 64 == 0 {
            format!("let render_cache_hot_path_{ix} = hot_cache_lookup({ix}); // preview search")
        } else if ix % 16 == 0 {
            format!("let render_cache_probe_{ix} = inspect_cache({ix});")
        } else if ix % 7 == 0 {
            format!("let stable_line_{ix} = keep({ix}); // wrapped preview line")
        } else {
            format!("let stable_line_{ix} = keep({ix});")
        };
        lines.push(line);
    }
    lines
}

fn build_synthetic_file_diff_search_texts(line_count: usize) -> (String, String) {
    let line_count = line_count.max(1);
    let mut old_text = String::with_capacity(line_count * 64);
    let mut new_text = String::with_capacity(line_count * 64);

    for ix in 0..line_count {
        if ix % 64 == 0 {
            old_text.push_str(&format!(
                "let render_cache_old_{ix} = old_cache_lookup({ix});\n"
            ));
            new_text.push_str(&format!(
                "let render_cache_hot_path_{ix} = hot_cache_lookup({ix});\n"
            ));
        } else if ix % 16 == 0 {
            let shared = format!("let render_cache_probe_{ix} = inspect_cache({ix});\n");
            old_text.push_str(&shared);
            new_text.push_str(&shared);
        } else if ix % 7 == 0 {
            old_text.push_str(&format!("let old_{ix} = old_call({ix});\n"));
            new_text.push_str(&format!("let new_{ix} = new_call({ix});\n"));
        } else {
            let shared = format!("let stable_line_{ix} = keep({ix});\n");
            old_text.push_str(&shared);
            new_text.push_str(&shared);
        }
    }

    (old_text, new_text)
}
