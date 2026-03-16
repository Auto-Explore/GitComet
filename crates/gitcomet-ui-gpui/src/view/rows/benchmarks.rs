use super::diff_text::{
    DiffSyntaxBudget, DiffSyntaxLanguage, DiffSyntaxMode, PrepareDiffSyntaxDocumentResult,
    benchmark_diff_syntax_cache_drop_payload_timed_step,
    benchmark_diff_syntax_cache_replacement_drop_step,
    benchmark_diff_syntax_prepared_cache_contains_document,
    benchmark_diff_syntax_prepared_cache_metrics,
    benchmark_diff_syntax_prepared_loaded_chunk_count,
    benchmark_flush_diff_syntax_deferred_drop_queue,
    benchmark_reset_diff_syntax_prepared_cache_metrics, diff_syntax_language_for_path,
    inject_background_prepared_diff_syntax_document,
    prepare_diff_syntax_document_in_background_text,
    prepare_diff_syntax_document_with_budget_reuse_text,
};
use super::*;
use crate::kit::text_model::TextModel;
use crate::kit::{
    benchmark_text_input_runs_legacy_visible_window,
    benchmark_text_input_runs_streamed_visible_window,
};
use crate::theme::AppTheme;
use crate::view::conflict_resolver::{
    self, ConflictBlock, ConflictChoice, ConflictSegment, ThreeWayVisibleItem,
};
use crate::view::history_graph;
use crate::view::markdown_preview::{self, MarkdownPreviewDiff, MarkdownPreviewDocument};
use crate::view::panes::main::diff_cache::{
    PagedPatchDiffRows, PagedPatchSplitRows, PatchInlineVisibleMap,
};
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::domain::{
    Branch, Commit, CommitDetails, CommitFileChange, CommitId, Diff, DiffArea, DiffRowProvider,
    DiffTarget, FileStatusKind, Remote, RemoteBranch, RepoSpec, StashEntry, Submodule,
    SubmoduleStatus, Upstream, UpstreamDivergence, Worktree,
};
use gitcomet_state::model::{ConflictFile, Loadable, RepoId, RepoState};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

pub struct OpenRepoFixture {
    repo: RepoState,
    commits: Vec<Commit>,
    theme: AppTheme,
}

impl OpenRepoFixture {
    pub fn new(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        let theme = AppTheme::zed_ayu_dark();
        let commits_vec = build_synthetic_commits(commits);
        let repo = build_synthetic_repo_state(
            local_branches,
            remote_branches,
            remotes,
            0,
            0,
            0,
            &commits_vec,
        );
        Self {
            repo,
            commits: commits_vec,
            theme,
        }
    }

    pub fn run(&self) -> u64 {
        // Branch sidebar is the main "many branches" transformation.
        let rows = GitCometView::branch_sidebar_rows(&self.repo);

        // History graph is the main "long history" transformation.
        let branch_heads = HashSet::default();
        let graph = history_graph::compute_graph(&self.commits, self.theme, &branch_heads);

        let mut h = DefaultHasher::new();
        rows.len().hash(&mut h);
        graph.len().hash(&mut h);
        graph
            .iter()
            .take(128)
            .map(|r| (r.lanes_now.len(), r.lanes_next.len(), r.is_merge))
            .collect::<Vec<_>>()
            .hash(&mut h);
        h.finish()
    }
}

pub struct BranchSidebarFixture {
    repo: RepoState,
}

impl BranchSidebarFixture {
    pub fn new(
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
        worktrees: usize,
        submodules: usize,
        stashes: usize,
    ) -> Self {
        let commits = build_synthetic_commits(1);
        let repo = build_synthetic_repo_state(
            local_branches,
            remote_branches,
            remotes,
            worktrees,
            submodules,
            stashes,
            &commits,
        );
        Self { repo }
    }

    pub fn run(&self) -> u64 {
        let rows = GitCometView::branch_sidebar_rows(&self.repo);
        let mut h = DefaultHasher::new();
        rows.len().hash(&mut h);
        for row in rows.iter().take(256) {
            std::mem::discriminant(row).hash(&mut h);
            match row {
                BranchSidebarRow::SectionHeader {
                    section,
                    top_border,
                } => {
                    match section {
                        BranchSection::Local => 0u8,
                        BranchSection::Remote => 1u8,
                    }
                    .hash(&mut h);
                    top_border.hash(&mut h);
                }
                BranchSidebarRow::Placeholder { section, message } => {
                    match section {
                        BranchSection::Local => 0u8,
                        BranchSection::Remote => 1u8,
                    }
                    .hash(&mut h);
                    message.len().hash(&mut h);
                }
                BranchSidebarRow::RemoteHeader { name } => name.len().hash(&mut h),
                BranchSidebarRow::GroupHeader { label, depth } => {
                    label.len().hash(&mut h);
                    depth.hash(&mut h);
                }
                BranchSidebarRow::Branch {
                    label,
                    name,
                    depth,
                    muted,
                    is_head,
                    is_upstream,
                    ..
                } => {
                    label.len().hash(&mut h);
                    name.len().hash(&mut h);
                    depth.hash(&mut h);
                    muted.hash(&mut h);
                    is_head.hash(&mut h);
                    is_upstream.hash(&mut h);
                }
                BranchSidebarRow::WorktreeItem {
                    label,
                    tooltip,
                    is_active,
                    ..
                } => {
                    label.len().hash(&mut h);
                    tooltip.len().hash(&mut h);
                    is_active.hash(&mut h);
                }
                BranchSidebarRow::SubmoduleItem { label, tooltip, .. } => {
                    label.len().hash(&mut h);
                    tooltip.len().hash(&mut h);
                }
                BranchSidebarRow::StashItem {
                    index,
                    message,
                    tooltip,
                    ..
                } => {
                    index.hash(&mut h);
                    message.len().hash(&mut h);
                    tooltip.len().hash(&mut h);
                }
                BranchSidebarRow::SectionSpacer
                | BranchSidebarRow::WorktreesHeader { .. }
                | BranchSidebarRow::WorktreePlaceholder { .. }
                | BranchSidebarRow::SubmodulesHeader { .. }
                | BranchSidebarRow::SubmodulePlaceholder { .. }
                | BranchSidebarRow::StashHeader { .. }
                | BranchSidebarRow::StashPlaceholder { .. } => {}
            }
        }
        h.finish()
    }

    #[cfg(test)]
    fn row_count(&self) -> usize {
        GitCometView::branch_sidebar_rows(&self.repo).len()
    }
}

pub struct HistoryGraphFixture {
    commits: Vec<Commit>,
    branch_head_indices: Vec<usize>,
    theme: AppTheme,
}

impl HistoryGraphFixture {
    pub fn new(commits: usize, merge_every: usize, branch_head_every: usize) -> Self {
        let commits_vec = build_synthetic_commits_with_merge_stride(commits, merge_every, 40);
        let mut branch_head_indices = Vec::new();
        if branch_head_every > 0 {
            for ix in (0..commits_vec.len()).step_by(branch_head_every) {
                branch_head_indices.push(ix);
            }
        }
        Self {
            commits: commits_vec,
            branch_head_indices,
            theme: AppTheme::zed_ayu_dark(),
        }
    }

    pub fn run(&self) -> u64 {
        let mut branch_heads: HashSet<&str> = HashSet::default();
        for &ix in &self.branch_head_indices {
            if let Some(commit) = self.commits.get(ix) {
                branch_heads.insert(commit.id.as_ref());
            }
        }

        let graph = history_graph::compute_graph(&self.commits, self.theme, &branch_heads);
        let mut h = DefaultHasher::new();
        graph.len().hash(&mut h);
        graph
            .iter()
            .take(256)
            .map(|r| {
                (
                    r.lanes_now.len(),
                    r.lanes_next.len(),
                    r.joins_in.len(),
                    r.edges_out.len(),
                    r.is_merge,
                )
            })
            .collect::<Vec<_>>()
            .hash(&mut h);
        h.finish()
    }

    #[cfg(test)]
    fn commit_count(&self) -> usize {
        self.commits.len()
    }
}

pub struct CommitDetailsFixture {
    details: CommitDetails,
}

impl CommitDetailsFixture {
    pub fn new(files: usize, depth: usize) -> Self {
        Self {
            details: build_synthetic_commit_details(files, depth),
        }
    }

    pub fn run(&self) -> u64 {
        // Approximation of the per-row work done by the commit files list:
        // kind->icon mapping and formatting the displayed path string.
        let mut h = DefaultHasher::new();
        self.details.id.as_ref().hash(&mut h);
        self.details.message.len().hash(&mut h);

        let mut counts = [0usize; 6];
        for f in &self.details.files {
            let icon: Option<&'static str> = match f.kind {
                FileStatusKind::Added => Some("icons/plus.svg"),
                FileStatusKind::Modified => Some("icons/pencil.svg"),
                FileStatusKind::Deleted => Some("icons/minus.svg"),
                FileStatusKind::Renamed => Some("icons/swap.svg"),
                FileStatusKind::Untracked => Some("icons/question.svg"),
                FileStatusKind::Conflicted => Some("icons/warning.svg"),
            };
            icon.hash(&mut h);
            let kind_key: u8 = match f.kind {
                FileStatusKind::Added => 0,
                FileStatusKind::Modified => 1,
                FileStatusKind::Deleted => 2,
                FileStatusKind::Renamed => 3,
                FileStatusKind::Untracked => 4,
                FileStatusKind::Conflicted => 5,
            };
            kind_key.hash(&mut h);

            // This allocation is a real part of row construction today.
            let path_text = f.path.display().to_string();
            path_text.hash(&mut h);

            counts[kind_key as usize] = counts[kind_key as usize].saturating_add(1);
        }
        counts.hash(&mut h);
        h.finish()
    }
}

pub struct LargeFileDiffScrollFixture {
    lines: Vec<String>,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    theme: AppTheme,
}

impl LargeFileDiffScrollFixture {
    pub fn new(lines: usize) -> Self {
        Self::new_with_line_bytes(lines, 96)
    }

