use super::*;

pub struct LargeFileDiffScrollFixture {
    lines: Vec<String>,
    line_bytes: Vec<usize>,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    prepared_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    theme: AppTheme,
    highlight_palette: super::diff_text::SyntaxHighlightPalette,
    row_fingerprints: LargeFileDiffScrollRowFingerprints,
}

enum LargeFileDiffScrollRowFingerprints {
    Warm(Vec<u64>),
    Lazy(Vec<Cell<Option<u64>>>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LargeFileDiffScrollMetrics {
    pub total_lines: u64,
    pub window_size: u64,
    pub start_line: u64,
    pub visible_text_bytes: u64,
    pub min_line_bytes: u64,
    pub language_detected: u64,
    pub syntax_mode_auto: u64,
}

impl LargeFileDiffScrollFixture {
    pub fn new(lines: usize) -> Self {
        Self::new_with_line_bytes(lines, 96)
    }

    pub fn new_with_line_bytes(lines: usize, line_bytes: usize) -> Self {
        let theme = AppTheme::gitcomet_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");
        let lines = build_synthetic_source_lines(lines, line_bytes);
        let line_count = lines.len();
        let line_bytes = lines.iter().map(String::len).collect::<Vec<_>>();
        let prepared_document = language.and_then(|language| {
            let text: SharedString = lines.join("\n").into();
            let line_starts: Arc<[usize]> = Arc::from(line_starts_for_text(text.as_ref()));
            let document = prepare_bench_diff_syntax_document_from_shared(
                language,
                DiffSyntaxBudget::default(),
                text.clone(),
                Arc::clone(&line_starts),
                None,
            )?;
            prewarm_bench_prepared_diff_syntax_document(
                theme,
                text.as_ref(),
                line_starts.as_ref(),
                document,
                language,
                lines.len(),
            );
            Some(document)
        });
        let mut fixture = Self {
            line_bytes,
            lines,
            language,
            prepared_document,
            theme,
            highlight_palette: super::diff_text::syntax_highlight_palette(theme),
            row_fingerprints: LargeFileDiffScrollRowFingerprints::Lazy(vec![
                Cell::new(None);
                line_count
            ]),
        };
        fixture.prewarm_row_fingerprints();
        fixture
    }

    fn prewarm_row_fingerprints(&mut self) {
        let mut warm = Vec::with_capacity(self.lines.len());
        let mut lazy: Option<Vec<Cell<Option<u64>>>> = None;

        for line_ix in 0..self.lines.len() {
            let (styled, pending) = self.build_styled_line(line_ix);
            let fingerprint = large_file_diff_scroll_row_fingerprint(line_ix, &styled);
            if let Some(cache) = lazy.as_mut() {
                cache.push(Cell::new((!pending).then_some(fingerprint)));
                continue;
            }

            if pending {
                let mut cache = warm
                    .drain(..)
                    .map(|cached_fingerprint| Cell::new(Some(cached_fingerprint)))
                    .collect::<Vec<_>>();
                cache.push(Cell::new(None));
                lazy = Some(cache);
                continue;
            }

            warm.push(fingerprint);
        }

        self.row_fingerprints = if let Some(cache) = lazy {
            LargeFileDiffScrollRowFingerprints::Lazy(cache)
        } else {
            LargeFileDiffScrollRowFingerprints::Warm(warm)
        };
    }

    fn build_styled_line(&self, line_ix: usize) -> (CachedDiffStyledText, bool) {
        let line = self
            .lines
            .get(line_ix)
            .map(String::as_str)
            .unwrap_or_default();
        if let Some(document) = self.prepared_document {
            return super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
                self.theme,
                &self.highlight_palette,
                super::diff_text::PreparedDiffTextBuildRequest {
                    build: super::diff_text::DiffTextBuildRequest {
                        text: line,
                        word_ranges: &[],
                        query: "",
                        syntax: super::diff_text::DiffSyntaxConfig {
                            language: self.language,
                            mode: DiffSyntaxMode::Auto,
                        },
                        word_color: None,
                    },
                    prepared_line: super::diff_text::PreparedDiffSyntaxLine {
                        document: Some(document),
                        line_ix,
                    },
                },
            )
            .into_parts();
        }

        (
            super::diff_text::build_cached_diff_styled_text_with_palette(
                self.theme,
                &self.highlight_palette,
                super::diff_text::DiffTextBuildRequest {
                    text: line,
                    word_ranges: &[],
                    query: "",
                    syntax: super::diff_text::DiffSyntaxConfig {
                        language: self.language,
                        mode: DiffSyntaxMode::Auto,
                    },
                    word_color: None,
                },
            ),
            false,
        )
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        let (actual_start, end) = self.visible_range(start, window);
        self.hash_visible_range(actual_start, end)
    }

    pub fn run_scroll_step_with_metrics(
        &self,
        start: usize,
        window: usize,
    ) -> (u64, LargeFileDiffScrollMetrics) {
        let (actual_start, end) = self.visible_range(start, window);
        let hash = self.hash_visible_range(actual_start, end);
        let visible_line_bytes = &self.line_bytes[actual_start..end];
        let visible_text_bytes = visible_line_bytes.iter().copied().sum::<usize>();
        (
            hash,
            LargeFileDiffScrollMetrics {
                total_lines: bench_counter_u64(self.lines.len()),
                window_size: bench_counter_u64(visible_line_bytes.len()),
                start_line: bench_counter_u64(actual_start),
                visible_text_bytes: bench_counter_u64(visible_text_bytes),
                min_line_bytes: bench_counter_u64(if visible_line_bytes.is_empty() {
                    0
                } else {
                    visible_line_bytes.iter().copied().min().unwrap_or_default()
                }),
                language_detected: u64::from(self.language.is_some()),
                syntax_mode_auto: 1,
            },
        )
    }

    fn visible_range(&self, start: usize, window: usize) -> (usize, usize) {
        if self.lines.is_empty() || window == 0 {
            return (0, 0);
        }

        let actual_start = start % self.lines.len();
        let end = (actual_start + window).min(self.lines.len());
        (actual_start, end)
    }

    fn hash_visible_range(&self, actual_start: usize, end: usize) -> u64 {
        match &self.row_fingerprints {
            LargeFileDiffScrollRowFingerprints::Warm(fingerprints) => {
                hash_row_fingerprint_slice(&fingerprints[actual_start..end])
            }
            LargeFileDiffScrollRowFingerprints::Lazy(cache) => {
                let mut hasher = FxHasher::default();
                end.saturating_sub(actual_start).hash(&mut hasher);

                for (line_ix, cache_slot) in cache.iter().enumerate().take(end).skip(actual_start) {
                    let fingerprint = if let Some(fingerprint) = cache_slot.get() {
                        fingerprint
                    } else {
                        let (styled, pending) = self.build_styled_line(line_ix);
                        let fingerprint = large_file_diff_scroll_row_fingerprint(line_ix, &styled);
                        if !pending {
                            cache_slot.set(Some(fingerprint));
                        }
                        fingerprint
                    };
                    fingerprint.hash(&mut hasher);
                }

                hasher.finish()
            }
        }
    }
}

fn large_file_diff_scroll_row_fingerprint(line_ix: usize, styled: &CachedDiffStyledText) -> u64 {
    let mut hasher = FxHasher::default();
    line_ix.hash(&mut hasher);
    styled.text_hash.hash(&mut hasher);
    styled.highlights_hash.hash(&mut hasher);
    hasher.finish()
}

#[inline]
fn hash_row_fingerprint_slice(fingerprints: &[u64]) -> u64 {
    let mut hasher = FxHasher::default();
    fingerprints.len().hash(&mut hasher);
    for &fingerprint in fingerprints {
        fingerprint.hash(&mut hasher);
    }
    hasher.finish()
}

const BENCH_PREPARED_DIFF_SYNTAX_CHUNK_ROWS: usize = 64;
const BENCH_PREPARED_DIFF_SYNTAX_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

fn prewarm_bench_prepared_diff_syntax_document(
    theme: AppTheme,
    text: &str,
    line_starts: &[usize],
    document: super::diff_text::PreparedDiffSyntaxDocument,
    language: DiffSyntaxLanguage,
    line_count: usize,
) {
    if text.is_empty() || line_count == 0 {
        return;
    }

    // Sustained diff scrolling should stay on the warmed prepared-document path
    // instead of timing background chunk scheduling and heuristic fallbacks.
    for chunk_start in (0..line_count).step_by(BENCH_PREPARED_DIFF_SYNTAX_CHUNK_ROWS) {
        let _ = super::diff_text::request_syntax_highlights_for_prepared_document_line_range(
            theme,
            text,
            line_starts,
            document,
            language,
            chunk_start..chunk_start.saturating_add(1).min(line_count),
        );
    }

    let started = Instant::now();
    while super::diff_text::has_pending_prepared_diff_syntax_chunk_builds_for_document(document) {
        if super::diff_text::drain_completed_prepared_diff_syntax_chunk_builds_for_document(
            document,
        ) == 0
        {
            if started.elapsed() >= BENCH_PREPARED_DIFF_SYNTAX_DRAIN_TIMEOUT {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Synthetic visible-window scrolling fixture for the history list.
///
/// This keeps the expensive history-cache build outside the measured loop and
/// approximates the per-frame work of painting visible commit rows plus graph
/// lane state during sustained scrolling.
pub struct HistoryListScrollFixture {
    row_fingerprints: Vec<u64>,
}

impl HistoryListScrollFixture {
    pub fn new(commits: usize, local_branches: usize, remote_branches: usize) -> Self {
        let commits = build_synthetic_commits_with_merge_stride(commits.max(1), 11, 5);
        let (branches, remote_branches) =
            build_branches_targeting_commits(&commits, local_branches, remote_branches);
        let graph_rows = history_graph::compute_graph(
            &commits,
            AppTheme::gitcomet_dark(),
            history_graph_heads_from_branches(&branches, &remote_branches)
                .iter()
                .copied(),
            None,
        )
        .into_iter()
        .map(Arc::new)
        .collect::<Vec<_>>();
        let row_fingerprints = commits
            .iter()
            .zip(graph_rows.iter())
            .map(|(commit, graph_row)| history_scroll_row_fingerprint(commit, graph_row))
            .collect();

        Self { row_fingerprints }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        let range = self.visible_range(start, window);
        let mut h = FxHasher::default();
        range.len().hash(&mut h);

        for row_fingerprint in &self.row_fingerprints[range] {
            row_fingerprint.hash(&mut h);
        }

        h.finish()
    }

    fn visible_range(&self, start: usize, window: usize) -> Range<usize> {
        let window = window.max(1).min(self.row_fingerprints.len());
        let max_start = self.row_fingerprints.len().saturating_sub(window);
        let start = start.min(max_start);
        start..start + window
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(crate) fn total_rows(&self) -> usize {
        self.row_fingerprints.len()
    }
}

fn history_scroll_row_fingerprint(commit: &Commit, graph_row: &history_graph::GraphRow) -> u64 {
    let mut hasher = FxHasher::default();
    commit.id.as_ref().hash(&mut hasher);
    commit.summary.hash(&mut hasher);
    commit.author.hash(&mut hasher);
    commit.parent_ids.len().hash(&mut hasher);
    (
        graph_row.lanes_now.len(),
        graph_row.lanes_next.len(),
        graph_row.joins_in.len(),
        graph_row.edges_out.len(),
        graph_row.is_merge,
    )
        .hash(&mut hasher);
    hasher.finish()
}

enum KeyboardArrowScrollScenario {
    History(HistoryListScrollFixture),
    Diff(Box<LargeFileDiffScrollFixture>),
}

pub struct KeyboardArrowScrollFixture {
    scenario: KeyboardArrowScrollScenario,
    total_rows: usize,
    window_rows: usize,
    scroll_step_rows: usize,
    repeat_events: usize,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyboardArrowScrollMetrics {
    pub total_rows: u64,
    pub window_rows: u64,
    pub scroll_step_rows: u64,
    pub repeat_events: u64,
    pub rows_requested_total: u64,
    pub unique_windows_visited: u64,
    pub wrap_count: u64,
    pub final_start_row: u64,
}

impl KeyboardArrowScrollFixture {
    pub fn history(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        window_rows: usize,
        scroll_step_rows: usize,
        repeat_events: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let fixture = HistoryListScrollFixture::new(commits, local_branches, remote_branches);
        let total_rows = fixture.total_rows();
        Self {
            scenario: KeyboardArrowScrollScenario::History(fixture),
            total_rows,
            window_rows: window_rows.max(1),
            scroll_step_rows: scroll_step_rows.max(1),
            repeat_events: repeat_events.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn diff(
        lines: usize,
        line_bytes: usize,
        window_rows: usize,
        scroll_step_rows: usize,
        repeat_events: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let total_rows = lines.max(1);
        Self {
            scenario: KeyboardArrowScrollScenario::Diff(Box::new(
                LargeFileDiffScrollFixture::new_with_line_bytes(total_rows, line_bytes.max(1)),
            )),
            total_rows,
            window_rows: window_rows.max(1),
            scroll_step_rows: scroll_step_rows.max(1),
            repeat_events: repeat_events.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        KeyboardArrowScrollMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_step(&self, start: usize, window_rows: usize) -> u64 {
        match &self.scenario {
            KeyboardArrowScrollScenario::History(fixture) => {
                fixture.run_scroll_step(start, window_rows)
            }
            KeyboardArrowScrollScenario::Diff(fixture) => {
                fixture.run_scroll_step(start, window_rows)
            }
        }
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, KeyboardArrowScrollMetrics) {
        let window_rows = self.window_rows.max(1).min(self.total_rows.max(1));
        let scroll_step_rows = self.scroll_step_rows.max(1);
        let repeat_events = self.repeat_events.max(1);
        let max_start = self.total_rows.saturating_sub(window_rows);
        let mut hash = 0u64;
        let mut start = 0usize;
        let mut wrap_count = 0u64;

        for _ in 0..repeat_events {
            if let Some(capture) = capture.as_deref_mut() {
                let frame_started = std::time::Instant::now();
                hash ^= self.run_step(start, window_rows);
                capture.record_frame(frame_started.elapsed());
            } else {
                hash ^= self.run_step(start, window_rows);
            }

            if max_start > 0 {
                let next = start.saturating_add(scroll_step_rows);
                if next > max_start {
                    wrap_count = wrap_count.saturating_add(1);
                    start = next % (max_start + 1);
                } else {
                    start = next;
                }
            }
        }

        (
            hash,
            KeyboardArrowScrollMetrics {
                total_rows: u64::try_from(self.total_rows).unwrap_or(u64::MAX),
                window_rows: u64::try_from(window_rows).unwrap_or(u64::MAX),
                scroll_step_rows: u64::try_from(scroll_step_rows).unwrap_or(u64::MAX),
                repeat_events: u64::try_from(repeat_events).unwrap_or(u64::MAX),
                rows_requested_total: u64::try_from(window_rows)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(u64::try_from(repeat_events).unwrap_or(u64::MAX)),
                unique_windows_visited: keyboard_scroll_unique_window_count(
                    max_start,
                    scroll_step_rows,
                    repeat_events,
                ),
                wrap_count,
                final_start_row: u64::try_from(start).unwrap_or(u64::MAX),
            },
        )
    }
}

fn keyboard_scroll_unique_window_count(
    max_start: usize,
    scroll_step_rows: usize,
    repeat_events: usize,
) -> u64 {
    if repeat_events == 0 {
        return 0;
    }
    if max_start == 0 {
        return 1;
    }

    let cycle_len = max_start
        .saturating_add(1)
        .checked_div(greatest_common_divisor(
            max_start.saturating_add(1),
            scroll_step_rows,
        ))
        .unwrap_or(1);

    u64::try_from(repeat_events.min(cycle_len)).unwrap_or(u64::MAX)
}

fn greatest_common_divisor(mut left: usize, mut right: usize) -> usize {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.max(1)
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum KeyboardFocusNodeKind {
    RepoTab,
    TabBarSpacer,
    HistoryPanel,
    SidebarResizeHandle,
    DiffPanel,
    DetailsResizeHandle,
    CommitMessageInput,
    CommitShaInput,
    CommitDateInput,
    CommitParentInput,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Debug)]
struct KeyboardFocusNode {
    kind: KeyboardFocusNodeKind,
    label_len: usize,
    focusable: bool,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
struct KeyboardFocusTraversal {
    hash: u64,
    prefix_max_scan_len: usize,
}

/// Fixture for `keyboard/tab_focus_cycle_all_panes`.
///
/// Models the tab-order traversal across the major focusable chrome in a
/// typical open-repo view: repo tabs, the history panel, the diff panel,
/// and the commit-details text inputs. Two structural nodes (the sidebar and
/// details split handles) are present but skipped because they are not tab
/// stops, so the benchmark measures both focus-target switching and the scan
/// needed to find the next focusable node.
pub struct KeyboardTabFocusCycleFixture {
    focus_traversal: Box<[KeyboardFocusTraversal]>,
    repo_tab_count: usize,
    detail_input_count: usize,
    cycle_events: usize,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyboardTabFocusCycleMetrics {
    pub focus_target_count: u64,
    pub repo_tab_count: u64,
    pub detail_input_count: u64,
    pub cycle_events: u64,
    pub unique_targets_visited: u64,
    pub wrap_count: u64,
    pub max_scan_len: u64,
    pub final_target_index: u64,
}

impl KeyboardTabFocusCycleFixture {
    pub fn all_panes(repo_tab_count: usize, cycle_events: usize, frame_budget_ns: u64) -> Self {
        let repo_tab_count = repo_tab_count.max(1);
        let detail_input_count = 4usize;
        let mut nodes = Vec::with_capacity(repo_tab_count + detail_input_count + 4);

        for ix in 0..repo_tab_count {
            nodes.push(KeyboardFocusNode {
                kind: KeyboardFocusNodeKind::RepoTab,
                label_len: format!("repo-tab-{ix:02}").len(),
                focusable: true,
            });
        }

        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::TabBarSpacer,
            label_len: 0,
            focusable: false,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::HistoryPanel,
            label_len: "History".len(),
            focusable: true,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::SidebarResizeHandle,
            label_len: 0,
            focusable: false,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::DiffPanel,
            label_len: "Diff".len(),
            focusable: true,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::DetailsResizeHandle,
            label_len: 0,
            focusable: false,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::CommitMessageInput,
            label_len: "Commit message".len(),
            focusable: true,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::CommitShaInput,
            label_len: "Commit SHA".len(),
            focusable: true,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::CommitDateInput,
            label_len: "Commit date".len(),
            focusable: true,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::CommitParentInput,
            label_len: "Parent commit".len(),
            focusable: true,
        });

        let focusable_node_indices = nodes
            .iter()
            .enumerate()
            .filter_map(|(ix, node)| node.focusable.then_some(ix))
            .collect::<Vec<_>>();

        let mut focus_traversal = Vec::with_capacity(focusable_node_indices.len());
        let mut prefix_max_scan_len = 0usize;

        for (focus_ix, &node_ix) in focusable_node_indices.iter().enumerate() {
            let node = &nodes[node_ix];
            let next_focus_ix = (focus_ix + 1) % focusable_node_indices.len();
            let next_node_ix = focusable_node_indices[next_focus_ix];
            let scan_len = keyboard_focus_scan_len(node_ix, next_node_ix, nodes.len());
            prefix_max_scan_len = prefix_max_scan_len.max(scan_len);
            focus_traversal.push(KeyboardFocusTraversal {
                hash: keyboard_focus_node_hash(focus_ix, node_ix, node),
                prefix_max_scan_len,
            });
        }

        Self {
            focus_traversal: focus_traversal.into_boxed_slice(),
            repo_tab_count,
            detail_input_count,
            cycle_events: cycle_events.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        KeyboardTabFocusCycleMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::with_expected_frames(
            self.frame_budget_ns,
            self.cycle_events,
        );
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, KeyboardTabFocusCycleMetrics) {
        let focus_target_count = self.focus_traversal.len().max(1);
        let mut hash = 0u64;
        let mut current_focus_ix = 0usize;

        for _ in 0..self.cycle_events {
            let traversal = self.focus_traversal[current_focus_ix];

            if let Some(capture) = capture.as_deref_mut() {
                let frame_started = std::time::Instant::now();
                hash ^= traversal.hash;
                capture.record_frame(frame_started.elapsed());
            } else {
                hash ^= traversal.hash;
            }

            current_focus_ix += 1;
            if current_focus_ix == focus_target_count {
                current_focus_ix = 0;
            }
        }

        (
            hash,
            KeyboardTabFocusCycleMetrics {
                focus_target_count: u64::try_from(focus_target_count).unwrap_or(u64::MAX),
                repo_tab_count: u64::try_from(self.repo_tab_count).unwrap_or(u64::MAX),
                detail_input_count: u64::try_from(self.detail_input_count).unwrap_or(u64::MAX),
                cycle_events: u64::try_from(self.cycle_events).unwrap_or(u64::MAX),
                unique_targets_visited: keyboard_focus_unique_target_count(
                    focus_target_count,
                    self.cycle_events,
                ),
                wrap_count: keyboard_focus_wrap_count(focus_target_count, self.cycle_events),
                max_scan_len: keyboard_focus_max_scan_len(&self.focus_traversal, self.cycle_events),
                final_target_index: u64::try_from(current_focus_ix).unwrap_or(u64::MAX),
            },
        )
    }
}

fn keyboard_focus_node_hash(focus_ix: usize, node_ix: usize, node: &KeyboardFocusNode) -> u64 {
    let mut hasher = FxHasher::default();
    focus_ix.hash(&mut hasher);
    node_ix.hash(&mut hasher);
    std::mem::discriminant(&node.kind).hash(&mut hasher);
    node.label_len.hash(&mut hasher);
    hasher.finish()
}

fn keyboard_focus_scan_len(
    current_node_ix: usize,
    next_node_ix: usize,
    node_count: usize,
) -> usize {
    if next_node_ix > current_node_ix {
        next_node_ix - current_node_ix
    } else {
        node_count - current_node_ix + next_node_ix
    }
    .max(1)
}

fn keyboard_focus_unique_target_count(focus_target_count: usize, cycle_events: usize) -> u64 {
    u64::try_from(cycle_events.min(focus_target_count)).unwrap_or(u64::MAX)
}

fn keyboard_focus_wrap_count(focus_target_count: usize, cycle_events: usize) -> u64 {
    if focus_target_count == 0 {
        0
    } else {
        u64::try_from(cycle_events / focus_target_count).unwrap_or(u64::MAX)
    }
}

fn keyboard_focus_max_scan_len(
    focus_traversal: &[KeyboardFocusTraversal],
    cycle_events: usize,
) -> u64 {
    if cycle_events == 0 || focus_traversal.is_empty() {
        return 0;
    }

    let max_scan_len = if cycle_events >= focus_traversal.len() {
        focus_traversal
            .last()
            .map(|traversal| traversal.prefix_max_scan_len)
            .unwrap_or(0)
    } else {
        focus_traversal[cycle_events - 1].prefix_max_scan_len
    };

    u64::try_from(max_scan_len).unwrap_or(u64::MAX)
}

/// Fixture for `keyboard/stage_unstage_toggle_rapid`.
///
/// Uses a partially staged synthetic status list so the same path corpus exists
/// in both the unstaged and staged areas. Each keyboard event dispatches either
/// `StagePath` or `UnstagePath`, immediately followed by `SelectDiff` for the
/// same path in the opposite area to model rapid toggling between the two
/// keyboard actions while keeping the diff view active.
pub struct KeyboardStageUnstageToggleFixture {
    baseline: AppState,
    paths: Vec<std::path::PathBuf>,
    toggle_events: usize,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyboardStageUnstageToggleMetrics {
    pub path_count: u64,
    pub toggle_events: u64,
    pub effect_count: u64,
    pub stage_effect_count: u64,
    pub unstage_effect_count: u64,
    pub select_diff_effect_count: u64,
    pub ops_rev_delta: u64,
    pub diff_state_rev_delta: u64,
    pub area_flip_count: u64,
    pub path_wrap_count: u64,
}

impl KeyboardStageUnstageToggleFixture {
    pub fn rapid_toggle(path_count: usize, toggle_events: usize, frame_budget_ns: u64) -> Self {
        let path_count = path_count.max(1);
        let entries = build_synthetic_partially_staged_entries(path_count);
        let paths = entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        seed_repo_status_entries(&mut repo, entries.clone(), entries);
        repo.open = Loadable::Ready(());
        repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
            path: paths[0].clone(),
            area: DiffArea::Unstaged,
        });
        repo.diff_state.diff_state_rev = 1;

        Self {
            baseline: bench_app_state(vec![repo], Some(RepoId(1))),
            paths,
            toggle_events: toggle_events.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        KeyboardStageUnstageToggleMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::with_expected_frames(
            self.frame_budget_ns,
            self.toggle_events,
        );
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn fresh_state(&self) -> AppState {
        self.baseline.clone()
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, KeyboardStageUnstageToggleMetrics) {
        let mut state = self.fresh_state();
        let repo = &state.repos[0];
        let ops_rev_before = repo.ops_rev;
        let diff_state_rev_before = repo.diff_state.diff_state_rev;

        let mut hash = 0u64;
        let mut total_effects = 0u64;
        let mut stage_effect_count = 0u64;
        let mut unstage_effect_count = 0u64;
        let mut select_diff_effect_count = 0u64;
        let mut area = DiffArea::Unstaged;
        let mut path_ix = 0usize;
        let mut path_wrap_count = 0u64;

        for _ in 0..self.toggle_events {
            let path = self.paths[path_ix].as_path();

            if let Some(capture) = capture.as_deref_mut() {
                let frame_started = std::time::Instant::now();
                hash ^= self.run_toggle_step(
                    &mut state,
                    area,
                    path,
                    &mut total_effects,
                    &mut stage_effect_count,
                    &mut unstage_effect_count,
                    &mut select_diff_effect_count,
                );
                capture.record_frame(frame_started.elapsed());
            } else {
                hash ^= self.run_toggle_step(
                    &mut state,
                    area,
                    path,
                    &mut total_effects,
                    &mut stage_effect_count,
                    &mut unstage_effect_count,
                    &mut select_diff_effect_count,
                );
            }

            area = match area {
                DiffArea::Unstaged => DiffArea::Staged,
                DiffArea::Staged => DiffArea::Unstaged,
            };
            path_ix = (path_ix + 1) % self.paths.len();
            if path_ix == 0 {
                path_wrap_count = path_wrap_count.saturating_add(1);
            }
        }

        let repo = &state.repos[0];
        (
            hash,
            KeyboardStageUnstageToggleMetrics {
                path_count: u64::try_from(self.paths.len()).unwrap_or(u64::MAX),
                toggle_events: u64::try_from(self.toggle_events).unwrap_or(u64::MAX),
                effect_count: total_effects,
                stage_effect_count,
                unstage_effect_count,
                select_diff_effect_count,
                ops_rev_delta: repo.ops_rev.wrapping_sub(ops_rev_before),
                diff_state_rev_delta: repo
                    .diff_state
                    .diff_state_rev
                    .wrapping_sub(diff_state_rev_before),
                area_flip_count: u64::try_from(self.toggle_events).unwrap_or(u64::MAX),
                path_wrap_count,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_toggle_step(
        &self,
        state: &mut AppState,
        area: DiffArea,
        path: &std::path::Path,
        total_effects: &mut u64,
        stage_effect_count: &mut u64,
        unstage_effect_count: &mut u64,
        select_diff_effect_count: &mut u64,
    ) -> u64 {
        let repo_id = RepoId(1);
        let mut hasher = FxHasher::default();
        let toggle_path = path.to_path_buf();
        match area {
            DiffArea::Unstaged => {
                with_stage_path_sync(state, repo_id, toggle_path, |_state, effects| {
                    record_keyboard_stage_unstage_toggle_effects(
                        effects,
                        total_effects,
                        stage_effect_count,
                        unstage_effect_count,
                        &mut hasher,
                    );
                });
            }
            DiffArea::Staged => {
                with_unstage_path_sync(state, repo_id, toggle_path, |_state, effects| {
                    record_keyboard_stage_unstage_toggle_effects(
                        effects,
                        total_effects,
                        stage_effect_count,
                        unstage_effect_count,
                        &mut hasher,
                    );
                });
            }
        }

        let next_area = match area {
            DiffArea::Unstaged => DiffArea::Staged,
            DiffArea::Staged => DiffArea::Unstaged,
        };
        with_select_diff_sync(
            state,
            repo_id,
            DiffTarget::WorkingTree {
                path: path.to_path_buf(),
                area: next_area,
            },
            |_state, effects| {
                record_keyboard_stage_unstage_select_effects(
                    effects,
                    total_effects,
                    select_diff_effect_count,
                    &mut hasher,
                );
            },
        );

        state.repos[0].ops_rev.hash(&mut hasher);
        state.repos[0].diff_state.diff_state_rev.hash(&mut hasher);
        hasher.finish()
    }
}

fn record_keyboard_stage_unstage_toggle_effects(
    effects: &[Effect],
    total_effects: &mut u64,
    stage_effect_count: &mut u64,
    unstage_effect_count: &mut u64,
    hasher: &mut FxHasher,
) {
    for effect in effects {
        *total_effects = total_effects.saturating_add(1);
        match effect {
            Effect::StagePath { .. } | Effect::StagePaths { .. } => {
                *stage_effect_count = stage_effect_count.saturating_add(1);
            }
            Effect::UnstagePath { .. } | Effect::UnstagePaths { .. } => {
                *unstage_effect_count = unstage_effect_count.saturating_add(1);
            }
            _ => {}
        }
        std::mem::discriminant(effect).hash(hasher);
    }
}

fn record_keyboard_stage_unstage_select_effects(
    effects: &[Effect],
    total_effects: &mut u64,
    select_diff_effect_count: &mut u64,
    hasher: &mut FxHasher,
) {
    for effect in effects {
        let logical_effects = match effect {
            Effect::LoadDiff { .. }
            | Effect::LoadDiffFile { .. }
            | Effect::LoadDiffFileImage { .. } => 1,
            _ => 0,
        };
        *total_effects = total_effects.saturating_add(logical_effects);
        *select_diff_effect_count = select_diff_effect_count.saturating_add(logical_effects);
        std::mem::discriminant(effect).hash(hasher);
    }
}

// ---------------------------------------------------------------------------
// Frame-timing: sidebar resize drag sustained
// ---------------------------------------------------------------------------

/// Fixture for `frame_timing/sidebar_resize_drag_sustained`.
///
/// Runs `frames` drag-step updates on the sidebar pane boundary inside a
/// per-frame timing capture, measuring both the pane-clamp math cost and
/// the layout recomputation cost under sustained interaction. Each frame
/// performs one drag step (same work as `PaneResizeDragStepFixture::run`)
/// and records frame duration via `FrameTimingCapture`.
pub struct SidebarResizeDragSustainedFixture {
    inner: PaneResizeDragStepFixture,
    frames: usize,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct SidebarResizeDragSustainedMetrics {
    pub frames: u64,
    pub steps_per_frame: u64,
    pub total_clamp_at_min: u64,
    pub total_clamp_at_max: u64,
}

impl SidebarResizeDragSustainedFixture {
    pub fn new(frames: usize, frame_budget_ns: u64) -> Self {
        Self {
            inner: PaneResizeDragStepFixture::new(PaneResizeTarget::Sidebar),
            frames: frames.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&mut self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &mut self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        SidebarResizeDragSustainedMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_internal(
        &mut self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, SidebarResizeDragSustainedMetrics) {
        let mut combined_hash = 0u64;
        let mut total_clamp_at_min = 0u64;
        let mut total_clamp_at_max = 0u64;

        // Reset the inner fixture to starting state each invocation so the
        // benchmark is deterministic across iterations.
        self.inner = PaneResizeDragStepFixture::new(PaneResizeTarget::Sidebar);

        for _ in 0..self.frames {
            if let Some(capture) = capture.as_deref_mut() {
                let frame_started = std::time::Instant::now();
                let (hash, clamp_at_min_count, clamp_at_max_count) =
                    self.inner.run_hash_and_clamp_counts();
                capture.record_frame(frame_started.elapsed());
                combined_hash ^= hash;
                total_clamp_at_min = total_clamp_at_min.saturating_add(clamp_at_min_count);
                total_clamp_at_max = total_clamp_at_max.saturating_add(clamp_at_max_count);
            } else {
                let (hash, clamp_at_min_count, clamp_at_max_count) =
                    self.inner.run_hash_and_clamp_counts();
                combined_hash ^= hash;
                total_clamp_at_min = total_clamp_at_min.saturating_add(clamp_at_min_count);
                total_clamp_at_max = total_clamp_at_max.saturating_add(clamp_at_max_count);
            }
        }

        (
            combined_hash,
            SidebarResizeDragSustainedMetrics {
                frames: u64::try_from(self.frames).unwrap_or(u64::MAX),
                steps_per_frame: 200, // PaneResizeDragStepFixture default
                total_clamp_at_min,
                total_clamp_at_max,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Frame-timing: rapid commit selection changes
// ---------------------------------------------------------------------------

/// Fixture for `frame_timing/rapid_commit_selection_changes`.
///
/// Builds `commit_count` synthetic commit details and cycles through them
/// in a round-robin pattern, measuring per-frame cost of replacing the
/// selected commit details. This captures the interactive cost of rapidly
/// arrowing through the history list where each selection triggers a full
/// commit-details replacement render.
pub struct RapidCommitSelectionFixture {
    commits: Vec<CommitDetails>,
    prewarmed_file_rows: CommitFileRowPresentationCache<CommitId>,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct RapidCommitSelectionMetrics {
    pub commit_count: u64,
    pub files_per_commit: u64,
    pub selections: u64,
}

impl RapidCommitSelectionFixture {
    pub fn new(commit_count: usize, files_per_commit: usize, frame_budget_ns: u64) -> Self {
        let commits: Vec<CommitDetails> = (0..commit_count.max(2))
            .map(|ix| {
                // Each commit gets a unique 40-char hex ID by zero-padding the index.
                let mut details = build_synthetic_commit_details(files_per_commit, 4);
                details.id = CommitId(format!("{ix:040x}").into());
                details
            })
            .collect();
        let mut prewarmed_file_rows = CommitFileRowPresentationCache::default();
        if let Some(first) = commits.first() {
            let _ = prewarmed_file_rows.rows_for(&first.id, &first.files);
        }

        Self {
            commits,
            prewarmed_file_rows,
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        RapidCommitSelectionMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, RapidCommitSelectionMetrics) {
        let mut hash = 0u64;
        let count = self.commits.len();
        let mut file_rows = self.prewarmed_file_rows.clone();

        // Start from an already-rendered first commit, then cycle through the
        // remaining selections. This mirrors the warm replacement path the
        // details pane repeats while arrowing through history.
        for ix in 0..count {
            let current = &self.commits[(ix + 1) % count];
            if let Some(capture) = capture.as_deref_mut() {
                let frame_started = std::time::Instant::now();
                hash ^= commit_details_cached_row_hash(current, None, &mut file_rows);
                capture.record_frame(frame_started.elapsed());
            } else {
                hash ^= commit_details_cached_row_hash(current, None, &mut file_rows);
            }
        }

        (
            hash,
            RapidCommitSelectionMetrics {
                commit_count: u64::try_from(count).unwrap_or(u64::MAX),
                files_per_commit: self
                    .commits
                    .first()
                    .map(|c| u64::try_from(c.files.len()).unwrap_or(u64::MAX))
                    .unwrap_or(0),
                selections: u64::try_from(count).unwrap_or(u64::MAX),
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Frame-timing: repo switch during scroll
// ---------------------------------------------------------------------------

/// Fixture for `frame_timing/repo_switch_during_scroll`.
///
/// Interleaves history-list scroll steps with periodic repo switches,
/// measuring per-frame timing for the combined interaction. Every
/// `switch_every_n_frames` frames, a repo switch is performed (via
/// `RepoSwitchFixture::run_with_state`) instead of a scroll step. This
/// captures the jank risk of switching repos while scrolling through
/// the history list.
pub struct RepoSwitchDuringScrollFixture {
    history_fixture: HistoryListScrollFixture,
    repo_switch_fixture: RepoSwitchFixture,
    repo_switch_fixture_reverse: RepoSwitchFixture,
    repo_switch_state: RefCell<AppState>,
    frames: usize,
    window_rows: usize,
    scroll_step_rows: usize,
    switch_every_n_frames: usize,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct RepoSwitchDuringScrollMetrics {
    pub total_frames: u64,
    pub scroll_frames: u64,
    pub switch_frames: u64,
    pub total_rows: u64,
    pub window_rows: u64,
}

impl RepoSwitchDuringScrollFixture {
    pub fn new(
        history_commits: usize,
        local_branches: usize,
        remote_branches: usize,
        window_rows: usize,
        scroll_step_rows: usize,
        frames: usize,
        switch_every_n_frames: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let history_fixture =
            HistoryListScrollFixture::new(history_commits, local_branches, remote_branches);

        // Two-hot-repos switch: models the common case of switching between
        // two active repositories.
        let repo_switch_fixture = RepoSwitchFixture::two_hot_repos(
            history_commits.min(1_000),
            local_branches.min(20),
            remote_branches.min(60),
            4,
        );
        let repo_switch_fixture_reverse = repo_switch_fixture.flipped_direction();
        let repo_switch_state = RefCell::new(repo_switch_fixture.fresh_state());

        Self {
            history_fixture,
            repo_switch_fixture,
            repo_switch_fixture_reverse,
            repo_switch_state,
            frames: frames.max(1),
            window_rows: window_rows.max(1),
            scroll_step_rows: scroll_step_rows.max(1),
            switch_every_n_frames: switch_every_n_frames.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        RepoSwitchDuringScrollMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, RepoSwitchDuringScrollMetrics) {
        let total_rows = self.history_fixture.total_rows();
        let window_rows = self.window_rows.min(total_rows.max(1));
        let max_start = total_rows.saturating_sub(window_rows);
        let mut hash = 0u64;
        let mut start = 0usize;
        let mut scroll_frames = 0u64;
        let mut switch_frames = 0u64;
        let mut repo_state_ref = self.repo_switch_state.borrow_mut();
        let repo_state: &mut AppState = &mut repo_state_ref;
        reset_repo_switch_bench_state(repo_state, &self.repo_switch_fixture.baseline);

        for frame_ix in 0..self.frames {
            let is_switch_frame = frame_ix > 0 && frame_ix % self.switch_every_n_frames == 0;

            if is_switch_frame {
                // Alternate between the two already-live repo states instead of
                // cloning a fresh baseline after every switch. That keeps the
                // timed work on the real hot repo-switch reducer path.
                let switch_fixture =
                    if repo_state.active_repo == Some(self.repo_switch_fixture.target_repo_id) {
                        &self.repo_switch_fixture_reverse
                    } else {
                        &self.repo_switch_fixture
                    };

                if let Some(capture) = capture.as_deref_mut() {
                    let frame_started = std::time::Instant::now();
                    let switch_hash = switch_fixture.run_with_state_hash_only(repo_state);
                    capture.record_frame(frame_started.elapsed());
                    hash ^= switch_hash;
                } else {
                    let switch_hash = switch_fixture.run_with_state_hash_only(repo_state);
                    hash ^= switch_hash;
                }
                switch_frames += 1;
            } else {
                // Scroll frame
                if let Some(capture) = capture.as_deref_mut() {
                    let frame_started = std::time::Instant::now();
                    hash ^= self.history_fixture.run_scroll_step(start, window_rows);
                    capture.record_frame(frame_started.elapsed());
                } else {
                    hash ^= self.history_fixture.run_scroll_step(start, window_rows);
                }
                scroll_frames += 1;

                if max_start > 0 {
                    start = start.saturating_add(self.scroll_step_rows);
                    if start > max_start {
                        start %= max_start + 1;
                    }
                }
            }
        }

        (
            hash,
            RepoSwitchDuringScrollMetrics {
                total_frames: u64::try_from(self.frames).unwrap_or(u64::MAX),
                scroll_frames,
                switch_frames,
                total_rows: u64::try_from(total_rows).unwrap_or(u64::MAX),
                window_rows: u64::try_from(window_rows).unwrap_or(u64::MAX),
            },
        )
    }
}
