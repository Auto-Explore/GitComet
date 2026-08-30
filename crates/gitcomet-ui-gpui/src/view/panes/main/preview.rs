use super::*;
use crate::view::markdown_preview::{
    MarkdownPreviewDocument, MarkdownPreviewRow, MarkdownPreviewVisualRow,
};
#[cfg(test)]
use std::borrow::Cow;
use std::io::Read;

/// Largest file the editor will open.
///
/// Deliberately its own number rather than the tree-sitter parse ceiling: that
/// one is a *highlighting* budget, and a 3 MB log or CSV is perfectly editable
/// with the heuristic fallback. This one is about what the buffer itself can
/// carry — soft wrap in particular still measures the whole document, budgeted
/// but document-wide (see the note in `text_input/element.rs`), so the editor
/// keeps an upper bound rather than accepting a file of any size.
pub(in crate::view) const FILE_EDITOR_MAX_TEXT_BYTES: usize = 32 * 1024 * 1024;

const WORKTREE_PREVIEW_INDEX_SCAN_BUFFER_BYTES: usize = 64 * 1024;
const WORKTREE_PREVIEW_INDEX_LINE_CAPACITY_MAX: usize = 64 * 1024;

struct IndexedWorktreePreview {
    source_len: usize,
    line_starts: Arc<[usize]>,
    line_flags: Arc<[u8]>,
    source_text: Option<SharedString>,
}

#[inline]
fn packed_preview_line_flags(ascii_only: bool, has_tabs: bool) -> u8 {
    preview_line_flags_from_bools(ascii_only, has_tabs)
}

#[inline]
fn worktree_preview_index_line_capacity_hint(source_len_hint: usize) -> usize {
    source_len_hint
        .saturating_div(64)
        .saturating_add(1)
        .min(WORKTREE_PREVIEW_INDEX_LINE_CAPACITY_MAX)
}

#[inline]
fn worktree_preview_materialized_source_arc(source_text: &SharedString) -> Arc<str> {
    source_text.clone().into()
}

#[inline]
fn worktree_preview_materialized_line_raw_text(
    source_text: &SharedString,
    range: std::ops::Range<usize>,
) -> gitcomet_core::file_diff::FileDiffLineText {
    gitcomet_core::file_diff::FileDiffLineText::shared_slice(
        worktree_preview_materialized_source_arc(source_text),
        range,
    )
}

fn validate_utf8_chunk_streaming(
    utf8_tail: &mut Vec<u8>,
    validation_buffer: &mut Vec<u8>,
    chunk: &[u8],
) -> Result<(), String> {
    validation_buffer.clear();
    if !utf8_tail.is_empty() {
        validation_buffer.extend_from_slice(utf8_tail.as_slice());
    }
    validation_buffer.extend_from_slice(chunk);

    match std::str::from_utf8(validation_buffer.as_slice()) {
        Ok(_) => {
            utf8_tail.clear();
            Ok(())
        }
        Err(error) => {
            if error.error_len().is_some() {
                return Err("File is not valid UTF-8; binary preview is not supported.".to_string());
            }

            let valid_up_to = error.valid_up_to();
            utf8_tail.clear();
            utf8_tail.extend_from_slice(&validation_buffer[valid_up_to..]);
            Ok(())
        }
    }
}

/// Read a working-tree file for the editor.
///
/// Shares the preview's reader so both agree on what "editable text" means: it
/// rejects directories and non-UTF-8 up front.
///
/// The size limit is the *editor's*, not the syntax engine's. The reader stops
/// materializing past the tree-sitter parse ceiling, which is a highlighting
/// budget — a 3 MB log or CSV is perfectly editable, it just does not get a
/// tree, and the editor already has a heuristic fallback for exactly that. So
/// past the ceiling this re-reads the file plainly rather than refusing it.
pub(super) fn read_worktree_file_for_editing(
    path: &std::path::Path,
) -> Result<SharedString, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| match e.kind() {
            // Reachable from a commit's file list: the editor always opens the
            // workspace copy, and a file deleted since that commit has none.
            // A raw "No such file or directory (os error 2)" as the editor body
            // says nothing about why.
            std::io::ErrorKind::NotFound => {
                "This file does not exist in the working tree.".to_string()
            }
            _ => e.to_string(),
        })?
        .len();
    if len > FILE_EDITOR_MAX_TEXT_BYTES as u64 {
        return Err(format!(
            "File is larger than {} MB; editing is not supported.",
            FILE_EDITOR_MAX_TEXT_BYTES / (1024 * 1024)
        ));
    }
    let indexed = index_utf8_worktree_preview_file(path)?;
    if let Some(text) = indexed.source_text {
        return Ok(text);
    }
    // Between the parse ceiling and the editor's own limit the indexer stops
    // materializing, so read it plainly. Already validated as UTF-8 above.
    std::fs::read_to_string(path)
        .map(SharedString::from)
        .map_err(|e| e.to_string())
}

fn index_utf8_worktree_preview_file(
    path: &std::path::Path,
) -> Result<IndexedWorktreePreview, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.is_dir() {
        return Err(
            "Selected path is a directory. Select a file inside to preview, or stage the directory to add its contents.".to_string(),
        );
    }

    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut reader =
        std::io::BufReader::with_capacity(WORKTREE_PREVIEW_INDEX_SCAN_BUFFER_BYTES, file);
    let source_len_hint = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    let line_capacity_hint = worktree_preview_index_line_capacity_hint(source_len_hint);
    let mut line_starts = Vec::with_capacity(line_capacity_hint);
    let mut line_flags = Vec::with_capacity(line_capacity_hint);
    let mut validation_buffer =
        Vec::with_capacity(WORKTREE_PREVIEW_INDEX_SCAN_BUFFER_BYTES.saturating_add(4));
    let mut utf8_tail = Vec::with_capacity(4);
    let mut scan_buffer = vec![0u8; WORKTREE_PREVIEW_INDEX_SCAN_BUFFER_BYTES];
    let mut source_len = 0usize;
    let mut line_ascii_only = true;
    let mut line_has_tabs = false;
    let mut source_bytes = (source_len_hint <= rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES)
        .then(|| Vec::with_capacity(source_len_hint));

    if source_len_hint > 0 {
        line_starts.push(0);
    }

    loop {
        let read_len = reader
            .read(scan_buffer.as_mut_slice())
            .map_err(|e| e.to_string())?;
        if read_len == 0 {
            break;
        }
        if source_len == 0 && line_starts.is_empty() {
            line_starts.push(0);
        }
        let chunk = &scan_buffer[..read_len];
        validate_utf8_chunk_streaming(&mut utf8_tail, &mut validation_buffer, chunk)?;
        if let Some(bytes) = source_bytes.as_mut() {
            if bytes.len().saturating_add(chunk.len())
                <= rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES
            {
                bytes.extend_from_slice(chunk);
            } else {
                source_bytes = None;
            }
        }

        for &byte in chunk {
            if byte == b'\n' {
                line_flags.push(packed_preview_line_flags(line_ascii_only, line_has_tabs));
                source_len = source_len.saturating_add(1);
                line_starts.push(source_len);
                line_ascii_only = true;
                line_has_tabs = false;
                continue;
            }

            if !byte.is_ascii() {
                line_ascii_only = false;
            }
            if byte == b'\t' {
                line_has_tabs = true;
            }
            source_len = source_len.saturating_add(1);
        }
    }

    if !utf8_tail.is_empty() {
        return Err("File is not valid UTF-8; binary preview is not supported.".to_string());
    }

    if source_len > 0 {
        line_flags.push(packed_preview_line_flags(line_ascii_only, line_has_tabs));
    }
    let source_text = source_bytes
        .map(String::from_utf8)
        .transpose()
        .map_err(|_| "File is not valid UTF-8; binary preview is not supported.".to_string())?
        .map(SharedString::from);

    Ok(IndexedWorktreePreview {
        source_len,
        line_starts: Arc::from(line_starts),
        line_flags: Arc::from(line_flags),
        source_text,
    })
}

