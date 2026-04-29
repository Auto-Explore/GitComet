use super::*;
use crate::view::diff_utils::compute_diff_yaml_block_scalar_for_src_ix;
use crate::view::markdown_preview;
use crate::view::perf::{self, ViewPerfSpan};
use crate::view::rows;
use gitcomet_core::domain::DiffRowProvider;

mod file_diff;
mod image_cache;
mod patch_diff;
mod word_highlight;

#[cfg(any(test, feature = "benchmarks"))]
#[allow(unused_imports)]
pub(in crate::view) use self::file_diff::build_file_diff_cache_rebuild;
pub(in crate::view) use self::file_diff::{
    PagedFileDiffInlineRows, PagedFileDiffRows, build_file_diff_cache_rebuild_with_patch,
};
use self::file_diff::{build_inline_text, file_diff_text_signature};
#[cfg(feature = "benchmarks")]
pub(in crate::view) use self::image_cache::render_svg_image_diff_preview;

use self::patch_diff::{
    PATCH_DIFF_PAGE_SIZE, PatchSplitVisibleMeta, build_patch_split_visible_meta_from_src,
    scrollbar_markers_from_visible_flags, should_hide_unified_diff_header_raw,
};
pub(in crate::view) use self::patch_diff::{
    PagedPatchDiffRows, PagedPatchSplitRows, PatchInlineVisibleMap,
};

const PREPARED_SYNTAX_DOCUMENT_CACHE_MAX_ENTRIES: usize = 256;
const FILE_DIFF_PAGE_SIZE: usize = 256;
const FILE_DIFF_MAX_CACHED_PAGES: usize = 64;
const COLLAPSED_DIFF_REVEAL_STEP: usize = 20;

// Full-document views (file diff, worktree preview) always attempt prepared
// syntax and fall back to plain/heuristic rendering until it is ready.
const FULL_DOCUMENT_SYNTAX_MODE: rows::DiffSyntaxMode = rows::DiffSyntaxMode::Auto;

fn patch_diff_content_signature(diff: &gitcomet_core::domain::Diff) -> u64 {
    use std::hash::Hasher;

    let mut hasher = rustc_hash::FxHasher::default();
    hasher.write_usize(diff.lines.len());
    for line in diff.lines.iter() {
        let kind = match line.kind {
            gitcomet_core::domain::DiffLineKind::Header => 0,
            gitcomet_core::domain::DiffLineKind::Hunk => 1,
            gitcomet_core::domain::DiffLineKind::Add => 2,
            gitcomet_core::domain::DiffLineKind::Remove => 3,
            gitcomet_core::domain::DiffLineKind::Context => 4,
        };
        hasher.write_u8(kind);
        hasher.write_usize(line.text.len());
        hasher.write(line.text.as_ref().as_bytes());
    }
    hasher.finish()
}

fn file_diff_markdown_source_len(
    source: Option<&gitcomet_core::domain::FileDiffTextSource>,
    legacy_text: Option<&Arc<str>>,
) -> usize {
    if let Some(text) = legacy_text {
        return text.len();
    }
    source
        .and_then(|source| std::fs::metadata(&source.path).ok())
        .and_then(|metadata| usize::try_from(metadata.len()).ok())
        .unwrap_or(0)
}