    pub fn new_with_line_bytes(lines: usize, line_bytes: usize) -> Self {
        let theme = AppTheme::zed_ayu_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");
        Self {
            lines: build_synthetic_source_lines(lines, line_bytes),
            language,
            theme,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        // Approximate "a scroll step": style the newly visible rows in a window.
        let end = (start + window).min(self.lines.len());
        let mut h = DefaultHasher::new();
        for line in &self.lines[start..end] {
            let styled = super::diff_text::build_cached_diff_styled_text(
                self.theme,
                line,
                &[],
                "",
                self.language,
                DiffSyntaxMode::Auto,
                None,
            );
            styled.text.len().hash(&mut h);
            styled.highlights.len().hash(&mut h);
        }
        h.finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TextInputShapeCacheKey {
    line_ix: usize,
    wrap_width_key: i32,
    style_epoch: u64,
    text_hash_slice: u64,
}

pub struct TextInputPrepaintWindowedFixture {
    lines: Vec<String>,
    wrap_width_key: i32,
    style_epoch: u64,
    guard_rows: usize,
    max_shape_bytes: usize,
    shape_cache: HashMap<TextInputShapeCacheKey, u64>,
}

impl TextInputPrepaintWindowedFixture {
    pub fn new(lines: usize, line_bytes: usize, wrap_width_px: usize) -> Self {
        Self {
            lines: build_synthetic_source_lines(lines.max(1), line_bytes),
            wrap_width_key: wrap_width_px.max(1) as i32,
            style_epoch: 1,
            guard_rows: 2,
            max_shape_bytes: 4 * 1024,
            shape_cache: HashMap::default(),
        }
    }

    pub fn run_windowed_step(&mut self, start_row: usize, viewport_rows: usize) -> u64 {
        if self.lines.is_empty() || viewport_rows == 0 {
            return 0;
        }

        let line_count = self.lines.len();
        let total_rows = viewport_rows
            .saturating_add(self.guard_rows.saturating_mul(2))
            .max(1);
        let mut h = DefaultHasher::new();

        for row in 0..total_rows {
            let line_ix = start_row.wrapping_add(row) % line_count;
            let (slice_hash, capped_len) = hash_text_input_shaping_slice(
                self.lines.get(line_ix).map(String::as_str).unwrap_or(""),
                self.max_shape_bytes,
            );
            let key = TextInputShapeCacheKey {
                line_ix,
                wrap_width_key: self.wrap_width_key,
                style_epoch: self.style_epoch,
                text_hash_slice: slice_hash,
            };
            let shaped = *self.shape_cache.entry(key).or_insert_with(|| {
                let mut shaped_hash = DefaultHasher::new();
                line_ix.hash(&mut shaped_hash);
                capped_len.hash(&mut shaped_hash);
                slice_hash.hash(&mut shaped_hash);
                shaped_hash.finish()
            });
            shaped.hash(&mut h);
        }

        self.shape_cache.len().hash(&mut h);
        h.finish()
    }

    pub fn run_full_document_step(&mut self) -> u64 {
        self.run_windowed_step(0, self.lines.len())
    }

    pub fn total_rows(&self) -> usize {
        self.lines.len()
    }

    #[cfg(test)]
    fn cache_entries(&self) -> usize {
        self.shape_cache.len()
    }
}

pub struct TextInputLongLineCapFixture {
    line: String,
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
        let mut h = DefaultHasher::new();
        for nonce in 0..64usize {
            let (slice_hash, capped_len) =
                hash_text_input_shaping_slice(self.line.as_str(), max_bytes.max(1));
            nonce.hash(&mut h);
            slice_hash.hash(&mut h);
            capped_len.hash(&mut h);
        }
        h.finish()
    }

    pub fn run_without_cap(&self) -> u64 {
        self.run_with_cap(self.line.len().saturating_add(8))
    }

    #[cfg(test)]
    fn capped_len(&self, max_bytes: usize) -> usize {
        let (_hash, len) = hash_text_input_shaping_slice(self.line.as_str(), max_bytes.max(1));
        len
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextInputHighlightDensity {
    Dense,
    Sparse,
}

pub struct TextInputRunsStreamedHighlightFixture {
    text: String,
    line_starts: Vec<usize>,
    highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
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

    pub fn run_legacy_step(&self, start_row: usize) -> u64 {
        benchmark_text_input_runs_legacy_visible_window(
            self.text.as_str(),
            self.line_starts.as_slice(),
            self.visible_range(start_row),
            self.highlights.as_slice(),
        )
    }

    pub fn run_streamed_step(&self, start_row: usize) -> u64 {
        benchmark_text_input_runs_streamed_visible_window(
            self.text.as_str(),
            self.line_starts.as_slice(),
            self.visible_range(start_row),
            self.highlights.as_slice(),
        )
    }

    pub fn next_start_row(&self, start_row: usize) -> usize {
        let max_start = self.max_start_row().max(1);
        start_row.wrapping_add(self.scroll_step) % (max_start + 1)
    }

    #[cfg(test)]
    fn highlights_len(&self) -> usize {
        self.highlights.len()
    }
}

pub struct TextInputWrapIncrementalTabsFixture {
    lines: Vec<String>,
    row_counts: Vec<usize>,
    wrap_columns: usize,
    edit_nonce: usize,
}

impl TextInputWrapIncrementalTabsFixture {
    pub fn new(lines: usize, line_bytes: usize, wrap_width_px: usize) -> Self {
        let lines = build_synthetic_tabbed_source_lines(lines.max(1), line_bytes.max(8));
        let wrap_columns = wrap_columns_for_benchmark_width(wrap_width_px.max(1));
        let row_counts = lines
            .iter()
            .map(|line| estimate_tabbed_wrap_rows(line.as_str(), wrap_columns))
            .collect::<Vec<_>>();
        Self {
            lines,
            row_counts,
            wrap_columns,
            edit_nonce: 0,
        }
    }

    pub fn run_full_recompute_step(&mut self, edit_line_ix: usize) -> u64 {
        if self.lines.is_empty() {
            return 0;
        }
        let line_ix = edit_line_ix % self.lines.len();
        let _ = mutate_tabbed_line_for_wrap_patch(
            self.lines.get_mut(line_ix).expect("line index must exist"),
            self.edit_nonce,
        );
        self.edit_nonce = self.edit_nonce.wrapping_add(1);
        self.row_counts = self
            .lines
            .iter()
            .map(|line| estimate_tabbed_wrap_rows(line.as_str(), self.wrap_columns))
            .collect();
        hash_wrap_rows(self.row_counts.as_slice())
    }

    pub fn run_incremental_step(&mut self, edit_line_ix: usize) -> u64 {
        if self.lines.is_empty() {
            return 0;
        }
        let line_ix = edit_line_ix % self.lines.len();
        let edit_col = mutate_tabbed_line_for_wrap_patch(
            self.lines.get_mut(line_ix).expect("line index must exist"),
            self.edit_nonce,
        );
        self.edit_nonce = self.edit_nonce.wrapping_add(1);
        let dirty = expand_tabbed_dirty_line_range(
            self.lines.as_slice(),
            line_ix,
            edit_col,
            self.wrap_columns,
        );
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

    #[cfg(test)]
    fn row_counts(&self) -> &[usize] {
        self.row_counts.as_slice()
    }
}

pub struct TextInputWrapIncrementalBurstEditsFixture {
    lines: Vec<String>,
    row_counts: Vec<usize>,
    wrap_columns: usize,
    edit_nonce: usize,
}

impl TextInputWrapIncrementalBurstEditsFixture {
    pub fn new(lines: usize, line_bytes: usize, wrap_width_px: usize) -> Self {
        let lines = build_synthetic_tabbed_source_lines(lines.max(1), line_bytes.max(8));
        let wrap_columns = wrap_columns_for_benchmark_width(wrap_width_px.max(1));
        let row_counts = lines
            .iter()
            .map(|line| estimate_tabbed_wrap_rows(line.as_str(), wrap_columns))
            .collect::<Vec<_>>();
        Self {
            lines,
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
            let _ = mutate_tabbed_line_for_wrap_patch(
                self.lines.get_mut(line_ix).expect("line index must exist"),
                self.edit_nonce.wrapping_add(step),
            );
            self.row_counts = self
                .lines
                .iter()
                .map(|line| estimate_tabbed_wrap_rows(line.as_str(), self.wrap_columns))
                .collect();
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
            let edit_col = mutate_tabbed_line_for_wrap_patch(
                self.lines.get_mut(line_ix).expect("line index must exist"),
                self.edit_nonce.wrapping_add(step),
            );
            let dirty = expand_tabbed_dirty_line_range(
                self.lines.as_slice(),
                line_ix,
                edit_col,
                self.wrap_columns,
            );
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

    #[cfg(test)]
    fn row_counts(&self) -> &[usize] {
        self.row_counts.as_slice()
    }
}

pub struct TextModelSnapshotCloneCostFixture {
    model: TextModel,
    string_control: SharedString,
}

impl TextModelSnapshotCloneCostFixture {
    pub fn new(min_bytes: usize) -> Self {
        let text = build_text_model_document(min_bytes.max(1));
        let model = TextModel::from_large_text(text.as_str());
        let string_control = model.as_shared_string();
        Self {
            model,
            string_control,
        }
    }

    pub fn run_snapshot_clone_step(&self, clones: usize) -> u64 {
        let clones = clones.max(1);
        let snapshot = self.model.snapshot();
        let mut h = DefaultHasher::new();
        self.model.model_id().hash(&mut h);
        self.model.revision().hash(&mut h);

        for nonce in 0..clones {
            let cloned = snapshot.clone();
            nonce.hash(&mut h);
            cloned.len().hash(&mut h);
            cloned.line_starts().len().hash(&mut h);
            let prefix_end = cloned.clamp_to_char_boundary(cloned.len().min(96));
            let prefix = cloned.slice_to_string(0..prefix_end);
            prefix.len().hash(&mut h);
        }
        h.finish()
    }

    pub fn run_string_clone_control_step(&self, clones: usize) -> u64 {
        let clones = clones.max(1);
        let mut h = DefaultHasher::new();
        for nonce in 0..clones {
            let cloned = self.string_control.clone();
            nonce.hash(&mut h);
            cloned.len().hash(&mut h);
            cloned.as_ref().bytes().take(96).count().hash(&mut h);
        }
        h.finish()
    }
}

pub struct TextModelBulkLoadLargeFixture {
    text: String,
}

impl TextModelBulkLoadLargeFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let mut text = String::new();
        let synthetic_lines = build_synthetic_source_lines(lines.max(1), line_bytes.max(32));
        for line in synthetic_lines {
            text.push_str(line.as_str());
            text.push('\n');
        }
        Self { text }
    }

    pub fn run_piece_table_bulk_load_step(&self) -> u64 {
        if self.text.is_empty() {
            return 0;
        }

        let mut model = TextModel::new();
        let mut split = self.text.len() / 2;
        while split > 0 && !self.text.is_char_boundary(split) {
            split = split.saturating_sub(1);
        }

        let _ = model.append_large(&self.text[..split]);
        let _ = model.append_large(&self.text[split..]);
        let snapshot = model.snapshot();

        let mut h = DefaultHasher::new();
        snapshot.len().hash(&mut h);
        snapshot.line_starts().len().hash(&mut h);
        let suffix_start = snapshot.clamp_to_char_boundary(snapshot.len().saturating_sub(96));
        let suffix = snapshot.slice_to_string(suffix_start..snapshot.len());
        suffix.len().hash(&mut h);
        h.finish()
    }

    pub fn run_piece_table_from_large_text_step(&self) -> u64 {
        if self.text.is_empty() {
            return 0;
        }

        let model = TextModel::from_large_text(self.text.as_str());
        let snapshot = model.snapshot();
        let mut h = DefaultHasher::new();
        snapshot.len().hash(&mut h);
        snapshot.line_starts().len().hash(&mut h);
        let prefix_end = snapshot.clamp_to_char_boundary(snapshot.len().min(96));
        let prefix = snapshot.slice_to_string(0..prefix_end);
        prefix.len().hash(&mut h);
        h.finish()
    }

    pub fn run_string_bulk_load_control_step(&self) -> u64 {
        if self.text.is_empty() {
            return 0;
        }

        let mut loaded = String::with_capacity(self.text.len());
        for chunk in self.text.as_bytes().chunks(32 * 1024) {
            if let Ok(chunk_text) = std::str::from_utf8(chunk) {
                loaded.push_str(chunk_text);
            }
        }
        let mut h = DefaultHasher::new();
        loaded.len().hash(&mut h);
        loaded.bytes().take(96).count().hash(&mut h);
        h.finish()
    }
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

fn wrap_columns_for_benchmark_width(wrap_width_px: usize) -> usize {
    let estimated_char_px = (13.0f32 * 0.6).max(1.0);
    ((wrap_width_px as f32) / estimated_char_px)
        .floor()
        .max(1.0) as usize
}

fn estimate_tabbed_wrap_rows(line: &str, wrap_columns: usize) -> usize {
    if line.is_empty() {
        return 1;
    }
    let wrap_columns = wrap_columns.max(1);
    let mut rows = 1usize;
    let mut column = 0usize;
    for ch in line.chars() {
        let width = if ch == '\t' {
            let rem = column % 4;
            if rem == 0 { 4 } else { 4 - rem }
        } else {
            1
        };

        if width >= wrap_columns {
            if column > 0 {
                rows = rows.saturating_add(1);
            }
            rows = rows.saturating_add(width / wrap_columns);
            column = width % wrap_columns;
            if column == 0 {
                column = wrap_columns;
            }
            continue;
        }

        if column + width > wrap_columns {
            rows = rows.saturating_add(1);
            column = width;
        } else {
            column += width;
        }
    }
    rows.max(1)
}

fn mutate_tabbed_line_for_wrap_patch(line: &mut String, nonce: usize) -> usize {
    if line.is_empty() {
        line.push('\t');
    }
    let mut insert_ix = line.find('\t').unwrap_or(0);
    insert_ix = insert_ix.min(line.len());
    let ch = (b'a' + (nonce % 26) as u8) as char;
    line.insert(insert_ix, ch);

    if line.chars().count() > 1 {
        let remove_ix = line
            .char_indices()
            .next_back()
            .map(|(ix, _)| ix)
            .unwrap_or(0);
        let _ = line.remove(remove_ix);
    }
    insert_ix
}

fn expand_tabbed_dirty_line_range(
    lines: &[String],
    line_ix: usize,
    edit_column: usize,
    _wrap_columns: usize,
) -> Range<usize> {
    if lines.is_empty() {
        return 0..0;
    }
    let line_ix = line_ix.min(lines.len().saturating_sub(1));
    let mut end = (line_ix + 1).min(lines.len());
    if let Some(line) = lines.get(line_ix)
        && line
            .get(edit_column.min(line.len())..)
            .is_some_and(|suffix| suffix.contains('\t'))
    {
        end = end.max((line_ix + 1).min(lines.len()));
    }
    if end < lines.len() && lines.get(end).is_some_and(|line| line.starts_with('\t')) {
        end = (end + 1).min(lines.len());
    }
    line_ix..end
}

fn hash_wrap_rows(row_counts: &[usize]) -> u64 {
    let mut h = DefaultHasher::new();
    row_counts.len().hash(&mut h);
    for rows in row_counts.iter().take(512) {
        rows.hash(&mut h);
    }
    h.finish()
}

fn hash_text_input_shaping_slice(text: &str, max_bytes: usize) -> (u64, usize) {
    if text.len() <= max_bytes {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        return (hasher.finish(), text.len());
    }

    let suffix = "…";
    let suffix_len = suffix.len();
    let mut end = max_bytes.saturating_sub(suffix_len).min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }

    let mut truncated = String::with_capacity(end.saturating_add(suffix_len));
    if end > 0 {
        truncated.push_str(&text[..end]);
    }
    truncated.push_str(suffix);

    let mut hasher = DefaultHasher::new();
    truncated.hash(&mut hasher);
    (hasher.finish(), truncated.len())
}

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

fn should_hide_unified_diff_header_for_bench(kind: DiffLineKind, text: &str) -> bool {
    matches!(kind, DiffLineKind::Header)
        && (text.starts_with("index ") || text.starts_with("--- ") || text.starts_with("+++ "))
}

pub struct PatchDiffPagedRowsFixture {
    diff: Arc<Diff>,
}

impl PatchDiffPagedRowsFixture {
    pub fn new(lines: usize) -> Self {
        let target = DiffTarget::WorkingTree {
            path: std::path::PathBuf::from("src/lib.rs"),
            area: DiffArea::Unstaged,
        };
        let text = build_synthetic_unified_patch(lines);
        Self {
            diff: Arc::new(Diff::from_unified(target, text.as_str())),
        }
    }

    pub fn run_eager_full_materialize_step(&self) -> u64 {
        let annotated = annotate_unified(&self.diff);
        let split = build_patch_split_rows(&annotated);
        let theme = AppTheme::zed_ayu_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");
        let mut hasher = DefaultHasher::new();
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
        let split_provider = PagedPatchSplitRows::new(Arc::clone(&rows_provider));
        let theme = AppTheme::zed_ayu_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");

        let mut hasher = DefaultHasher::new();
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
                let styled = super::diff_text::build_cached_diff_styled_text(
                    theme,
                    diff_content_text(&line),
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

    pub fn run_inline_visible_eager_scan_step(&self) -> u64 {
        let rows_provider = PagedPatchDiffRows::new(Arc::clone(&self.diff), 256);
        let mut visible_indices = Vec::new();
        for (src_ix, line) in rows_provider.slice(0, rows_provider.len_hint()).enumerate() {
            if !should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()) {
                visible_indices.push(src_ix);
            }
        }

        let mut hasher = DefaultHasher::new();
        visible_indices.len().hash(&mut hasher);
        for src_ix in visible_indices.into_iter().take(512) {
            src_ix.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn run_inline_visible_hidden_map_step(&self) -> u64 {
        let hidden_flags = self
            .diff
            .lines
            .iter()
            .map(|line| should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()))
            .collect::<Vec<_>>();
        let visible_map = PatchInlineVisibleMap::from_hidden_flags(hidden_flags.as_slice());

        let mut hasher = DefaultHasher::new();
        visible_map.visible_len().hash(&mut hasher);
        for visible_ix in 0..visible_map.visible_len().min(512) {
            visible_map
                .src_ix_for_visible_ix(visible_ix)
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    #[cfg(test)]
    fn inline_visible_indices_eager(&self) -> Vec<usize> {
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
    fn inline_visible_indices_map(&self) -> Vec<usize> {
        let hidden_flags = self
            .diff
            .lines
            .iter()
            .map(|line| should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()))
            .collect::<Vec<_>>();
        let visible_map = PatchInlineVisibleMap::from_hidden_flags(hidden_flags.as_slice());
        (0..visible_map.visible_len())
            .filter_map(|visible_ix| visible_map.src_ix_for_visible_ix(visible_ix))
            .collect()
    }

    #[cfg(test)]
    fn total_rows(&self) -> usize {
        self.diff.lines.len()
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
    query_cache: Vec<Option<CachedDiffStyledText>>,
    query_cache_query: SharedString,
}

impl PatchDiffSearchQueryUpdateFixture {
    pub fn new(lines: usize) -> Self {
        let theme = AppTheme::zed_ayu_dark();
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
    }

    fn sync_query_cache(&mut self, query: &str) {
        if self.query_cache_query.as_ref() != query {
            self.query_cache_query = query.to_string().into();
            self.query_cache.fill(None);
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
            if self
                .query_cache
                .get(src_ix)
                .and_then(Option::as_ref)
                .is_none()
            {
                let base = self.stable_cache.get(src_ix).and_then(Option::as_ref)?;
                let overlay = super::diff_text::build_cached_diff_query_overlay_styled_text(
                    self.theme, base, query,
                );
                if let Some(slot) = self.query_cache.get_mut(src_ix) {
                    *slot = Some(overlay);
                }
            }
            return self
                .query_cache
                .get(src_ix)
                .and_then(Option::as_ref)
                .cloned();
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

        let mut h = DefaultHasher::new();
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

    fn stable_cache_entries(&self) -> usize {
        self.stable_cache
            .iter()
            .filter(|entry| entry.is_some())
            .count()
    }

    fn query_cache_entries(&self) -> usize {
        self.query_cache
            .iter()
            .filter(|entry| entry.is_some())
            .count()
    }
}

fn prepare_bench_diff_syntax_document(
    language: DiffSyntaxLanguage,
    budget: DiffSyntaxBudget,
    text: &str,
    old_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
    let text: SharedString = text.to_owned().into();
    let line_starts: Arc<[usize]> = Arc::from(line_starts_for_text(text.as_ref()));

    match prepare_diff_syntax_document_with_budget_reuse_text(
        language,
        DiffSyntaxMode::Auto,
        text.clone(),
        Arc::clone(&line_starts),
        budget,
        old_document,
        None,
    ) {
        PrepareDiffSyntaxDocumentResult::Ready(document) => Some(document),
        PrepareDiffSyntaxDocumentResult::TimedOut => {
            let old_reparse_seed = old_document.and_then(prepared_diff_syntax_reparse_seed);
            prepare_diff_syntax_document_in_background_text_with_reuse(
                language,
                DiffSyntaxMode::Auto,
                text,
                line_starts,
                old_reparse_seed,
                None,
            )
            .map(inject_background_prepared_diff_syntax_document)
        }
        PrepareDiffSyntaxDocumentResult::Unsupported => None,
    }
}

pub struct FileDiffSyntaxPrepareFixture {
    lines: Vec<String>,
    language: DiffSyntaxLanguage,
    theme: AppTheme,
    budget: DiffSyntaxBudget,
}

impl FileDiffSyntaxPrepareFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let language =
            diff_syntax_language_for_path("src/lib.rs").unwrap_or(DiffSyntaxLanguage::Rust);
        Self {
            lines: build_synthetic_source_lines(lines, line_bytes),
            language,
            theme: AppTheme::zed_ayu_dark(),
            budget: DiffSyntaxBudget::default(),
        }
    }

    pub fn new_query_stress(lines: usize, line_bytes: usize, nesting_depth: usize) -> Self {
        let language =
            diff_syntax_language_for_path("src/lib.rs").unwrap_or(DiffSyntaxLanguage::Rust);
        Self {
            lines: build_synthetic_nested_query_stress_lines(lines, line_bytes, nesting_depth),
            language,
            theme: AppTheme::zed_ayu_dark(),
            budget: DiffSyntaxBudget::default(),
        }
    }

    pub fn prewarm(&self) {
        let _ = self.prepare_document(&self.lines);
    }

    pub fn run_prepare_cold(&self, nonce: u64) -> u64 {
        let lines = self
            .lines
            .iter()
            .enumerate()
            .map(|(ix, line)| format!("{line} // cold_{nonce}_{ix}"))
            .collect::<Vec<_>>();
        let document = self.prepare_document(&lines);
        self.hash_prepared(&lines, document)
    }

    pub fn run_prepare_warm(&self) -> u64 {
        let document = self.prepare_document(&self.lines);
        self.hash_prepared(&self.lines, document)
    }

    pub fn run_prepared_syntax_multidoc_cache_hit_rate_step(&self, docs: usize, nonce: u64) -> u64 {
        let docs = docs.clamp(3, 6);
        benchmark_reset_diff_syntax_prepared_cache_metrics();

        let mut prepared = Vec::with_capacity(docs);
        for doc_ix in 0..docs {
            let lines = self
                .lines
                .iter()
                .enumerate()
                .map(|(line_ix, line)| format!("{line} // multidoc_{nonce}_{doc_ix}_{line_ix}"))
                .collect::<Vec<_>>();
            if let Some(document) = self.prepare_document(&lines) {
                prepared.push((lines, document));
            }
        }

        for (lines, document) in &prepared {
            let _ = self.hash_prepared_line(lines, Some(*document), 0);
        }
        for _ in 0..4 {
            for (lines, document) in &prepared {
                let _ = self.hash_prepared_line(lines, Some(*document), 0);
            }
        }

        let metrics = benchmark_diff_syntax_prepared_cache_metrics();
        let total = metrics.hit.saturating_add(metrics.miss);
        let hit_rate_per_mille = if total == 0 {
            0
        } else {
            metrics.hit.saturating_mul(1000) / total
        };

        let mut h = DefaultHasher::new();
        prepared.len().hash(&mut h);
        metrics.hit.hash(&mut h);
        metrics.miss.hash(&mut h);
        metrics.evict.hash(&mut h);
        metrics.chunk_build_ms.hash(&mut h);
        hit_rate_per_mille.hash(&mut h);
        h.finish()
    }

    pub fn run_prepared_syntax_chunk_miss_cost_step(&self, nonce: u64) -> Duration {
        let lines = self
            .lines
            .iter()
            .enumerate()
            .map(|(ix, line)| {
                if ix == 0 {
                    format!("{line} // chunk_miss_{nonce}")
                } else {
                    line.clone()
                }
            })
            .collect::<Vec<_>>();
        let Some(document) = self.prepare_document(&lines) else {
            return Duration::ZERO;
        };

        benchmark_reset_diff_syntax_prepared_cache_metrics();
        let line_count = lines.len().max(1);
        let chunk_rows = 64usize;
        let chunk_count = line_count.div_ceil(chunk_rows).max(1);
        let chunk_ix = (nonce as usize) % chunk_count;
        let line_ix = chunk_ix
            .saturating_mul(chunk_rows)
            .min(line_count.saturating_sub(1));

        let start = std::time::Instant::now();
        let _ = self.hash_prepared_line(&lines, Some(document), line_ix);
        let elapsed = start.elapsed();

        let metrics = benchmark_diff_syntax_prepared_cache_metrics();
        let _loaded_chunks = benchmark_diff_syntax_prepared_loaded_chunk_count(document);
        let _is_cached = benchmark_diff_syntax_prepared_cache_contains_document(document);
        if metrics.miss == 0 {
            return Duration::ZERO.max(elapsed);
        }
        elapsed
    }

    fn prepare_document(
        &self,
        lines: &[String],
    ) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        let text = lines.join("\n");
        prepare_bench_diff_syntax_document(self.language, self.budget, text.as_str(), None)
    }

    fn hash_prepared(
        &self,
        lines: &[String],
        document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ) -> u64 {
        self.hash_prepared_line(lines, document, 0)
    }

    fn hash_prepared_line(
        &self,
        lines: &[String],
        document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
        line_ix: usize,
    ) -> u64 {
        let line_ix = line_ix.min(lines.len().saturating_sub(1));
        let text = lines.get(line_ix).map(String::as_str).unwrap_or("");
        let styled =
            super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                self.theme,
                text,
                &[],
                "",
                super::diff_text::DiffSyntaxConfig {
                    language: Some(self.language),
                    mode: DiffSyntaxMode::Auto,
                },
                None,
                super::diff_text::PreparedDiffSyntaxLine { document, line_ix },
            )
            .into_inner();

        let mut h = DefaultHasher::new();
        lines.len().hash(&mut h);
        line_ix.hash(&mut h);
        styled.text_hash.hash(&mut h);
        styled.highlights_hash.hash(&mut h);
        h.finish()
    }
}

pub struct FileDiffSyntaxReparseFixture {
    lines: Vec<String>,
    language: DiffSyntaxLanguage,
    theme: AppTheme,
    budget: DiffSyntaxBudget,
    nonce: u64,
    prepared_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
}

impl FileDiffSyntaxReparseFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let language =
            diff_syntax_language_for_path("src/lib.rs").unwrap_or(DiffSyntaxLanguage::Rust);
        Self {
            lines: build_synthetic_source_lines(lines, line_bytes),
            language,
            theme: AppTheme::zed_ayu_dark(),
            budget: DiffSyntaxBudget::default(),
            nonce: 0,
            prepared_document: None,
        }
    }

    pub fn run_small_edit_step(&mut self) -> u64 {
        self.ensure_prepared_document();
        let mut next_lines = self.lines.clone();
        if next_lines.is_empty() {
            next_lines.push(String::new());
        }
        let line_ix = (self.nonce as usize) % next_lines.len();
        let marker = format!(" tiny_reparse_{}", self.nonce);
        next_lines[line_ix].push_str(marker.as_str());
        self.nonce = self.nonce.wrapping_add(1);

        let next_document = self.prepare_document_with_reuse(&next_lines, self.prepared_document);
        if next_document.is_some() {
            self.lines = next_lines;
            self.prepared_document = next_document;
        }

        self.hash_prepared(&self.lines, self.prepared_document)
    }

    pub fn run_large_edit_step(&mut self) -> u64 {
        self.ensure_prepared_document();
        let mut next_lines = self.lines.clone();
        if next_lines.is_empty() {
            next_lines.push(String::new());
        }

        let total_lines = next_lines.len();
        let changed_lines = total_lines.saturating_mul(3) / 5;
        let changed_lines = changed_lines.max(1).min(total_lines);
        let start = if total_lines == 0 {
            0
        } else {
            (self.nonce as usize).wrapping_mul(13) % total_lines
        };
        for offset in 0..changed_lines {
            let ix = (start + offset) % total_lines;
            next_lines[ix] = format!(
                "pub fn fallback_edit_{}_{offset}() {{ let value = {}; }}",
                self.nonce,
                offset.wrapping_mul(17)
            );
        }
        self.nonce = self.nonce.wrapping_add(1);

        let next_document = self.prepare_document_with_reuse(&next_lines, self.prepared_document);
        if next_document.is_some() {
            self.lines = next_lines;
            self.prepared_document = next_document;
        }

        self.hash_prepared(&self.lines, self.prepared_document)
    }

    fn ensure_prepared_document(&mut self) {
        if self.prepared_document.is_some() {
            return;
        }
        self.prepared_document = self.prepare_document_with_reuse(&self.lines, None);
    }

    fn prepare_document_with_reuse(
        &self,
        lines: &[String],
        old_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        let text = lines.join("\n");
        prepare_bench_diff_syntax_document(self.language, self.budget, text.as_str(), old_document)
    }

    fn hash_prepared(
        &self,
        lines: &[String],
        document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ) -> u64 {
        let text = lines.first().map(String::as_str).unwrap_or("");
        let styled =
            super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                self.theme,
                text,
                &[],
                "",
                super::diff_text::DiffSyntaxConfig {
                    language: Some(self.language),
                    mode: DiffSyntaxMode::Auto,
                },
                None,
                super::diff_text::PreparedDiffSyntaxLine {
                    document,
                    line_ix: 0,
                },
            )
            .into_inner();

        let mut h = DefaultHasher::new();
        lines.len().hash(&mut h);
        styled.text_hash.hash(&mut h);
        styled.highlights_hash.hash(&mut h);
        h.finish()
    }
}

pub struct FileDiffInlineSyntaxProjectionFixture {
    inline_rows: Vec<AnnotatedDiffLine>,
    inline_word_highlights: Vec<Option<Vec<Range<usize>>>>,
    language: DiffSyntaxLanguage,
    theme: AppTheme,
    old_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    new_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
}

impl FileDiffInlineSyntaxProjectionFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let language =
            diff_syntax_language_for_path("src/lib.rs").unwrap_or(DiffSyntaxLanguage::Rust);
        let generated_lines = build_synthetic_source_lines(lines.max(1), line_bytes.max(32));

        let mut old_lines = Vec::with_capacity(generated_lines.len());
        let mut new_lines = Vec::with_capacity(generated_lines.len());
        let mut inline_rows = Vec::with_capacity(generated_lines.len().saturating_mul(2));
        let mut inline_word_highlights =
            Vec::with_capacity(generated_lines.len().saturating_mul(2));
        let mut old_line_no = 1u32;
        let mut new_line_no = 1u32;

        for (slot_ix, base_line) in generated_lines.into_iter().enumerate() {
            match slot_ix % 9 {
                0 => {
                    let old_line = format!("{base_line} // inline_remove_{slot_ix}");
                    old_lines.push(old_line.clone());
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Remove,
                        text: format!("-{old_line}").into(),
                        old_line: Some(old_line_no),
                        new_line: None,
                    });
                    inline_word_highlights.push(None);
                    old_line_no = old_line_no.saturating_add(1);
                }
                1 => {
                    let new_line = format!("{base_line} // inline_add_{slot_ix}");
                    new_lines.push(new_line.clone());
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Add,
                        text: format!("+{new_line}").into(),
                        old_line: None,
                        new_line: Some(new_line_no),
                    });
                    inline_word_highlights.push(None);
                    new_line_no = new_line_no.saturating_add(1);
                }
                2 => {
                    let old_line = format!("{base_line} // inline_before_{slot_ix}");
                    let new_line = format!("{base_line} // inline_after_{slot_ix}");
                    old_lines.push(old_line.clone());
                    new_lines.push(new_line.clone());
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Remove,
                        text: format!("-{old_line}").into(),
                        old_line: Some(old_line_no),
                        new_line: None,
                    });
                    inline_word_highlights.push(None);
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Add,
                        text: format!("+{new_line}").into(),
                        old_line: None,
                        new_line: Some(new_line_no),
                    });
                    inline_word_highlights.push(None);
                    old_line_no = old_line_no.saturating_add(1);
                    new_line_no = new_line_no.saturating_add(1);
                }
                _ => {
                    old_lines.push(base_line.clone());
                    new_lines.push(base_line.clone());
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Context,
                        text: format!(" {base_line}").into(),
                        old_line: Some(old_line_no),
                        new_line: Some(new_line_no),
                    });
                    inline_word_highlights.push(None);
                    old_line_no = old_line_no.saturating_add(1);
                    new_line_no = new_line_no.saturating_add(1);
                }
            }
        }

        let budget = DiffSyntaxBudget::default();
        let old_text = old_lines.join("\n");
        let old_document =
            prepare_bench_diff_syntax_document(language, budget, old_text.as_str(), None);
        let new_text = new_lines.join("\n");
        let new_document =
            prepare_bench_diff_syntax_document(language, budget, new_text.as_str(), None);

        Self {
            inline_rows,
            inline_word_highlights,
            language,
            theme: AppTheme::zed_ayu_dark(),
            old_document,
            new_document,
        }
    }

    pub fn run_window_pending_step(&self, start: usize, window: usize) -> u64 {
        self.hash_window_step(start, window).0
    }

    pub fn run_window_step(&self, start: usize, window: usize) -> u64 {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let (hash, pending) = self.hash_window_step(start, window);
            if !pending {
                return hash;
            }
            if std::time::Instant::now() >= deadline {
                return hash;
            }

            let mut applied = 0usize;
            if let Some(document) = self.old_document {
                applied = applied.saturating_add(
                    drain_completed_prepared_diff_syntax_chunk_builds_for_document(document),
                );
            }
            if let Some(document) = self.new_document {
                applied = applied.saturating_add(
                    drain_completed_prepared_diff_syntax_chunk_builds_for_document(document),
                );
            }
            if applied == 0 && self.has_pending_chunks() {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    pub fn prime_window(&self, window: usize) {
        let _ = self.run_window_step(0, window);
    }

    pub fn next_start_row(&self, start: usize, window: usize) -> usize {
        let step = (window.max(1) / 2).saturating_add(1);
        start.wrapping_add(step) % self.inline_rows.len().max(1)
    }

    #[cfg(test)]
    fn visible_rows(&self) -> usize {
        self.inline_rows.len()
    }

    fn has_pending_chunks(&self) -> bool {
        self.old_document
            .is_some_and(has_pending_prepared_diff_syntax_chunk_builds_for_document)
            || self
                .new_document
                .is_some_and(has_pending_prepared_diff_syntax_chunk_builds_for_document)
    }

    fn projected_syntax_line(
        &self,
        line: &AnnotatedDiffLine,
    ) -> super::diff_text::PreparedDiffSyntaxLine {
        super::diff_text::prepared_diff_syntax_line_for_inline_diff_row(
            self.old_document,
            self.new_document,
            line,
        )
    }

    fn hash_window_step(&self, start: usize, window: usize) -> (u64, bool) {
        if self.inline_rows.is_empty() || window == 0 {
            return (0, false);
        }

        let start = start % self.inline_rows.len();
        let end = (start + window).min(self.inline_rows.len());
        let mut pending = false;
        let mut h = DefaultHasher::new();
        for row_ix in start..end {
            let Some(line) = self.inline_rows.get(row_ix) else {
                continue;
            };
            let word_ranges = self
                .inline_word_highlights
                .get(row_ix)
                .and_then(|ranges| ranges.as_deref())
                .unwrap_or(&[]);
            let projected = self.projected_syntax_line(line);
            let syntax_mode =
                super::diff_text::syntax_mode_for_prepared_document(projected.document);
            let word_color = match line.kind {
                DiffLineKind::Add => Some(self.theme.colors.success),
                DiffLineKind::Remove => Some(self.theme.colors.danger),
                _ => None,
            };
            let (styled, is_pending) =
                super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                    self.theme,
                    diff_content_text(line),
                    word_ranges,
                    "",
                    super::diff_text::DiffSyntaxConfig {
                        language: Some(self.language),
                        mode: syntax_mode,
                    },
                    word_color,
                    projected,
                )
                .into_parts();
            pending |= is_pending;
            row_ix.hash(&mut h);
            is_pending.hash(&mut h);
            styled.text_hash.hash(&mut h);
            styled.highlights_hash.hash(&mut h);
        }
        self.inline_rows.len().hash(&mut h);
        (h.finish(), pending)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LargeHtmlSyntaxSource {
    External,
    Synthetic,
}

pub struct LargeHtmlSyntaxFixture {
    source: LargeHtmlSyntaxSource,
    text: Arc<str>,
    line_starts: Arc<[usize]>,
    line_count: usize,
    theme: AppTheme,
    prepared_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
}

impl LargeHtmlSyntaxFixture {
    pub fn new(
        fixture_path: Option<&str>,
        synthetic_lines: usize,
        synthetic_line_bytes: usize,
    ) -> Self {
        Self::new_internal(fixture_path, synthetic_lines, synthetic_line_bytes, false)
    }

    pub fn new_prewarmed(
        fixture_path: Option<&str>,
        synthetic_lines: usize,
        synthetic_line_bytes: usize,
    ) -> Self {
        Self::new_internal(fixture_path, synthetic_lines, synthetic_line_bytes, true)
    }

    fn new_internal(
        fixture_path: Option<&str>,
        synthetic_lines: usize,
        synthetic_line_bytes: usize,
        prewarm_document: bool,
    ) -> Self {
        let (source, text) = load_large_html_bench_text(fixture_path).unwrap_or_else(|| {
            (
                LargeHtmlSyntaxSource::Synthetic,
                build_synthetic_large_html_text(synthetic_lines, synthetic_line_bytes),
            )
        });
        let text: Arc<str> = Arc::from(text);
        let line_starts: Arc<[usize]> = Arc::from(line_starts_for_text(text.as_ref()));
        let line_count = line_starts.len().max(1);
        let prepared_document = prewarm_document
            .then(|| Self::prepare_document(text.as_ref()))
            .flatten();

        Self {
            source,
            text,
            line_starts,
            line_count,
            theme: AppTheme::zed_ayu_dark(),
            prepared_document,
        }
    }

    pub fn source_label(&self) -> &'static str {
        match self.source {
            LargeHtmlSyntaxSource::External => "external_html_fixture",
            LargeHtmlSyntaxSource::Synthetic => "synthetic_html_fixture",
        }
    }

    pub fn run_background_prepare_step(&self) -> u64 {
        let prepared = prepare_diff_syntax_document_in_background_text(
            DiffSyntaxLanguage::Html,
            DiffSyntaxMode::Auto,
            self.text.as_ref().to_owned().into(),
            Arc::clone(&self.line_starts),
        );

        let mut h = DefaultHasher::new();
        self.text.len().hash(&mut h);
        self.line_count.hash(&mut h);
        self.source_label().hash(&mut h);
        prepared.is_some().hash(&mut h);
        h.finish()
    }

    pub fn run_visible_window_pending_step(&self, start_line: usize, window_lines: usize) -> u64 {
        let Some(document) = self.prepared_document_handle() else {
            return 0;
        };
        let Some(result) =
            self.request_visible_window_for_lines(document, start_line, window_lines)
        else {
            return 0;
        };
        self.hash_visible_window_result(start_line, window_lines, &result)
    }

    pub fn run_visible_window_step(&self, start_line: usize, window_lines: usize) -> u64 {
        let Some(document) = self.prepared_document_handle() else {
            return 0;
        };
        let Some(result) =
            self.request_visible_window_until_ready(document, start_line, window_lines)
        else {
            return 0;
        };
        self.hash_visible_window_result(start_line, window_lines, &result)
    }

    pub fn prime_visible_window(&self, window_lines: usize) {
        let _ = self.run_visible_window_step(0, window_lines);
    }

    pub fn next_start_line(&self, start_line: usize, window_lines: usize) -> usize {
        let step = (window_lines.max(1) / 2).saturating_add(1);
        start_line.wrapping_add(step) % self.line_count.max(1)
    }

    #[cfg(test)]
    fn source(&self) -> LargeHtmlSyntaxSource {
        self.source
    }

    #[cfg(test)]
    fn line_count(&self) -> usize {
        self.line_count
    }

    fn prepared_document_handle(&self) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        self.prepared_document
            .or_else(|| Self::prepare_document(self.text.as_ref()))
    }

    fn prepare_document(text: &str) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        prepare_bench_diff_syntax_document(
            DiffSyntaxLanguage::Html,
            DiffSyntaxBudget::default(),
            text,
            None,
        )
    }

    fn visible_window_byte_range(&self, start_line: usize, window_lines: usize) -> Range<usize> {
        if self.line_count == 0 || window_lines == 0 {
            return 0..0;
        }

        let start_line = start_line % self.line_count.max(1);
        let end_line = (start_line + window_lines.max(1)).min(self.line_count);
        let text_len = self.text.len();
        let start = self
            .line_starts
            .get(start_line)
            .copied()
            .unwrap_or(text_len)
            .min(text_len);
        let end = self
            .line_starts
            .get(end_line)
            .copied()
            .unwrap_or(text_len)
            .min(text_len)
            .max(start);
        start..end
    }

    fn request_visible_window_for_lines(
        &self,
        document: super::diff_text::PreparedDiffSyntaxDocument,
        start_line: usize,
        window_lines: usize,
    ) -> Option<super::diff_text::PreparedDocumentByteRangeHighlights> {
        let byte_range = self.visible_window_byte_range(start_line, window_lines);
        self.request_visible_window(document, byte_range)
    }

    fn request_visible_window_until_ready(
        &self,
        document: super::diff_text::PreparedDiffSyntaxDocument,
        start_line: usize,
        window_lines: usize,
    ) -> Option<super::diff_text::PreparedDocumentByteRangeHighlights> {
        let byte_range = self.visible_window_byte_range(start_line, window_lines);
        let mut result = self.request_visible_window(document, byte_range.clone());
        for _ in 0..64 {
            if match result.as_ref() {
                None => true,
                Some(highlights) => !highlights.pending,
            } {
                break;
            }

            let applied = drain_completed_prepared_diff_syntax_chunk_builds_for_document(document);
            if applied == 0 && has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
            {
                std::thread::yield_now();
            }
            result = self.request_visible_window(document, byte_range.clone());
        }
        result
    }

    fn request_visible_window(
        &self,
        document: super::diff_text::PreparedDiffSyntaxDocument,
        byte_range: Range<usize>,
    ) -> Option<super::diff_text::PreparedDocumentByteRangeHighlights> {
        super::diff_text::request_syntax_highlights_for_prepared_document_byte_range(
            self.theme,
            self.text.as_ref(),
            self.line_starts.as_ref(),
            document,
            DiffSyntaxLanguage::Html,
            byte_range,
        )
    }

    fn hash_visible_window_result(
        &self,
        start_line: usize,
        window_lines: usize,
        result: &super::diff_text::PreparedDocumentByteRangeHighlights,
    ) -> u64 {
        let mut h = DefaultHasher::new();
        start_line.hash(&mut h);
        window_lines.hash(&mut h);
        result.pending.hash(&mut h);
        result.highlights.len().hash(&mut h);
        for (range, _style) in result.highlights.iter().take(256) {
            range.start.hash(&mut h);
            range.end.hash(&mut h);
        }
        h.finish()
    }
}