type ConflictPreviewImagePayload = (gpui::ImageFormat, Vec<u8>);

fn conflict_preview_side_bytes(
    file: Option<&gitcomet_state::model::ConflictFile>,
    side: ThreeWayColumn,
    fallback_text: &SharedString,
) -> Option<Vec<u8>> {
    let file_bytes = file.and_then(|file| match side {
        ThreeWayColumn::Base => file.base_bytes.as_deref(),
        ThreeWayColumn::Ours => file.ours_bytes.as_deref(),
        ThreeWayColumn::Theirs => file.theirs_bytes.as_deref(),
    });
    if let Some(bytes) = file_bytes
        && !bytes.is_empty()
    {
        return Some(bytes.to_vec());
    }

    let file_text = file.and_then(|file| match side {
        ThreeWayColumn::Base => file.base.as_deref(),
        ThreeWayColumn::Ours => file.ours.as_deref(),
        ThreeWayColumn::Theirs => file.theirs.as_deref(),
    });
    if let Some(text) = file_text
        && !text.is_empty()
    {
        return Some(text.as_bytes().to_vec());
    }

    (!fallback_text.is_empty()).then(|| fallback_text.as_ref().as_bytes().to_vec())
}

fn ready_conflict_preview_image_from_bytes(
    format: gpui::ImageFormat,
    bytes: Option<Vec<u8>>,
) -> LoadableImagePreview {
    match bytes {
        Some(bytes) => Loadable::Ready(Some(Arc::new(gpui::Image::from_bytes(format, bytes)))),
        None => Loadable::Ready(None),
    }
}

fn loading_conflict_preview_image(has_source: bool) -> LoadableImagePreview {
    if has_source {
        Loadable::Loading
    } else {
        Loadable::Ready(None)
    }
}

fn rasterize_conflict_preview_svg_payload(
    svg_bytes: Option<Vec<u8>>,
) -> Option<ConflictPreviewImagePayload> {
    let svg_bytes = svg_bytes?;
    if let Some(png) = crate::view::diff_utils::rasterize_svg_preview_png(&svg_bytes) {
        return Some((gpui::ImageFormat::Png, png));
    }
    Some((gpui::ImageFormat::Svg, svg_bytes))
}

fn loadable_conflict_preview_svg_image(
    payload: Option<ConflictPreviewImagePayload>,
    had_source: bool,
) -> LoadableImagePreview {
    match payload {
        Some((format, bytes)) => {
            Loadable::Ready(Some(Arc::new(gpui::Image::from_bytes(format, bytes))))
        }
        None if had_source => Loadable::Error("Preview unavailable.".into()),
        None => Loadable::Ready(None),
    }
}

impl MainPaneView {
    /// Clears worktree preview source text, line starts, and the segments
    /// cache. Use this when the preview content is invalidated but the caller
    /// still needs to set identity fields (path, loadable state, syntax
    /// language) separately.
    pub(in crate::view) fn reset_worktree_preview_source_state(&mut self) {
        self.worktree_preview_source_path = None;
        self.worktree_preview_source_len = 0;
        self.worktree_preview_text = SharedString::default();
        self.worktree_preview_line_starts = Arc::default();
        self.worktree_preview_line_flags = Arc::default();
        self.worktree_preview_search_trigram_index = None;
        self.worktree_preview_segments_cache_path = None;
        self.worktree_preview_cache_write_blocked_until_rev = None;
        self.worktree_preview_segments_cache.clear();
    }

    /// Force the read-only preview to re-read `path` from disk.
    ///
    /// The preview is otherwise only invalidated when the rendered `DiffTarget`
    /// *changes* (`apply_state_snapshot`), so a write to the file already on
    /// screen — which is exactly what saving from the editor is — would leave
    /// the pre-save text up until the user navigated away and back.
    pub(in crate::view) fn invalidate_worktree_preview_for_saved_path(
        &mut self,
        path: &std::path::Path,
    ) {
        // Compared as absolute paths. `Path::ends_with` on a repo-relative path
        // over-matches — saving root `foo.rs` would also discard a preview of
        // `deep/dir/foo.rs`.
        let Some(absolute) = self.absolute_worktree_path(path) else {
            return;
        };
        let matches_preview = self
            .worktree_preview_path
            .as_ref()
            .is_some_and(|shown| *shown == absolute);
        if !matches_preview {
            return;
        }
        self.worktree_preview_path = None;
        self.worktree_preview = Loadable::NotLoaded;
        self.worktree_preview_content_rev = self.worktree_preview_content_rev.wrapping_add(1);
        self.worktree_preview_syntax_language = None;
        self.reset_worktree_preview_source_state();
    }

    pub(in super::super::super) fn is_file_diff_target(target: Option<&DiffTarget>) -> bool {
        matches!(
            target,
            Some(
                DiffTarget::WorkingTree { .. }
                    | DiffTarget::Commit { path: Some(_), .. }
                    | DiffTarget::CommitRange { path: Some(_), .. }
            )
        )
    }

    pub(in crate::view) fn is_file_preview_active(&self) -> bool {
        let preview_text_file_available = self.active_repo().is_some_and(|repo| {
            matches!(
                repo.diff_state.diff_preview_text_file,
                Loadable::Loading | Loadable::Error(_) | Loadable::Ready(Some(_))
            )
        });
        let has_untracked_preview = self.untracked_worktree_preview_path().is_some_and(|p| {
            !crate::view::should_bypass_text_file_preview_for_path(&p) && p.is_file()
        });
        let has_added_preview = self.added_file_preview_abs_path().is_some_and(|p| {
            !crate::view::should_bypass_text_file_preview_for_path(&p)
                && !p.is_dir()
                && (p.is_file() || preview_text_file_available)
        });
        let has_deleted_preview = self.deleted_file_preview_abs_path().is_some_and(|p| {
            !crate::view::should_bypass_text_file_preview_for_path(&p)
                && !p.is_dir()
                && preview_text_file_available
        });
        // File-browser "open content" forces a full-content preview for any file.
        let has_content_preview = self.content_preview_abs_path().is_some_and(|p| {
            !self.content_preview_is_picture(&p)
                && !p.is_dir()
                && (p.is_file() || preview_text_file_available)
        });
        has_untracked_preview || has_added_preview || has_deleted_preview || has_content_preview
    }

