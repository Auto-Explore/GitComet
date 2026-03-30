use super::*;

pub enum StagingScenario {
    /// Dispatch `Msg::StagePaths` with all paths in one batch.
    StageAll,
    /// Dispatch `Msg::UnstagePaths` with all paths in one batch.
    UnstageAll,
    /// Alternate `Msg::StagePath` / `Msg::UnstagePath` for each path.
    Interleaved,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StagingMetrics {
    pub file_count: u64,
    pub effect_count: u64,
    pub ops_rev_delta: u64,
    pub local_actions_delta: u64,
    pub stage_effect_count: u64,
    pub unstage_effect_count: u64,
}

pub struct StagingFixture {
    baseline: AppState,
    paths: RepoPathList,
    scenario: StagingScenario,
}

impl StagingFixture {
    pub fn stage_all(file_count: usize) -> Self {
        let entries = build_synthetic_status_entries(file_count.max(1), DiffArea::Unstaged);
        let paths = RepoPathList::from(
            entries
                .iter()
                .map(|e| e.path.clone())
                .collect::<Vec<std::path::PathBuf>>(),
        );

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            unstaged: entries,
            ..RepoStatus::default()
        }));
        repo.status_rev = 1;
        repo.open = Loadable::Ready(());

        Self {
            baseline: AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            paths,
            scenario: StagingScenario::StageAll,
        }
    }

    pub fn unstage_all(file_count: usize) -> Self {
        let entries = build_synthetic_status_entries(file_count.max(1), DiffArea::Staged);
        let paths = RepoPathList::from(
            entries
                .iter()
                .map(|e| e.path.clone())
                .collect::<Vec<std::path::PathBuf>>(),
        );

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            staged: entries,
            ..RepoStatus::default()
        }));
        repo.status_rev = 1;
        repo.open = Loadable::Ready(());

        Self {
            baseline: AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            paths,
            scenario: StagingScenario::UnstageAll,
        }
    }

    pub fn interleaved(file_count: usize) -> Self {
        // Start with half unstaged, half staged — toggle operations will alternate.
        let half = file_count.max(2) / 2;
        let unstaged = build_synthetic_status_entries(half, DiffArea::Unstaged);
        let staged = build_synthetic_status_entries(half, DiffArea::Staged);
        let paths = RepoPathList::from(
            unstaged
                .iter()
                .map(|e| e.path.clone())
                .chain(staged.iter().map(|e| e.path.clone()))
                .collect::<Vec<std::path::PathBuf>>(),
        );

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            unstaged,
            staged,
            ..RepoStatus::default()
        }));
        repo.status_rev = 1;
        repo.open = Loadable::Ready(());

        Self {
            baseline: AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            paths,
            scenario: StagingScenario::Interleaved,
        }
    }

    pub fn fresh_state(&self) -> AppState {
        self.baseline.clone()
    }

    pub fn run(&self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&self) -> (u64, StagingMetrics) {
        let mut state = self.fresh_state();
        self.run_with_state(&mut state)
    }

    pub fn run_with_state(&self, state: &mut AppState) -> (u64, StagingMetrics) {
        let ops_rev_before = state.repos[0].ops_rev;
        let actions_before = state.repos[0].local_actions_in_flight;

        let mut total_effects = 0u64;
        let mut stage_effect_count = 0u64;
        let mut unstage_effect_count = 0u64;
        let mut h = FxHasher::default();

        match self.scenario {
            StagingScenario::StageAll => {
                with_stage_paths_sync(state, RepoId(1), self.paths.clone(), |_state, effects| {
                    total_effects += effects.len() as u64;
                    for effect in effects {
                        match effect {
                            Effect::StagePaths { .. } | Effect::StagePath { .. } => {
                                stage_effect_count += 1;
                            }
                            _ => {}
                        }
                        std::mem::discriminant(effect).hash(&mut h);
                    }
                });
            }
            StagingScenario::UnstageAll => {
                with_unstage_paths_sync(state, RepoId(1), self.paths.clone(), |_state, effects| {
                    total_effects += effects.len() as u64;
                    for effect in effects {
                        match effect {
                            Effect::UnstagePaths { .. } | Effect::UnstagePath { .. } => {
                                unstage_effect_count += 1;
                            }
                            _ => {}
                        }
                        std::mem::discriminant(effect).hash(&mut h);
                    }
                });
            }
            StagingScenario::Interleaved => {
                let repo_id = RepoId(1);
                for (ix, path) in self.paths.as_slice().iter().enumerate() {
                    let mut record_effects = |effects: &[Effect]| {
                        total_effects += effects.len() as u64;
                        for effect in effects {
                            match effect {
                                Effect::StagePaths { .. } | Effect::StagePath { .. } => {
                                    stage_effect_count += 1;
                                }
                                Effect::UnstagePaths { .. } | Effect::UnstagePath { .. } => {
                                    unstage_effect_count += 1;
                                }
                                _ => {}
                            }
                            std::mem::discriminant(effect).hash(&mut h);
                        }
                    };
                    if ix % 2 == 0 {
                        with_stage_path_sync(state, repo_id, path.clone(), |_state, effects| {
                            record_effects(effects);
                        });
                    } else {
                        with_unstage_path_sync(state, repo_id, path.clone(), |_state, effects| {
                            record_effects(effects);
                        });
                    }
                }
            }
        }

        let ops_rev_after = state.repos[0].ops_rev;
        let actions_after = state.repos[0].local_actions_in_flight;

        state.repos[0].ops_rev.hash(&mut h);
        state.repos[0].local_actions_in_flight.hash(&mut h);
        total_effects.hash(&mut h);

        let metrics = StagingMetrics {
            file_count: self.paths.len() as u64,
            effect_count: total_effects,
            ops_rev_delta: ops_rev_after.wrapping_sub(ops_rev_before),
            local_actions_delta: actions_after.wrapping_sub(actions_before) as u64,
            stage_effect_count,
            unstage_effect_count,
        };

        (h.finish(), metrics)
    }
}

// ---------------------------------------------------------------------------
// Undo/redo — conflict resolution deep stack and undo-replay benchmarks
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "benchmarks"))]
pub enum UndoRedoScenario {
    /// Apply a `ConflictSetRegionChoice` to every region in a deep session.
    DeepStack,
    /// Apply N region choices, reset all, then replay the same N choices.
    UndoReplay,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UndoRedoMetrics {
    pub region_count: u64,
    pub apply_dispatches: u64,
    pub reset_dispatches: u64,
    pub replay_dispatches: u64,
    pub conflict_rev_delta: u64,
    pub total_effects: u64,
}

pub struct UndoRedoFixture {
    baseline: AppState,
    conflict_path: RepoPath,
    region_count: usize,
    scenario: UndoRedoScenario,
}

impl UndoRedoFixture {
    /// Deep stack: apply a choice to every region in a session with `region_count`
    /// conflict regions. Measures the cost of N sequential `ConflictSetRegionChoice`
    /// dispatches building up resolver state.
    pub fn deep_stack(region_count: usize) -> Self {
        let (baseline, conflict_path) = build_undo_redo_baseline(region_count);
        Self {
            baseline,
            conflict_path,
            region_count,
            scenario: UndoRedoScenario::DeepStack,
        }
    }

    /// Undo-replay: apply `region_count` choices, reset all via
    /// `ConflictResetResolutions`, then replay the same choices.
    /// Measures the full undo + replay cycle cost.
    pub fn undo_replay(region_count: usize) -> Self {
        let (baseline, conflict_path) = build_undo_redo_baseline(region_count);
        Self {
            baseline,
            conflict_path,
            region_count,
            scenario: UndoRedoScenario::UndoReplay,
        }
    }

