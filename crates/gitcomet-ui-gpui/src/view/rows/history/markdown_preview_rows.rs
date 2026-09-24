use super::*;
use crate::kit::interaction as controls;

pub(in crate::view) const MARKDOWN_PREVIEW_BASE_FONT_PX: f32 = 13.0;
pub(in crate::view) const MARKDOWN_PREVIEW_CONTENT_PAD_X_PX: f32 = 18.0;
pub(in crate::view) const MARKDOWN_PREVIEW_INDENT_STEP_PX: f32 = 24.0;
pub(in crate::view) const MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_WIDTH_PX: f32 = 4.0;
pub(in crate::view) const MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX: f32 = 22.0;
pub(in crate::view) const MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX: f32 = 10.0;
pub(in crate::view) const MARKDOWN_PREVIEW_SHELL_PAD_X_PX: f32 = 12.0;

pub(in crate::view) fn markdown_preview_scaled_px(value: f32, ui_scale_percent: u32) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, ui_scale_percent)
}

/// The rendered-preview link under the pointer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct MarkdownPreviewHoveredLink {
    pub(in crate::view) region: DiffTextRegion,
    /// Document row holding the link, and the link's bytes in its text.
    pub(in crate::view) row_ix: usize,
    pub(in crate::view) byte_range: Range<usize>,
}

impl MarkdownPreviewHoveredLink {
    /// The link bytes to underline in document row `row_ix`.
    pub(in crate::view) fn range_in_row(
        hovered: Option<&Self>,
        region: DiffTextRegion,
        row_ix: usize,
    ) -> Option<&Range<usize>> {
        hovered
            .filter(|hovered| hovered.region == region && hovered.row_ix == row_ix)
            .map(|hovered| &hovered.byte_range)
    }

    /// The cursor row `row_ix` shows.
    pub(in crate::view) fn cursor(
        hovered: Option<&Self>,
        region: DiffTextRegion,
        row_ix: usize,
    ) -> gpui::CursorStyle {
        if hovered.is_some_and(|hovered| hovered.region == region && hovered.row_ix == row_ix) {
            gpui::CursorStyle::PointingHand
        } else {
            gpui::CursorStyle::IBeam
        }
    }
}

/// A snapshot of the per-window permission state used by one render pass.
#[derive(Clone)]
pub(in crate::view) struct MarkdownRemoteImageAccess {
    pub(in crate::view) policy: RemoteMarkdownImagePolicy,
    pub(in crate::view) approved_urls: Arc<FxHashSet<SharedString>>,
    pub(in crate::view) approval_view: Option<Entity<MainPaneView>>,
}

impl Default for MarkdownRemoteImageAccess {
    fn default() -> Self {
        Self {
            policy: RemoteMarkdownImagePolicy::AlwaysLoad,
            approved_urls: Arc::default(),
            approval_view: None,
        }
    }
}

impl MarkdownRemoteImageAccess {
    pub(in crate::view) fn permits(&self, url: &SharedString) -> bool {
        match self.policy {
            RemoteMarkdownImagePolicy::AlwaysLoad => true,
            RemoteMarkdownImagePolicy::AskBeforeLoading => self.approved_urls.contains(url),
            RemoteMarkdownImagePolicy::NeverLoad => false,
        }
    }
}

pub(in crate::view) struct MarkdownPreviewSharedHighlightsText {
    pub(in crate::view) text: SharedString,
    pub(in crate::view) highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
    pub(in crate::view) inner: Option<gpui::StyledText>,
}

impl MarkdownPreviewSharedHighlightsText {
    pub(in crate::view) fn new(
        text: SharedString,
        highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
    ) -> Self {
        Self {
            text,
            highlights,
            inner: None,
        }
    }
}

impl gpui::Element for MarkdownPreviewSharedHighlightsText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut inner = gpui::StyledText::new(self.text.clone())
            .with_default_highlights(&window.text_style(), self.highlights.iter().cloned());
        let layout = inner.request_layout(id, inspector_id, window, cx);
        self.inner = Some(inner);
        layout
    }

    fn prepaint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .as_mut()
            .expect("markdown preview shared-highlights text should be laid out before prepaint")
            .prepaint(id, inspector_id, bounds, request_layout, window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .as_mut()
            .expect("markdown preview shared-highlights text should be laid out before paint")
            .paint(
                id,
                inspector_id,
                bounds,
                request_layout,
                prepaint,
                window,
                cx,
            );
    }
}

impl gpui::IntoElement for MarkdownPreviewSharedHighlightsText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Map a `row.text` byte range onto the tab-expanded text that is painted.
///
/// Styled preview text replaces every tab with [`DIFF_WRAP_TAB_EXPANDED_COLUMNS`]
/// spaces, so raw offsets would slice the painted text in the wrong place —
/// shifted by three bytes per preceding tab, and cutting the tail short.
pub(in crate::view) fn markdown_preview_expanded_slice_range(
    raw_text: &str,
    expanded_len: usize,
    range: &Range<usize>,
) -> Range<usize> {
    if expanded_len == raw_text.len() {
        return range.clone();
    }

    let expand = |offset: usize| {
        let offset = offset.min(raw_text.len());
        let tabs = raw_text.as_bytes()[..offset]
            .iter()
            .filter(|byte| **byte == b'\t')
            .count();
        offset + tabs * (DIFF_WRAP_TAB_EXPANDED_COLUMNS - 1)
    };

    expand(range.start)..expand(range.end)
}

/// Pixel sizes read from picture headers, keyed by the source the document
/// wrote. Empty for anything that could not be measured without decoding.
pub(in crate::view) type MarkdownPreviewPictureSizes = Arc<FxHashMap<SharedString, (u32, u32)>>;

/// Where a markdown image source resolves to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum MarkdownPreviewImageSource {
    /// A file inside the previewed document's own directory tree.
    File(std::path::PathBuf),
    /// An `http(s)` URL, fetched and cached by `gpui`'s image loader.
    Remote(SharedString),
}

