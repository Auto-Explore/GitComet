use super::*;
use gitcomet_core::diff::annotate_unified;

fn build_synthetic_unified_patch(line_count: usize) -> String {
    let line_count = line_count.max(1);
    let mut out = String::new();
    out.push_str("diff --git a/src/lib.rs b/src/lib.rs\n");
    out.push_str("index 1111111..2222222 100644\n");
    out.push_str("--- a/src/lib.rs\n");
    out.push_str("+++ b/src/lib.rs\n");
    out.push_str(&format!(
        "@@ -1,{} +1,{} @@ fn synthetic() {{\n",
        line_count.saturating_mul(2),
        line_count.saturating_mul(2)
    ));

    for ix in 0..line_count {
        if ix % 7 == 0 {
            out.push_str(&format!("-let old_{ix} = old_call({ix});\n"));
            out.push_str(&format!("+let new_{ix} = new_call({ix});\n"));
        } else {
            out.push_str(&format!(" let shared_{ix} = keep({ix});\n"));
        }
    }
    out
}

pub(crate) fn should_hide_unified_diff_header_for_bench(kind: DiffLineKind, text: &str) -> bool {
    matches!(kind, DiffLineKind::Header)
        && (text.starts_with("index ") || text.starts_with("--- ") || text.starts_with("+++ "))
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PatchDiffFirstWindowMetrics {
    pub rows_requested: u64,
    pub patch_rows_painted: u64,
    pub patch_rows_materialized: u64,
    pub patch_page_cache_entries: u64,
    pub split_rows_painted: u64,
    pub split_rows_materialized: u64,
    pub full_text_materializations: u64,
}

pub struct PatchDiffPagedRowsFixture {
    diff: Arc<Diff>,
    hidden_flags: Vec<bool>,
    split_row_count: usize,
}

impl PatchDiffPagedRowsFixture {
    pub fn new(lines: usize) -> Self {
        let target = DiffTarget::WorkingTree {
            path: std::path::PathBuf::from("src/lib.rs"),
            area: DiffArea::Unstaged,
        };
        let text = build_synthetic_unified_patch(lines);
        let diff = Arc::new(Diff::from_unified(target, text.as_str()));
        let mut pending_removes = 0usize;
        let mut pending_adds = 0usize;
        let mut split_row_count = 0usize;
        let hidden_flags = diff
            .lines
            .iter()
            .map(|line| {
                match line.kind {
                    DiffLineKind::Remove => pending_removes += 1,
                    DiffLineKind::Add => pending_adds += 1,
                    DiffLineKind::Context | DiffLineKind::Header | DiffLineKind::Hunk => {
                        split_row_count += pending_removes.max(pending_adds) + 1;
                        pending_removes = 0;
                        pending_adds = 0;
                    }
                }
                should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref())
            })
            .collect();
        split_row_count += pending_removes.max(pending_adds);
        Self {
            diff,
            hidden_flags,
            split_row_count,
        }
    }

    pub fn run_eager_full_materialize_step(&self) -> u64 {
        let annotated = annotate_unified(&self.diff);
        let split = build_patch_split_rows(&annotated);
        let theme = AppTheme::gitcomet_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");
        let mut hasher = FxHasher::default();
        annotated.len().hash(&mut hasher);
        split.len().hash(&mut hasher);
        for line in annotated.iter().take(256) {
            let kind_key: u8 = match line.kind {
                DiffLineKind::Header => 0,
                DiffLineKind::Hunk => 1,
                DiffLineKind::Add => 2,
                DiffLineKind::Remove => 3,
                DiffLineKind::Context => 4,
            };
            kind_key.hash(&mut hasher);
            line.text.len().hash(&mut hasher);
            line.old_line.hash(&mut hasher);
            line.new_line.hash(&mut hasher);
        }
        for row in split.iter().take(256) {
            match row {
                PatchSplitRow::Raw { src_ix, click_kind } => {
                    src_ix.hash(&mut hasher);
                    let click_kind_key: u8 = match click_kind {
                        DiffClickKind::Line => 0,
                        DiffClickKind::HunkHeader => 1,
                        DiffClickKind::FileHeader => 2,
                    };
                    click_kind_key.hash(&mut hasher);
                }
                PatchSplitRow::Aligned {
                    row,
                    old_src_ix,
                    new_src_ix,
                } => {
                    let kind_key: u8 = match row.kind {
                        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                    };
                    kind_key.hash(&mut hasher);
                    row.old_line.hash(&mut hasher);
                    row.new_line.hash(&mut hasher);
                    row.old.as_ref().map(|s| s.len()).hash(&mut hasher);
                    row.new.as_ref().map(|s| s.len()).hash(&mut hasher);
                    old_src_ix.hash(&mut hasher);
                    new_src_ix.hash(&mut hasher);
                }
            }
        }
        for line in &annotated {
            if !matches!(
                line.kind,
                DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
            ) {
                continue;
            }
            let styled = super::diff_text::build_cached_diff_styled_text(
                theme,
                diff_content_text(line),
                &[],
                "",
                language,
                DiffSyntaxMode::HeuristicOnly,
                None,
            );
            styled.text.len().hash(&mut hasher);
            styled.highlights.len().hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn run_paged_first_window_step(&self, window: usize) -> u64 {
        let window = window.max(1);
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&self.diff), 256));
        let split_provider = PagedPatchSplitRows::new_with_len_hint(
            Arc::clone(&rows_provider),
            self.split_row_count,
        );
        let theme = AppTheme::gitcomet_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");

        let mut hasher = FxHasher::default();
        rows_provider.len_hint().hash(&mut hasher);
        split_provider.len_hint().hash(&mut hasher);

        for line in rows_provider.slice(0, window).take(window) {
            let kind_key: u8 = match line.kind {
                DiffLineKind::Header => 0,
                DiffLineKind::Hunk => 1,
                DiffLineKind::Add => 2,
                DiffLineKind::Remove => 3,
                DiffLineKind::Context => 4,
            };
            kind_key.hash(&mut hasher);
            line.text.len().hash(&mut hasher);
            line.old_line.hash(&mut hasher);
            line.new_line.hash(&mut hasher);
            if matches!(
                line.kind,
                DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
            ) {
                let content_text = diff_content_text(&line);
                let styled = super::diff_text::build_cached_diff_styled_text_with_source_identity(
                    theme,
                    content_text,
                    Some(super::diff_text::DiffTextSourceIdentity::from_str(
                        content_text,
                    )),
                    &[],
                    "",
                    language,
                    DiffSyntaxMode::HeuristicOnly,
                    None,
                );
                styled.text.len().hash(&mut hasher);
                styled.highlights.len().hash(&mut hasher);
            }
        }
        for row in split_provider.slice(0, window).take(window) {
            match row {
                PatchSplitRow::Raw { src_ix, click_kind } => {
                    src_ix.hash(&mut hasher);
                    let click_kind_key: u8 = match click_kind {
                        DiffClickKind::Line => 0,
                        DiffClickKind::HunkHeader => 1,
                        DiffClickKind::FileHeader => 2,
                    };
                    click_kind_key.hash(&mut hasher);
                }
                PatchSplitRow::Aligned {
                    row,
                    old_src_ix,
                    new_src_ix,
                } => {
                    let kind_key: u8 = match row.kind {
                        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                    };
                    kind_key.hash(&mut hasher);
                    row.old_line.hash(&mut hasher);
                    row.new_line.hash(&mut hasher);
                    row.old.as_ref().map(|s| s.len()).hash(&mut hasher);
                    row.new.as_ref().map(|s| s.len()).hash(&mut hasher);
                    old_src_ix.hash(&mut hasher);
                    new_src_ix.hash(&mut hasher);
                }
            }
        }

        hasher.finish()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn measure_paged_first_window_step(&self, window: usize) -> PatchDiffFirstWindowMetrics {
        let window = window.max(1);
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&self.diff), 256));
        let split_provider = PagedPatchSplitRows::new_with_len_hint(
            Arc::clone(&rows_provider),
            self.split_row_count,
        );

        let patch_rows_painted = rows_provider.slice(0, window).take(window).count();
        let split_rows_painted = split_provider.slice(0, window).take(window).count();

        PatchDiffFirstWindowMetrics {
            rows_requested: bench_counter_u64(window),
            patch_rows_painted: bench_counter_u64(patch_rows_painted),
            patch_rows_materialized: bench_counter_u64(rows_provider.materialized_row_count()),
            patch_page_cache_entries: bench_counter_u64(rows_provider.cached_page_count()),
            split_rows_painted: bench_counter_u64(split_rows_painted),
            split_rows_materialized: bench_counter_u64(split_provider.materialized_row_count()),
            full_text_materializations: 0,
        }
    }

    pub fn run_inline_visible_eager_scan_step(&self) -> u64 {
        let rows_provider = PagedPatchDiffRows::new(Arc::clone(&self.diff), 256);
        let mut visible_indices = Vec::new();
        for (src_ix, line) in rows_provider.slice(0, rows_provider.len_hint()).enumerate() {
            if !should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()) {
                visible_indices.push(src_ix);
            }
        }

        let mut hasher = FxHasher::default();
        visible_indices.len().hash(&mut hasher);
        for src_ix in visible_indices.into_iter().take(512) {
            src_ix.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn run_inline_visible_hidden_map_step(&self) -> u64 {
        let visible_map = PatchInlineVisibleMap::from_hidden_flags(self.hidden_flags.as_slice());

        let mut hasher = FxHasher::default();
        visible_map.visible_len().hash(&mut hasher);
        for visible_ix in 0..visible_map.visible_len().min(512) {
            visible_map
                .src_ix_for_visible_ix(visible_ix)
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    #[cfg(test)]
    pub(crate) fn inline_visible_indices_eager(&self) -> Vec<usize> {
        self.diff
            .lines
            .iter()
            .enumerate()
            .filter_map(|(src_ix, line)| {
                (!should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()))
                    .then_some(src_ix)
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn inline_visible_indices_map(&self) -> Vec<usize> {
        let visible_map = PatchInlineVisibleMap::from_hidden_flags(self.hidden_flags.as_slice());
        (0..visible_map.visible_len())
            .filter_map(|visible_ix| visible_map.src_ix_for_visible_ix(visible_ix))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn total_rows(&self) -> usize {
        self.diff.lines.len()
    }

    /// Total row count hint for benchmark use (deep-window offset calculation).
    #[cfg(feature = "benchmarks")]
    pub fn total_rows_hint(&self) -> usize {
        self.diff.lines.len()
    }

    /// Like `run_paged_first_window_step` but starts at `start_row` (patch
    /// offset).  The split provider offset is scaled to 90% of its own
    /// `len_hint()` to avoid indexing past the end.  Used for deep-scroll
    /// benchmarks.
    pub fn run_paged_window_at_step(&self, start_row: usize, window: usize) -> u64 {
        let window = window.max(1);
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&self.diff), 256));
        let split_provider = PagedPatchSplitRows::new_with_len_hint(
            Arc::clone(&rows_provider),
            self.split_row_count,
        );
        let theme = AppTheme::gitcomet_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");

        // Compute per-provider deep offsets clamped to valid range.
        let patch_start = start_row.min(rows_provider.len_hint().saturating_sub(window).max(0));
        let split_start = split_provider
            .len_hint()
            .saturating_mul(9)
            .checked_div(10)
            .unwrap_or(0)
            .min(split_provider.len_hint().saturating_sub(window));

        let mut hasher = FxHasher::default();
        rows_provider.len_hint().hash(&mut hasher);
        split_provider.len_hint().hash(&mut hasher);
        patch_start.hash(&mut hasher);

        for line in rows_provider
            .slice(patch_start, patch_start + window)
            .take(window)
        {
            let kind_key: u8 = match line.kind {
                DiffLineKind::Header => 0,
                DiffLineKind::Hunk => 1,
                DiffLineKind::Add => 2,
                DiffLineKind::Remove => 3,
                DiffLineKind::Context => 4,
            };
            kind_key.hash(&mut hasher);
            line.text.len().hash(&mut hasher);
            line.old_line.hash(&mut hasher);
            line.new_line.hash(&mut hasher);
            if matches!(
                line.kind,
                DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
            ) {
                let content_text = diff_content_text(&line);
                let styled = super::diff_text::build_cached_diff_styled_text_with_source_identity(
                    theme,
                    content_text,
                    Some(super::diff_text::DiffTextSourceIdentity::from_str(
                        content_text,
                    )),
                    &[],
                    "",
                    language,
                    DiffSyntaxMode::HeuristicOnly,
                    None,
                );
                styled.text.len().hash(&mut hasher);
                styled.highlights.len().hash(&mut hasher);
            }
        }
        for row in split_provider
            .slice(split_start, split_start + window)
            .take(window)
        {
            match row {
                PatchSplitRow::Raw { src_ix, click_kind } => {
                    src_ix.hash(&mut hasher);
                    let click_kind_key: u8 = match click_kind {
                        DiffClickKind::Line => 0,
                        DiffClickKind::HunkHeader => 1,
                        DiffClickKind::FileHeader => 2,
                    };
                    click_kind_key.hash(&mut hasher);
                }
                PatchSplitRow::Aligned {
                    row,
                    old_src_ix,
                    new_src_ix,
                } => {
                    let kind_key: u8 = match row.kind {
                        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                    };
                    kind_key.hash(&mut hasher);
                    row.old_line.hash(&mut hasher);
                    row.new_line.hash(&mut hasher);
                    row.old.as_ref().map(|s| s.len()).hash(&mut hasher);
                    row.new.as_ref().map(|s| s.len()).hash(&mut hasher);
                    old_src_ix.hash(&mut hasher);
                    new_src_ix.hash(&mut hasher);
                }
            }
        }

        hasher.finish()
    }

    /// Collect sidecar metrics for a deep-window paging run.
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn measure_paged_deep_window_step(
        &self,
        start_row: usize,
        window: usize,
    ) -> PatchDiffFirstWindowMetrics {
        let window = window.max(1);
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&self.diff), 256));
        let split_provider = PagedPatchSplitRows::new_with_len_hint(
            Arc::clone(&rows_provider),
            self.split_row_count,
        );

        let patch_start = start_row.min(rows_provider.len_hint().saturating_sub(window));
        let split_start = split_provider
            .len_hint()
            .saturating_mul(9)
            .checked_div(10)
            .unwrap_or(0)
            .min(split_provider.len_hint().saturating_sub(window));

        let patch_rows_painted = rows_provider
            .slice(patch_start, patch_start + window)
            .take(window)
            .count();
        let split_rows_painted = split_provider
            .slice(split_start, split_start + window)
            .take(window)
            .count();

        PatchDiffFirstWindowMetrics {
            rows_requested: bench_counter_u64(window),
            patch_rows_painted: bench_counter_u64(patch_rows_painted),
            patch_rows_materialized: bench_counter_u64(rows_provider.materialized_row_count()),
            patch_page_cache_entries: bench_counter_u64(rows_provider.cached_page_count()),
            split_rows_painted: bench_counter_u64(split_rows_painted),
            split_rows_materialized: bench_counter_u64(split_provider.materialized_row_count()),
            full_text_materializations: 0,
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn bench_counter_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

// ---------------------------------------------------------------------------
// diff_refresh_rev_only_same_content — rekey vs rebuild benchmark
// ---------------------------------------------------------------------------

/// Sidecar metrics emitted by `DiffRefreshFixture`.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiffRefreshMetrics {
    /// Number of rekey-fast-path invocations (should be 1 per same-content refresh).
    pub diff_cache_rekeys: u64,
    /// Number of full rebuilds (should be 0 for same-content refresh).
    pub full_rebuilds: u64,
    /// Whether the content signature matched (1 = yes, 0 = no).
    pub content_signature_matches: u64,
    /// Row count preserved by the rekey path (same as initial row count when content unchanged).
    pub rows_preserved: u64,
    /// Split row count after a full rebuild.
    pub rebuild_rows: u64,
    /// Inline row count after a full rebuild.
    pub rebuild_inline_rows: u64,
    /// Old-side document bytes rebuilt into the cache.
    pub old_text_bytes: u64,
    /// New-side document bytes rebuilt into the cache.
    pub new_text_bytes: u64,
    /// Old-side line-start entries rebuilt into the cache.
    pub old_line_starts: u64,
    /// New-side line-start entries rebuilt into the cache.
    pub new_line_starts: u64,
}

/// Benchmark fixture for `diff_refresh_rev_only_same_content`.
///
/// Simulates the file diff cache fast path: when a store-side refresh bumps
/// `diff_file_rev` with an identical file payload, the cache should rekey its
/// prepared syntax documents and reuse the existing row provider instead of
/// performing an expensive `side_by_side_plan` + row provider rebuild.
///
/// Two benchmark sub-cases:
/// - **rekey**: compute content signature, compare, bump rev (the fast path).
/// - **rebuild**: the synchronous file-diff cache rebuild through document
///   source cloning, line-start indexing, `side_by_side_plan`, and split/inline
///   row-provider construction (the slow path before syntax refresh).
pub struct DiffRefreshFixture {
    incoming_file: gitcomet_core::domain::FileDiffText,
    /// Content signature from initial build, precomputed on `FileDiffText`.
    initial_signature: u64,
    /// Row count from the initial side-by-side plan.
    initial_plan_row_count: usize,
}

impl DiffRefreshFixture {
    /// Create a fixture with synthetic old/new file text.
    ///
    /// `old_lines` controls the file size.  Every 7th line is modified in the
    /// "new" version to produce a realistic mix of context and change runs.
    pub fn new(old_lines: usize) -> Self {
        let old_lines = old_lines.max(10);
        let mut old_text = String::with_capacity(old_lines * 40);
        let mut new_text = String::with_capacity(old_lines * 40);
        for i in 0..old_lines {
            if i % 7 == 0 {
                old_text.push_str(&format!("let old_{i} = old_call({i});\n"));
                new_text.push_str(&format!("let new_{i} = new_call({i});\n"));
            } else {
                let shared = format!("let shared_{i} = keep({i});\n");
                old_text.push_str(&shared);
                new_text.push_str(&shared);
            }
        }
        let incoming_file = gitcomet_core::domain::FileDiffText::new(
            std::path::PathBuf::from("src/bench_diff_refresh.rs"),
            Some(old_text.clone()),
            Some(new_text.clone()),
        );
        let initial_signature = incoming_file.content_signature();
        let plan = gitcomet_core::file_diff::side_by_side_plan(&old_text, &new_text);
        let initial_plan_row_count = plan.row_count;

        Self {
            incoming_file,
            initial_signature,
            initial_plan_row_count,
        }
    }

    /// **Rekey path**: reads the precomputed content signature of an identical
    /// payload and verifies it matches the cached signature. Returns a
    /// deterministic hash to prevent dead-code elimination.
    ///
    /// This mirrors the fast path in `ensure_file_diff_cache` where
    /// `file_content_signature == self.file_diff_cache_content_signature`.
    pub fn run_rekey_step(&self) -> u64 {
        let incoming_signature = std::hint::black_box(self.incoming_file.content_signature());
        let cached_signature = std::hint::black_box(self.initial_signature);

        // The real code checks `same_repo_and_target && signature == cached`.
        // Simulate that comparison cost.
        let matched = incoming_signature == cached_signature;

        let mut hasher = FxHasher::default();
        matched.hash(&mut hasher);
        incoming_signature.hash(&mut hasher);
        // In the real code this path also increments the rev counter and
        // possibly re-resolves syntax document keys.  We simulate that by
        // hashing the plan row count (which stays unchanged).
        std::hint::black_box(self.initial_plan_row_count).hash(&mut hasher);
        hasher.finish()
    }

    /// **Rebuild path**: performs the full `side_by_side_plan` + plan scan
    /// that would occur when the content actually changes.
    pub fn run_rebuild_step(&self) -> u64 {
        #[cfg(feature = "benchmarks")]
        let rebuild = crate::view::panes::main::diff_cache::build_file_diff_cache_rebuild(
            &self.incoming_file,
            std::path::Path::new("/tmp/gitcomet-bench-diff-refresh"),
        );
        #[cfg(not(feature = "benchmarks"))]
        let rebuild =
            unreachable!("DiffRefreshFixture::run_rebuild_step requires benchmarks feature");

        let mut hasher = FxHasher::default();
        bench_counter_u64(rebuild.row_provider.len_hint()).hash(&mut hasher);
        bench_counter_u64(rebuild.inline_row_provider.len_hint()).hash(&mut hasher);
        bench_counter_u64(rebuild.old_text.len()).hash(&mut hasher);
        bench_counter_u64(rebuild.new_text.len()).hash(&mut hasher);
        bench_counter_u64(rebuild.old_line_starts.len()).hash(&mut hasher);
        bench_counter_u64(rebuild.new_line_starts.len()).hash(&mut hasher);
        hasher.finish()
    }

    /// Collect sidecar metrics for the same-content refresh.
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn measure_rekey(&self) -> DiffRefreshMetrics {
        let incoming_signature = self.incoming_file.content_signature();
        let matched = incoming_signature == self.initial_signature;
        DiffRefreshMetrics {
            diff_cache_rekeys: if matched { 1 } else { 0 },
            full_rebuilds: 0,
            content_signature_matches: if matched { 1 } else { 0 },
            rows_preserved: bench_counter_u64(self.initial_plan_row_count),
            rebuild_rows: 0,
            rebuild_inline_rows: 0,
            old_text_bytes: 0,
            new_text_bytes: 0,
            old_line_starts: 0,
            new_line_starts: 0,
        }
    }

    /// Collect sidecar metrics for the full-rebuild path (content changed).
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn measure_rebuild(&self) -> DiffRefreshMetrics {
        #[cfg(feature = "benchmarks")]
        let rebuild = crate::view::panes::main::diff_cache::build_file_diff_cache_rebuild(
            &self.incoming_file,
            std::path::Path::new("/tmp/gitcomet-bench-diff-refresh"),
        );
        #[cfg(not(feature = "benchmarks"))]
        let rebuild =
            unreachable!("DiffRefreshFixture::measure_rebuild requires benchmarks feature");
        DiffRefreshMetrics {
            diff_cache_rekeys: 0,
            full_rebuilds: 1,
            content_signature_matches: 0,
            rows_preserved: 0,
            rebuild_rows: bench_counter_u64(rebuild.row_provider.len_hint()),
            rebuild_inline_rows: bench_counter_u64(rebuild.inline_row_provider.len_hint()),
            old_text_bytes: bench_counter_u64(rebuild.old_text.len()),
            new_text_bytes: bench_counter_u64(rebuild.new_text.len()),
            old_line_starts: bench_counter_u64(rebuild.old_line_starts.len()),
            new_line_starts: bench_counter_u64(rebuild.new_line_starts.len()),
        }
    }
}

// ---------------------------------------------------------------------------
// File diff open fixtures (split / inline first window)
// ---------------------------------------------------------------------------

/// Sidecar metrics for `diff_open_file_split_first_window` and
/// `diff_open_file_inline_first_window` benchmarks.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileDiffOpenMetrics {
    pub rows_requested: u64,
    pub split_total_rows: u64,
    pub split_rows_painted: u64,
    pub inline_total_rows: u64,
    pub inline_rows_painted: u64,
}