    pub fn fresh_state(&self) -> AppState {
        self.baseline.clone()
    }

    pub fn run(&self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&self) -> (u64, UndoRedoMetrics) {
        let mut state = self.fresh_state();
        self.run_with_state(&mut state)
    }

    pub fn run_with_state(&self, state: &mut AppState) -> (u64, UndoRedoMetrics) {
        let conflict_rev_before = state.repos[0].conflict_state.conflict_rev;

        let total_effects = 0u64;
        let mut apply_dispatches = 0u64;
        let mut reset_dispatches = 0u64;
        let mut replay_dispatches = 0u64;
        let mut h = FxHasher::default();

        let choices = [
            gitcomet_state::msg::ConflictRegionChoice::Ours,
            gitcomet_state::msg::ConflictRegionChoice::Theirs,
            gitcomet_state::msg::ConflictRegionChoice::Both,
            gitcomet_state::msg::ConflictRegionChoice::Base,
        ];

        match self.scenario {
            UndoRedoScenario::DeepStack => {
                // Apply one choice per region, cycling through choice variants.
                for i in 0..self.region_count {
                    set_conflict_region_choice_sync(
                        state,
                        RepoId(1),
                        self.conflict_path.clone(),
                        i,
                        choices[i % choices.len()],
                    );
                    apply_dispatches += 1;
                }
            }
            UndoRedoScenario::UndoReplay => {
                // Phase 1: Apply choices.
                for i in 0..self.region_count {
                    set_conflict_region_choice_sync(
                        state,
                        RepoId(1),
                        self.conflict_path.clone(),
                        i,
                        choices[i % choices.len()],
                    );
                    apply_dispatches += 1;
                }

                // Phase 2: Reset all resolutions (undo).
                reset_conflict_resolutions_sync(state, RepoId(1), self.conflict_path.clone());
                reset_dispatches += 1;

                // Phase 3: Replay the same choices.
                for i in 0..self.region_count {
                    set_conflict_region_choice_sync(
                        state,
                        RepoId(1),
                        self.conflict_path.clone(),
                        i,
                        choices[i % choices.len()],
                    );
                    replay_dispatches += 1;
                }
            }
        }

        let conflict_rev_after = state.repos[0].conflict_state.conflict_rev;
        conflict_rev_after.hash(&mut h);
        total_effects.hash(&mut h);

        let metrics = UndoRedoMetrics {
            region_count: self.region_count as u64,
            apply_dispatches,
            reset_dispatches,
            replay_dispatches,
            conflict_rev_delta: conflict_rev_after.wrapping_sub(conflict_rev_before),
            total_effects,
        };

        (h.finish(), metrics)
    }
}

/// Build an `AppState` with a conflict session containing `region_count` unresolved regions.
fn build_undo_redo_baseline(region_count: usize) -> (AppState, RepoPath) {
    use gitcomet_core::conflict_session::{
        ConflictPayload, ConflictRegion, ConflictRegionResolution, ConflictRegionText,
        ConflictSession,
    };
    use gitcomet_core::domain::FileConflictKind;

    let conflict_path_buf = std::path::PathBuf::from("src/conflict_undo_redo.rs");
    let conflict_path = RepoPath::from(conflict_path_buf.clone());

    // Build a full-text-resolver session with N synthetic conflict regions.
    let base_text: Arc<str> = Arc::from("base content\n");
    let ours_text: Arc<str> = Arc::from("ours content\n");
    let theirs_text: Arc<str> = Arc::from("theirs content\n");

    let mut session = ConflictSession::new(
        conflict_path_buf.clone(),
        FileConflictKind::BothModified,
        ConflictPayload::Text(Arc::clone(&base_text)),
        ConflictPayload::Text(Arc::clone(&ours_text)),
        ConflictPayload::Text(Arc::clone(&theirs_text)),
    );

    // Populate with N synthetic conflict regions.
    session.regions.clear();
    for i in 0..region_count {
        session.regions.push(ConflictRegion {
            base: Some(ConflictRegionText::from(format!(
                "base region {i} content line\n"
            ))),
            ours: ConflictRegionText::from(format!("ours region {i} modified line\n")),
            theirs: ConflictRegionText::from(format!("theirs region {i} modified line\n")),
            resolution: ConflictRegionResolution::Unresolved,
        });
    }

    let commits = build_synthetic_commits(100);
    let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
    repo.conflict_state.conflict_file_path = Some(conflict_path_buf);
    repo.conflict_state.conflict_session = Some(session);
    repo.conflict_state.conflict_rev = 1;
    repo.open = Loadable::Ready(());

    let baseline = AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        clone: None,
        notifications: Vec::new(),
        banner_error: None,
        auth_prompt: None,
    };

    (baseline, conflict_path)
}

pub struct ReplacementAlignmentFixture {
    old_text: String,
    new_text: String,
}

impl ReplacementAlignmentFixture {
    pub fn new(
        blocks: usize,
        old_block_lines: usize,
        new_block_lines: usize,
        context_lines: usize,
        line_bytes: usize,
    ) -> Self {
        let (old_text, new_text) = build_synthetic_replacement_alignment_documents(
            blocks,
            old_block_lines,
            new_block_lines,
            context_lines,
            line_bytes,
        );
        Self { old_text, new_text }
    }

    pub fn run_plan_step(&self) -> u64 {
        let plan = gitcomet_core::file_diff::side_by_side_plan(&self.old_text, &self.new_text);
        hash_file_diff_plan(&plan)
    }