impl MarkdownPreviewImageSource {
    /// The key `gpui` stores this picture's decoded frames under.
    ///
    /// Everything that wants to know whether a picture is ready — the element
    /// that draws it and the pane waiting to be told it finished decoding —
    /// has to name it the same way, or they would be asking about two
    /// different entries in the asset cache.
    pub(in crate::view) fn to_resource(&self) -> gpui::Resource {
        match self {
            Self::File(path) => gpui::Resource::from(path.clone()),
            Self::Remote(url) => gpui::Resource::Uri(gpui::SharedUri::from(url.to_string())),
        }
    }
}

#[cfg(test)]
thread_local! {
    // Local picture sources checked on disk, one stat each.
    static MARKDOWN_IMAGE_STATS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Local picture stats since the last call, which resets the count.
#[cfg(test)]
pub(in crate::view) fn take_markdown_image_stats_for_tests() -> usize {
    MARKDOWN_IMAGE_STATS.with(|stats| stats.replace(0))
}

/// Resolve a markdown image source to something the preview can draw.
///
/// A local path must stay inside the repository and out of `.git`, so
/// document content cannot aim the preview at arbitrary files on disk.
/// Anything else — `data:` payloads, other schemes, paths that climb out of
/// the repository — resolves to nothing and falls back to the alt text.
pub(in crate::view) fn markdown_preview_image_source(
    image_root: Option<&MarkdownImageRoot>,
    source: &str,
) -> Option<MarkdownPreviewImageSource> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    if let Some(remote) = markdown_preview_remote_image_url(source) {
        return Some(MarkdownPreviewImageSource::Remote(remote));
    }
    if source.contains("://") || source.starts_with("data:") {
        return None;
    }
    // A local picture resolves the way a link does: against the document,
    // from the repository root when it starts with `/`, and never out of the
    // repository or into `.git` — so document content cannot aim the preview
    // at arbitrary files.
    let root = image_root?;
    let resolved = root
        .workdir
        .join(markdown_preview_local_link_path(&root.document, source)?);
    #[cfg(test)]
    MARKDOWN_IMAGE_STATS.with(|stats| stats.set(stats.get() + 1));
    resolved
        .is_file()
        .then_some(MarkdownPreviewImageSource::File(resolved))
}

/// Where a document's local pictures are read from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct MarkdownImageRoot {
    /// The working tree: pictures are read from it even when the preview shows
    /// an older revision, whose blobs are not on disk.
    pub(in crate::view) workdir: Arc<std::path::Path>,
    /// The previewed document, relative to `workdir`.
    pub(in crate::view) document: Arc<std::path::Path>,
}