/// Benchmark fixture for `diff_open_file_split_first_window/N` and
/// `diff_open_file_inline_first_window/N`.
///
/// Constructs synthetic old/new file text with every 7th line modified,
/// builds a `side_by_side_plan`, and creates paged row providers.  The
/// benchmark measures the cost of materializing the first visible window
/// of split (side-by-side) or inline rows — the dominant cost when a user
/// opens a file diff.
pub struct FileDiffOpenFixture {
    split: std::sync::Arc<crate::view::panes::main::diff_cache::PagedFileDiffRows>,
    inline: std::sync::Arc<crate::view::panes::main::diff_cache::PagedFileDiffInlineRows>,
}

impl FileDiffOpenFixture {
    /// Create a fixture with `old_lines` lines in the old file.
    /// Every 7th line is modified in the new version.
    pub fn new(old_lines: usize) -> Self {
        let old_lines = old_lines.max(10);
        let mut old_text = String::with_capacity(old_lines * 80);
        let mut new_text = String::with_capacity(old_lines * 80);
        for i in 0..old_lines {
            if i % 7 == 0 {
                old_text.push_str(&format!("let old_{i} = old_call({i});\n"));
                new_text.push_str(&format!("let new_{i} = new_call({i});\n"));
            } else {
                let shared = format!("let shared_{i} = keep({i});\n");
                old_text.push_str(&shared);
                new_text.push_str(&shared);
            }
        }
        #[cfg(feature = "benchmarks")]
        let (split, inline) =
            build_bench_file_diff_rebuild_from_text("src/bench_diff_open.rs", &old_text, &new_text);
        #[cfg(not(feature = "benchmarks"))]
        let (split, inline) = unreachable!("FileDiffOpenFixture requires benchmarks feature");

        Self { split, inline }
    }

