use super::*;

/// parse time. Must track `MARKDOWN_PREVIEW_ROW_HEIGHT_PX`; both scale with the
/// UI together, so the row count is scale-independent.
const MARKDOWN_PREVIEW_IMAGE_ROW_HEIGHT_PX: u32 = 28;

/// A picture that shares a line with the text around it.
///
/// Markdown draws no distinction between a picture on a line of its own and one
/// written mid-sentence — badges, shields, and a logo beside a heading are all
/// ordinary inline content. A row therefore carries its pictures alongside its
/// text instead of displacing it, and only a picture that is alone on its line
/// becomes a block of its own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkdownInlineImage {
    /// Byte offset in the row's text where the picture belongs.
    pub(crate) byte_offset: usize,
    /// Byte offset in the *source document* where the picture was written.
    ///
    /// Unique across the document, which makes it the element id a renderer
    /// can key on without allocating one, and the only thing left to tie a
    /// picture back to the line it came from once its row is built.
    pub(crate) source_byte: usize,
    pub(crate) image: Arc<MarkdownImage>,
    /// Description shown when the picture cannot be drawn.
    pub(crate) alt: SharedString,
    /// The link the picture stands in for, when it is wrapped in one.
    pub(crate) link_url: Option<SharedString>,
}

/// An image a preview row draws, with whatever size the document declared.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkdownImage {
    /// The source exactly as written in the document.
    pub(crate) source: SharedString,
    pub(crate) width_px: Option<u32>,
    pub(crate) height_px: Option<u32>,
}

