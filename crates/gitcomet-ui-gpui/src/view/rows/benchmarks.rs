use super::diff_text::{
    DiffSyntaxBudget, DiffSyntaxLanguage, DiffSyntaxMode, PrepareDiffSyntaxDocumentResult,
    diff_syntax_language_for_path, inject_background_prepared_diff_syntax_document,
    prepare_diff_syntax_document_with_budget_reuse_text,
};
use super::*;
use crate::kit::text_model::TextModel;
use crate::kit::{
    benchmark_text_input_runs_legacy_visible_window,
    benchmark_text_input_runs_streamed_visible_window,
};
use crate::theme::AppTheme;
use crate::view::history_graph;
use crate::view::panes::main::diff_cache::{
    PagedPatchDiffRows, PagedPatchSplitRows, PatchInlineVisibleMap,
};
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::domain::{
    Branch, Commit, CommitDetails, CommitFileChange, CommitId, Diff, DiffArea, DiffRowProvider,
    DiffTarget, FileStatusKind, Remote, RemoteBranch, RepoSpec, StashEntry, Submodule,
    SubmoduleStatus, Upstream, UpstreamDivergence, Worktree,
};
use gitcomet_state::model::{Loadable, RepoId, RepoState};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

mod conflict;
mod syntax;

pub use conflict::*;
pub use syntax::*;

#[cfg(test)]
mod tests;

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
        let graph = history_graph::compute_graph(&self.commits, self.theme, &branch_heads, None);

        let mut h = FxHasher::default();
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
        let mut h = FxHasher::default();
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

        let graph = history_graph::compute_graph(&self.commits, self.theme, &branch_heads, None);
        let mut h = FxHasher::default();
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
        let mut h = FxHasher::default();
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
        let mut h = FxHasher::default();
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
        let mut h = FxHasher::default();

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
                let mut shaped_hash = FxHasher::default();
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
        let mut h = FxHasher::default();
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
        let mut h = FxHasher::default();
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

        let mut h = FxHasher::default();
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
        let mut h = FxHasher::default();
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
        let mut h = FxHasher::default();
        loaded.len().hash(&mut h);
        loaded.bytes().take(96).count().hash(&mut h);
        h.finish()
    }
}

pub struct TextModelFragmentedEditFixture {
    /// The initial document text, used to build fresh models per iteration.
    initial_text: String,
    /// Pre-computed edit sequence: (byte_offset, delete_len, insert_text).
    edits: Vec<(usize, usize, String)>,
}

impl TextModelFragmentedEditFixture {
    pub fn new(min_bytes: usize, edit_count: usize) -> Self {
        let initial_text = build_text_model_document(min_bytes.max(1024));
        let doc_len = initial_text.len();
        let edits = build_deterministic_edits(&initial_text, doc_len, edit_count.max(1));
        Self {
            initial_text,
            edits,
        }
    }

    /// Benchmark: apply all edits to a fresh piece-table model.
    pub fn run_fragmented_edit_step(&self) -> u64 {
        let mut model = TextModel::from_large_text(&self.initial_text);
        let mut h = FxHasher::default();
        for (offset, delete_len, insert) in &self.edits {
            let end = offset.saturating_add(*delete_len).min(model.len());
            let start = (*offset).min(model.len());
            let _ = model.replace_range(start..end, insert);
        }
        model.len().hash(&mut h);
        model.revision().hash(&mut h);
        h.finish()
    }

    /// Benchmark: apply all edits, then materialize via `as_str()`.
    pub fn run_materialize_after_edits_step(&self) -> u64 {
        let mut model = TextModel::from_large_text(&self.initial_text);
        for (offset, delete_len, insert) in &self.edits {
            let end = offset.saturating_add(*delete_len).min(model.len());
            let start = (*offset).min(model.len());
            let _ = model.replace_range(start..end, insert);
        }
        let text = model.as_str();
        let mut h = FxHasher::default();
        text.len().hash(&mut h);
        text.bytes().take(128).count().hash(&mut h);
        h.finish()
    }