    /// Measure the cost of paging the first `window` split (side-by-side) rows.
    pub fn run_split_first_window(&self, window: usize) -> u64 {
        use gitcomet_core::domain::DiffRowProvider;
        let window = window.max(1);
        let mut h = FxHasher::default();
        self.split.len_hint().hash(&mut h);
        for row in self.split.slice(0, window).take(window) {
            let kind_key: u8 = match row.kind {
                gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
            };
            kind_key.hash(&mut h);
            row.old_line.hash(&mut h);
            row.new_line.hash(&mut h);
            row.old.as_ref().map(|s| s.len()).hash(&mut h);
            row.new.as_ref().map(|s| s.len()).hash(&mut h);
        }
        h.finish()
    }

    /// Measure the cost of paging the first `window` inline rows.
    pub fn run_inline_first_window(&self, window: usize) -> u64 {
        use gitcomet_core::domain::DiffRowProvider;
        let window = window.max(1);
        let mut h = FxHasher::default();
        self.inline.len_hint().hash(&mut h);
        for line in self.inline.slice(0, window).take(window) {
            let kind_key: u8 = match line.kind {
                DiffLineKind::Header => 0,
                DiffLineKind::Hunk => 1,
                DiffLineKind::Add => 2,
                DiffLineKind::Remove => 3,
                DiffLineKind::Context => 4,
            };
            kind_key.hash(&mut h);
            line.text.len().hash(&mut h);
            line.old_line.hash(&mut h);
            line.new_line.hash(&mut h);
        }
        h.finish()
    }

    /// Collect structural sidecar metrics for the first-window operation.
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn measure_first_window(&self, window: usize) -> FileDiffOpenMetrics {
        use gitcomet_core::domain::DiffRowProvider;
        let window = window.max(1);
        let split_painted = self.split.slice(0, window).take(window).count();
        let inline_painted = self.inline.slice(0, window).take(window).count();
        FileDiffOpenMetrics {
            rows_requested: bench_counter_u64(window),
            split_total_rows: bench_counter_u64(self.split.len_hint()),
            split_rows_painted: bench_counter_u64(split_painted),
            inline_total_rows: bench_counter_u64(self.inline.len_hint()),
            inline_rows_painted: bench_counter_u64(inline_painted),
        }
    }
}

// ---------------------------------------------------------------------------
// Pane resize drag step fixture
// ---------------------------------------------------------------------------

/// Drag target for `pane_resize_drag_step/*`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneResizeTarget {
    Sidebar,
    Details,
}

/// Benchmark fixture for `pane_resize_drag_step/sidebar` and `.../details`.
///
/// Each iteration models a single drag-step update using the production pane
/// clamp math from `view/mod.rs`, then records the resulting pane width and the
/// main-pane width after layout recomputation.
pub struct PaneResizeDragStepFixture {
    target: PaneResizeTarget,
    total_width: Pixels,
    sidebar_width: Pixels,
    details_width: Pixels,
    sidebar_collapsed: bool,
    details_collapsed: bool,
    drag_step_px: f32,
    drag_direction: f32,
    steps: usize,
}