/// Repo-relative path a local markdown link names, resolved from the
/// previewed document's own repo-relative path.
///
/// `..` is folded rather than refused — `../README.md` is how a `docs/` page
/// links to the root — but a path that climbs out of the repository, enters
/// `.git`, or is absolute on the OS resolves to nothing. A leading `/` is
/// repository-root-relative, as GitHub reads it.
pub(in crate::view) fn markdown_preview_local_link_path(
    document_path: &std::path::Path,
    destination: &str,
) -> Option<std::path::PathBuf> {
    use std::path::Component;

    let destination = destination.trim();
    // Query and fragment suffixes address something inside the file.
    let destination = destination.split(['#', '?']).next().unwrap_or(destination);
    let destination = percent_decode_link_path(destination);
    let (mut stack, rest) = match destination.strip_prefix('/') {
        Some(rest) => (Vec::new(), rest),
        None => {
            let base = document_path
                .parent()
                .into_iter()
                .flat_map(|dir| dir.components())
                .filter_map(|component| match component {
                    Component::Normal(name) => Some(name.to_os_string()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            (base, destination.as_ref())
        }
    };
    // A link that names nothing of its own (`.`, `..`, `/`) is not a file link.
    let mut names_something = false;
    for component in std::path::Path::new(rest).components() {
        match component {
            Component::CurDir => {}
            // Nothing left to climb out of: the link leaves the repository.
            Component::ParentDir => {
                stack.pop()?;
            }
            Component::Normal(name) => {
                if gitcomet_core::path_utils::is_git_metadata_component(name) {
                    return None;
                }
                // On Windows `./C:..` parses as a Normal `C:..`, which a
                // `PathBuf` reparses as a drive prefix and drops the base.
                let mut reparsed = std::path::Path::new(name).components();
                if !matches!(
                    (reparsed.next(), reparsed.next()),
                    (Some(Component::Normal(_)), None)
                ) {
                    return None;
                }
                stack.push(name.to_os_string());
                names_something = true;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if !names_something || stack.is_empty() {
        return None;
    }
    Some(stack.iter().collect())
}

/// What a local link in the preview of `target` opens: the version of the
/// tree it reads from, and the repo-relative path it names.
///
/// A document shown at a commit links into that commit; a range diff has no
/// single tree to read from, so its links are inert.
pub(in crate::view) fn markdown_preview_local_link_target(
    workdir: &std::path::Path,
    target: &DiffTarget,
    destination: &str,
) -> Option<(gitcomet_core::domain::FileSource, std::path::PathBuf)> {
    use gitcomet_core::domain::FileSource;

    let (document_path, source) = match target {
        DiffTarget::WorkingTree { path, .. } => (path.as_path(), FileSource::WorkingDirectory),
        DiffTarget::Commit {
            commit_id,
            path: Some(path),
        } => (path.as_path(), FileSource::Commit(commit_id.clone())),
        DiffTarget::Commit { path: None, .. } | DiffTarget::CommitRange { .. } => return None,
    };
    let document_path = markdown_preview_document_path(workdir, document_path)?;
    let path = markdown_preview_local_link_path(document_path, destination)?;
    Some((source, path))
}

/// The previewed document's repo-relative path, or `None` when it lies
/// outside `workdir`. Windows calls `\docs\a.md` (rooted, no drive) relative,
/// but it is not repo-relative, so any root or prefix counts as outside.
pub(in crate::view) fn markdown_preview_document_path<'a>(
    workdir: &std::path::Path,
    path: &'a std::path::Path,
) -> Option<&'a std::path::Path> {
    use std::path::Component;

    match path.components().next() {
        Some(Component::Prefix(_) | Component::RootDir) => path.strip_prefix(workdir).ok(),
        _ => Some(path),
    }
}

/// Whether the file a resolved local link names is missing, or `None` when
/// the link must stay inert.
///
/// Only the working tree is on disk. There a symlink can carry a lexically
/// clean path out of the repository or into `.git`, so the canonical
/// destination is checked too. A commit's tree is read by the backend, which
/// reports a file that is not there.
pub(in crate::view) fn markdown_preview_local_link_missing(
    workdir: &std::path::Path,
    source: &gitcomet_core::domain::FileSource,
    path: &std::path::Path,
) -> Option<bool> {
    if *source != gitcomet_core::domain::FileSource::WorkingDirectory {
        return Some(false);
    }
    let (Ok(workdir), Ok(destination)) =
        (workdir.canonicalize(), workdir.join(path).canonicalize())
    else {
        // Nothing (or a dangling link) is there.
        return Some(true);
    };
    let inside = destination.strip_prefix(&workdir).ok()?;
    if inside.components().any(|component| {
        gitcomet_core::path_utils::is_git_metadata_component(component.as_os_str())
    }) {
        return None;
    }
    Some(!destination.is_file())
}

/// Decode `%XX` escapes in a link path. A malformed escape or a result that
/// is not UTF-8 keeps the text as written, which then names no file.
pub(in crate::view) fn percent_decode_link_path(path: &str) -> std::borrow::Cow<'_, str> {
    if !path.contains('%') {
        return std::borrow::Cow::Borrowed(path);
    }
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        if bytes[ix] == b'%' {
            let Some(byte) = bytes
                .get(ix + 1..ix + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            else {
                return std::borrow::Cow::Borrowed(path);
            };
            decoded.push(byte);
            ix += 3;
        } else {
            decoded.push(bytes[ix]);
            ix += 1;
        }
    }
    match String::from_utf8(decoded) {
        Ok(decoded) => std::borrow::Cow::Owned(decoded),
        Err(_) => std::borrow::Cow::Borrowed(path),
    }
}

/// The `http(s)` URL an image source names, if it names one.
///
/// Only these two schemes are followed; anything else a document might carry
/// (`file:`, `javascript:`, and so on) is not something a preview should
/// dereference.
pub(in crate::view) fn markdown_preview_remote_image_url(source: &str) -> Option<SharedString> {
    let source = source.trim();
    let scheme_end = source.find("://")?;
    let scheme = &source[..scheme_end];
    (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        .then(|| SharedString::from(source.to_owned()))
}

/// The quick-search state a markdown preview renders under.
///
/// Carried by both preview renderers — the virtualized lists and the flowing
/// single document — so a Ctrl+F match is washed in place instead of the view
/// having to fall back to the markdown source.
#[derive(Clone)]
pub(in crate::view) struct MarkdownPreviewQuery {
    pub(in crate::view) matcher: Arc<DiffSearchMatcher>,
    /// Visible index of the row the search cursor is on, if it is in this list.
    pub(in crate::view) current_row: Option<usize>,
}

impl MarkdownPreviewQuery {
    pub(in crate::view) fn emphasis(&self, visible_ix: usize) -> DiffSearchMatchEmphasis {
        if self.current_row == Some(visible_ix) {
            DiffSearchMatchEmphasis::Current
        } else {
            DiffSearchMatchEmphasis::Other
        }
    }
}

/// A pending "bring this row into view" request for the flowing markdown
/// preview.
///
/// The flowing document has no fixed row height and is not a `uniform_list`, so
/// there is no `scroll_to_item` to hand the work to: the offset can only be
/// computed once the target row has been laid out. The request is therefore
/// shared into the renderer, which reports the row's bounds back through
/// [`Self::take`] during prepaint and applies the scroll then.
#[derive(Clone, Default)]
pub(in crate::view) struct MarkdownPreviewRevealRequest(
    std::rc::Rc<std::cell::Cell<Option<(usize, MarkdownPreviewRevealAlign)>>>,
);

/// Where a revealed row lands in the viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum MarkdownPreviewRevealAlign {
    /// Search matches sit mid-screen, with context on both sides.
    Center,
    /// An anchor jump puts the heading at the top, as a browser does.
    Top,
}

impl MarkdownPreviewRevealRequest {
    pub(in crate::view) fn request(&self, row_ix: usize) {
        self.0
            .set(Some((row_ix, MarkdownPreviewRevealAlign::Center)));
    }

    pub(in crate::view) fn request_top(&self, row_ix: usize) {
        self.0.set(Some((row_ix, MarkdownPreviewRevealAlign::Top)));
    }

    pub(in crate::view) fn clear(&self) {
        self.0.set(None);
    }

    pub(in crate::view) fn pending(&self) -> Option<usize> {
        self.0.get().map(|(row_ix, _)| row_ix)
    }

    /// Claim the request, so the reveal runs once instead of fighting the user
    /// on every later frame.
    pub(in crate::view) fn take(&self) -> Option<(usize, MarkdownPreviewRevealAlign)> {
        self.0.take()
    }
}

/// The vertical extent of a laid-out row, from the bounds of its parts.
///
/// A row shell holds a marker, an alert badge and the text line; the row is the
/// band they span together.
pub(in crate::view) fn markdown_preview_row_extent(
    children: &[gpui::Bounds<Pixels>],
) -> Option<(Pixels, Pixels)> {
    let top = children
        .iter()
        .map(|bounds| bounds.origin.y)
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;
    let bottom = children
        .iter()
        .map(|bounds| bounds.bottom())
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;
    Some((top, (bottom - top).max(px(0.0))))
}

/// Where a row sits inside a scroll container, and how tall it is.
///
/// Split out from the prepaint listener so the arithmetic that decides the new
/// offset is testable without a window.
pub(in crate::view) fn markdown_preview_reveal_offset_y(
    align: MarkdownPreviewRevealAlign,
    row_top_in_content: Pixels,
    row_height: Pixels,
    viewport_height: Pixels,
    max_offset_y: Pixels,
    current_y: Pixels,
) -> Option<Pixels> {
    if viewport_height <= px(0.0) {
        return None;
    }
    // Place the row the way a uniform list would, then clamp into the
    // scrollable range. Offsets are negative as you scroll down.
    let top = match align {
        MarkdownPreviewRevealAlign::Center => {
            row_top_in_content + row_height / 2.0 - viewport_height / 2.0
        }
        MarkdownPreviewRevealAlign::Top => row_top_in_content,
    };
    let target = (-top).clamp(-max_offset_y.max(px(0.0)), px(0.0));
    (target != current_y).then_some(target)
}

/// Styled text for one row with the search wash layered on, shared with the
/// flowing renderer.
///
/// The base styling lives in a `OnceLock` on the row itself — it belongs to the
/// document, which outlives any one query — so the wash is merged on top per
/// frame rather than stored. Rows with no match return the base untouched, so
/// the extra work is a substring scan per visible row.
pub(in crate::view) fn markdown_preview_styled_row_with_query<'a>(
    theme: AppTheme,
    row: &'a MarkdownPreviewRow,
    visible_ix: usize,
    query: Option<&MarkdownPreviewQuery>,
    hovered_link: Option<&Range<usize>>,
) -> std::borrow::Cow<'a, CachedDiffStyledText> {
    // Only the hovered row pays for a restyle; every other row keeps its cache.
    let base = match hovered_link {
        Some(range) => markdown_preview_hovered_link_styled_text(theme, row, range),
        None => markdown_preview_row_styled_text(theme, row),
    };
    let Some(query) = query.filter(|query| query.matcher.is_match(base.text.as_ref())) else {
        return std::borrow::Cow::Owned(base);
    };
    std::borrow::Cow::Owned(build_cached_diff_query_overlay_styled_text(
        theme,
        &base,
        &query.matcher,
        query.emphasis(visible_ix),
    ))
}

/// The row's styling with the link in `hovered` underlined.
fn markdown_preview_hovered_link_styled_text(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
    hovered: &Range<usize>,
) -> CachedDiffStyledText {
    let underline = markdown_preview_link_hover_underline(theme);
    let highlights = row
        .inline_spans
        .iter()
        .filter_map(|span| {
            let mut style = markdown_preview_inline_highlight(theme, span.style);
            if span.link_url.is_some()
                && hovered.start <= span.byte_range.start
                && span.byte_range.end <= hovered.end
            {
                style.underline = Some(underline);
            }
            (style != gpui::HighlightStyle::default()).then_some((span.byte_range.clone(), style))
        })
        .collect::<Vec<_>>();
    build_cached_diff_styled_text_from_relative_highlights(row.text.as_ref(), &highlights)
}

/// The underline a link shows while the pointer is on it.
fn markdown_preview_link_hover_underline(theme: AppTheme) -> gpui::UnderlineStyle {
    gpui::UnderlineStyle {
        thickness: px(1.0),
        color: Some(theme.colors.accent.foreground.into_color()),
        wavy: false,
    }
}

/// Text element carrying inline highlights, shared with the flowing renderer.
pub(in crate::view) fn markdown_preview_highlighted_text(
    text: SharedString,
    highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
) -> impl IntoElement {
    MarkdownPreviewSharedHighlightsText::new(text, highlights)
}

/// A task-list checkbox, shared with the flowing renderer; `box_size` is the
/// scaled square and the check glyph is drawn inside it.
pub(in crate::view) fn markdown_preview_task_checkbox(
    theme: AppTheme,
    checked: bool,
    box_size: Pixels,
) -> gpui::Div {
    let checkbox = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(box_size)
        .rounded(box_size * (3.0 / 14.0))
        .border_1();
    if checked {
        checkbox
            .bg(theme.colors.accent.solid)
            .border_color(theme.colors.accent.solid)
            .child(crate::view::icons::svg_icon(
                "icons/check.svg",
                theme.colors.accent.on_solid,
                box_size * (11.0 / 14.0),
            ))
    } else {
        checkbox.border_color(theme.colors.foreground.secondary)
    }
}

/// List bullet or number for a row, shared with the flowing renderer.
pub(in crate::view) fn markdown_preview_marker_label(
    row: &MarkdownPreviewRow,
) -> Option<SharedString> {
    markdown_preview_row_marker(row)
}

/// Accent colour for an alert blockquote, shared with the flowing renderer.
pub(in crate::view) fn markdown_preview_alert_bar_color(
    theme: AppTheme,
    kind: MarkdownAlertKind,
) -> gpui::Rgba {
    markdown_preview_alert_color(theme, kind)
}

/// Badge label for an alert blockquote, shared with the flowing renderer.
pub(in crate::view) fn markdown_preview_alert_label(
    kind: MarkdownAlertKind,
) -> Option<SharedString> {
    Some(SharedString::new_static(match kind {
        MarkdownAlertKind::Note => "NOTE",
        MarkdownAlertKind::Tip => "TIP",
        MarkdownAlertKind::Important => "IMPORTANT",
        MarkdownAlertKind::Warning => "WARNING",
        MarkdownAlertKind::Caution => "CAUTION",
    }))
}

/// An image sized the way the document asked: one element that keeps its
/// aspect ratio.
pub(in crate::view) fn markdown_preview_flow_image(
    row: &MarkdownPreviewRow,
    row_ix: usize,
    theme: AppTheme,
    ui_scale_percent: u32,
    pictures: MarkdownPictureContext<'_>,
) -> AnyElement {
    let MarkdownPictureContext {
        picture_sizes,
        remote_image_access,
        ..
    } = pictures;
    let label_color = theme.colors.foreground.secondary;
    let font_size = theme.markdown_px(MARKDOWN_PREVIEW_BASE_FONT_PX, ui_scale_percent);
    let skeleton = markdown_preview_picture_skeleton(row, ui_scale_percent, picture_sizes);

    let source = row.image.as_ref().map(|image| image.source.as_ref());
    if let Some(url) = source.and_then(markdown_preview_remote_image_url)
        && !remote_image_access.permits(&url)
    {
        let blocked = markdown_preview_blocked_image(
            markdown_preview_image_label(row, "Remote image blocked"),
            url,
            format!("markdown_preview_block_image_load_{row_ix}"),
            markdown_preview_scaled_px(MARKDOWN_PREVIEW_BLOCKED_IMAGE_ICON_PX, ui_scale_percent),
            theme,
            remote_image_access,
            false,
        );
        return div()
            .w_full()
            .min_w(px(0.0))
            .child(skeleton.size_element(div().child(blocked)))
            .into_any_element();
    }
    let picture = source.and_then(|source| {
        pictures.resolved_picture(source, ("markdown_preview_block_image", row_ix).into())
    });
    let Some(image) = picture else {
        return markdown_preview_image_placeholder_element(
            markdown_preview_image_label(row, "Image unavailable"),
            font_size,
            label_color,
        )
        .into_any_element();
    };

    let declared = row
        .image
        .as_ref()
        .map(|image| (image.width_px, image.height_px))
        .unwrap_or_default();
    let failed_label = markdown_preview_image_label(row, "Failed to load");
    let image = match declared {
        (Some(width), Some(height)) => image
            .w(markdown_preview_scaled_px(width as f32, ui_scale_percent))
            .h(markdown_preview_scaled_px(height as f32, ui_scale_percent))
            .object_fit(gpui::ObjectFit::Contain),
        (Some(width), None) => image.w(markdown_preview_scaled_px(width as f32, ui_scale_percent)),
        (None, Some(height)) => {
            image.h(markdown_preview_scaled_px(height as f32, ui_scale_percent))
        }
        // Without a declared size the picture keeps its own, up to the width
        // of the document.
        (None, None) => image.max_w_full(),
    };

    div()
        .w_full()
        .min_w(px(0.0))
        .child(
            image
                .debug_selector(move || format!("markdown_preview_block_image_{row_ix}"))
                .with_fallback(move || {
                    markdown_preview_image_placeholder_element(
                        failed_label.clone(),
                        font_size,
                        label_color,
                    )
                    .into_any_element()
                })
                .with_loading(move || skeleton.render(theme)),
        )
        .into_any_element()
}

/// The box a picture will occupy, worked out before it has been decoded.
///
/// `gpui` reads every frame of an animated picture before it reports a size, so
/// a block that waited for that would leave a hole in the document and then
/// shove everything down when the picture arrived. What the document declared
/// comes first; the picture's own header fills in the rest.
#[derive(Clone, Copy)]
pub(in crate::view) struct MarkdownPreviewPictureSkeleton {
    /// Widest the picture will draw, or `None` to fill the document.
    pub(in crate::view) width: Option<Pixels>,
    /// Width over height, or `None` when no intrinsic ratio is available.
    pub(in crate::view) aspect_ratio: Option<f32>,
    /// Used when the aspect ratio is unknown: the declared height, or the rows
    /// the parser set aside when no height was declared.
    pub(in crate::view) reserved_height: Pixels,
}

impl MarkdownPreviewPictureSkeleton {
    fn size_element(self, mut block: gpui::Div) -> gpui::Div {
        block = match self.width {
            Some(width) => block.w(width).max_w_full(),
            None => block.w_full(),
        };
        match self.aspect_ratio {
            Some(ratio) => block.aspect_ratio(ratio),
            None => block.h(self.reserved_height),
        }
    }

    pub(in crate::view) fn render(self, theme: AppTheme) -> AnyElement {
        self.size_element(
            components::skeleton(theme)
                .debug_selector(|| "markdown_preview_picture_skeleton".to_string()),
        )
        .into_any_element()
    }
}

pub(in crate::view) fn markdown_preview_picture_skeleton(
    row: &MarkdownPreviewRow,
    ui_scale_percent: u32,
    picture_sizes: &MarkdownPreviewPictureSizes,
) -> MarkdownPreviewPictureSkeleton {
    let image = row.image.as_ref();
    let declared_width = image.and_then(|image| image.width_px).filter(|w| *w > 0);
    let declared_height = image.and_then(|image| image.height_px).filter(|h| *h > 0);
    // A declared size is in design pixels and scales with the UI; a size read
    // from the file is in the picture's own pixels, which is what `gpui` lays
    // an undeclared picture out at.
    let measured = image
        .and_then(|image| picture_sizes.get(&image.source))
        .copied()
        .filter(|(width, height)| *width > 0 && *height > 0);
    let measured_aspect_ratio = measured.map(|(width, height)| width as f32 / height as f32);

    let width = match (
        declared_width,
        declared_height,
        measured_aspect_ratio,
        measured,
    ) {
        (Some(width), _, _, _) => Some(markdown_preview_scaled_px(width as f32, ui_scale_percent)),
        (None, Some(height), Some(ratio), _) => {
            Some(markdown_preview_scaled_px(height as f32, ui_scale_percent) * ratio)
        }
        (None, None, _, Some((width, _))) => Some(px(width as f32)),
        (None, _, _, _) => None,
    };
    let aspect_ratio = match (declared_width, declared_height, measured_aspect_ratio) {
        (Some(width), Some(height), _) => Some(width as f32 / height as f32),
        (_, _, Some(ratio)) => Some(ratio),
        _ => None,
    };

    MarkdownPreviewPictureSkeleton {
        width,
        aspect_ratio,
        reserved_height: markdown_preview_scaled_px(
            image.map_or(
                crate::view::markdown_preview::MARKDOWN_PREVIEW_IMAGE_DEFAULT_HEIGHT_PX,
                |image| image.reserved_height_px(),
            ) as f32,
            ui_scale_percent,
        ),
    }
}

/// Tallest an inline picture may be when the document declares no size, so a
/// stray screenshot written mid-sentence cannot push the line open.
pub(in crate::view) const MARKDOWN_PREVIEW_INLINE_IMAGE_MAX_HEIGHT_PX: f32 = 26.0;
pub(in crate::view) const MARKDOWN_PREVIEW_BLOCKED_IMAGE_ICON_PX: f32 = 24.0;
pub(in crate::view) const MARKDOWN_PREVIEW_BLOCKED_INLINE_IMAGE_ICON_PX: f32 = 14.0;

/// Space between an inline picture and whatever shares its line.
///
/// Both previews use it, but only the row grid has to reserve it: that preview
/// measures a row's width to drive horizontal scrolling, so the gap is part of
/// the row chrome there and purely visual in the flowing renderer.
pub(in crate::view) const MARKDOWN_PREVIEW_INLINE_IMAGE_GAP_PX: f32 = 4.0;

/// One picture drawn on the same line as the text around it.
///
/// Badges, shields, and a logo beside a heading are all written inline, so they
/// are sized to the line rather than to the document: a declared width wins,
/// and anything else keeps its own size up to the inline height cap.
pub(in crate::view) fn markdown_preview_inline_image(
    inline: &MarkdownInlineImage,
    theme: AppTheme,
    ui_scale_percent: u32,
    pictures: MarkdownPictureContext<'_>,
) -> AnyElement {
    let MarkdownPictureContext {
        picture_sizes,
        remote_image_access,
        ..
    } = pictures;
    let source_byte = inline.source_byte;
    let label_color = theme.colors.foreground.secondary;
    let font_size = theme.markdown_px(MARKDOWN_PREVIEW_BASE_FONT_PX, ui_scale_percent);
    let described = if inline.alt.is_empty() {
        inline.image.source.clone()
    } else {
        inline.alt.clone()
    };
    let measured_aspect_ratio = picture_sizes
        .get(&inline.image.source)
        .filter(|(width, height)| *width > 0 && *height > 0)
        .map(|(width, height)| *width as f32 / *height as f32);
    // A blocked picture must hold the same slot as the loading picture. Remote
    // intrinsic dimensions are deliberately unavailable until permission is
    // granted, but HTML width/height declarations remain authoritative.
    let loading_height = inline.image.height_px.map_or_else(
        || {
            markdown_preview_scaled_px(
                MARKDOWN_PREVIEW_INLINE_IMAGE_MAX_HEIGHT_PX,
                ui_scale_percent,
            )
        },
        |height| markdown_preview_scaled_px(height as f32, ui_scale_percent),
    );
    let loading_width = match (inline.image.width_px, measured_aspect_ratio) {
        (Some(width), _) => markdown_preview_scaled_px(width as f32, ui_scale_percent),
        (None, Some(ratio)) => loading_height * ratio,
        (None, None) => markdown_preview_scaled_px(
            MARKDOWN_PREVIEW_INLINE_IMAGE_LOADING_WIDTH_PX,
            ui_scale_percent,
        ),
    };

    if let Some(url) = markdown_preview_remote_image_url(inline.image.source.as_ref())
        && !remote_image_access.permits(&url)
    {
        return div()
            .flex_none()
            .w(loading_width)
            .h(loading_height)
            .max_w_full()
            .child(markdown_preview_blocked_image(
                markdown_preview_image_reason("Remote image blocked", &described),
                url,
                format!("markdown_preview_inline_image_load_{source_byte}"),
                markdown_preview_scaled_px(
                    MARKDOWN_PREVIEW_BLOCKED_INLINE_IMAGE_ICON_PX,
                    ui_scale_percent,
                ),
                theme,
                remote_image_access,
                inline.link_url.is_some(),
            ))
            .into_any_element();
    }

    let picture = pictures.resolved_picture(
        inline.image.source.as_ref(),
        ("markdown_preview_inline_image", source_byte).into(),
    );
    let Some(image) = picture else {
        return markdown_preview_inline_image_placeholder(
            markdown_preview_image_reason("Image unavailable", &described),
            source_byte,
            font_size,
            label_color,
        );
    };

    let failed_label = markdown_preview_image_reason("Failed to load", &described);
    let image =
        image.debug_selector(move || format!("markdown_preview_inline_image_{source_byte}"));
    let image = match (inline.image.width_px, inline.image.height_px) {
        (Some(width), Some(height)) => image
            .w(markdown_preview_scaled_px(width as f32, ui_scale_percent))
            .h(markdown_preview_scaled_px(height as f32, ui_scale_percent))
            .object_fit(gpui::ObjectFit::Contain),
        (Some(width), None) => image.w(markdown_preview_scaled_px(width as f32, ui_scale_percent)),
        (None, Some(height)) => {
            image.h(markdown_preview_scaled_px(height as f32, ui_scale_percent))
        }
        (None, None) => image.max_h(markdown_preview_scaled_px(
            MARKDOWN_PREVIEW_INLINE_IMAGE_MAX_HEIGHT_PX,
            ui_scale_percent,
        )),
    }
    // The height cap leaves a wide, short banner unbounded, and a declared
    // width can be larger than the pane; either would push the document into
    // horizontal overflow.
    .max_w_full();

    image
        .with_fallback(move || {
            markdown_preview_inline_image_placeholder(
                failed_label.clone(),
                source_byte,
                font_size,
                label_color,
            )
        })
        .with_loading(move || {
            components::skeleton(theme)
                .debug_selector(move || format!("markdown_preview_inline_image_{source_byte}"))
                .flex_none()
                .w(loading_width)
                .h(loading_height)
                .max_w_full()
                .into_any_element()
        })
        .into_any_element()
}

fn markdown_preview_blocked_image(
    label: SharedString,
    url: SharedString,
    control_id: String,
    icon_size: Pixels,
    theme: AppTheme,
    access: &MarkdownRemoteImageAccess,
    linked: bool,
) -> AnyElement {
    let danger = theme.colors.status.danger.foreground;
    let blocked_background = with_alpha(danger, if theme.is_dark { 0.08 } else { 0.05 });
    let base = || {
        div()
            .w_full()
            .h_full()
            .min_w(px(0.0))
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded(px(theme.radii.row))
            .border_1()
            .border_color(theme.colors.stroke.default)
            .bg(blocked_background)
    };

    if access.policy == RemoteMarkdownImagePolicy::AskBeforeLoading {
        let debug_selector = control_id.clone();
        let icon_selector = format!("{control_id}_retry_icon");
        let tooltip = SharedString::from(format!("Load image — {label}"));
        return base()
            .debug_selector(move || debug_selector.clone())
            .id(SharedString::from(control_id))
            .control_interaction(
                controls::InteractionStyle::new(theme).resting_background(blocked_background),
                controls::InteractionState::default().disabled(access.approval_view.is_none()),
            )
            .child(
                div()
                    .debug_selector(move || icon_selector.clone())
                    .child(svg_icon("icons/refresh.svg", danger, icon_size)),
            )
            .gitcomet_tooltip(theme, tooltip)
            .when_some(
                access.approval_view.clone().filter(|_| !linked),
                move |control, view| {
                    control.on_activate(
                        false,
                        // Linked images route approval through their link menu.
                        controls::ControlActivation::Nested,
                        move |_event, _window, cx| {
                            cx.stop_propagation();
                            view.update(cx, |this, cx| {
                                this.approve_remote_markdown_image(url.clone(), cx);
                            });
                        },
                    )
                },
            )
            .into_any_element();
    }

    let box_selector = format!("{control_id}_blocked_box");
    let icon_selector = format!("{control_id}_blocked_icon");
    base()
        .debug_selector(move || box_selector.clone())
        .id(SharedString::from(format!("{control_id}_blocked")))
        .child(
            div()
                .debug_selector(move || icon_selector.clone())
                .child(svg_icon("icons/generic_close.svg", danger, icon_size)),
        )
        .gitcomet_tooltip(theme, label)
        .into_any_element()
}

/// Slot an inline picture of unknown size holds while it loads. Wide enough for
/// the badges a README opens with, which is what this mostly stands in for.
pub(in crate::view) const MARKDOWN_PREVIEW_INLINE_IMAGE_LOADING_WIDTH_PX: f32 = 90.0;

/// A picture element that keeps per-frame state.
///
/// The id matters: `gpui` only remembers which frame an animated image is
/// showing for elements that have one, so an `img` without an id freezes on the
/// first frame of a GIF.
pub(in crate::view) fn markdown_preview_image_element(
    source: MarkdownPreviewImageSource,
    id: gpui::ElementId,
) -> gpui::Stateful<gpui::Img> {
    gpui::img(gpui::ImageSource::Resource(source.to_resource())).id(id)
}

/// Stand-in for a picture that cannot be drawn.
///
/// It carries the picture's selector too: the slot has to hold its place
/// whether or not the source loaded, and a test asking whether the picture was
/// drawn is really asking whether that slot exists.
pub(in crate::view) fn markdown_preview_inline_image_placeholder(
    label: SharedString,
    source_byte: usize,
    font_size: Pixels,
    color: gpui::Rgba,
) -> AnyElement {
    div()
        .debug_selector(move || format!("markdown_preview_inline_image_{source_byte}"))
        .flex_none()
        .text_size(font_size)
        .text_color(color)
        .child(label)
        .into_any_element()
}

/// Label for a picture that is not on screen: the reason, plus the alt text or
/// the source so the reader can tell which image is missing.
pub(in crate::view) fn markdown_preview_image_label(
    row: &MarkdownPreviewRow,
    reason: &str,
) -> SharedString {
    let described = if row.text.is_empty() {
        row.image
            .as_ref()
            .map(|image| image.source.clone())
            .unwrap_or_default()
    } else {
        row.text.clone()
    };
    markdown_preview_image_reason(reason, &described)
}

/// "reason: what the picture was", or just the reason when nothing describes it.
pub(in crate::view) fn markdown_preview_image_reason(
    reason: &str,
    described: &SharedString,
) -> SharedString {
    if described.is_empty() {
        SharedString::from(reason.to_owned())
    } else {
        SharedString::from(format!("{reason}: {described}"))
    }
}

/// Pictures a frame drew. `gpui` wakes only the first view that asked for an
/// image, so the pane waits on the ones it draws itself.
#[derive(Clone, Default)]
pub(in crate::view) struct MarkdownDrawnPictures(
    std::rc::Rc<std::cell::RefCell<Vec<gpui::Resource>>>,
);

impl MarkdownDrawnPictures {
    pub(in crate::view) fn take(&self) -> Vec<gpui::Resource> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

/// What drawing a document's pictures takes besides the document.
#[derive(Clone, Copy)]
pub(in crate::view) struct MarkdownPictureContext<'a> {
    pub(in crate::view) image_root: Option<&'a MarkdownImageRoot>,
    pub(in crate::view) picture_sizes: &'a MarkdownPreviewPictureSizes,
    pub(in crate::view) remote_image_access: &'a MarkdownRemoteImageAccess,
    /// Where drawn pictures are listed, when something waits on them.
    pub(in crate::view) drawn: Option<&'a MarkdownDrawnPictures>,
}

impl MarkdownPictureContext<'_> {
    /// As [`markdown_preview_resolved_picture`], listing the picture as drawn.
    fn resolved_picture(
        &self,
        source: &str,
        id: gpui::ElementId,
    ) -> Option<gpui::Stateful<gpui::Img>> {
        let source = markdown_preview_image_source(self.image_root, source)?;
        if let Some(drawn) = self.drawn {
            drawn.0.borrow_mut().push(source.to_resource());
        }
        Some(markdown_preview_image_element(source, id))
    }
}