fn read_file_diff_markdown_source(
    source: Option<&gitcomet_core::domain::FileDiffTextSource>,
    legacy_text: Option<&Arc<str>>,
) -> std::result::Result<String, String> {
    if let Some(text) = legacy_text {
        return Ok(text.to_string());
    }
    let Some(source) = source else {
        return Ok(String::new());
    };
    std::fs::read_to_string(&source.path).map_err(|err| err.to_string())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FileDiffPreparedSyntaxApplyResult {
    split_left: bool,
    split_right: bool,
}

impl FileDiffPreparedSyntaxApplyResult {
    fn any(self) -> bool {
        self.split_left || self.split_right
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SyncFileDiffPreparedSyntaxApplyResult {
    inserted: bool,
    needs_background_prepare: bool,
}

#[cfg(test)]
fn preview_lines_source_len(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| line.len())
        .sum::<usize>()
        .saturating_add(lines.len().saturating_sub(1))
}

fn build_single_markdown_preview_document(
    source: &str,
) -> Result<Arc<markdown_preview::MarkdownPreviewDocument>, String> {
    if source.len() > markdown_preview::MAX_PREVIEW_SOURCE_BYTES {
        return Err(markdown_preview::single_preview_unavailable_reason(source.len()).to_string());
    }

    markdown_preview::parse_markdown(source)
        .map(Arc::new)
        .ok_or_else(|| {
            markdown_preview::single_preview_unavailable_reason(source.len()).to_string()
        })
}

#[derive(Clone, Debug, Default)]
struct FileDiffBackgroundPreparedSyntaxDocuments {
    split_left: Option<rows::BackgroundPreparedDiffSyntaxDocument>,
    split_right: Option<rows::BackgroundPreparedDiffSyntaxDocument>,
}

fn prepared_syntax_document_key(
    repo_id: RepoId,
    target_rev: u64,
    file_path: &std::path::Path,
    view_mode: PreparedSyntaxViewMode,
) -> PreparedSyntaxDocumentKey {
    PreparedSyntaxDocumentKey {
        repo_id,
        target_rev,
        file_path: file_path.to_path_buf(),
        view_mode,
    }
}

fn diff_syntax_edit_from_text_change(old: &str, new: &str) -> Option<rows::DiffSyntaxEdit> {
    if old == new {
        return None;
    }

    let old_bytes = old.as_bytes();
    let new_bytes = new.as_bytes();

    let mut prefix = 0usize;
    let max_prefix = old_bytes.len().min(new_bytes.len());
    while prefix < max_prefix && old_bytes[prefix] == new_bytes[prefix] {
        prefix += 1;
    }

    let mut old_suffix_start = old_bytes.len();
    let mut new_suffix_start = new_bytes.len();
    while old_suffix_start > prefix
        && new_suffix_start > prefix
        && old_bytes[old_suffix_start - 1] == new_bytes[new_suffix_start - 1]
    {
        old_suffix_start -= 1;
        new_suffix_start -= 1;
    }

    Some(rows::DiffSyntaxEdit {
        old_range: prefix..old_suffix_start,
        new_range: prefix..new_suffix_start,
    })
}

impl MainPaneView {
    pub(in crate::view) fn file_diff_split_row_len(&self) -> usize {
        self.file_diff_row_provider
            .as_ref()
            .map(|provider| provider.len_hint())
            .unwrap_or_else(|| self.file_diff_cache_rows.len())
    }

    pub(in crate::view) fn file_diff_split_row(&self, row_ix: usize) -> Option<FileDiffRow> {
        if let Some(provider) = self.file_diff_row_provider.as_ref() {
            provider.row(row_ix)
        } else {
            self.file_diff_cache_rows.get(row_ix).cloned()
        }
    }

    pub(in crate::view) fn file_diff_split_render_data(
        &self,
        row_ix: usize,
    ) -> Option<FileDiffRow> {
        if let Some(provider) = self.file_diff_row_provider.as_ref() {
            provider.render_data(row_ix)
        } else {
            self.file_diff_cache_rows.get(row_ix).cloned()
        }
    }

    pub(in crate::view) fn file_diff_inline_row_len(&self) -> usize {
        self.file_diff_inline_row_provider
            .as_ref()
            .map(|provider| provider.len_hint())
            .unwrap_or_else(|| self.file_diff_inline_cache.len())
    }

    pub(in crate::view) fn file_diff_inline_row(
        &self,
        inline_ix: usize,
    ) -> Option<AnnotatedDiffLine> {
        if let Some(provider) = self.file_diff_inline_row_provider.as_ref() {
            provider.row(inline_ix)
        } else {
            self.file_diff_inline_cache.get(inline_ix).cloned()
        }
    }

    pub(in crate::view) fn file_diff_inline_render_data(
        &self,
        inline_ix: usize,
    ) -> Option<self::file_diff::InlineFileDiffRowRenderData> {
        if let Some(provider) = self.file_diff_inline_row_provider.as_ref() {
            provider.render_data(inline_ix)
        } else {
            let line = self.file_diff_inline_cache.get(inline_ix)?.clone();
            Some(self::file_diff::InlineFileDiffRowRenderData {
                kind: line.kind,
                old_line: line.old_line,
                new_line: line.new_line,
                text: crate::view::diff_utils::diff_content_line_text(&line),
            })
        }
    }

    pub(in crate::view) fn file_diff_split_modify_pair_texts(
        &self,
        row_ix: usize,
    ) -> Option<(
        gitcomet_core::file_diff::FileDiffLineText,
        gitcomet_core::file_diff::FileDiffLineText,
    )> {
        self.file_diff_row_provider
            .as_ref()
            .and_then(|provider| provider.modify_pair_texts(row_ix))
    }

    pub(in crate::view) fn file_diff_inline_modify_pair_texts(
        &self,
        inline_ix: usize,
    ) -> Option<(
        gitcomet_core::file_diff::FileDiffLineText,
        gitcomet_core::file_diff::FileDiffLineText,
        gitcomet_core::domain::DiffLineKind,
    )> {
        self.file_diff_inline_row_provider
            .as_ref()
            .and_then(|provider| provider.modify_pair_texts(inline_ix))
    }

    pub(in crate::view) fn ensure_file_diff_inline_text_materialized(&mut self) {
        if !self.file_diff_inline_text.is_empty() || self.file_diff_inline_row_len() == 0 {
            return;
        }
        if let Some(provider) = self.file_diff_inline_row_provider.as_ref() {
            self.file_diff_inline_text = provider.build_full_text();
        } else {
            self.file_diff_inline_text = build_inline_text(self.file_diff_inline_cache.as_slice());
        }
    }

    pub(in crate::view) fn patch_diff_row_len(&self) -> usize {
        self.diff_row_provider
            .as_ref()
            .map(|provider| provider.len_hint())
            .unwrap_or_else(|| self.diff_cache.len())
    }

    pub(in crate::view) fn patch_diff_row(&self, src_ix: usize) -> Option<AnnotatedDiffLine> {
        if let Some(provider) = self.diff_row_provider.as_ref() {
            provider.row(src_ix)
        } else {
            self.diff_cache.get(src_ix).cloned()
        }
    }

    pub(in crate::view) fn patch_diff_rows_slice(
        &self,
        start: usize,
        end: usize,
    ) -> Vec<AnnotatedDiffLine> {
        if let Some(provider) = self.diff_row_provider.as_ref() {
            provider.slice(start, end).collect()
        } else {
            let end = end.min(self.diff_cache.len());
            if start >= end {
                Vec::new()
            } else {
                self.diff_cache[start..end].to_vec()
            }
        }
    }

    pub(in crate::view) fn patch_diff_split_row_len(&self) -> usize {
        self.diff_split_row_provider
            .as_ref()
            .map(|provider| provider.len_hint())
            .unwrap_or_else(|| self.diff_split_cache.len())
    }

    pub(in crate::view) fn patch_diff_split_row(&self, row_ix: usize) -> Option<PatchSplitRow> {
        if let Some(provider) = self.diff_split_row_provider.as_ref() {
            provider.row(row_ix)
        } else {
            self.diff_split_cache.get(row_ix).cloned()
        }
    }

    fn patch_split_visible_meta_from_source(&self) -> PatchSplitVisibleMeta {
        build_patch_split_visible_meta_from_src(
            self.diff_line_kind_for_src_ix.as_slice(),
            self.diff_click_kinds.as_slice(),
            self.diff_hide_unified_header_for_src_ix.as_slice(),
        )
    }

    pub(in crate::view) fn ensure_patch_diff_word_highlight_for_src_ix(&mut self, src_ix: usize) {
        use gitcomet_core::domain::DiffLineKind as DK;

        let len = self.patch_diff_row_len();
        if src_ix >= len {
            return;
        }
        if self.diff_word_highlights.len() != len {
            self.diff_word_highlights.resize(len, None);
        }
        if self
            .diff_word_highlights
            .get(src_ix)
            .and_then(Option::as_ref)
            .is_some()
        {
            return;
        }

        let Some(line) = self.patch_diff_row(src_ix) else {
            return;
        };
        if !matches!(line.kind, DK::Add | DK::Remove) {
            return;
        }

        let mut group_start = src_ix;
        while group_start > 0 {
            let Some(prev) = self.patch_diff_row(group_start.saturating_sub(1)) else {
                break;
            };
            if matches!(prev.kind, DK::Remove) {
                group_start = group_start.saturating_sub(1);
            } else {
                break;
            }
        }

        let mut ix = group_start;
        let mut removed: Vec<(usize, AnnotatedDiffLine)> = Vec::new();
        while ix < len {
            let Some(line) = self.patch_diff_row(ix) else {
                break;
            };
            if !matches!(line.kind, DK::Remove) {
                break;
            }
            removed.push((ix, line));
            ix += 1;
        }

        let mut added: Vec<(usize, AnnotatedDiffLine)> = Vec::new();
        while ix < len {
            let Some(line) = self.patch_diff_row(ix) else {
                break;
            };
            if !matches!(line.kind, DK::Add) {
                break;
            }
            added.push((ix, line));
            ix += 1;
        }

        let pairs = removed.len().min(added.len());
        for i in 0..pairs {
            let (old_ix, old_line) = &removed[i];
            let (new_ix, new_line) = &added[i];
            let (old_ranges, new_ranges) =
                capped_word_diff_ranges(diff_content_text(old_line), diff_content_text(new_line));
            if !old_ranges.is_empty() {
                self.diff_word_highlights[*old_ix] = Some(old_ranges);
            }
            if !new_ranges.is_empty() {
                self.diff_word_highlights[*new_ix] = Some(new_ranges);
            }
        }

        for (old_ix, old_line) in removed.into_iter().skip(pairs) {
            let text = diff_content_text(&old_line);
            if !text.is_empty() {
                self.diff_word_highlights[old_ix] = Some(vec![Range {
                    start: 0,
                    end: text.len(),
                }]);
            }
        }
        for (new_ix, new_line) in added.into_iter().skip(pairs) {
            let text = diff_content_text(&new_line);
            if !text.is_empty() {
                self.diff_word_highlights[new_ix] = Some(vec![Range {
                    start: 0,
                    end: text.len(),
                }]);
            }
        }
    }

    fn current_file_diff_line_to_row_maps(&self) -> (&[Option<usize>], &[Option<usize>], usize) {
        match self.diff_view {
            DiffViewMode::Inline => (
                self.file_diff_old_line_to_inline_row.as_ref(),
                self.file_diff_new_line_to_inline_row.as_ref(),
                self.file_diff_inline_row_len(),
            ),
            DiffViewMode::Split => (
                self.file_diff_old_line_to_row.as_ref(),
                self.file_diff_new_line_to_row.as_ref(),
                self.file_diff_split_row_len(),
            ),
        }
    }

    fn collapsed_hunk_row_range_for_parsed(
        &self,
        parsed: &crate::view::diff_utils::ParsedHunkHeader,
    ) -> Option<(usize, usize)> {
        let (old_line_to_row, new_line_to_row, _row_count) =
            self.current_file_diff_line_to_row_maps();

        let map_range_start = |line_to_row: &[Option<usize>], start_line: u32, line_count: u32| {
            (line_count > 0)
                .then_some(start_line)
                .filter(|line| *line > 0)
                .and_then(|line| usize::try_from(line.saturating_sub(1)).ok())
                .and_then(|line_ix| line_to_row.get(line_ix).copied().flatten())
        };
        let map_range_end = |line_to_row: &[Option<usize>], start_line: u32, line_count: u32| {
            (line_count > 0)
                .then_some(start_line.saturating_add(line_count).saturating_sub(1))
                .filter(|line| *line > 0)
                .and_then(|line| usize::try_from(line.saturating_sub(1)).ok())
                .and_then(|line_ix| line_to_row.get(line_ix).copied().flatten())
                .map(|row_ix| row_ix.saturating_add(1))
        };

        let start = [
            map_range_start(
                old_line_to_row,
                parsed.old_start_line,
                parsed.old_line_count,
            ),
            map_range_start(
                new_line_to_row,
                parsed.new_start_line,
                parsed.new_line_count,
            ),
        ]
        .into_iter()
        .flatten()
        .min()?;

        let end = [
            map_range_end(
                old_line_to_row,
                parsed.old_start_line,
                parsed.old_line_count,
            ),
            map_range_end(
                new_line_to_row,
                parsed.new_start_line,
                parsed.new_line_count,
            ),
        ]
        .into_iter()
        .flatten()
        .max()?;

        (start < end).then_some((start, end))
    }

    fn collapsed_hunk_change_summary(&self, src_ix: usize) -> (bool, bool) {
        let mut has_additions = false;
        let mut has_removals = false;

        for candidate_ix in src_ix.saturating_add(1)..self.patch_diff_row_len() {
            let click_kind = self
                .diff_click_kinds
                .get(candidate_ix)
                .copied()
                .unwrap_or(DiffClickKind::Line);
            if click_kind != DiffClickKind::Line {
                break;
            }

            let Some(line) = self.patch_diff_row(candidate_ix) else {
                continue;
            };
            match line.kind {
                gitcomet_core::domain::DiffLineKind::Add => has_additions = true,
                gitcomet_core::domain::DiffLineKind::Remove => has_removals = true,
                gitcomet_core::domain::DiffLineKind::Context
                | gitcomet_core::domain::DiffLineKind::Header
                | gitcomet_core::domain::DiffLineKind::Hunk => {}
            }

            if has_additions && has_removals {
                break;
            }
        }

        (has_additions, has_removals)
    }

    fn reindex_collapsed_diff_hunks(&mut self) {
        self.collapsed_diff_hunk_ix_by_src_ix.clear();
        for (hunk_ix, hunk) in self.collapsed_diff_hunks.iter().enumerate() {
            let previous = self
                .collapsed_diff_hunk_ix_by_src_ix
                .insert(hunk.src_ix, hunk_ix);
            debug_assert!(previous.is_none());
        }
    }

    fn ensure_collapsed_diff_hunk_index(&mut self) {
        if self.collapsed_diff_hunk_ix_by_src_ix.len() != self.collapsed_diff_hunks.len() {
            self.reindex_collapsed_diff_hunks();
        }
    }

    fn ensure_collapsed_diff_hunks_initialized(&mut self) {
        if !self.collapsed_diff_hunks.is_empty() {
            self.ensure_collapsed_diff_hunk_index();
            return;
        }

        for src_ix in 0..self.patch_diff_row_len() {
            let click_kind = self
                .diff_click_kinds
                .get(src_ix)
                .copied()
                .unwrap_or(DiffClickKind::Line);
            if click_kind != DiffClickKind::HunkHeader {
                continue;
            }

            let Some(line) = self.patch_diff_row(src_ix) else {
                continue;
            };
            let Some(parsed) =
                crate::view::diff_utils::parse_unified_hunk_header_for_display(line.text.as_ref())
            else {
                continue;
            };
            let Some((base_row_start, base_row_end_exclusive)) =
                self.collapsed_hunk_row_range_for_parsed(&parsed)
            else {
                continue;
            };
            let (has_additions, has_removals) = self.collapsed_hunk_change_summary(src_ix);
            let reveal = self
                .collapsed_diff_reveals
                .get(&src_ix)
                .copied()
                .unwrap_or_default();
            self.collapsed_diff_hunks.push(CollapsedDiffHunk {
                src_ix,
                base_row_start,
                base_row_end_exclusive,
                has_additions,
                has_removals,
                reveal_up_lines: reveal.up_lines,
                reveal_down_lines: reveal.down_lines,
            });
        }
        self.reindex_collapsed_diff_hunks();
    }

    fn persist_collapsed_diff_hunk_reveal(&mut self, hunk_ix: usize) {
        let Some(hunk) = self.collapsed_diff_hunks.get(hunk_ix).copied() else {
            return;
        };
        let reveal = CollapsedDiffReveal {
            up_lines: hunk.reveal_up_lines,
            down_lines: hunk.reveal_down_lines,
        };
        if reveal == CollapsedDiffReveal::default() {
            self.collapsed_diff_reveals.remove(&hunk.src_ix);
        } else {
            self.collapsed_diff_reveals.insert(hunk.src_ix, reveal);
        }
    }

    fn collapsed_diff_expansion_kind(
        &self,
        hunk_ix: usize,
    ) -> crate::view::panes::main::CollapsedDiffExpansionKind {
        use crate::view::panes::main::CollapsedDiffExpansionKind;

        let Some(hunk) = self.collapsed_diff_hunks.get(hunk_ix).copied() else {
            return CollapsedDiffExpansionKind::None;
        };

        let hidden_up = self.collapsed_diff_hidden_up_rows(hunk.src_ix);
        if hunk_ix == 0 {
            if hidden_up > 0 {
                CollapsedDiffExpansionKind::Up
            } else {
                CollapsedDiffExpansionKind::None
            }
        } else if hidden_up == 0 {
            CollapsedDiffExpansionKind::None
        } else if hidden_up <= COLLAPSED_DIFF_REVEAL_STEP {
            CollapsedDiffExpansionKind::Short
        } else {
            CollapsedDiffExpansionKind::Both
        }
    }

    fn collapsed_diff_hidden_rows_for_expansion_kind(
        &self,
        src_ix: usize,
        expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind,
    ) -> usize {
        match expansion_kind {
            crate::view::panes::main::CollapsedDiffExpansionKind::Down => {
                self.collapsed_diff_hidden_down_rows(src_ix)
            }
            crate::view::panes::main::CollapsedDiffExpansionKind::Up
            | crate::view::panes::main::CollapsedDiffExpansionKind::Both
            | crate::view::panes::main::CollapsedDiffExpansionKind::Short => {
                self.collapsed_diff_hidden_up_rows(src_ix)
            }
            crate::view::panes::main::CollapsedDiffExpansionKind::None => 0,
        }
    }

    fn merge_collapsed_diff_hunks_up(&mut self, hunk_ix: usize) {
        if hunk_ix == 0 || hunk_ix >= self.collapsed_diff_hunks.len() {
            return;
        }

        let previous = self.collapsed_diff_hunks[hunk_ix - 1];
        let current = self.collapsed_diff_hunks[hunk_ix];
        self.collapsed_diff_hunks[hunk_ix - 1] = CollapsedDiffHunk {
            src_ix: previous.src_ix,
            base_row_start: previous.base_row_start,
            base_row_end_exclusive: current.base_row_end_exclusive,
            has_additions: previous.has_additions || current.has_additions,
            has_removals: previous.has_removals || current.has_removals,
            reveal_up_lines: previous.reveal_up_lines,
            reveal_down_lines: current.reveal_down_lines,
        };
        self.collapsed_diff_hunks.remove(hunk_ix);
        self.reindex_collapsed_diff_hunks();
        self.collapsed_diff_header_display_cache.clear();
    }

    fn merge_collapsed_diff_hunks_down(&mut self, hunk_ix: usize) {
        if hunk_ix + 1 >= self.collapsed_diff_hunks.len() {
            return;
        }

        let current = self.collapsed_diff_hunks[hunk_ix];
        let next = self.collapsed_diff_hunks[hunk_ix + 1];
        self.collapsed_diff_hunks[hunk_ix] = CollapsedDiffHunk {
            src_ix: current.src_ix,
            base_row_start: current.base_row_start,
            base_row_end_exclusive: next.base_row_end_exclusive,
            has_additions: current.has_additions || next.has_additions,
            has_removals: current.has_removals || next.has_removals,
            reveal_up_lines: current.reveal_up_lines,
            reveal_down_lines: next.reveal_down_lines,
        };
        self.collapsed_diff_hunks.remove(hunk_ix + 1);
        self.reindex_collapsed_diff_hunks();
        self.collapsed_diff_header_display_cache.clear();
    }

    fn collapsed_diff_gap_fully_revealed_after_rebuild(&self, hunk_ix: usize) -> bool {
        let Some(current) = self.collapsed_diff_hunks.get(hunk_ix).copied() else {
            return false;
        };
        let Some(next) = self.collapsed_diff_hunks.get(hunk_ix + 1).copied() else {
            return false;
        };

        let gap_len = next
            .base_row_start
            .saturating_sub(current.base_row_end_exclusive);
        if gap_len == 0 {
            return false;
        }

        current
            .reveal_down_lines
            .min(gap_len)
            .saturating_add(next.reveal_up_lines.min(gap_len))
            >= gap_len
    }

    fn normalize_collapsed_diff_hunks_after_rebuild(&mut self) {
        let mut hunk_ix = 0;
        while hunk_ix + 1 < self.collapsed_diff_hunks.len() {
            if self.collapsed_diff_gap_fully_revealed_after_rebuild(hunk_ix) {
                self.merge_collapsed_diff_hunks_down(hunk_ix);
            } else {
                hunk_ix += 1;
            }
        }
    }

    fn rebuild_collapsed_diff_header_display_cache(&mut self) {
        self.collapsed_diff_header_display_cache.clear();
        let src_ixs = self
            .collapsed_diff_hunks
            .iter()
            .map(|hunk| hunk.src_ix)
            .collect::<Vec<_>>();
        for src_ix in src_ixs {
            if let Some(display) = self.collapsed_diff_dynamic_hunk_range_display(src_ix) {
                self.collapsed_diff_header_display_cache
                    .insert(src_ix, display);
            }
        }
    }

    fn rebuild_collapsed_diff_projection(&mut self) {
        self.collapsed_diff_visible_rows.clear();
        self.collapsed_diff_hunk_visible_indices.clear();
        self.collapsed_diff_header_display_cache.clear();

        if !self.is_collapsed_diff_projection_active() {
            return;
        }

        let (_, _, total_rows) = self.current_file_diff_line_to_row_maps();
        if total_rows == 0 {
            return;
        }

        self.ensure_collapsed_diff_hunks_initialized();
        self.normalize_collapsed_diff_hunks_after_rebuild();
        self.reindex_collapsed_diff_hunks();

        if self.collapsed_diff_hunks.is_empty() {
            return;
        }

        for hunk_ix in 0..self.collapsed_diff_hunks.len() {
            let hunk = self.collapsed_diff_hunks[hunk_ix];
            let expansion_kind = self.collapsed_diff_expansion_kind(hunk_ix);
            let has_expansion_header =
                expansion_kind != crate::view::panes::main::CollapsedDiffExpansionKind::None;

            let up_revealed_rows = if hunk_ix == 0 {
                let leading_start = hunk
                    .base_row_start
                    .saturating_sub(hunk.reveal_up_lines.min(hunk.base_row_start));
                leading_start..hunk.base_row_start
            } else {
                let previous = self.collapsed_diff_hunks[hunk_ix - 1];
                let gap_start = previous.base_row_end_exclusive;
                let gap_end = hunk.base_row_start.max(gap_start);
                let gap_len = gap_end.saturating_sub(gap_start);
                let top_end = gap_start.saturating_add(previous.reveal_down_lines.min(gap_len));
                let bottom_start = gap_end.saturating_sub(hunk.reveal_up_lines.min(gap_len));

                for row_ix in gap_start..top_end {
                    self.collapsed_diff_visible_rows
                        .push(CollapsedDiffVisibleRow::FileRow { row_ix });
                }
                bottom_start.max(top_end)..gap_end
            };

            if !has_expansion_header {
                for row_ix in up_revealed_rows.clone() {
                    self.collapsed_diff_visible_rows
                        .push(CollapsedDiffVisibleRow::FileRow { row_ix });
                }
            }

            self.collapsed_diff_hunk_visible_indices
                .push(self.collapsed_diff_visible_rows.len());
            if has_expansion_header {
                let hidden_rows =
                    self.collapsed_diff_hidden_rows_for_expansion_kind(hunk.src_ix, expansion_kind);
                self.collapsed_diff_visible_rows
                    .push(CollapsedDiffVisibleRow::HunkHeader {
                        src_ix: hunk.src_ix,
                        expansion_kind,
                        display_src_ix: Some(hunk.src_ix),
                        hidden_rows,
                    });
                for row_ix in up_revealed_rows {
                    self.collapsed_diff_visible_rows
                        .push(CollapsedDiffVisibleRow::FileRow { row_ix });
                }
            }
            for row_ix in hunk.base_row_start..hunk.base_row_end_exclusive {
                self.collapsed_diff_visible_rows
                    .push(CollapsedDiffVisibleRow::FileRow { row_ix });
            }
        }

        if let Some(last_hunk) = self.collapsed_diff_hunks.last().copied() {
            let trailing_end = last_hunk
                .base_row_end_exclusive
                .saturating_add(
                    last_hunk
                        .reveal_down_lines
                        .min(total_rows.saturating_sub(last_hunk.base_row_end_exclusive)),
                )
                .min(total_rows);
            for row_ix in last_hunk.base_row_end_exclusive..trailing_end {
                self.collapsed_diff_visible_rows
                    .push(CollapsedDiffVisibleRow::FileRow { row_ix });
            }

            let hidden_rows = self.collapsed_diff_hidden_down_rows(last_hunk.src_ix);
            if hidden_rows > 0 {
                self.collapsed_diff_visible_rows
                    .push(CollapsedDiffVisibleRow::HunkHeader {
                        src_ix: last_hunk.src_ix,
                        expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Down,
                        display_src_ix: None,
                        hidden_rows,
                    });
            }
        }
        self.rebuild_collapsed_diff_header_display_cache();
    }

    fn collapsed_diff_hunk_index_for_src_ix(&self, src_ix: usize) -> Option<usize> {
        self.collapsed_diff_hunk_ix_by_src_ix.get(&src_ix).copied()
    }

    pub(in crate::view) fn collapsed_diff_hunk_for_src_ix(
        &self,
        src_ix: usize,
    ) -> Option<CollapsedDiffHunk> {
        self.collapsed_diff_hunk_index_for_src_ix(src_ix)
            .and_then(|hunk_ix| self.collapsed_diff_hunks.get(hunk_ix).copied())
    }

    pub(in crate::view) fn collapsed_diff_hidden_up_rows(&self, src_ix: usize) -> usize {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return 0;
        };
        let hunk = self.collapsed_diff_hunks[hunk_ix];
        if hunk_ix == 0 {
            return hunk
                .base_row_start
                .saturating_sub(hunk.reveal_up_lines.min(hunk.base_row_start));
        }

        let prev = self.collapsed_diff_hunks[hunk_ix - 1];
        let gap_len = hunk
            .base_row_start
            .saturating_sub(prev.base_row_end_exclusive);
        let visible = prev
            .reveal_down_lines
            .min(gap_len)
            .saturating_add(hunk.reveal_up_lines.min(gap_len));
        gap_len.saturating_sub(visible.min(gap_len))
    }

    pub(in crate::view) fn collapsed_diff_hidden_down_rows(&self, src_ix: usize) -> usize {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return 0;
        };
        let hunk = self.collapsed_diff_hunks[hunk_ix];
        let (_, _, total_rows) = self.current_file_diff_line_to_row_maps();
        if hunk_ix + 1 >= self.collapsed_diff_hunks.len() {
            return total_rows
                .saturating_sub(hunk.base_row_end_exclusive)
                .saturating_sub(
                    hunk.reveal_down_lines
                        .min(total_rows.saturating_sub(hunk.base_row_end_exclusive)),
                );
        }

        let next = self.collapsed_diff_hunks[hunk_ix + 1];
        let gap_len = next
            .base_row_start
            .saturating_sub(hunk.base_row_end_exclusive);
        let visible = hunk
            .reveal_down_lines
            .min(gap_len)
            .saturating_add(next.reveal_up_lines.min(gap_len));
        gap_len.saturating_sub(visible.min(gap_len))
    }

    fn collapsed_diff_file_row_line_numbers(
        &self,
        row_ix: usize,
    ) -> Option<(Option<u32>, Option<u32>)> {
        match self.diff_view {
            DiffViewMode::Inline => self
                .file_diff_inline_render_data(row_ix)
                .map(|row| (row.old_line, row.new_line)),
            DiffViewMode::Split => self
                .file_diff_split_row(row_ix)
                .map(|row| (row.old_line, row.new_line)),
        }
    }

    fn collapsed_diff_dynamic_hunk_range_display(&self, src_ix: usize) -> Option<SharedString> {
        fn update_bounds(min: &mut Option<u32>, max: &mut Option<u32>, line: Option<u32>) {
            let Some(line) = line else {
                return;
            };
            *min = Some(min.map_or(line, |current| current.min(line)));
            *max = Some(max.map_or(line, |current| current.max(line)));
        }

        fn format_range(
            prefix: char,
            fallback_start: u32,
            min: Option<u32>,
            max: Option<u32>,
        ) -> String {
            let (start, count) = match (min, max) {
                (Some(min), Some(max)) if max >= min => (min, max.saturating_sub(min) + 1),
                _ => (fallback_start, 0),
            };
            if count == 1 {
                format!("{prefix}{start}")
            } else {
                format!("{prefix}{start},{count}")
            }
        }

        let (_, _, total_rows) = self.current_file_diff_line_to_row_maps();
        let hunk_ix = self.collapsed_diff_hunk_index_for_src_ix(src_ix)?;
        let hunk = self.collapsed_diff_hunks[hunk_ix];
        let has_revealed_above = if hunk_ix == 0 {
            hunk.reveal_up_lines.min(hunk.base_row_start) > 0
        } else {
            let previous = self.collapsed_diff_hunks[hunk_ix - 1];
            let gap_len = hunk
                .base_row_start
                .saturating_sub(previous.base_row_end_exclusive);
            previous.reveal_down_lines.min(gap_len) > 0 || hunk.reveal_up_lines.min(gap_len) > 0
        };
        let has_revealed_below = if hunk_ix + 1 < self.collapsed_diff_hunks.len() {
            let next = self.collapsed_diff_hunks[hunk_ix + 1];
            let gap_len = next
                .base_row_start
                .saturating_sub(hunk.base_row_end_exclusive);
            hunk.reveal_down_lines.min(gap_len) > 0
        } else {
            hunk.reveal_down_lines
                .min(total_rows.saturating_sub(hunk.base_row_end_exclusive))
                > 0
        };
        if !has_revealed_above && !has_revealed_below {
            return None;
        }

        let parsed = self.patch_diff_row(src_ix).and_then(|line| {
            crate::view::diff_utils::parse_unified_hunk_header_for_display(line.text.as_ref())
        })?;
        let mut old_min = None;
        let mut old_max = None;
        let mut new_min = None;
        let mut new_max = None;
        let mut has_revealed_context = false;

        let mut visit_rows = |range: std::ops::Range<usize>,
                              revealed_context: bool,
                              this: &Self| {
            if range.is_empty() {
                return;
            }
            has_revealed_context |= revealed_context;
            for row_ix in range {
                let Some((old_line, new_line)) = this.collapsed_diff_file_row_line_numbers(row_ix)
                else {
                    continue;
                };
                update_bounds(&mut old_min, &mut old_max, old_line);
                update_bounds(&mut new_min, &mut new_max, new_line);
            }
        };

        if hunk_ix == 0 {
            let leading_start = hunk
                .base_row_start
                .saturating_sub(hunk.reveal_up_lines.min(hunk.base_row_start));
            visit_rows(leading_start..hunk.base_row_start, true, self);
        } else {
            let previous = self.collapsed_diff_hunks[hunk_ix - 1];
            let gap_start = previous.base_row_end_exclusive;
            let gap_end = hunk.base_row_start.max(gap_start);
            let gap_len = gap_end.saturating_sub(gap_start);
            let top_end = gap_start.saturating_add(previous.reveal_down_lines.min(gap_len));
            let bottom_start = gap_end.saturating_sub(hunk.reveal_up_lines.min(gap_len));

            visit_rows(gap_start..top_end, true, self);
            visit_rows(bottom_start.max(top_end)..gap_end, true, self);
        }

        visit_rows(
            hunk.base_row_start..hunk.base_row_end_exclusive,
            false,
            self,
        );

        let trailing_end = if hunk_ix + 1 < self.collapsed_diff_hunks.len() {
            let next = self.collapsed_diff_hunks[hunk_ix + 1];
            let gap_len = next
                .base_row_start
                .saturating_sub(hunk.base_row_end_exclusive);
            hunk.base_row_end_exclusive
                .saturating_add(hunk.reveal_down_lines.min(gap_len))
        } else {
            hunk.base_row_end_exclusive
                .saturating_add(
                    hunk.reveal_down_lines
                        .min(total_rows.saturating_sub(hunk.base_row_end_exclusive)),
                )
                .min(total_rows)
        };
        visit_rows(hunk.base_row_end_exclusive..trailing_end, true, self);

        has_revealed_context.then(|| {
            format!(
                "{} {}",
                format_range('-', parsed.old_start_line, old_min, old_max),
                format_range('+', parsed.new_start_line, new_min, new_max)
            )
            .into()
        })
    }

    pub(in crate::view) fn collapsed_diff_hunk_header_display(
        &self,
        src_ix: usize,
    ) -> Option<SharedString> {
        self.collapsed_diff_header_display_cache
            .get(&src_ix)
            .cloned()
            .or_else(|| self.diff_header_display_cache.get(&src_ix).cloned())
            .or_else(|| {
                self.patch_diff_row(src_ix)
                    .map(|line| SharedString::from(line.text.as_ref().to_owned()))
            })
    }

    pub(in crate::view) fn collapsed_diff_reveal_hunk_up(
        &mut self,
        src_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return;
        };
        let delta = self
            .collapsed_diff_hidden_up_rows(src_ix)
            .min(COLLAPSED_DIFF_REVEAL_STEP);
        if delta == 0 {
            return;
        }
        self.collapsed_diff_hunks[hunk_ix].reveal_up_lines = self.collapsed_diff_hunks[hunk_ix]
            .reveal_up_lines
            .saturating_add(delta);
        self.persist_collapsed_diff_hunk_reveal(hunk_ix);
        if self.collapsed_diff_hidden_up_rows(src_ix) == 0 && hunk_ix > 0 {
            self.merge_collapsed_diff_hunks_up(hunk_ix);
        }
        self.invalidate_collapsed_diff_visible_projection();
        self.ensure_diff_visible_indices();
        cx.notify();
    }

    pub(in crate::view) fn collapsed_diff_reveal_hunk_down(
        &mut self,
        src_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return;
        };
        let delta = self
            .collapsed_diff_hidden_down_rows(src_ix)
            .min(COLLAPSED_DIFF_REVEAL_STEP);
        if delta == 0 {
            return;
        }
        self.collapsed_diff_hunks[hunk_ix].reveal_down_lines = self.collapsed_diff_hunks[hunk_ix]
            .reveal_down_lines
            .saturating_add(delta);
        self.persist_collapsed_diff_hunk_reveal(hunk_ix);
        if hunk_ix + 1 < self.collapsed_diff_hunks.len()
            && self.collapsed_diff_hidden_down_rows(src_ix) == 0
        {
            self.merge_collapsed_diff_hunks_down(hunk_ix);
        }
        self.invalidate_collapsed_diff_visible_projection();
        self.ensure_diff_visible_indices();
        cx.notify();
    }

    pub(in crate::view) fn collapsed_diff_reveal_hunk_down_before(
        &mut self,
        src_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return;
        };
        if hunk_ix == 0 {
            return;
        }
        let previous_hunk_ix = hunk_ix - 1;
        let previous_src_ix = self.collapsed_diff_hunks[previous_hunk_ix].src_ix;
        let delta = self
            .collapsed_diff_hidden_down_rows(previous_src_ix)
            .min(COLLAPSED_DIFF_REVEAL_STEP);
        if delta == 0 {
            return;
        }
        self.collapsed_diff_hunks[previous_hunk_ix].reveal_down_lines = self.collapsed_diff_hunks
            [previous_hunk_ix]
            .reveal_down_lines
            .saturating_add(delta);
        self.persist_collapsed_diff_hunk_reveal(previous_hunk_ix);
        if previous_hunk_ix + 1 < self.collapsed_diff_hunks.len()
            && self.collapsed_diff_hidden_down_rows(previous_src_ix) == 0
        {
            self.merge_collapsed_diff_hunks_down(previous_hunk_ix);
        }
        self.invalidate_collapsed_diff_visible_projection();
        self.ensure_diff_visible_indices();
        cx.notify();
    }

    pub(in crate::view) fn collapsed_diff_reveal_hunk_short(
        &mut self,
        src_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return;
        };
        if hunk_ix == 0 {
            return;
        }
        let delta = self.collapsed_diff_hidden_up_rows(src_ix);
        if delta == 0 {
            return;
        }
        self.collapsed_diff_hunks[hunk_ix].reveal_up_lines = self.collapsed_diff_hunks[hunk_ix]
            .reveal_up_lines
            .saturating_add(delta);
        self.persist_collapsed_diff_hunk_reveal(hunk_ix);
        self.merge_collapsed_diff_hunks_up(hunk_ix);
        self.invalidate_collapsed_diff_visible_projection();
        self.ensure_diff_visible_indices();
        cx.notify();
    }

    fn prepared_syntax_document(
        &self,
        key: &PreparedSyntaxDocumentKey,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        self.prepared_syntax_documents.get(key).copied()
    }

    fn prepared_syntax_reparse_seed_document(
        &self,
        key: &PreparedSyntaxDocumentKey,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        self.prepared_syntax_documents
            .iter()
            .filter(|(candidate_key, _)| {
                candidate_key.repo_id == key.repo_id
                    && candidate_key.file_path == key.file_path
                    && candidate_key.view_mode == key.view_mode
                    && candidate_key.target_rev != key.target_rev
            })
            .max_by_key(|(candidate_key, _)| candidate_key.target_rev)
            .map(|(_, document)| *document)
    }

    fn insert_prepared_syntax_document(
        &mut self,
        key: PreparedSyntaxDocumentKey,
        document: rows::PreparedDiffSyntaxDocument,
    ) -> bool {
        if self.prepared_syntax_documents.contains_key(&key) {
            return false;
        }
        if self.prepared_syntax_documents.len() >= PREPARED_SYNTAX_DOCUMENT_CACHE_MAX_ENTRIES
            && let Some(evict_key) = self.prepared_syntax_documents.keys().next().cloned()
        {
            self.prepared_syntax_documents.remove(&evict_key);
        }
        self.prepared_syntax_documents.insert(key, document);
        true
    }

    fn rekey_prepared_syntax_document(
        &mut self,
        old_key: PreparedSyntaxDocumentKey,
        new_key: PreparedSyntaxDocumentKey,
    ) {
        if old_key == new_key {
            return;
        }
        let Some(document) = self.prepared_syntax_documents.remove(&old_key) else {
            return;
        };
        self.prepared_syntax_documents
            .entry(new_key)
            .or_insert(document);
    }

    fn rekey_file_diff_prepared_syntax_documents_for_rev(&mut self, new_rev: u64) {
        let Some(repo_id) = self.file_diff_cache_repo_id else {
            return;
        };
        let Some(path) = self.file_diff_cache_path.clone() else {
            return;
        };
        let old_rev = self.file_diff_cache_rev;
        if old_rev == new_rev {
            return;
        }

        for view_mode in [
            PreparedSyntaxViewMode::FileDiffSplitLeft,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        ] {
            let old_key = prepared_syntax_document_key(repo_id, old_rev, &path, view_mode);
            let new_key = prepared_syntax_document_key(repo_id, new_rev, &path, view_mode);
            self.rekey_prepared_syntax_document(old_key, new_key);
        }
    }

    pub(super) fn full_document_syntax_budget(&self) -> rows::DiffSyntaxBudget {
        #[cfg(test)]
        if let Some(budget) = self.diff_syntax_budget_override {
            return budget;
        }

        rows::DiffSyntaxBudget::default()
    }

    #[cfg(test)]
    pub(in crate::view) fn set_full_document_syntax_budget_override_for_tests(
        &mut self,
        budget: rows::DiffSyntaxBudget,
    ) {
        self.diff_syntax_budget_override = Some(budget);
    }

    pub(in crate::view) fn file_diff_prepared_syntax_key(
        &self,
        view_mode: PreparedSyntaxViewMode,
    ) -> Option<PreparedSyntaxDocumentKey> {
        let repo_id = self.file_diff_cache_repo_id?;
        let path = self.file_diff_cache_path.as_ref()?;
        Some(prepared_syntax_document_key(
            repo_id,
            self.file_diff_cache_rev,
            path,
            view_mode,
        ))
    }

    fn file_diff_prepared_syntax_document(
        &self,
        view_mode: PreparedSyntaxViewMode,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let key = self.file_diff_prepared_syntax_key(view_mode)?;
        self.prepared_syntax_document(&key)
    }

    pub(in crate::view) fn file_diff_split_style_cache_epoch(&self, region: DiffTextRegion) -> u64 {
        self.file_diff_style_cache_epochs.split_epoch(region)
    }

    pub(in crate::view) fn file_diff_inline_style_cache_epoch(
        &self,
        line: &AnnotatedDiffLine,
    ) -> u64 {
        self.file_diff_style_cache_epochs.inline_epoch(line.kind)
    }

    /// Project inline-diff syntax from the real old/new (split) documents.
    ///
    /// Instead of parsing the synthetic mixed inline stream, project each row into
    /// the correct real old/new document using its 1-based diff line numbers.
    pub(in crate::view) fn file_diff_inline_projected_syntax(
        &self,
        line: &AnnotatedDiffLine,
    ) -> rows::PreparedDiffSyntaxLine {
        rows::prepared_diff_syntax_line_for_inline_diff_row(
            self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
            self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            line,
        )
    }

    pub(in crate::view) fn file_diff_split_prepared_syntax_document(
        &self,
        region: DiffTextRegion,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let view_mode = match region {
            DiffTextRegion::SplitLeft => PreparedSyntaxViewMode::FileDiffSplitLeft,
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => {
                PreparedSyntaxViewMode::FileDiffSplitRight
            }
        };
        self.file_diff_prepared_syntax_document(view_mode)
    }

    pub(in crate::view) fn worktree_preview_prepared_syntax_key(
        &self,
    ) -> Option<PreparedSyntaxDocumentKey> {
        let repo_id = self.active_repo_id()?;
        let path = self.worktree_preview_path.as_ref()?;
        Some(prepared_syntax_document_key(
            repo_id,
            self.worktree_preview_content_rev,
            path,
            PreparedSyntaxViewMode::WorktreePreview,
        ))
    }

    pub(in crate::view) fn worktree_preview_prepared_syntax_document(
        &self,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let key = self.worktree_preview_prepared_syntax_key()?;
        self.prepared_syntax_document(&key)
    }

    pub(in super::super::super) fn ensure_single_markdown_preview_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(path) = self.worktree_preview_path.clone() else {
            return;
        };
        let source_rev = self.worktree_preview_content_rev;
        if !matches!(self.worktree_preview, Loadable::Ready(_)) {
            return;
        }

        let cache_matches = self.worktree_markdown_preview_path.as_ref() == Some(&path)
            && self.worktree_markdown_preview_source_rev == source_rev;
        if cache_matches {
            match &self.worktree_markdown_preview {
                Loadable::Ready(_) | Loadable::Error(_) => return,
                Loadable::Loading if self.worktree_markdown_preview_inflight.is_some() => return,
                _ => {}
            }
        }

        self.worktree_markdown_preview_path = Some(path.clone());
        self.worktree_markdown_preview_source_rev = source_rev;

        let source_len = if self.worktree_preview_text.is_empty() {
            self.worktree_preview_source_len
        } else {
            self.worktree_preview_text.len()
        };
        if source_len > markdown_preview::MAX_PREVIEW_SOURCE_BYTES {
            self.worktree_markdown_preview = Loadable::Error(
                markdown_preview::single_preview_unavailable_reason(source_len).to_string(),
            );
            self.worktree_markdown_preview_inflight = None;
            return;
        }

        self.worktree_markdown_preview = Loadable::Loading;
        self.worktree_markdown_preview_seq = self.worktree_markdown_preview_seq.wrapping_add(1);
        let seq = self.worktree_markdown_preview_seq;
        self.worktree_markdown_preview_inflight = Some(seq);
        let source_text =
            (!self.worktree_preview_text.is_empty()).then_some(self.worktree_preview_text.clone());
        let source_path = self.worktree_preview_source_path.clone();

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let build_preview = move || {
                    let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewParse);
                    let source_text = match source_text {
                        Some(source_text) => source_text,
                        None => {
                            let source_path = source_path
                                .ok_or_else(|| "Preview source path is unavailable.".to_string())?;
                            std::fs::read_to_string(&source_path)
                                .map(SharedString::from)
                                .map_err(|e| {
                                    if e.kind() == std::io::ErrorKind::InvalidData {
                                        "File is not valid UTF-8; binary preview is not supported."
                                            .to_string()
                                    } else {
                                        e.to_string()
                                    }
                                })?
                        }
                    };
                    build_single_markdown_preview_document(source_text.as_ref())
                };
                let result = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(build_preview).await
                } else {
                    build_preview()
                };

                let _ = view.update(cx, |this, cx| {
                    if this.worktree_markdown_preview_inflight != Some(seq) {
                        return;
                    }
                    if this.worktree_preview_path.as_ref() != Some(&path)
                        || this.worktree_preview_content_rev != source_rev
                    {
                        return;
                    }

                    this.worktree_markdown_preview_inflight = None;
                    match result {
                        Ok(document) => this.worktree_markdown_preview = Loadable::Ready(document),
                        Err(error) => this.worktree_markdown_preview = Loadable::Error(error),
                    }
                    cx.notify();
                });
            },
        )
        .detach();
    }

    fn apply_worktree_preview_ready_state(
        &mut self,
        display_path: std::path::PathBuf,
        source_path: std::path::PathBuf,
        source_len: usize,
        source_text: SharedString,
        line_starts: Arc<[usize]>,
        line_flags: Arc<[u8]>,
        cx: &mut gpui::Context<Self>,
    ) {
        let line_count = indexed_line_count_from_len(source_len, line_starts.as_ref());
        let source_changed = self.worktree_preview_path.as_ref() != Some(&display_path)
            || self.worktree_preview_source_path.as_ref() != Some(&source_path)
            || self.worktree_preview_line_count() != Some(line_count)
            || self.worktree_preview_source_len != source_len
            || self.worktree_preview_text.as_ref() != source_text.as_ref()
            || self.worktree_preview_line_starts.as_ref() != line_starts.as_ref()
            || self.worktree_preview_line_flags.as_ref() != line_flags.as_ref();
        let cache_binding_changed =
            self.worktree_preview_segments_cache_path.as_ref() != Some(&display_path);
        let same_path_source_refresh = source_changed && !cache_binding_changed;

        self.worktree_preview_path = Some(display_path.clone());
        self.worktree_preview_source_path = Some(source_path);
        self.worktree_preview = Loadable::Ready(line_count);
        self.worktree_preview_source_len = source_len;
        self.worktree_preview_text = source_text;
        self.worktree_preview_line_starts = line_starts;
        self.worktree_preview_line_flags = line_flags;
        self.worktree_preview_search_trigram_index = None;
        self.worktree_preview_syntax_language = rows::diff_syntax_language_for_path(&display_path);
        self.worktree_preview_segments_cache_path = Some(display_path);
        self.worktree_preview_cache_write_blocked_until_rev = None;
        if source_changed || cache_binding_changed {
            self.worktree_preview_segments_cache.clear();
        }

        if source_changed {
            self.worktree_preview_content_rev = self.worktree_preview_content_rev.wrapping_add(1);
            self.worktree_preview_style_cache_epoch =
                self.worktree_preview_style_cache_epoch.wrapping_add(1);
            self.worktree_markdown_preview_path = None;
            self.worktree_markdown_preview_source_rev = 0;
            self.worktree_markdown_preview = Loadable::NotLoaded;
            self.worktree_markdown_preview_inflight = None;
        }

        if same_path_source_refresh {
            let blocked_rev = self.worktree_preview_content_rev;
            self.worktree_preview_cache_write_blocked_until_rev = Some(blocked_rev);
            if !crate::ui_runtime::current().uses_background_compute() {
                if self.worktree_preview_cache_write_blocked_until_rev == Some(blocked_rev) {
                    self.worktree_preview_cache_write_blocked_until_rev = None;
                }
            } else {
                cx.spawn(
                    async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                        smol::Timer::after(std::time::Duration::from_millis(1)).await;
                        let _ = view.update(cx, |this, _cx| {
                            if this.worktree_preview_cache_write_blocked_until_rev
                                == Some(blocked_rev)
                            {
                                this.worktree_preview_cache_write_blocked_until_rev = None;
                            }
                        });
                    },
                )
                .detach();
            }
        }

        self.refresh_worktree_preview_syntax_document(cx);
    }

    pub(in crate::view) fn set_worktree_preview_ready_source(
        &mut self,
        path: std::path::PathBuf,
        source_text: SharedString,
        line_starts: Arc<[usize]>,
        cx: &mut gpui::Context<Self>,
    ) {
        let line_flags = preview_line_flags_from_source(source_text.as_ref(), line_starts.as_ref());
        self.apply_worktree_preview_ready_state(
            path.clone(),
            path,
            source_text.len(),
            source_text,
            line_starts,
            line_flags,
            cx,
        );
    }

    pub(in crate::view) fn set_worktree_preview_ready_materialized_source(
        &mut self,
        display_path: std::path::PathBuf,
        source_path: std::path::PathBuf,
        source_text: SharedString,
        line_starts: Arc<[usize]>,
        line_flags: Arc<[u8]>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.apply_worktree_preview_ready_state(
            display_path,
            source_path,
            source_text.len(),
            source_text,
            line_starts,
            line_flags,
            cx,
        );
    }

    pub(in crate::view) fn set_worktree_preview_ready_indexed_source(
        &mut self,
        display_path: std::path::PathBuf,
        source_path: std::path::PathBuf,
        source_len: usize,
        line_starts: Arc<[usize]>,
        line_flags: Arc<[u8]>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.apply_worktree_preview_ready_state(
            display_path,
            source_path,
            source_len,
            SharedString::default(),
            line_starts,
            line_flags,
            cx,
        );
    }

    pub(in crate::view) fn set_worktree_preview_ready_rows(
        &mut self,
        path: std::path::PathBuf,
        lines: &[String],
        source_len: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let (source_text, line_starts) =
            preview_source_text_and_line_starts_from_lines(lines, source_len);
        self.set_worktree_preview_ready_source(path, source_text, line_starts, cx);
    }

    pub(in crate::view) fn refresh_worktree_preview_syntax_document(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(language) = self.worktree_preview_syntax_language else {
            return;
        };
        let Some(key) = self.worktree_preview_prepared_syntax_key() else {
            return;
        };
        if !matches!(self.worktree_preview, Loadable::Ready(_)) {
            return;
        }
        if self.worktree_preview_text.is_empty() {
            return;
        }
        let source_text = self.worktree_preview_text.clone();
        let line_starts = Arc::clone(&self.worktree_preview_line_starts);

        if self.prepared_syntax_document(&key).is_some() {
            return;
        }
        let reparse_seed = self.prepared_syntax_reparse_seed_document(&key);
        let background_reparse_seed: Option<rows::PreparedDiffSyntaxReparseSeed> =
            reparse_seed.and_then(rows::prepared_diff_syntax_reparse_seed);

        let budget = self.full_document_syntax_budget();
        match rows::prepare_diff_syntax_document_with_budget_reuse_text(
            language,
            FULL_DOCUMENT_SYNTAX_MODE,
            source_text.clone(),
            Arc::clone(&line_starts),
            budget,
            reparse_seed,
            None,
        ) {
            rows::PrepareDiffSyntaxDocumentResult::Ready(document) => {
                self.insert_prepared_syntax_document(key, document);
            }
            rows::PrepareDiffSyntaxDocumentResult::TimedOut => {
                cx.spawn(
                    async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                        let prepare_document = move || {
                            rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                                language,
                                FULL_DOCUMENT_SYNTAX_MODE,
                                source_text,
                                line_starts,
                                background_reparse_seed,
                                None,
                            )
                        };
                        let parsed_document =
                            if crate::ui_runtime::current().uses_background_compute() {
                                smol::unblock(prepare_document).await
                            } else {
                                prepare_document()
                            };

                        let _ = view.update(cx, |this, cx| {
                            let Some(parsed_document) = parsed_document else {
                                return;
                            };

                            let inserted = this.insert_prepared_syntax_document(
                                key.clone(),
                                rows::inject_background_prepared_diff_syntax_document(
                                    parsed_document,
                                ),
                            );
                            if inserted
                                && this.worktree_preview_prepared_syntax_key().as_ref()
                                    == Some(&key)
                            {
                                this.worktree_preview_style_cache_epoch =
                                    this.worktree_preview_style_cache_epoch.wrapping_add(1);
                                cx.notify();
                            }
                        });
                    },
                )
                .detach();
            }
            rows::PrepareDiffSyntaxDocumentResult::Unsupported => {}
        }
    }

    /// Applies a foreground sync prepare result for one side. Returns `true` if
    /// the side needs a background async parse instead.
    fn apply_sync_syntax_result(
        &mut self,
        attempt: Option<rows::PrepareDiffSyntaxDocumentResult>,
        key: &Option<PreparedSyntaxDocumentKey>,
    ) -> SyncFileDiffPreparedSyntaxApplyResult {
        match attempt {
            Some(rows::PrepareDiffSyntaxDocumentResult::Ready(document)) => {
                SyncFileDiffPreparedSyntaxApplyResult {
                    inserted: key.as_ref().is_some_and(|key| {
                        self.insert_prepared_syntax_document(key.clone(), document)
                    }),
                    needs_background_prepare: false,
                }
            }
            Some(rows::PrepareDiffSyntaxDocumentResult::TimedOut) => {
                SyncFileDiffPreparedSyntaxApplyResult {
                    inserted: false,
                    needs_background_prepare: true,
                }
            }
            _ => SyncFileDiffPreparedSyntaxApplyResult::default(),
        }
    }

    /// Applies background-parsed documents for both sides and reports which
    /// side became newly cacheable.
    fn apply_background_syntax_documents(
        &mut self,
        left_key: &Option<PreparedSyntaxDocumentKey>,
        left_doc: Option<rows::BackgroundPreparedDiffSyntaxDocument>,
        right_key: &Option<PreparedSyntaxDocumentKey>,
        right_doc: Option<rows::BackgroundPreparedDiffSyntaxDocument>,
    ) -> FileDiffPreparedSyntaxApplyResult {
        let mut applied = FileDiffPreparedSyntaxApplyResult::default();
        if let (Some(key), Some(document)) = (left_key.as_ref(), left_doc) {
            applied.split_left = self.insert_prepared_syntax_document(
                key.clone(),
                rows::inject_background_prepared_diff_syntax_document(document),
            );
        }
        if let (Some(key), Some(document)) = (right_key.as_ref(), right_doc) {
            applied.split_right = self.insert_prepared_syntax_document(
                key.clone(),
                rows::inject_background_prepared_diff_syntax_document(document),
            );
        }
        applied
    }

    fn refresh_file_diff_syntax_documents(
        &mut self,
        cx: &mut gpui::Context<Self>,
        split_left_reparse_seed_override: Option<rows::PreparedDiffSyntaxDocument>,
        split_right_reparse_seed_override: Option<rows::PreparedDiffSyntaxDocument>,
        split_left_edit_hint: Option<rows::DiffSyntaxEdit>,
        split_right_edit_hint: Option<rows::DiffSyntaxEdit>,
    ) {
        if self.file_diff_old_text.is_empty() && self.file_diff_new_text.is_empty() {
            return;
        }

        let Some(language) = self.file_diff_cache_language else {
            return;
        };

        // Split and inline syntax both project from the real old/new documents.
        // Only those real side documents are parsed here; inline rows later map
        // through old_line/new_line instead of parsing any synthetic diff stream.
        let split_left_key =
            self.file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitLeft);
        let split_right_key =
            self.file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitRight);
        let split_left_reparse_seed = split_left_reparse_seed_override.or_else(|| {
            split_left_key
                .as_ref()
                .and_then(|key| self.prepared_syntax_reparse_seed_document(key))
        });
        let split_right_reparse_seed = split_right_reparse_seed_override.or_else(|| {
            split_right_key
                .as_ref()
                .and_then(|key| self.prepared_syntax_reparse_seed_document(key))
        });

        let needs_split_left_prepare = split_left_key
            .as_ref()
            .is_some_and(|key| self.prepared_syntax_document(key).is_none());
        let needs_split_right_prepare = split_right_key
            .as_ref()
            .is_some_and(|key| self.prepared_syntax_document(key).is_none());
        if !needs_split_left_prepare && !needs_split_right_prepare {
            return;
        }

        let budget = self.full_document_syntax_budget();

        let split_left_attempt = needs_split_left_prepare.then(|| {
            rows::prepare_diff_syntax_document_with_budget_reuse_text(
                language,
                FULL_DOCUMENT_SYNTAX_MODE,
                self.file_diff_old_text.clone(),
                Arc::clone(&self.file_diff_old_line_starts),
                budget,
                split_left_reparse_seed,
                split_left_edit_hint.clone(),
            )
        });
        let split_right_attempt = needs_split_right_prepare.then(|| {
            rows::prepare_diff_syntax_document_with_budget_reuse_text(
                language,
                FULL_DOCUMENT_SYNTAX_MODE,
                self.file_diff_new_text.clone(),
                Arc::clone(&self.file_diff_new_line_starts),
                budget,
                split_right_reparse_seed,
                split_right_edit_hint.clone(),
            )
        });

        let split_left_sync = self.apply_sync_syntax_result(split_left_attempt, &split_left_key);
        let split_right_sync = self.apply_sync_syntax_result(split_right_attempt, &split_right_key);
        let needs_split_left_async = split_left_sync.needs_background_prepare;
        let needs_split_right_async = split_right_sync.needs_background_prepare;

        if split_left_sync.inserted {
            self.file_diff_style_cache_epochs.bump_left();
        }
        if split_right_sync.inserted {
            self.file_diff_style_cache_epochs.bump_right();
        }
        if split_left_sync.inserted || split_right_sync.inserted {
            cx.notify();
        }

        if !needs_split_left_async && !needs_split_right_async {
            return;
        }

        let syntax_generation = self.file_diff_syntax_generation;
        let repo_id = self.file_diff_cache_repo_id;
        let diff_file_rev = self.file_diff_cache_rev;
        let diff_target = self.file_diff_cache_target.clone();

        let split_left_source = needs_split_left_async.then(|| {
            (
                self.file_diff_old_text.clone(),
                Arc::clone(&self.file_diff_old_line_starts),
            )
        });
        let split_left_background_reparse_seed = split_left_reparse_seed
            .filter(|_| needs_split_left_async)
            .and_then(rows::prepared_diff_syntax_reparse_seed);
        let split_left_edit_hint = split_left_edit_hint.filter(|_| needs_split_left_async);
        let split_right_source = needs_split_right_async.then(|| {
            (
                self.file_diff_new_text.clone(),
                Arc::clone(&self.file_diff_new_line_starts),
            )
        });
        let split_right_background_reparse_seed = split_right_reparse_seed
            .filter(|_| needs_split_right_async)
            .and_then(rows::prepared_diff_syntax_reparse_seed);
        let split_right_edit_hint = split_right_edit_hint.filter(|_| needs_split_right_async);

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let prepare_documents = move || FileDiffBackgroundPreparedSyntaxDocuments {
                    split_left: split_left_source.and_then(|(text, line_starts)| {
                        rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                            language,
                            FULL_DOCUMENT_SYNTAX_MODE,
                            text,
                            line_starts,
                            split_left_background_reparse_seed,
                            split_left_edit_hint,
                        )
                    }),
                    split_right: split_right_source.and_then(|(text, line_starts)| {
                        rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                            language,
                            FULL_DOCUMENT_SYNTAX_MODE,
                            text,
                            line_starts,
                            split_right_background_reparse_seed,
                            split_right_edit_hint,
                        )
                    }),
                };
                let parsed_documents = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(prepare_documents).await
                } else {
                    prepare_documents()
                };

                let _ = view.update(cx, |this, cx| {
                    if this.file_diff_syntax_generation != syntax_generation {
                        return;
                    }
                    if this.file_diff_cache_repo_id != repo_id
                        || this.file_diff_cache_rev != diff_file_rev
                        || this.file_diff_cache_target != diff_target
                    {
                        return;
                    }

                    let applied = this.apply_background_syntax_documents(
                        &split_left_key,
                        parsed_documents.split_left,
                        &split_right_key,
                        parsed_documents.split_right,
                    );

                    if applied.any() {
                        if applied.split_left {
                            this.file_diff_style_cache_epochs.bump_left();
                        }
                        if applied.split_right {
                            this.file_diff_style_cache_epochs.bump_right();
                        }
                        cx.notify();
                    }
                });
            },
        )
        .detach();
    }

    /// Resets file-diff data fields (syntax, rows, text, highlights) without
    /// touching the identity fields (repo_id, target, rev).
    fn reset_file_diff_cache_data(&mut self) {
        self.reset_collapsed_diff_projection(false);
        self.file_diff_cache_content_signature = None;
        self.file_diff_cache_inflight = None;
        self.file_diff_syntax_generation = self.file_diff_syntax_generation.wrapping_add(1);
        self.file_diff_style_cache_epochs.bump_both();
        self.file_diff_cache_path = None;
        self.file_diff_cache_language = None;
        self.file_diff_cache_rows.clear();
        self.file_diff_row_provider = None;
        self.file_diff_old_text = SharedString::default();
        self.file_diff_old_line_starts = Arc::default();
        self.file_diff_old_line_to_row = Arc::default();
        self.file_diff_old_line_to_inline_row = Arc::default();
        self.file_diff_new_text = SharedString::default();
        self.file_diff_new_line_starts = Arc::default();
        self.file_diff_new_line_to_row = Arc::default();
        self.file_diff_new_line_to_inline_row = Arc::default();
        self.file_diff_inline_cache.clear();
        self.file_diff_inline_row_provider = None;
        self.file_diff_inline_text = SharedString::default();
        self.file_diff_inline_word_highlights =
            rows::new_lru_cache(FILE_DIFF_WORD_HIGHLIGHT_CACHE_MAX_ENTRIES);
        self.file_diff_split_word_highlights =
            rows::new_lru_cache(FILE_DIFF_WORD_HIGHLIGHT_CACHE_MAX_ENTRIES);
    }

    pub(in super::super::super) fn ensure_file_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((
            repo_id,
            diff_file_rev,
            diff_target,
            workdir,
            expected_abs_path,
            file,
            patch_diff,
        )) = (|| {
            let (repo_id, diff_file_rev, diff_target, workdir, expected_abs_path) =
                self.rendered_file_diff_identity()?;
            let file: Option<Arc<gitcomet_core::domain::FileDiffText>> =
                match self.rendered_file_diff_loadable()? {
                    Loadable::Ready(Some(file)) => Some(Arc::clone(file)),
                    _ => None,
                };
            let patch_diff: Option<Arc<gitcomet_core::domain::Diff>> =
                match self.rendered_patch_diff_loadable()? {
                    Loadable::Ready(diff) => Some(Arc::clone(diff)),
                    _ => None,
                };

            Some((
                repo_id,
                diff_file_rev,
                diff_target,
                workdir,
                expected_abs_path,
                file,
                patch_diff,
            ))
        })()
        else {
            self.file_diff_cache_repo_id = None;
            self.file_diff_cache_target = None;
            self.file_diff_cache_rev = 0;
            self.reset_file_diff_cache_data();
            return;
        };

        let diff_target_for_task = diff_target.clone();
        let file_content_signature = file.as_ref().map(|file| {
            let mut signature = file_diff_text_signature(file.as_ref());
            if let Some(patch_diff) = patch_diff.as_ref() {
                signature ^= patch_diff_content_signature(patch_diff.as_ref()).rotate_left(1);
            }
            signature
        });
        let same_repo_and_target = self.file_diff_cache_repo_id == Some(repo_id)
            && self.file_diff_cache_target == Some(diff_target.clone())
            && self.file_diff_cache_path.as_ref() == Some(&expected_abs_path);
        let previous_split_left_reparse_seed = same_repo_and_target
            .then(|| self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft))
            .flatten();
        let previous_split_right_reparse_seed = same_repo_and_target
            .then(|| self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight))
            .flatten();
        let previous_old_text = same_repo_and_target.then(|| self.file_diff_old_text.clone());
        let previous_new_text = same_repo_and_target.then(|| self.file_diff_new_text.clone());

        if same_repo_and_target && self.file_diff_cache_rev == diff_file_rev {
            return;
        }

        if same_repo_and_target
            && let Some(signature) = file_content_signature
            && self.file_diff_cache_content_signature == Some(signature)
        {
            // Store-side refreshes can bump diff_file_rev with identical file payloads.
            // Keep the row cache and prepared syntax documents alive across rev-only refreshes.
            // Any older row rebuild is now redundant because the current rows already match
            // the active content signature.
            self.file_diff_cache_inflight = None;
            self.rekey_file_diff_prepared_syntax_documents_for_rev(diff_file_rev);
            self.file_diff_cache_rev = diff_file_rev;
            self.refresh_file_diff_syntax_documents(cx, None, None, None, None);
            return;
        }

        self.file_diff_cache_repo_id = Some(repo_id);
        self.file_diff_cache_rev = diff_file_rev;
        self.file_diff_cache_target = Some(diff_target);
        self.reset_file_diff_cache_data();

        // Reset the segment cache to avoid mixing patch/file indices.
        self.clear_diff_text_style_caches();

        let Some(file) = file else {
            return;
        };
        let content_signature =
            file_content_signature.unwrap_or_else(|| file_diff_text_signature(file.as_ref()));

        self.file_diff_cache_seq = self.file_diff_cache_seq.wrapping_add(1);
        let seq = self.file_diff_cache_seq;
        self.file_diff_cache_inflight = Some(seq);
        self.file_diff_syntax_generation = seq;

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let rebuild_cache = move || {
                    build_file_diff_cache_rebuild_with_patch(
                        file.as_ref(),
                        &workdir,
                        patch_diff.as_deref(),
                    )
                };
                let rebuild = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(rebuild_cache).await
                } else {
                    rebuild_cache()
                };

                let _ = view.update(cx, |this, cx| {
                    if this.file_diff_cache_inflight != Some(seq) {
                        return;
                    }
                    if this.file_diff_cache_repo_id != Some(repo_id)
                        || this.file_diff_cache_rev != diff_file_rev
                        || this.file_diff_cache_target != Some(diff_target_for_task.clone())
                    {
                        return;
                    }

                    this.file_diff_cache_inflight = None;
                    this.file_diff_cache_path = rebuild.file_path;
                    this.file_diff_cache_language = rebuild.language;
                    this.file_diff_row_provider = Some(rebuild.row_provider);
                    this.file_diff_old_text = rebuild.old_text;
                    this.file_diff_old_line_starts = rebuild.old_line_starts;
                    this.file_diff_old_line_to_row = rebuild.old_line_to_row;
                    this.file_diff_old_line_to_inline_row = rebuild.old_line_to_inline_row;
                    this.file_diff_new_text = rebuild.new_text;
                    this.file_diff_new_line_starts = rebuild.new_line_starts;
                    this.file_diff_new_line_to_row = rebuild.new_line_to_row;
                    this.file_diff_new_line_to_inline_row = rebuild.new_line_to_inline_row;
                    this.file_diff_inline_row_provider = Some(rebuild.inline_row_provider);
                    this.file_diff_inline_text = rebuild.inline_text;
                    this.file_diff_cache_content_signature = Some(content_signature);
                    #[cfg(test)]
                    {
                        this.file_diff_cache_rows = rebuild.rows;
                        this.file_diff_inline_cache = rebuild.inline_rows;
                    }
                    let split_left_edit_hint = previous_old_text.as_ref().and_then(|previous| {
                        diff_syntax_edit_from_text_change(
                            previous.as_ref(),
                            this.file_diff_old_text.as_ref(),
                        )
                    });
                    let split_right_edit_hint = previous_new_text.as_ref().and_then(|previous| {
                        diff_syntax_edit_from_text_change(
                            previous.as_ref(),
                            this.file_diff_new_text.as_ref(),
                        )
                    });
                    this.refresh_file_diff_syntax_documents(
                        cx,
                        previous_split_left_reparse_seed,
                        previous_split_right_reparse_seed,
                        split_left_edit_hint,
                        split_right_edit_hint,
                    );

                    // Reset the segment cache to avoid mixing patch/file indices.
                    this.clear_diff_text_style_caches();
                    cx.notify();
                });
            },
        )
        .detach();
    }

    pub(in super::super::super) fn ensure_file_markdown_preview_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let clear_cache = |this: &mut Self| {
            this.file_markdown_preview_cache_repo_id = None;
            this.file_markdown_preview_cache_target = None;
            this.file_markdown_preview_cache_rev = 0;
            this.file_markdown_preview_cache_content_signature = None;
            this.file_markdown_preview = Loadable::NotLoaded;
            this.file_markdown_preview_inflight = None;
        };

        let Some((repo_id, diff_file_rev, diff_target, expected_abs_path, file)) = (|| {
            let (repo_id, diff_file_rev, diff_target, _workdir, expected_abs_path) =
                self.rendered_file_diff_identity()?;
            let file: Option<Arc<gitcomet_core::domain::FileDiffText>> =
                match self.rendered_file_diff_loadable()? {
                    Loadable::Ready(Some(file)) => Some(Arc::clone(file)),
                    _ => None,
                };

            Some((repo_id, diff_file_rev, diff_target, expected_abs_path, file))
        })() else {
            clear_cache(self);
            return;
        };

        let diff_target_for_task = diff_target.clone();
        let file_content_signature = file
            .as_ref()
            .map(|file| file_diff_text_signature(file.as_ref()));
        let same_repo_and_target = self.file_markdown_preview_cache_repo_id == Some(repo_id)
            && self.file_markdown_preview_cache_target == Some(diff_target.clone())
            && self.file_diff_cache_path.as_ref() == Some(&expected_abs_path);

        if same_repo_and_target && self.file_markdown_preview_cache_rev == diff_file_rev {
            return;
        }

        if same_repo_and_target
            && let Some(signature) = file_content_signature
            && self.file_markdown_preview_cache_content_signature == Some(signature)
        {
            if self.file_markdown_preview_inflight.is_none() {
                self.file_markdown_preview_cache_rev = diff_file_rev;
            }
            return;
        }

        self.file_markdown_preview_cache_repo_id = Some(repo_id);
        self.file_markdown_preview_cache_rev = diff_file_rev;
        self.file_markdown_preview_cache_content_signature = None;
        self.file_markdown_preview_cache_target = Some(diff_target);
        self.file_markdown_preview = Loadable::NotLoaded;
        self.file_markdown_preview_inflight = None;

        let Some(file) = file else {
            return;
        };
        // `file` was `Some` when `file_content_signature` was computed, so unwrap is safe.
        let content_signature = file_content_signature.unwrap();
        let old_source = file.old_source.clone();
        let new_source = file.new_source.clone();
        let old_legacy_text = file.old.clone();
        let new_legacy_text = file.new.clone();

        let combined_len =
            file_diff_markdown_source_len(old_source.as_ref(), old_legacy_text.as_ref())
                + file_diff_markdown_source_len(new_source.as_ref(), new_legacy_text.as_ref());
        if combined_len > markdown_preview::MAX_DIFF_PREVIEW_SOURCE_BYTES {
            self.file_markdown_preview = Loadable::Error(
                markdown_preview::diff_preview_unavailable_reason(combined_len).to_string(),
            );
            self.file_markdown_preview_cache_content_signature = Some(content_signature);
            return;
        }

        self.file_markdown_preview = Loadable::Loading;
        self.file_markdown_preview_seq = self.file_markdown_preview_seq.wrapping_add(1);
        let seq = self.file_markdown_preview_seq;
        self.file_markdown_preview_inflight = Some(seq);

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let build_preview = move || {
                    let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewParse);
                    let old_source = read_file_diff_markdown_source(
                        old_source.as_ref(),
                        old_legacy_text.as_ref(),
                    )?;
                    let new_source = read_file_diff_markdown_source(
                        new_source.as_ref(),
                        new_legacy_text.as_ref(),
                    )?;
                    markdown_preview::build_markdown_diff_preview(
                        old_source.as_ref(),
                        new_source.as_ref(),
                    )
                    .map(Arc::new)
                    .ok_or_else(|| {
                        markdown_preview::diff_preview_unavailable_reason(
                            old_source.len() + new_source.len(),
                        )
                        .to_string()
                    })
                };
                let result = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(build_preview).await
                } else {
                    build_preview()
                };

                let _ = view.update(cx, |this, cx| {
                    if this.file_markdown_preview_inflight != Some(seq) {
                        return;
                    }
                    if this.file_markdown_preview_cache_repo_id != Some(repo_id)
                        || this.file_markdown_preview_cache_rev != diff_file_rev
                        || this.file_markdown_preview_cache_target
                            != Some(diff_target_for_task.clone())
                    {
                        return;
                    }

                    this.file_markdown_preview_inflight = None;
                    this.file_markdown_preview_cache_content_signature = Some(content_signature);
                    match result {
                        Ok(preview) => this.file_markdown_preview = Loadable::Ready(preview),
                        Err(error) => this.file_markdown_preview = Loadable::Error(error),
                    }
                    cx.notify();
                });
            },
        )
        .detach();
    }

    pub(in super::super::super) fn rebuild_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        let next_cache_state = self.active_repo().map(|repo| {
            let workdir: Option<std::path::PathBuf> = self
                .rendered_diff_workdir()
                .map(std::path::Path::to_path_buf);
            let diff = match self.rendered_patch_diff_loadable() {
                Some(Loadable::Ready(diff)) => Some(Arc::clone(diff)),
                _ => None,
            };
            (
                repo.id,
                self.rendered_patch_diff_rev(),
                self.rendered_diff_target().cloned(),
                workdir,
                diff,
            )
        });
        let next_content_signature = next_cache_state
            .as_ref()
            .and_then(|(_, _, _, _, diff)| diff.as_ref())
            .map(|diff| patch_diff_content_signature(diff.as_ref()));
        let clear_reveals = match next_cache_state.as_ref() {
            Some((repo_id, _, diff_target, _, Some(_))) if diff_target.is_some() => {
                self.diff_cache_repo_id != Some(*repo_id)
                    || self.diff_cache_target.as_ref() != diff_target.as_ref()
                    || self.diff_cache_content_signature != next_content_signature
            }
            _ => true,
        };

        self.reset_collapsed_diff_projection(clear_reveals);
        self.diff_cache.clear();
        self.diff_row_provider = None;
        self.diff_split_row_provider = None;
        self.diff_cache_repo_id = None;
        self.diff_cache_rev = 0;
        self.diff_cache_content_signature = None;
        self.diff_cache_target = None;
        self.diff_file_for_src_ix.clear();
        self.diff_language_for_src_ix.clear();
        self.diff_yaml_block_scalar_for_src_ix.clear();
        self.diff_click_kinds.clear();
        self.diff_line_kind_for_src_ix.clear();
        self.diff_hide_unified_header_for_src_ix.clear();
        self.diff_header_display_cache.clear();
        self.diff_split_cache.clear();
        self.diff_split_cache_len = 0;
        self.diff_visible_indices.clear();
        self.diff_visible_inline_map = None;
        self.diff_visible_cache_len = 0;
        self.diff_visible_is_file_view = false;
        self.diff_scrollbar_markers_cache.clear();
        self.diff_word_highlights.clear();
        self.diff_word_highlights_inflight = None;
        self.diff_file_stats.clear();
        self.clear_diff_text_style_caches();
        self.diff_selection_anchor = None;
        self.diff_selection_range = None;
        self.diff_preview_is_new_file = false;

        let Some((repo_id, diff_rev, diff_target, workdir, diff)) = next_cache_state else {
            return;
        };

        self.diff_cache_repo_id = Some(repo_id);
        self.diff_cache_rev = diff_rev;
        self.diff_cache_content_signature = next_content_signature;
        self.diff_cache_target = diff_target;

        let Some(diff) = diff else {
            return;
        };
        let Some(workdir) = workdir else {
            return;
        };

        let row_provider = Arc::new(PagedPatchDiffRows::new(
            Arc::clone(&diff),
            PATCH_DIFF_PAGE_SIZE,
        ));
        let mut split_row_count = 0usize;
        let mut pending_split_removes = 0usize;
        let mut pending_split_adds = 0usize;
        self.diff_row_provider = Some(row_provider);

        self.diff_file_for_src_ix = compute_diff_file_for_src_ix(diff.lines.as_slice());
        self.diff_line_kind_for_src_ix = diff
            .lines
            .iter()
            .map(|line| {
                match line.kind {
                    gitcomet_core::domain::DiffLineKind::Remove => pending_split_removes += 1,
                    gitcomet_core::domain::DiffLineKind::Add => pending_split_adds += 1,
                    gitcomet_core::domain::DiffLineKind::Context
                    | gitcomet_core::domain::DiffLineKind::Header
                    | gitcomet_core::domain::DiffLineKind::Hunk => {
                        split_row_count += pending_split_removes.max(pending_split_adds) + 1;
                        pending_split_removes = 0;
                        pending_split_adds = 0;
                    }
                }
                line.kind
            })
            .collect();
        split_row_count += pending_split_removes.max(pending_split_adds);
        self.diff_split_row_provider = Some(Arc::new(PagedPatchSplitRows::new_with_len_hint(
            Arc::clone(self.diff_row_provider.as_ref().expect("set just above")),
            split_row_count,
        )));
        self.diff_hide_unified_header_for_src_ix = diff
            .lines
            .iter()
            .map(|line| should_hide_unified_diff_header_raw(line.kind, line.text.as_ref()))
            .collect();
        self.diff_click_kinds = diff
            .lines
            .iter()
            .map(|line| {
                if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk) {
                    DiffClickKind::HunkHeader
                } else if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
                    && line.text.starts_with("diff --git ")
                {
                    DiffClickKind::FileHeader
                } else {
                    DiffClickKind::Line
                }
            })
            .collect();
        for (src_ix, click_kind) in self.diff_click_kinds.iter().enumerate() {
            match click_kind {
                DiffClickKind::FileHeader => {
                    let Some(line) = diff.lines.get(src_ix) else {
                        continue;
                    };
                    let display = parse_diff_git_header_path(line.text.as_ref())
                        .unwrap_or_else(|| line.text.as_ref().to_string());
                    self.diff_header_display_cache
                        .insert(src_ix, display.into());
                }
                DiffClickKind::HunkHeader => {
                    let Some(line) = diff.lines.get(src_ix) else {
                        continue;
                    };
                    let display = parse_unified_hunk_header_for_display(line.text.as_ref())
                        .map(|p| {
                            let heading = p.heading.unwrap_or_default();
                            if heading.is_empty() {
                                format!("{} {}", p.old, p.new)
                            } else {
                                format!("{} {}  {heading}", p.old, p.new)
                            }
                        })
                        .unwrap_or_else(|| line.text.as_ref().to_string());
                    self.diff_header_display_cache
                        .insert(src_ix, display.into());
                }
                DiffClickKind::Line => {}
            }
        }
        self.diff_file_stats = compute_diff_file_stats(diff.lines.as_slice());
        self.diff_word_highlights = vec![None; self.patch_diff_row_len()];
        self.diff_word_highlights_inflight = None;

        let mut current_file: Option<Arc<str>> = None;
        let mut current_language: Option<rows::DiffSyntaxLanguage> = None;
        for (src_ix, line) in diff.lines.iter().enumerate() {
            let file = self
                .diff_file_for_src_ix
                .get(src_ix)
                .and_then(|p| p.as_ref());
            let file_changed = match (&current_file, file) {
                (Some(cur), Some(next)) => !Arc::ptr_eq(cur, next),
                (None, None) => false,
                _ => true,
            };
            if file_changed {
                current_file = file.cloned();
                current_language =
                    file.and_then(|p| rows::diff_syntax_language_for_path(p.as_ref()));
            }

            let language = match line.kind {
                gitcomet_core::domain::DiffLineKind::Add
                | gitcomet_core::domain::DiffLineKind::Remove
                | gitcomet_core::domain::DiffLineKind::Context => current_language,
                gitcomet_core::domain::DiffLineKind::Header
                | gitcomet_core::domain::DiffLineKind::Hunk => None,
            };
            self.diff_language_for_src_ix.push(language);
        }
        self.diff_yaml_block_scalar_for_src_ix = compute_diff_yaml_block_scalar_for_src_ix(
            diff.lines.as_slice(),
            self.diff_file_for_src_ix.as_slice(),
            self.diff_language_for_src_ix.as_slice(),
        );
        if let Some(preview) = build_new_file_preview_from_diff(
            diff.lines.as_slice(),
            &workdir,
            self.diff_cache_target.as_ref(),
        ) {
            self.diff_preview_is_new_file = true;
            self.set_worktree_preview_ready_rows(
                preview.abs_path,
                preview.lines.as_slice(),
                preview.source_len,
                cx,
            );
            self.worktree_preview_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        }
    }

    fn ensure_diff_split_cache(&mut self) {
        if self.diff_split_row_provider.is_some() {
            return;
        }
        if self.diff_split_cache_len == self.diff_cache.len() && !self.diff_split_cache.is_empty() {
            return;
        }
        self.diff_split_cache_len = self.diff_cache.len();
        self.diff_split_cache = build_patch_split_rows(&self.diff_cache);
    }

    fn diff_scrollbar_markers_patch(&self) -> Vec<components::ScrollbarMarker> {
        match self.diff_view {
            DiffViewMode::Inline => {
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(src_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    let Some(line) = self.patch_diff_row(src_ix) else {
                        return 0;
                    };
                    match line.kind {
                        gitcomet_core::domain::DiffLineKind::Add => 1,
                        gitcomet_core::domain::DiffLineKind::Remove => 2,
                        _ => 0,
                    }
                })
            }
            DiffViewMode::Split => {
                if self.diff_split_row_provider.is_some() {
                    let meta = self.patch_split_visible_meta_from_source();
                    debug_assert_eq!(meta.visible_indices.as_slice(), self.diff_visible_indices);
                    return scrollbar_markers_from_visible_flags(meta.visible_flags.as_slice());
                }
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(row_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    let Some(row) = self.patch_diff_split_row(row_ix) else {
                        return 0;
                    };
                    match &row {
                        PatchSplitRow::Aligned { row, .. } => match row.kind {
                            gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                            gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                            gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                            gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                        },
                        PatchSplitRow::Raw { .. } => 0,
                    }
                })
            }
        }
    }

    fn collapsed_diff_hunk_marker_flag(hunk: CollapsedDiffHunk) -> u8 {
        match (hunk.has_additions, hunk.has_removals) {
            (true, true) => 3,
            (true, false) => 1,
            (false, true) => 2,
            (false, false) => 0,
        }
    }

    fn collapsed_diff_hunk_visible_file_bounds(
        &self,
        hunk_ix: usize,
        hunk: CollapsedDiffHunk,
    ) -> Option<(usize, usize)> {
        let mut visible_ix = *self.collapsed_diff_hunk_visible_indices.get(hunk_ix)?;
        while let Some(row) = self.collapsed_diff_visible_rows.get(visible_ix).copied() {
            match row {
                CollapsedDiffVisibleRow::HunkHeader { .. } => visible_ix += 1,
                CollapsedDiffVisibleRow::FileRow { row_ix } if row_ix < hunk.base_row_start => {
                    visible_ix += 1;
                }
                CollapsedDiffVisibleRow::FileRow { row_ix } if row_ix == hunk.base_row_start => {
                    let end_ix = visible_ix
                        .saturating_add(
                            hunk.base_row_end_exclusive
                                .saturating_sub(hunk.base_row_start),
                        )
                        .min(self.collapsed_diff_visible_rows.len());
                    return (visible_ix < end_ix).then_some((visible_ix, end_ix));
                }
                CollapsedDiffVisibleRow::FileRow { .. } => return None,
            }
        }
        None
    }

    fn diff_scrollbar_markers_collapsed(&self) -> Vec<components::ScrollbarMarker> {
        let ranges = self
            .collapsed_diff_hunks
            .iter()
            .enumerate()
            .filter_map(|(hunk_ix, hunk)| {
                let flag = Self::collapsed_diff_hunk_marker_flag(*hunk);
                let (start, end) = self.collapsed_diff_hunk_visible_file_bounds(hunk_ix, *hunk)?;
                Some((start, end, flag))
            });
        scrollbar_markers_from_visible_ranges(self.diff_visible_len(), ranges)
    }

    fn compute_diff_scrollbar_markers(&self) -> Vec<components::ScrollbarMarker> {
        if self.is_collapsed_diff_projection_active() {
            return self.diff_scrollbar_markers_collapsed();
        }

        if !self.is_file_diff_view_active() {
            return self.diff_scrollbar_markers_patch();
        }

        match self.diff_view {
            DiffViewMode::Inline => {
                if let Some(provider) = self.file_diff_inline_row_provider.as_ref() {
                    return provider.scrollbar_markers();
                }
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(inline_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    let Some(line) = self.file_diff_inline_cache.get(inline_ix) else {
                        return 0;
                    };
                    match line.kind {
                        gitcomet_core::domain::DiffLineKind::Add => 1,
                        gitcomet_core::domain::DiffLineKind::Remove => 2,
                        _ => 0,
                    }
                })
            }
            DiffViewMode::Split => {
                if let Some(provider) = self.file_diff_row_provider.as_ref() {
                    return provider.scrollbar_markers();
                }
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(row_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    let Some(row) = self.file_diff_cache_rows.get(row_ix) else {
                        return 0;
                    };
                    match row.kind {
                        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                    }
                })
            }
        }
    }

    pub(in super::super::super) fn ensure_diff_visible_indices(&mut self) {
        let is_file_view = self.is_file_diff_view_active();
        let collapsed_projection_active = self.is_collapsed_diff_projection_active();
        let projection_rev = if collapsed_projection_active {
            self.diff_visible_projection_rev
        } else {
            0
        };
        let needs_collapsed_rebuild = collapsed_projection_active
            && (self.diff_visible_cache_projection_rev != projection_rev
                || self.diff_visible_view != self.diff_view
                || self.diff_visible_is_file_view != is_file_view);
        if needs_collapsed_rebuild {
            self.rebuild_collapsed_diff_projection();
        }

        let current_len = if collapsed_projection_active {
            self.collapsed_diff_visible_rows.len()
        } else if is_file_view {
            match self.diff_view {
                DiffViewMode::Inline => self.file_diff_inline_row_len(),
                DiffViewMode::Split => self.file_diff_split_row_len(),
            }
        } else {
            match self.diff_view {
                DiffViewMode::Inline => self.patch_diff_row_len(),
                DiffViewMode::Split => self.patch_diff_split_row_len(),
            }
        };

        if self.diff_visible_cache_len == current_len
            && self.diff_visible_view == self.diff_view
            && self.diff_visible_is_file_view == is_file_view
            && self.diff_visible_cache_projection_rev == projection_rev
        {
            return;
        }

        self.diff_visible_cache_len = current_len;
        self.diff_visible_view = self.diff_view;
        self.diff_visible_is_file_view = is_file_view;
        self.diff_visible_cache_projection_rev = projection_rev;
        self.diff_horizontal_min_width = px(0.0);
        self.diff_visible_inline_map = None;
        self.diff_search_inline_patch_trigram_index = None;

        if collapsed_projection_active {
            self.diff_visible_indices.clear();
            self.diff_scrollbar_markers_cache = self.compute_diff_scrollbar_markers();
            if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
                self.diff_search_recompute_matches_for_current_view();
            }
            return;
        }

        if is_file_view {
            self.diff_visible_indices = (0..current_len).collect();
            self.diff_scrollbar_markers_cache = self.compute_diff_scrollbar_markers();
            if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
                self.diff_search_recompute_matches_for_current_view();
            }
            return;
        }

        let mut split_visible_flags: Option<Vec<u8>> = None;
        match self.diff_view {
            DiffViewMode::Inline => {
                if self.diff_hide_unified_header_for_src_ix.len() == current_len {
                    self.diff_visible_inline_map = Some(PatchInlineVisibleMap::from_hidden_flags(
                        self.diff_hide_unified_header_for_src_ix.as_slice(),
                    ));
                    self.diff_visible_indices = Vec::new();
                } else {
                    self.diff_visible_indices = self
                        .patch_diff_rows_slice(0, current_len)
                        .into_iter()
                        .enumerate()
                        .filter_map(|(ix, line)| {
                            (!should_hide_unified_diff_header_line(&line)).then_some(ix)
                        })
                        .collect();
                }
            }
            DiffViewMode::Split => {
                if self.diff_split_row_provider.is_some() {
                    let meta = self.patch_split_visible_meta_from_source();
                    debug_assert_eq!(meta.total_rows, current_len);
                    self.diff_visible_indices = meta.visible_indices;
                    split_visible_flags = Some(meta.visible_flags);
                } else {
                    self.ensure_diff_split_cache();

                    self.diff_visible_indices = self
                        .diff_split_cache
                        .iter()
                        .enumerate()
                        .filter_map(|(ix, row)| match row {
                            PatchSplitRow::Raw { src_ix, .. } => self
                                .diff_cache
                                .get(*src_ix)
                                .is_some_and(|line| !should_hide_unified_diff_header_line(line))
                                .then_some(ix),
                            PatchSplitRow::Aligned { .. } => Some(ix),
                        })
                        .collect();
                }
            }
        }

        self.diff_scrollbar_markers_cache = split_visible_flags
            .map(|flags| scrollbar_markers_from_visible_flags(flags.as_slice()))
            .unwrap_or_else(|| self.compute_diff_scrollbar_markers());

        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches_for_current_view();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{DiffArea, DiffTarget};
    use std::path::Path;
    use std::path::PathBuf;

    #[test]
    fn preview_source_text_from_lines_preserves_missing_trailing_newline() {
        let lines = vec![
            "fn main() {".to_string(),
            "    42".to_string(),
            "}".to_string(),
        ];
        let source_len = "fn main() {\n    42\n}".len();

        let source = preview_source_text_from_lines(&lines, source_len);
        let (_, line_starts) = preview_source_text_and_line_starts_from_lines(&lines, source_len);

        assert_eq!(source.as_ref(), "fn main() {\n    42\n}");
        assert_eq!(line_starts.as_ref(), &[0, 12, 19]);
    }

    #[test]
    fn preview_source_text_from_lines_restores_trailing_newline() {
        let lines = vec!["alpha".to_string(), "beta".to_string()];
        let source_len = "alpha\nbeta\n".len();

        let source = preview_source_text_from_lines(&lines, source_len);
        let (_, line_starts) = preview_source_text_and_line_starts_from_lines(&lines, source_len);

        assert_eq!(source.as_ref(), "alpha\nbeta\n");
        assert_eq!(line_starts.as_ref(), &[0, 6, 11]);
    }

    #[test]
    fn full_document_syntax_mode_is_always_auto() {
        assert_eq!(FULL_DOCUMENT_SYNTAX_MODE, rows::DiffSyntaxMode::Auto);
    }

    #[test]
    fn file_diff_style_cache_epochs_map_rows_to_matching_side() {
        let epochs = FileDiffStyleCacheEpochs {
            split_left: 11,
            split_right: 23,
        };

        assert_eq!(
            epochs.split_epoch(crate::view::DiffTextRegion::SplitLeft),
            11
        );
        assert_eq!(
            epochs.split_epoch(crate::view::DiffTextRegion::SplitRight),
            23
        );
        assert_eq!(
            epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Remove),
            11
        );
        assert_eq!(
            epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Add),
            23
        );
        assert_eq!(
            epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Context),
            23
        );
        assert_eq!(
            epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Header),
            0
        );
        assert_eq!(
            epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Hunk),
            0
        );
    }

    #[test]
    fn build_single_markdown_preview_document_reports_row_limit() {
        let preview_lines =
            vec!["---\n".repeat(crate::view::markdown_preview::MAX_PREVIEW_ROWS + 1)];
        let source = preview_source_text_from_lines(
            &preview_lines,
            preview_lines_source_len(&preview_lines),
        );
        assert!(source.len() < crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES);

        let error = build_single_markdown_preview_document(source.as_ref())
            .expect_err("row-limit markdown preview should return an error");
        assert!(
            error.contains("row limit"),
            "row-limit markdown preview should mention the rendered row limit: {error}"
        );
    }

    #[test]
    fn file_diff_style_cache_epochs_bump_only_changed_side() {
        let mut epochs = FileDiffStyleCacheEpochs {
            split_left: 5,
            split_right: 9,
        };

        epochs.bump_left();
        assert_eq!(
            epochs,
            FileDiffStyleCacheEpochs {
                split_left: 6,
                split_right: 9,
            }
        );

        epochs.bump_right();
        assert_eq!(
            epochs,
            FileDiffStyleCacheEpochs {
                split_left: 6,
                split_right: 10,
            }
        );

        epochs.bump_both();
        assert_eq!(
            epochs,
            FileDiffStyleCacheEpochs {
                split_left: 7,
                split_right: 11,
            }
        );
    }

    #[test]
    fn build_single_markdown_preview_document_respects_exact_source_length() {
        let mut source = "x".repeat(crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES);
        source.push('\n');
        assert_eq!(
            source.len(),
            crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES + 1
        );

        let error = build_single_markdown_preview_document(&source)
            .expect_err("exact source length over the cap should return an error");
        assert!(
            error.contains("1 MiB"),
            "exact-size markdown preview should mention the size limit: {error}"
        );
    }

    #[test]
    fn build_single_markdown_preview_document_from_deleted_markdown_table_preview_parses() {
        let diff = vec![
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "diff --git a/docs/table.md b/docs/table.md".into(),
                old_line: None,
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "deleted file mode 100644".into(),
                old_line: None,
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: "-| **Header Bold** | B |".into(),
                old_line: Some(1),
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: "-| --- | --- |".into(),
                old_line: Some(2),
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: "-| [link](https://example.com) | plain |".into(),
                old_line: Some(3),
                new_line: None,
            },
        ];
        let workdir = PathBuf::from("repo");
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("docs/table.md"),
            area: DiffArea::Unstaged,
        };

        let preview = crate::view::diff_preview::build_deleted_file_preview_from_diff(
            &diff,
            &workdir,
            Some(&target),
        )
        .expect("deleted markdown preview should reconstruct from diff");
        let source = preview_source_text_from_lines(&preview.lines, preview.source_len);
        let document = build_single_markdown_preview_document(source.as_ref())
            .expect("deleted markdown table preview should parse");
        let table_rows = document
            .rows
            .iter()
            .filter(|row| {
                matches!(
                    row.kind,
                    crate::view::markdown_preview::MarkdownPreviewRowKind::TableRow { .. }
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(table_rows.len(), 2);
        assert_eq!(table_rows[0].text.as_ref(), "Header Bold | B");
        assert_eq!(table_rows[1].text.as_ref(), "link        | plain");
    }

    #[test]
    fn prepared_syntax_document_key_includes_repo_rev_path_and_view_mode() {
        let path = Path::new("src/lib.rs");
        let base = prepared_syntax_document_key(
            RepoId(7),
            42,
            path,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        );
        let different_rev = prepared_syntax_document_key(
            RepoId(7),
            43,
            path,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        );
        let different_view_mode = prepared_syntax_document_key(
            RepoId(7),
            42,
            path,
            PreparedSyntaxViewMode::FileDiffSplitLeft,
        );
        let different_repo = prepared_syntax_document_key(
            RepoId(8),
            42,
            path,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        );
        let different_path = prepared_syntax_document_key(
            RepoId(7),
            42,
            Path::new("src/main.rs"),
            PreparedSyntaxViewMode::FileDiffSplitRight,
        );

        assert_ne!(base, different_rev);
        assert_ne!(base, different_view_mode);
        assert_ne!(base, different_repo);
        assert_ne!(base, different_path);
    }

    #[test]
    fn diff_syntax_edit_identical_texts_returns_none() {
        assert!(diff_syntax_edit_from_text_change("hello world", "hello world").is_none());
        assert!(diff_syntax_edit_from_text_change("", "").is_none());
    }

    #[test]
    fn diff_syntax_edit_completely_different_texts() {
        let edit = diff_syntax_edit_from_text_change("abc", "xyz").unwrap();
        assert_eq!(edit.old_range, 0..3);
        assert_eq!(edit.new_range, 0..3);
    }

    #[test]
    fn diff_syntax_edit_shared_prefix() {
        let edit = diff_syntax_edit_from_text_change("hello world", "hello rust").unwrap();
        assert_eq!(edit.old_range, 6..11);
        assert_eq!(edit.new_range, 6..10);
    }

    #[test]
    fn diff_syntax_edit_shared_suffix() {
        let edit = diff_syntax_edit_from_text_change("old suffix", "new suffix").unwrap();
        assert_eq!(edit.old_range, 0..3);
        assert_eq!(edit.new_range, 0..3);
    }

    #[test]
    fn diff_syntax_edit_shared_prefix_and_suffix() {
        let edit = diff_syntax_edit_from_text_change("fn foo() {}", "fn bar() {}").unwrap();
        // "fn " is shared prefix (3 bytes), "() {}" is shared suffix (5 bytes)
        assert_eq!(edit.old_range, 3..6);
        assert_eq!(edit.new_range, 3..6);
    }

    #[test]
    fn diff_syntax_edit_insertion_at_beginning() {
        let edit = diff_syntax_edit_from_text_change("fn main() {}", "/* comment */\nfn main() {}")
            .unwrap();
        assert_eq!(edit.old_range, 0..0);
        assert_eq!(edit.new_range, 0..14);
    }

    #[test]
    fn diff_syntax_edit_insertion_at_end() {
        let edit =
            diff_syntax_edit_from_text_change("fn main() {}", "fn main() {}\n// end").unwrap();
        // "fn main() {}" is 12 bytes; insertion starts after byte 12
        assert_eq!(edit.old_range, 12..12);
        assert_eq!(edit.new_range, 12..19);
    }

    #[test]
    fn diff_syntax_edit_deletion() {
        let edit = diff_syntax_edit_from_text_change("fn foo() { body }", "fn foo() {}").unwrap();
        // shared prefix: "fn foo() {" (10 bytes), shared suffix: "}" (1 byte)
        assert_eq!(edit.old_range, 10..16);
        assert_eq!(edit.new_range, 10..10);
    }

    #[test]
    fn diff_syntax_edit_one_empty_string() {
        let edit = diff_syntax_edit_from_text_change("", "hello").unwrap();
        assert_eq!(edit.old_range, 0..0);
        assert_eq!(edit.new_range, 0..5);

        let edit = diff_syntax_edit_from_text_change("hello", "").unwrap();
        assert_eq!(edit.old_range, 0..5);
        assert_eq!(edit.new_range, 0..0);
    }

    #[test]
    fn diff_syntax_edit_multibyte_utf8() {
        // "café" is 5 bytes (é is 2 bytes), "caff" is 4 bytes
        let edit = diff_syntax_edit_from_text_change("café", "caff").unwrap();
        // shared prefix: "caf" (3 bytes), diverges at é vs f
        assert_eq!(edit.old_range, 3..5);
        assert_eq!(edit.new_range, 3..4);
    }
}