/// Sidecar metrics for pane resize drag benchmarks.
pub struct PaneResizeDragMetrics {
    pub steps: u64,
    pub width_bounds_recomputes: u64,
    pub layout_recomputes: u64,
    pub min_pane_width_px: f64,
    pub max_pane_width_px: f64,
    pub min_main_width_px: f64,
    pub max_main_width_px: f64,
    pub clamp_at_min_count: u64,
    pub clamp_at_max_count: u64,
}

impl PaneResizeDragStepFixture {
    pub fn new(target: PaneResizeTarget) -> Self {
        Self {
            target,
            total_width: px(1_280.0),
            sidebar_width: px(280.0),
            details_width: px(420.0),
            sidebar_collapsed: false,
            details_collapsed: false,
            drag_step_px: 24.0,
            drag_direction: 1.0,
            steps: 200,
        }
    }

    pub fn run(&mut self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_hash_and_clamp_counts(&mut self) -> (u64, u64, u64) {
        use crate::view::panes::main::pane_content_width_for_layout;

        let handle = self.handle();
        let total_width = self.total_width;
        let sidebar_collapsed = self.sidebar_collapsed;
        let details_collapsed = self.details_collapsed;

        let mut h = FxHasher::default();
        let mut clamp_at_min_count: u64 = 0;
        let mut clamp_at_max_count: u64 = 0;

        for _ in 0..self.steps {
            let state = PaneResizeState::new(
                handle,
                px(0.0),
                self.sidebar_width,
                self.details_width,
                total_width,
                sidebar_collapsed,
                details_collapsed,
            );
            let current_x = px(self.drag_step_px * self.drag_direction);
            let (min_width, max_width) =
                state.drag_width_bounds(total_width, sidebar_collapsed, details_collapsed);
            let next_width = next_pane_resize_drag_width(
                &state,
                current_x,
                total_width,
                sidebar_collapsed,
                details_collapsed,
            );

            match self.target {
                PaneResizeTarget::Sidebar => self.sidebar_width = next_width,
                PaneResizeTarget::Details => self.details_width = next_width,
            }

            let next_width_px: f32 = next_width.into();
            let min_width_px: f32 = min_width.into();
            let max_width_px: f32 = max_width.into();

            if next_width_px <= min_width_px + f32::EPSILON {
                clamp_at_min_count += 1;
                self.drag_direction = -self.drag_direction;
            } else if next_width_px >= max_width_px - f32::EPSILON {
                clamp_at_max_count += 1;
                self.drag_direction = -self.drag_direction;
            }

            let main_width = pane_content_width_for_layout(
                total_width,
                self.sidebar_width,
                self.details_width,
                sidebar_collapsed,
                details_collapsed,
            );
            let main_width_px: f32 = main_width.into();

            next_width_px.to_bits().hash(&mut h);
            main_width_px.to_bits().hash(&mut h);
            self.drag_direction.to_bits().hash(&mut h);
        }

        (h.finish(), clamp_at_min_count, clamp_at_max_count)
    }

    pub fn run_with_metrics(&mut self) -> (u64, PaneResizeDragMetrics) {
        use crate::view::panes::main::pane_content_width_for_layout;

        let handle = self.handle();
        let total_width = self.total_width;
        let sidebar_collapsed = self.sidebar_collapsed;
        let details_collapsed = self.details_collapsed;

        let mut h = FxHasher::default();
        let mut min_pane_width = f32::MAX;
        let mut max_pane_width = f32::MIN;
        let mut min_main_width = f32::MAX;
        let mut max_main_width = f32::MIN;
        let mut clamp_at_min_count: u64 = 0;
        let mut clamp_at_max_count: u64 = 0;
        let mut width_bounds_recomputes: u64 = 0;
        let mut layout_recomputes: u64 = 0;

        for _ in 0..self.steps {
            let state = PaneResizeState::new(
                handle,
                px(0.0),
                self.sidebar_width,
                self.details_width,
                total_width,
                sidebar_collapsed,
                details_collapsed,
            );
            let current_x = px(self.drag_step_px * self.drag_direction);
            let (min_width, max_width) =
                state.drag_width_bounds(total_width, sidebar_collapsed, details_collapsed);
            width_bounds_recomputes = width_bounds_recomputes.saturating_add(1);
            let next_width = next_pane_resize_drag_width(
                &state,
                current_x,
                total_width,
                sidebar_collapsed,
                details_collapsed,
            );

            match self.target {
                PaneResizeTarget::Sidebar => self.sidebar_width = next_width,
                PaneResizeTarget::Details => self.details_width = next_width,
            }

            let next_width_px: f32 = next_width.into();
            let min_width_px: f32 = min_width.into();
            let max_width_px: f32 = max_width.into();

            if next_width_px <= min_width_px + f32::EPSILON {
                clamp_at_min_count += 1;
                self.drag_direction = -self.drag_direction;
            } else if next_width_px >= max_width_px - f32::EPSILON {
                clamp_at_max_count += 1;
                self.drag_direction = -self.drag_direction;
            }

            min_pane_width = min_pane_width.min(next_width_px);
            max_pane_width = max_pane_width.max(next_width_px);

            let main_width = pane_content_width_for_layout(
                total_width,
                self.sidebar_width,
                self.details_width,
                sidebar_collapsed,
                details_collapsed,
            );
            layout_recomputes = layout_recomputes.saturating_add(1);
            let main_width_px: f32 = main_width.into();
            min_main_width = min_main_width.min(main_width_px);
            max_main_width = max_main_width.max(main_width_px);

            next_width_px.to_bits().hash(&mut h);
            main_width_px.to_bits().hash(&mut h);
            min_width_px.to_bits().hash(&mut h);
            max_width_px.to_bits().hash(&mut h);
            self.drag_direction.to_bits().hash(&mut h);
        }

        let metrics = PaneResizeDragMetrics {
            steps: self.steps as u64,
            width_bounds_recomputes,
            layout_recomputes,
            min_pane_width_px: min_pane_width as f64,
            max_pane_width_px: max_pane_width as f64,
            min_main_width_px: min_main_width as f64,
            max_main_width_px: max_main_width as f64,
            clamp_at_min_count,
            clamp_at_max_count,
        };

        (h.finish(), metrics)
    }

    fn handle(&self) -> PaneResizeHandle {
        match self.target {
            PaneResizeTarget::Sidebar => PaneResizeHandle::Sidebar,
            PaneResizeTarget::Details => PaneResizeHandle::Details,
        }
    }

    #[cfg(test)]
    pub(super) fn pane_widths(&self) -> (f32, f32) {
        let sidebar: f32 = self.sidebar_width.into();
        let details: f32 = self.details_width.into();
        (sidebar, details)
    }
}

// ---------------------------------------------------------------------------
// Diff split resize drag step fixture
// ---------------------------------------------------------------------------

/// Benchmark fixture for `diff_split_resize_drag_step/window_200`.
///
/// Simulates 200 drag-step updates on the diff-split divider using the
/// production clamp math from `view/mod.rs::next_diff_split_drag_ratio`.
/// The fixture sweeps the split ratio back and forth across the available
/// main-pane width, reversing direction when the ratio hits the column
/// minimum bounds.
pub struct DiffSplitResizeDragStepFixture {
    /// Main pane content width (the area that holds left + handle + right).
    main_pane_width: Pixels,
    /// Current diff split ratio (0.0–1.0).
    ratio: f32,
    /// Pixel step per drag event.
    drag_step_px: f32,
    /// Current direction (+1.0 = right, -1.0 = left).
    drag_direction: f32,
    /// Number of drag steps per benchmark iteration.
    steps: usize,
}

/// Sidecar metrics for diff split resize drag benchmarks.
pub struct DiffSplitResizeDragMetrics {
    pub steps: u64,
    pub ratio_recomputes: u64,
    pub column_width_recomputes: u64,
    pub min_ratio: f64,
    pub max_ratio: f64,
    pub min_left_col_px: f64,
    pub max_left_col_px: f64,
    pub min_right_col_px: f64,
    pub max_right_col_px: f64,
    pub clamp_at_min_count: u64,
    pub clamp_at_max_count: u64,
    pub narrow_fallback_count: u64,
}

impl DiffSplitResizeDragStepFixture {
    /// Create a fixture simulating a 200-row visible diff window.
    ///
    /// The `main_pane_width` is set to a realistic value for a ~1280 px window
    /// with sidebar (280) and details (420) open: 1280 - 280 - 420 - 16 = 564 px.
    pub fn window_200() -> Self {
        Self {
            main_pane_width: px(564.0),
            ratio: 0.5,
            drag_step_px: 12.0,
            drag_direction: 1.0,
            steps: 200,
        }
    }