/// Stand-in shown in place of a picture, so the row is never silently blank.
pub(in crate::view) fn markdown_preview_image_placeholder_element(
    label: SharedString,
    font_size: Pixels,
    color: gpui::Rgba,
) -> gpui::Div {
    div()
        .w_full()
        .h_full()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(font_size)
        .text_color(color)
        .child(label)
}

/// Gutter colour the flowing markdown preview marks a wholly added or removed
/// file with, shared with the source preview so the two agree.
pub(in crate::view) fn worktree_markdown_preview_bar_color(
    this: &MainPaneView,
    theme: AppTheme,
) -> Option<gpui::Rgba> {
    worktree_preview_bar_color(this, theme)
}

pub(in crate::view) fn worktree_preview_bar_color(
    this: &MainPaneView,
    theme: AppTheme,
) -> Option<gpui::Rgba> {
    let highlight_deleted_file = this.deleted_file_preview_abs_path().is_some();
    let highlight_new_file = this.untracked_worktree_preview_path().is_some()
        || this.added_file_preview_abs_path().is_some()
        || this.diff_preview_is_new_file;
    if highlight_deleted_file {
        Some(theme.colors.status.danger.foreground)
    } else if highlight_new_file {
        Some(theme.colors.status.success.foreground)
    } else {
        None
    }
}