pub struct FileDiffSyntaxCacheDropFixture {
    lines: usize,
    tokens_per_line: usize,
    replacements: usize,
}

impl FileDiffSyntaxCacheDropFixture {
    pub fn new(lines: usize, tokens_per_line: usize, replacements: usize) -> Self {
        Self {
            lines: lines.max(1),
            tokens_per_line: tokens_per_line.max(1),
            replacements: replacements.max(1),
        }
    }

    pub fn run_deferred_drop_step(&self) -> u64 {
        benchmark_diff_syntax_cache_replacement_drop_step(
            self.lines,
            self.tokens_per_line,
            self.replacements,
            true,
        )
    }

    pub fn run_inline_drop_control_step(&self) -> u64 {
        benchmark_diff_syntax_cache_replacement_drop_step(
            self.lines,
            self.tokens_per_line,
            self.replacements,
            false,
        )
    }

    pub fn run_deferred_drop_timed_step(&self, seed: usize) -> Duration {
        let mut total = Duration::ZERO;
        for step in 0..self.replacements {
            total = total.saturating_add(benchmark_diff_syntax_cache_drop_payload_timed_step(
                self.lines,
                self.tokens_per_line,
                seed.wrapping_add(step),
                true,
            ));
        }
        total
    }

    pub fn run_inline_drop_control_timed_step(&self, seed: usize) -> Duration {
        let mut total = Duration::ZERO;
        for step in 0..self.replacements {
            total = total.saturating_add(benchmark_diff_syntax_cache_drop_payload_timed_step(
                self.lines,
                self.tokens_per_line,
                seed.wrapping_add(step),
                false,
            ));
        }
        total
    }