    /// Benchmark: apply all edits, then call `as_shared_string()` repeatedly.
    pub fn run_shared_string_after_edits_step(&self, reads: usize) -> u64 {
        let mut model = TextModel::from_large_text(&self.initial_text);
        for (offset, delete_len, insert) in &self.edits {
            let end = offset.saturating_add(*delete_len).min(model.len());
            let start = (*offset).min(model.len());
            let _ = model.replace_range(start..end, insert);
        }
        let mut h = FxHasher::default();
        for nonce in 0..reads.max(1) {
            let ss = model.as_shared_string();
            nonce.hash(&mut h);
            ss.len().hash(&mut h);
        }
        h.finish()
    }

    /// Control: apply the same edits to a plain `String`.
    pub fn run_string_edit_control_step(&self) -> u64 {
        let mut text = self.initial_text.clone();
        let mut h = FxHasher::default();
        for (offset, delete_len, insert) in &self.edits {
            let start = (*offset).min(text.len());
            let end = offset.saturating_add(*delete_len).min(text.len());
            text.replace_range(start..end, insert);
        }
        text.len().hash(&mut h);
        text.bytes().take(128).count().hash(&mut h);
        h.finish()
    }
}

/// Build a deterministic pseudo-random edit sequence that stays within document bounds.
fn build_deterministic_edits(
    text: &str,
    initial_len: usize,
    count: usize,
) -> Vec<(usize, usize, String)> {
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
        let insert = match ix % 5 {
            0 => format!("edit_{ix}"),
            1 => format!("fn f{ix}() {{ }}\n"),
            2 => String::new(), // pure delete
            3 => format!("/* {ix} */"),
            _ => format!("x{ix}\ny{ix}\n"),
        };
        approx_len = approx_len
            .saturating_sub(delete_len.min(approx_len.saturating_sub(offset)))
            .saturating_add(insert.len());
        edits.push((offset, delete_len, insert));
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
    let mut h = FxHasher::default();
    row_counts.len().hash(&mut h);
    for rows in row_counts.iter().take(512) {
        rows.hash(&mut h);
    }
    h.finish()
}

fn hash_text_input_shaping_slice(text: &str, max_bytes: usize) -> (u64, usize) {
    if text.len() <= max_bytes {
        let mut hasher = FxHasher::default();
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

    let mut hasher = FxHasher::default();
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
        let split_provider = PagedPatchSplitRows::new(Arc::clone(&rows_provider));
        let theme = AppTheme::zed_ayu_dark();
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

        let mut hasher = FxHasher::default();
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
        .unwrap_or_else(|| CommitId("0".repeat(40).into()));

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
            head: CommitId(format!("{:040x}", 200_000usize.saturating_add(ix)).into()),
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
            id: CommitId(format!("{:040x}", 300_000usize.saturating_add(ix)).into()),
            message: format!("WIP synthetic stash #{ix}").into(),
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
        let id = CommitId(format!("{:040x}", ix).into());

        let mut parent_ids = Vec::new();
        if ix > 0 {
            parent_ids.push(CommitId(format!("{:040x}", ix - 1).into()));
        }
        // Synthetic merge-like commits at a fixed cadence.
        if merge_every > 0
            && merge_back_distance > 0
            && ix >= merge_back_distance
            && ix % merge_every == 0
        {
            parent_ids.push(CommitId(
                format!("{:040x}", ix - merge_back_distance).into(),
            ));
        }

        commits.push(Commit {
            id,
            parent_ids,
            summary: format!("Commit {ix} - synthetic benchmark history entry").into(),
            author: format!("Author {}", ix % 10).into(),
            time: base + Duration::from_secs(ix as u64),
        });
    }

    commits
}