/// What a markdown row's styling takes from the theme: the syntax colours of
/// its code lines and the colours of its inline styles.
pub(in crate::view) fn markdown_preview_theme_signature(theme: AppTheme) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    crate::view::rows::diff_text::syntax_theme_signature(theme).hash(&mut hasher);
    theme.is_dark.hash(&mut hasher);
    for color in [
        theme.colors.accent.foreground,
        theme.colors.foreground.primary,
        theme.colors.foreground.secondary,
        markdown_preview_code_background(theme),
    ] {
        crate::view::rows::diff_text::hash_rgba_bits(&mut hasher, color);
    }
    hasher.finish()
}

pub(in crate::view) fn markdown_preview_row_styled_text(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
) -> CachedDiffStyledText {
    let signature = markdown_preview_theme_signature(theme);
    row.styled_text_cache.get_or_insert_with(signature, || {
        if matches!(row.kind, MarkdownPreviewRowKind::CodeLine { .. }) {
            return build_cached_diff_styled_text(
                theme,
                row.text.as_ref(),
                &[],
                "",
                row.code_language,
                DiffSyntaxMode::Auto,
                None,
            );
        }

        let highlights = row
            .inline_spans
            .iter()
            .filter_map(|span| {
                let style = markdown_preview_inline_highlight(theme, span.style);
                (style != gpui::HighlightStyle::default())
                    .then_some((span.byte_range.start..span.byte_range.end, style))
            })
            .collect::<Vec<_>>();
        build_cached_diff_styled_text_from_relative_highlights(row.text.as_ref(), &highlights)
    })
}