    /// Whether this content view should be drawn as a picture rather than as
    /// text.
    ///
    /// Images always are; an SVG only while its toggle says Rendered, because
    /// Code is exactly the request to read (and edit) its source.
    pub(in crate::view) fn content_preview_is_picture(&self, path: &std::path::Path) -> bool {
        if !crate::view::should_bypass_text_file_preview_for_path(path) {
            return false;
        }
        match crate::view::preview_path_rendered_kind(path) {
            Some(RenderedPreviewKind::Svg) => {
                self.rendered_preview_modes.get(RenderedPreviewKind::Svg)
                    == RenderedPreviewMode::Rendered
            }
            _ => true,
        }
    }

    /// Returns `true` when the markdown rendered preview is currently shown
    /// (either single-pane file preview or two-sided diff preview).
    pub(in crate::view) fn is_markdown_preview_active(&self) -> bool {
        // The editor replaces the rendered preview entirely: it is the branch
        // that wins in the body, so reporting the preview as active here would
        // grey out Edit and Blame over a buffer that is plainly showing text.
        if self.is_file_editor_active() {
            return false;
        }
        let has_submodule_summary = self
            .active_repo()
            .is_some_and(|repo| !matches!(repo.diff_state.submodule_summary, Loadable::NotLoaded));
        if has_submodule_summary && !self.is_inline_submodule_diff_active() {
            return false;
        }

        let is_file_preview =
            self.is_file_preview_active() && self.untracked_directory_notice().is_none();
        let wants_file_diff = self.wants_file_diff_view(is_file_preview);
        let wants_collapsed_diff = self.wants_collapsed_diff_view(is_file_preview);
        let rendered_preview_kind =
            crate::view::diff_target_rendered_preview_kind(self.rendered_diff_target());
        let toggle_kind = crate::view::main_diff_rendered_preview_toggle_kind(
            wants_file_diff,
            wants_collapsed_diff,
            is_file_preview,
            rendered_preview_kind,
        );
        toggle_kind == Some(RenderedPreviewKind::Markdown)
            && self
                .rendered_preview_modes
                .get(RenderedPreviewKind::Markdown)
                == RenderedPreviewMode::Rendered
    }

    /// Returns `true` when the current diff target is a conflicted file and
    /// there is an applicable conflict resolver strategy.
    pub(in crate::view) fn is_conflict_resolver_active(&self) -> bool {
        self.active_repo().is_some_and(|repo| {
            let Some(DiffTarget::WorkingTree { path, area }) = repo.diff_state.diff_target.as_ref()
            else {
                return false;
            };
            if *area != DiffArea::Unstaged {
                return false;
            }
            let conflict_kind = repo
                .status_entry_for_path(DiffArea::Unstaged, path.as_path())
                .filter(|entry| entry.kind == FileStatusKind::Conflicted)
                .and_then(|e| e.conflict);
            Self::conflict_resolver_strategy(conflict_kind, false).is_some()
        })
    }

    /// Whether the merge tool is showing a rendered *markdown* preview.
    ///
    /// `is_conflict_rendered_preview_active` is also true for a rendered SVG,
    /// which is a picture with no text to search; search has to tell the two
    /// apart or it would report no matches over a file it could have searched.
    pub(in crate::view) fn is_conflict_rendered_markdown_preview_active(&self) -> bool {
        self.is_conflict_rendered_preview_active()
            && self.conflict_resolver.path.as_ref().is_some_and(|path| {
                crate::view::preview_path_rendered_kind(path) == Some(RenderedPreviewKind::Markdown)
            })
    }

    pub(in crate::view) fn is_conflict_rendered_preview_active(&self) -> bool {
        self.conflict_resolver.path.as_ref().is_some_and(|path| {
            crate::view::preview_path_rendered_kind(path).is_some()
                && self.conflict_resolver.resolver_preview_mode
                    == ConflictResolverPreviewMode::Preview
        })
    }

