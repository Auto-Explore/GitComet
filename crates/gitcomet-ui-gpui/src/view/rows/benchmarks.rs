use super::diff_text::{DiffSyntaxMode, diff_syntax_language_for_path};
use super::*;
use crate::theme::AppTheme;
use crate::view::conflict_resolver::{
    self, ConflictBlock, ConflictChoice, ConflictSegment, ThreeWayVisibleItem,
};
use crate::view::history_graph;
use gitcomet_core::domain::{
    Branch, Commit, CommitDetails, CommitFileChange, CommitId, FileStatusKind, Remote,
    RemoteBranch, RepoSpec, StashEntry, Submodule, Upstream, UpstreamDivergence, Worktree,
};
use gitcomet_state::model::{Loadable, RepoId, RepoState};
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

pub struct ConflictThreeWayScrollFixture {
    base_lines: Vec<SharedString>,
    ours_lines: Vec<SharedString>,
    theirs_lines: Vec<SharedString>,
    base_word_highlights: conflict_resolver::WordHighlights,
    ours_word_highlights: conflict_resolver::WordHighlights,
    theirs_word_highlights: conflict_resolver::WordHighlights,
    base_line_conflict_map: Vec<Option<usize>>,
    ours_line_conflict_map: Vec<Option<usize>>,
    theirs_line_conflict_map: Vec<Option<usize>>,
    visible_map: Vec<ThreeWayVisibleItem>,
    conflict_count: usize,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    theme: AppTheme,
}

impl ConflictThreeWayScrollFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let theme = AppTheme::zed_ayu_dark();
        let segments = build_synthetic_three_way_segments(lines, conflict_blocks);
        let (base_text, ours_text, theirs_text) = materialize_three_way_side_texts(&segments);
        let base_lines = split_lines_shared(&base_text);
        let ours_lines = split_lines_shared(&ours_text);
        let theirs_lines = split_lines_shared(&theirs_text);
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
            &conflict_maps.conflict_ranges,
            &segments,
            false,
        );
        let (base_word_highlights, ours_word_highlights, theirs_word_highlights) =
            conflict_resolver::compute_three_way_word_highlights(
                &base_lines,
                &ours_lines,
                &theirs_lines,
                &segments,
            );
        let syntax_mode = if three_way_len > 4_000 {
            DiffSyntaxMode::HeuristicOnly
        } else {
            DiffSyntaxMode::Auto
        };

        Self {
            base_lines,
            ours_lines,
            theirs_lines,
            base_word_highlights,
            ours_word_highlights,
            theirs_word_highlights,
            base_line_conflict_map: conflict_maps.base_line_conflict_map,
            ours_line_conflict_map: conflict_maps.ours_line_conflict_map,
            theirs_line_conflict_map: conflict_maps.theirs_line_conflict_map,
            visible_map,
            conflict_count: conflict_maps.conflict_ranges.len(),
            language: diff_syntax_language_for_path("src/conflict.rs"),
            syntax_mode,
            theme,
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

            self.base_line_conflict_map
                .get(line_ix)
                .copied()
                .flatten()
                .hash(&mut h);
            self.ours_line_conflict_map
                .get(line_ix)
                .copied()
                .flatten()
                .hash(&mut h);
            self.theirs_line_conflict_map
                .get(line_ix)
                .copied()
                .flatten()
                .hash(&mut h);

            if let Some(line) = self.base_lines.get(line_ix) {
                let styled = super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    line.as_ref(),
                    word_ranges_for_line(&self.base_word_highlights, line_ix),
                    "",
                    self.language,
                    self.syntax_mode,
                    None,
                );
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }
            if let Some(line) = self.ours_lines.get(line_ix) {
                let styled = super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    line.as_ref(),
                    word_ranges_for_line(&self.ours_word_highlights, line_ix),
                    "",
                    self.language,
                    self.syntax_mode,
                    None,
                );
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }
            if let Some(line) = self.theirs_lines.get(line_ix) {
                let styled = super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    line.as_ref(),
                    word_ranges_for_line(&self.theirs_word_highlights, line_ix),
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
        self.visible_map.len()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_count
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

impl ConflictTwoWaySplitScrollFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let theme = AppTheme::zed_ayu_dark();
        let segments = build_synthetic_two_way_segments(lines, conflict_blocks);
        let (ours_text, theirs_text) = materialize_two_way_side_texts(&segments);
        let diff_rows = gitcomet_core::file_diff::side_by_side_rows(&ours_text, &theirs_text);
        let inline_rows = conflict_resolver::build_inline_rows(&diff_rows);
        let (diff_row_conflict_map, _) =
            conflict_resolver::map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);
        let visible_row_indices = conflict_resolver::build_two_way_visible_indices(
            &diff_row_conflict_map,
            &segments,
            false,
        );
        let diff_word_highlights_split =
            conflict_resolver::compute_two_way_word_highlights(&diff_rows);
        let syntax_mode = if diff_rows.len() > 4_000 {
            DiffSyntaxMode::HeuristicOnly
        } else {
            DiffSyntaxMode::Auto
        };

        Self {
            diff_rows,
            diff_word_highlights_split,
            diff_row_conflict_map,
            visible_row_indices,
            conflict_count: segments
                .iter()
                .filter(|segment| matches!(segment, ConflictSegment::Block(_)))
                .count(),
            language: diff_syntax_language_for_path("src/conflict.rs"),
            syntax_mode,
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
        let (ours_text, theirs_text) = materialize_two_way_side_texts(&segments);
        let diff_rows = gitcomet_core::file_diff::side_by_side_rows(&ours_text, &theirs_text);
        let inline_rows = conflict_resolver::build_inline_rows(&diff_rows);
        let (diff_row_conflict_map, _) =
            conflict_resolver::map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);
        let visible_row_indices = conflict_resolver::build_two_way_visible_indices(
            &diff_row_conflict_map,
            &segments,
            false,
        );
        let diff_word_highlights_split =
            conflict_resolver::compute_two_way_word_highlights(&diff_rows);
        let syntax_mode = if diff_rows.len() > 4_000 {
            DiffSyntaxMode::HeuristicOnly
        } else {
            DiffSyntaxMode::Auto
        };

        let mut fixture = Self {
            diff_rows,
            diff_word_highlights_split,
            visible_row_indices,
            conflict_count: segments
                .iter()
                .filter(|segment| matches!(segment, ConflictSegment::Block(_)))
                .count(),
            language: diff_syntax_language_for_path("src/conflict.rs"),
            syntax_mode,
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
            status: if ix % 5 == 0 { 'M' } else { ' ' },
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
            segments.push(ConflictSegment::Text(text));
        }

        if slot_ix < conflict_blocks {
            let choice = match slot_ix % 4 {
                0 => ConflictChoice::Base,
                1 => ConflictChoice::Ours,
                2 => ConflictChoice::Theirs,
                _ => ConflictChoice::Both,
            };
            segments.push(ConflictSegment::Block(ConflictBlock {
                base: Some(format!("let shared_{slot_ix} = compute_base({slot_ix});\n")),
                ours: format!("let shared_{slot_ix} = compute_local({slot_ix});\n"),
                theirs: format!("let shared_{slot_ix} = compute_remote({slot_ix});\n"),
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
            segments.push(ConflictSegment::Text(text));
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
                ours,
                theirs,
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

fn word_ranges_for_line(
    highlights: &conflict_resolver::WordHighlights,
    line_ix: usize,
) -> &[Range<usize>] {
    highlights
        .get(line_ix)
        .and_then(|ranges| ranges.as_deref())
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
    fn conflict_two_way_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictTwoWaySplitScrollFixture::new(120, 12);
        assert_eq!(fixture.conflict_count(), 12);
        assert!(fixture.visible_rows() > 0);
    }

    #[test]
    fn conflict_two_way_fixture_wraps_start_offsets() {
        let fixture = ConflictTwoWaySplitScrollFixture::new(180, 18);
        let hash_a = fixture.run_scroll_step(17, 40);
        let hash_b = fixture.run_scroll_step(17 + fixture.visible_rows() * 3, 40);
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn conflict_search_query_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictSearchQueryUpdateFixture::new(120, 12);
        assert_eq!(fixture.conflict_count(), 12);
        assert!(fixture.visible_rows() > 0);
        assert!(fixture.stable_cache_entries() > 0);
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
}