pub(in crate::view) fn markdown_preview_row_marker(
    row: &MarkdownPreviewRow,
) -> Option<SharedString> {
    if let Some(label) = row.footnote_label.as_ref() {
        return Some(format!("[^{}]:", label.as_ref()).into());
    }

    // A later paragraph or line of an item sits under the item's marker.
    if row.continues_item {
        return None;
    }
    match row.kind {
        MarkdownPreviewRowKind::DetailsSummary => Some("v".into()),
        MarkdownPreviewRowKind::ListItem { number: Some(n) } => Some(format!("{n}.").into()),
        MarkdownPreviewRowKind::ListItem { number: None } => Some("•".into()),
        _ => None,
    }
}

pub(in crate::view) fn markdown_preview_alert_color(
    theme: AppTheme,
    kind: MarkdownAlertKind,
) -> gpui::Rgba {
    match kind {
        MarkdownAlertKind::Note => theme.colors.accent.foreground,
        MarkdownAlertKind::Tip => theme.colors.status.success.foreground,
        MarkdownAlertKind::Important => with_alpha(theme.colors.accent.foreground, 0.85),
        MarkdownAlertKind::Warning => theme.colors.status.warning.foreground,
        MarkdownAlertKind::Caution => theme.colors.status.danger.foreground,
    }
}