    pub fn run(&mut self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&mut self) -> (u64, DiffSplitResizeDragMetrics) {
        use crate::view::{
            diff_split_column_widths, diff_split_drag_params, next_diff_split_drag_ratio,
        };

        let (available_base, min_col_w) = diff_split_drag_params(self.main_pane_width);

        let mut h = FxHasher::default();
        let mut min_ratio = f64::MAX;
        let mut max_ratio = f64::MIN;
        let mut min_left_col = f64::MAX;
        let mut max_left_col = f64::MIN;
        let mut min_right_col = f64::MAX;
        let mut max_right_col = f64::MIN;
        let mut clamp_at_min_count: u64 = 0;
        let mut clamp_at_max_count: u64 = 0;
        let mut narrow_fallback_count: u64 = 0;
        let mut ratio_recomputes: u64 = 0;
        let mut column_width_recomputes: u64 = 0;

        for _ in 0..self.steps {
            let dx = px(self.drag_step_px * self.drag_direction);
            ratio_recomputes = ratio_recomputes.saturating_add(1);

            match next_diff_split_drag_ratio(available_base, min_col_w, self.ratio, dx) {
                None => {
                    // Too narrow — force 50/50.
                    self.ratio = 0.5;
                    narrow_fallback_count += 1;
                }
                Some(next_ratio) => {
                    // Detect clamping by checking if the ratio is at the
                    // min or max boundary.
                    let available_f: f32 = available_base.into();
                    let min_col_f: f32 = min_col_w.into();
                    let min_bound = min_col_f / available_f;
                    let max_bound = 1.0 - min_bound;

                    if next_ratio <= min_bound + f32::EPSILON {
                        clamp_at_min_count += 1;
                        self.drag_direction = -self.drag_direction;
                    } else if next_ratio >= max_bound - f32::EPSILON {
                        clamp_at_max_count += 1;
                        self.drag_direction = -self.drag_direction;
                    }

                    self.ratio = next_ratio;
                }
            }

            // Compute column widths for this ratio (exercises the layout path).
            let (left_w, right_w) = diff_split_column_widths(self.main_pane_width, self.ratio);
            column_width_recomputes = column_width_recomputes.saturating_add(1);
            let left_f = f32::from(left_w);
            let right_f = f32::from(right_w);

            let ratio_f64 = self.ratio as f64;
            min_ratio = min_ratio.min(ratio_f64);
            max_ratio = max_ratio.max(ratio_f64);
            min_left_col = min_left_col.min(left_f as f64);
            max_left_col = max_left_col.max(left_f as f64);
            min_right_col = min_right_col.min(right_f as f64);
            max_right_col = max_right_col.max(right_f as f64);

            self.ratio.to_bits().hash(&mut h);
            left_f.to_bits().hash(&mut h);
            right_f.to_bits().hash(&mut h);
            self.drag_direction.to_bits().hash(&mut h);
        }

        let metrics = DiffSplitResizeDragMetrics {
            steps: self.steps as u64,
            ratio_recomputes,
            column_width_recomputes,
            min_ratio,
            max_ratio,
            min_left_col_px: min_left_col,
            max_left_col_px: max_left_col,
            min_right_col_px: min_right_col,
            max_right_col_px: max_right_col,
            clamp_at_min_count,
            clamp_at_max_count,
            narrow_fallback_count,
        };

        (h.finish(), metrics)
    }

    #[cfg(test)]
    pub(super) fn current_ratio(&self) -> f32 {
        self.ratio
    }
}

// ---------------------------------------------------------------------------
// Window resize layout fixture
// ---------------------------------------------------------------------------

/// Benchmark fixture for `window_resize_layout/sidebar_main_details`.
///
/// Simulates a sustained window-resize drag by sweeping through a range of
/// total window widths and recomputing the main-pane content width at each
/// step.  This exercises `pane_content_width_for_layout`, the sidebar/details
/// collapse/expand thresholds, and the resize-handle accounting.
pub struct WindowResizeLayoutFixture {
    sidebar_w: f32,
    details_w: f32,
    sidebar_collapsed: bool,
    details_collapsed: bool,
    start_total_w: f32,
    end_total_w: f32,
    steps: usize,
}

/// Sidecar metrics for window resize layout benchmarks.
pub struct WindowResizeLayoutMetrics {
    pub steps: u64,
    pub layout_recomputes: u64,
    pub min_main_w_px: f64,
    pub max_main_w_px: f64,
    pub clamp_at_zero_count: u64,
}

impl WindowResizeLayoutFixture {
    /// Standard 3-pane layout: sidebar 280, details 420, sweep 800..1800 in 200 steps.
    pub fn sidebar_main_details() -> Self {
        Self {
            sidebar_w: 280.0,
            details_w: 420.0,
            sidebar_collapsed: false,
            details_collapsed: false,
            start_total_w: 800.0,
            end_total_w: 1800.0,
            steps: 200,
        }
    }

    pub fn run(&self) -> u64 {
        let (_, metrics) = self.run_with_metrics();
        let mut h = FxHasher::default();
        metrics.steps.hash(&mut h);
        metrics.clamp_at_zero_count.hash(&mut h);
        h.finish()
    }

    pub fn run_with_metrics(&self) -> (u64, WindowResizeLayoutMetrics) {
        use crate::view::panes::main::pane_content_width_for_layout;

        let step_delta = (self.end_total_w - self.start_total_w) / self.steps.max(1) as f32;
        let sidebar_px = px(self.sidebar_w);
        let details_px = px(self.details_w);

        let mut min_main: f32 = f32::MAX;
        let mut max_main: f32 = f32::MIN;
        let mut clamp_zero: u64 = 0;
        let mut layout_recomputes: u64 = 0;
        let mut h = FxHasher::default();

        for i in 0..self.steps {
            let total_w = self.start_total_w + step_delta * i as f32;
            let main_w = pane_content_width_for_layout(
                px(total_w),
                sidebar_px,
                details_px,
                self.sidebar_collapsed,
                self.details_collapsed,
            );
            layout_recomputes = layout_recomputes.saturating_add(1);
            let main_f: f32 = main_w.into();
            main_f.to_bits().hash(&mut h);
            if main_f < min_main {
                min_main = main_f;
            }
            if main_f > max_main {
                max_main = main_f;
            }
            if main_f <= 0.0 {
                clamp_zero += 1;
            }
        }

        let metrics = WindowResizeLayoutMetrics {
            steps: self.steps as u64,
            layout_recomputes,
            min_main_w_px: min_main as f64,
            max_main_w_px: max_main as f64,
            clamp_at_zero_count: clamp_zero,
        };

        (h.finish(), metrics)
    }
}

/// Benchmark fixture for
/// `window_resize_layout/history_50k_commits_diff_20k_lines`.
///
/// Unlike the baseline resize-layout fixture, this keeps two large hot-path
/// workloads resident and replays them on every width change:
///
/// - a precomputed 50k-commit history list window
/// - an open 20k-line split file diff window
///
/// Each step recomputes the production main-pane layout width, diff split
/// widths, and history-column visibility before repainting stable visible
/// windows from both fixtures. This approximates resize cost once the repo is
/// already open and both heavy views are warm.
pub struct WindowResizeLayoutExtremeFixture {
    sidebar_w: f32,
    details_w: f32,
    sidebar_collapsed: bool,
    details_collapsed: bool,
    start_total_w: f32,
    end_total_w: f32,
    steps: usize,
    history: HistoryListScrollFixture,
    history_start_row: usize,
    history_window_rows: usize,
    diff: FileDiffOpenFixture,
    diff_window_rows: usize,
    diff_split_ratio: f32,
    history_commits: usize,
    diff_lines: usize,
}

/// Sidecar metrics for the extreme-scale window resize layout benchmark.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowResizeLayoutExtremeMetrics {
    pub steps: u64,
    pub layout_recomputes: u64,
    pub history_visibility_recomputes: u64,
    pub diff_width_recomputes: u64,
    pub history_commits: u64,
    pub history_window_rows: u64,
    pub history_rows_processed_total: u64,
    pub history_columns_hidden_steps: u64,
    pub history_all_columns_visible_steps: u64,
    pub diff_lines: u64,
    pub diff_window_rows: u64,
    pub diff_split_total_rows: u64,
    pub diff_rows_processed_total: u64,
    pub diff_narrow_fallback_steps: u64,
    pub min_main_w_px: f64,
    pub max_main_w_px: f64,
}

impl WindowResizeLayoutExtremeFixture {
    const HISTORY_COMMITS: usize = 50_000;
    const HISTORY_LOCAL_BRANCHES: usize = 200;
    const HISTORY_REMOTE_BRANCHES: usize = 800;
    const HISTORY_WINDOW_ROWS: usize = 64;
    const DIFF_LINES: usize = 20_000;
    const DIFF_WINDOW_ROWS: usize = 200;

