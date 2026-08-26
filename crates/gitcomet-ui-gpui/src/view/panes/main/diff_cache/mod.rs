use super::*;
use crate::view::diff_utils::compute_diff_yaml_block_scalar_for_src_ix;
use crate::view::markdown_preview;
use crate::view::perf::{self, ViewPerfSpan};
use crate::view::rows;
use gitcomet_core::domain::DiffRowProvider;
use rustc_hash::FxHasher;

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
use self::file_diff::{file_diff_source_identity, file_diff_text_signature, line_starts_describe};
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
// A cold click may do a small read/parse inline so ordinary files respond in
// the same event. Larger syntax-sized documents are prepared by the dedicated
// worker below and the click is replayed when it completes.
const DIFF_CLICK_FOREGROUND_COMPLETION_MAX_TEXT_BYTES: usize = 1024 * 1024;

// Full-document views (file diff, worktree preview) always attempt prepared
// syntax and fall back to plain/heuristic rendering until it is ready.
const FULL_DOCUMENT_SYNTAX_MODE: rows::DiffSyntaxMode = rows::DiffSyntaxMode::Auto;

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

    pub(in crate::view) fn file_diff_split_visual_kind(
        &self,
        row_ix: usize,
    ) -> gitcomet_core::file_diff::FileDiffRowKind {
        self.file_diff_row_provider
            .as_ref()
            .and_then(|provider| provider.visual_kind(row_ix))
            .or_else(|| self.file_diff_cache_rows.get(row_ix).map(|row| row.kind))
            .unwrap_or(gitcomet_core::file_diff::FileDiffRowKind::Context)
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

    pub(in crate::view) fn file_diff_inline_visual_kind(
        &self,
        inline_ix: usize,
    ) -> gitcomet_core::domain::DiffLineKind {
        self.file_diff_inline_row_provider
            .as_ref()
            .and_then(|provider| provider.visual_kind(inline_ix))
            .or_else(|| {
                self.file_diff_inline_cache
                    .get(inline_ix)
                    .map(|row| row.kind)
            })
            .unwrap_or(gitcomet_core::domain::DiffLineKind::Context)
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

    pub(in crate::view) fn patch_visual_line_kind(
        &self,
        src_ix: usize,
    ) -> gitcomet_core::domain::DiffLineKind {
        self.diff_visual_line_kind_for_src_ix
            .get(src_ix)
            .copied()
            .or_else(|| self.diff_line_kind_for_src_ix.get(src_ix).copied())
            .or_else(|| self.patch_diff_row(src_ix).map(|line| line.kind))
            .unwrap_or(gitcomet_core::domain::DiffLineKind::Context)
    }

    pub(in crate::view) fn patch_split_visual_row_kind(
        &self,
        row: &PatchSplitRow,
    ) -> gitcomet_core::file_diff::FileDiffRowKind {
        use gitcomet_core::domain::DiffLineKind as DK;
        use gitcomet_core::file_diff::FileDiffRowKind as RK;

        let PatchSplitRow::Aligned {
            row,
            old_src_ix,
            new_src_ix,
        } = row
        else {
            return RK::Context;
        };

        let old_changed = old_src_ix
            .is_some_and(|src_ix| matches!(self.patch_visual_line_kind(src_ix), DK::Remove));
        let new_changed =
            new_src_ix.is_some_and(|src_ix| matches!(self.patch_visual_line_kind(src_ix), DK::Add));

        match (old_changed, new_changed) {
            (true, true) => RK::Modify,
            (true, false) => RK::Remove,
            (false, true) => RK::Add,
            (false, false) => {
                if matches!(row.kind, RK::Add | RK::Remove | RK::Modify) {
                    RK::Context
                } else {
                    row.kind
                }
            }
        }
    }
}

impl MainPaneView {
    fn advance_file_diff_syntax_generation(&mut self) {
        self.file_diff_syntax_generation = self.file_diff_syntax_generation.wrapping_add(1);
        self.file_diff_pair_syntax_text.clear();
        self.file_diff_click_syntax_inflight.clear();
    }

    /// Drop the memoized intra-line word-diff ranges. They are keyed by bare row
    /// index, so they only describe the rows they were computed from: anything
    /// that changes what row *n* holds has to clear them or row *n* keeps ranges
    /// measured against text it no longer shows.
    pub(in crate::view) fn reset_file_diff_word_highlight_caches(&mut self) {
        self.file_diff_inline_word_highlights =
            rows::new_lru_cache(FILE_DIFF_WORD_HIGHLIGHT_CACHE_MAX_ENTRIES);
        self.file_diff_split_word_highlights =
            rows::new_lru_cache(FILE_DIFF_WORD_HIGHLIGHT_CACHE_MAX_ENTRIES);
    }

    pub(in crate::view) fn ensure_file_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((
            repo_id,
            diff_file_rev,
            diff_target,
            workdir,
            expected_abs_path,
            file,
            patch_diff,
            patch_diff_loading,
        )) = (|| {
            let (repo_id, diff_file_rev, diff_target, workdir, expected_abs_path) =
                self.rendered_file_diff_identity()?;
            let file: Option<Arc<gitcomet_core::domain::FileDiffText>> =
                match self.rendered_file_diff_loadable()? {
                    Loadable::Ready(Some(file)) => Some(Arc::clone(file)),
                    _ => None,
                };
            let patch_diff_loadable = self.rendered_patch_diff_loadable()?;
            let patch_diff: Option<Arc<gitcomet_core::domain::Diff>> = match patch_diff_loadable {
                Loadable::Ready(diff) => Some(Arc::clone(diff)),
                _ => None,
            };
            let patch_diff_loading = matches!(patch_diff_loadable, Loadable::Loading);

            Some((
                repo_id,
                diff_file_rev,
                diff_target,
                workdir,
                expected_abs_path,
                file,
                patch_diff,
                patch_diff_loading,
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
            signature ^= (self.diff_whitespace_mode.key().len() as u64).rotate_left(7);
            signature
        });
        let same_repo_and_target = self.file_diff_cache_repo_id == Some(repo_id)
            && self.file_diff_cache_target == Some(diff_target.clone())
            && self.file_diff_cache_whitespace_mode == self.diff_whitespace_mode
            && self.file_diff_cache_path.as_ref() == Some(&expected_abs_path);
        let previous_split_left_reparse_seed = same_repo_and_target
            .then(|| self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft))
            .flatten();
        let previous_split_right_reparse_seed = same_repo_and_target
            .then(|| self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight))
            .flatten();
        let previous_old_text = same_repo_and_target.then(|| self.file_diff_old_text.clone());
        let previous_new_text = same_repo_and_target.then(|| self.file_diff_new_text.clone());

        if patch_diff_loading
            && patch_diff.is_none()
            && file
                .as_ref()
                .is_some_and(|file| file_diff_text_is_source_backed(file.as_ref()))
        {
            if same_repo_and_target {
                self.file_diff_cache_inflight = None;
                self.rekey_file_diff_prepared_syntax_documents_for_rev(diff_file_rev);
                self.file_diff_cache_rev = diff_file_rev;
            } else {
                self.file_diff_cache_repo_id = Some(repo_id);
                self.file_diff_cache_rev = diff_file_rev;
                self.file_diff_cache_whitespace_mode = self.diff_whitespace_mode;
                self.file_diff_cache_target = Some(diff_target);
                self.reset_file_diff_cache_data();
                self.clear_diff_text_style_caches();
            }
            return;
        }

        if same_repo_and_target
            && file.is_none()
            && self.file_diff_cache_content_signature.is_some()
        {
            // Keep the current same-target rows visible while a refresh is pending.
            // Dropping them would create a zero-width frame, and GPUI would clamp
            // any horizontal offset back to the start before the ready payload returns.
            self.rekey_file_diff_prepared_syntax_documents_for_rev(diff_file_rev);
            self.file_diff_cache_rev = diff_file_rev;
            return;
        }

        if same_repo_and_target && self.file_diff_cache_rev == diff_file_rev {
            // Reselecting the same file enters Loading with an unchanged file rev; keep the
            // current cache until a ready file payload proves the effective content changed.
            let content_changed_without_rev_bump = file_content_signature
                .is_some_and(|signature| self.file_diff_cache_content_signature != Some(signature));
            if !content_changed_without_rev_bump {
                return;
            }
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
        self.file_diff_cache_whitespace_mode = self.diff_whitespace_mode;
        self.file_diff_cache_target = Some(diff_target);

        // Rebuilding the same file keeps the rows that are already on screen:
        // they are a complete, self-consistent generation, the completion below
        // swaps every field of the next one in atomically, and dropping them
        // first is what makes the pane flash "Processing file…" on every staged
        // line. A different file — or one with no content to rebuild from — must
        // still clear immediately, since its rows are not this file's.
        let keep_rows_while_rebuilding = same_repo_and_target && file.is_some();
        if !keep_rows_while_rebuilding {
            self.reset_file_diff_cache_data();

            // Reset the segment cache to avoid mixing patch/file indices.
            self.clear_diff_text_style_caches();
        }

        let Some(file) = file else {
            return;
        };
        let content_signature =
            file_content_signature.unwrap_or_else(|| file_diff_text_signature(file.as_ref()));

        self.file_diff_cache_seq = self.file_diff_cache_seq.wrapping_add(1);
        let seq = self.file_diff_cache_seq;
        self.file_diff_cache_inflight = Some(seq);
        let whitespace_mode = self.diff_whitespace_mode;

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let rebuild_cache = move || {
                    build_file_diff_cache_rebuild_with_patch(
                        file.as_ref(),
                        &workdir,
                        patch_diff.as_deref(),
                        whitespace_mode,
                    )
                };
                let rebuild_result = if crate::ui_runtime::current().uses_background_compute() {
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
                        || this.file_diff_cache_whitespace_mode != whitespace_mode
                        || this.file_diff_cache_target != Some(diff_target_for_task.clone())
                    {
                        return;
                    }

                    this.file_diff_cache_inflight = None;
                    let rebuild = match rebuild_result {
                        Ok(rebuild) => rebuild,
                        Err(error) => {
                            this.reset_file_diff_cache_data();
                            this.file_diff_cache_repo_id = Some(repo_id);
                            this.file_diff_cache_rev = diff_file_rev;
                            this.file_diff_cache_whitespace_mode = whitespace_mode;
                            this.file_diff_cache_target = Some(diff_target_for_task.clone());
                            this.file_diff_cache_path = Some(expected_abs_path.clone());
                            this.file_diff_cache_content_signature = Some(content_signature);
                            this.file_diff_cache_error = Some(error);
                            cx.notify();
                            return;
                        }
                    };
                    this.file_diff_cache_error = None;
                    // The old rows remained interactive during a same-target
                    // rebuild. Invalidate their workers and retained source only
                    // now that every replacement field is ready to be swapped.
                    this.advance_file_diff_syntax_generation();
                    // A worker that finished before this swap could legitimately
                    // serve the still-visible old rows, but the cache rev already
                    // names this incoming payload. Drop any documents it put
                    // under that rev before preparing the replacement sources.
                    this.remove_file_diff_prepared_syntax_documents_for_rev(
                        repo_id,
                        diff_file_rev,
                        &expected_abs_path,
                    );
                    this.file_diff_cache_path = rebuild.file_path;
                    this.file_diff_cache_language = rebuild.language;
                    this.file_diff_row_provider = Some(rebuild.row_provider);
                    this.file_diff_old_source_path = rebuild.old_source_path;
                    this.file_diff_new_source_path = rebuild.new_source_path;
                    this.file_diff_old_source_identity = rebuild.old_source_identity;
                    this.file_diff_new_source_identity = rebuild.new_source_identity;
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
                    this.clear_diff_text_projected_highlights();
                    // The rows just changed under their own indices. On the
                    // clearing path `reset_file_diff_cache_data` already did
                    // this; the kept-rows path deliberately skips it, so without
                    // this a staged line leaves every row holding the previous
                    // generation's word ranges.
                    this.reset_file_diff_word_highlight_caches();
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

    pub(in crate::view) fn ensure_file_markdown_preview_cache(
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
                    // See the single-document preview: a search opened while
                    // this was parsing found nothing and needs to rescan.
                    this.diff_search_recompute_matches();
                    cx.notify();
                });
            },
        )
        .detach();
    }

    pub(in crate::view) fn ensure_rendered_patch_diff_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let ready_diff = match self.rendered_patch_diff_loadable() {
            Some(Loadable::Ready(diff)) => Some(Arc::clone(diff)),
            _ => None,
        };
        let metadata_current = self.diff_cache_repo_id == self.active_repo_id()
            && self.diff_cache_rev == self.rendered_patch_diff_rev()
            && self.diff_cache_target == self.rendered_diff_target().cloned();
        let ready_content_changed = metadata_current
            && ready_diff.as_ref().is_some_and(|diff| {
                self.patch_diff_row_len() != diff.lines.len()
                    || self.diff_cache_content_signature
                        != Some(patch_diff_content_signature(diff.as_ref()))
            });
        let should_rebuild = !metadata_current || ready_content_changed;
        if should_rebuild {
            self.rebuild_diff_cache(cx);
        }
    }

    pub(in crate::view) fn rebuild_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
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
        if let Some((repo_id, diff_rev, diff_target, _, diff)) = next_cache_state.as_ref() {
            let same_repo_and_target = self.diff_cache_repo_id == Some(*repo_id)
                && self.diff_cache_target.as_ref() == diff_target.as_ref();
            if same_repo_and_target {
                if diff.is_none() && self.diff_cache_content_signature.is_some() {
                    // Preserve the last ready same-target patch rows through transient Loading.
                    self.diff_cache_rev = *diff_rev;
                    return;
                }

                if diff.is_some()
                    && next_content_signature.is_some()
                    && self.diff_cache_content_signature == next_content_signature
                {
                    // Store-side refreshes can bump diff_rev without changing the rendered patch.
                    // Keep visible rows and horizontal width hints intact across those rev-only
                    // redraws.
                    self.diff_cache_rev = *diff_rev;
                    return;
                }
            }
        }
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
        self.diff_visual_line_kind_for_src_ix.clear();
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
        self.clear_diff_selection_state();
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
        self.rebuild_patch_visual_line_kinds_from_ready_diff(diff.as_ref());
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
                    // The header row opens its own file section, so the path
                    // resolved for it above is exactly what to show.
                    let display: SharedString = self
                        .diff_file_for_src_ix
                        .get(src_ix)
                        .and_then(|path| path.as_ref())
                        .map(|path| SharedString::new(Arc::clone(path)))
                        .unwrap_or_else(|| SharedString::from(line.text.as_ref().to_string()));
                    self.diff_header_display_cache.insert(src_ix, display);
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
}

mod collapsed_projection;
use markdown_preview_docs::*;
use patch_visual::*;
mod markdown_preview_docs;
mod patch_visual;
mod scrollbar_markers;
mod syntax_documents;

#[cfg(test)]
mod tests;