pub(in crate::view) fn markdown_preview_inline_highlight(
    theme: AppTheme,
    style: MarkdownInlineStyle,
) -> gpui::HighlightStyle {
    match style {
        MarkdownInlineStyle::Normal => gpui::HighlightStyle::default(),
        MarkdownInlineStyle::Bold => gpui::HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Italic => gpui::HighlightStyle {
            font_style: Some(gpui::FontStyle::Italic),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::BoldItalic => gpui::HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            font_style: Some(gpui::FontStyle::Italic),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Code => gpui::HighlightStyle {
            background_color: Some(markdown_preview_code_background(theme).into_color()),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Strikethrough => gpui::HighlightStyle {
            color: Some(theme.colors.foreground.secondary.into_color()),
            strikethrough: Some(gpui::StrikethroughStyle {
                thickness: px(1.0),
                color: Some(theme.colors.foreground.secondary.into_color()),
            }),
            ..gpui::HighlightStyle::default()
        },
        // Underlined only on hover; see `markdown_preview_hovered_link_styled_text`.
        MarkdownInlineStyle::Link => gpui::HighlightStyle {
            color: Some(theme.colors.accent.foreground.into_color()),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Underline => gpui::HighlightStyle {
            underline: Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.colors.foreground.primary.into_color()),
                wavy: false,
            }),
            ..gpui::HighlightStyle::default()
        },
    }
}