    pub fn history_50k_commits_diff_20k_lines() -> Self {
        let history = HistoryListScrollFixture::new(
            Self::HISTORY_COMMITS,
            Self::HISTORY_LOCAL_BRANCHES,
            Self::HISTORY_REMOTE_BRANCHES,
        );
        let history_window_rows = Self::HISTORY_WINDOW_ROWS.min(Self::HISTORY_COMMITS.max(1));
        let history_start_row = Self::HISTORY_COMMITS.saturating_sub(history_window_rows) / 2;

        Self {
            sidebar_w: 280.0,
            details_w: 420.0,
            sidebar_collapsed: false,
            details_collapsed: false,
            start_total_w: 820.0,
            end_total_w: 2_200.0,
            steps: 200,
            history,
            history_start_row,
            history_window_rows,
            diff: FileDiffOpenFixture::new(Self::DIFF_LINES),
            diff_window_rows: Self::DIFF_WINDOW_ROWS,
            diff_split_ratio: 0.5,
            history_commits: Self::HISTORY_COMMITS,
            diff_lines: Self::DIFF_LINES,
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal().0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, WindowResizeLayoutExtremeMetrics) {
        let (
            hash,
            min_main_w_px,
            max_main_w_px,
            history_columns_hidden_steps,
            history_all_columns_visible_steps,
            diff_narrow_fallback_steps,
        ) = self.run_internal();

        let diff_metrics = self.diff.measure_first_window(self.diff_window_rows);
        let steps = bench_counter_u64(self.steps);
        let history_window_rows = bench_counter_u64(self.history_window_rows);
        let diff_window_rows = bench_counter_u64(self.diff_window_rows);

        let metrics = WindowResizeLayoutExtremeMetrics {
            steps,
            layout_recomputes: steps,
            history_visibility_recomputes: steps,
            diff_width_recomputes: steps,
            history_commits: bench_counter_u64(self.history_commits),
            history_window_rows,
            history_rows_processed_total: history_window_rows.saturating_mul(steps),
            history_columns_hidden_steps,
            history_all_columns_visible_steps,
            diff_lines: bench_counter_u64(self.diff_lines),
            diff_window_rows,
            diff_split_total_rows: diff_metrics.split_total_rows,
            diff_rows_processed_total: diff_metrics.split_rows_painted.saturating_mul(steps),
            diff_narrow_fallback_steps,
            min_main_w_px,
            max_main_w_px,
        };

        (hash, metrics)
    }

    fn run_internal(&self) -> (u64, f64, f64, u64, u64, u64) {
        use crate::view::panes::main::pane_content_width_for_layout;
        use crate::view::{diff_split_column_widths, diff_split_drag_params};

        let step_delta = (self.end_total_w - self.start_total_w) / self.steps.max(1) as f32;
        let sidebar_px = px(self.sidebar_w);
        let details_px = px(self.details_w);

        let mut min_main = f32::MAX;
        let mut max_main = f32::MIN;
        let mut history_columns_hidden_steps = 0u64;
        let mut history_all_columns_visible_steps = 0u64;
        let mut diff_narrow_fallback_steps = 0u64;
        let mut h = FxHasher::default();

        for i in 0..self.steps {
            let total_w = self.start_total_w + step_delta * i as f32;
            let main_w = pane_content_width_for_layout(
                px(total_w),
                sidebar_px,
                details_px,
                self.sidebar_collapsed,
                self.details_collapsed,
            );
            let main_w_px: f32 = main_w.into();
            min_main = min_main.min(main_w_px);
            max_main = max_main.max(main_w_px);

            let (show_author, show_date, show_sha) =
                Self::history_column_visibility_for_window_width(total_w);
            if !(show_author && show_date && show_sha) {
                history_columns_hidden_steps = history_columns_hidden_steps.saturating_add(1);
            }
            if show_author && show_date && show_sha {
                history_all_columns_visible_steps =
                    history_all_columns_visible_steps.saturating_add(1);
            }

            let (available, min_col_w) = diff_split_drag_params(main_w);
            if available <= min_col_w * 2.0 {
                diff_narrow_fallback_steps = diff_narrow_fallback_steps.saturating_add(1);
            }
            let (left_w, right_w) = diff_split_column_widths(main_w, self.diff_split_ratio);
            let left_w_px = f32::from(left_w);
            let right_w_px = f32::from(right_w);

            let history_hash = self
                .history
                .run_scroll_step(self.history_start_row, self.history_window_rows);
            let diff_hash = self.diff.run_split_first_window(self.diff_window_rows);

            total_w.to_bits().hash(&mut h);
            main_w_px.to_bits().hash(&mut h);
            left_w_px.to_bits().hash(&mut h);
            right_w_px.to_bits().hash(&mut h);
            show_author.hash(&mut h);
            show_date.hash(&mut h);
            show_sha.hash(&mut h);
            history_hash.hash(&mut h);
            diff_hash.hash(&mut h);
        }

        (
            h.finish(),
            min_main as f64,
            max_main as f64,
            history_columns_hidden_steps,
            history_all_columns_visible_steps,
            diff_narrow_fallback_steps,
        )
    }

    fn history_column_visibility_for_window_width(total_w: f32) -> (bool, bool, bool) {
        let available = (total_w - 280.0 - 420.0 - 64.0).max(0.0);
        if available <= 0.0 {
            return (false, false, false);
        }

        let min_message = HistoryColumnResizeDragStepFixture::MESSAGE_MIN_PX;
        let fixed_base = HistoryColumnResizeDragStepFixture::COL_BRANCH_PX
            + HistoryColumnResizeDragStepFixture::COL_GRAPH_PX;
        let mut fixed = fixed_base
            + HistoryColumnResizeDragStepFixture::COL_AUTHOR_PX
            + HistoryColumnResizeDragStepFixture::COL_DATE_PX
            + HistoryColumnResizeDragStepFixture::COL_SHA_PX;

        let mut show_author = true;
        let mut show_date = true;
        let mut show_sha = true;

        if available - fixed < min_message && show_sha {
            show_sha = false;
            fixed -= HistoryColumnResizeDragStepFixture::COL_SHA_PX;
        }
        if available - fixed < min_message {
            if show_date {
                show_date = false;
                fixed -= HistoryColumnResizeDragStepFixture::COL_DATE_PX;
            }
            show_sha = false;
        }
        if available - fixed < min_message && show_author {
            show_author = false;
            fixed -= HistoryColumnResizeDragStepFixture::COL_AUTHOR_PX;
        }
        if available - fixed < min_message {
            show_author = false;
            show_date = false;
            show_sha = false;
        }

        (show_author, show_date, show_sha)
    }
}

// ---------------------------------------------------------------------------
// History column resize drag step fixture
// ---------------------------------------------------------------------------

/// Benchmark fixture for `history_column_resize_drag_step/*`.
///
/// Simulates a sustained column-resize drag over a history-pane column.
/// Each step applies a pixel delta, clamps the candidate width against the
/// column's static bounds and the message-area minimum, then recomputes
/// which optional columns (author, date, SHA) remain visible.
///
/// The fixture replicates the `history_column_drag_clamped_width` +
/// `history_visible_columns` math from `crate::view::panes::history`.
pub struct HistoryColumnResizeDragStepFixture {
    col_branch: f32,
    col_graph: f32,
    col_author: f32,
    col_date: f32,
    col_sha: f32,
    window_width: f32,
    drag_step_px: f32,
    drag_direction: f32,
    steps: usize,
}

/// Sidecar metrics for history column resize drag benchmarks.
pub struct HistoryColumnResizeMetrics {
    pub steps: u64,
    pub width_clamp_recomputes: u64,
    pub visible_column_recomputes: u64,
    pub columns_hidden_count: u64,
    pub clamp_at_min_count: u64,
    pub clamp_at_max_count: u64,
}

/// Column identity for the column being dragged.
#[derive(Clone, Copy)]
pub enum HistoryResizeColumn {
    Branch,
    Graph,
    Author,
    Date,
    Sha,
}

impl HistoryColumnResizeDragStepFixture {
    // Constants replicated from view/mod.rs.
    const COL_BRANCH_PX: f32 = 130.0;
    const COL_GRAPH_PX: f32 = 80.0;
    const COL_AUTHOR_PX: f32 = 140.0;
    const COL_DATE_PX: f32 = 160.0;
    const COL_SHA_PX: f32 = 88.0;
    const MESSAGE_MIN_PX: f32 = 220.0;

    const COL_BRANCH_MIN: f32 = 60.0;
    const COL_BRANCH_MAX: f32 = 320.0;
    const COL_GRAPH_MIN: f32 = 44.0;
    const COL_GRAPH_MAX: f32 = 240.0;
    const COL_AUTHOR_MIN: f32 = 80.0;
    const COL_AUTHOR_MAX: f32 = 260.0;
    const COL_DATE_MIN: f32 = 110.0;
    const COL_DATE_MAX: f32 = 240.0;
    const COL_SHA_MIN: f32 = 60.0;
    const COL_SHA_MAX: f32 = 160.0;

    pub fn new(column: HistoryResizeColumn) -> Self {
        let _ = column; // all columns start from the same defaults
        Self {
            col_branch: Self::COL_BRANCH_PX,
            col_graph: Self::COL_GRAPH_PX,
            col_author: Self::COL_AUTHOR_PX,
            col_date: Self::COL_DATE_PX,
            col_sha: Self::COL_SHA_PX,
            window_width: 1600.0,
            drag_step_px: 8.0,
            drag_direction: 1.0,
            steps: 200,
        }
    }

    pub fn run(&mut self, column: HistoryResizeColumn) -> u64 {
        let (hash, _) = self.run_with_metrics(column);
        hash
    }

    pub fn run_with_metrics(
        &mut self,
        column: HistoryResizeColumn,
    ) -> (u64, HistoryColumnResizeMetrics) {
        // Available width for columns: window - sidebar(280) - details(420) - misc(64)
        let available = (self.window_width - 280.0 - 420.0 - 64.0).max(0.0);

        let (min_w, static_max) = Self::static_bounds(column);

        let mut h = FxHasher::default();
        let mut columns_hidden: u64 = 0;
        let mut clamp_min: u64 = 0;
        let mut clamp_max: u64 = 0;
        let mut width_clamp_recomputes: u64 = 0;
        let mut visible_column_recomputes: u64 = 0;

        for _ in 0..self.steps {
            let current = self.col_for(column);
            let candidate = current + self.drag_step_px * self.drag_direction;

            // Compute right_fixed_w (other columns excluding the one being dragged)
            let show_author = true;
            let show_date = true;
            let show_sha = true;
            let right_fixed = self.right_fixed_excluding(column, show_author, show_date, show_sha);
            let dynamic_max = (available - right_fixed - Self::MESSAGE_MIN_PX).max(min_w);
            let max_w = static_max.min(dynamic_max).max(min_w);
            width_clamp_recomputes = width_clamp_recomputes.saturating_add(1);
            let clamped = candidate.max(min_w).min(max_w);

            self.set_col(column, clamped);

            if clamped <= min_w + f32::EPSILON {
                clamp_min += 1;
                self.drag_direction = 1.0;
            } else if clamped >= max_w - f32::EPSILON {
                clamp_max += 1;
                self.drag_direction = -1.0;
            }

            // Recompute visible columns (the message-area squeeze check)
            let (vis_author, vis_date, vis_sha) = self.visible_columns(available);
            visible_column_recomputes = visible_column_recomputes.saturating_add(1);
            if !vis_author || !vis_date || !vis_sha {
                columns_hidden += 1;
            }

            clamped.to_bits().hash(&mut h);
            vis_author.hash(&mut h);
            vis_date.hash(&mut h);
            vis_sha.hash(&mut h);
        }

        let metrics = HistoryColumnResizeMetrics {
            steps: self.steps as u64,
            width_clamp_recomputes,
            visible_column_recomputes,
            columns_hidden_count: columns_hidden,
            clamp_at_min_count: clamp_min,
            clamp_at_max_count: clamp_max,
        };

        (h.finish(), metrics)
    }