    pub fn flush_deferred_drop_queue(&self) -> bool {
        benchmark_flush_diff_syntax_deferred_drop_queue()
    }
}

pub struct WorktreePreviewRenderFixture {
    lines: Vec<String>,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    prepared_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    theme: AppTheme,
}

impl WorktreePreviewRenderFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let generated_lines = build_synthetic_source_lines(lines, line_bytes);
        let language = diff_syntax_language_for_path("src/lib.rs");
        let syntax_mode = DiffSyntaxMode::Auto;
        let generated_text = generated_lines.join("\n");
        let prepared_document = language.and_then(|language| {
            prepare_bench_diff_syntax_document(
                language,
                DiffSyntaxBudget::default(),
                &generated_text,
                None,
            )
        });

        Self {
            lines: generated_lines,
            language,
            syntax_mode,
            prepared_document,
            theme: AppTheme::zed_ayu_dark(),
        }
    }

    pub fn run_cached_lookup_step(&self, start: usize, window: usize) -> u64 {
        self.hash_window(start, window, self.prepared_document)
    }

    pub fn run_render_time_prepare_step(&self, start: usize, window: usize) -> u64 {
        let text = self.lines.join("\n");
        let prepared_document = self.language.and_then(|language| {
            prepare_bench_diff_syntax_document(
                language,
                DiffSyntaxBudget::default(),
                text.as_str(),
                None,
            )
        });
        self.hash_window(start, window, prepared_document)
    }

    fn hash_window(
        &self,
        start: usize,
        window: usize,
        prepared_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ) -> u64 {
        if self.lines.is_empty() || window == 0 {
            return 0;
        }

        let start = start % self.lines.len();
        let end = (start + window).min(self.lines.len());
        let mut h = DefaultHasher::new();
        for line_ix in start..end {
            let line = self.lines.get(line_ix).map(String::as_str).unwrap_or("");
            let styled = super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                self.theme,
                line,
                &[],
                "",
                super::diff_text::DiffSyntaxConfig {
                    language: self.language,
                    mode: self.syntax_mode,
                },
                None,
                super::diff_text::PreparedDiffSyntaxLine {
                    document: prepared_document,
                    line_ix,
                },
            ).into_inner();
            line_ix.hash(&mut h);
            styled.text_hash.hash(&mut h);
            styled.highlights_hash.hash(&mut h);
        }
        h.finish()
    }
}

pub struct MarkdownPreviewFixture {
    single_source: String,
    old_source: String,
    new_source: String,
    single_document: MarkdownPreviewDocument,
    diff_preview: MarkdownPreviewDiff,
    theme: AppTheme,
}

impl MarkdownPreviewFixture {
    pub fn new(sections: usize, line_bytes: usize) -> Self {
        let sections = sections.max(1);
        let line_bytes = line_bytes.max(48);
        let single_source = build_synthetic_markdown_document(sections, line_bytes, "single");
        let old_source = build_synthetic_markdown_document(sections, line_bytes, "before");
        let new_source = build_synthetic_markdown_document(sections, line_bytes, "after");
        let single_document = markdown_preview::parse_markdown(&single_source)
            .expect("synthetic markdown benchmark fixture should stay within preview limits");
        let diff_preview = markdown_preview::build_markdown_diff_preview(&old_source, &new_source)
            .expect("synthetic markdown diff benchmark fixture should stay within preview limits");

        Self {
            single_source,
            old_source,
            new_source,
            single_document,
            diff_preview,
            theme: AppTheme::zed_ayu_dark(),
        }
    }

    pub fn run_parse_single_step(&self) -> u64 {
        let Some(document) = markdown_preview::parse_markdown(&self.single_source) else {
            return 0;
        };
        hash_markdown_preview_document(&document)
    }

    pub fn run_parse_diff_step(&self) -> u64 {
        let Some(preview) =
            markdown_preview::build_markdown_diff_preview(&self.old_source, &self.new_source)
        else {
            return 0;
        };
        let mut h = DefaultHasher::new();
        hash_markdown_preview_document_into(&preview.old, &mut h);
        hash_markdown_preview_document_into(&preview.new, &mut h);
        h.finish()
    }

    pub fn run_render_single_step(&self, start: usize, window: usize) -> u64 {
        self.hash_render_window(&self.single_document, start, window)
    }

    pub fn run_render_diff_step(&self, start: usize, window: usize) -> u64 {
        if window == 0 {
            return 0;
        }

        let left = self.render_window(&self.diff_preview.old, start, window);
        let right = self.render_window(&self.diff_preview.new, start, window);

        let mut h = DefaultHasher::new();
        start.hash(&mut h);
        window.hash(&mut h);
        std::hint::black_box(left).len().hash(&mut h);
        std::hint::black_box(right).len().hash(&mut h);
        h.finish()
    }

    fn hash_render_window(
        &self,
        document: &MarkdownPreviewDocument,
        start: usize,
        window: usize,
    ) -> u64 {
        if window == 0 {
            return 0;
        }

        let rows = self.render_window(document, start, window);
        let mut h = DefaultHasher::new();
        start.hash(&mut h);
        window.hash(&mut h);
        std::hint::black_box(rows).len().hash(&mut h);
        h.finish()
    }

    fn render_window(
        &self,
        document: &MarkdownPreviewDocument,
        start: usize,
        window: usize,
    ) -> Vec<AnyElement> {
        if document.rows.is_empty() || window == 0 {
            return Vec::new();
        }

        let start = start % document.rows.len();
        let end = (start + window).min(document.rows.len());
        super::history::render_markdown_preview_document_rows(
            self.theme,
            document,
            start..end,
            None,
            px(0.0),
            "benchmark_markdown_preview",
            None,
        )
    }
}

pub struct ConflictThreeWayScrollFixture {
    base_lines: Vec<SharedString>,
    ours_lines: Vec<SharedString>,
    theirs_lines: Vec<SharedString>,
    base_word_highlights: conflict_resolver::WordHighlights,
    ours_word_highlights: conflict_resolver::WordHighlights,
    theirs_word_highlights: conflict_resolver::WordHighlights,
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
        let theme = AppTheme::zed_ayu_dark();
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

        let mut h = DefaultHasher::new();
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

        let mut h = DefaultHasher::new();
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
    fn syntax_mode(&self) -> DiffSyntaxMode {
        self.syntax_mode
    }

    #[cfg(test)]
    fn has_prepared_documents(&self) -> bool {
        self.base_document.is_some()
            && self.ours_document.is_some()
            && self.theirs_document.is_some()
    }
}

