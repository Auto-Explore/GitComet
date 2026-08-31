use super::CachedDiffStyledText;
use gpui::SharedString;
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock};

/// Maximum source size (bytes) for a single markdown preview document.
pub(super) const MAX_PREVIEW_SOURCE_BYTES: usize = 1_024 * 1_024; // 1 MiB

/// Maximum combined source size (bytes) for a two-sided diff preview.
pub(super) const MAX_DIFF_PREVIEW_SOURCE_BYTES: usize = 2 * 1_024 * 1_024; // 2 MiB

/// Maximum number of preview rows per document.
pub(super) const MAX_PREVIEW_ROWS: usize = 20_000;

/// Maximum number of rows the single-document preview renders.
///
/// That preview lays its whole document out at once so text can wrap and
/// pictures can sit inline, which means every row costs layout on every frame —
/// unlike the diff preview, which paints a virtualized window of a fixed row
/// grid and is bounded by [`MAX_PREVIEW_ROWS`] instead. A document past this
/// budget falls back to source mode rather than making the pane crawl.
pub(super) const MAX_FLOWING_PREVIEW_ROWS: usize = 4_000;

/// Maximum number of inline spans per row before degrading to plain text.
const MAX_INLINE_SPANS_PER_ROW: usize = 512;

// ── Core types ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownPreviewDocument {
    pub(super) rows: Vec<MarkdownPreviewRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownPreviewDiff {
    pub(super) old: MarkdownPreviewDocument,
    pub(super) new: MarkdownPreviewDocument,
    pub(super) inline: MarkdownPreviewDocument,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownPreviewRow {
    pub(super) kind: MarkdownPreviewRowKind,
    pub(super) text: SharedString,
    pub(super) inline_spans: Arc<Vec<MarkdownInlineSpan>>,
    pub(super) code_language: Option<crate::view::rows::DiffSyntaxLanguage>,
    pub(super) code_block_horizontal_scroll_hint: bool,
    pub(super) source_line_range: Range<usize>,
    pub(super) change_hint: MarkdownChangeHint,
    pub(super) indent_level: u8,
    pub(super) blockquote_level: u8,
    pub(super) footnote_label: Option<SharedString>,
    pub(super) alert_kind: Option<MarkdownAlertKind>,
    pub(super) starts_alert: bool,
    /// The image an [`MarkdownPreviewRowKind::Image`] row paints.
    pub(super) image: Option<Arc<MarkdownImage>>,
    /// Pictures that share this row's line with its text, in document order.
    pub(super) inline_images: Arc<[MarkdownInlineImage]>,
    pub(super) styled_text_cache: MarkdownPreviewRowStyledTextCache,
    pub(super) measured_width_px: MarkdownPreviewRowWidthCache,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MarkdownPreviewRowKind {
    Heading {
        level: u8,
    },
    Paragraph,
    DetailsSummary,
    ListItem {
        number: Option<u64>,
    },
    BlockquoteLine,
    CodeLine {
        is_first: bool,
        is_last: bool,
    },
    ThematicBreak,
    TableRow {
        is_header: bool,
    },
    /// One horizontal band of an image block.
    ///
    /// The preview paints into a uniform (fixed row height) list, so an image
    /// occupies `slice_count` consecutive rows and each row shows the band of
    /// the picture at its own `slice_ix`. Slicing it this way — rather than
    /// letting one tall row overflow its neighbours — keeps the image correct
    /// when it is scrolled half out of view, because every row draws itself.
    Image {
        slice_ix: u8,
        slice_count: u8,
    },
    PlainFallback,
    Spacer,
}

impl MarkdownPreviewRowKind {
    fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. })
    }
}

impl MarkdownPreviewRow {
    /// Whether this row only continues a picture an earlier row already
    /// carries, and so is not a line of the document in its own right.
    pub(super) fn continues_a_picture(&self) -> bool {
        matches!(self.kind, MarkdownPreviewRowKind::Image { slice_ix, .. } if slice_ix > 0)
    }
}

/// Rows an image block occupies when the document says nothing about the
/// picture's size.
pub(super) const MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS: u8 = 8;

/// Combine the inline style stack into a single effective style.
fn resolve_style_stack(stack: &[MarkdownInlineStyle]) -> MarkdownInlineStyle {
    let mut has_bold = false;
    let mut has_italic = false;
    let mut has_strikethrough = false;
    let mut has_link = false;
    let mut has_code = false;
    let mut has_underline = false;

    for &s in stack {
        match s {
            MarkdownInlineStyle::Bold => has_bold = true,
            MarkdownInlineStyle::Italic => has_italic = true,
            MarkdownInlineStyle::Strikethrough => has_strikethrough = true,
            MarkdownInlineStyle::Link => has_link = true,
            MarkdownInlineStyle::Code => has_code = true,
            MarkdownInlineStyle::Underline => has_underline = true,
            _ => {}
        }
    }

    if has_code {
        MarkdownInlineStyle::Code
    } else if has_bold && has_italic {
        MarkdownInlineStyle::BoldItalic
    } else if has_bold {
        MarkdownInlineStyle::Bold
    } else if has_italic {
        MarkdownInlineStyle::Italic
    } else if has_strikethrough {
        MarkdownInlineStyle::Strikethrough
    } else if has_link {
        MarkdownInlineStyle::Link
    } else if has_underline {
        MarkdownInlineStyle::Underline
    } else {
        MarkdownInlineStyle::Normal
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

mod document;
mod flatten;
mod html;
mod inline;
mod tables;
mod wrap;

pub(super) use document::*;
pub(super) use flatten::*;
pub(super) use html::*;
pub(super) use inline::*;
pub(super) use tables::*;
pub(super) use wrap::*;

#[cfg(test)]
mod tests;