    fn static_bounds(column: HistoryResizeColumn) -> (f32, f32) {
        match column {
            HistoryResizeColumn::Branch => (Self::COL_BRANCH_MIN, Self::COL_BRANCH_MAX),
            HistoryResizeColumn::Graph => (Self::COL_GRAPH_MIN, Self::COL_GRAPH_MAX),
            HistoryResizeColumn::Author => (Self::COL_AUTHOR_MIN, Self::COL_AUTHOR_MAX),
            HistoryResizeColumn::Date => (Self::COL_DATE_MIN, Self::COL_DATE_MAX),
            HistoryResizeColumn::Sha => (Self::COL_SHA_MIN, Self::COL_SHA_MAX),
        }
    }

    fn col_for(&self, column: HistoryResizeColumn) -> f32 {
        match column {
            HistoryResizeColumn::Branch => self.col_branch,
            HistoryResizeColumn::Graph => self.col_graph,
            HistoryResizeColumn::Author => self.col_author,
            HistoryResizeColumn::Date => self.col_date,
            HistoryResizeColumn::Sha => self.col_sha,
        }
    }

    fn set_col(&mut self, column: HistoryResizeColumn, value: f32) {
        match column {
            HistoryResizeColumn::Branch => self.col_branch = value,
            HistoryResizeColumn::Graph => self.col_graph = value,
            HistoryResizeColumn::Author => self.col_author = value,
            HistoryResizeColumn::Date => self.col_date = value,
            HistoryResizeColumn::Sha => self.col_sha = value,
        }
    }

    fn right_fixed_excluding(
        &self,
        column: HistoryResizeColumn,
        show_author: bool,
        show_date: bool,
        show_sha: bool,
    ) -> f32 {
        let mut sum = 0.0;
        if !matches!(column, HistoryResizeColumn::Branch) {
            sum += self.col_branch;
        }
        if !matches!(column, HistoryResizeColumn::Graph) {
            sum += self.col_graph;
        }
        if show_author && !matches!(column, HistoryResizeColumn::Author) {
            sum += self.col_author;
        }
        if show_date && !matches!(column, HistoryResizeColumn::Date) {
            sum += self.col_date;
        }
        if show_sha && !matches!(column, HistoryResizeColumn::Sha) {
            sum += self.col_sha;
        }
        sum
    }

    fn visible_columns(&self, available: f32) -> (bool, bool, bool) {
        let min_message = Self::MESSAGE_MIN_PX;
        let mut show_author = true;
        let mut show_date = true;
        let mut show_sha = true;

        let fixed_base = self.col_branch + self.col_graph;
        let mut fixed = fixed_base + self.col_author + self.col_date + self.col_sha;

        if available - fixed < min_message && show_sha {
            show_sha = false;
            fixed -= self.col_sha;
        }
        if available - fixed < min_message {
            if show_date {
                show_date = false;
                fixed -= self.col_date;
            }
            show_sha = false;
        }
        if available - fixed < min_message && show_author {
            show_author = false;
        }

        (show_author, show_date, show_sha)
    }
}

// ---------------------------------------------------------------------------
// Repo tab drag fixtures
// ---------------------------------------------------------------------------

/// Benchmark fixture for `repo_tab_drag_hit_test/*` and `repo_tab_reorder_reduce/*`.
///
/// Simulates a sustained tab drag by performing hit-test position lookups
/// across a tab bar and dispatching `Msg::ReorderRepoTabs` through the
/// reducer.  The fixture splits into two sub-benchmarks:
///
/// - **hit_test**: Pure hit-testing — determines which tab the cursor is over
///   and the insertion point.  This exercises the same logic as
///   `repo_tab_insert_before_for_drop` without going through GPUI.
///
/// - **reorder_reduce**: Full reducer dispatch of `Msg::ReorderRepoTabs`
///   through `dispatch_sync` for each drag step.
pub struct RepoTabDragFixture {
    tab_count: usize,
    tab_width_px: f32,
    baseline: AppState,
}

/// Sidecar metrics for repo tab drag benchmarks.
pub struct RepoTabDragMetrics {
    pub tab_count: u64,
    pub hit_test_steps: u64,
    pub reorder_steps: u64,
    pub effects_emitted: u64,
    pub noop_reorders: u64,
}

impl RepoTabDragFixture {
    pub fn new(tab_count: usize) -> Self {
        let commits = build_synthetic_commits(10);
        let repos: Vec<RepoState> = (0..tab_count)
            .map(|i| {
                let repo_id = RepoId(i as u64 + 1);
                let mut repo = RepoState::new_opening(
                    repo_id,
                    RepoSpec {
                        workdir: std::path::PathBuf::from(format!("/tmp/bench-tab-{i}")),
                    },
                );
                repo.open = Loadable::Ready(());
                repo.log = Loadable::Ready(Arc::new(LogPage {
                    commits: commits.clone(),
                    next_cursor: None,
                }));
                repo
            })
            .collect();

        let active = repos.first().map(|r| r.id);
        Self {
            tab_count,
            tab_width_px: 120.0,
            baseline: bench_app_state(repos, active),
        }
    }

    /// Hit-test only — determine insert_before for each step across the tab bar.
    pub fn run_hit_test(&self) -> (u64, RepoTabDragMetrics) {
        let repos = &self.baseline.repos;
        let steps = self.tab_count * 3; // sweep across all tabs multiple times
        let total_bar_width = self.tab_count as f32 * self.tab_width_px;

        let mut h = FxHasher::default();
        let mut hit_steps: u64 = 0;

        for step in 0..steps {
            // Simulate cursor position sweeping across the tab bar.
            let frac = (step as f32) / (steps.max(1) as f32);
            let cursor_x = frac * total_bar_width;

            // Determine which tab the cursor is over.
            let tab_ix = (cursor_x / self.tab_width_px) as usize;
            let tab_ix = tab_ix.min(self.tab_count.saturating_sub(1));
            let target_repo_id = repos[tab_ix].id;

            // Replicate repo_tab_insert_before_for_drop logic.
            let tab_left = tab_ix as f32 * self.tab_width_px;
            let tab_center = tab_left + self.tab_width_px / 2.0;
            let insert_before = if cursor_x <= tab_center {
                Some(target_repo_id)
            } else {
                repos.get(tab_ix + 1).map(|r| r.id)
            };

            insert_before.hash(&mut h);
            target_repo_id.0.hash(&mut h);
            hit_steps += 1;
        }

        let metrics = RepoTabDragMetrics {
            tab_count: self.tab_count as u64,
            hit_test_steps: hit_steps,
            reorder_steps: 0,
            effects_emitted: 0,
            noop_reorders: 0,
        };

        (h.finish(), metrics)
    }

    #[cfg(test)]
    pub fn hit_test_target_repo_ids(&self) -> Vec<RepoId> {
        let repos = &self.baseline.repos;
        let steps = self.tab_count * 3;
        let total_bar_width = self.tab_count as f32 * self.tab_width_px;
        let mut ids = Vec::with_capacity(steps);

        for step in 0..steps {
            let frac = (step as f32) / (steps.max(1) as f32);
            let cursor_x = frac * total_bar_width;
            let tab_ix = (cursor_x / self.tab_width_px) as usize;
            let tab_ix = tab_ix.min(self.tab_count.saturating_sub(1));
            ids.push(repos[tab_ix].id);
        }

        ids
    }