    pub(in crate::view) fn ensure_conflict_markdown_preview_cache(&mut self) {
        if self.is_conflict_rendered_preview_active()
            && self
                .request_conflict_file_load_mode(gitcomet_state::model::ConflictFileLoadMode::Full)
        {
            return;
        }

        let Some(source_hash) = self.conflict_resolver.source_hash else {
            self.conflict_resolver.markdown_preview =
                ConflictResolverMarkdownPreviewState::default();
            return;
        };

        let previews = &self.conflict_resolver.markdown_preview;
        let cache_ready = previews.source_hash == Some(source_hash)
            && !matches!(previews.documents.base, Loadable::NotLoaded)
            && !matches!(previews.documents.ours, Loadable::NotLoaded)
            && !matches!(previews.documents.theirs, Loadable::NotLoaded);
        if cache_ready {
            return;
        }

        let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewParse);
        self.conflict_resolver.markdown_preview = ConflictResolverMarkdownPreviewState {
            source_hash: Some(source_hash),
            documents: build_conflict_markdown_preview_documents(
                &self.conflict_resolver.three_way_text,
            ),
        };
        // These documents are what an open search scans; until now there were
        // none, so it found nothing and has to look again.
        self.diff_search_recompute_matches();
    }

    pub(in crate::view) fn ensure_conflict_image_preview_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(source_hash) = self.conflict_resolver.source_hash else {
            self.conflict_resolver.image_preview = ConflictResolverImagePreviewState::default();
            return;
        };
        let Some(path) = self.conflict_resolver.path.clone() else {
            self.conflict_resolver.image_preview = ConflictResolverImagePreviewState::default();
            return;
        };
        let Some(format) = crate::view::diff_utils::image_format_for_path(&path) else {
            self.conflict_resolver.image_preview = ConflictResolverImagePreviewState::default();
            return;
        };

        let previews = &self.conflict_resolver.image_preview;
        let cache_ready = previews.source_hash == Some(source_hash)
            && previews.path.as_ref() == Some(&path)
            && !matches!(previews.images.base, Loadable::NotLoaded)
            && !matches!(previews.images.ours, Loadable::NotLoaded)
            && !matches!(previews.images.theirs, Loadable::NotLoaded);
        if cache_ready {
            return;
        }

        let loaded_file = self.conflict_resolver.loaded_file.as_ref();
        let base_bytes = conflict_preview_side_bytes(
            loaded_file,
            ThreeWayColumn::Base,
            &self.conflict_resolver.three_way_text.base,
        );
        let ours_bytes = conflict_preview_side_bytes(
            loaded_file,
            ThreeWayColumn::Ours,
            &self.conflict_resolver.three_way_text.ours,
        );
        let theirs_bytes = conflict_preview_side_bytes(
            loaded_file,
            ThreeWayColumn::Theirs,
            &self.conflict_resolver.three_way_text.theirs,
        );

        if format != gpui::ImageFormat::Svg {
            self.conflict_resolver.image_preview = ConflictResolverImagePreviewState {
                source_hash: Some(source_hash),
                path: Some(path),
                images: ThreeWaySides {
                    base: ready_conflict_preview_image_from_bytes(format, base_bytes),
                    ours: ready_conflict_preview_image_from_bytes(format, ours_bytes),
                    theirs: ready_conflict_preview_image_from_bytes(format, theirs_bytes),
                },
            };
            return;
        }

        let base_has_source = base_bytes.is_some();
        let ours_has_source = ours_bytes.is_some();
        let theirs_has_source = theirs_bytes.is_some();
        self.conflict_resolver.image_preview = ConflictResolverImagePreviewState {
            source_hash: Some(source_hash),
            path: Some(path.clone()),
            images: ThreeWaySides {
                base: loading_conflict_preview_image(base_has_source),
                ours: loading_conflict_preview_image(ours_has_source),
                theirs: loading_conflict_preview_image(theirs_has_source),
            },
        };

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let rasterize_payloads = move || {
                    (
                        rasterize_conflict_preview_svg_payload(base_bytes),
                        rasterize_conflict_preview_svg_payload(ours_bytes),
                        rasterize_conflict_preview_svg_payload(theirs_bytes),
                    )
                };
                let (base_payload, ours_payload, theirs_payload) =
                    if crate::ui_runtime::current().uses_background_compute() {
                        smol::unblock(rasterize_payloads).await
                    } else {
                        rasterize_payloads()
                    };

                let _ = view.update(cx, |this, cx| {
                    if this.conflict_resolver.image_preview.source_hash != Some(source_hash)
                        || this.conflict_resolver.image_preview.path.as_ref() != Some(&path)
                    {
                        return;
                    }

                    this.conflict_resolver.image_preview.images.base =
                        loadable_conflict_preview_svg_image(base_payload, base_has_source);
                    this.conflict_resolver.image_preview.images.ours =
                        loadable_conflict_preview_svg_image(ours_payload, ours_has_source);
                    this.conflict_resolver.image_preview.images.theirs =
                        loadable_conflict_preview_svg_image(theirs_payload, theirs_has_source);
                    cx.notify();
                });
            },
        )
        .detach();
    }

    pub(in crate::view) fn is_worktree_target_directory(&self) -> bool {
        let Some(DiffTarget::WorkingTree { path, .. }) = self.rendered_diff_target() else {
            return false;
        };
        let Some(workdir) = self.rendered_diff_workdir() else {
            return false;
        };
        let abs_path = if path.is_absolute() {
            path.clone()
        } else {
            workdir.join(path)
        };
        abs_path.is_dir()
    }

    pub(in crate::view) fn untracked_directory_notice(&self) -> Option<SharedString> {
        let repo = self.active_repo()?;
        let DiffTarget::WorkingTree { path, area } = repo.diff_state.diff_target.as_ref()? else {
            return None;
        };
        let abs_path = if path.is_absolute() {
            path.clone()
        } else {
            repo.spec.workdir.join(path)
        };
        if !abs_path.is_dir() {
            return None;
        }

        let is_untracked = *area == DiffArea::Unstaged
            && repo
                .status_entry_for_path(DiffArea::Unstaged, path.as_path())
                .is_some_and(|entry| entry.kind == FileStatusKind::Untracked);

        if is_untracked {
            Some(
                "Folder is untracked. Select a file inside it, or stage the folder to inspect tracked changes."
                    .into(),
            )
        } else {
            Some(
                "Selected path is a directory. Select a file inside it to preview its contents."
                    .into(),
            )
        }
    }

    pub(in crate::view) fn worktree_preview_line_count(&self) -> Option<usize> {
        match &self.worktree_preview {
            Loadable::Ready(line_count) => Some(*line_count),
            _ => None,
        }
    }

    /// Rows the file preview list draws.
    ///
    /// With word wrap on a long line occupies several of them, so this is not
    /// the file's line count — every caller that indexes the list wants this
    /// one, and everything that means "a line of the file" wants the other.
    pub(in crate::view) fn worktree_preview_visible_len(&self) -> Option<usize> {
        let line_count = self.worktree_preview_line_count()?;
        if !self.worktree_preview_wrap_active() {
            return Some(line_count);
        }
        Some(self.diff_wrap_visible_rows.len())
    }

    /// Whether the file preview's rows are currently a wrap projection of its
    /// lines rather than the lines themselves.
    pub(in crate::view) fn worktree_preview_wrap_active(&self) -> bool {
        self.is_file_preview_active()
            && self.diff_word_wrap
            && self.diff_wrap_visible_cache_key.is_some()
            && !self.diff_wrap_visible_rows.is_empty()
    }

    pub(in crate::view) fn worktree_preview_line_raw_text(
        &self,
        line_ix: usize,
    ) -> Option<gitcomet_core::file_diff::FileDiffLineText> {
        let range = indexed_line_byte_range(
            self.worktree_preview_line_starts.as_ref(),
            self.worktree_preview_source_len,
            line_ix,
        )?;

        if self.worktree_preview_source_len > 0 && self.worktree_preview_text.is_empty() {
            let source_path = Arc::new(self.worktree_preview_source_path.clone()?);
            let flags = self
                .worktree_preview_line_flags
                .get(line_ix)
                .copied()
                .unwrap_or_default();
            return Some(gitcomet_core::file_diff::FileDiffLineText::file_slice(
                source_path,
                range,
                preview_line_is_ascii_without_loading(flags),
                preview_line_has_tabs_without_loading(flags),
            ));
        }

        Some(worktree_preview_materialized_line_raw_text(
            &self.worktree_preview_text,
            range,
        ))
    }

    #[cfg(test)]
    pub(in crate::view) fn worktree_preview_line_text(
        &self,
        line_ix: usize,
    ) -> Option<Cow<'_, str>> {
        if self.worktree_preview_source_len > 0 && self.worktree_preview_text.is_empty() {
            return self
                .worktree_preview_line_raw_text(line_ix)
                .map(|line| Cow::Owned(line.as_ref().to_string()));
        }

        let range = indexed_line_byte_range(
            self.worktree_preview_line_starts.as_ref(),
            self.worktree_preview_source_len,
            line_ix,
        )?;
        Some(Cow::Borrowed(
            self.worktree_preview_text
                .as_ref()
                .get(range)
                .unwrap_or_default(),
        ))
    }

    /// Rows the active markdown preview list renders, or `None` when no
    /// markdown preview is active.
    ///
    /// This is the list length, so with word wrap on it counts visual rows —
    /// every caller indexes rows by list position.
    pub(in crate::view) fn markdown_preview_row_count(&self) -> Option<usize> {
        if self.is_file_preview_active() {
            if let Loadable::Ready(doc) = &self.worktree_markdown_preview {
                // The single document flows rather than wrapping into a fixed
                // row grid, so a list position is always a source row index.
                return Some(doc.rows.len());
            }
            return None;
        }
        if let Loadable::Ready(diff) = &self.file_markdown_preview {
            let wrapped_len = |list, rows: usize| {
                self.markdown_preview_wrap_plan(list)
                    .map_or(rows, |plan| plan.len())
            };
            return Some(match self.diff_view {
                DiffViewMode::Inline => {
                    wrapped_len(MarkdownPreviewList::Inline, diff.inline.rows.len())
                }
                DiffViewMode::Split => wrapped_len(MarkdownPreviewList::Old, diff.old.rows.len())
                    .max(wrapped_len(MarkdownPreviewList::New, diff.new.rows.len())),
            });
        }
        None
    }

    /// Number of visual rows belonging to one selectable Markdown region.
    ///
    /// Split previews can have different document lengths, so EOF resolution
    /// must use the selected column rather than the combined row count. The
    /// flowing worktree preview has no wrap plan and therefore returns its
    /// document-row count directly.
    pub(in crate::view) fn markdown_preview_region_row_count(
        &self,
        region: DiffTextRegion,
    ) -> Option<usize> {
        let (list, document) = self.markdown_preview_list_for_region(region)?;
        Some(
            self.markdown_preview_wrap_plan(list)
                .map_or(document.rows.len(), |plan| plan.len()),
        )
    }

    /// Returns the text painted by the markdown preview row at `visible_ix`
    /// for the given `region`. For file preview (added/deleted/untracked) only
    /// `DiffTextRegion::Inline` is meaningful.
    ///
    /// `visible_ix` is a list position, which is a source row index only while
    /// word wrap is off. With wrap on, a source row occupies several list rows
    /// and each paints one slice of its text, so the wrap plan has to resolve
    /// the index — otherwise selection, hit testing, and copy all operate on a
    /// different row than the one under the pointer.
    pub(in crate::view) fn markdown_preview_row_text(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> SharedString {
        self.markdown_preview_row_at(visible_ix, region)
            .map(|(row, visual)| match visual {
                Some(visual) => visual.text_slice(row),
                None => row.text.clone(),
            })
            .unwrap_or_default()
    }

    /// Byte length of [`Self::markdown_preview_row_text`] without building it.
    ///
    /// The selection overlay asks for this for every visible row on every
    /// frame, and slicing a wrapped row allocates, so the length is taken
    /// straight from the plan instead.
    pub(in crate::view) fn markdown_preview_row_text_len(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> usize {
        self.markdown_preview_row_at(visible_ix, region)
            .map(|(row, visual)| match visual {
                Some(visual) => row.text.get(visual.byte_range.clone()).map_or(0, str::len),
                None => row.text.len(),
            })
            .unwrap_or(0)
    }

    /// Arrange for the pane to repaint when a picture in the rendered preview
    /// finishes decoding.
    ///
    /// `gpui` decodes an image once and hands the result to everyone, but it
    /// only wakes the *first* view that asked for it. A pane that starts
    /// showing a picture another one is already decoding is therefore never
    /// told the decode finished, and holds an empty slot until something
    /// unrelated happens to repaint it. Animated pictures are where this bites:
    /// `gpui` decodes every frame before it yields anything, so a long GIF
    /// takes seconds — time enough to open the same document in a second tab.
    pub(in crate::view) fn watch_pending_markdown_preview_images(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        use futures::FutureExt as _;

        let Loadable::Ready(document) = &self.worktree_markdown_preview else {
            return;
        };
        // A document past the flowing renderer's budget is shown as source, so
        // it draws no pictures and there is nothing to wait for.
        if document.rows.len() > crate::view::markdown_preview::MAX_FLOWING_PREVIEW_ROWS {
            return;
        }
        let document = Arc::clone(document);
        let base_dir = self.markdown_preview_image_base_dir();

        let mut resources = Vec::new();
        let mut push = |source: &str| {
            if let Some(resolved) =
                crate::view::rows::markdown_preview_image_source(base_dir.as_deref(), source)
            {
                resources.push(resolved.to_resource());
            }
        };
        for row in document.rows.iter() {
            // Only the first band of a picture draws it; the rest are height.
            if !row.continues_a_picture()
                && let Some(image) = row.image.as_ref()
            {
                push(image.source.as_ref());
            }
            for inline in row.inline_images.iter() {
                push(inline.image.source.as_ref());
            }
        }

        for resource in resources {
            if self
                .worktree_markdown_preview_image_waits
                .contains(&resource)
            {
                continue;
            }
            let (task, _) = cx.fetch_asset::<gpui::ImgResourceLoader>(&resource);
            if task.clone().now_or_never().is_some() {
                continue;
            }
            self.worktree_markdown_preview_image_waits
                .insert(resource.clone());
            cx.spawn(async move |view, cx| {
                // Whether the picture decoded or failed, the pane has to hear
                // about it: a failure is what draws the stand-in.
                let _ = task.await;
                let _ = view.update(cx, |this, cx| {
                    this.worktree_markdown_preview_image_waits.remove(&resource);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Whether a markdown preview row only continues a picture an earlier row
    /// already carries.
    ///
    /// An image block occupies as many rows as it is tall so the row grid can
    /// give it height, and every one of them carries the picture's alt text.
    /// Only the first is a line of the document, so copying a selection that
    /// runs over a picture would otherwise repeat its alt text once per row.
    ///
    /// Only the rendered preview is laid out that way. Text mode is showing the
    /// file, where a row index is a line number, and the parsed document that
    /// is still cached beside it describes nothing about those lines.
    pub(in crate::view) fn markdown_preview_row_repeats_a_picture(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> bool {
        self.is_markdown_preview_active()
            && self
                .markdown_preview_row_at(visible_ix, region)
                .is_some_and(|(row, _)| row.continues_a_picture())
    }

    /// Directory that relative image paths in the rendered preview resolve
    /// against — the directory of the file being previewed.
    ///
    /// Images are read from the working tree even when the preview shows an
    /// older revision of the document: the historical blob is not on disk, and
    /// showing the current picture beats showing nothing.
    pub(in crate::view) fn markdown_preview_image_base_dir(&self) -> Option<std::path::PathBuf> {
        let repo = self.active_repo()?;
        let workdir = repo.spec.workdir.clone();
        let path = match repo.diff_state.diff_target.as_ref()? {
            DiffTarget::WorkingTree { path, .. } => path.clone(),
            DiffTarget::Commit { path, .. } | DiffTarget::CommitRange { path, .. } => {
                path.clone()?
            }
        };
        let absolute = if path.is_absolute() {
            path
        } else {
            workdir.join(path)
        };
        absolute.parent().map(ToOwned::to_owned)
    }

    /// Web link under `position` in a rendered markdown preview row, and where
    /// it sits in that row.
    ///
    /// Preview rows paint link *text*, not the destination, so the URL comes
    /// from the inline span the click landed in. Offsets from the hitbox are
    /// relative to the slice a row painted, which is the whole row only while
    /// word wrap is off — and the range comes back in that same space, so it
    /// can be turned back into a box on screen. A link that began on an earlier
    /// visual line is clamped to the start of this one, which is where it does
    /// begin as far as this row is concerned.
    pub(in crate::view) fn markdown_preview_link_span_at(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) -> Option<(SharedString, Range<usize>)> {
        if !self.is_markdown_preview_active() {
            return None;
        }
        let (row, visual) = self.markdown_preview_row_at(visible_ix, region)?;
        let slice_start = visual.map_or(0, |visual| visual.byte_range.start);
        let offset =
            slice_start + self.diff_text_offset_for_position(visible_ix, region, position)?;

        row.inline_spans
            .iter()
            .find(|span| span.byte_range.contains(&offset))
            .and_then(|span| {
                let url = span.link_url.clone()?;
                let start = span.byte_range.start.saturating_sub(slice_start);
                let end = span.byte_range.end.saturating_sub(slice_start);
                Some((url, start..end))
            })
    }

    /// The wrap plan `list` renders with, once it is confirmed to describe the
    /// preview document currently loaded rather than one it replaced.
    pub(in crate::view) fn markdown_preview_wrap_plan(
        &self,
        list: MarkdownPreviewList,
    ) -> Option<&crate::view::markdown_preview::MarkdownPreviewWrapPlan> {
        self.markdown_preview_wrap
            .plan_for_rev(list, self.file_markdown_preview_seq)
    }

    /// The source row a list position paints, plus the visual row describing
    /// which slice of it — `None` when the list is not wrapped.
    fn markdown_preview_row_at(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> Option<(&MarkdownPreviewRow, Option<&MarkdownPreviewVisualRow>)> {
        let (list, document) = self.markdown_preview_list_for_region(region)?;
        let Some(plan) = self.markdown_preview_wrap_plan(list) else {
            return document.rows.get(visible_ix).map(|row| (row, None));
        };
        let visual = plan.get(visible_ix)?;
        document
            .rows
            .get(visual.row_ix)
            .map(|row| (row, Some(visual)))
    }

    /// The preview list and document a diff text region reads from.
    fn markdown_preview_list_for_region(
        &self,
        region: DiffTextRegion,
    ) -> Option<(MarkdownPreviewList, &MarkdownPreviewDocument)> {
        if self.is_file_preview_active() {
            let Loadable::Ready(doc) = &self.worktree_markdown_preview else {
                return None;
            };
            return Some((MarkdownPreviewList::Worktree, doc.as_ref()));
        }

        let Loadable::Ready(diff) = &self.file_markdown_preview else {
            return None;
        };

        Some(match self.diff_view {
            DiffViewMode::Inline => (MarkdownPreviewList::Inline, &diff.inline),
            DiffViewMode::Split => match region {
                DiffTextRegion::SplitLeft | DiffTextRegion::Inline => {
                    (MarkdownPreviewList::Old, &diff.old)
                }
                DiffTextRegion::SplitRight => (MarkdownPreviewList::New, &diff.new),
            },
        })
    }

    /// Whether a rendered markdown preview owns the view — whether or not it
    /// actually has a document on screen to search.
    ///
    /// The distinction matters while the preview is still parsing, or when it
    /// failed to: the pane paints a notice, and the markdown source underneath
    /// is not what the reader is looking at, so search reports nothing rather
    /// than quietly scanning a view that is not there. (A document that parses
    /// but is too big to lay out never gets here — `build_single_markdown_preview_document`
    /// refuses it and the pane switches itself to Source.)
    pub(in crate::view) fn rendered_markdown_preview_owns_view(&self) -> bool {
        self.is_conflict_rendered_markdown_preview_active() || self.is_markdown_preview_active()
    }

    /// Which rendered markdown surface Ctrl+F should search, if any.
    ///
    /// Search used to answer this by flipping the preview back to Source and
    /// searching the markdown text. It searches the rendered rows in place
    /// instead, so it has to know which of the four list shapes is on screen —
    /// they have different row spaces and different ways of being scrolled.
    pub(in crate::view) fn markdown_search_surface(&self) -> Option<MarkdownSearchSurface> {
        if self.is_conflict_rendered_markdown_preview_active() {
            return Some(MarkdownSearchSurface::Conflict);
        }
        if !self.is_markdown_preview_active() {
            return None;
        }
        if self.is_file_preview_active() {
            if !matches!(self.worktree_markdown_preview, Loadable::Ready(_)) {
                return None;
            }
            return Some(MarkdownSearchSurface::Worktree);
        }
        if !matches!(self.file_markdown_preview, Loadable::Ready(_)) {
            return None;
        }
        Some(match self.diff_view {
            DiffViewMode::Inline => MarkdownSearchSurface::DiffInline,
            DiffViewMode::Split => MarkdownSearchSurface::DiffSplit,
        })
    }

    /// The quick-search state the markdown preview renderers paint under.
    ///
    /// One value for every list on screen: the split diff's two sides share a
    /// visual row space by construction, and the conflict columns are addressed
    /// by the same index, so the current-match row means the same thing in each.
    pub(in crate::view) fn markdown_preview_search_query(
        &self,
    ) -> Option<crate::view::rows::MarkdownPreviewQuery> {
        if !self.diff_search_active || self.markdown_search_surface().is_none() {
            return None;
        }
        let query = self.diff_search_query.clone();
        let matcher = super::diff_search::DiffSearchMatcher::new(
            query.as_ref(),
            self.diff_search_options_or_default(),
        );
        if matcher.is_empty() || matcher.regex_error().is_some() {
            return None;
        }
        Some(crate::view::rows::MarkdownPreviewQuery {
            matcher: std::sync::Arc::new(matcher),
            current_row: self.diff_search_current_match_row(),
        })
    }

    /// The documents a markdown surface shows, in the order their lists are
    /// laid out, each paired with the wrap plan its list renders through —
    /// `None` for a list that paints one row per source row.
    pub(in crate::view) fn markdown_search_documents(
        &self,
        surface: MarkdownSearchSurface,
    ) -> Vec<(Option<MarkdownPreviewList>, &MarkdownPreviewDocument)> {
        match surface {
            MarkdownSearchSurface::Worktree => match &self.worktree_markdown_preview {
                // The flowing renderer wraps natively and keeps no plan.
                Loadable::Ready(document) => vec![(None, document.as_ref())],
                _ => Vec::new(),
            },
            MarkdownSearchSurface::DiffInline => match &self.file_markdown_preview {
                Loadable::Ready(diff) => {
                    vec![(Some(MarkdownPreviewList::Inline), &diff.inline)]
                }
                _ => Vec::new(),
            },
            MarkdownSearchSurface::DiffSplit => match &self.file_markdown_preview {
                Loadable::Ready(diff) => vec![
                    (Some(MarkdownPreviewList::Old), &diff.old),
                    (Some(MarkdownPreviewList::New), &diff.new),
                ],
                _ => Vec::new(),
            },
            MarkdownSearchSurface::Conflict => [
                ThreeWayColumn::Base,
                ThreeWayColumn::Ours,
                ThreeWayColumn::Theirs,
            ]
            .into_iter()
            .filter_map(|side| {
                // The conflict columns are plain unwrapped lists.
                match self.conflict_resolver.markdown_preview.document(side) {
                    Loadable::Ready(document) => Some((None, document.as_ref())),
                    _ => None,
                }
            })
            .collect(),
        }
    }

    pub(in super::super::super) fn untracked_worktree_preview_path(
        &self,
    ) -> Option<std::path::PathBuf> {
        let repo = self.active_repo()?;
        let workdir = repo.spec.workdir.clone();
        let DiffTarget::WorkingTree { path, area } = repo.diff_state.diff_target.as_ref()? else {
            return None;
        };
        if *area != DiffArea::Unstaged {
            return None;
        }
        let is_untracked = repo
            .status_entry_for_path(DiffArea::Unstaged, path.as_path())
            .is_some_and(|entry| entry.kind == FileStatusKind::Untracked);
        is_untracked.then(|| {
            if path.is_absolute() {
                path.clone()
            } else {
                workdir.join(path)
            }
        })
    }

    pub(in super::super::super) fn added_file_preview_abs_path(
        &self,
    ) -> Option<std::path::PathBuf> {
        let repo = self.active_repo()?;
        let workdir = repo.spec.workdir.clone();
        let target = repo.diff_state.diff_target.as_ref()?;

        match target {
            DiffTarget::WorkingTree { path, area } => {
                if *area != DiffArea::Staged {
                    return None;
                }
                let is_added = repo
                    .status_entry_for_path(DiffArea::Staged, path.as_path())
                    .is_some_and(|entry| entry.kind == FileStatusKind::Added);
                if !is_added {
                    return None;
                }
                Some(if path.is_absolute() {
                    path.clone()
                } else {
                    workdir.join(path)
                })
            }
            DiffTarget::Commit {
                commit_id,
                path: Some(path),
            } => {
                let details = match &repo.history_state.commit_details {
                    Loadable::Ready(d) => d,
                    _ => return None,
                };
                if &details.id != commit_id {
                    return None;
                }
                let is_added = details
                    .files
                    .iter()
                    .any(|f| f.kind == FileStatusKind::Added && &f.path == path);
                if !is_added {
                    return None;
                }
                Some(workdir.join(path))
            }
            _ => None,
        }
    }

    pub(in super::super::super) fn deleted_file_preview_abs_path(
        &self,
    ) -> Option<std::path::PathBuf> {
        let repo = self.active_repo()?;
        let workdir = repo.spec.workdir.clone();
        let target = repo.diff_state.diff_target.as_ref()?;

        match target {
            DiffTarget::WorkingTree { path, area } => {
                let is_deleted = repo
                    .status_entry_for_path(*area, path.as_path())
                    .is_some_and(|entry| entry.kind == FileStatusKind::Deleted);
                if !is_deleted {
                    return None;
                }
                Some(if path.is_absolute() {
                    path.clone()
                } else {
                    workdir.join(path)
                })
            }
            DiffTarget::Commit {
                commit_id,
                path: Some(path),
            } => {
                let details = match &repo.history_state.commit_details {
                    Loadable::Ready(d) => d,
                    _ => return None,
                };
                if &details.id != commit_id {
                    return None;
                }
                let is_deleted = details
                    .files
                    .iter()
                    .any(|f| f.kind == FileStatusKind::Deleted && &f.path == path);
                if !is_deleted {
                    return None;
                }
                Some(workdir.join(path))
            }
            _ => None,
        }
    }

    /// Display path for a file opened via the file browser's "open content"
    /// (working-tree path on disk, or the file path within a commit).
    pub(in super::super::super) fn content_preview_abs_path(&self) -> Option<std::path::PathBuf> {
        let repo = self.active_repo()?;
        if !repo.diff_state.content_preview {
            return None;
        }
        let workdir = repo.spec.workdir.clone();
        match repo.diff_state.diff_target.as_ref()? {
            DiffTarget::WorkingTree { path, .. } => Some(if path.is_absolute() {
                path.clone()
            } else {
                workdir.join(path)
            }),
            DiffTarget::Commit {
                path: Some(path), ..
            } => Some(workdir.join(path)),
            _ => None,
        }
    }

    /// Source path the preview reads from: working-tree content is read straight
    /// from disk; commit content comes from the New-side blob temp file the diff
    /// effect materializes.
    pub(in super::super::super) fn content_preview_source_path(
        &self,
    ) -> Option<std::path::PathBuf> {
        let repo = self.active_repo()?;
        if !repo.diff_state.content_preview {
            return None;
        }
        match repo.diff_state.diff_target.as_ref()? {
            DiffTarget::WorkingTree { .. } => self.content_preview_abs_path(),
            DiffTarget::Commit { .. } => self.preview_text_file_source_path_for_side(
                gitcomet_core::domain::DiffPreviewTextSide::New,
            ),
            _ => None,
        }
    }

    fn preview_text_file_source_path_for_side(
        &self,
        side: gitcomet_core::domain::DiffPreviewTextSide,
    ) -> Option<std::path::PathBuf> {
        let repo = self.active_repo()?;
        match &repo.diff_state.diff_preview_text_file {
            Loadable::Ready(Some(file)) if file.side == side => Some(file.path.clone()),
            _ => None,
        }
    }

    pub(in super::super::super) fn added_file_preview_source_path(
        &self,
    ) -> Option<std::path::PathBuf> {
        self.added_file_preview_abs_path()?;
        self.preview_text_file_source_path_for_side(gitcomet_core::domain::DiffPreviewTextSide::New)
    }

    pub(in super::super::super) fn deleted_file_preview_source_path(
        &self,
    ) -> Option<std::path::PathBuf> {
        self.deleted_file_preview_abs_path()?;
        self.preview_text_file_source_path_for_side(gitcomet_core::domain::DiffPreviewTextSide::Old)
    }

    pub(in super::super::super) fn ensure_preview_loading(&mut self, path: std::path::PathBuf) {
        let should_reset = match self.worktree_preview_path.as_ref() {
            Some(p) => p != &path,
            None => true,
        };
        if should_reset {
            self.worktree_preview_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            self.worktree_preview_syntax_language = rows::diff_syntax_language_for_path(&path);
            self.worktree_preview_path = Some(path);
            self.worktree_preview = Loadable::Loading;
            self.reset_worktree_preview_source_state();
            self.reset_diff_horizontal_scroll_state();
        } else if matches!(self.worktree_preview, Loadable::NotLoaded) {
            self.worktree_preview = Loadable::Loading;
            self.reset_worktree_preview_source_state();
            self.reset_diff_horizontal_scroll_state();
        }
    }

    pub(in super::super::super) fn ensure_worktree_preview_loaded(
        &mut self,
        display_path: std::path::PathBuf,
        source_path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        let should_reload = self.worktree_preview_path.as_ref() != Some(&display_path)
            || self.worktree_preview_source_path.as_ref() != Some(&source_path)
            || matches!(self.worktree_preview, Loadable::NotLoaded);
        if !should_reload {
            return;
        }

        self.worktree_preview_syntax_language = rows::diff_syntax_language_for_path(&display_path);
        self.worktree_preview_path = Some(display_path.clone());
        self.worktree_preview = Loadable::Loading;
        self.reset_worktree_preview_source_state();
        self.worktree_preview_source_path = Some(source_path.clone());
        self.reset_diff_horizontal_scroll_state();
        self.worktree_preview_scroll
            .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);

        cx.spawn(async move |view, cx| {
            let index_preview = {
                let source_path_for_task = source_path.clone();
                move || index_utf8_worktree_preview_file(&source_path_for_task)
            };
            let result = if crate::ui_runtime::current().uses_background_compute() {
                smol::unblock(index_preview).await
            } else {
                index_preview()
            };
            let _ = view.update(cx, |this, cx| {
                if this.worktree_preview_path.as_ref() != Some(&display_path)
                    || this.worktree_preview_source_path.as_ref() != Some(&source_path)
                {
                    return;
                }
                this.worktree_preview_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                match result {
                    Ok(preview) => {
                        if let Some(source_text) = preview.source_text {
                            this.set_worktree_preview_ready_materialized_source(
                                display_path.clone(),
                                source_path.clone(),
                                source_text,
                                preview.line_starts,
                                preview.line_flags,
                                cx,
                            );
                        } else {
                            this.set_worktree_preview_ready_indexed_source(
                                display_path.clone(),
                                source_path.clone(),
                                preview.source_len,
                                preview.line_starts,
                                preview.line_flags,
                                cx,
                            );
                        }
                    }
                    Err(e) => {
                        this.worktree_preview = Loadable::Error(e);
                        this.reset_worktree_preview_source_state();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in super::super::super) fn ensure_selected_file_preview_loaded(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self
            .active_repo()
            .is_some_and(|repo| repo.diff_state.content_preview)
        {
            match (
                self.content_preview_abs_path(),
                self.content_preview_source_path(),
            ) {
                (Some(display_path), Some(source_path)) => {
                    self.ensure_worktree_preview_loaded(display_path, source_path, cx);
                }
                (Some(display_path), None) => self.ensure_preview_loading(display_path),
                (None, _) => {}
            }
            return;
        }

        if let Some(path) = self.untracked_worktree_preview_path() {
            self.ensure_worktree_preview_loaded(path.clone(), path, cx);
            return;
        }

        let display_path = self
            .added_file_preview_abs_path()
            .or_else(|| self.deleted_file_preview_abs_path());
        let source_path = self
            .added_file_preview_source_path()
            .or_else(|| self.deleted_file_preview_source_path());

        match (display_path, source_path) {
            (Some(display_path), Some(source_path)) => {
                self.ensure_worktree_preview_loaded(display_path, source_path, cx);
            }
            (Some(display_path), None) => self.ensure_preview_loading(display_path),
            (None, _) => {}
        }
    }
}

fn build_conflict_markdown_preview_documents(
    sources: &ThreeWaySides<SharedString>,
) -> ThreeWaySides<LoadableMarkdownDoc> {
    use crate::view::markdown_preview;

    let build = |source: &str| -> LoadableMarkdownDoc {
        match markdown_preview::parse_markdown(source) {
            Some(document) => Loadable::Ready(Arc::new(document)),
            None => Loadable::Error(
                markdown_preview::single_preview_unavailable_reason(source.len()).to_string(),
            ),
        }
    };
    ThreeWaySides {
        base: build(sources.base.as_ref()),
        ours: build(sources.ours.as_ref()),
        theirs: build(sources.theirs.as_ref()),
    }
}

#[cfg(test)]
mod tests {
    use crate::perf_alloc::measure_allocations;

    use super::*;

    #[test]
    fn build_conflict_markdown_preview_documents_parses_each_side() {
        let documents = build_conflict_markdown_preview_documents(&ThreeWaySides {
            base: "# Base\n".into(),
            ours: "- item\n".into(),
            theirs: "plain text".into(),
        });

        assert!(matches!(documents.base, Loadable::Ready(_)));
        assert!(matches!(documents.ours, Loadable::Ready(_)));
        assert!(matches!(documents.theirs, Loadable::Ready(_)));
    }

    #[test]
    fn worktree_preview_index_line_capacity_hint_is_bounded_for_massive_files() {
        assert_eq!(worktree_preview_index_line_capacity_hint(0), 1);
        assert_eq!(worktree_preview_index_line_capacity_hint(128), 3);
        assert_eq!(
            worktree_preview_index_line_capacity_hint(usize::MAX),
            WORKTREE_PREVIEW_INDEX_LINE_CAPACITY_MAX
        );
    }

    #[test]
    fn materialized_preview_line_raw_text_avoids_full_source_copy() {
        let source: SharedString = "x".repeat(1024 * 1024).into();
        let iterations = 4u64;

        let ((copied_len, copied_slice_len), copied_metrics) = measure_allocations(|| {
            let mut len = 0usize;
            let mut slice_len = 0usize;
            for _ in 0..iterations as usize {
                let copied_source: Arc<str> = Arc::from(source.as_ref());
                let line = gitcomet_core::file_diff::FileDiffLineText::shared_slice(
                    copied_source,
                    0..source.len(),
                );
                len = len.wrapping_add(line.len());
                slice_len =
                    slice_len.wrapping_add(line.slice_bytes(0..16).map_or(0, |slice| slice.len()));
            }
            (len, slice_len)
        });

        let ((shared_len, shared_slice_len), shared_metrics) = measure_allocations(|| {
            let mut len = 0usize;
            let mut slice_len = 0usize;
            for _ in 0..iterations as usize {
                let line = worktree_preview_materialized_line_raw_text(&source, 0..source.len());
                len = len.wrapping_add(line.len());
                slice_len =
                    slice_len.wrapping_add(line.slice_bytes(0..16).map_or(0, |slice| slice.len()));
            }
            (len, slice_len)
        });

        assert_eq!(copied_len, source.len() * iterations as usize);
        assert_eq!(copied_slice_len, 16 * iterations as usize);
        assert_eq!(shared_len, source.len() * iterations as usize);
        assert_eq!(shared_slice_len, 16 * iterations as usize);
        assert!(
            copied_metrics.alloc_bytes >= source.len() as u64 * iterations,
            "copying baseline should allocate the source each time: {copied_metrics:?}"
        );
        assert!(
            shared_metrics.alloc_bytes.saturating_mul(8) < copied_metrics.alloc_bytes,
            "materialized preview row lookup should stay far below full-source copy cost: shared={shared_metrics:?} copied={copied_metrics:?}"
        );
    }

    #[test]
    fn build_conflict_markdown_preview_documents_reports_per_side_size_limits() {
        let documents = build_conflict_markdown_preview_documents(&ThreeWaySides {
            base: "x"
                .repeat(crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES + 1)
                .into(),
            ours: "".into(),
            theirs: "".into(),
        });

        let Loadable::Error(message) = documents.base else {
            panic!("expected oversize base preview to error: {documents:?}");
        };
        assert!(
            message.contains("1 MiB"),
            "should mention size limit: {message}"
        );
        assert!(matches!(documents.ours, Loadable::Ready(_)));
        assert!(matches!(documents.theirs, Loadable::Ready(_)));
    }

    #[test]
    fn build_conflict_markdown_preview_documents_handles_empty_sources() {
        let documents = build_conflict_markdown_preview_documents(&ThreeWaySides {
            base: "".into(),
            ours: "".into(),
            theirs: "".into(),
        });

        // Empty sources should still produce Ready documents, not errors.
        assert!(matches!(documents.base, Loadable::Ready(_)));
        assert!(matches!(documents.ours, Loadable::Ready(_)));
        assert!(matches!(documents.theirs, Loadable::Ready(_)));
    }

    #[test]
    fn conflict_markdown_preview_state_document_returns_correct_side() {
        let state = ConflictResolverMarkdownPreviewState {
            source_hash: Some(42),
            documents: build_conflict_markdown_preview_documents(&ThreeWaySides {
                base: "# Base".into(),
                ours: "# Ours".into(),
                theirs: "# Theirs".into(),
            }),
        };

        // Each side should have its own document with the expected content.
        let base = state.document(ThreeWayColumn::Base);
        let ours = state.document(ThreeWayColumn::Ours);
        let theirs = state.document(ThreeWayColumn::Theirs);

        let base_doc = match base {
            Loadable::Ready(d) => d,
            _ => panic!("expected Ready for base"),
        };
        let ours_doc = match ours {
            Loadable::Ready(d) => d,
            _ => panic!("expected Ready for ours"),
        };
        let theirs_doc = match theirs {
            Loadable::Ready(d) => d,
            _ => panic!("expected Ready for theirs"),
        };

        assert!(base_doc.rows[0].text.contains("Base"));
        assert!(ours_doc.rows[0].text.contains("Ours"));
        assert!(theirs_doc.rows[0].text.contains("Theirs"));
    }
}
