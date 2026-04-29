use super::*;
use crate::kit::text_truncation::{
    PathTruncationAlignmentGroup, TextTruncationProfile,
    clear_truncated_layout_cache_for_benchmark, measure_candidate_calls_for_benchmark,
    path_alignment_style_key, path_alignment_visible_signature,
    reset_measure_candidate_calls_for_benchmark, shape_truncated_line_cached_with_path_anchor,
    truncated_line_ellipsis_x,
};
use gpui::TextStyle;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct StatusSelectionBenchCounters {
    position_scan_steps: u64,
}

thread_local! {
    static STATUS_SELECTION_BENCH_COUNTERS: Cell<StatusSelectionBenchCounters> =
        const { Cell::new(StatusSelectionBenchCounters { position_scan_steps: 0 }) };
}

fn bench_reset_status_selection() {
    STATUS_SELECTION_BENCH_COUNTERS
        .with(|counters| counters.set(StatusSelectionBenchCounters::default()));
}

fn bench_snapshot_status_selection() -> StatusSelectionBenchCounters {
    STATUS_SELECTION_BENCH_COUNTERS.with(Cell::get)
}

fn apply_status_multi_selection_to_slice_bench(
    selected: &mut Vec<std::path::PathBuf>,
    anchor: &mut Option<std::path::PathBuf>,
    clicked_path: std::path::PathBuf,
    modifiers: gpui::Modifiers,
    entries: Option<&[std::path::PathBuf]>,
) {
    if modifiers.shift {
        let Some(entries) = entries else {
            *selected = vec![clicked_path.clone()];
            *anchor = Some(clicked_path);
            return;
        };

        let Some(clicked_ix) = entries.iter().position(|p| p == &clicked_path) else {
            *selected = vec![clicked_path.clone()];
            *anchor = Some(clicked_path);
            return;
        };

        let anchor_path = anchor.clone().unwrap_or_else(|| clicked_path.clone());
        let anchor_ix = entries
            .iter()
            .position(|p| p == &anchor_path)
            .unwrap_or(clicked_ix);
        let (a, b) = if anchor_ix <= clicked_ix {
            (anchor_ix, clicked_ix)
        } else {
            (clicked_ix, anchor_ix)
        };
        *selected = entries[a..=b].to_vec();
        *anchor = Some(anchor_path);
        return;
    }

    if modifiers.secondary() || modifiers.control || modifiers.platform {
        if let Some(ix) = selected.iter().position(|p| p == &clicked_path) {
            selected.remove(ix);
            if selected.is_empty() {
                *anchor = None;
            }
        } else {
            selected.push(clicked_path.clone());
            *anchor = Some(clicked_path);
        }
        return;
    }

    *selected = vec![clicked_path.clone()];
    *anchor = Some(clicked_path);
}

#[allow(clippy::too_many_arguments)]
fn apply_status_multi_selection_click(
    selection: &mut StatusMultiSelection,
    section: StatusSection,
    clicked_path: std::path::PathBuf,
    anchor_index: Option<usize>,
    modifiers: gpui::Modifiers,
    _selection_count_hint: Option<usize>,
    _prefer_anchor_hint: bool,
    entries: Option<&[std::path::PathBuf]>,
) {
    if modifiers.shift {
        let scan_steps = if anchor_index.is_some() {
            0
        } else {
            entries.map_or(0, |v| v.len())
        };
        STATUS_SELECTION_BENCH_COUNTERS.with(|counters| {
            let mut snapshot = counters.get();
            snapshot.position_scan_steps = snapshot
                .position_scan_steps
                .saturating_add(scan_steps as u64);
            counters.set(snapshot);
        });
    }
    match section {
        StatusSection::CombinedUnstaged | StatusSection::Unstaged => {
            selection.untracked.clear();
            selection.untracked_anchor = None;
            selection.staged.clear();
            selection.staged_anchor = None;
            apply_status_multi_selection_to_slice_bench(
                &mut selection.unstaged,
                &mut selection.unstaged_anchor,
                clicked_path,
                modifiers,
                entries,
            );
        }
        StatusSection::Untracked => {
            selection.unstaged.clear();
            selection.unstaged_anchor = None;
            selection.staged.clear();
            selection.staged_anchor = None;
            apply_status_multi_selection_to_slice_bench(
                &mut selection.untracked,
                &mut selection.untracked_anchor,
                clicked_path,
                modifiers,
                entries,
            );
        }
        StatusSection::Staged => {
            selection.untracked.clear();
            selection.untracked_anchor = None;
            selection.unstaged.clear();
            selection.unstaged_anchor = None;
            apply_status_multi_selection_to_slice_bench(
                &mut selection.staged,
                &mut selection.staged_anchor,
                clicked_path,
                modifiers,
                entries,
            );
        }
    }
}