impl MarkdownImage {
    /// Rows this image's block occupies.
    ///
    /// A declared height is authoritative. With only a width — the common
    /// `<img width="26">` used for an inline logo — the picture is assumed no
    /// taller than it is wide, which keeps small images from reserving a
    /// screenful of blank rows. `object_fit: contain` letterboxes anything
    /// that turns out to be taller.
    pub(crate) fn block_rows(&self) -> u8 {
        // A declared size of zero says nothing about how tall the picture is,
        // so it is treated as undeclared rather than collapsing the block to a
        // single row — and each dimension is judged on its own, so `height="0"`
        // falls through to a usable width instead of discarding it.
        let Some(declared) = self
            .height_px
            .filter(|declared| *declared > 0)
            .or(self.width_px.filter(|declared| *declared > 0))
        else {
            return MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS;
        };
        let rows = declared
            .div_ceil(MARKDOWN_PREVIEW_IMAGE_ROW_HEIGHT_PX)
            .max(1);
        u8::try_from(rows)
            .unwrap_or(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS)
            .min(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkdownInlineSpan {
    pub(crate) byte_range: Range<usize>,
    pub(crate) style: MarkdownInlineStyle,
    /// Destination of the link this span sits inside.
    ///
    /// Carried on the span rather than in a parallel list so it survives the
    /// byte remapping that whitespace normalisation and table alignment apply,
    /// and independently of `style` because a bold or code span inside a link
    /// resolves to that style while still being clickable.
    pub(crate) link_url: Option<SharedString>,
}

impl MarkdownInlineSpan {
    pub(crate) fn restyled(&self, byte_range: Range<usize>) -> Self {
        Self {
            byte_range,
            style: self.style,
            link_url: self.link_url.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MarkdownInlineStyle {
    Normal,
    Bold,
    Italic,
    BoldItalic,
    Code,
    Strikethrough,
    Link,
    Underline,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MarkdownChangeHint {
    #[default]
    None,
    Added,
    Removed,
    Modified,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MarkdownAlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkdownBlockQuoteContext {
    pub(crate) alert_kind: Option<MarkdownAlertKind>,
    pub(crate) emitted_row: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkdownFootnoteContext {
    pub(crate) label: SharedString,
    pub(crate) emitted_label: bool,
}

/// Parse state every row flush consults.
///
/// Both parts are answers to "what does the row being closed inherit?": the
/// blockquote stack decides its alert, and `pending_images` holds the pictures
/// read since the last flush, which belong to the line they were written on.
#[derive(Default)]
pub(crate) struct MarkdownRowContext {
    pub(crate) blockquote_stack: Vec<MarkdownBlockQuoteContext>,
    pub(crate) pending_images: Vec<MarkdownInlineImage>,
}

impl MarkdownRowContext {
    /// True when a row has to be emitted even though its text is empty.
    ///
    /// Pictures only reach the document through the row that closes over them,
    /// so a construct that would otherwise skip an empty row — a list item
    /// holding nothing but a badge — has to emit one anyway or the picture is
    /// carried onto an unrelated row later, or dropped at the end of the parse.
    pub(crate) fn has_pending_images(&self) -> bool {
        !self.pending_images.is_empty()
    }
}

pub(crate) struct MarkdownPreviewRowInput<'a> {
    pub(crate) kind: MarkdownPreviewRowKind,
    pub(crate) text: &'a str,
    pub(crate) inline_spans: &'a [MarkdownInlineSpan],
    pub(crate) code_language: Option<crate::view::rows::DiffSyntaxLanguage>,
    pub(crate) code_block_horizontal_scroll_hint: bool,
    pub(crate) source_line_range: Range<usize>,
    pub(crate) indent_level: u8,
    pub(crate) blockquote_level: u8,
    pub(crate) image: Option<Arc<MarkdownImage>>,
    pub(crate) inline_images: Arc<[MarkdownInlineImage]>,
}

impl<'a> MarkdownPreviewRowInput<'a> {
    pub(crate) fn plain(
        kind: MarkdownPreviewRowKind,
        text: &'a str,
        inline_spans: &'a [MarkdownInlineSpan],
        source_line_range: Range<usize>,
        indent_level: u8,
        blockquote_level: u8,
    ) -> Self {
        Self {
            kind,
            text,
            inline_spans,
            code_language: None,
            code_block_horizontal_scroll_hint: false,
            source_line_range,
            indent_level,
            blockquote_level,
            image: None,
            inline_images: Arc::from(Vec::new()),
        }
    }

    pub(crate) fn code(
        kind: MarkdownPreviewRowKind,
        text: &'a str,
        source_line_range: Range<usize>,
        code_language: Option<crate::view::rows::DiffSyntaxLanguage>,
        code_block_horizontal_scroll_hint: bool,
        indent_level: u8,
        blockquote_level: u8,
    ) -> Self {
        Self {
            kind,
            text,
            inline_spans: &[],
            code_language,
            code_block_horizontal_scroll_hint,
            source_line_range,
            indent_level,
            blockquote_level,
            image: None,
            inline_images: Arc::from(Vec::new()),
        }
    }

    pub(crate) fn image(
        slice_ix: u8,
        slice_count: u8,
        alt: &'a str,
        image: Arc<MarkdownImage>,
        source_line_range: Range<usize>,
        indent_level: u8,
        blockquote_level: u8,
    ) -> Self {
        Self {
            kind: MarkdownPreviewRowKind::Image {
                slice_ix,
                slice_count,
            },
            // The alt text stays the row text so selection and copy still see
            // something meaningful, and so a picture that cannot be loaded can
            // fall back to describing itself.
            text: alt,
            inline_spans: &[],
            code_language: None,
            code_block_horizontal_scroll_hint: false,
            source_line_range,
            indent_level,
            blockquote_level,
            image: Some(image),
            inline_images: Arc::from(Vec::new()),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct MarkdownPreviewRowDecoration {
    pub(crate) footnote_label: Option<SharedString>,
    pub(crate) alert_kind: Option<MarkdownAlertKind>,
    pub(crate) starts_alert: bool,
}

#[derive(Debug, Default)]
pub(crate) struct MarkdownPreviewRowWidthCache(Mutex<Option<(u64, u32)>>);

impl Clone for MarkdownPreviewRowWidthCache {
    fn clone(&self) -> Self {
        let cached = match self.0.lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        };

        Self(Mutex::new(cached))
    }
}

impl MarkdownPreviewRowWidthCache {
    pub(crate) fn get_or_init(&self, key: u64, compute: impl FnOnce() -> u32) -> u32 {
        let mut cached = match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some((cached_key, value)) = *cached
            && cached_key == key
        {
            return value;
        }

        let value = compute();
        *cached = Some((key, value));
        value
    }
}

impl PartialEq for MarkdownPreviewRowWidthCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for MarkdownPreviewRowWidthCache {}

#[derive(Clone, Debug, Default)]
pub(crate) struct MarkdownPreviewRowStyledTextCache {
    pub(crate) dark: OnceLock<CachedDiffStyledText>,
    pub(crate) light: OnceLock<CachedDiffStyledText>,
}

impl MarkdownPreviewRowStyledTextCache {
    pub(crate) fn get_or_init(
        &self,
        is_dark: bool,
        compute: impl FnOnce() -> CachedDiffStyledText,
    ) -> &CachedDiffStyledText {
        if is_dark {
            self.dark.get_or_init(compute)
        } else {
            self.light.get_or_init(compute)
        }
    }
}

impl PartialEq for MarkdownPreviewRowStyledTextCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for MarkdownPreviewRowStyledTextCache {}

// ── Flowing document blocks ─────────────────────────────────────────────

/// A run of rows that renders as one element in the flowing preview.
///
/// The row model is shaped for the diff preview, which paints into a uniform
/// (fixed row height) list and therefore needs one row per painted line. The
/// single-document preview lays out naturally instead, so consecutive rows
/// belonging to the same construct — the lines of a code block, the bands of
/// an image, the rows of a table — are grouped back into the block they came
/// from. Both previews stay on one parsed model this way.
/// Blocks address rows by index rather than by reference: selection, copy, and
/// hit testing are all keyed by row index, so the flowing renderer hands the
/// same indices to the same machinery the row preview used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MarkdownBlock {
    Heading {
        level: u8,
        row_ix: usize,
    },
    Paragraph(usize),
    /// The bands of one image; only the first carries the source.
    Image(Range<usize>),
    ThematicBreak(usize),
    List(Range<usize>),
    Blockquote(Range<usize>),
    Code(Range<usize>),
    Table(Range<usize>),
}

impl MarkdownBlock {
    /// Rows this block paints, in document order.
    pub(crate) fn row_range(&self) -> Range<usize> {
        match self {
            Self::Heading { row_ix, .. }
            | Self::Paragraph(row_ix)
            | Self::ThematicBreak(row_ix) => *row_ix..*row_ix + 1,
            Self::Image(range)
            | Self::List(range)
            | Self::Blockquote(range)
            | Self::Code(range)
            | Self::Table(range) => range.clone(),
        }
    }
}

/// Group a document's rows into the blocks the flowing preview renders.
///
/// Spacer rows are dropped: they exist to open a gap in a fixed row grid, and
/// the flowing layout expresses the same gap as a margin.
pub(crate) fn markdown_document_blocks(document: &MarkdownPreviewDocument) -> Vec<MarkdownBlock> {
    let mut blocks: Vec<MarkdownBlock> = Vec::new();
    let mut ix = 0usize;

    while ix < document.rows.len() {
        let row = &document.rows[ix];
        match row.kind {
            MarkdownPreviewRowKind::Spacer => ix += 1,
            MarkdownPreviewRowKind::ThematicBreak => {
                blocks.push(MarkdownBlock::ThematicBreak(ix));
                ix += 1;
            }
            MarkdownPreviewRowKind::Heading { level } => {
                blocks.push(MarkdownBlock::Heading { level, row_ix: ix });
                ix += 1;
            }
            MarkdownPreviewRowKind::Image { .. } => {
                // Every band of one image repeats the same source; the block
                // needs it once.
                let source = row.image.as_ref().map(|image| image.source.clone());
                let start = ix;
                ix += 1;
                while ix < document.rows.len()
                    && document.rows[ix].kind.is_image()
                    && document.rows[ix]
                        .image
                        .as_ref()
                        .map(|image| image.source.clone())
                        == source
                    && !matches!(
                        document.rows[ix].kind,
                        MarkdownPreviewRowKind::Image { slice_ix: 0, .. }
                    )
                {
                    ix += 1;
                }
                blocks.push(MarkdownBlock::Image(start..ix));
            }
            MarkdownPreviewRowKind::ListItem { .. } => {
                blocks.push(MarkdownBlock::List(take_run(
                    document,
                    &mut ix,
                    |_, row| matches!(row.kind, MarkdownPreviewRowKind::ListItem { .. }),
                )));
            }
            MarkdownPreviewRowKind::BlockquoteLine => {
                // Two alerts that touch are two blocks: each carries its own
                // bar and badge, and folding them together would label the
                // second one with the first one's kind.
                blocks.push(MarkdownBlock::Blockquote(take_run(
                    document,
                    &mut ix,
                    |offset, row| {
                        matches!(row.kind, MarkdownPreviewRowKind::BlockquoteLine)
                            && (offset == 0 || !row.starts_alert)
                    },
                )));
            }
            MarkdownPreviewRowKind::CodeLine { .. } => {
                blocks.push(MarkdownBlock::Code(take_run(
                    document,
                    &mut ix,
                    |_, row| matches!(row.kind, MarkdownPreviewRowKind::CodeLine { .. }),
                )));
            }
            MarkdownPreviewRowKind::TableRow { .. } => {
                // A header row opens a table, so it ends the one before it for
                // the same reason an alert's first row ends the quote above.
                blocks.push(MarkdownBlock::Table(take_run(
                    document,
                    &mut ix,
                    |offset, row| match row.kind {
                        MarkdownPreviewRowKind::TableRow { is_header } => offset == 0 || !is_header,
                        _ => false,
                    },
                )));
            }
            MarkdownPreviewRowKind::Paragraph
            | MarkdownPreviewRowKind::DetailsSummary
            | MarkdownPreviewRowKind::PlainFallback => {
                blocks.push(MarkdownBlock::Paragraph(ix));
                ix += 1;
            }
        }
    }

    blocks
}

/// Consume the run of consecutive rows `belongs` accepts, which sees each row
/// together with its offset from the start of the run.
pub(crate) fn take_run(
    document: &MarkdownPreviewDocument,
    ix: &mut usize,
    belongs: impl Fn(usize, &MarkdownPreviewRow) -> bool,
) -> Range<usize> {
    let start = *ix;
    while let Some(row) = document.rows.get(*ix) {
        if !belongs(*ix - start, row) {
            break;
        }
        *ix += 1;
    }
    start..*ix
}

// ── Word wrap ───────────────────────────────────────────────────────────

/// One rendered row of a wrapped preview document.
///
/// Preview rows are painted into a uniform (fixed row height) list, so word
/// wrap works the same way it does in the text diff: a source row that does
/// not fit is split into several visual rows, each carrying the byte range of
/// `MarkdownPreviewRow::text` it paints. `wrap_ix > 0` marks a continuation,
/// which drops the list marker and alert badge so the text keeps its indent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkdownPreviewVisualRow {
    pub(crate) row_ix: usize,
    pub(crate) wrap_ix: u32,
    pub(crate) byte_range: Range<usize>,
}

impl MarkdownPreviewVisualRow {
    pub(crate) fn is_continuation(&self) -> bool {
        self.wrap_ix > 0
    }

    /// The portion of `row.text` this visual row paints.
    ///
    /// Hit testing, selection, and copy index rows by visual position, so they
    /// need the slice the row painted rather than the whole source row.
    pub(crate) fn text_slice(&self, row: &MarkdownPreviewRow) -> SharedString {
        if self.byte_range == (0..row.text.len()) {
            return row.text.clone();
        }
        row.text
            .get(self.byte_range.clone())
            .map(SharedString::new)
            .unwrap_or_default()
    }
}

/// Source-row to visual-row mapping for one wrapped preview document.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MarkdownPreviewWrapPlan {
    pub(crate) rows: Vec<MarkdownPreviewVisualRow>,
}

impl MarkdownPreviewWrapPlan {
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn get(&self, visual_ix: usize) -> Option<&MarkdownPreviewVisualRow> {
        self.rows.get(visual_ix)
    }

    /// First visual row painted for `row_ix`, for scroll and autoscroll targets.
    pub(crate) fn visual_ix_for_row(&self, row_ix: usize) -> usize {
        self.rows.partition_point(|row| row.row_ix < row_ix)
    }
}

/// Build the visual-row mapping for `document`.
///
/// `wrap_row` returns the byte ranges a row's painted text splits into at the
/// current width; an empty result (or a row that fits) yields a single visual
/// row covering the whole text, so every source row keeps at least one row.
///
/// Returns `None` when the wrapped document would exceed
/// `MAX_PREVIEW_WRAPPED_ROWS`, which the caller must treat as "do not wrap".
/// Truncating the plan instead would drop the tail of the document out of the
/// list with no way to scroll to it.
pub(crate) fn build_markdown_preview_wrap_plan(
    document: &MarkdownPreviewDocument,
    mut wrap_row: impl FnMut(&MarkdownPreviewRow) -> Vec<Range<usize>>,
) -> Option<MarkdownPreviewWrapPlan> {
    let mut rows = Vec::with_capacity(document.rows.len());
    for (row_ix, row) in document.rows.iter().enumerate() {
        push_wrapped_visual_rows(&mut rows, row_ix, wrap_row(row), row, 0)?;
    }
    rows.shrink_to_fit();
    Some(MarkdownPreviewWrapPlan { rows })
}

/// Build the visual-row mappings for the two sides of a split diff preview.
///
/// `align_markdown_diff_rows` pads the two documents so source row `ix` is the
/// same diff row on both sides; wrapping each side independently would break
/// that, because a long paragraph on the left would push every later left row
/// down relative to its right-hand counterpart while the synced scroll keeps
/// the two lists at the same offset. Both sides therefore get the same number
/// of visual rows per source row, the shorter side padded with empty
/// continuations.
pub(crate) fn build_markdown_preview_split_wrap_plans(
    old_doc: &MarkdownPreviewDocument,
    new_doc: &MarkdownPreviewDocument,
    mut wrap_row: impl FnMut(&MarkdownPreviewRow) -> Vec<Range<usize>>,
) -> Option<(MarkdownPreviewWrapPlan, MarkdownPreviewWrapPlan)> {
    // `align_markdown_diff_rows` pushes to both sides in lockstep, so the two
    // documents are the same length by the time they reach a split preview.
    debug_assert_eq!(old_doc.rows.len(), new_doc.rows.len());

    let row_count = old_doc.rows.len().min(new_doc.rows.len());
    let mut old_rows = Vec::with_capacity(row_count);
    let mut new_rows = Vec::with_capacity(row_count);

    for (row_ix, (old_row, new_row)) in old_doc.rows.iter().zip(new_doc.rows.iter()).enumerate() {
        let old_ranges = wrap_row(old_row);
        let new_ranges = wrap_row(new_row);
        let visual_count = old_ranges.len().max(new_ranges.len()).max(1);

        push_wrapped_visual_rows(&mut old_rows, row_ix, old_ranges, old_row, visual_count)?;
        push_wrapped_visual_rows(&mut new_rows, row_ix, new_ranges, new_row, visual_count)?;
    }

    old_rows.shrink_to_fit();
    new_rows.shrink_to_fit();
    Some((
        MarkdownPreviewWrapPlan { rows: old_rows },
        MarkdownPreviewWrapPlan { rows: new_rows },
    ))
}

/// Append the visual rows for one source row, padding up to `min_visual_rows`
/// with empty continuations so a split counterpart stays row-aligned.
pub(crate) fn push_wrapped_visual_rows(
    out: &mut Vec<MarkdownPreviewVisualRow>,
    row_ix: usize,
    ranges: Vec<Range<usize>>,
    row: &MarkdownPreviewRow,
    min_visual_rows: usize,
) -> Option<()> {
    let text_len = row.text.len();
    let push =
        |out: &mut Vec<MarkdownPreviewVisualRow>, wrap_ix: usize, byte_range: Range<usize>| {
            out.push(MarkdownPreviewVisualRow {
                row_ix,
                wrap_ix: u32::try_from(wrap_ix).unwrap_or(u32::MAX),
                byte_range,
            });
            (out.len() <= MAX_PREVIEW_WRAPPED_ROWS).then_some(())
        };

    // A row that fits keeps one visual row covering all of its text; building
    // a one-element Vec for that common case would allocate per source row.
    let mut wrap_ix = 0usize;
    if ranges.len() < 2 {
        push(out, wrap_ix, 0..text_len)?;
        wrap_ix += 1;
    } else {
        for byte_range in ranges {
            push(out, wrap_ix, byte_range)?;
            wrap_ix += 1;
        }
    }
    while wrap_ix < min_visual_rows {
        push(out, wrap_ix, text_len..text_len)?;
        wrap_ix += 1;
    }
    Some(())
}

/// Upper bound on visual rows in a wrapped document. A pathological window
/// width (a few pixels wide) would otherwise wrap every character onto its own
/// row and blow up the uniform list.
const MAX_PREVIEW_WRAPPED_ROWS: usize = MAX_PREVIEW_ROWS * 8;

// ── Error messages ──────────────────────────────────────────────────────

/// Return a user-facing reason why a single-document markdown preview is
/// unavailable for a source of `source_len` bytes.
pub(crate) fn single_preview_unavailable_reason(source_len: usize) -> &'static str {
    if source_len > MAX_PREVIEW_SOURCE_BYTES {
        "Markdown preview unavailable: file exceeds the 1 MiB preview limit."
    } else {
        "Markdown preview unavailable: rendered row limit exceeded."
    }
}

/// Why a single-document preview could not be produced.
///
/// The two cases read the same to a user — no preview — but they are not the
/// same problem: one document cannot be parsed within the row cap at all, the
/// other parses fine and is only too big for a renderer that lays every row
/// out at once. Only the second has a good answer, which is to show the source.
pub(crate) const TOO_MANY_ROWS_TO_RENDER_MESSAGE: &str =
    "Markdown preview unavailable: document is too large to render; showing source.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MarkdownPreviewRefusal {
    /// Unreadable, or past the source-size or parsed-row cap.
    Unavailable(String),
    /// Parsed, but past what the flowing renderer will lay out in a frame.
    TooManyRowsToRender,
}

impl From<String> for MarkdownPreviewRefusal {
    fn from(reason: String) -> Self {
        Self::Unavailable(reason)
    }
}

impl MarkdownPreviewRefusal {
    pub(crate) fn into_message(self) -> String {
        match self {
            Self::Unavailable(reason) => reason,
            Self::TooManyRowsToRender => TOO_MANY_ROWS_TO_RENDER_MESSAGE.to_owned(),
        }
    }

    /// True when the reader is better served by the source than by an error.
    pub(crate) fn prefers_source(&self) -> bool {
        matches!(self, Self::TooManyRowsToRender)
    }
}

/// Return a user-facing reason why a two-sided diff markdown preview is
/// unavailable for sources of `combined_len` bytes.
pub(crate) fn diff_preview_unavailable_reason(combined_len: usize) -> &'static str {
    if combined_len > MAX_DIFF_PREVIEW_SOURCE_BYTES {
        "Markdown preview unavailable: diff exceeds the 2 MiB preview limit."
    } else {
        "Markdown preview unavailable: rendered row limit exceeded."
    }
}