    pub fn run_plan_step_with_backend(
        &self,
        backend: gitcomet_core::file_diff::BenchmarkReplacementDistanceBackend,
    ) -> u64 {
        let plan = gitcomet_core::file_diff::benchmark_side_by_side_plan_with_replacement_backend(
            &self.old_text,
            &self.new_text,
            backend,
        );
        hash_file_diff_plan(&plan)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputPrepaintWindowedMetrics {
    pub total_lines: u64,
    pub viewport_rows: u64,
    pub guard_rows: u64,
    pub max_shape_bytes: u64,
    pub cache_entries_after: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

// Mirrors the production shaped-row cache identity in `TextInput`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TextInputShapeCacheKey {
    line_ix: usize,
    wrap_width_key: i32,
}

pub struct TextInputPrepaintWindowedFixture {
    lines: Vec<String>,
    wrap_width_key: i32,
    guard_rows: usize,
    max_shape_bytes: usize,
    shape_cache: HashMap<TextInputShapeCacheKey, u64>,
}

impl TextInputPrepaintWindowedFixture {
    pub fn new(lines: usize, line_bytes: usize, wrap_width_px: usize) -> Self {
        Self {
            lines: build_synthetic_source_lines(lines.max(1), line_bytes),
            wrap_width_key: wrap_width_px.max(1) as i32,
            guard_rows: 2,
            max_shape_bytes: 4 * 1024,
            shape_cache: HashMap::default(),
        }
    }

    fn cached_shape_hash_for_line(&mut self, line_ix: usize) -> u64 {
        let key = TextInputShapeCacheKey {
            line_ix,
            wrap_width_key: self.wrap_width_key,
        };
        if let Some(cached) = self.shape_cache.get(&key) {
            return *cached;
        }

        let (slice_hash, capped_len) = hash_text_input_shaping_slice(
            self.lines.get(line_ix).map(String::as_str).unwrap_or(""),
            self.max_shape_bytes,
        );
        let mut shaped_hash = FxHasher::default();
        line_ix.hash(&mut shaped_hash);
        capped_len.hash(&mut shaped_hash);
        slice_hash.hash(&mut shaped_hash);
        let shaped = shaped_hash.finish();
        self.shape_cache.insert(key, shaped);
        shaped
    }

    pub fn run_windowed_step(&mut self, start_row: usize, viewport_rows: usize) -> u64 {
        if self.lines.is_empty() || viewport_rows == 0 {
            return 0;
        }

        let line_count = self.lines.len();
        let total_rows = viewport_rows
            .saturating_add(self.guard_rows.saturating_mul(2))
            .max(1);
        let mut h = FxHasher::default();

        for row in 0..total_rows {
            let line_ix = start_row.wrapping_add(row) % line_count;
            let shaped = self.cached_shape_hash_for_line(line_ix);
            shaped.hash(&mut h);
        }

        self.shape_cache.len().hash(&mut h);
        h.finish()
    }

    pub fn run_windowed_step_with_metrics(
        &mut self,
        start_row: usize,
        viewport_rows: usize,
    ) -> (u64, TextInputPrepaintWindowedMetrics) {
        if self.lines.is_empty() || viewport_rows == 0 {
            return (0, TextInputPrepaintWindowedMetrics::default());
        }

        let line_count = self.lines.len();
        let total_rows = viewport_rows
            .saturating_add(self.guard_rows.saturating_mul(2))
            .max(1);
        let mut h = FxHasher::default();
        let cache_before = self.shape_cache.len();

        for row in 0..total_rows {
            let line_ix = start_row.wrapping_add(row) % line_count;
            let shaped = self.cached_shape_hash_for_line(line_ix);
            shaped.hash(&mut h);
        }

        self.shape_cache.len().hash(&mut h);
        let cache_after = self.shape_cache.len();
        let cache_misses = cache_after.saturating_sub(cache_before);
        let cache_hits = total_rows.saturating_sub(cache_misses);

        (
            h.finish(),
            TextInputPrepaintWindowedMetrics {
                total_lines: bench_counter_u64(line_count),
                viewport_rows: bench_counter_u64(viewport_rows),
                guard_rows: bench_counter_u64(self.guard_rows),
                max_shape_bytes: bench_counter_u64(self.max_shape_bytes),
                cache_entries_after: bench_counter_u64(cache_after),
                cache_hits: bench_counter_u64(cache_hits),
                cache_misses: bench_counter_u64(cache_misses),
            },
        )
    }

    pub fn run_full_document_step(&mut self) -> u64 {
        self.run_windowed_step(0, self.lines.len())
    }

    pub fn run_full_document_step_with_metrics(
        &mut self,
    ) -> (u64, TextInputPrepaintWindowedMetrics) {
        let len = self.lines.len();
        self.run_windowed_step_with_metrics(0, len)
    }

    pub fn total_rows(&self) -> usize {
        self.lines.len()
    }

    #[cfg(test)]
    pub(crate) fn cache_entries(&self) -> usize {
        self.shape_cache.len()
    }
}

pub struct TextInputLongLineCapFixture {
    line: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputLongLineCapMetrics {
    pub line_bytes: u64,
    pub max_shape_bytes: u64,
    pub capped_len: u64,
    pub iterations: u64,
    pub cap_active: u64,
}

impl TextInputLongLineCapFixture {
    pub fn new(bytes: usize) -> Self {
        let bytes = bytes.max(1);
        let mut line = String::with_capacity(bytes.saturating_add(16));
        let token = "let very_long_identifier = \"token\"; ";
        while line.len() < bytes {
            line.push_str(token);
        }
        line.truncate(bytes);
        Self { line }
    }

    pub fn run_with_cap(&self, max_bytes: usize) -> u64 {
        let mut h = FxHasher::default();
        for nonce in 0..64usize {
            let (slice_hash, capped_len) =
                hash_text_input_shaping_slice(self.line.as_str(), max_bytes.max(1));
            nonce.hash(&mut h);
            slice_hash.hash(&mut h);
            capped_len.hash(&mut h);
        }
        h.finish()
    }

    pub fn run_with_cap_with_metrics(
        &self,
        max_bytes: usize,
    ) -> (u64, TextInputLongLineCapMetrics) {
        let hash = self.run_with_cap(max_bytes);
        let (_h, capped_len) = hash_text_input_shaping_slice(self.line.as_str(), max_bytes.max(1));
        let cap_active = if capped_len < self.line.len() { 1 } else { 0 };
        (
            hash,
            TextInputLongLineCapMetrics {
                line_bytes: self.line.len() as u64,
                max_shape_bytes: max_bytes as u64,
                capped_len: capped_len as u64,
                iterations: 64,
                cap_active,
            },
        )
    }

    pub fn run_without_cap_with_metrics(&self) -> (u64, TextInputLongLineCapMetrics) {
        self.run_with_cap_with_metrics(self.line.len().saturating_add(8))
    }

    pub fn run_without_cap(&self) -> u64 {
        self.run_with_cap(self.line.len().saturating_add(8))
    }

    #[cfg(test)]
    pub(crate) fn capped_len(&self, max_bytes: usize) -> usize {
        let (_hash, len) = hash_text_input_shaping_slice(self.line.as_str(), max_bytes.max(1));
        len
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextInputHighlightDensity {
    Dense,
    Sparse,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputRunsStreamedHighlightMetrics {
    pub total_lines: u64,
    pub visible_rows: u64,
    pub scroll_step: u64,
    pub total_highlights: u64,
    pub visible_highlights: u64,
    pub visible_lines_with_highlights: u64,
    pub density_dense: u64,
    pub algorithm_streamed: u64,
}

pub struct TextInputRunsStreamedHighlightFixture {
    text: String,
    line_starts: Vec<usize>,
    highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    density: TextInputHighlightDensity,
    visible_rows: usize,
    scroll_step: usize,
}

impl TextInputRunsStreamedHighlightFixture {
    pub fn new(
        lines: usize,
        line_bytes: usize,
        visible_rows: usize,
        density: TextInputHighlightDensity,
    ) -> Self {
        let source_lines = build_synthetic_source_lines(lines.max(1), line_bytes.max(24));
        let text = source_lines.join("\n");
        let line_starts = line_starts_for_text(text.as_str());
        let highlights =
            build_text_input_streamed_highlights(text.as_str(), line_starts.as_slice(), density);
        let visible_rows = visible_rows.max(1).min(line_starts.len().max(1));
        let scroll_step = (visible_rows / 2).max(1);
        Self {
            text,
            line_starts,
            highlights,
            density,
            visible_rows,
            scroll_step,
        }
    }

    fn max_start_row(&self) -> usize {
        self.line_starts.len().saturating_sub(self.visible_rows)
    }

    fn visible_range(&self, start_row: usize) -> Range<usize> {
        if self.line_starts.is_empty() {
            return 0..0;
        }
        let max_start = self.max_start_row();
        let start = if max_start == 0 {
            0
        } else {
            start_row % (max_start + 1)
        };
        start..start.saturating_add(self.visible_rows)
    }

    fn line_byte_range(&self, line_ix: usize) -> Range<usize> {
        let start = self
            .line_starts
            .get(line_ix)
            .copied()
            .unwrap_or(self.text.len());
        let mut end = self
            .line_starts
            .get(line_ix + 1)
            .copied()
            .unwrap_or(self.text.len());
        if end > start && self.text.as_bytes().get(end - 1) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        start..end
    }

    fn metrics_for_visible_range(
        &self,
        visible_range: Range<usize>,
        algorithm_streamed: bool,
    ) -> TextInputRunsStreamedHighlightMetrics {
        // Highlights are generated line-by-line and sorted by start offset, so a
        // monotonic scan preserves deterministic counts without rescanning the
        // whole highlight list for every visible line.
        let mut highlight_ix = 0usize;
        let mut visible_highlights = 0usize;
        let mut visible_lines_with_highlights = 0usize;

        for line_ix in visible_range.clone() {
            let line_range = self.line_byte_range(line_ix);

            while self
                .highlights
                .get(highlight_ix)
                .map(|(range, _)| range.end <= line_range.start)
                .unwrap_or(false)
            {
                highlight_ix += 1;
            }

            let mut scan_ix = highlight_ix;
            let mut line_has_highlight = false;
            while let Some((range, _)) = self.highlights.get(scan_ix) {
                if range.start >= line_range.end {
                    break;
                }
                if range.end > line_range.start {
                    visible_highlights += 1;
                    line_has_highlight = true;
                }
                scan_ix += 1;
            }

            if line_has_highlight {
                visible_lines_with_highlights += 1;
            }
            highlight_ix = scan_ix;
        }

        TextInputRunsStreamedHighlightMetrics {
            total_lines: bench_counter_u64(self.line_starts.len()),
            visible_rows: bench_counter_u64(visible_range.len()),
            scroll_step: bench_counter_u64(self.scroll_step),
            total_highlights: bench_counter_u64(self.highlights.len()),
            visible_highlights: bench_counter_u64(visible_highlights),
            visible_lines_with_highlights: bench_counter_u64(visible_lines_with_highlights),
            density_dense: if matches!(self.density, TextInputHighlightDensity::Dense) {
                1
            } else {
                0
            },
            algorithm_streamed: if algorithm_streamed { 1 } else { 0 },
        }
    }

    pub fn run_legacy_step(&self, start_row: usize) -> u64 {
        benchmark_text_input_runs_legacy_visible_window(
            self.text.as_str(),
            self.line_starts.as_slice(),
            self.visible_range(start_row),
            self.highlights.as_slice(),
        )
    }

    pub fn run_legacy_step_with_metrics(
        &self,
        start_row: usize,
    ) -> (u64, TextInputRunsStreamedHighlightMetrics) {
        let visible_range = self.visible_range(start_row);
        let hash = benchmark_text_input_runs_legacy_visible_window(
            self.text.as_str(),
            self.line_starts.as_slice(),
            visible_range.clone(),
            self.highlights.as_slice(),
        );
        (hash, self.metrics_for_visible_range(visible_range, false))
    }

    pub fn run_streamed_step(&self, start_row: usize) -> u64 {
        benchmark_text_input_runs_streamed_visible_window(
            self.text.as_str(),
            self.line_starts.as_slice(),
            self.visible_range(start_row),
            self.highlights.as_slice(),
        )
    }

    pub fn run_streamed_step_with_metrics(
        &self,
        start_row: usize,
    ) -> (u64, TextInputRunsStreamedHighlightMetrics) {
        let visible_range = self.visible_range(start_row);
        let hash = benchmark_text_input_runs_streamed_visible_window(
            self.text.as_str(),
            self.line_starts.as_slice(),
            visible_range.clone(),
            self.highlights.as_slice(),
        );
        (hash, self.metrics_for_visible_range(visible_range, true))
    }

    pub fn next_start_row(&self, start_row: usize) -> usize {
        let max_start = self.max_start_row().max(1);
        start_row.wrapping_add(self.scroll_step) % (max_start + 1)
    }

    #[cfg(test)]
    pub(crate) fn highlights_len(&self) -> usize {
        self.highlights.len()
    }
}

pub struct TextInputWrapIncrementalTabsFixture {
    lines: Vec<String>,
    first_tab_ixs: Vec<Option<usize>>,
    row_counts: Vec<usize>,
    wrap_columns: usize,
    edit_nonce: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputWrapIncrementalTabsMetrics {
    pub total_lines: u64,
    pub line_bytes: u64,
    pub wrap_columns: u64,
    pub edit_line_ix: u64,
    pub dirty_lines: u64,
    pub total_rows_after: u64,
    pub recomputed_lines: u64,
    pub incremental_patch: u64,
}

impl TextInputWrapIncrementalTabsFixture {
    pub fn new(lines: usize, line_bytes: usize, wrap_width_px: usize) -> Self {
        let lines = build_synthetic_tabbed_source_lines(lines.max(1), line_bytes.max(8));
        let first_tab_ixs = vec![Some(0); lines.len()];
        let wrap_columns = wrap_columns_for_benchmark_width(wrap_width_px.max(1));
        let mut row_counts = Vec::with_capacity(lines.len());
        recompute_all_tabbed_wrap_rows_in_place(lines.as_slice(), wrap_columns, &mut row_counts);
        Self {
            lines,
            first_tab_ixs,
            row_counts,
            wrap_columns,
            edit_nonce: 0,
        }
    }

    fn normalized_line_ix(&self, edit_line_ix: usize) -> usize {
        if self.lines.is_empty() {
            0
        } else {
            edit_line_ix % self.lines.len()
        }
    }

    fn apply_edit(&mut self, edit_line_ix: usize) -> (usize, usize, Range<usize>) {
        if self.lines.is_empty() {
            return (0, 0, 0..0);
        }

        let line_ix = self.normalized_line_ix(edit_line_ix);
        mutate_tabbed_line_for_wrap_patch(
            self.lines.get_mut(line_ix).expect("line index must exist"),
            self.first_tab_ixs
                .get_mut(line_ix)
                .expect("line metadata must exist"),
            self.edit_nonce,
        );
        self.edit_nonce = self.edit_nonce.wrapping_add(1);
        let line_bytes = self.lines.get(line_ix).map(String::len).unwrap_or(0);
        let dirty = expand_tabbed_dirty_line_range(self.first_tab_ixs.as_slice(), line_ix);
        (line_ix, line_bytes, dirty)
    }

    fn metrics_for_step(
        &self,
        line_ix: usize,
        line_bytes: usize,
        dirty: &Range<usize>,
        recomputed_lines: usize,
        incremental_patch: bool,
    ) -> TextInputWrapIncrementalTabsMetrics {
        let total_rows_after = self.row_counts.iter().copied().sum::<usize>();
        TextInputWrapIncrementalTabsMetrics {
            total_lines: bench_counter_u64(self.lines.len()),
            line_bytes: bench_counter_u64(line_bytes),
            wrap_columns: bench_counter_u64(self.wrap_columns),
            edit_line_ix: bench_counter_u64(line_ix),
            dirty_lines: bench_counter_u64(dirty.end.saturating_sub(dirty.start)),
            total_rows_after: bench_counter_u64(total_rows_after),
            recomputed_lines: bench_counter_u64(recomputed_lines),
            incremental_patch: if incremental_patch { 1 } else { 0 },
        }
    }

    pub fn run_full_recompute_step(&mut self, edit_line_ix: usize) -> u64 {
        if self.lines.is_empty() {
            return 0;
        }
        let (_line_ix, _line_bytes, _dirty) = self.apply_edit(edit_line_ix);
        recompute_all_tabbed_wrap_rows_in_place(
            self.lines.as_slice(),
            self.wrap_columns,
            &mut self.row_counts,
        );
        hash_wrap_rows(self.row_counts.as_slice())
    }

    pub fn run_full_recompute_step_with_metrics(
        &mut self,
        edit_line_ix: usize,
    ) -> (u64, TextInputWrapIncrementalTabsMetrics) {
        if self.lines.is_empty() {
            return (0, TextInputWrapIncrementalTabsMetrics::default());
        }
        let (line_ix, line_bytes, dirty) = self.apply_edit(edit_line_ix);
        recompute_all_tabbed_wrap_rows_in_place(
            self.lines.as_slice(),
            self.wrap_columns,
            &mut self.row_counts,
        );
        let hash = hash_wrap_rows(self.row_counts.as_slice());
        let metrics =
            self.metrics_for_step(line_ix, line_bytes, &dirty, self.row_counts.len(), false);
        (hash, metrics)
    }

    pub fn run_incremental_step(&mut self, edit_line_ix: usize) -> u64 {
        if self.lines.is_empty() {
            return 0;
        }
        let (_line_ix, _line_bytes, dirty) = self.apply_edit(edit_line_ix);
        for ix in dirty {
            if let Some(slot) = self.row_counts.get_mut(ix) {
                *slot = estimate_tabbed_wrap_rows(
                    self.lines.get(ix).map(String::as_str).unwrap_or(""),
                    self.wrap_columns,
                );
            }
        }
        hash_wrap_rows(self.row_counts.as_slice())
    }

    pub fn run_incremental_step_with_metrics(
        &mut self,
        edit_line_ix: usize,
    ) -> (u64, TextInputWrapIncrementalTabsMetrics) {
        if self.lines.is_empty() {
            return (0, TextInputWrapIncrementalTabsMetrics::default());
        }
        let (line_ix, line_bytes, dirty) = self.apply_edit(edit_line_ix);
        let recomputed_lines = dirty.end.saturating_sub(dirty.start);
        for ix in dirty.clone() {
            if let Some(slot) = self.row_counts.get_mut(ix) {
                *slot = estimate_tabbed_wrap_rows(
                    self.lines.get(ix).map(String::as_str).unwrap_or(""),
                    self.wrap_columns,
                );
            }
        }
        let hash = hash_wrap_rows(self.row_counts.as_slice());
        let metrics = self.metrics_for_step(line_ix, line_bytes, &dirty, recomputed_lines, true);
        (hash, metrics)
    }

    #[cfg(test)]
    pub(crate) fn row_counts(&self) -> &[usize] {
        self.row_counts.as_slice()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputWrapIncrementalBurstEditsMetrics {
    pub total_lines: u64,
    pub edits_per_burst: u64,
    pub wrap_columns: u64,
    pub total_dirty_lines: u64,
    pub total_rows_after: u64,
    pub recomputed_lines: u64,
    pub incremental_patch: u64,
}

pub struct TextInputWrapIncrementalBurstEditsFixture {
    lines: Vec<String>,
    first_tab_ixs: Vec<Option<usize>>,
    row_counts: Vec<usize>,
    wrap_columns: usize,
    edit_nonce: usize,
}

impl TextInputWrapIncrementalBurstEditsFixture {
    pub fn new(lines: usize, line_bytes: usize, wrap_width_px: usize) -> Self {
        let lines = build_synthetic_tabbed_source_lines(lines.max(1), line_bytes.max(8));
        let first_tab_ixs = vec![Some(0); lines.len()];
        let wrap_columns = wrap_columns_for_benchmark_width(wrap_width_px.max(1));
        let mut row_counts = Vec::with_capacity(lines.len());
        recompute_all_tabbed_wrap_rows_in_place(lines.as_slice(), wrap_columns, &mut row_counts);
        Self {
            lines,
            first_tab_ixs,
            row_counts,
            wrap_columns,
            edit_nonce: 0,
        }
    }

    pub fn run_full_recompute_burst_step(&mut self, edits_per_burst: usize) -> u64 {
        if self.lines.is_empty() {
            return 0;
        }
        let edits_per_burst = edits_per_burst.max(1);
        for step in 0..edits_per_burst {
            let line_ix = self.edit_nonce.wrapping_add(step).wrapping_mul(17) % self.lines.len();
            mutate_tabbed_line_for_wrap_patch(
                self.lines.get_mut(line_ix).expect("line index must exist"),
                self.first_tab_ixs
                    .get_mut(line_ix)
                    .expect("line metadata must exist"),
                self.edit_nonce.wrapping_add(step),
            );
            recompute_all_tabbed_wrap_rows_in_place(
                self.lines.as_slice(),
                self.wrap_columns,
                &mut self.row_counts,
            );
        }
        self.edit_nonce = self.edit_nonce.wrapping_add(edits_per_burst);
        hash_wrap_rows(self.row_counts.as_slice())
    }

    pub fn run_incremental_burst_step(&mut self, edits_per_burst: usize) -> u64 {
        if self.lines.is_empty() {
            return 0;
        }
        let edits_per_burst = edits_per_burst.max(1);
        for step in 0..edits_per_burst {
            let line_ix = self.edit_nonce.wrapping_add(step).wrapping_mul(17) % self.lines.len();
            mutate_tabbed_line_for_wrap_patch(
                self.lines.get_mut(line_ix).expect("line index must exist"),
                self.first_tab_ixs
                    .get_mut(line_ix)
                    .expect("line metadata must exist"),
                self.edit_nonce.wrapping_add(step),
            );
            let dirty = burst_edit_dirty_line_range(self.lines.len(), line_ix);
            for ix in dirty {
                if let Some(slot) = self.row_counts.get_mut(ix) {
                    *slot = estimate_tabbed_wrap_rows(
                        self.lines.get(ix).map(String::as_str).unwrap_or(""),
                        self.wrap_columns,
                    );
                }
            }
        }
        self.edit_nonce = self.edit_nonce.wrapping_add(edits_per_burst);
        hash_wrap_rows(self.row_counts.as_slice())
    }

    pub fn run_full_recompute_burst_step_with_metrics(
        &mut self,
        edits_per_burst: usize,
    ) -> (u64, TextInputWrapIncrementalBurstEditsMetrics) {
        if self.lines.is_empty() {
            return (0, TextInputWrapIncrementalBurstEditsMetrics::default());
        }
        let edits_per_burst = edits_per_burst.max(1);
        let mut total_dirty_lines: usize = 0;
        let mut recomputed_lines: usize = 0;
        for step in 0..edits_per_burst {
            let line_ix = self.edit_nonce.wrapping_add(step).wrapping_mul(17) % self.lines.len();
            mutate_tabbed_line_for_wrap_patch(
                self.lines.get_mut(line_ix).expect("line index must exist"),
                self.first_tab_ixs
                    .get_mut(line_ix)
                    .expect("line metadata must exist"),
                self.edit_nonce.wrapping_add(step),
            );
            let dirty = burst_edit_dirty_line_range(self.lines.len(), line_ix);
            total_dirty_lines += dirty.end.saturating_sub(dirty.start);
            recompute_all_tabbed_wrap_rows_in_place(
                self.lines.as_slice(),
                self.wrap_columns,
                &mut self.row_counts,
            );
            recomputed_lines += self.lines.len();
        }
        self.edit_nonce = self.edit_nonce.wrapping_add(edits_per_burst);
        let hash = hash_wrap_rows(self.row_counts.as_slice());
        let total_rows_after = self.row_counts.iter().copied().sum::<usize>();
        let metrics = TextInputWrapIncrementalBurstEditsMetrics {
            total_lines: bench_counter_u64(self.lines.len()),
            edits_per_burst: bench_counter_u64(edits_per_burst),
            wrap_columns: bench_counter_u64(self.wrap_columns),
            total_dirty_lines: bench_counter_u64(total_dirty_lines),
            total_rows_after: bench_counter_u64(total_rows_after),
            recomputed_lines: bench_counter_u64(recomputed_lines),
            incremental_patch: 0,
        };
        (hash, metrics)
    }

    pub fn run_incremental_burst_step_with_metrics(
        &mut self,
        edits_per_burst: usize,
    ) -> (u64, TextInputWrapIncrementalBurstEditsMetrics) {
        if self.lines.is_empty() {
            return (0, TextInputWrapIncrementalBurstEditsMetrics::default());
        }
        let edits_per_burst = edits_per_burst.max(1);
        let mut total_dirty_lines: usize = 0;
        let mut recomputed_lines: usize = 0;
        for step in 0..edits_per_burst {
            let line_ix = self.edit_nonce.wrapping_add(step).wrapping_mul(17) % self.lines.len();
            mutate_tabbed_line_for_wrap_patch(
                self.lines.get_mut(line_ix).expect("line index must exist"),
                self.first_tab_ixs
                    .get_mut(line_ix)
                    .expect("line metadata must exist"),
                self.edit_nonce.wrapping_add(step),
            );
            let dirty = burst_edit_dirty_line_range(self.lines.len(), line_ix);
            let dirty_count = dirty.end.saturating_sub(dirty.start);
            total_dirty_lines += dirty_count;
            recomputed_lines += dirty_count;
            for ix in dirty {
                if let Some(slot) = self.row_counts.get_mut(ix) {
                    *slot = estimate_tabbed_wrap_rows(
                        self.lines.get(ix).map(String::as_str).unwrap_or(""),
                        self.wrap_columns,
                    );
                }
            }
        }
        self.edit_nonce = self.edit_nonce.wrapping_add(edits_per_burst);
        let hash = hash_wrap_rows(self.row_counts.as_slice());
        let total_rows_after = self.row_counts.iter().copied().sum::<usize>();
        let metrics = TextInputWrapIncrementalBurstEditsMetrics {
            total_lines: bench_counter_u64(self.lines.len()),
            edits_per_burst: bench_counter_u64(edits_per_burst),
            wrap_columns: bench_counter_u64(self.wrap_columns),
            total_dirty_lines: bench_counter_u64(total_dirty_lines),
            total_rows_after: bench_counter_u64(total_rows_after),
            recomputed_lines: bench_counter_u64(recomputed_lines),
            incremental_patch: 1,
        };
        (hash, metrics)
    }

    #[cfg(test)]
    pub(crate) fn row_counts(&self) -> &[usize] {
        self.row_counts.as_slice()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextModelSnapshotCloneCostMetrics {
    pub document_bytes: u64,
    pub line_starts: u64,
    pub clone_count: u64,
    pub sampled_prefix_bytes: u64,
    pub snapshot_path: u64,
}

const TEXT_MODEL_SNAPSHOT_CLONE_SAMPLE_BYTES: usize = 96;

pub struct TextModelSnapshotCloneCostFixture {
    pub(crate) model: TextModel,
    pub(crate) string_control: SharedString,
    string_control_sampled_prefix_bytes: usize,
}

impl TextModelSnapshotCloneCostFixture {
    pub fn new(min_bytes: usize) -> Self {
        let text = build_text_model_document(min_bytes.max(1));
        let model = TextModel::from_large_text(text.as_str());
        let string_control = model.as_shared_string();
        let string_control_sampled_prefix_bytes = string_control
            .len()
            .min(TEXT_MODEL_SNAPSHOT_CLONE_SAMPLE_BYTES);
        Self {
            model,
            string_control,
            string_control_sampled_prefix_bytes,
        }
    }

    fn metrics(
        &self,
        clones: usize,
        sampled_prefix_bytes: usize,
        snapshot_path: bool,
    ) -> TextModelSnapshotCloneCostMetrics {
        TextModelSnapshotCloneCostMetrics {
            document_bytes: bench_counter_u64(self.model.len()),
            line_starts: bench_counter_u64(self.model.line_starts().len()),
            clone_count: bench_counter_u64(clones),
            sampled_prefix_bytes: bench_counter_u64(sampled_prefix_bytes),
            snapshot_path: if snapshot_path { 1 } else { 0 },
        }
    }

    pub fn run_snapshot_clone_step(&self, clones: usize) -> u64 {
        self.run_snapshot_clone_step_with_metrics(clones).0
    }

    pub fn run_snapshot_clone_step_with_metrics(
        &self,
        clones: usize,
    ) -> (u64, TextModelSnapshotCloneCostMetrics) {
        let clones = clones.max(1);
        let snapshot = self.model.snapshot();
        let mut h = FxHasher::default();
        self.model.model_id().hash(&mut h);
        self.model.revision().hash(&mut h);
        let mut sampled_prefix_bytes = 0usize;

        for nonce in 0..clones {
            let cloned = snapshot.clone();
            nonce.hash(&mut h);
            cloned.len().hash(&mut h);
            cloned.line_starts().len().hash(&mut h);
            let prefix = cloned.slice(0..TEXT_MODEL_SNAPSHOT_CLONE_SAMPLE_BYTES);
            sampled_prefix_bytes = prefix.len();
            prefix.len().hash(&mut h);
        }
        let metrics = self.metrics(clones, sampled_prefix_bytes, true);
        (h.finish(), metrics)
    }

    pub fn run_string_clone_control_step(&self, clones: usize) -> u64 {
        self.run_string_clone_control_step_with_metrics(clones).0
    }

    pub fn run_string_clone_control_step_with_metrics(
        &self,
        clones: usize,
    ) -> (u64, TextModelSnapshotCloneCostMetrics) {
        let clones = clones.max(1);
        let mut h = FxHasher::default();
        let sampled_prefix_bytes = self.string_control_sampled_prefix_bytes;
        for nonce in 0..clones {
            let cloned = self.string_control.clone();
            nonce.hash(&mut h);
            cloned.len().hash(&mut h);
            sampled_prefix_bytes.hash(&mut h);
        }
        let metrics = self.metrics(clones, sampled_prefix_bytes, false);
        (h.finish(), metrics)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextModelBulkLoadLargeMetrics {
    pub source_bytes: u64,
    pub document_bytes_after: u64,
    pub line_starts_after: u64,
    pub chunk_count: u64,
    pub load_variant: u64,
}

pub struct TextModelBulkLoadLargeFixture {
    pub text: String,
    pub(crate) control_chunk_ranges: Vec<Range<usize>>,
    control_sampled_prefix_bytes: usize,
}

impl TextModelBulkLoadLargeFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let mut text = String::new();
        let synthetic_lines = build_synthetic_source_lines(lines.max(1), line_bytes.max(32));
        for line in synthetic_lines {
            text.push_str(line.as_str());
            text.push('\n');
        }
        let control_chunk_ranges = utf8_chunk_ranges(text.as_str(), 32 * 1024);
        let control_sampled_prefix_bytes = text.len().min(96);
        Self {
            text,
            control_chunk_ranges,
            control_sampled_prefix_bytes,
        }
    }

    pub fn run_piece_table_bulk_load_step(&self) -> u64 {
        self.run_piece_table_bulk_load_step_with_metrics().0
    }

    pub fn run_piece_table_bulk_load_step_with_metrics(
        &self,
    ) -> (u64, TextModelBulkLoadLargeMetrics) {
        if self.text.is_empty() {
            return (0, TextModelBulkLoadLargeMetrics::default());
        }

        let mut model = TextModel::new();
        let mut split = self.text.len() / 2;
        while split > 0 && !self.text.is_char_boundary(split) {
            split = split.saturating_sub(1);
        }

        let _ = model.append_large(&self.text[..split]);
        let _ = model.append_large(&self.text[split..]);
        let snapshot = model.snapshot();

        let mut h = FxHasher::default();
        snapshot.len().hash(&mut h);
        snapshot.line_starts().len().hash(&mut h);
        let suffix_start = snapshot.clamp_to_char_boundary(snapshot.len().saturating_sub(96));
        let suffix = snapshot.slice_to_string(suffix_start..snapshot.len());
        suffix.len().hash(&mut h);

        let metrics = TextModelBulkLoadLargeMetrics {
            source_bytes: bench_counter_u64(self.text.len()),
            document_bytes_after: bench_counter_u64(snapshot.len()),
            line_starts_after: bench_counter_u64(snapshot.line_starts().len()),
            chunk_count: 2,
            load_variant: 0,
        };
        (h.finish(), metrics)
    }

    pub fn run_piece_table_from_large_text_step(&self) -> u64 {
        self.run_piece_table_from_large_text_step_with_metrics().0
    }

    pub fn run_piece_table_from_large_text_step_with_metrics(
        &self,
    ) -> (u64, TextModelBulkLoadLargeMetrics) {
        if self.text.is_empty() {
            return (0, TextModelBulkLoadLargeMetrics::default());
        }

        let model = TextModel::from_large_text(self.text.as_str());
        let snapshot = model.snapshot();
        let mut h = FxHasher::default();
        snapshot.len().hash(&mut h);
        snapshot.line_starts().len().hash(&mut h);
        let prefix_end = snapshot.clamp_to_char_boundary(snapshot.len().min(96));
        let prefix = snapshot.slice_to_string(0..prefix_end);
        prefix.len().hash(&mut h);

        let metrics = TextModelBulkLoadLargeMetrics {
            source_bytes: bench_counter_u64(self.text.len()),
            document_bytes_after: bench_counter_u64(snapshot.len()),
            line_starts_after: bench_counter_u64(snapshot.line_starts().len()),
            chunk_count: 1,
            load_variant: 1,
        };
        (h.finish(), metrics)
    }

    pub fn run_string_bulk_load_control_step(&self) -> u64 {
        self.run_string_bulk_load_control_step_with_metrics().0
    }

    pub fn run_string_bulk_load_control_step_with_metrics(
        &self,
    ) -> (u64, TextModelBulkLoadLargeMetrics) {
        if self.text.is_empty() {
            return (0, TextModelBulkLoadLargeMetrics::default());
        }

        let mut loaded = String::with_capacity(self.text.len());
        for range in &self.control_chunk_ranges {
            loaded.push_str(&self.text[range.clone()]);
        }
        let mut h = FxHasher::default();
        loaded.len().hash(&mut h);
        self.control_sampled_prefix_bytes.hash(&mut h);

        let metrics = TextModelBulkLoadLargeMetrics {
            source_bytes: bench_counter_u64(self.text.len()),
            document_bytes_after: bench_counter_u64(loaded.len()),
            line_starts_after: 0,
            chunk_count: bench_counter_u64(self.control_chunk_ranges.len()),
            load_variant: 2,
        };
        (h.finish(), metrics)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextModelFragmentedEditsMetrics {
    pub initial_bytes: u64,
    pub edit_count: u64,
    pub deleted_bytes: u64,
    pub inserted_bytes: u64,
    pub final_bytes: u64,
    pub line_starts_after: u64,
    pub readback_operations: u64,
    pub string_control: u64,
}

#[derive(Clone, Debug)]
struct TextModelFragmentedEdit {
    offset: usize,
    delete_len: usize,
    insert: String,
    insert_newlines: usize,
}

pub struct TextModelFragmentedEditFixture {
    /// The initial document text, used to build fresh models per iteration.
    initial_text: String,
    initial_line_starts: usize,
    /// Pre-computed edit sequence over ASCII-only document content.
    edits: Vec<TextModelFragmentedEdit>,
}

impl TextModelFragmentedEditFixture {
    pub fn new(min_bytes: usize, edit_count: usize) -> Self {
        let initial_text = build_text_model_document(min_bytes.max(1024));
        let doc_len = initial_text.len();
        let edits = build_deterministic_edits(&initial_text, doc_len, edit_count.max(1));
        let initial_line_starts = text_line_starts_for_benchmark(initial_text.as_str());
        Self {
            initial_text,
            initial_line_starts,
            edits,
        }
    }

    fn metrics(
        &self,
        deleted_bytes: usize,
        inserted_bytes: usize,
        final_bytes: usize,
        line_starts_after: usize,
        readback_operations: usize,
        string_control: bool,
    ) -> TextModelFragmentedEditsMetrics {
        TextModelFragmentedEditsMetrics {
            initial_bytes: bench_counter_u64(self.initial_text.len()),
            edit_count: bench_counter_u64(self.edits.len()),
            deleted_bytes: bench_counter_u64(deleted_bytes),
            inserted_bytes: bench_counter_u64(inserted_bytes),
            final_bytes: bench_counter_u64(final_bytes),
            line_starts_after: bench_counter_u64(line_starts_after),
            readback_operations: bench_counter_u64(readback_operations),
            string_control: if string_control { 1 } else { 0 },
        }
    }

    fn apply_edits_to_model(&self) -> (TextModel, usize, usize) {
        let mut model = TextModel::from_large_text(&self.initial_text);
        let mut deleted_bytes = 0usize;
        let mut inserted_bytes = 0usize;
        for edit in &self.edits {
            let end = edit.offset.saturating_add(edit.delete_len).min(model.len());
            let start = edit.offset.min(model.len());
            deleted_bytes = deleted_bytes.saturating_add(end.saturating_sub(start));
            inserted_bytes = inserted_bytes.saturating_add(edit.insert.len());
            let _ = model.replace_range(start..end, edit.insert.as_str());
        }
        (model, deleted_bytes, inserted_bytes)
    }

    fn apply_edits_to_string(&self) -> (String, usize, usize, usize) {
        let mut text = self.initial_text.clone();
        let mut deleted_bytes = 0usize;
        let mut inserted_bytes = 0usize;
        let mut line_starts_after = self.initial_line_starts;
        for edit in &self.edits {
            let start = edit.offset.min(text.len());
            let end = edit.offset.saturating_add(edit.delete_len).min(text.len());
            let deleted_newlines = memchr::memchr_iter(b'\n', text[start..end].as_bytes()).count();
            deleted_bytes = deleted_bytes.saturating_add(end.saturating_sub(start));
            inserted_bytes = inserted_bytes.saturating_add(edit.insert.len());
            line_starts_after = line_starts_after
                .saturating_sub(deleted_newlines)
                .saturating_add(edit.insert_newlines);
            text.replace_range(start..end, edit.insert.as_str());
        }
        (text, deleted_bytes, inserted_bytes, line_starts_after)
    }

    /// Benchmark: apply all edits to a fresh piece-table model.
    pub fn run_fragmented_edit_step(&self) -> u64 {
        self.run_fragmented_edit_step_with_metrics().0
    }

    pub fn run_fragmented_edit_step_with_metrics(&self) -> (u64, TextModelFragmentedEditsMetrics) {
        let (model, deleted_bytes, inserted_bytes) = self.apply_edits_to_model();
        let mut h = FxHasher::default();
        model.len().hash(&mut h);
        model.revision().hash(&mut h);
        let metrics = self.metrics(
            deleted_bytes,
            inserted_bytes,
            model.len(),
            model.line_starts().len(),
            0,
            false,
        );
        (h.finish(), metrics)
    }

    /// Benchmark: apply all edits, then materialize via `as_str()`.
    pub fn run_materialize_after_edits_step(&self) -> u64 {
        self.run_materialize_after_edits_step_with_metrics().0
    }

    pub fn run_materialize_after_edits_step_with_metrics(
        &self,
    ) -> (u64, TextModelFragmentedEditsMetrics) {
        let (model, deleted_bytes, inserted_bytes) = self.apply_edits_to_model();
        let text = model.as_str();
        let mut h = FxHasher::default();
        text.len().hash(&mut h);
        text.bytes().take(128).count().hash(&mut h);
        let metrics = self.metrics(
            deleted_bytes,
            inserted_bytes,
            text.len(),
            model.line_starts().len(),
            1,
            false,
        );
        (h.finish(), metrics)
    }

    /// Benchmark: apply all edits, then call `as_shared_string()` repeatedly.
    pub fn run_shared_string_after_edits_step(&self, reads: usize) -> u64 {
        self.run_shared_string_after_edits_step_with_metrics(reads)
            .0
    }

    pub fn run_shared_string_after_edits_step_with_metrics(
        &self,
        reads: usize,
    ) -> (u64, TextModelFragmentedEditsMetrics) {
        let reads = reads.max(1);
        let (model, deleted_bytes, inserted_bytes) = self.apply_edits_to_model();
        let mut h = FxHasher::default();
        for nonce in 0..reads {
            let ss = model.as_shared_string();
            nonce.hash(&mut h);
            ss.len().hash(&mut h);
        }
        let metrics = self.metrics(
            deleted_bytes,
            inserted_bytes,
            model.len(),
            model.line_starts().len(),
            reads,
            false,
        );
        (h.finish(), metrics)
    }

    /// Control: apply the same edits to a plain `String`.
    pub fn run_string_edit_control_step(&self) -> u64 {
        self.run_string_edit_control_step_with_metrics().0
    }

    pub fn run_string_edit_control_step_with_metrics(
        &self,
    ) -> (u64, TextModelFragmentedEditsMetrics) {
        let (text, deleted_bytes, inserted_bytes, line_starts_after) = self.apply_edits_to_string();
        let mut h = FxHasher::default();
        text.len().hash(&mut h);
        text.len().min(128).hash(&mut h);
        let metrics = self.metrics(
            deleted_bytes,
            inserted_bytes,
            text.len(),
            line_starts_after,
            0,
            true,
        );
        (h.finish(), metrics)
    }
}

/// Build a deterministic pseudo-random edit sequence that stays within document bounds.
fn build_deterministic_edits(
    text: &str,
    initial_len: usize,
    count: usize,
) -> Vec<TextModelFragmentedEdit> {
    let mut edits = Vec::with_capacity(count);
    // Track approximate document length to keep offsets in bounds.
    let mut approx_len = initial_len;
    let mut seed = 0x517cc1b727220a95u64;
    for ix in 0..count {
        // Simple xorshift-style PRNG for determinism.
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed = seed.wrapping_add(ix as u64);

        let offset = if approx_len > 0 {
            (seed as usize) % approx_len
        } else {
            0
        };
        // Clamp offset to a char boundary in the initial text (approximate).
        let offset = clamp_byte_to_char_boundary(text, offset);

        let delete_len = ((seed >> 16) as usize) % 16;
        let (insert, insert_newlines) = match ix % 5 {
            0 => (format!("edit_{ix}"), 0),
            1 => (format!("fn f{ix}() {{ }}\n"), 1),
            2 => (String::new(), 0), // pure delete
            3 => (format!("/* {ix} */"), 0),
            _ => (format!("x{ix}\ny{ix}\n"), 2),
        };
        approx_len = approx_len
            .saturating_sub(delete_len.min(approx_len.saturating_sub(offset)))
            .saturating_add(insert.len());
        edits.push(TextModelFragmentedEdit {
            offset,
            delete_len,
            insert,
            insert_newlines,
        });
    }
    edits
}

fn clamp_byte_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn utf8_chunk_ranges(text: &str, chunk_bytes: usize) -> Vec<Range<usize>> {
    if text.is_empty() {
        return Vec::new();
    }

    let chunk_bytes = chunk_bytes.max(1);
    let mut ranges = Vec::with_capacity(text.len() / chunk_bytes + 1);
    let mut start = 0usize;
    while start < text.len() {
        let mut end = clamp_byte_to_char_boundary(text, (start + chunk_bytes).min(text.len()));
        if end == start {
            end = text.len();
        }
        ranges.push(start..end);
        start = end;
    }
    ranges
}

fn text_line_starts_for_benchmark(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
        .saturating_add(1)
}

fn build_text_model_document(min_bytes: usize) -> String {
    let mut out = String::with_capacity(min_bytes.saturating_add(64));
    let mut ix = 0usize;
    while out.len() < min_bytes {
        out.push_str(
            format!("line_{ix:06}: fn synthetic_{ix}() {{ let value = {ix}; }}\n").as_str(),
        );
        ix = ix.wrapping_add(1);
    }
    out
}

fn build_synthetic_tabbed_source_lines(lines: usize, min_line_bytes: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.max(1));
    let target = min_line_bytes.max(8);
    for ix in 0..lines.max(1) {
        let mut line = String::new();
        line.push('\t');
        line.push_str(&format!("section_{ix:05}\t"));
        line.push_str("value = ");
        while line.len() < target {
            line.push_str("token\t");
        }
        out.push(line);
    }
    out
}

pub(crate) fn wrap_columns_for_benchmark_width(wrap_width_px: usize) -> usize {
    let estimated_char_px = (13.0f32 * 0.6).max(1.0);
    ((wrap_width_px as f32) / estimated_char_px)
        .floor()
        .max(1.0) as usize
}

fn recompute_all_tabbed_wrap_rows_in_place(
    lines: &[String],
    wrap_columns: usize,
    row_counts: &mut Vec<usize>,
) {
    row_counts.resize(lines.len().max(1), 1);
    for (slot, line) in row_counts.iter_mut().zip(lines.iter()) {
        *slot = estimate_tabbed_wrap_rows(line.as_str(), wrap_columns);
    }
}

pub(crate) fn estimate_tabbed_wrap_rows(line: &str, wrap_columns: usize) -> usize {
    estimate_text_input_wrap_rows_for_line(line, wrap_columns)
}

fn mutate_tabbed_line_for_wrap_patch(
    line: &mut String,
    first_tab_ix: &mut Option<usize>,
    nonce: usize,
) {
    if line.is_empty() {
        line.push('\t');
        *first_tab_ix = Some(0);
    }
    let insert_ix = first_tab_ix.unwrap_or(0).min(line.len());
    let ch = (b'a' + (nonce % 26) as u8) as char;
    line.insert(insert_ix, ch);

    if line.chars().nth(1).is_some() {
        let _ = line.pop();
    }

    *first_tab_ix = match *first_tab_ix {
        Some(_) => {
            let next_ix = insert_ix.saturating_add(1);
            (next_ix < line.len()).then_some(next_ix)
        }
        None => None,
    };
}

fn expand_tabbed_dirty_line_range(first_tab_ixs: &[Option<usize>], line_ix: usize) -> Range<usize> {
    if first_tab_ixs.is_empty() {
        return 0..0;
    }
    let line_ix = line_ix.min(first_tab_ixs.len().saturating_sub(1));
    let mut end = (line_ix + 1).min(first_tab_ixs.len());
    if end < first_tab_ixs.len() && first_tab_ixs.get(end).copied().flatten() == Some(0) {
        end = (end + 1).min(first_tab_ixs.len());
    }
    line_ix..end
}

fn burst_edit_dirty_line_range(line_count: usize, line_ix: usize) -> Range<usize> {
    if line_count == 0 {
        return 0..0;
    }
    let line_ix = line_ix.min(line_count.saturating_sub(1));
    // Live TextInput dirty-wrap invalidation only patches the edited line for
    // these single-line mutations, so burst benchmarks should not rescan a
    // synthetic leading-tab neighbor.
    line_ix..(line_ix + 1).min(line_count)
}

fn hash_wrap_rows(row_counts: &[usize]) -> u64 {
    let mut h = FxHasher::default();
    row_counts.len().hash(&mut h);
    for rows in row_counts.iter().take(512) {
        rows.hash(&mut h);
    }
    h.finish()
}