    /// Full reducer dispatch — hit-test + reorder_repo_tabs for each step.
    pub fn run_reorder(&self) -> (u64, RepoTabDragMetrics) {
        use gitcomet_state::benchmarks::dispatch_sync;
        use gitcomet_state::msg::Msg;

        let mut state = self.baseline.clone();
        let steps = self.tab_count * 2;
        let total_bar_width = self.tab_count as f32 * self.tab_width_px;

        // We'll drag the first tab across the bar.
        let dragged_repo_id = state.repos[0].id;

        let mut h = FxHasher::default();
        let mut reorder_steps: u64 = 0;
        let mut effects_emitted: u64 = 0;
        let mut noop_reorders: u64 = 0;

        for step in 0..steps {
            let frac = (step as f32) / (steps.max(1) as f32);
            let cursor_x = frac * total_bar_width;

            let tab_ix = (cursor_x / self.tab_width_px) as usize;
            let tab_ix = tab_ix.min(state.repos.len().saturating_sub(1));
            let target_repo_id = state.repos[tab_ix].id;
            let tab_left = tab_ix as f32 * self.tab_width_px;
            let tab_center = tab_left + self.tab_width_px / 2.0;
            let insert_before = if cursor_x <= tab_center {
                Some(target_repo_id)
            } else {
                state.repos.get(tab_ix + 1).map(|r| r.id)
            };

            let effects = dispatch_sync(
                &mut state,
                Msg::ReorderRepoTabs {
                    repo_id: dragged_repo_id,
                    insert_before,
                },
            );

            if effects.is_empty() {
                noop_reorders += 1;
            } else {
                effects_emitted += effects.len() as u64;
            }

            state.active_repo.hash(&mut h);
            state.repos.len().hash(&mut h);
            // Hash the current tab order to detect reorder fidelity.
            for repo in state.repos.iter().take(8) {
                repo.id.0.hash(&mut h);
            }
            reorder_steps += 1;
        }

        let metrics = RepoTabDragMetrics {
            tab_count: self.tab_count as u64,
            hit_test_steps: 0,
            reorder_steps,
            effects_emitted,
            noop_reorders,
        };

        (h.finish(), metrics)
    }
}

pub struct PatchDiffSearchQueryUpdateFixture {
    diff_rows: Vec<AnnotatedDiffLine>,
    click_kinds: Vec<DiffClickKind>,
    word_highlights: Vec<Option<Vec<Range<usize>>>>,
    language_for_src_ix: Vec<Option<DiffSyntaxLanguage>>,
    visible_row_indices: Vec<usize>,
    theme: AppTheme,
    syntax_mode: DiffSyntaxMode,
    stable_cache: Vec<Option<CachedDiffStyledText>>,
    query_cache: Vec<Option<PatchDiffSearchQueryCacheEntry>>,
    query_cache_query: SharedString,
    query_cache_generation: u64,
}

#[derive(Clone)]
struct PatchDiffSearchQueryCacheEntry {
    generation: u64,
    styled: CachedDiffStyledText,
}

impl PatchDiffSearchQueryUpdateFixture {
    pub fn new(lines: usize) -> Self {
        let theme = AppTheme::gitcomet_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");
        let target_lines = lines.max(1);
        let mut diff_rows = Vec::with_capacity(target_lines);
        let mut click_kinds = Vec::with_capacity(target_lines);
        let mut word_highlights = Vec::with_capacity(target_lines);
        let mut language_for_src_ix = Vec::with_capacity(target_lines);

        let mut file_ix = 0usize;
        while diff_rows.len() < target_lines {
            diff_rows.push(AnnotatedDiffLine {
                kind: DiffLineKind::Header,
                text: format!("diff --git a/src/file_{file_ix}.rs b/src/file_{file_ix}.rs").into(),
                old_line: None,
                new_line: None,
            });
            click_kinds.push(DiffClickKind::FileHeader);
            word_highlights.push(None);
            language_for_src_ix.push(None);
            if diff_rows.len() >= target_lines {
                break;
            }

            diff_rows.push(AnnotatedDiffLine {
                kind: DiffLineKind::Hunk,
                text: format!("@@ -1,12 +1,12 @@ fn synthetic_{file_ix}() {{").into(),
                old_line: None,
                new_line: None,
            });
            click_kinds.push(DiffClickKind::HunkHeader);
            word_highlights.push(None);
            language_for_src_ix.push(None);
            if diff_rows.len() >= target_lines {
                break;
            }

            for line_in_file in 0..12 {
                if diff_rows.len() >= target_lines {
                    break;
                }

                let content = format!(
                    "let shared_{file_ix}_{line_in_file} = compute_shared({line_in_file});"
                );
                let (kind, text) = match line_in_file % 3 {
                    0 => (DiffLineKind::Add, format!("+{content}")),
                    1 => (DiffLineKind::Remove, format!("-{content}")),
                    _ => (DiffLineKind::Context, format!(" {content}")),
                };

                let word_start = content.find("shared").unwrap_or(0);
                let word_end = (word_start + "shared".len()).min(content.len());

                diff_rows.push(AnnotatedDiffLine {
                    kind,
                    text: text.into(),
                    old_line: None,
                    new_line: None,
                });
                click_kinds.push(DiffClickKind::Line);
                let ranges = std::iter::once(word_start..word_end).collect::<Vec<_>>();
                word_highlights.push(Some(ranges));
                language_for_src_ix.push(language);
            }

            file_ix = file_ix.saturating_add(1);
        }

        let syntax_mode = if diff_rows.len() > 4_000 {
            DiffSyntaxMode::HeuristicOnly
        } else {
            DiffSyntaxMode::Auto
        };
        let mut fixture = Self {
            visible_row_indices: (0..diff_rows.len()).collect(),
            stable_cache: vec![None; diff_rows.len()],
            query_cache: vec![None; diff_rows.len()],
            query_cache_query: SharedString::default(),
            query_cache_generation: 0,
            diff_rows,
            click_kinds,
            word_highlights,
            language_for_src_ix,
            theme,
            syntax_mode,
        };
        fixture.prewarm_stable_cache();
        fixture
    }

    fn prewarm_stable_cache(&mut self) {
        for src_ix in 0..self.diff_rows.len() {
            let click_kind = self
                .click_kinds
                .get(src_ix)
                .copied()
                .unwrap_or(DiffClickKind::Line);
            if !matches!(click_kind, DiffClickKind::Line) {
                continue;
            }
            let _ = self.row_styled(src_ix, "");
        }
        self.query_cache.fill(None);
        self.query_cache_query = SharedString::default();
        self.query_cache_generation = 0;
    }

    fn sync_query_cache(&mut self, query: &str) {
        if self.query_cache_query.as_ref() != query {
            self.query_cache_query = query.to_string().into();
            self.query_cache_generation = self.query_cache_generation.wrapping_add(1);
        }
    }

    fn row_styled(&mut self, src_ix: usize, query: &str) -> Option<CachedDiffStyledText> {
        let query = query.trim();
        let query_active = !query.is_empty();
        let click_kind = self
            .click_kinds
            .get(src_ix)
            .copied()
            .unwrap_or(DiffClickKind::Line);
        let should_style = matches!(click_kind, DiffClickKind::Line) || query_active;
        if !should_style {
            return None;
        }

        if self
            .stable_cache
            .get(src_ix)
            .and_then(Option::as_ref)
            .is_none()
        {
            let line = self.diff_rows.get(src_ix)?;
            let stable = if matches!(click_kind, DiffClickKind::Line) {
                let word_ranges = self
                    .word_highlights
                    .get(src_ix)
                    .and_then(|ranges| ranges.as_deref())
                    .unwrap_or(&[]);
                let language = self.language_for_src_ix.get(src_ix).copied().flatten();
                let word_color = match line.kind {
                    DiffLineKind::Add => Some(self.theme.colors.success),
                    DiffLineKind::Remove => Some(self.theme.colors.danger),
                    _ => None,
                };

                super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    diff_content_text(line),
                    word_ranges,
                    "",
                    language,
                    self.syntax_mode,
                    word_color,
                )
            } else {
                super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    line.text.as_ref(),
                    &[],
                    "",
                    None,
                    self.syntax_mode,
                    None,
                )
            };
            if let Some(slot) = self.stable_cache.get_mut(src_ix) {
                *slot = Some(stable);
            }
        }

        if query_active {
            let query_generation = self.query_cache_generation;
            if self
                .query_cache
                .get(src_ix)
                .and_then(Option::as_ref)
                .is_none_or(|entry| entry.generation != query_generation)
            {
                let base = self.stable_cache.get(src_ix).and_then(Option::as_ref)?;
                let overlay = super::diff_text::build_cached_diff_query_overlay_styled_text(
                    self.theme, base, query,
                );
                if let Some(slot) = self.query_cache.get_mut(src_ix) {
                    *slot = Some(PatchDiffSearchQueryCacheEntry {
                        generation: query_generation,
                        styled: overlay,
                    });
                }
            }
            return self
                .query_cache
                .get(src_ix)
                .and_then(Option::as_ref)
                .filter(|entry| entry.generation == query_generation)
                .map(|entry| entry.styled.clone());
        }

        self.stable_cache
            .get(src_ix)
            .and_then(Option::as_ref)
            .cloned()
    }

    pub fn run_query_update_step(&mut self, query: &str, start: usize, window: usize) -> u64 {
        if self.visible_row_indices.is_empty() || window == 0 {
            return 0;
        }

        self.sync_query_cache(query);
        let start = start % self.visible_row_indices.len();
        let end = (start + window).min(self.visible_row_indices.len());
        let query = self.query_cache_query.clone();

        let mut h = FxHasher::default();
        for visible_ix in start..end {
            let src_ix = self.visible_row_indices[visible_ix];
            src_ix.hash(&mut h);
            if let Some(styled) = self.row_styled(src_ix, query.as_ref()) {
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }
        }
        self.stable_cache_entries().hash(&mut h);
        self.query_cache_entries().hash(&mut h);
        h.finish()
    }

    pub fn visible_rows(&self) -> usize {
        self.visible_row_indices.len()
    }

    pub(crate) fn stable_cache_entries(&self) -> usize {
        self.stable_cache
            .iter()
            .filter(|entry| entry.is_some())
            .count()
    }

    pub(crate) fn query_cache_entries(&self) -> usize {
        self.query_cache
            .iter()
            .filter(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|entry| entry.generation == self.query_cache_generation)
            })
            .count()
    }
}