pub(in crate::view) fn markdown_preview_code_background(theme: AppTheme) -> gpui::Rgba {
    if theme.is_dark {
        with_alpha(theme.colors.surface.raised, 0.88)
    } else {
        with_alpha(theme.colors.surface.panel, 0.86)
    }
}

/// The wash a row carries in its own right: a diff change hint, an alert's
/// tint, or the warning band on a line the parser could not interpret.
pub(in crate::view) fn markdown_preview_row_background(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
) -> Option<gpui::Rgba> {
    use MarkdownChangeHint as Hint;
    use MarkdownPreviewRowKind as Kind;

    match row.change_hint {
        Hint::Added => Some(with_alpha(
            theme.colors.status.success.foreground,
            if theme.is_dark { 0.18 } else { 0.12 },
        )),
        Hint::Removed => Some(with_alpha(
            theme.colors.status.danger.foreground,
            if theme.is_dark { 0.16 } else { 0.10 },
        )),
        Hint::Modified => Some(with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.18 } else { 0.10 },
        )),
        Hint::None => {
            if let Some(alert_kind) = row.alert_kind {
                return Some(with_alpha(
                    markdown_preview_alert_color(theme, alert_kind),
                    if theme.is_dark { 0.10 } else { 0.06 },
                ));
            }

            match row.kind {
                Kind::PlainFallback => Some(with_alpha(
                    theme.colors.status.warning.foreground,
                    if theme.is_dark { 0.08 } else { 0.06 },
                )),
                _ => None,
            }
        }
    }
}
