use super::*;
use crate::view::diff_utils::compute_diff_yaml_block_scalar_for_src_ix;
use crate::view::markdown_preview;
use crate::view::perf::{self, ViewPerfSpan};
use crate::view::rows;
use gitcomet_core::domain::DiffRowProvider;

mod file_diff;
mod image_cache;
mod patch_diff;

#[cfg(feature = "benchmarks")]
pub(in crate::view) use self::file_diff::bench_build_file_diff_providers;
pub(in crate::view) use self::file_diff::{PagedFileDiffInlineRows, PagedFileDiffRows};
use self::file_diff::{build_file_diff_cache_rebuild, build_inline_text, file_diff_text_signature};

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

// Full-document views (file diff, worktree preview) always attempt prepared
// syntax and fall back to plain/heuristic rendering until it is ready.
const FULL_DOCUMENT_SYNTAX_MODE: rows::DiffSyntaxMode = rows::DiffSyntaxMode::Auto;

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

    pub(in crate::view) fn file_diff_split_modify_pair_texts(
        &self,
        row_ix: usize,
    ) -> Option<(&str, &str)> {
        self.file_diff_row_provider
            .as_ref()
            .and_then(|provider| provider.modify_pair_texts(row_ix))
    }

    pub(in crate::view) fn file_diff_inline_modify_pair_texts(
        &self,
        inline_ix: usize,
    ) -> Option<(&str, &str, gitcomet_core::domain::DiffLineKind)> {
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

        let source_text = self.worktree_preview_text.clone();
        if source_text.len() > markdown_preview::MAX_PREVIEW_SOURCE_BYTES {
            self.worktree_markdown_preview = Loadable::Error(
                markdown_preview::single_preview_unavailable_reason(source_text.len()).to_string(),
            );
            self.worktree_markdown_preview_inflight = None;
            return;
        }

        self.worktree_markdown_preview = Loadable::Loading;
        self.worktree_markdown_preview_seq = self.worktree_markdown_preview_seq.wrapping_add(1);
        let seq = self.worktree_markdown_preview_seq;
        self.worktree_markdown_preview_inflight = Some(seq);

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let result = smol::unblock(move || {
                    let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewParse);
                    build_single_markdown_preview_document(source_text.as_ref())
                })
                .await;

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

    pub(in crate::view) fn set_worktree_preview_ready_source(
        &mut self,
        path: std::path::PathBuf,
        source_text: SharedString,
        line_starts: Arc<[usize]>,
        cx: &mut gpui::Context<Self>,
    ) {
        let line_count = indexed_line_count(source_text.as_ref(), line_starts.as_ref());
        let source_changed = self.worktree_preview_path.as_ref() != Some(&path)
            || self.worktree_preview_line_count() != Some(line_count)
            || self.worktree_preview_text.len() != source_text.len()
            || self.worktree_preview_text.as_ref() != source_text.as_ref();
        let cache_binding_changed =
            self.worktree_preview_segments_cache_path.as_ref() != Some(&path);

        self.worktree_preview_path = Some(path.clone());
        self.worktree_preview = Loadable::Ready(line_count);
        self.worktree_preview_text = source_text;
        self.worktree_preview_line_starts = line_starts;
        self.worktree_preview_syntax_language = rows::diff_syntax_language_for_path(&path);
        self.worktree_preview_segments_cache_path = Some(path);
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

        self.refresh_worktree_preview_syntax_document(cx);
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
                        let parsed_document = smol::unblock(move || {
                            rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                                language,
                                FULL_DOCUMENT_SYNTAX_MODE,
                                source_text,
                                line_starts,
                                background_reparse_seed,
                                None,
                            )
                        })
                        .await;

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
                let parsed_documents =
                    smol::unblock(move || FileDiffBackgroundPreparedSyntaxDocuments {
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
                    })
                    .await;

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
        self.file_diff_new_text = SharedString::default();
        self.file_diff_new_line_starts = Arc::default();
        self.file_diff_inline_cache.clear();
        self.file_diff_inline_row_provider = None;
        self.file_diff_inline_text = SharedString::default();
        self.file_diff_inline_word_highlights.clear();
        self.file_diff_split_word_highlights_old.clear();
        self.file_diff_split_word_highlights_new.clear();
    }

    pub(in super::super::super) fn ensure_file_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, diff_file_rev, diff_target, workdir, file)) = (|| {
            let repo = self.active_repo()?;
            if !Self::is_file_diff_target(repo.diff_state.diff_target.as_ref()) {
                return None;
            }

            let file = match &repo.diff_state.diff_file {
                Loadable::Ready(Some(file)) => Some(Arc::clone(file)),
                _ => None,
            };

            Some((
                repo.id,
                repo.diff_state.diff_file_rev,
                repo.diff_state.diff_target.clone(),
                repo.spec.workdir.clone(),
                file,
            ))
        })() else {
            self.file_diff_cache_repo_id = None;
            self.file_diff_cache_target = None;
            self.file_diff_cache_rev = 0;
            self.reset_file_diff_cache_data();
            return;
        };

        let diff_target_for_task = diff_target.clone();
        let file_content_signature = file
            .as_ref()
            .map(|file| file_diff_text_signature(file.as_ref()));
        let same_repo_and_target = self.file_diff_cache_repo_id == Some(repo_id)
            && self.file_diff_cache_target == diff_target;
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
            // If syntax was still missing, kick the syntax refresh path for the new active key.
            if self.file_diff_cache_inflight.is_none() {
                self.rekey_file_diff_prepared_syntax_documents_for_rev(diff_file_rev);
                self.file_diff_cache_rev = diff_file_rev;
                self.refresh_file_diff_syntax_documents(cx, None, None, None, None);
            }
            return;
        }

        self.file_diff_cache_repo_id = Some(repo_id);
        self.file_diff_cache_rev = diff_file_rev;
        self.file_diff_cache_target = diff_target;
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
                let rebuild =
                    smol::unblock(move || build_file_diff_cache_rebuild(file.as_ref(), &workdir))
                        .await;

                let _ = view.update(cx, |this, cx| {
                    if this.file_diff_cache_inflight != Some(seq) {
                        return;
                    }
                    if this.file_diff_cache_repo_id != Some(repo_id)
                        || this.file_diff_cache_rev != diff_file_rev
                        || this.file_diff_cache_target != diff_target_for_task
                    {
                        return;
                    }

                    this.file_diff_cache_inflight = None;
                    this.file_diff_cache_path = rebuild.file_path;
                    this.file_diff_cache_language = rebuild.language;
                    this.file_diff_row_provider = Some(rebuild.row_provider);
                    this.file_diff_old_text = rebuild.old_text;
                    this.file_diff_old_line_starts = rebuild.old_line_starts;
                    this.file_diff_new_text = rebuild.new_text;
                    this.file_diff_new_line_starts = rebuild.new_line_starts;
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

        let Some((repo_id, diff_file_rev, diff_target, file)) = (|| {
            let repo = self.active_repo()?;
            if !Self::is_file_diff_target(repo.diff_state.diff_target.as_ref()) {
                return None;
            }

            let file = match &repo.diff_state.diff_file {
                Loadable::Ready(Some(file)) => Some(Arc::clone(file)),
                _ => None,
            };

            Some((
                repo.id,
                repo.diff_state.diff_file_rev,
                repo.diff_state.diff_target.clone(),
                file,
            ))
        })() else {
            clear_cache(self);
            return;
        };

        let diff_target_for_task = diff_target.clone();
        let file_content_signature = file
            .as_ref()
            .map(|file| file_diff_text_signature(file.as_ref()));
        let same_repo_and_target = self.file_markdown_preview_cache_repo_id == Some(repo_id)
            && self.file_markdown_preview_cache_target == diff_target;

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
        self.file_markdown_preview_cache_target = diff_target;
        self.file_markdown_preview = Loadable::NotLoaded;
        self.file_markdown_preview_inflight = None;

        let Some(file) = file else {
            return;
        };
        // `file` was `Some` when `file_content_signature` was computed, so unwrap is safe.
        let content_signature = file_content_signature.unwrap();
        let old_source = file.old.clone().unwrap_or_default();
        let new_source = file.new.clone().unwrap_or_default();

        let combined_len = old_source.len() + new_source.len();
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
                let result = smol::unblock(move || {
                    let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewParse);
                    markdown_preview::build_markdown_diff_preview(
                        old_source.as_str(),
                        new_source.as_str(),
                    )
                    .map(Arc::new)
                    .ok_or_else(|| {
                        markdown_preview::diff_preview_unavailable_reason(
                            old_source.len() + new_source.len(),
                        )
                        .to_string()
                    })
                })
                .await;

                let _ = view.update(cx, |this, cx| {
                    if this.file_markdown_preview_inflight != Some(seq) {
                        return;
                    }
                    if this.file_markdown_preview_cache_repo_id != Some(repo_id)
                        || this.file_markdown_preview_cache_rev != diff_file_rev
                        || this.file_markdown_preview_cache_target != diff_target_for_task
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
        self.diff_cache.clear();
        self.diff_row_provider = None;
        self.diff_split_row_provider = None;
        self.diff_cache_repo_id = None;
        self.diff_cache_rev = 0;
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

        let (repo_id, diff_rev, diff_target, workdir, diff) = {
            let Some(repo) = self.active_repo() else {
                return;
            };
            let workdir = repo.spec.workdir.clone();
            let diff = match &repo.diff_state.diff {
                Loadable::Ready(diff) => Some(Arc::clone(diff)),
                _ => None,
            };
            (
                repo.id,
                repo.diff_state.diff_rev,
                repo.diff_state.diff_target.clone(),
                workdir,
                diff,
            )
        };

        self.diff_cache_repo_id = Some(repo_id);
        self.diff_cache_rev = diff_rev;
        self.diff_cache_target = diff_target;

        let Some(diff) = diff else {
            return;
        };

        let row_provider = Arc::new(PagedPatchDiffRows::new(
            Arc::clone(&diff),
            PATCH_DIFF_PAGE_SIZE,
        ));
        let split_row_provider = Arc::new(PagedPatchSplitRows::new(Arc::clone(&row_provider)));
        self.diff_row_provider = Some(row_provider);
        self.diff_split_row_provider = Some(split_row_provider);

        self.diff_file_for_src_ix = compute_diff_file_for_src_ix(diff.lines.as_slice());
        self.diff_line_kind_for_src_ix = diff.lines.iter().map(|line| line.kind).collect();
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

    fn compute_diff_scrollbar_markers(&self) -> Vec<components::ScrollbarMarker> {
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
        let current_len = if is_file_view {
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
        {
            return;
        }

        self.diff_visible_cache_len = current_len;
        self.diff_visible_view = self.diff_view;
        self.diff_visible_is_file_view = is_file_view;
        self.diff_horizontal_min_width = px(0.0);
        self.diff_visible_inline_map = None;

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
                text: Arc::from("diff --git a/docs/table.md b/docs/table.md"),
                old_line: None,
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: Arc::from("deleted file mode 100644"),
                old_line: None,
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: Arc::from("-| **Header Bold** | B |"),
                old_line: Some(1),
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: Arc::from("-| --- | --- |"),
                old_line: Some(2),
                new_line: None,
            },
            AnnotatedDiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: Arc::from("-| [link](https://example.com) | plain |"),
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