fn hash_three_way_visible_map_items(items: &[ThreeWayVisibleItem]) -> u64 {
    let mut h = DefaultHasher::new();
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
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> Vec<ThreeWayVisibleItem> {
    if !hide_resolved {
        return (0..total_lines).map(ThreeWayVisibleItem::Line).collect();
    }

    let resolved_blocks: Vec<bool> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b.resolved),
            _ => None,
        })
        .collect();

    let mut visible = Vec::with_capacity(total_lines);
    let mut line = 0usize;
    while line < total_lines {
        if let Some((range_ix, range)) = conflict_ranges
            .iter()
            .enumerate()
            .find(|(_, r)| r.contains(&line))
            .filter(|(ri, _)| resolved_blocks.get(*ri).copied().unwrap_or(false))
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
        let conflict_count = ours_ranges.len();

        Self {
            total_lines,
            conflict_ranges: ours_ranges,
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
            &self.segments,
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
    fn build_linear_map(&self) -> Vec<ThreeWayVisibleItem> {
        conflict_resolver::build_three_way_visible_map(
            self.total_lines,
            &self.conflict_ranges,
            &self.segments,
            true,
        )
    }

    #[cfg(test)]
    fn build_legacy_map(&self) -> Vec<ThreeWayVisibleItem> {
        build_three_way_visible_map_legacy(
            self.total_lines,
            &self.conflict_ranges,
            &self.segments,
            true,
        )
    }
}

pub struct ConflictTwoWaySplitScrollFixture {
    diff_rows: Vec<gitcomet_core::file_diff::FileDiffRow>,
    diff_word_highlights_split: conflict_resolver::TwoWayWordHighlights,
    diff_row_conflict_map: Vec<Option<usize>>,
    visible_row_indices: Vec<usize>,
    conflict_count: usize,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    theme: AppTheme,
}

struct BlockLocalTwoWayBenchmarkRows {
    diff_rows: Vec<gitcomet_core::file_diff::FileDiffRow>,
    diff_word_highlights_split: conflict_resolver::TwoWayWordHighlights,
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

impl ConflictTwoWaySplitScrollFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let theme = AppTheme::zed_ayu_dark();
        let segments = build_synthetic_two_way_segments(lines, conflict_blocks);
        let conflict_count = conflict_block_count_for_segments(&segments);
        let BlockLocalTwoWayBenchmarkRows {
            diff_rows,
            diff_word_highlights_split,
            diff_row_conflict_map,
            visible_row_indices,
        } = build_block_local_two_way_benchmark_rows(&segments);

        Self {
            diff_rows,
            diff_word_highlights_split,
            diff_row_conflict_map,
            visible_row_indices,
            conflict_count,
            language: diff_syntax_language_for_path("src/conflict.rs"),
            syntax_mode: DiffSyntaxMode::Auto,
            theme,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        if self.visible_row_indices.is_empty() || window == 0 {
            return 0;
        }
        let start = start % self.visible_row_indices.len();
        let end = (start + window).min(self.visible_row_indices.len());

        let mut h = DefaultHasher::new();
        for &row_ix in &self.visible_row_indices[start..end] {
            self.diff_row_conflict_map
                .get(row_ix)
                .copied()
                .flatten()
                .hash(&mut h);

            let Some(row) = self.diff_rows.get(row_ix) else {
                continue;
            };
            let (old_word_ranges, new_word_ranges) =
                two_way_word_ranges_for_row(&self.diff_word_highlights_split, row_ix);

            if let Some(old_text) = row.old.as_deref() {
                let styled = super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    old_text,
                    old_word_ranges,
                    "",
                    self.language,
                    self.syntax_mode,
                    None,
                );
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }

            if let Some(new_text) = row.new.as_deref() {
                let styled = super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    new_text,
                    new_word_ranges,
                    "",
                    self.language,
                    self.syntax_mode,
                    None,
                );
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
    fn diff_rows(&self) -> usize {
        self.diff_rows.len()
    }

    #[cfg(test)]
    fn syntax_mode(&self) -> DiffSyntaxMode {
        self.syntax_mode
    }
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
    path: std::path::PathBuf,
    session: ConflictSession,
    current_text: Arc<str>,
    current_bytes: Arc<[u8]>,
    line_count: usize,
    conflict_count: usize,
}

impl ConflictLoadDuplicationFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let path = std::path::PathBuf::from("fixtures/large_conflict.html");
        let (base_text, ours_text, theirs_text, current_text) =
            build_synthetic_html_conflict_texts(lines, conflict_blocks);
        let current_text: Arc<str> = current_text.into();
        let current_bytes = Arc::<[u8]>::from(current_text.as_bytes());
        let session = ConflictSession::from_merged_text(
            path.clone(),
            gitcomet_core::domain::FileConflictKind::BothModified,
            ConflictPayload::Text(base_text.into()),
            ConflictPayload::Text(ours_text.into()),
            ConflictPayload::Text(theirs_text.into()),
            current_text.as_ref(),
        );
        let conflict_count = session.regions.len();
        let line_count = session
            .ours
            .as_text()
            .map(|text| text.lines().count())
            .unwrap_or_default();

        Self {
            path,
            session,
            current_text,
            current_bytes,
            line_count,
            conflict_count,
        }
    }

    pub fn run_shared_payload_forwarding_step(&self) -> u64 {
        let file = self.build_shared_conflict_file();
        let mut h = DefaultHasher::new();
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
        let mut h = DefaultHasher::new();
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

    fn build_shared_conflict_file(&self) -> ConflictFile {
        let (base_bytes, base) = shared_conflict_file_side_from_payload(&self.session.base);
        let (ours_bytes, ours) = shared_conflict_file_side_from_payload(&self.session.ours);
        let (theirs_bytes, theirs) = shared_conflict_file_side_from_payload(&self.session.theirs);

        ConflictFile {
            path: self.path.clone(),
            base_bytes,
            ours_bytes,
            theirs_bytes,
            current_bytes: None,
            base,
            ours,
            theirs,
            current: Some(self.current_text.clone()),
        }
    }

    fn build_duplicated_conflict_file(&self) -> ConflictFile {
        let (base_bytes, base) = duplicated_conflict_file_side_from_payload(&self.session.base);
        let (ours_bytes, ours) = duplicated_conflict_file_side_from_payload(&self.session.ours);
        let (theirs_bytes, theirs) =
            duplicated_conflict_file_side_from_payload(&self.session.theirs);

        ConflictFile {
            path: self.path.clone(),
            base_bytes,
            ours_bytes,
            theirs_bytes,
            current_bytes: Some(Arc::<[u8]>::from(self.current_bytes.as_ref())),
            base,
            ours,
            theirs,
            current: Some(Arc::<str>::from(self.current_text.as_ref())),
        }
    }
}

fn shared_conflict_file_side_from_payload(
    payload: &ConflictPayload,
) -> (Option<Arc<[u8]>>, Option<Arc<str>>) {
    match payload {
        ConflictPayload::Text(text) => (None, Some(text.clone())),
        ConflictPayload::Binary(bytes) => (Some(bytes.clone()), None),
        ConflictPayload::Absent => (None, None),
    }
}

fn duplicated_conflict_file_side_from_payload(
    payload: &ConflictPayload,
) -> (Option<Arc<[u8]>>, Option<Arc<str>>) {
    match payload {
        ConflictPayload::Text(text) => (
            Some(Arc::<[u8]>::from(text.as_bytes())),
            Some(Arc::<str>::from(text.as_ref())),
        ),
        ConflictPayload::Binary(bytes) => (Some(Arc::<[u8]>::from(bytes.as_ref())), None),
        ConflictPayload::Absent => (None, None),
    }
}

fn hash_conflict_file_load(
    session: &ConflictSession,
    current_text: &Arc<str>,
    current_bytes: &Arc<[u8]>,
    file: &ConflictFile,
) -> u64 {
    let mut h = DefaultHasher::new();
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
    h: &mut DefaultHasher,
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
    let mut h = DefaultHasher::new();
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
    let mut h = DefaultHasher::new();
    highlights.len().hash(&mut h);
    let step = (highlights.len() / 128).max(1);
    for highlight in highlights.iter().step_by(step).take(128) {
        match highlight {
            Some((old_ranges, new_ranges)) => {
                hash_ranges(old_ranges, &mut h);
                hash_ranges(new_ranges, &mut h);
            }
            None => 0usize.hash(&mut h),
        }
    }
    h.finish()
}

fn hash_ranges(ranges: &[Range<usize>], hasher: &mut DefaultHasher) {
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

pub struct ConflictSearchQueryUpdateFixture {
    diff_rows: Vec<gitcomet_core::file_diff::FileDiffRow>,
    diff_word_highlights_split: conflict_resolver::TwoWayWordHighlights,
    visible_row_indices: Vec<usize>,
    conflict_count: usize,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    theme: AppTheme,
    stable_cache: HashMap<(usize, ConflictPickSide), CachedDiffStyledText>,
    query_cache: HashMap<(usize, ConflictPickSide), CachedDiffStyledText>,
    query_cache_query: SharedString,
}

impl ConflictSearchQueryUpdateFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let theme = AppTheme::zed_ayu_dark();
        let segments = build_synthetic_two_way_segments(lines, conflict_blocks);
        let conflict_count = conflict_block_count_for_segments(&segments);
        let BlockLocalTwoWayBenchmarkRows {
            diff_rows,
            diff_word_highlights_split,
            diff_row_conflict_map: _,
            visible_row_indices,
        } = build_block_local_two_way_benchmark_rows(&segments);

        let mut fixture = Self {
            diff_rows,
            diff_word_highlights_split,
            visible_row_indices,
            conflict_count,
            language: diff_syntax_language_for_path("src/conflict.rs"),
            syntax_mode: DiffSyntaxMode::Auto,
            theme,
            stable_cache: HashMap::default(),
            query_cache: HashMap::default(),
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
                row.old.as_deref(),
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
                row.new.as_deref(),
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
        stable_cache: &mut HashMap<(usize, ConflictPickSide), CachedDiffStyledText>,
        query_cache: &mut HashMap<(usize, ConflictPickSide), CachedDiffStyledText>,
        row_ix: usize,
        side: ConflictPickSide,
        text: Option<&str>,
        word_ranges: &[Range<usize>],
        query: &str,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
    ) -> Option<CachedDiffStyledText> {
        let text = text?;
        if text.is_empty() {
            return None;
        }

        let query = query.trim();
        let query_active = !query.is_empty();
        let base_has_style = !word_ranges.is_empty() || syntax_lang.is_some();
        let key = (row_ix, side);

        if base_has_style {
            stable_cache.entry(key).or_insert_with(|| {
                super::diff_text::build_cached_diff_styled_text(
                    theme,
                    text,
                    word_ranges,
                    "",
                    syntax_lang,
                    syntax_mode,
                    None,
                )
            });
        }

        if query_active {
            query_cache.entry(key).or_insert_with(|| {
                if let Some(base) = stable_cache.get(&key) {
                    super::diff_text::build_cached_diff_query_overlay_styled_text(
                        theme, base, query,
                    )
                } else {
                    super::diff_text::build_cached_diff_styled_text(
                        theme,
                        text,
                        word_ranges,
                        query,
                        syntax_lang,
                        syntax_mode,
                        None,
                    )
                }
            });
            return query_cache.get(&key).cloned();
        }

        if base_has_style {
            stable_cache.get(&key).cloned()
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

        let mut h = DefaultHasher::new();
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
                row.old.as_deref(),
                old_word_ranges,
                query,
                self.language,
                self.syntax_mode,
            );
            if let Some(styled) = old {
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }

            let new = Self::split_row_styled(
                self.theme,
                &mut self.stable_cache,
                &mut self.query_cache,
                row_ix,
                ConflictPickSide::Theirs,
                row.new.as_deref(),
                new_word_ranges,
                query,
                self.language,
                self.syntax_mode,
            );
            if let Some(styled) = new {
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
    fn stable_cache_entries(&self) -> usize {
        self.stable_cache.len()
    }

    #[cfg(test)]
    fn query_cache_entries(&self) -> usize {
        self.query_cache.len()
    }

    #[cfg(test)]
    fn diff_rows(&self) -> usize {
        self.diff_rows.len()
    }

    #[cfg(test)]
    fn syntax_mode(&self) -> DiffSyntaxMode {
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

        let mut h = DefaultHasher::new();
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
    fn stable_cache_entries(&self) -> usize {
        self.inner.stable_cache_entries()
    }

    #[cfg(test)]
    fn query_cache_entries(&self) -> usize {
        self.inner.query_cache_entries()
    }

    #[cfg(test)]
    fn split_ratio(&self) -> f32 {
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
    line_sources: Vec<conflict_resolver::ResolvedLineSource>,
    markers: Vec<Option<ResolvedOutputGutterMarker>>,
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

        Self {
            line_sources,
            markers,
            active_conflict: conflict_count / 2,
            conflict_count,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        if self.line_sources.is_empty() || window == 0 {
            return 0;
        }
        let start = start % self.line_sources.len();
        let end = (start + window).min(self.line_sources.len());

        let mut h = DefaultHasher::new();
        for line_ix in start..end {
            let source = self
                .line_sources
                .get(line_ix)
                .copied()
                .unwrap_or(conflict_resolver::ResolvedLineSource::Manual);
            source.hash(&mut h);
            source.badge_char().hash(&mut h);
            (line_ix + 1).hash(&mut h);

            let marker = self.markers.get(line_ix).copied().flatten();
            (source == conflict_resolver::ResolvedLineSource::Manual && marker.is_none())
                .hash(&mut h);

            if let Some(marker) = marker {
                marker.conflict_ix.hash(&mut h);
                marker.is_start.hash(&mut h);
                marker.is_end.hash(&mut h);
                marker.unresolved.hash(&mut h);
                let lane_state = if marker.unresolved {
                    0u8
                } else if marker.conflict_ix == self.active_conflict {
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
        self.line_sources.len()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_count
    }
}

pub struct ResolvedOutputRecomputeIncrementalFixture {
    base_text: String,
    ours_text: String,
    theirs_text: String,
    base_line_starts: Vec<usize>,
    ours_line_starts: Vec<usize>,
    theirs_line_starts: Vec<usize>,
    block_ranges: Vec<Range<usize>>,
    block_unresolved: Vec<bool>,
    output_text: String,
    output_line_starts: Vec<usize>,
    meta: Vec<conflict_resolver::ResolvedLineMeta>,
    markers: Vec<Option<ResolvedOutputGutterMarker>>,
    edit_line_ix: usize,
    edit_nonce: u64,
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
        let line_count = conflict_resolver::split_output_lines_for_outline(&output_text).len();
        let block_unresolved = marker_segments
            .iter()
            .filter_map(|segment| match segment {
                ConflictSegment::Block(block) => Some(!block.resolved),
                _ => None,
            })
            .collect::<Vec<_>>();

        let mut fixture = Self {
            base_text,
            ours_text,
            theirs_text,
            base_line_starts,
            ours_line_starts,
            theirs_line_starts,
            block_ranges,
            block_unresolved,
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

    fn next_single_line_edit(&mut self) -> (String, Range<usize>, Range<usize>) {
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
        (next, old_range, new_range)
    }

    fn hash_outline_state(&self) -> u64 {
        let mut h = DefaultHasher::new();
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

    pub fn run_full_recompute_step(&mut self) -> u64 {
        let (next_output, _old_range, _new_range) = self.next_single_line_edit();
        let next_line_starts = line_starts_for_text(&next_output);
        let line_count = conflict_resolver::split_output_lines_for_outline(&next_output).len();
        let next_meta = self.recompute_meta_full(next_output.as_str());
        let next_markers = self.rebuild_markers(line_count);

        self.output_text = next_output;
        self.output_line_starts = next_line_starts;
        self.meta = next_meta;
        self.markers = next_markers;

        self.hash_outline_state()
    }

    pub fn run_incremental_recompute_step(&mut self) -> u64 {
        let old_text = self.output_text.clone();
        let old_line_starts = self.output_line_starts.clone();

        let (next_output, old_byte_range, new_byte_range) = self.next_single_line_edit();
        let next_line_starts = line_starts_for_text(&next_output);
        let next_line_count = conflict_resolver::split_output_lines_for_outline(&next_output).len();
        let source_lookup = self.build_source_lookup();

        let mut old_dirty =
            Self::dirty_line_range(old_line_starts.as_slice(), old_text.len(), old_byte_range);
        let mut new_dirty = Self::dirty_line_range(
            next_line_starts.as_slice(),
            next_output.len(),
            new_byte_range,
        );
        old_dirty.start = old_dirty.start.saturating_sub(1);
        old_dirty.end = old_dirty.end.saturating_add(1).min(self.meta.len());
        new_dirty.start = new_dirty.start.saturating_sub(1);
        new_dirty.end = new_dirty.end.saturating_add(1).min(next_line_count);
        if old_dirty.start != new_dirty.start {
            // Keep this fixture conservative; fall back to full for odd shifts.
            self.output_text = next_output;
            self.output_line_starts = next_line_starts;
            self.meta = self.recompute_meta_full(self.output_text.as_str());
            self.markers = self.rebuild_markers(next_line_count);
            return self.hash_outline_state();
        }

        let line_delta = new_dirty.len() as isize - old_dirty.len() as isize;
        let mut next_meta = Vec::with_capacity(next_line_count);
        next_meta.extend(
            self.meta
                .iter()
                .take(old_dirty.start.min(self.meta.len()))
                .cloned(),
        );
        for line_ix in new_dirty.clone() {
            let line = self.line_text(next_output.as_str(), next_line_starts.as_slice(), line_ix);
            let (source, input_line) = source_lookup
                .get(line)
                .copied()
                .unwrap_or((conflict_resolver::ResolvedLineSource::Manual, None));
            next_meta.push(conflict_resolver::ResolvedLineMeta {
                output_line: u32::try_from(line_ix).unwrap_or(u32::MAX),
                source,
                input_line,
            });
        }
        for meta in self.meta.iter().skip(old_dirty.end.min(self.meta.len())) {
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
            self.output_text = next_output;
            self.output_line_starts = next_line_starts;
            self.meta = self.recompute_meta_full(self.output_text.as_str());
            self.markers = self.rebuild_markers(next_line_count);
            return self.hash_outline_state();
        }

        let mut next_markers = if self.markers.len() == next_line_count {
            self.markers.clone()
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

        self.hash_outline_state()
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
        let mut h = DefaultHasher::new();
        index.total_rows().hash(&mut h);
        h.finish()
    }

    fn hash_visible_window(&self, start: usize, end: usize) -> u64 {
        let mut h = DefaultHasher::new();
        for vi in start..end {
            if let Some((source_ix, _conflict_ix)) = self.two_way_projection.get(vi) {
                if let Some(row) = self.split_row_index.row_at(&self.segments, source_ix) {
                    std::mem::discriminant(&row.kind).hash(&mut h);
                    row.old.as_deref().map(|s| s.len()).hash(&mut h);
                    row.new.as_deref().map(|s| s.len()).hash(&mut h);
                }
            }
        }
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
            .search_matching_rows(&self.segments, |line| line.contains(needle));
        let mut h = DefaultHasher::new();
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
        let mut h = DefaultHasher::new();
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
    fn cached_page_count(&self) -> usize {
        self.split_row_index.cached_page_count()
    }

    #[cfg(test)]
    fn anchor_count(&self) -> usize {
        // Access the single block entry's anchor pair count.
        match &self.segments[0] {
            ConflictSegment::Block(block) => {
                // Rebuild just the anchor index to inspect it.
                let ours_starts = line_starts_for_text(&block.ours);
                let theirs_starts = line_starts_for_text(&block.theirs);
                let anchor = conflict_resolver::ConflictAnchorIndex::build_for_benchmark(
                    &block.ours,
                    &ours_starts,
                    block.ours.lines().count(),
                    &block.theirs,
                    &theirs_starts,
                    block.theirs.lines().count(),
                );
                anchor.ours_to_theirs.len()
            }
            _ => 0,
        }
    }

    /// Total metadata bytes: split row index + two-way projection (excludes page cache
    /// and source text, which are shared).
    #[cfg(test)]
    fn metadata_byte_size(&self) -> usize {
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
        let mut h = DefaultHasher::new();
        projection.len().hash(&mut h);
        projection.output_hash().hash(&mut h);
        h.finish()
    }

    fn hash_visible_window(&self, start: usize, end: usize) -> u64 {
        let mut h = DefaultHasher::new();
        for line_ix in start..end {
            if let Some(line) = self.projection.line_text(&self.segments, line_ix) {
                line.len().hash(&mut h);
                line.as_bytes().first().copied().hash(&mut h);
                line.as_bytes().last().copied().hash(&mut h);
            }
        }
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
    fn metadata_byte_size(&self) -> usize {
        self.projection.metadata_byte_size()
    }
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

fn build_synthetic_repo_state(
    local_branches: usize,
    remote_branches: usize,
    remotes: usize,
    worktrees: usize,
    submodules: usize,
    stashes: usize,
    commits: &[Commit],
) -> RepoState {
    let id = RepoId(1);
    let spec = RepoSpec {
        workdir: std::path::PathBuf::from("/tmp/bench"),
    };
    let mut repo = RepoState::new_opening(id, spec);

    let head = "main".to_string();
    repo.head_branch = Loadable::Ready(head.clone());

    let target = commits
        .first()
        .map(|c| c.id.clone())
        .unwrap_or_else(|| CommitId("0".repeat(40)));

    let mut branches = Vec::with_capacity(local_branches.max(1));
    branches.push(Branch {
        name: head.clone(),
        target: target.clone(),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: head.clone(),
        }),
        divergence: Some(UpstreamDivergence {
            ahead: 1,
            behind: 2,
        }),
    });
    for ix in 0..local_branches.saturating_sub(1) {
        branches.push(Branch {
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target: target.clone(),
            upstream: None,
            divergence: None,
        });
    }
    repo.branches = Loadable::Ready(Arc::new(branches));

    let mut remotes_vec = Vec::with_capacity(remotes.max(1));
    for r in 0..remotes.max(1) {
        remotes_vec.push(Remote {
            name: if r == 0 {
                "origin".to_string()
            } else {
                format!("remote{r}")
            },
            url: None,
        });
    }
    repo.remotes = Loadable::Ready(Arc::new(remotes_vec.clone()));

    let mut remote = Vec::with_capacity(remote_branches);
    for ix in 0..remote_branches {
        let remote_name = if remotes <= 1 || ix % remotes == 0 {
            "origin".to_string()
        } else {
            format!("remote{}", ix % remotes)
        };
        remote.push(RemoteBranch {
            remote: remote_name,
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target: target.clone(),
        });
    }
    repo.remote_branches = Loadable::Ready(Arc::new(remote));

    let mut worktrees_vec = Vec::with_capacity(worktrees);
    for ix in 0..worktrees {
        let path = if ix == 0 {
            repo.spec.workdir.clone()
        } else {
            std::path::PathBuf::from(format!("/tmp/bench-worktree-{ix}"))
        };
        worktrees_vec.push(Worktree {
            path,
            head: Some(target.clone()),
            branch: Some(format!("feature/worktree/{ix}")),
            detached: ix % 7 == 0,
        });
    }
    repo.worktrees = Loadable::Ready(Arc::new(worktrees_vec));

    let mut submodules_vec = Vec::with_capacity(submodules);
    for ix in 0..submodules {
        submodules_vec.push(Submodule {
            path: std::path::PathBuf::from(format!("deps/submodule_{ix}")),
            head: CommitId(format!("{:040x}", 200_000usize.saturating_add(ix))),
            status: if ix % 5 == 0 {
                SubmoduleStatus::HeadMismatch
            } else {
                SubmoduleStatus::UpToDate
            },
        });
    }
    repo.submodules = Loadable::Ready(Arc::new(submodules_vec));

    let stash_base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_100_000);
    let mut stashes_vec = Vec::with_capacity(stashes);
    for ix in 0..stashes {
        stashes_vec.push(StashEntry {
            index: ix,
            id: CommitId(format!("{:040x}", 300_000usize.saturating_add(ix))),
            message: format!("WIP synthetic stash #{ix}"),
            created_at: Some(stash_base + Duration::from_secs(ix as u64)),
        });
    }
    repo.stashes = Loadable::Ready(Arc::new(stashes_vec));

    // Minimal "repo is open" status.
    repo.open = Loadable::Ready(());

    repo
}

fn build_synthetic_commits(count: usize) -> Vec<Commit> {
    build_synthetic_commits_with_merge_stride(count, 50, 40)
}

fn build_synthetic_commits_with_merge_stride(
    count: usize,
    merge_every: usize,
    merge_back_distance: usize,
) -> Vec<Commit> {
    if count == 0 {
        return Vec::new();
    }

    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let mut commits = Vec::with_capacity(count);

    for ix in 0..count {
        let id = CommitId(format!("{:040x}", ix));

        let mut parent_ids = Vec::new();
        if ix > 0 {
            parent_ids.push(CommitId(format!("{:040x}", ix - 1)));
        }
        // Synthetic merge-like commits at a fixed cadence.
        if merge_every > 0
            && merge_back_distance > 0
            && ix >= merge_back_distance
            && ix % merge_every == 0
        {
            parent_ids.push(CommitId(format!("{:040x}", ix - merge_back_distance)));
        }

        commits.push(Commit {
            id,
            parent_ids,
            summary: format!("Commit {ix} - synthetic benchmark history entry"),
            author: format!("Author {}", ix % 10),
            time: base + Duration::from_secs(ix as u64),
        });
    }

    commits
}

fn build_synthetic_commit_details(files: usize, depth: usize) -> CommitDetails {
    let id = CommitId("d".repeat(40));
    let mut out = Vec::with_capacity(files);
    for ix in 0..files {
        let kind = match ix % 23 {
            0 => FileStatusKind::Deleted,
            1 | 2 => FileStatusKind::Renamed,
            3..=5 => FileStatusKind::Added,
            6 => FileStatusKind::Conflicted,
            7 => FileStatusKind::Untracked,
            _ => FileStatusKind::Modified,
        };

        let mut path = std::path::PathBuf::new();
        let depth = depth.max(1);
        for d in 0..depth {
            path.push(format!("dir{}_{}", d, ix % 128));
        }
        path.push(format!("file_{ix}.rs"));

        out.push(CommitFileChange { path, kind });
    }

    CommitDetails {
        id,
        message: "Synthetic benchmark commit details message\n\nWith body.".to_string(),
        committed_at: "2024-01-01T00:00:00Z".to_string(),
        parent_ids: vec![CommitId("c".repeat(40))],
        files: out,
    }
}

fn build_synthetic_source_lines(count: usize, target_line_bytes: usize) -> Vec<String> {
    let target_line_bytes = target_line_bytes.max(32);
    let mut lines = Vec::with_capacity(count);
    for ix in 0..count {
        let indent = " ".repeat((ix % 8) * 2);
        let mut line = match ix % 10 {
            0 => format!("{indent}fn func_{ix}(x: usize) -> usize {{ x + {ix} }}"),
            1 => format!("{indent}let value_{ix} = \"string {ix}\";"),
            2 => format!("{indent}// comment {ix} with some extra words and tokens"),
            3 => format!("{indent}if value_{ix} > 10 {{ return value_{ix}; }}"),
            4 => format!(
                "{indent}for i in 0..{r} {{ sum += i; }}",
                r = (ix % 100) + 1
            ),
            5 => format!("{indent}match tag_{ix} {{ Some(v) => v, None => 0 }}"),
            6 => format!("{indent}struct S{ix} {{ a: i32, b: String }}"),
            7 => format!(
                "{indent}impl S{ix} {{ fn new() -> Self {{ Self {{ a: 0, b: String::new() }} }} }}"
            ),
            8 => format!("{indent}const CONST_{ix}: u64 = {v};", v = ix as u64 * 31),
            _ => format!("{indent}println!(\"{ix} {{}}\", value_{ix});"),
        };
        if line.len() < target_line_bytes {
            line.push(' ');
            line.push_str("//");
            while line.len() < target_line_bytes {
                line.push_str(" token_");
                line.push_str(&(ix % 997).to_string());
            }
        }
        lines.push(line);
    }
    lines
}

fn load_large_html_bench_text(
    fixture_path: Option<&str>,
) -> Option<(LargeHtmlSyntaxSource, String)> {
    let path = fixture_path?.trim();
    if path.is_empty() {
        return None;
    }

    let text = std::fs::read_to_string(path).ok()?;
    if text.is_empty() {
        return None;
    }

    Some((LargeHtmlSyntaxSource::External, text))
}

fn build_synthetic_large_html_text(line_count: usize, target_line_bytes: usize) -> String {
    let line_count = line_count.max(12);
    let target_line_bytes = target_line_bytes.max(96);
    let mut lines = Vec::with_capacity(line_count);

    lines.push("<!doctype html>".to_string());
    lines.push("<html lang=\"en\">".to_string());
    lines.push("<head>".to_string());
    lines.push("<meta charset=\"utf-8\">".to_string());
    lines.push("<title>GitComet Synthetic HTML Fixture</title>".to_string());
    lines.push("<style>".to_string());
    lines.push(
        ".fixture-root { color: #222; background: linear-gradient(90deg, #fff, #f5f5f5); }"
            .to_string(),
    );
    lines.push("</style>".to_string());
    lines.push("</head>".to_string());
    lines.push("<body class=\"fixture-root\">".to_string());

    let reserved_suffix_lines = 2usize;
    let body_lines = line_count.saturating_sub(lines.len().saturating_add(reserved_suffix_lines));
    for ix in 0..body_lines {
        let mut line = match ix % 8 {
            0 => format!(
                r#"<style>.row-{ix} {{ color: rgb({r}, {g}, {b}); padding: {pad}px; }}</style>"#,
                r = (ix * 13) % 255,
                g = (ix * 29) % 255,
                b = (ix * 47) % 255,
                pad = (ix % 9) + 2,
            ),
            1 => format!(
                r#"<script>const card{ix} = {ix}; function bump{ix}() {{ return card{ix} + 1; }}</script>"#
            ),
            2 => format!(
                r#"<div class="row row-{ix}" data-row="{ix}" style="color: rgb({r}, {g}, {b}); background: linear-gradient(90deg, #fff, #eee);" onclick="const next = {ix}; return next + 1;">card {ix}</div>"#,
                r = (ix * 7) % 255,
                g = (ix * 17) % 255,
                b = (ix * 23) % 255,
            ),
            3 => format!(
                r#"<section id="panel-{ix}"><h2>Panel {ix}</h2><p>row {ix} content for syntax benchmarking</p></section>"#
            ),
            4 => {
                format!(r#"<!-- html comment {ix} with repeated tokens for benchmark coverage -->"#)
            }
            5 => {
                format!(r#"<template><span class="slot-{ix}">{{{{value_{ix}}}}}</span></template>"#)
            }
            6 => format!(
                r#"<svg viewBox="0 0 10 10"><path d="M0 0 L10 {y}" stroke="currentColor" /></svg>"#,
                y = (ix % 9) + 1,
            ),
            _ => format!(
                r#"<article data-kind="bench-{ix}" aria-label="row {ix}"><a href="/items/{ix}">open {ix}</a></article>"#
            ),
        };

        if line.len() < target_line_bytes {
            line.push(' ');
            while line.len() < target_line_bytes {
                line.push_str("<!-- filler_token_html_bench -->");
            }
        }
        lines.push(line);
    }

    lines.push("</body>".to_string());
    lines.push("</html>".to_string());
    lines.truncate(line_count);
    lines.join("\n")
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
    let max_conflicts = total_lines.saturating_sub(header_lines.len()).max(1);
    let conflict_blocks = requested_conflict_blocks.max(1).min(max_conflicts);
    let context_lines = total_lines
        .saturating_sub(header_lines.len())
        .saturating_sub(conflict_blocks);
    let context_slots = conflict_blocks;
    let context_per_slot = context_lines / context_slots;
    let context_remainder = context_lines % context_slots;

    let mut base_lines = header_lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut ours_lines = base_lines.clone();
    let mut theirs_lines = base_lines.clone();
    let mut current_lines = base_lines.clone();
    let mut next_context_row = 0usize;

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

fn build_synthetic_markdown_document(
    sections: usize,
    target_line_bytes: usize,
    variant: &str,
) -> String {
    let sections = sections.max(1);
    let target_line_bytes = target_line_bytes.max(48);
    let mut source = String::new();

    for ix in 0..sections {
        if !source.is_empty() {
            source.push('\n');
        }

        push_padded_markdown_line(
            &mut source,
            format!("# Section {variant} {ix}"),
            target_line_bytes,
            ix,
        );
        source.push_str("\n\n");
        push_padded_markdown_line(
            &mut source,
            format!(
                "Paragraph {variant} {ix} explains markdown preview rendering and diff tinting."
            ),
            target_line_bytes,
            ix + 1,
        );
        source.push_str("\n\n");
        push_padded_markdown_line(
            &mut source,
            format!("- [x] completed item {variant} {ix}"),
            target_line_bytes,
            ix + 2,
        );
        source.push('\n');
        push_padded_markdown_line(
            &mut source,
            format!("- [ ] pending item {variant} {ix}"),
            target_line_bytes,
            ix + 3,
        );
        source.push_str("\n\n");
        push_padded_markdown_line(
            &mut source,
            format!("> quoted note {variant} {ix} for preview rows"),
            target_line_bytes,
            ix + 4,
        );
        source.push_str("\n\n```rust\n");
        push_padded_markdown_line(
            &mut source,
            format!("fn section_{ix}_before_after() {{ println!(\"{variant}_{ix}\"); }}"),
            target_line_bytes,
            ix + 5,
        );
        source.push('\n');
        push_padded_markdown_line(
            &mut source,
            format!("let preview_{ix} = \"{variant}_code_{ix}\";"),
            target_line_bytes,
            ix + 6,
        );
        source.push_str("\n```\n\n| key | value |\n| --- | ----- |\n");
        push_padded_markdown_line(
            &mut source,
            format!("| section_{ix} | table value {variant} {ix} |"),
            target_line_bytes,
            ix + 7,
        );
        source.push('\n');
    }

    source
}

fn push_padded_markdown_line(
    buffer: &mut String,
    mut line: String,
    target_line_bytes: usize,
    seed: usize,
) {
    if line.len() < target_line_bytes {
        line.push(' ');
        while line.len() < target_line_bytes {
            line.push_str(" markdown_token_");
            line.push_str(&(seed % 997).to_string());
        }
    }
    buffer.push_str(&line);
}

fn hash_markdown_preview_document(document: &MarkdownPreviewDocument) -> u64 {
    let mut h = DefaultHasher::new();
    hash_markdown_preview_document_into(document, &mut h);
    h.finish()
}

fn hash_markdown_preview_document_into(
    document: &MarkdownPreviewDocument,
    hasher: &mut DefaultHasher,
) {
    document.rows.len().hash(hasher);
    if document.rows.is_empty() {
        return;
    }

    let step = (document.rows.len() / 8).max(1);
    for (ix, row) in document.rows.iter().enumerate().step_by(step).take(8) {
        ix.hash(hasher);
        std::mem::discriminant(&row.kind).hash(hasher);
        row.source_line_range.start.hash(hasher);
        row.source_line_range.end.hash(hasher);
        row.indent_level.hash(hasher);
        row.blockquote_level.hash(hasher);
        row.footnote_label
            .as_ref()
            .map(AsRef::<str>::as_ref)
            .hash(hasher);
        row.alert_kind.hash(hasher);
        row.starts_alert.hash(hasher);
        std::mem::discriminant(&row.change_hint).hash(hasher);
        row.inline_spans.len().hash(hasher);

        let sample_len = row.text.len().min(32);
        row.text
            .as_ref()
            .get(..sample_len)
            .unwrap_or("")
            .hash(hasher);
    }
}

fn build_synthetic_nested_query_stress_lines(
    count: usize,
    target_line_bytes: usize,
    nesting_depth: usize,
) -> Vec<String> {
    let target_line_bytes = target_line_bytes.max(256);
    let nesting_depth = nesting_depth.max(32);
    let mut lines = Vec::with_capacity(count);
    for ix in 0..count {
        let mut line = String::with_capacity(target_line_bytes.saturating_add(nesting_depth * 2));
        line.push_str("let stress_");
        line.push_str(&ix.to_string());
        line.push_str(" = ");
        line.push_str(&"(".repeat(nesting_depth));
        line.push_str("value_");
        line.push_str(&(ix % 97).to_string());
        line.push_str(&")".repeat(nesting_depth));
        line.push_str("; // nested");
        while line.len() < target_line_bytes {
            line.push_str(" (deep_token_");
            line.push_str(&(ix % 101).to_string());
            line.push(')');
        }
        lines.push(line);
    }
    lines
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

fn line_starts_for_text(text: &str) -> Vec<usize> {
    let mut line_starts =
        Vec::with_capacity(text.as_bytes().iter().filter(|&&b| b == b'\n').count() + 1);
    line_starts.push(0);
    for (ix, byte) in text.as_bytes().iter().enumerate() {
        if *byte == b'\n' {
            line_starts.push(ix + 1);
        }
    }
    line_starts
}

fn build_text_input_streamed_highlights(
    text: &str,
    line_starts: &[usize],
    density: TextInputHighlightDensity,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let theme = AppTheme::zed_ayu_dark();
    let style_primary = gpui::HighlightStyle {
        color: Some(theme.colors.accent.into()),
        ..gpui::HighlightStyle::default()
    };
    let style_secondary = gpui::HighlightStyle {
        color: Some(theme.colors.warning.into()),
        ..gpui::HighlightStyle::default()
    };
    let style_overlay = gpui::HighlightStyle {
        color: Some(theme.colors.success.into()),
        ..gpui::HighlightStyle::default()
    };

    let mut highlights = Vec::new();
    for line_ix in 0..line_starts.len() {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let mut line_end = line_starts.get(line_ix + 1).copied().unwrap_or(text.len());
        if line_end > line_start && text.as_bytes().get(line_end - 1) == Some(&b'\n') {
            line_end = line_end.saturating_sub(1);
        }
        if line_end <= line_start {
            continue;
        }
        let line_len = line_end.saturating_sub(line_start);

        match density {
            TextInputHighlightDensity::Dense => {
                let mut local = 0usize;
                while local + 2 < line_len {
                    let start = line_start + local;
                    let end = (start + 20).min(line_end);
                    if start < end {
                        let style = if local.is_multiple_of(24) {
                            style_primary
                        } else {
                            style_secondary
                        };
                        highlights.push((start..end, style));
                    }

                    let overlap_start = start.saturating_add(4).min(line_end);
                    let overlap_end = (overlap_start + 14).min(line_end);
                    if overlap_start < overlap_end {
                        highlights.push((overlap_start..overlap_end, style_overlay));
                    }

                    local = local.saturating_add(12);
                }
            }
            TextInputHighlightDensity::Sparse => {
                if line_ix % 8 == 0 {
                    let start = line_start.saturating_add(2).min(line_end);
                    let end = (start + 26).min(line_end);
                    if start < end {
                        highlights.push((start..end, style_primary));
                    }
                }
                if line_ix % 24 == 0 {
                    let start = line_start.saturating_add(10).min(line_end);
                    let end = (start + 18).min(line_end);
                    if start < end {
                        highlights.push((start..end, style_overlay));
                    }
                }
            }
        }
    }

    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    highlights
}

fn word_ranges_for_line(
    highlights: &conflict_resolver::WordHighlights,
    line_ix: usize,
) -> &[Range<usize>] {
    highlights
        .get(&line_ix)
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}

fn two_way_word_ranges_for_row(
    highlights: &conflict_resolver::TwoWayWordHighlights,
    row_ix: usize,
) -> (&[Range<usize>], &[Range<usize>]) {
    highlights
        .get(row_ix)
        .and_then(|entry| entry.as_ref())
        .map(|(old, new)| (old.as_slice(), new.as_slice()))
        .unwrap_or((&[], &[]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_three_way_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictThreeWayScrollFixture::new(120, 12);
        assert_eq!(fixture.conflict_count(), 12);
        assert_eq!(fixture.visible_rows(), 120);
    }

    #[test]
    fn conflict_three_way_fixture_wraps_start_offsets() {
        let fixture = ConflictThreeWayScrollFixture::new(180, 18);
        let hash_a = fixture.run_scroll_step(17, 40);
        let hash_b = fixture.run_scroll_step(17 + fixture.visible_rows() * 3, 40);
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn conflict_three_way_fixture_uses_auto_syntax_for_large_inputs() {
        let fixture = ConflictThreeWayScrollFixture::new(6_000, 24);
        assert_eq!(fixture.syntax_mode(), DiffSyntaxMode::Auto);
    }

    #[test]
    fn conflict_three_way_prepared_fixture_has_documents_for_all_sides() {
        let fixture = ConflictThreeWayScrollFixture::new_with_prepared_documents(120, 12);
        assert!(
            fixture.has_prepared_documents(),
            "prepared fixture should produce documents for base, ours, and theirs"
        );
        assert_eq!(fixture.conflict_count(), 12);
        assert_eq!(fixture.visible_rows(), 120);
    }

    #[test]
    fn conflict_three_way_prepared_fixture_scroll_step_differs_from_fallback() {
        let prepared = ConflictThreeWayScrollFixture::new_with_prepared_documents(180, 18);
        let fallback = ConflictThreeWayScrollFixture::new(180, 18);
        let prepared_hash = prepared.run_prepared_scroll_step(0, 40);
        let fallback_hash = fallback.run_scroll_step(0, 40);
        // The prepared-document path includes pending state and uses tree-sitter
        // highlights, so its hash should generally differ from the per-line
        // fallback path.
        assert_ne!(
            prepared_hash, fallback_hash,
            "prepared-document and fallback scroll steps should produce different hashes"
        );
    }

    #[test]
    fn conflict_three_way_prepared_fixture_wraps_start_offsets() {
        let fixture = ConflictThreeWayScrollFixture::new_with_prepared_documents(180, 18);
        let hash_a = fixture.run_prepared_scroll_step(17, 40);
        let hash_b = fixture.run_prepared_scroll_step(17 + fixture.visible_rows() * 3, 40);
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn conflict_three_way_plain_fixture_has_no_prepared_documents() {
        let fixture = ConflictThreeWayScrollFixture::new(120, 12);
        assert!(
            !fixture.has_prepared_documents(),
            "plain fixture should not have prepared documents"
        );
    }

    #[test]
    fn conflict_three_way_visible_map_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictThreeWayVisibleMapBuildFixture::new(120, 12);
        assert_eq!(fixture.conflict_count(), 12);
        assert!(fixture.visible_rows() > 0);
    }

    #[test]
    fn conflict_three_way_visible_map_fixture_linear_matches_legacy_scan() {
        let fixture = ConflictThreeWayVisibleMapBuildFixture::new(240, 24);
        assert_eq!(fixture.build_linear_map(), fixture.build_legacy_map());
        assert_eq!(fixture.run_linear_step(), fixture.run_legacy_step());
    }

    #[test]
    fn conflict_two_way_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictTwoWaySplitScrollFixture::new(120, 12);
        assert_eq!(fixture.conflict_count(), 12);
        assert!(fixture.visible_rows() > 0);
    }

    #[test]
    fn conflict_two_way_fixture_matches_block_local_rows_and_auto_syntax() {
        let fixture = ConflictTwoWaySplitScrollFixture::new(1_200, 12);
        let build_fixture = ConflictTwoWayDiffBuildFixture::new(1_200, 12);
        assert_eq!(fixture.diff_rows(), build_fixture.block_local_diff_rows());
        assert!(fixture.diff_rows() < build_fixture.full_diff_rows());
        assert_eq!(fixture.syntax_mode(), DiffSyntaxMode::Auto);
    }

    #[test]
    fn conflict_two_way_fixture_wraps_start_offsets() {
        let fixture = ConflictTwoWaySplitScrollFixture::new(180, 18);
        let hash_a = fixture.run_scroll_step(17, 40);
        let hash_b = fixture.run_scroll_step(17 + fixture.visible_rows() * 3, 40);
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn conflict_two_way_diff_build_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictTwoWayDiffBuildFixture::new(120, 12);
        assert_eq!(fixture.conflict_count(), 12);
        assert!(fixture.full_diff_rows() >= 120);
        assert!(fixture.block_local_diff_rows() > 0);
    }

    #[test]
    fn conflict_two_way_diff_build_fixture_keeps_block_local_rows_sparse() {
        let fixture = ConflictTwoWayDiffBuildFixture::new(1_200, 12);
        assert!(
            fixture.block_local_diff_rows() < fixture.full_diff_rows(),
            "block-local rows should stay smaller than the full-file diff for sparse conflicts"
        );
    }

    #[test]
    fn conflict_two_way_diff_build_fixture_runs_build_and_highlight_paths() {
        let fixture = ConflictTwoWayDiffBuildFixture::new(240, 24);
        assert_ne!(fixture.run_full_diff_build_step(), 0);
        assert_ne!(fixture.run_block_local_diff_build_step(), 0);
        assert_ne!(fixture.run_full_word_highlights_step(), 0);
        assert_ne!(fixture.run_block_local_word_highlights_step(), 0);
    }

    #[test]
    fn conflict_load_duplication_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictLoadDuplicationFixture::new(1_200, 12);
        assert_eq!(fixture.line_count, 1_200);
        assert_eq!(fixture.conflict_count(), 12);
    }

    #[test]
    fn conflict_load_duplication_fixture_reuses_shared_payloads_only_in_shared_path() {
        let fixture = ConflictLoadDuplicationFixture::new(240, 24);
        let shared = fixture.build_shared_conflict_file();
        let duplicated = fixture.build_duplicated_conflict_file();

        let ConflictPayload::Text(base_payload) = &fixture.session.base else {
            panic!("synthetic conflict-load fixture should use text payloads");
        };
        let shared_base = shared
            .base
            .as_ref()
            .expect("shared conflict file should keep base text");
        let duplicated_base = duplicated
            .base
            .as_ref()
            .expect("duplicated conflict file should keep base text");

        assert!(Arc::ptr_eq(base_payload, shared_base));
        assert!(!Arc::ptr_eq(base_payload, duplicated_base));
        assert!(shared.base_bytes.is_none());
        assert!(duplicated.base_bytes.is_some());

        let shared_current = shared
            .current
            .as_ref()
            .expect("shared conflict file should keep current text");
        let duplicated_current = duplicated
            .current
            .as_ref()
            .expect("duplicated conflict file should keep current text");
        assert!(Arc::ptr_eq(&fixture.current_text, shared_current));
        assert!(!Arc::ptr_eq(&fixture.current_text, duplicated_current));
        assert!(shared.current_bytes.is_none());
        assert!(duplicated.current_bytes.is_some());
    }

    #[test]
    fn conflict_load_duplication_fixture_runs_shared_and_duplicated_paths() {
        let fixture = ConflictLoadDuplicationFixture::new(240, 24);
        assert_ne!(fixture.run_shared_payload_forwarding_step(), 0);
        assert_ne!(fixture.run_duplicated_payload_forwarding_step(), 0);
    }

    #[test]
    fn conflict_search_query_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictSearchQueryUpdateFixture::new(120, 12);
        assert_eq!(fixture.conflict_count(), 12);
        assert!(fixture.visible_rows() > 0);
        assert!(fixture.stable_cache_entries() > 0);
    }

    #[test]
    fn conflict_search_query_fixture_uses_block_local_rows_and_auto_syntax() {
        let fixture = ConflictSearchQueryUpdateFixture::new(1_200, 12);
        let build_fixture = ConflictTwoWayDiffBuildFixture::new(1_200, 12);
        assert_eq!(fixture.diff_rows(), build_fixture.block_local_diff_rows());
        assert!(fixture.diff_rows() < build_fixture.full_diff_rows());
        assert_eq!(fixture.syntax_mode(), DiffSyntaxMode::Auto);
    }

    #[test]
    fn conflict_search_query_fixture_reuses_stable_cache_across_queries() {
        let mut fixture = ConflictSearchQueryUpdateFixture::new(180, 18);
        let stable_before = fixture.stable_cache_entries();
        assert_eq!(fixture.query_cache_entries(), 0);

        let _ = fixture.run_query_update_step("conf", 5, 40);
        let first_query_cache = fixture.query_cache_entries();
        assert!(first_query_cache > 0);
        assert_eq!(fixture.stable_cache_entries(), stable_before);

        let _ = fixture.run_query_update_step("conflict", 5, 40);
        let second_query_cache = fixture.query_cache_entries();
        assert!(second_query_cache > 0);
        assert_eq!(fixture.stable_cache_entries(), stable_before);
    }

    #[test]
    fn conflict_search_query_fixture_wraps_start_offsets() {
        let mut fixture = ConflictSearchQueryUpdateFixture::new(180, 18);
        let hash_a = fixture.run_query_update_step("shared", 17, 40);
        let hash_b = fixture.run_query_update_step("shared", 17 + fixture.visible_rows() * 3, 40);
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn patch_diff_search_query_fixture_tracks_requested_rows() {
        let fixture = PatchDiffSearchQueryUpdateFixture::new(240);
        assert_eq!(fixture.visible_rows(), 240);
        assert!(fixture.stable_cache_entries() > 0);
        assert_eq!(fixture.query_cache_entries(), 0);
    }

    #[test]
    fn patch_diff_search_query_fixture_reuses_stable_cache_across_queries() {
        let mut fixture = PatchDiffSearchQueryUpdateFixture::new(360);
        let stable_before = fixture.stable_cache_entries();
        assert_eq!(fixture.query_cache_entries(), 0);

        let _ = fixture.run_query_update_step("shared", 20, 80);
        let stable_after_first = fixture.stable_cache_entries();
        let first_query_entries = fixture.query_cache_entries();
        assert!(first_query_entries > 0);
        assert!(stable_after_first >= stable_before);

        let _ = fixture.run_query_update_step("compute_shared", 20, 80);
        let stable_after_second = fixture.stable_cache_entries();
        let second_query_entries = fixture.query_cache_entries();
        assert!(second_query_entries > 0);
        assert_eq!(stable_after_second, stable_after_first);
    }

    #[test]
    fn patch_diff_search_query_fixture_wraps_start_offsets() {
        let mut fixture = PatchDiffSearchQueryUpdateFixture::new(420);
        let hash_a = fixture.run_query_update_step("shared", 31, 120);
        let hash_b = fixture.run_query_update_step("shared", 31 + fixture.visible_rows() * 2, 120);
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn patch_diff_paged_rows_fixture_builds_requested_line_count() {
        let fixture = PatchDiffPagedRowsFixture::new(1_024);
        assert!(fixture.total_rows() >= 1_024);
    }

    #[test]
    fn patch_diff_paged_rows_fixture_runs_eager_and_paged_paths() {
        let fixture = PatchDiffPagedRowsFixture::new(2_048);
        let eager = fixture.run_eager_full_materialize_step();
        let paged = fixture.run_paged_first_window_step(160);
        assert_ne!(eager, 0);
        assert_ne!(paged, 0);
    }

    #[test]
    fn patch_diff_paged_rows_fixture_inline_visible_map_matches_eager_scan() {
        let fixture = PatchDiffPagedRowsFixture::new(2_048);
        assert_eq!(
            fixture.inline_visible_indices_map(),
            fixture.inline_visible_indices_eager()
        );
    }

    #[test]
    fn patch_diff_paged_rows_fixture_runs_inline_visible_paths() {
        let fixture = PatchDiffPagedRowsFixture::new(2_048);
        let eager = fixture.run_inline_visible_eager_scan_step();
        let mapped = fixture.run_inline_visible_hidden_map_step();
        assert_ne!(eager, 0);
        assert_ne!(mapped, 0);
    }

    #[test]
    fn conflict_split_resize_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictSplitResizeStepFixture::new(120, 12);
        assert_eq!(fixture.conflict_count(), 12);
        assert!(fixture.visible_rows() > 0);
    }

    #[test]
    fn conflict_split_resize_fixture_reuses_caches_across_drag_steps() {
        let mut fixture = ConflictSplitResizeStepFixture::new(180, 18);
        let stable_before = fixture.stable_cache_entries();
        assert_eq!(fixture.query_cache_entries(), 0);

        let _ = fixture.run_resize_step("shared", 5, 40);
        let ratio_after_first = fixture.split_ratio();
        let first_query_cache = fixture.query_cache_entries();
        assert!(first_query_cache > 0);
        assert_eq!(fixture.stable_cache_entries(), stable_before);

        let _ = fixture.run_resize_step("shared", 25, 40);
        let ratio_after_second = fixture.split_ratio();
        let second_query_cache = fixture.query_cache_entries();
        assert!((ratio_after_first - ratio_after_second).abs() > f32::EPSILON);
        assert!(second_query_cache >= first_query_cache);
        assert_eq!(fixture.stable_cache_entries(), stable_before);
    }

    #[test]
    fn conflict_split_resize_fixture_clamps_ratio_bounds() {
        let mut fixture = ConflictSplitResizeStepFixture::new(180, 18);
        for _ in 0..400 {
            let _ = fixture.run_resize_step("shared", 0, 32);
            let ratio = fixture.split_ratio();
            assert!((0.1..=0.9).contains(&ratio));
        }
    }

    #[test]
    fn conflict_resolved_output_gutter_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictResolvedOutputGutterScrollFixture::new(120, 12);
        assert_eq!(fixture.conflict_count(), 12);
        assert!(fixture.visible_rows() > 0);
    }

    #[test]
    fn conflict_resolved_output_gutter_fixture_wraps_start_offsets() {
        let fixture = ConflictResolvedOutputGutterScrollFixture::new(180, 18);
        let hash_a = fixture.run_scroll_step(17, 40);
        let hash_b = fixture.run_scroll_step(17 + fixture.visible_rows() * 3, 40);
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn resolved_output_recompute_incremental_fixture_tracks_rows() {
        let fixture = ResolvedOutputRecomputeIncrementalFixture::new(240, 24);
        assert!(fixture.visible_rows() > 0);
    }

    #[test]
    fn resolved_output_recompute_incremental_fixture_runs_full_and_incremental_steps() {
        let mut fixture = ResolvedOutputRecomputeIncrementalFixture::new(240, 24);
        let full_hash = fixture.run_full_recompute_step();
        let incremental_hash = fixture.run_incremental_recompute_step();
        assert_ne!(full_hash, 0);
        assert_ne!(incremental_hash, 0);
    }

    #[test]
    fn branch_sidebar_fixture_scales_with_more_entries() {
        let small = BranchSidebarFixture::new(8, 16, 2, 0, 0, 0);
        let large = BranchSidebarFixture::new(120, 600, 6, 40, 40, 80);
        assert!(small.row_count() > 0);
        assert!(large.row_count() > small.row_count());
    }

    #[test]
    fn history_graph_fixture_preserves_requested_commit_count() {
        let fixture = HistoryGraphFixture::new(2_000, 7, 9);
        assert_eq!(fixture.commit_count(), 2_000);
        assert_ne!(fixture.run(), 0);
    }

    #[test]
    fn synthetic_source_lines_honor_requested_min_line_bytes() {
        let lines = build_synthetic_source_lines(64, 512);
        assert_eq!(lines.len(), 64);
        assert!(lines.iter().all(|line| line.len() >= 512));
    }

    #[test]
    fn large_file_fixture_handles_very_long_lines() {
        let fixture = LargeFileDiffScrollFixture::new_with_line_bytes(512, 4_096);
        assert_ne!(fixture.run_scroll_step(0, 64), 0);
    }

    #[test]
    fn text_input_prepaint_windowed_fixture_wraps_start_offsets() {
        let mut fixture = TextInputPrepaintWindowedFixture::new(512, 96, 640);
        let hash_a = fixture.run_windowed_step(17, 48);
        let hash_b = fixture.run_windowed_step(17 + fixture.total_rows() * 3, 48);
        assert_eq!(hash_a, hash_b);
        assert!(fixture.cache_entries() > 0);
    }

    #[test]
    fn text_input_runs_streamed_highlight_fixture_matches_legacy_dense() {
        let fixture = TextInputRunsStreamedHighlightFixture::new(
            512,
            112,
            96,
            TextInputHighlightDensity::Dense,
        );
        assert!(fixture.highlights_len() > 0);

        let mut start = 0usize;
        for _ in 0..8 {
            let legacy = fixture.run_legacy_step(start);
            let streamed = fixture.run_streamed_step(start);
            assert_eq!(legacy, streamed);
            start = fixture.next_start_row(start);
        }
    }

    #[test]
    fn text_input_runs_streamed_highlight_fixture_matches_legacy_sparse() {
        let fixture = TextInputRunsStreamedHighlightFixture::new(
            512,
            112,
            96,
            TextInputHighlightDensity::Sparse,
        );
        assert!(fixture.highlights_len() > 0);

        let mut start = 0usize;
        for _ in 0..8 {
            let legacy = fixture.run_legacy_step(start);
            let streamed = fixture.run_streamed_step(start);
            assert_eq!(legacy, streamed);
            start = fixture.next_start_row(start);
        }
    }

    #[test]
    fn text_input_long_line_cap_fixture_bounds_shaping_slice() {
        let fixture = TextInputLongLineCapFixture::new(128 * 1024);
        let capped_len = fixture.capped_len(4 * 1024);
        let uncapped_len = fixture.capped_len(256 * 1024);
        assert!(capped_len < uncapped_len);
        assert_ne!(fixture.run_with_cap(4 * 1024), 0);
        assert_ne!(fixture.run_without_cap(), 0);
    }

    #[test]
    fn text_input_wrap_incremental_tabs_fixture_matches_full_recompute() {
        let mut full = TextInputWrapIncrementalTabsFixture::new(512, 96, 680);
        let mut incremental = TextInputWrapIncrementalTabsFixture::new(512, 96, 680);
        for step in 0..48usize {
            let line_ix = step.wrapping_mul(17);
            let full_hash = full.run_full_recompute_step(line_ix);
            let incremental_hash = incremental.run_incremental_step(line_ix);
            assert_eq!(full_hash, incremental_hash);
        }
        assert_eq!(full.row_counts(), incremental.row_counts());
    }

    #[test]
    fn text_input_wrap_incremental_burst_fixture_matches_full_recompute() {
        let mut full = TextInputWrapIncrementalBurstEditsFixture::new(768, 112, 720);
        let mut incremental = TextInputWrapIncrementalBurstEditsFixture::new(768, 112, 720);
        for burst in [1usize, 3, 6, 9, 12] {
            let full_hash = full.run_full_recompute_burst_step(burst);
            let incremental_hash = incremental.run_incremental_burst_step(burst);
            assert_eq!(full_hash, incremental_hash);
        }
        assert_eq!(full.row_counts(), incremental.row_counts());
    }

    #[test]
    fn text_model_snapshot_clone_fixture_runs_model_and_string_control_paths() {
        let fixture = TextModelSnapshotCloneCostFixture::new(512 * 1024);
        let model_hash = fixture.run_snapshot_clone_step(2_048);
        let string_hash = fixture.run_string_clone_control_step(2_048);
        assert_ne!(model_hash, 0);
        assert_ne!(string_hash, 0);
    }

    #[test]
    fn text_model_bulk_load_fixture_runs_piece_table_and_control_paths() {
        let fixture = TextModelBulkLoadLargeFixture::new(4_096, 96);
        let piece_table_hash = fixture.run_piece_table_bulk_load_step();
        let piece_table_from_large_hash = fixture.run_piece_table_from_large_text_step();
        let control_hash = fixture.run_string_bulk_load_control_step();
        assert_ne!(piece_table_hash, 0);
        assert_ne!(piece_table_from_large_hash, 0);
        assert_ne!(control_hash, 0);
    }

    #[test]
    fn nested_query_stress_source_lines_honor_requested_min_line_bytes() {
        let lines = build_synthetic_nested_query_stress_lines(32, 2_048, 64);
        assert_eq!(lines.len(), 32);
        assert!(lines.iter().all(|line| line.len() >= 2_048));
        assert!(lines.iter().all(|line| line.contains("nested")));
    }

    #[test]
    fn file_diff_syntax_stress_fixture_has_bounded_latency_distribution() {
        let fixture = FileDiffSyntaxPrepareFixture::new_query_stress(64, 1_536, 96);
        let mut samples = Vec::new();
        for nonce in 0..10u64 {
            let start = std::time::Instant::now();
            let _ = fixture.run_prepare_cold(nonce);
            samples.push(start.elapsed().as_secs_f64());
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        let median = samples[samples.len() / 2].max(f64::EPSILON);
        let p95 = samples[samples.len() - 1];
        assert!(
            p95 <= median * 12.0,
            "query stress latency distribution widened too far: median={median:.6}s p95={p95:.6}s"
        );
    }

    #[test]
    fn file_diff_syntax_reparse_fixture_runs_small_and_large_edit_steps() {
        let mut fixture = FileDiffSyntaxReparseFixture::new(512, 128);
        let small = fixture.run_small_edit_step();
        let large = fixture.run_large_edit_step();
        assert_ne!(small, 0);
        assert_ne!(large, 0);
    }

    #[test]
    fn file_diff_syntax_cache_drop_fixture_runs_both_modes() {
        let fixture = FileDiffSyntaxCacheDropFixture::new(1_024, 8, 4);
        assert_ne!(fixture.run_deferred_drop_step(), 0);
        assert_ne!(fixture.run_inline_drop_control_step(), 0);
    }

    #[test]
    fn file_diff_inline_syntax_projection_fixture_runs_pending_and_ready_windows() {
        let fixture = FileDiffInlineSyntaxProjectionFixture::new(384, 96);
        assert!(fixture.visible_rows() > 0);
        assert_ne!(fixture.run_window_pending_step(0, 64), 0);
        assert_ne!(fixture.run_window_step(0, 64), 0);
    }

    #[test]
    fn file_diff_inline_syntax_projection_fixture_wraps_start_offsets() {
        let fixture = FileDiffInlineSyntaxProjectionFixture::new(512, 128);
        let hash_a = fixture.run_window_step(17, 48);
        let hash_b = fixture.run_window_step(17 + fixture.visible_rows() * 3, 48);
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn prepared_syntax_multidoc_cache_hit_rate_fixture_runs() {
        let fixture = FileDiffSyntaxPrepareFixture::new(512, 96);
        let hash = fixture.run_prepared_syntax_multidoc_cache_hit_rate_step(4, 1);
        assert_ne!(hash, 0);
    }

    #[test]
    fn prepared_syntax_chunk_miss_cost_fixture_runs() {
        let fixture = FileDiffSyntaxPrepareFixture::new(1_024, 96);
        let elapsed = fixture.run_prepared_syntax_chunk_miss_cost_step(1);
        assert!(elapsed >= Duration::ZERO);
    }

    #[test]
    fn file_diff_syntax_prepare_fixture_keeps_prepared_document_for_large_documents() {
        let fixture = FileDiffSyntaxPrepareFixture::new(4_001, 96);
        let prepared = fixture.prepare_document(&fixture.lines);

        assert!(
            prepared.is_some(),
            "file diff syntax prepare should stay enabled above the old 4,000-line gate"
        );
        assert_ne!(fixture.run_prepare_warm(), 0);
    }

    #[test]
    fn large_html_syntax_fixture_synthetic_fallback_runs() {
        let prepare_fixture = LargeHtmlSyntaxFixture::new(None, 128, 160);
        let visible_fixture = LargeHtmlSyntaxFixture::new_prewarmed(None, 128, 160);

        assert_eq!(prepare_fixture.source(), LargeHtmlSyntaxSource::Synthetic);
        assert_eq!(visible_fixture.source(), LargeHtmlSyntaxSource::Synthetic);
        assert_eq!(visible_fixture.line_count(), 128);
        assert_ne!(prepare_fixture.run_background_prepare_step(), 0);

        visible_fixture.prime_visible_window(48);
        assert_ne!(visible_fixture.run_visible_window_step(0, 48), 0);
    }

    #[test]
    fn large_html_syntax_fixture_pending_window_is_nonblocking_until_primed() {
        let path = std::env::temp_dir().join(format!(
            "gitcomet-large-html-pending-bench-{}.html",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        let text = (0..96)
            .map(|ix| {
                format!(
                    "<div class=\"row-{ix}\" style=\"color: red\" onclick=\"const value = {ix};\">row {ix}</div>"
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, text).expect("temp html fixture should be writable");

        let fixture = LargeHtmlSyntaxFixture::new_prewarmed(path.to_str(), 64, 128);
        let document = fixture
            .prepared_document_handle()
            .expect("HTML fixture should prepare a document");

        let first = fixture
            .request_visible_window_for_lines(document, 0, 48)
            .expect("cold visible-window request should return fallback highlights");
        assert!(
            first.pending,
            "cold request should stay nonblocking until chunk work completes"
        );

        assert_ne!(fixture.run_visible_window_pending_step(0, 48), 0);

        let started = std::time::Instant::now();
        let mut second = fixture
            .request_visible_window_for_lines(document, 0, 48)
            .expect("second visible-window request should still succeed");
        while second.pending && started.elapsed() < Duration::from_secs(2) {
            if drain_completed_prepared_diff_syntax_chunk_builds_for_document(document) == 0 {
                std::thread::sleep(Duration::from_millis(5));
            }
            second = fixture
                .request_visible_window_for_lines(document, 0, 48)
                .expect("ready visible-window request should still succeed");
        }
        assert!(
            !second.pending,
            "drained request should return ready prepared-document highlights"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn large_html_syntax_fixture_keeps_prepared_document_for_large_documents() {
        let fixture = LargeHtmlSyntaxFixture::new_prewarmed(None, 4_001, 192);

        assert_eq!(fixture.source(), LargeHtmlSyntaxSource::Synthetic);
        assert!(
            fixture.prepared_document_handle().is_some(),
            "large HTML fixture should still produce a prepared document above the old 4,000-line gate"
        );

        fixture.prime_visible_window(96);
        assert_ne!(fixture.run_visible_window_step(128, 96), 0);
    }

    #[test]
    fn large_html_syntax_fixture_uses_external_text_when_available() {
        let path = std::env::temp_dir().join(format!(
            "gitcomet-large-html-bench-{}.html",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        let text = [
            "<!doctype html>",
            "<html>",
            "<body>",
            "<div style=\"color: red\" onclick=\"const value = 1;\">hi</div>",
            "</body>",
            "</html>",
        ]
        .join("\n");
        std::fs::write(&path, text).expect("temp html fixture should be writable");

        let fixture = LargeHtmlSyntaxFixture::new_prewarmed(path.to_str(), 64, 128);
        assert_eq!(fixture.source(), LargeHtmlSyntaxSource::External);
        assert_eq!(fixture.line_count(), 6);
        fixture.prime_visible_window(6);
        assert_ne!(fixture.run_visible_window_step(0, 6), 0);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn worktree_preview_render_fixture_preserves_output_with_cached_lookup() {
        let fixture = WorktreePreviewRenderFixture::new(1_024, 128);
        let cached = fixture.run_cached_lookup_step(96, 160);
        let render_time_prepare = fixture.run_render_time_prepare_step(96, 160);
        assert_eq!(cached, render_time_prepare);
    }

    #[test]
    fn worktree_preview_render_fixture_keeps_auto_mode_for_large_documents() {
        let fixture = WorktreePreviewRenderFixture::new(8_192, 128);
        assert_eq!(fixture.syntax_mode, DiffSyntaxMode::Auto);
        assert!(fixture.prepared_document.is_some());
    }

    #[test]
    fn worktree_preview_render_fixture_handles_long_windows() {
        let fixture = WorktreePreviewRenderFixture::new(2_048, 192);
        assert_ne!(fixture.run_cached_lookup_step(0, 256), 0);
        assert_ne!(fixture.run_render_time_prepare_step(0, 256), 0);
    }

    #[test]
    fn markdown_preview_fixture_runs_parse_steps() {
        let fixture = MarkdownPreviewFixture::new(64, 96);
        assert_ne!(fixture.run_parse_single_step(), 0);
        assert_ne!(fixture.run_parse_diff_step(), 0);
    }

    #[test]
    fn markdown_preview_fixture_runs_render_steps() {
        let fixture = MarkdownPreviewFixture::new(96, 112);
        assert_ne!(fixture.run_render_single_step(24, 64), 0);
        assert_ne!(fixture.run_render_diff_step(24, 64), 0);
    }

    #[test]
    fn streamed_provider_fixture_builds_with_expected_row_counts() {
        let fixture = ConflictStreamedProviderFixture::new(1_000);
        // Total rows should cover all lines (ours + theirs via anchor mapping).
        assert!(fixture.total_rows() > 0);
        assert!(fixture.visible_rows() > 0);
        assert_eq!(fixture.total_rows(), fixture.visible_rows());
    }

    #[test]
    fn streamed_provider_fixture_generates_rows_at_all_positions() {
        let fixture = ConflictStreamedProviderFixture::new(200);
        assert_ne!(fixture.run_first_page_step(40), 0);
        assert_ne!(fixture.run_deep_scroll_step(0.5, 40), 0);
        assert_ne!(fixture.run_deep_scroll_step(0.9, 40), 0);
    }

    #[test]
    fn streamed_provider_fixture_search_finds_known_patterns() {
        let fixture = ConflictStreamedProviderFixture::new(500);
        // "shared_" lines exist in both ours and theirs.
        let h = fixture.run_search_step("shared_");
        assert_ne!(h, 0, "search should find shared lines");
        // "ours_only_" lines exist only in ours.
        let h = fixture.run_search_step("ours_only_");
        assert_ne!(h, 0, "search should find ours-only lines");
    }

    #[test]
    fn streamed_provider_fixture_index_build_is_deterministic() {
        let fixture = ConflictStreamedProviderFixture::new(500);
        let h1 = fixture.run_index_build_step();
        let h2 = fixture.run_index_build_step();
        assert_eq!(h1, h2, "index build should be deterministic");
    }

    #[test]
    fn streamed_provider_fixture_has_anchors_for_shared_content() {
        let fixture = ConflictStreamedProviderFixture::new(1_000);
        let anchors = fixture.anchor_count();
        // ~60% of lines are shared, so we should have a meaningful anchor set.
        assert!(
            anchors > 0,
            "whole-file conflict with shared content should produce anchors"
        );
    }

    #[test]
    fn streamed_provider_fixture_projection_build_is_deterministic() {
        let fixture = ConflictStreamedProviderFixture::new(500);
        let h1 = fixture.run_projection_build_step();
        let h2 = fixture.run_projection_build_step();
        assert_eq!(h1, h2, "projection build should be deterministic");
    }

    #[test]
    fn streamed_provider_fixture_reuses_first_page_cache() {
        let fixture = ConflictStreamedProviderFixture::new(1_000);
        fixture.prime_first_page_cache(160);
        assert_ne!(fixture.run_first_page_cache_hit_step(160), 0);
        assert_eq!(
            fixture.cached_page_count(),
            1,
            "cache-hit benchmark should keep the warmed first page resident"
        );
    }

    /// Phase 8 RSS invariant: streamed provider metadata is much smaller than
    /// what eager mode would allocate.  Verifies that memory scales with
    /// metadata (anchors, spans, line starts) not with rendered rows.
    #[test]
    fn streamed_provider_metadata_is_sublinear_in_line_count() {
        let small_lines = 2_000;
        let large_lines = 20_000;
        let small = ConflictStreamedProviderFixture::new(small_lines);
        let large = ConflictStreamedProviderFixture::new(large_lines);

        let small_meta = small.metadata_byte_size();
        let large_meta = large.metadata_byte_size();

        // At 10x the line count, metadata should grow less than 10x.
        // The split row index stores per-line-start data (O(N)) but NOT
        // per-rendered-row data; the projection stores O(segments) spans.
        let growth_ratio = large_meta as f64 / small_meta.max(1) as f64;
        assert!(
            growth_ratio < 12.0,
            "metadata should grow sublinearly vs eager rows: \
             small({small_lines})={small_meta}B, large({large_lines})={large_meta}B, \
             ratio={growth_ratio:.1}x (expected <12x for 10x line count)"
        );

        // The large fixture's metadata should be much smaller than the equivalent
        // eager allocation: N rows * ~100 bytes/FileDiffRow.
        let eager_estimate = large.total_rows() * 100;
        assert!(
            large_meta < eager_estimate / 2,
            "streamed metadata ({large_meta}B) should be well under eager estimate ({eager_estimate}B)"
        );
    }

    /// Phase 8 RSS invariant: page cache stays bounded regardless of how many
    /// distinct positions are accessed.
    #[test]
    fn streamed_provider_page_cache_stays_bounded() {
        let fixture = ConflictStreamedProviderFixture::new(10_000);
        let visible_len = fixture.visible_rows();

        // Access pages at many distinct positions.
        for pct in [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95, 0.99] {
            let _ = fixture.run_deep_scroll_step(pct, 200);
        }

        let cached = fixture.cached_page_count();
        // CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES = 8.
        assert!(
            cached <= 8,
            "page cache should be bounded at 8 pages, got {cached} after scrolling \
             through {visible_len} visible rows"
        );
    }

    #[test]
    fn streamed_resolved_output_fixture_builds_visible_rows() {
        let fixture = ConflictStreamedResolvedOutputFixture::new(1_000, 200);
        assert!(fixture.visible_rows() > 0);
    }

    #[test]
    fn streamed_resolved_output_fixture_generates_windows_at_all_positions() {
        let fixture = ConflictStreamedResolvedOutputFixture::new(2_000, 300);
        assert_ne!(fixture.run_window_step(160), 0);
        assert_ne!(fixture.run_deep_window_step(0.5, 160), 0);
        assert_ne!(fixture.run_deep_window_step(0.9, 160), 0);
    }

    #[test]
    fn streamed_resolved_output_fixture_projection_build_is_deterministic() {
        let fixture = ConflictStreamedResolvedOutputFixture::new(2_000, 300);
        let first = fixture.run_projection_build_step();
        let second = fixture.run_projection_build_step();
        assert_eq!(first, second);
    }

    #[test]
    fn streamed_resolved_output_metadata_stays_compact() {
        let small_lines = 2_000;
        let large_lines = 20_000;
        let small = ConflictStreamedResolvedOutputFixture::new(small_lines, 200);
        let large = ConflictStreamedResolvedOutputFixture::new(large_lines, 2_000);

        let small_meta = small.metadata_byte_size();
        let large_meta = large.metadata_byte_size();
        let growth_ratio = large_meta as f64 / small_meta.max(1) as f64;
        assert!(
            growth_ratio < 12.0,
            "streamed resolved-output metadata should scale with spans/line starts, not rendered rows: \
             small({small_lines})={small_meta}B large({large_lines})={large_meta}B \
             ratio={growth_ratio:.1}x"
        );

        let materialized_len = conflict_resolver::generate_resolved_text(&large.segments)
            .len()
            .max(1);
        assert!(
            large_meta < materialized_len,
            "streamed resolved-output metadata ({large_meta}B) should stay below \
             the materialized output size ({materialized_len}B)"
        );
    }
}