pub struct StatusMultiSelectMetrics {
    pub entries_total: u64,
    pub selected_paths: u64,
    pub anchor_index: u64,
    pub clicked_index: u64,
    pub anchor_preserved: u64,
    pub position_scan_steps: u64,
}

pub struct StatusMultiSelectFixture {
    entries: Vec<std::path::PathBuf>,
    anchor_index: usize,
    clicked_index: usize,
    baseline_selection: StatusMultiSelection,
}

impl StatusMultiSelectFixture {
    pub fn range_select(entries: usize, anchor_index: usize, selected_paths: usize) -> Self {
        let entries = build_synthetic_status_entries(entries.max(1), DiffArea::Unstaged)
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>();
        let max_index = entries.len().saturating_sub(1);
        let anchor_index = anchor_index.min(max_index);
        let selected_paths = selected_paths.max(1);
        let clicked_index = anchor_index
            .saturating_add(selected_paths.saturating_sub(1))
            .min(max_index);

        let mut baseline_selection = StatusMultiSelection::default();
        apply_status_multi_selection_click(
            &mut baseline_selection,
            StatusSection::CombinedUnstaged,
            entries[anchor_index].clone(),
            Some(anchor_index),
            gpui::Modifiers::default(),
            Some(1),
            true,
            Some(&entries),
        );

        Self {
            entries,
            anchor_index,
            clicked_index,
            baseline_selection,
        }
    }

    pub fn run(&self) -> u64 {
        let selection = self.run_selection();
        hash_status_multi_selection(&selection)
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, StatusMultiSelectMetrics) {
        bench_reset_status_selection();
        let selection = self.run_selection();
        let hash = hash_status_multi_selection(&selection);
        let counters = bench_snapshot_status_selection();
        let anchor_path = &self.entries[self.anchor_index];
        let selected_paths = selection.unstaged.as_slice();

        (
            hash,
            StatusMultiSelectMetrics {
                entries_total: self.entries.len() as u64,
                selected_paths: selected_paths.len() as u64,
                anchor_index: self.anchor_index as u64,
                clicked_index: self.clicked_index as u64,
                anchor_preserved: u64::from(
                    selection.unstaged_anchor.as_ref() == Some(anchor_path)
                        && selected_paths.iter().any(|path| path == anchor_path),
                ),
                position_scan_steps: counters.position_scan_steps,
            },
        )
    }

    fn run_selection(&self) -> StatusMultiSelection {
        let mut selection = self.baseline_selection.clone();
        apply_status_multi_selection_click(
            &mut selection,
            StatusSection::CombinedUnstaged,
            self.entries[self.clicked_index].clone(),
            Some(self.clicked_index),
            gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            Some(1),
            true,
            Some(&self.entries),
        );
        selection
    }
}

