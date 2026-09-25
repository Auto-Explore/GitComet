use super::file_disk::{DiskIdentity, DiskStamp, DiskSurface, disk_content_hash, disk_stamp};
use super::*;
use crate::view::markdown_preview::{
    MarkdownPreviewDocument, MarkdownPreviewRow, MarkdownPreviewRowKind, MarkdownTaskMarker,
};
#[cfg(test)]
use std::borrow::Cow;
use std::io::Read;

#[cfg(test)]
thread_local! {
    // Link followability checks, each of which may stat the linked file.
    static LINK_FOLLOWABILITY_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Link followability checks since the last call, which resets the count.
#[cfg(test)]
pub(in crate::view) fn take_link_followability_checks_for_tests() -> usize {
    LINK_FOLLOWABILITY_CHECKS.with(|checks| checks.replace(0))
}

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

#[cfg(test)]
thread_local! {
    static REMOTE_MARKDOWN_IMAGE_ROW_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[inline]
fn record_remote_markdown_image_row_visit() {
    #[cfg(test)]
    REMOTE_MARKDOWN_IMAGE_ROW_VISITS.with(|visits| visits.set(visits.get().saturating_add(1)));
}

#[cfg(test)]
pub(in crate::view) fn reset_remote_markdown_image_row_visits_for_tests() {
    REMOTE_MARKDOWN_IMAGE_ROW_VISITS.with(|visits| visits.set(0));
}

#[cfg(test)]
pub(in crate::view) fn remote_markdown_image_row_visits_for_tests() -> usize {
    REMOTE_MARKDOWN_IMAGE_ROW_VISITS.with(std::cell::Cell::get)
}

struct IndexedWorktreePreview {
    source_len: usize,
    line_starts: Arc<[usize]>,
    line_flags: Arc<[u8]>,
    source_text: Option<SharedString>,
    /// Taken before the read, so a write during it reads as a change.
    stamp: DiskStamp,
    /// Of the materialized text, computed here on the read's thread.
    content_hash: Option<u64>,
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
pub(super) fn worktree_preview_materialized_line_raw_text(
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
) -> Result<(SharedString, DiskStamp, u64), String> {
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
    let stamp = indexed.stamp;
    if let (Some(text), Some(hash)) = (indexed.source_text, indexed.content_hash) {
        return Ok((text, stamp, hash));
    }
    // Between the parse ceiling and the editor's own limit the indexer stops
    // materializing, so read it plainly. Already validated as UTF-8 above.
    std::fs::read_to_string(path)
        .map(|text| {
            let hash = disk_content_hash(text.as_bytes());
            (SharedString::from(text), stamp, hash)
        })
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
    let stamp = disk_stamp(&metadata);

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
    let content_hash = source_text
        .as_ref()
        .map(|text| disk_content_hash(text.as_bytes()));

    Ok(IndexedWorktreePreview {
        source_len,
        line_starts: Arc::from(line_starts),
        line_flags: Arc::from(line_flags),
        source_text,
        stamp,
        content_hash,
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

fn ready_conflict_preview_encoded_image_from_bytes(
    format: gpui::ImageFormat,
    bytes: Option<Vec<u8>>,
) -> LoadableImagePreview {
    match bytes {
        Some(bytes) => Loadable::Ready(Some(ConflictPreviewImage::Encoded(Arc::new(
            gpui::Image::from_bytes(format, bytes),
        )))),
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
            ready_conflict_preview_encoded_image_from_bytes(format, Some(bytes))
        }
        None if had_source => Loadable::Error("Preview unavailable.".into()),
        None => Loadable::Ready(None),
    }
}

fn loadable_conflict_preview_render_image(
    image: Option<Arc<gpui::RenderImage>>,
    had_source: bool,
) -> LoadableImagePreview {
    match image {
        Some(image) => Loadable::Ready(Some(ConflictPreviewImage::Rendered(image))),
        None if had_source => Loadable::Error("Preview unavailable.".into()),
        None => Loadable::Ready(None),
    }
}

fn map_deduplicated_conflict_preview_sides<T: Clone>(
    sides: [Option<Vec<u8>>; 3],
    cancel: &std::sync::atomic::AtomicBool,
    mut map: impl FnMut(&[u8]) -> Option<T>,
) -> [Option<T>; 3] {
    let mut output: [Option<T>; 3] = std::array::from_fn(|_| None);
    let mut unique = Vec::<(Vec<u8>, Option<T>)>::new();

    for (index, bytes) in sides.into_iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let Some(bytes) = bytes else {
            continue;
        };
        if let Some((_, mapped)) = unique
            .iter()
            .find(|(source, _)| source.as_slice() == bytes.as_slice())
        {
            output[index] = mapped.clone();
            continue;
        }

        let mapped = map(&bytes);
        output[index] = mapped.clone();
        unique.push((bytes, mapped));
    }

    output
}

pub(in crate::view) fn release_conflict_preview_render_images<T>(
    preview: &ConflictResolverImagePreviewState,
    cx: &mut gpui::Context<T>,
) {
    let mut images = Vec::new();
    for side in ThreeWayColumn::ALL {
        if let Loadable::Ready(Some(ConflictPreviewImage::Rendered(image))) = preview.image(side)
            && images
                .iter()
                .all(|known: &Arc<gpui::RenderImage>| known.id != image.id)
        {
            images.push(Arc::clone(image));
        }
    }
    if !images.is_empty() {
        cx.defer(move |cx| {
            for image in images {
                cx.drop_image(image, None);
            }
        });
    }
}

impl MainPaneView {
    fn current_remote_markdown_image_documents(&self) -> RemoteMarkdownImageDocumentSet {
        if self.is_conflict_rendered_markdown_preview_active() {
            let documents = &self.conflict_resolver.markdown_preview.documents;
            return RemoteMarkdownImageDocumentSet::Conflict([
                match &documents.base {
                    Loadable::Ready(document) => Some(Arc::clone(document)),
                    _ => None,
                },
                match &documents.ours {
                    Loadable::Ready(document) => Some(Arc::clone(document)),
                    _ => None,
                },
                match &documents.theirs {
                    Loadable::Ready(document) => Some(Arc::clone(document)),
                    _ => None,
                },
            ]);
        } else if self.is_markdown_preview_active() && self.is_file_preview_active() {
            if let Loadable::Ready(document) = &self.worktree_markdown.document {
                return RemoteMarkdownImageDocumentSet::Worktree(Arc::clone(document));
            }
        } else if self.is_markdown_preview_active()
            && let Loadable::Ready(preview) = &self.diff_markdown.preview
        {
            return RemoteMarkdownImageDocumentSet::Diff(Arc::clone(preview));
        }
        RemoteMarkdownImageDocumentSet::None
    }

    fn collect_remote_markdown_image_urls(
        documents: &RemoteMarkdownImageDocumentSet,
    ) -> FxHashSet<SharedString> {
        fn collect(
            document: &crate::view::markdown_preview::MarkdownPreviewDocument,
            urls: &mut FxHashSet<SharedString>,
        ) {
            let mut add = |source: &str| {
                if let Some(url) = crate::view::rows::markdown_preview_remote_image_url(source) {
                    urls.insert(url);
                }
            };
            for row in &document.rows {
                record_remote_markdown_image_row_visit();
                if let Some(image) = &row.image {
                    add(image.source.as_ref());
                }
                for inline in row.inline_images.iter() {
                    add(inline.image.source.as_ref());
                }
            }
        }

        let mut urls = FxHashSet::default();
        match documents {
            RemoteMarkdownImageDocumentSet::None => {}
            RemoteMarkdownImageDocumentSet::Worktree(document) => collect(document, &mut urls),
            RemoteMarkdownImageDocumentSet::Diff(preview) => {
                collect(&preview.old, &mut urls);
                collect(&preview.new, &mut urls);
                collect(&preview.inline, &mut urls);
            }
            RemoteMarkdownImageDocumentSet::Conflict(documents) => {
                for document in documents.iter().flatten() {
                    collect(document, &mut urls);
                }
            }
        }
        urls
    }

    fn remote_markdown_image_summary(&self) -> (Arc<FxHashSet<SharedString>>, bool) {
        let documents = self.current_remote_markdown_image_documents();
        let mut cache = self.remote_markdown_images.summary_cache.borrow_mut();
        let documents_changed = !cache.documents.has_same_identity(&documents);
        if documents_changed {
            cache.urls = Arc::new(Self::collect_remote_markdown_image_urls(&documents));
            cache.documents = documents;
        }

        if documents_changed
            || cache.approval_revision != self.remote_markdown_images.approval_revision
        {
            cache.has_blocked = cache
                .urls
                .iter()
                .any(|url| !self.remote_markdown_images.approved_urls.contains(url));
            cache.approval_revision = self.remote_markdown_images.approval_revision;
        }

        (Arc::clone(&cache.urls), cache.has_blocked)
    }

    fn current_remote_markdown_image_urls(&self) -> Arc<FxHashSet<SharedString>> {
        self.remote_markdown_image_summary().0
    }

    pub(in crate::view) fn has_blocked_remote_markdown_images(&self) -> bool {
        self.remote_markdown_images.policy == RemoteMarkdownImagePolicy::AskBeforeLoading
            && self.remote_markdown_image_summary().1
    }

    pub(in crate::view) fn approve_all_remote_markdown_images(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.remote_markdown_images.policy != RemoteMarkdownImagePolicy::AskBeforeLoading {
            return;
        }
        let urls = self.current_remote_markdown_image_urls();
        let previous_len = self.remote_markdown_images.approved_urls.len();
        Arc::make_mut(&mut self.remote_markdown_images.approved_urls).extend(urls.iter().cloned());
        if self.remote_markdown_images.approved_urls.len() != previous_len {
            self.remote_markdown_images.approval_revision = self
                .remote_markdown_images
                .approval_revision
                .wrapping_add(1);
            cx.notify();
        }
    }

    fn cancel_conflict_image_preview_task(&mut self) {
        if let Some(cancel) = self.conflict_image_preview_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        self.conflict_image_preview_inflight = None;

        // Keep a running task owned so the next preview can await it before
        // starting another blocking decode. Dropping the outer GPUI task does
        // not stop a smol::unblock job that is already running.
        if self
            .conflict_image_preview_task
            .as_ref()
            .is_some_and(gpui::Task::is_ready)
        {
            self.conflict_image_preview_task = None;
        }
    }

    fn supersede_conflict_image_preview_task(&mut self) -> Option<gpui::Task<()>> {
        if let Some(cancel) = self.conflict_image_preview_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        self.conflict_image_preview_inflight = None;
        self.conflict_image_preview_task.take()
    }

    fn release_conflict_render_images(&mut self, cx: &mut gpui::Context<Self>) {
        release_conflict_preview_render_images(&self.conflict_resolver.image_preview, cx);
    }

    pub(in crate::view) fn reset_conflict_image_preview_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.cancel_conflict_image_preview_task();
        self.release_conflict_render_images(cx);
        self.conflict_resolver.image_preview = ConflictResolverImagePreviewState::default();
    }

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

    /// Whether a whole-file preview applies to the target, whatever covers
    /// it; see [`MainPaneSurface`].
    pub(in crate::view) fn is_file_preview_active(&self) -> bool {
        self.main_pane_surface().file_preview_target
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

    /// Whether the rendered markdown preview — of the file, or of its diff —
    /// is what the pane shows.
    pub(in crate::view) fn is_markdown_preview_active(&self) -> bool {
        self.main_pane_surface().markdown_preview
    }

    /// Returns `true` when the current diff target is a conflicted file and
    /// there is an applicable conflict resolver strategy.
    pub(in crate::view) fn is_conflict_resolver_active(&self) -> bool {
        self.conflicted_worktree_target()
            .is_some_and(|(_, kind)| Self::conflict_resolver_strategy(kind, false).is_some())
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

    /// Parse the three sides of a conflicted markdown file for its rendered
    /// preview — in the background, as the other previews do: a large file
    /// takes tens of milliseconds per side.
    pub(in crate::view) fn ensure_conflict_markdown_preview_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
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

        self.conflict_resolver.markdown_preview = ConflictResolverMarkdownPreviewState {
            source_hash: Some(source_hash),
            documents: ThreeWaySides {
                base: Loadable::Loading,
                ours: Loadable::Loading,
                theirs: Loadable::Loading,
            },
            columns: ThreeWaySides::default(),
        };
        let sources = self.conflict_resolver.three_way_text.clone();
        cx.spawn(async move |view, cx| {
            let build = move || {
                let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewParse);
                build_conflict_markdown_preview_documents(&sources)
            };
            let documents = if crate::ui_runtime::current().uses_background_compute() {
                smol::unblock(build).await
            } else {
                build()
            };
            let _ = view.update(cx, |this, cx| {
                // The sources moved on while these parsed.
                if this.conflict_resolver.markdown_preview.source_hash != Some(source_hash) {
                    return;
                }
                this.conflict_resolver.markdown_preview.documents = documents;
                // These documents are what an open search scans; until now
                // there were none, so it found nothing and has to look again.
                this.diff_search_recompute_matches();
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::view) fn ensure_conflict_image_preview_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(source_hash) = self.conflict_resolver.source_hash else {
            self.reset_conflict_image_preview_cache(cx);
            return;
        };
        let Some(path) = self.conflict_resolver.path.clone() else {
            self.reset_conflict_image_preview_cache(cx);
            return;
        };
        let Some(format) = crate::view::diff_utils::image_format_for_path(&path) else {
            self.reset_conflict_image_preview_cache(cx);
            return;
        };
        let conflict_rev = self.conflict_resolver.conflict_rev;

        let previews = &self.conflict_resolver.image_preview;
        let terminal = [
            &previews.images.base,
            &previews.images.ours,
            &previews.images.theirs,
        ]
        .into_iter()
        .all(|image| !matches!(image, Loadable::NotLoaded | Loadable::Loading));
        let current_worker = self.conflict_image_preview_inflight.is_some()
            && self
                .conflict_image_preview_task
                .as_ref()
                .is_some_and(|task| !task.is_ready());
        let cache_ready = previews.source_hash == Some(source_hash)
            && previews.path.as_ref() == Some(&path)
            && previews.conflict_rev == conflict_rev
            && (terminal || current_worker);
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

        let base_has_source = base_bytes.is_some();
        let ours_has_source = ours_bytes.is_some();
        let theirs_has_source = theirs_bytes.is_some();
        let previous_task = self.supersede_conflict_image_preview_task();
        self.release_conflict_render_images(cx);
        self.conflict_image_preview_seq = self.conflict_image_preview_seq.wrapping_add(1);
        let seq = self.conflict_image_preview_seq;
        self.conflict_image_preview_inflight = Some(seq);
        self.conflict_resolver.image_preview = ConflictResolverImagePreviewState {
            source_hash: Some(source_hash),
            path: Some(path.clone()),
            conflict_rev,
            images: ThreeWaySides {
                base: loading_conflict_preview_image(base_has_source),
                ours: loading_conflict_preview_image(ours_has_source),
                theirs: loading_conflict_preview_image(theirs_has_source),
            },
        };

        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.conflict_image_preview_cancel = Some(Arc::clone(&cancel));

        if format != gpui::ImageFormat::Svg {
            let task = cx.spawn(
                async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                    if let Some(previous_task) = previous_task {
                        previous_task.await;
                    }
                    let decode_payloads = move || {
                        map_deduplicated_conflict_preview_sides(
                            [base_bytes, ours_bytes, theirs_bytes],
                            &cancel,
                            |bytes| {
                                super::diff_cache::render_raster_conflict_preview(format, bytes)
                            },
                        )
                    };
                    let [base_image, ours_image, theirs_image] =
                        if crate::ui_runtime::current().uses_background_compute() {
                            smol::unblock(decode_payloads).await
                        } else {
                            decode_payloads()
                        };

                    let _ = view.update(cx, |this, cx| {
                        if this.conflict_image_preview_inflight != Some(seq)
                            || this.conflict_resolver.conflict_rev != conflict_rev
                            || this.conflict_resolver.image_preview.conflict_rev != conflict_rev
                            || this.conflict_resolver.image_preview.source_hash != Some(source_hash)
                            || this.conflict_resolver.image_preview.path.as_ref() != Some(&path)
                        {
                            return;
                        }

                        this.conflict_image_preview_inflight = None;
                        this.conflict_image_preview_cancel = None;
                        this.conflict_resolver.image_preview.images.base =
                            loadable_conflict_preview_render_image(base_image, base_has_source);
                        this.conflict_resolver.image_preview.images.ours =
                            loadable_conflict_preview_render_image(ours_image, ours_has_source);
                        this.conflict_resolver.image_preview.images.theirs =
                            loadable_conflict_preview_render_image(theirs_image, theirs_has_source);
                        cx.notify();
                    });
                },
            );
            self.conflict_image_preview_task = Some(task);
            return;
        }

        let task = cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                if let Some(previous_task) = previous_task {
                    previous_task.await;
                }
                let rasterize_payloads = move || {
                    map_deduplicated_conflict_preview_sides(
                        [base_bytes, ours_bytes, theirs_bytes],
                        &cancel,
                        |bytes| rasterize_conflict_preview_svg_payload(Some(bytes.to_vec())),
                    )
                };
                let [base_payload, ours_payload, theirs_payload] =
                    if crate::ui_runtime::current().uses_background_compute() {
                        smol::unblock(rasterize_payloads).await
                    } else {
                        rasterize_payloads()
                    };

                let _ = view.update(cx, |this, cx| {
                    if this.conflict_image_preview_inflight != Some(seq)
                        || this.conflict_resolver.conflict_rev != conflict_rev
                        || this.conflict_resolver.image_preview.conflict_rev != conflict_rev
                        || this.conflict_resolver.image_preview.source_hash != Some(source_hash)
                        || this.conflict_resolver.image_preview.path.as_ref() != Some(&path)
                    {
                        return;
                    }

                    this.conflict_image_preview_inflight = None;
                    this.conflict_image_preview_cancel = None;
                    this.conflict_resolver.image_preview.images.base =
                        loadable_conflict_preview_svg_image(base_payload, base_has_source);
                    this.conflict_resolver.image_preview.images.ours =
                        loadable_conflict_preview_svg_image(ours_payload, ours_has_source);
                    this.conflict_resolver.image_preview.images.theirs =
                        loadable_conflict_preview_svg_image(theirs_payload, theirs_has_source);
                    cx.notify();
                });
            },
        );
        self.conflict_image_preview_task = Some(task);
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

    /// Rows the active markdown preview renders, or `None` when no markdown
    /// preview is active. Split sides share one aligned row space.
    pub(in crate::view) fn markdown_preview_row_count(&self) -> Option<usize> {
        if self.is_file_preview_active() {
            if let Loadable::Ready(doc) = &self.worktree_markdown.document {
                // The single document flows rather than wrapping into a fixed
                // row grid, so a list position is always a source row index.
                return Some(doc.rows.len());
            }
            return None;
        }
        if let Loadable::Ready(diff) = &self.diff_markdown.preview {
            return Some(match self.diff_view {
                DiffViewMode::Inline => diff.inline.rows.len(),
                DiffViewMode::Split => diff.old.rows.len().max(diff.new.rows.len()),
            });
        }
        None
    }

    /// Logical EOF of one Markdown region as `(row, byte offset)`.
    ///
    /// Split preview documents are padded with `Spacer` rows to align additions
    /// and deletions. That padding is not part of this side's document, so EOF
    /// stays on the last non-spacer row.
    pub(in crate::view) fn markdown_preview_region_eof(
        &self,
        region: DiffTextRegion,
    ) -> Option<(usize, usize)> {
        let document = self.markdown_preview_document_for_region(region)?;
        let row_ix = document
            .rows
            .iter()
            .rposition(|row| !matches!(row.kind, MarkdownPreviewRowKind::Spacer))?;
        Some((row_ix, document.rows.get(row_ix)?.text.len()))
    }

    /// The text of the markdown preview row at `row_ix` in `region`. For
    /// file preview (added/deleted/untracked) only `DiffTextRegion::Inline`
    /// is meaningful.
    pub(in crate::view) fn markdown_preview_row_text(
        &self,
        row_ix: usize,
        region: DiffTextRegion,
    ) -> SharedString {
        self.markdown_preview_row_at(row_ix, region)
            .map(|row| row.text.clone())
            .unwrap_or_default()
    }

    /// Byte length of [`Self::markdown_preview_row_text`] without cloning it.
    pub(in crate::view) fn markdown_preview_row_text_len(
        &self,
        row_ix: usize,
        region: DiffTextRegion,
    ) -> usize {
        self.markdown_preview_row_at(row_ix, region)
            .map_or(0, |row| row.text.len())
    }

    /// Arrange for the pane to repaint when a picture it drew finishes
    /// decoding.
    ///
    /// `gpui` decodes an image once and hands the result to everyone, but it
    /// only wakes the *first* view that asked for it. A pane that starts
    /// showing a picture another one is already decoding is therefore never
    /// told the decode finished, and holds an empty slot until something
    /// unrelated happens to repaint it. Animated pictures are where this bites:
    /// `gpui` decodes every frame before it yields anything, so a long GIF
    /// takes seconds — time enough to open the same document in a second tab.
    ///
    /// `drawn` is what the frame drew, already resolved and permitted, so a
    /// long document's off-screen pictures cost nothing here.
    pub(in crate::view) fn watch_pending_markdown_preview_images(
        &mut self,
        drawn: Vec<gpui::Resource>,
        cx: &mut gpui::Context<Self>,
    ) {
        use futures::FutureExt as _;

        for resource in drawn {
            if self.worktree_markdown.image_waits.contains(&resource) {
                continue;
            }
            let (task, _) = cx.fetch_asset::<gpui::ImgResourceLoader>(&resource);
            if task.clone().now_or_never().is_some() {
                continue;
            }
            self.worktree_markdown.image_waits.insert(resource.clone());
            cx.spawn(async move |view, cx| {
                // Whether the picture decoded or failed, the pane has to hear
                // about it: a failure is what draws the stand-in.
                let _ = task.await;
                let _ = view.update(cx, |this, cx| {
                    this.worktree_markdown.image_waits.remove(&resource);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Whether a rendered row adds no line of its own to copied text: the
    /// spacers a diff pads one side with stand for no line of the document.
    ///
    /// Only the rendered preview is laid out that way. Text mode is showing the
    /// file, where a row index is a line number, and the parsed document that
    /// is still cached beside it describes nothing about those lines.
    pub(in crate::view) fn markdown_preview_row_copies_nothing(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> bool {
        self.is_markdown_preview_active()
            && self
                .markdown_preview_row_at(visible_ix, region)
                .is_some_and(MarkdownPreviewRow::is_alignment_padding)
    }

    /// Where relative picture sources in the rendered preview resolve: the
    /// working tree, and the previewed file's path in it.
    ///
    /// Images are read from the working tree even when the preview shows an
    /// older revision of the document: the historical blob is not on disk, and
    /// showing the current picture beats showing nothing.
    pub(in crate::view) fn markdown_preview_image_root(
        &self,
    ) -> Option<crate::view::rows::MarkdownImageRoot> {
        let repo = self.active_repo()?;
        let workdir = repo.spec.workdir.clone();
        let path = match repo.diff_state.diff_target.as_ref()? {
            DiffTarget::WorkingTree { path, .. } => path.clone(),
            DiffTarget::Commit { path, .. } | DiffTarget::CommitRange { path, .. } => {
                path.clone()?
            }
        };
        let document = crate::view::rows::markdown_preview_document_path(&workdir, &path)?;
        Some(crate::view::rows::MarkdownImageRoot {
            workdir: Arc::from(workdir.as_path()),
            document: Arc::from(document),
        })
    }

    /// Where the merge tool's rendered columns resolve local pictures: beside
    /// the conflicted file in the working tree.
    pub(in crate::view) fn conflict_markdown_image_root(
        &self,
    ) -> Option<crate::view::rows::MarkdownImageRoot> {
        let workdir = self.active_repo()?.spec.workdir.clone();
        let path = self.conflict_resolver.path.as_ref()?;
        let document = crate::view::rows::markdown_preview_document_path(&workdir, path)?;
        Some(crate::view::rows::MarkdownImageRoot {
            workdir: Arc::from(workdir.as_path()),
            document: Arc::from(document),
        })
    }

    /// The menu a click on a rendered-preview link opens, or `None` when the
    /// link is inert here. The row it was clicked in decides which version of
    /// the tree a local link reads from.
    pub(in crate::view) fn markdown_preview_link_popover_kind(
        &self,
        region: DiffTextRegion,
        row_ix: usize,
        destination: &SharedString,
        load_remote_image_url: Option<SharedString>,
    ) -> Option<PopoverKind> {
        use crate::view::markdown_preview::{
            MarkdownLinkTarget, classify_markdown_link_destination,
        };

        match classify_markdown_link_destination(destination)? {
            MarkdownLinkTarget::Web(url) => Some(PopoverKind::WebLinkMenu {
                url,
                load_remote_image_url,
            }),
            MarkdownLinkTarget::LocalFile(path) => self.markdown_preview_local_file_link_menu(
                region,
                row_ix,
                &path,
                load_remote_image_url,
            ),
            // Followed on click, never offered in a menu.
            MarkdownLinkTarget::Anchor(_) => None,
        }
    }

    /// Whether a click on this link does anything: scrolls to its heading or
    /// opens a menu.
    pub(in crate::view) fn markdown_preview_link_is_followable(
        &self,
        region: DiffTextRegion,
        row_ix: usize,
        destination: &SharedString,
    ) -> bool {
        #[cfg(test)]
        LINK_FOLLOWABILITY_CHECKS.with(|checks| checks.set(checks.get() + 1));
        self.markdown_preview_anchor_target(region, destination)
            .is_some()
            || self
                .markdown_preview_link_popover_kind(region, row_ix, destination, None)
                .is_some()
    }

    /// The row the heading a `#fragment` link names sits in.
    fn markdown_preview_anchor_target(
        &self,
        region: DiffTextRegion,
        destination: &str,
    ) -> Option<usize> {
        use crate::view::markdown_preview::{
            MarkdownLinkTarget, classify_markdown_link_destination, markdown_preview_anchor_row,
        };

        let MarkdownLinkTarget::Anchor(fragment) = classify_markdown_link_destination(destination)?
        else {
            return None;
        };
        // Flowing documents index rows, not wrapped lines; both split sides
        // share one row space and one scroller.
        if !matches!(
            self.markdown_search_surface(),
            Some(
                MarkdownSearchSurface::Worktree
                    | MarkdownSearchSurface::DiffInline
                    | MarkdownSearchSurface::DiffSplit
            )
        ) {
            return None;
        }
        let fragment = crate::view::rows::percent_decode_link_path(&fragment);
        markdown_preview_anchor_row(
            self.markdown_preview_document_for_region(region)?,
            &fragment,
        )
    }

    /// Scroll the preview to the heading a `#fragment` link names, and report
    /// whether there was one.
    pub(in crate::view) fn scroll_markdown_preview_to_anchor(
        &mut self,
        region: DiffTextRegion,
        destination: &str,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(row_ix) = self.markdown_preview_anchor_target(region, destination) else {
            return false;
        };
        self.markdown_interaction.reveal.request_top(row_ix);
        cx.notify();
        true
    }

    /// "Open in GitComet" for a link to a file in this repository.
    ///
    /// The entry is greyed out when the working-tree file is not there; a
    /// link at a commit always tries that commit's tree.
    fn markdown_preview_local_file_link_menu(
        &self,
        region: DiffTextRegion,
        row_ix: usize,
        destination: &str,
        load_remote_image_url: Option<SharedString>,
    ) -> Option<PopoverKind> {
        use crate::view::LocalFileLinkSource;

        // An inline submodule diff renders the submodule's file, but the menu
        // dispatches on the parent repository: the two trees do not line up.
        if self.active_inline_submodule_diff().is_some() {
            return None;
        }
        let repo = self.active_repo()?;
        let workdir = repo.spec.workdir.as_path();
        let target = repo.diff_state.diff_target.as_ref()?;
        let (source, path) =
            crate::view::rows::markdown_preview_local_link_target(workdir, target, destination)?;
        let source = if self.markdown_preview_row_shows_old_version(region, row_ix) {
            match target {
                DiffTarget::Commit { commit_id, .. } => {
                    LocalFileLinkSource::ParentOf(commit_id.clone())
                }
                // Staged changes sit on top of HEAD.
                DiffTarget::WorkingTree {
                    area: DiffArea::Staged,
                    ..
                } => LocalFileLinkSource::Version(
                    repo.head_commit_id()
                        .map(gitcomet_core::domain::FileSource::Commit)
                        .unwrap_or(source),
                ),
                // The index has no version to open; the working tree is nearest.
                _ => LocalFileLinkSource::Version(source),
            }
        } else {
            LocalFileLinkSource::Version(source)
        };
        let missing = match &source {
            LocalFileLinkSource::Version(source) => {
                crate::view::rows::markdown_preview_local_link_missing(workdir, source, &path)?
            }
            // Read by the backend, which reports a file that is not there.
            LocalFileLinkSource::ParentOf(_) => false,
        };
        Some(PopoverKind::LocalFileLinkMenu {
            repo_id: repo.id,
            source,
            path,
            missing,
            load_remote_image_url,
        })
    }

    /// Whether row `row_ix` of `region` shows the file as it was before the
    /// change: the old column of a split diff, or an old row of an inline one.
    fn markdown_preview_row_shows_old_version(
        &self,
        region: DiffTextRegion,
        row_ix: usize,
    ) -> bool {
        if self.is_file_preview_active() {
            return false;
        }
        let Loadable::Ready(diff) = &self.diff_markdown.preview else {
            return false;
        };
        match self.diff_view {
            // As `markdown_preview_document_for_region` maps the regions.
            DiffViewMode::Split => region != DiffTextRegion::SplitRight,
            DiffViewMode::Inline => diff.inline_old.get(row_ix).copied().unwrap_or(false),
        }
    }

    /// Link under `position` in a rendered markdown preview row, and where it
    /// sits in that row.
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
        let (row, span_ix) = self.markdown_preview_link_span_ix_at(visible_ix, region, position)?;
        let span = &row.inline_spans[span_ix];
        Some((span.link_url.clone()?, span.byte_range.clone()))
    }

    /// The link span under `position`: its row and the span's index. A point
    /// beside the text is on no link, so hover and click agree on where a
    /// link ends.
    fn markdown_preview_link_span_ix_at(
        &self,
        row_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) -> Option<(&MarkdownPreviewRow, usize)> {
        if !self.is_markdown_preview_active() {
            return None;
        }
        let row = self.markdown_preview_row_at(row_ix, region)?;
        let offset = self.diff_text_offset_on_text(row_ix, region, position)?;
        let span_ix = row
            .inline_spans
            .iter()
            .position(|span| span.byte_range.contains(&offset) && span.link_url.is_some())?;
        Some((row, span_ix))
    }

    /// Point the hovered-link underline and pointer cursor at the link under
    /// `position`, repainting only when that changes. A held button is a drag,
    /// not a hover, and a link a click would not follow is plain words.
    pub(in crate::view) fn update_markdown_preview_link_hover(
        &mut self,
        row_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
        button_held: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let candidate = (!button_held)
            .then(|| self.markdown_preview_link_span_ix_at(row_ix, region, position))
            .flatten()
            .map(|(row, span_ix)| {
                // A link whose text changes style spans several runs; the whole
                // link is what the pointer is on.
                let spans = &row.inline_spans;
                let url = spans[span_ix].link_url.clone();
                let same_link = |a: usize, b: usize| {
                    spans[a].link_url == url && spans[a].byte_range.end == spans[b].byte_range.start
                };
                let mut first = span_ix;
                while first > 0 && same_link(first - 1, first) {
                    first -= 1;
                }
                let mut last = span_ix;
                while last + 1 < spans.len() && same_link(last, last + 1) {
                    last += 1;
                }
                let hovered = rows::MarkdownPreviewHoveredLink {
                    region,
                    row_ix,
                    byte_range: spans[first].byte_range.start..spans[last].byte_range.end,
                };
                (hovered, url)
            });
        let hovered = match candidate {
            // Still on the link already shown, or already found to go
            // nowhere: nothing to re-check.
            Some((hovered, _))
                if self.markdown_interaction.hovered_link.as_ref() == Some(&hovered)
                    || self.markdown_interaction.plain_link.as_ref() == Some(&hovered) =>
            {
                return;
            }
            Some((hovered, Some(url))) => {
                if self.markdown_preview_link_is_followable(region, row_ix, &url) {
                    self.markdown_interaction.plain_link = None;
                    Some(hovered)
                } else {
                    self.markdown_interaction.plain_link = Some(hovered);
                    None
                }
            }
            _ => {
                self.markdown_interaction.plain_link = None;
                None
            }
        };
        if self.markdown_interaction.hovered_link != hovered {
            self.markdown_interaction.hovered_link = hovered;
            cx.notify();
        }
    }

    /// Drop the hovered link: the pointer is off every row. Back on it later,
    /// a link is checked afresh — its file or heading may have appeared.
    pub(in crate::view) fn clear_markdown_preview_link_hover(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.markdown_interaction.plain_link = None;
        if self.markdown_interaction.hovered_link.take().is_some() {
            cx.notify();
        }
    }

    /// The row at `row_ix` of the document `region` reads from.
    fn markdown_preview_row_at(
        &self,
        row_ix: usize,
        region: DiffTextRegion,
    ) -> Option<&MarkdownPreviewRow> {
        self.markdown_preview_document_for_region(region)?
            .rows
            .get(row_ix)
    }

    /// The preview document a diff text region reads from.
    fn markdown_preview_document_for_region(
        &self,
        region: DiffTextRegion,
    ) -> Option<&MarkdownPreviewDocument> {
        if self.is_file_preview_active() {
            let Loadable::Ready(doc) = &self.worktree_markdown.document else {
                return None;
            };
            return Some(doc.as_ref());
        }

        let Loadable::Ready(diff) = &self.diff_markdown.preview else {
            return None;
        };

        Some(match self.diff_view {
            DiffViewMode::Inline => &diff.inline,
            DiffViewMode::Split => match region {
                DiffTextRegion::SplitLeft | DiffTextRegion::Inline => &diff.old,
                DiffTextRegion::SplitRight => &diff.new,
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
        let surface = self.main_pane_surface();
        if !surface.markdown_preview {
            return None;
        }
        if surface.body == MainPaneBody::FilePreview {
            if !matches!(self.worktree_markdown.document, Loadable::Ready(_)) {
                return None;
            }
            return Some(MarkdownSearchSurface::Worktree);
        }
        if !matches!(self.diff_markdown.preview, Loadable::Ready(_)) {
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
    /// The open search as the rendered preview highlights it. The matcher is
    /// the one shared with the text diff, built once per query — a regex
    /// query compiles, and a render asks for it on every frame.
    pub(in crate::view) fn markdown_preview_search_query(
        &mut self,
    ) -> Option<crate::view::rows::MarkdownPreviewQuery> {
        if !self.diff_search_active || self.markdown_search_surface().is_none() {
            return None;
        }
        let matcher = self.diff_search_query_matcher_shared()?;
        if matcher.is_empty() || matcher.regex_error().is_some() {
            return None;
        }
        Some(crate::view::rows::MarkdownPreviewQuery {
            matcher,
            current_row: self.diff_search_current_match_row(),
        })
    }

    /// The documents a markdown surface shows, in the order they are laid out.
    pub(in crate::view) fn markdown_search_documents(
        &self,
        surface: MarkdownSearchSurface,
    ) -> Vec<&MarkdownPreviewDocument> {
        match surface {
            MarkdownSearchSurface::Worktree => match &self.worktree_markdown.document {
                Loadable::Ready(document) => vec![document.as_ref()],
                _ => Vec::new(),
            },
            MarkdownSearchSurface::DiffInline => match &self.diff_markdown.preview {
                Loadable::Ready(diff) => vec![&diff.inline],
                _ => Vec::new(),
            },
            MarkdownSearchSurface::DiffSplit => match &self.diff_markdown.preview {
                Loadable::Ready(diff) => vec![&diff.old, &diff.new],
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
                    Loadable::Ready(document) => Some(document.as_ref()),
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
        // A reload asked for by the disk notice keeps the reader's place;
        // everything else starts at the top.
        let restore_scroll_offset = self.worktree_preview_restore_scroll_offset.take();
        if restore_scroll_offset.is_none() {
            self.reset_diff_horizontal_scroll_state();
            self.worktree_preview_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        }
        let disk_revs = self.current_file_disk_revs();

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
                if restore_scroll_offset.is_none() {
                    this.worktree_preview_scroll
                        .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                }
                match result {
                    Ok(preview) => {
                        this.worktree_preview_disk =
                            DiskIdentity::loaded(preview.stamp, preview.content_hash);
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
                        // The list clamps it on the next paint if the file
                        // got shorter; `cx.notify()` below is that paint.
                        if let Some(offset) = restore_scroll_offset {
                            this.worktree_preview_scroll
                                .0
                                .borrow()
                                .base_handle
                                .set_offset(offset);
                        }
                        // After the ready state: only a ready preview is a
                        // surface the catch-up check can look at.
                        this.file_disk_read_landed(DiskSurface::Preview, disk_revs, cx);
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
    fn conflict_preview_mapping_decodes_identical_payloads_once() {
        let decodes = std::cell::Cell::new(0_usize);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let images = map_deduplicated_conflict_preview_sides(
            [Some(vec![1, 2, 3]), Some(vec![4, 5]), Some(vec![1, 2, 3])],
            &cancel,
            |bytes| {
                decodes.set(decodes.get() + 1);
                Some(bytes.len())
            },
        );

        assert_eq!(images, [Some(3), Some(2), Some(3)]);
        assert_eq!(decodes.get(), 2);
    }

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
            columns: ThreeWaySides::default(),
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

impl MainPaneView {
    /// The working-tree file whose markdown the preview can write task toggles
    /// into: an unstaged file that exists, whose preview — or diff's new side —
    /// is that file's text.
    fn markdown_preview_task_file(
        &self,
    ) -> Option<(RepoId, std::path::PathBuf, std::path::PathBuf)> {
        let repo = self.active_repo()?;
        let DiffTarget::WorkingTree {
            path,
            area: DiffArea::Unstaged,
        } = repo.diff_state.diff_target.as_ref()?
        else {
            return None;
        };
        // A deleted file's preview is its last committed text, and a submodule
        // diff describes a directory: neither has a file to write into.
        let deleted = repo
            .status_entry_for_path(DiffArea::Unstaged, path.as_path())
            .is_some_and(|entry| entry.kind == FileStatusKind::Deleted);
        if deleted || self.is_inline_submodule_diff_active() {
            return None;
        }
        let abs_path = self.absolute_worktree_path(path)?;
        Some((repo.id, path.clone(), abs_path))
    }

    pub(in crate::view) fn markdown_preview_tasks_editable(&self) -> bool {
        self.markdown_preview_task_file().is_some()
    }

    /// Flip one task-list checkbox in the file on disk.
    ///
    /// The file is re-read rather than trusting the preview's text, and the
    /// marker must still sit where it was parsed, so an edit made elsewhere
    /// since the preview loaded is never overwritten.
    pub(in crate::view) fn toggle_markdown_preview_task(
        &mut self,
        region: DiffTextRegion,
        task: MarkdownTaskMarker,
        cx: &mut gpui::Context<Self>,
    ) {
        // The old side of a diff is not the file.
        if matches!(region, DiffTextRegion::SplitLeft) {
            return;
        }
        let Some((repo_id, path, abs_path)) = self.markdown_preview_task_file() else {
            return;
        };
        let error = if self.file_edits_are_unsaved_for(repo_id, &path) {
            Some("Save or discard your unsaved edits to this file first")
        } else {
            match std::fs::read(&abs_path) {
                Ok(bytes) => match toggle_task_marker(bytes, task) {
                    Some(contents) => {
                        self.store.dispatch(Msg::SaveWorktreeFile {
                            repo_id,
                            path,
                            contents: contents.clone(),
                            stage: false,
                        });
                        // The file preview only reloads when its target
                        // changes, and re-reading now could beat the write:
                        // show the text being written, as a fresh read would.
                        if self.worktree_preview_source_path.as_ref() == Some(&abs_path) {
                            let line_starts: Arc<[usize]> = build_line_starts(&contents).into();
                            self.set_worktree_preview_ready_source(
                                abs_path,
                                contents.into(),
                                line_starts,
                                cx,
                            );
                        }
                        None
                    }
                    None => Some("The file changed on disk; the preview is catching up"),
                },
                Err(_) => Some("Couldn't read the file to update the checkbox"),
            }
        };
        if let Some(message) = error {
            let _ = self.root_view.update(cx, |root, cx| {
                root.push_toast(
                    crate::view::components::ToastKind::Error,
                    message.to_string(),
                    cx,
                );
            });
        }
        cx.notify();
    }
}

/// `bytes` with the task marker flipped, or `None` when that marker is no
/// longer where the preview found it, in the state the preview showed.
pub(in crate::view) fn toggle_task_marker(
    mut bytes: Vec<u8>,
    task: MarkdownTaskMarker,
) -> Option<String> {
    let offset = task.byte_offset(&bytes)?;
    let marker = bytes.get(offset..offset + 3)?;
    let checked = match marker {
        b"[ ]" => false,
        b"[x]" | b"[X]" => true,
        _ => return None,
    };
    if checked != task.checked {
        return None;
    }
    bytes[offset + 1] = if checked { b' ' } else { b'x' };
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod task_marker_tests {
    use super::toggle_task_marker;
    use crate::view::markdown_preview::parse_markdown;

    /// Toggle the task on the row reading `text`, as a click on it would.
    fn click(source: &str, text: &str) -> Option<String> {
        let doc = parse_markdown(source).expect("parses");
        let task = doc
            .rows
            .iter()
            .find(|row| row.text.as_ref() == text)
            .and_then(|row| row.task)
            .expect("task row");
        toggle_task_marker(source.as_bytes().to_vec(), task)
    }

    #[test]
    fn toggling_flips_only_the_marker_byte() {
        let source =
            "# Todo\r\n\r\n- [ ] write\r\n  - [X] nested\r\n1. [x] ordered\r\n> * [ ] quoted ✓\n";
        assert_eq!(
            click(source, "write").as_deref(),
            Some(source.replacen("[ ] write", "[x] write", 1).as_str())
        );
        assert_eq!(
            click(source, "nested").as_deref(),
            Some(source.replacen("[X]", "[ ]", 1).as_str())
        );
        assert_eq!(
            click(source, "ordered").as_deref(),
            Some(source.replacen("[x] ordered", "[ ] ordered", 1).as_str())
        );
        assert_eq!(
            click(source, "quoted ✓").as_deref(),
            Some(source.replacen("[ ] quoted", "[x] quoted", 1).as_str())
        );
    }

    #[test]
    fn toggling_twice_restores_the_file() {
        let source = "- [ ] a\n- [x] b\n";
        let once = click(source, "a").expect("toggles");
        assert_eq!(click(&once, "a").as_deref(), Some(source));
    }

    #[test]
    fn a_stale_offset_refuses_to_write() {
        let source = "- [ ] a\n- [ ] b\n";
        let doc = parse_markdown(source).expect("parses");
        let task = doc.rows[1].task.expect("task row");
        // The file gained a line above the item since the preview parsed it.
        let edited = format!("intro\n{source}");
        assert_eq!(toggle_task_marker(edited.into_bytes(), task), None);
        // The file was checked elsewhere since the preview parsed it.
        let checked = source.replacen("[ ] b", "[x] b", 1);
        assert_eq!(toggle_task_marker(checked.into_bytes(), task), None);
        // The file shrank past the marker.
        assert_eq!(toggle_task_marker(b"- [".to_vec(), task), None);
    }
}
