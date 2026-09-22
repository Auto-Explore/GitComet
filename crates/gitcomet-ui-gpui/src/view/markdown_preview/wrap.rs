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
    /// Destination of the link this span sits inside: a web URL or a local
    /// file path (see `classify_markdown_link_destination`).
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
/// the flowing layout expresses the same separation as one interactive gap
/// element between blocks.
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
        // A diff pads one side with spacers to line it up with the other; the
        // run goes on through them when the row after them still belongs.
        if matches!(row.kind, MarkdownPreviewRowKind::Spacer) {
            let next = (*ix..document.rows.len())
                .find(|&next| !matches!(document.rows[next].kind, MarkdownPreviewRowKind::Spacer));
            match next {
                Some(next) if next > start && belongs(next - start, &document.rows[next]) => {
                    *ix = next;
                    continue;
                }
                _ => break,
            }
        }
        if !belongs(*ix - start, row) {
            break;
        }
        *ix += 1;
    }
    start..*ix
}

/// A slice of an aligned diff that both sides draw side by side. No block on
/// either side crosses its edges, so the taller side sets its height and the
/// other is left with blank space, which is what keeps the two lined up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkdownDiffBand {
    /// Aligned row indices, the same on both sides.
    pub(crate) rows: Range<usize>,
    /// Indices into each side's blocks.
    pub(crate) old_blocks: Range<usize>,
    pub(crate) new_blocks: Range<usize>,
}

/// Cut an aligned diff into bands at every row boundary that no block on
/// either side spans. Bands where neither side draws anything are dropped.
pub(crate) fn markdown_diff_bands(
    old_blocks: &[MarkdownBlock],
    new_blocks: &[MarkdownBlock],
    row_count: usize,
) -> Vec<MarkdownDiffBand> {
    let mut inside = vec![false; row_count + 1];
    for block in old_blocks.iter().chain(new_blocks) {
        let range = block.row_range();
        let end = range.end.min(row_count);
        if range.start + 1 < end {
            inside[range.start + 1..end].fill(true);
        }
    }

    let mut bands = Vec::new();
    let (mut old_ix, mut new_ix) = (0usize, 0usize);
    let mut start = 0usize;
    let cuts = (1..=row_count).filter(|&end| end == row_count || !inside[end]);
    for end in cuts {
        let take = |blocks: &[MarkdownBlock], ix: &mut usize| {
            let first = *ix;
            while blocks
                .get(*ix)
                .is_some_and(|block| block.row_range().start < end)
            {
                *ix += 1;
            }
            first..*ix
        };
        let old = take(old_blocks, &mut old_ix);
        let new = take(new_blocks, &mut new_ix);
        if !old.is_empty() || !new.is_empty() {
            bands.push(MarkdownDiffBand {
                rows: start..end,
                old_blocks: old,
                new_blocks: new,
            });
        }
        start = end;
    }
    bands
}

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