fn hash_status_multi_selection(selection: &StatusMultiSelection) -> u64 {
    let mut h = FxHasher::default();
    selection.unstaged.len().hash(&mut h);
    hash_optional_path_identity(selection.unstaged_anchor.as_deref(), &mut h);
    hash_status_multi_selection_path_sample(selection.unstaged.as_slice(), &mut h);
    selection.staged.len().hash(&mut h);
    hash_optional_path_identity(selection.staged_anchor.as_deref(), &mut h);
    h.finish()
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatusSelectDiffOpenMetrics {
    pub effect_count: usize,
    pub load_selected_diff_effect_count: usize,
    pub load_diff_effect_count: usize,
    pub load_diff_file_effect_count: usize,
    pub load_diff_file_image_effect_count: usize,
    pub diff_state_rev_delta: u64,
}

impl StatusSelectDiffOpenMetrics {
    fn from_effects_and_rev(effects: &[Effect], rev_before: u64, rev_after: u64) -> Self {
        let mut metrics = Self {
            effect_count: effects.len(),
            diff_state_rev_delta: rev_after.wrapping_sub(rev_before),
            ..Self::default()
        };
        for effect in effects {
            match effect {
                Effect::LoadSelectedDiff { .. } => {
                    metrics.load_selected_diff_effect_count += 1;
                }
                Effect::LoadDiff { .. } => metrics.load_diff_effect_count += 1,
                Effect::LoadDiffFile { .. } => metrics.load_diff_file_effect_count += 1,
                Effect::LoadDiffFileImage { .. } => metrics.load_diff_file_image_effect_count += 1,
                _ => {}
            }
        }
        metrics
    }
}

fn hash_status_select_diff_target(target: &DiffTarget, hasher: &mut FxHasher) {
    match target {
        DiffTarget::WorkingTree { path, area } => {
            path.hash(hasher);
            (*area as u8).hash(hasher);
        }
        DiffTarget::Commit { commit_id, path } => {
            commit_id.hash(hasher);
            path.hash(hasher);
        }
    }
}

pub struct StatusSelectDiffOpenFixture {
    baseline: AppState,
    diff_target: DiffTarget,
}

impl StatusSelectDiffOpenFixture {
    pub fn unstaged(status_entries: usize) -> Self {
        let entries = build_synthetic_status_entries(status_entries, DiffArea::Unstaged);
        let target_path = entries[entries.len() / 2].path.clone();

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        seed_repo_status_entries(&mut repo, entries, Vec::new());
        repo.open = Loadable::Ready(());

        Self {
            baseline: bench_app_state(vec![repo], Some(RepoId(1))),
            diff_target: DiffTarget::WorkingTree {
                path: target_path,
                area: DiffArea::Unstaged,
            },
        }
    }

    pub fn staged(status_entries: usize) -> Self {
        let entries = build_synthetic_status_entries(status_entries, DiffArea::Staged);
        let target_path = entries[entries.len() / 2].path.clone();

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        seed_repo_status_entries(&mut repo, Vec::new(), entries);
        repo.open = Loadable::Ready(());

        Self {
            baseline: bench_app_state(vec![repo], Some(RepoId(1))),
            diff_target: DiffTarget::WorkingTree {
                path: target_path,
                area: DiffArea::Staged,
            },
        }
    }

    pub fn fresh_state(&self) -> AppState {
        self.baseline.clone()
    }

    pub fn run_with_state(&self, state: &mut AppState) -> (u64, StatusSelectDiffOpenMetrics) {
        let rev_before = state.repos[0].diff_state.diff_state_rev;
        with_select_diff_sync(
            state,
            RepoId(1),
            self.diff_target.clone(),
            |state, effects| {
                let rev_after = state.repos[0].diff_state.diff_state_rev;
                let metrics = StatusSelectDiffOpenMetrics::from_effects_and_rev(
                    effects, rev_before, rev_after,
                );

                let mut h = FxHasher::default();
                state.repos[0].diff_state.diff_state_rev.hash(&mut h);
                effects.len().hash(&mut h);
                for effect in effects.iter() {
                    std::mem::discriminant(effect).hash(&mut h);
                    match effect {
                        Effect::LoadSelectedDiff {
                            repo_id,
                            load_patch_diff,
                            load_file_text,
                            preview_text_side,
                            load_file_image,
                        } => {
                            repo_id.0.hash(&mut h);
                            load_patch_diff.hash(&mut h);
                            load_file_text.hash(&mut h);
                            let preview_text_side_key: u8 = match preview_text_side {
                                None => 0,
                                Some(gitcomet_core::domain::DiffPreviewTextSide::Old) => 1,
                                Some(gitcomet_core::domain::DiffPreviewTextSide::New) => 2,
                            };
                            preview_text_side_key.hash(&mut h);
                            load_file_image.hash(&mut h);
                        }
                        Effect::LoadDiff { repo_id, target }
                        | Effect::LoadDiffFile { repo_id, target }
                        | Effect::LoadDiffFileImage { repo_id, target } => {
                            repo_id.0.hash(&mut h);
                            hash_status_select_diff_target(target, &mut h);
                        }
                        _ => {}
                    }
                }
                metrics.load_selected_diff_effect_count.hash(&mut h);
                metrics.load_diff_effect_count.hash(&mut h);
                metrics.load_diff_file_effect_count.hash(&mut h);
                metrics.load_diff_file_image_effect_count.hash(&mut h);

                (h.finish(), metrics)
            },
        )
    }

    pub fn run(&self) -> (u64, StatusSelectDiffOpenMetrics) {
        let mut state = self.fresh_state();
        self.run_with_state(&mut state)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatusListMetrics {
    pub rows_requested: u64,
    pub rows_painted: u64,
    pub entries_total: u64,
    pub path_display_cache_hits: u64,
    pub path_display_cache_misses: u64,
    pub path_display_cache_clears: u64,
    pub max_path_depth: u64,
    pub prewarmed_entries: u64,
}

pub struct StatusListFixture {
    entries: Vec<FileStatus>,
    path_display_cache: path_display::PathDisplayCache,
}

impl StatusListFixture {
    pub fn unstaged_large(entries: usize) -> Self {
        Self {
            entries: build_synthetic_status_entries(entries, DiffArea::Unstaged),
            path_display_cache: path_display::PathDisplayCache::default(),
        }
    }

    pub fn staged_large(entries: usize) -> Self {
        Self {
            entries: build_synthetic_status_entries(entries, DiffArea::Staged),
            path_display_cache: path_display::PathDisplayCache::default(),
        }
    }

    pub fn mixed_depth(entries: usize) -> Self {
        Self {
            entries: build_synthetic_status_entries_mixed_depth(entries),
            path_display_cache: path_display::PathDisplayCache::default(),
        }
    }

    pub fn reset_runtime_state(&mut self) {
        self.path_display_cache.clear();
    }

    pub fn run_window_step(&mut self, start: usize, window: usize) -> u64 {
        let range = self.visible_range(start, window);
        self.hash_visible_range(range)
    }

    pub fn measure_window_step(&mut self, start: usize, window: usize) -> StatusListMetrics {
        self.measure_window_step_with_prewarm(start, window, 0)
    }

    pub fn prewarm_cache(&mut self, entries: usize) {
        let count = entries.min(self.entries.len());
        for entry in self.entries.iter().take(count) {
            let _ = path_display::cached_path_display(&mut self.path_display_cache, &entry.path);
        }
    }

    pub fn measure_window_step_with_prewarm(
        &mut self,
        start: usize,
        window: usize,
        prewarm_entries: usize,
    ) -> StatusListMetrics {
        let range = self.visible_range(start, window);
        self.reset_runtime_state();
        path_display::bench_reset();
        self.prewarm_cache(prewarm_entries);
        path_display::bench_reset();
        let _ = self.hash_visible_range(range.clone());
        let counters = path_display::bench_snapshot();
        path_display::bench_reset();

        StatusListMetrics {
            rows_requested: range.len() as u64,
            rows_painted: range.len() as u64,
            entries_total: self.entries.len() as u64,
            path_display_cache_hits: counters.cache_hits,
            path_display_cache_misses: counters.cache_misses,
            path_display_cache_clears: counters.cache_clears,
            max_path_depth: self.max_path_depth_for_range(range.clone()) as u64,
            prewarmed_entries: prewarm_entries.min(self.entries.len()) as u64,
        }
    }

    fn visible_range(&self, start: usize, window: usize) -> Range<usize> {
        let window = window.max(1).min(self.entries.len());
        if window == 0 {
            return 0..0;
        }

        let max_start = self.entries.len().saturating_sub(window);
        let start = if max_start == 0 {
            0
        } else {
            start % (max_start + 1)
        };
        start..start + window
    }

    fn hash_visible_range(&mut self, range: Range<usize>) -> u64 {
        let mut h = FxHasher::default();
        range.start.hash(&mut h);
        range.end.hash(&mut h);

        for (row_ix, entry) in self.entries[range].iter().enumerate() {
            let path_display =
                path_display::cached_path_display(&mut self.path_display_cache, &entry.path);
            hash_status_row_label(row_ix, entry.kind, &path_display, &mut h);
        }

        self.path_display_cache.len().hash(&mut h);
        h.finish()
    }

    fn max_path_depth_for_range(&self, range: Range<usize>) -> usize {
        self.entries[range]
            .iter()
            .map(|entry| entry.path.components().count())
            .max()
            .unwrap_or_default()
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusTruncationScenario {
    PathAligned,
    Middle,
    End,
    Focus,
}

impl StatusTruncationScenario {
    fn profile(self) -> TextTruncationProfile {
        match self {
            Self::PathAligned => TextTruncationProfile::Path,
            Self::Middle => TextTruncationProfile::Middle,
            Self::End | Self::Focus => TextTruncationProfile::End,
        }
    }

    fn layout_passes(self) -> u64 {
        match self {
            Self::PathAligned => 2,
            Self::Middle | Self::End | Self::Focus => 1,
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatusTruncationRenderMetrics {
    pub sections: u64,
    pub entries_total: u64,
    pub rows_visible: u64,
    pub rows_shaped: u64,
    pub layout_passes: u64,
    pub path_profile_shapes: u64,
    pub middle_profile_shapes: u64,
    pub end_profile_shapes: u64,
    pub focus_shapes: u64,
    pub truncated_shapes: u64,
    pub path_alignment_notifications: u64,
    pub candidate_measurements: u64,
    pub path_display_cache_hits: u64,
    pub path_display_cache_misses: u64,
    pub path_display_cache_clears: u64,
    pub max_path_depth: u64,
}

struct StatusTruncationSection {
    section: StatusSection,
    entries: Vec<FileStatus>,
}

pub struct StatusTruncationRenderFixture {
    cx: gpui::TestAppContext,
    window: gpui::WindowHandle<gpui::Empty>,
    sections: Vec<StatusTruncationSection>,
    path_display_cache: path_display::PathDisplayCache,
    max_width: Pixels,
    scenario: StatusTruncationScenario,
}

impl StatusTruncationRenderFixture {
    pub fn long_untracked_unstaged(
        entries_per_section: usize,
        scenario: StatusTruncationScenario,
        max_width_px: f32,
    ) -> Self {
        let entries_per_section = entries_per_section.max(1);
        let mut cx = gpui::TestAppContext::single();
        cx.skip_drawing();
        let window = cx.add_window(|_window, _cx| gpui::Empty);

        Self {
            cx,
            window,
            sections: vec![
                StatusTruncationSection {
                    section: StatusSection::Untracked,
                    entries: build_long_status_truncation_entries(
                        entries_per_section,
                        StatusSection::Untracked,
                    ),
                },
                StatusTruncationSection {
                    section: StatusSection::Unstaged,
                    entries: build_long_status_truncation_entries(
                        entries_per_section,
                        StatusSection::Unstaged,
                    ),
                },
            ],
            path_display_cache: path_display::PathDisplayCache::default(),
            max_width: px(max_width_px.max(1.0)),
            scenario,
        }
    }

    pub fn reset_runtime_state(&mut self) {
        self.path_display_cache.clear();
        clear_truncated_layout_cache_for_benchmark();
        reset_measure_candidate_calls_for_benchmark();
    }

    pub fn run(&mut self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&mut self) -> (u64, StatusTruncationRenderMetrics) {
        self.reset_runtime_state();
        path_display::bench_reset();

        let sections = &self.sections;
        let path_display_cache = &mut self.path_display_cache;
        let max_width = self.max_width;
        let scenario = self.scenario;
        let window = self.window;
        let layout_passes = scenario.layout_passes();
        let rows_visible = sections
            .iter()
            .map(|section| section.entries.len())
            .sum::<usize>();

        let (hash, mut metrics) = self
            .cx
            .update_window(window.into(), |_, window, app| {
                let mut hasher = FxHasher::default();
                let mut metrics = StatusTruncationRenderMetrics {
                    sections: sections.len() as u64,
                    entries_total: rows_visible as u64,
                    rows_visible: rows_visible as u64,
                    layout_passes,
                    max_path_depth: max_path_depth_for_sections(sections) as u64,
                    ..StatusTruncationRenderMetrics::default()
                };

                let style = window.text_style();
                let path_alignment_groups = if scenario == StatusTruncationScenario::PathAligned {
                    sections
                        .iter()
                        .map(|section| {
                            (
                                section_visible_signature(section),
                                PathTruncationAlignmentGroup::default(),
                            )
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };

                for _ in 0..layout_passes {
                    for (signature, group) in &path_alignment_groups {
                        let _ = group.visible_rows(*signature);
                    }

                    for (section_ix, section) in sections.iter().enumerate() {
                        status_section_hash_key(section.section).hash(&mut hasher);
                        let path_alignment_group = path_alignment_groups
                            .get(section_ix)
                            .map(|(_, group)| group);

                        for (row_ix, entry) in section.entries.iter().enumerate() {
                            row_ix.hash(&mut hasher);
                            status_row_kind_key(entry.kind).hash(&mut hasher);
                            let path_display =
                                path_display::cached_path_display(path_display_cache, &entry.path);
                            hash_shared_string_identity(&path_display, &mut hasher);

                            let shape_context = StatusTruncationShapeContext {
                                scenario,
                                max_width,
                                path_alignment_group,
                            };
                            shape_status_truncation_label(
                                window,
                                app,
                                &style,
                                &path_display,
                                &shape_context,
                                &mut metrics,
                                &mut hasher,
                            );
                        }
                    }
                }

                (hasher.finish(), metrics)
            })
            .expect("status truncation benchmark window should remain open");

        let path_display_counters = path_display::bench_snapshot();
        path_display::bench_reset();
        metrics.candidate_measurements = measure_candidate_calls_for_benchmark() as u64;
        metrics.path_display_cache_hits = path_display_counters.cache_hits;
        metrics.path_display_cache_misses = path_display_counters.cache_misses;
        metrics.path_display_cache_clears = path_display_counters.cache_clears;

        (hash, metrics)
    }
}

struct StatusTruncationShapeContext<'a> {
    scenario: StatusTruncationScenario,
    max_width: Pixels,
    path_alignment_group: Option<&'a PathTruncationAlignmentGroup>,
}

fn shape_status_truncation_label(
    window: &mut Window,
    app: &mut App,
    base_style: &TextStyle,
    path_display: &SharedString,
    shape_context: &StatusTruncationShapeContext<'_>,
    metrics: &mut StatusTruncationRenderMetrics,
    hasher: &mut FxHasher,
) {
    let profile = shape_context.scenario.profile();
    let focus_range = (shape_context.scenario == StatusTruncationScenario::Focus)
        .then(|| focus_range_for_status_path(path_display.as_ref()))
        .flatten();
    let style_key = (profile == TextTruncationProfile::Path)
        .then(|| path_alignment_style_key(base_style, window.rem_size()));
    let path_ellipsis_anchor = shape_context.path_alignment_group.and_then(|group| {
        style_key.and_then(|key| group.path_anchor_for_layout(Some(shape_context.max_width), key))
    });

    let line = shape_truncated_line_cached_with_path_anchor(
        window,
        app,
        base_style,
        path_display,
        Some(shape_context.max_width),
        profile,
        &[],
        focus_range.clone(),
        path_ellipsis_anchor,
    );

    if profile == TextTruncationProfile::Path
        && path_ellipsis_anchor.is_none()
        && let Some(group) = shape_context.path_alignment_group
        && let Some(style_key) = style_key
        && let Some(ellipsis_x) = truncated_line_ellipsis_x(&line)
        && group.report_natural_ellipsis(Some(shape_context.max_width), style_key, ellipsis_x)
    {
        metrics.path_alignment_notifications =
            metrics.path_alignment_notifications.saturating_add(1);
    }

    metrics.rows_shaped = metrics.rows_shaped.saturating_add(1);
    match profile {
        TextTruncationProfile::Path => {
            metrics.path_profile_shapes = metrics.path_profile_shapes.saturating_add(1);
        }
        TextTruncationProfile::Middle => {
            metrics.middle_profile_shapes = metrics.middle_profile_shapes.saturating_add(1);
        }
        TextTruncationProfile::End => {
            metrics.end_profile_shapes = metrics.end_profile_shapes.saturating_add(1);
        }
    }
    if focus_range.is_some() {
        metrics.focus_shapes = metrics.focus_shapes.saturating_add(1);
    }
    if line.truncated {
        metrics.truncated_shapes = metrics.truncated_shapes.saturating_add(1);
    }

    line.truncated.hash(hasher);
    f32::from(line.shaped_line.width).to_bits().hash(hasher);
    f32::from(line.line_height).to_bits().hash(hasher);
    truncated_line_ellipsis_x(&line)
        .map(|x| f32::from(x).to_bits())
        .unwrap_or_default()
        .hash(hasher);
    line.projection
        .source_to_display_offset(path_display.len() / 2)
        .hash(hasher);
    line.projection
        .source_to_display_offset(path_display.len())
        .hash(hasher);
}

fn build_long_status_truncation_entries(entries: usize, section: StatusSection) -> Vec<FileStatus> {
    let mut items = Vec::with_capacity(entries);
    for ix in 0..entries {
        let mut path = std::path::PathBuf::from("workspace");
        path.push(match section {
            StatusSection::Untracked => "untracked-files",
            StatusSection::Unstaged | StatusSection::CombinedUnstaged => "unstaged-files",
            StatusSection::Staged => "staged-files",
        });
        for depth_ix in 0..10 {
            path.push(format!(
                "very_long_component_{depth_ix:02}_group_{:03}_with_descriptive_context",
                (ix.wrapping_mul(17).wrapping_add(depth_ix)) % 997
            ));
        }
        path.push(format!(
            "file_{ix:05}_with_an_excessively_descriptive_name_and_tail_{:04}.rs",
            ix.wrapping_mul(29) % 10_000
        ));

        let kind = match section {
            StatusSection::Untracked => FileStatusKind::Untracked,
            StatusSection::Unstaged | StatusSection::CombinedUnstaged => {
                if ix % 7 == 0 {
                    FileStatusKind::Added
                } else {
                    FileStatusKind::Modified
                }
            }
            StatusSection::Staged => FileStatusKind::Added,
        };

        items.push(FileStatus {
            path,
            kind,
            conflict: None,
        });
    }
    items
}

fn focus_range_for_status_path(path_display: &str) -> Option<Range<usize>> {
    let filename_start = path_display
        .rfind('/')
        .or_else(|| path_display.rfind('\\'))
        .map_or(0, |ix| ix + 1);
    let focus_start = path_display[filename_start..]
        .find("descriptive_name")
        .map_or(filename_start, |ix| filename_start + ix);
    let focus_end = focus_start.saturating_add(24).min(path_display.len());
    (focus_start < focus_end).then_some(focus_start..focus_end)
}

fn section_visible_signature(section: &StatusTruncationSection) -> u64 {
    path_alignment_visible_signature(&(
        status_section_hash_key(section.section),
        section.entries.len(),
    ))
}

fn status_section_hash_key(section: StatusSection) -> u8 {
    match section {
        StatusSection::CombinedUnstaged => 0,
        StatusSection::Untracked => 1,
        StatusSection::Unstaged => 2,
        StatusSection::Staged => 3,
    }
}

fn max_path_depth_for_sections(sections: &[StatusTruncationSection]) -> usize {
    sections
        .iter()
        .flat_map(|section| &section.entries)
        .map(|entry| entry.path.components().count())
        .max()
        .unwrap_or_default()
}

fn status_row_kind_key(kind: FileStatusKind) -> u8 {
    match kind {
        FileStatusKind::Untracked => 0,
        FileStatusKind::Modified => 1,
        FileStatusKind::Added => 2,
        FileStatusKind::Deleted => 3,
        FileStatusKind::Renamed => 4,
        FileStatusKind::Conflicted => 5,
    }
}

fn hash_status_row_label(
    row_ix: usize,
    kind: FileStatusKind,
    path_label: &SharedString,
    hasher: &mut FxHasher,
) {
    // Production status rows reuse the cached SharedString label directly.
    // They do not rescan both the raw PathBuf text and the formatted label.
    status_row_kind_key(kind).hash(hasher);
    row_ix.hash(hasher);
    hash_shared_string_identity(path_label, hasher);
}