fn build_synthetic_commit_details(files: usize, depth: usize) -> CommitDetails {
    let id = CommitId("d".repeat(40).into());
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
        parent_ids: vec![CommitId("c".repeat(40).into())],
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

fn hash_file_diff_plan(plan: &gitcomet_core::file_diff::FileDiffPlan) -> u64 {
    let mut h = FxHasher::default();
    plan.row_count.hash(&mut h);
    plan.inline_row_count.hash(&mut h);
    match plan.eof_newline {
        Some(gitcomet_core::file_diff::FileDiffEofNewline::MissingInOld) => 1u8,
        Some(gitcomet_core::file_diff::FileDiffEofNewline::MissingInNew) => 2u8,
        None => 0u8,
    }
    .hash(&mut h);
    plan.runs.len().hash(&mut h);
    for run in plan.runs.iter().take(256) {
        std::mem::discriminant(run).hash(&mut h);
        match run {
            gitcomet_core::file_diff::FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            } => {
                old_start.hash(&mut h);
                new_start.hash(&mut h);
                len.hash(&mut h);
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, len } => {
                old_start.hash(&mut h);
                len.hash(&mut h);
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, len } => {
                new_start.hash(&mut h);
                len.hash(&mut h);
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                old_start.hash(&mut h);
                new_start.hash(&mut h);
                len.hash(&mut h);
            }
        }
    }
    h.finish()
}

fn build_synthetic_replacement_alignment_documents(
    blocks: usize,
    old_block_lines: usize,
    new_block_lines: usize,
    context_lines: usize,
    target_line_bytes: usize,
) -> (String, String) {
    let blocks = blocks.max(1);
    let old_block_lines = old_block_lines.max(1);
    let new_block_lines = new_block_lines.max(1);
    let context_lines = context_lines.max(1);
    let target_line_bytes = target_line_bytes.max(80);

    let mut old_lines = Vec::new();
    let mut new_lines = Vec::new();
    old_lines.push("fn replacement_alignment_fixture() {".to_string());
    new_lines.push("fn replacement_alignment_fixture() {".to_string());

    for block_ix in 0..blocks {
        for context_ix in 0..context_lines {
            let line =
                build_synthetic_replacement_context_line(block_ix, context_ix, target_line_bytes);
            old_lines.push(line.clone());
            new_lines.push(line);
        }

        for line_ix in 0..old_block_lines {
            old_lines.push(build_synthetic_replacement_change_line(
                block_ix,
                line_ix,
                old_block_lines,
                target_line_bytes,
                "before",
            ));
        }
        for line_ix in 0..new_block_lines {
            new_lines.push(build_synthetic_replacement_change_line(
                block_ix,
                line_ix,
                new_block_lines,
                target_line_bytes,
                "after",
            ));
        }
    }

    old_lines.push("}".to_string());
    new_lines.push("}".to_string());

    let mut old_text = old_lines.join("\n");
    old_text.push('\n');
    let mut new_text = new_lines.join("\n");
    new_text.push('\n');
    (old_text, new_text)
}

fn build_synthetic_replacement_context_line(
    block_ix: usize,
    context_ix: usize,
    target_line_bytes: usize,
) -> String {
    let mut line = format!(
        "    let context_{block_ix:03}_{context_ix:03} = stable_anchor(block_{block_ix:03}, {context_ix});"
    );
    if line.len() < target_line_bytes {
        line.push(' ');
        line.push_str("//");
        while line.len() < target_line_bytes {
            line.push_str(" keep_anchor");
        }
    }
    line
}

fn build_synthetic_replacement_change_line(
    block_ix: usize,
    line_ix: usize,
    block_lines: usize,
    target_line_bytes: usize,
    variant: &str,
) -> String {
    let logical_span = block_lines.max(1);
    let rotated_ix = (line_ix + (block_ix % 7) + 1) % logical_span;
    let logical_ix = if variant == "before" {
        line_ix
    } else {
        rotated_ix
    };

    let mut line = format!(
        "    let block_{block_ix:03}_slot_{logical_ix:03} = reconcile_entry(namespace::{variant}_source_{logical_ix:03}, synth_payload(block_{block_ix:03}, {logical_ix}), \"shared-payload-{block_ix:03}-{logical_ix:03}\");"
    );
    if line.len() < target_line_bytes {
        line.push(' ');
        line.push_str("//");
        while line.len() < target_line_bytes {
            if variant == "before" {
                line.push_str(" before_token");
            } else {
                line.push_str(" after_token");
            }
        }
    }
    line
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
