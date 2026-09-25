use super::CachedDiffStyledText;
use gpui::SharedString;
use std::ops::Range;
use std::sync::{Arc, Mutex};

/// Maximum source size (bytes) for a single markdown preview document.
pub(super) const MAX_PREVIEW_SOURCE_BYTES: usize = 1_024 * 1_024; // 1 MiB

/// Maximum combined source size (bytes) for a two-sided diff preview.
pub(super) const MAX_DIFF_PREVIEW_SOURCE_BYTES: usize = 2 * 1_024 * 1_024; // 2 MiB

/// Maximum number of preview rows per document.
pub(super) const MAX_PREVIEW_ROWS: usize = 20_000;

/// Maximum number of inline spans per row before degrading to plain text.
const MAX_INLINE_SPANS_PER_ROW: usize = 512;

// ── Core types ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownPreviewDocument {
    pub(super) rows: Vec<MarkdownPreviewRow>,
    pub(super) anchors: MarkdownAnchorIndexCell,
}

impl MarkdownPreviewDocument {
    pub(super) fn new(rows: Vec<MarkdownPreviewRow>) -> Self {
        Self {
            rows,
            anchors: MarkdownAnchorIndexCell::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownPreviewDiff {
    /// Both sides padded with spacer rows to the same aligned row indices.
    pub(super) old: MarkdownPreviewDocument,
    pub(super) new: MarkdownPreviewDocument,
    pub(super) inline: MarkdownPreviewDocument,
    /// Which rows of `inline` show the old version. A modified paragraph is
    /// drawn twice, both copies marked modified, so the change hint alone
    /// cannot tell the old copy — whose links open the file before the change —
    /// from the new one.
    pub(super) inline_old: Vec<bool>,
    /// How the flowing renderer groups each document's rows.
    pub(super) old_blocks: Vec<MarkdownBlock>,
    pub(super) new_blocks: Vec<MarkdownBlock>,
    pub(super) inline_blocks: Vec<MarkdownBlock>,
    /// Side-by-side slices of `old`/`new` for the split view.
    pub(super) bands: Vec<MarkdownDiffBand>,
    /// What each side's file held, which says why a side shows no block.
    pub(super) old_source: MarkdownDiffSideSource,
    pub(super) new_source: MarkdownDiffSideSource,
}

/// What one side of a diff held.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum MarkdownDiffSideSource {
    /// No file on this side: it was added, or deleted.
    Missing,
    #[default]
    Empty,
    /// Text, though perhaps none that renders.
    Text,
}

impl MarkdownDiffSideSource {
    pub(super) fn of(source: Option<&str>) -> Self {
        match source {
            None => Self::Missing,
            Some("") => Self::Empty,
            Some(_) => Self::Text,
        }
    }

    fn notice(self, missing: &'static str) -> &'static str {
        match self {
            Self::Missing => missing,
            Self::Empty => "Empty file.",
            Self::Text => "Nothing to render.",
        }
    }
}

impl MarkdownPreviewDiff {
    pub(super) fn new(
        old: MarkdownPreviewDocument,
        new: MarkdownPreviewDocument,
        inline: MarkdownPreviewDocument,
    ) -> Self {
        let old_blocks = markdown_document_blocks(&old);
        let new_blocks = markdown_document_blocks(&new);
        let inline_blocks = markdown_document_blocks(&inline);
        let bands =
            markdown_diff_bands(&old_blocks, &new_blocks, old.rows.len().max(new.rows.len()));
        Self {
            inline_old: vec![false; inline.rows.len()],
            old,
            new,
            inline,
            old_blocks,
            new_blocks,
            inline_blocks,
            bands,
            old_source: MarkdownDiffSideSource::default(),
            new_source: MarkdownDiffSideSource::default(),
        }
    }

    /// The notice standing in for the old side when it has no block.
    pub(super) fn old_empty_notice(&self) -> &'static str {
        self.old_source.notice("File added.")
    }

    /// The notice standing in for the new side when it has no block.
    pub(super) fn new_empty_notice(&self) -> &'static str {
        self.new_source.notice("File deleted.")
    }

    /// The notice standing in for a diff with no block on either side.
    pub(super) fn empty_notice(&self) -> &'static str {
        if [self.old_source, self.new_source].contains(&MarkdownDiffSideSource::Text) {
            "Nothing to render."
        } else {
            "Empty file."
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownPreviewRow {
    pub(super) kind: MarkdownPreviewRowKind,
    pub(super) text: SharedString,
    pub(super) inline_spans: Arc<Vec<MarkdownInlineSpan>>,
    pub(super) code_language: Option<crate::view::rows::DiffSyntaxLanguage>,
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
    /// Cell layout of a [`MarkdownPreviewRowKind::TableRow`]; `None` otherwise.
    pub(super) table: Option<MarkdownTableRow>,
    /// The `[ ]`/`[x]` a task-list item opens with; `None` otherwise.
    pub(super) task: Option<MarkdownTaskMarker>,
    /// A later row of a list item (another paragraph, a line after a hard
    /// break): it keeps the item's indent but draws no second bullet.
    pub(super) continues_item: bool,
}

impl Default for MarkdownPreviewRow {
    /// An empty spacer row; fixtures override what they need.
    fn default() -> Self {
        Self {
            kind: MarkdownPreviewRowKind::Spacer,
            text: SharedString::default(),
            inline_spans: Arc::default(),
            code_language: None,
            source_line_range: 0..0,
            change_hint: MarkdownChangeHint::None,
            indent_level: 0,
            blockquote_level: 0,
            footnote_label: None,
            alert_kind: None,
            starts_alert: false,
            image: None,
            inline_images: Arc::from(Vec::new()),
            styled_text_cache: MarkdownPreviewRowStyledTextCache::default(),
            table: None,
            task: None,
            continues_item: false,
        }
    }
}

/// A task-list checkbox and where its `[` sits in the parsed source.
///
/// Kept as a line and a byte column rather than a file offset: a diff's new
/// side is parsed from git's normalized text, whose line endings can differ
/// from the working-tree file's (CRLF → LF), while its lines and columns do
/// not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MarkdownTaskMarker {
    pub(super) checked: bool,
    pub(super) line: usize,
    pub(super) column: usize,
    /// The marker's line as parsed, so a line that moved there since — another
    /// item, its `[ ]` in the same column — is not mistaken for it.
    pub(super) line_hash: u64,
}

impl MarkdownTaskMarker {
    /// Where the marker's `[` falls in `text`, if that line is still the one
    /// the marker was parsed from.
    pub(super) fn byte_offset(&self, text: &[u8]) -> Option<usize> {
        let line_start = if self.line == 0 {
            0
        } else {
            memchr::memchr_iter(b'\n', text).nth(self.line - 1)? + 1
        };
        let line_end =
            memchr::memchr(b'\n', &text[line_start..]).map_or(text.len(), |end| line_start + end);
        (task_line_hash(&text[line_start..line_end], self.column) == self.line_hash)
            .then_some(line_start + self.column)
    }
}

/// A task line's identity: its text without the line ending, and with the
/// checkbox's own state blanked, since that is what a toggle changes.
pub(super) fn task_line_hash(line: &[u8], column: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let mut hasher = rustc_hash::FxHasher::default();
    for (ix, byte) in line.iter().enumerate() {
        let byte = if ix == column + 1 { b' ' } else { *byte };
        byte.hash(&mut hasher);
    }
    line.len().hash(&mut hasher);
    hasher.finish()
}

/// One table row's cells: byte ranges in the row text, whose cells are joined
/// by `\t` so a copied selection reads as tab-separated values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownTableRow {
    /// One range per column; a row shorter than the table gets empty cells.
    pub(super) cells: Arc<[Range<usize>]>,
    pub(super) table: Arc<MarkdownTableInfo>,
}

/// What the rows of one table share.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct MarkdownTableInfo {
    /// One per column, from the `:---:` delimiter row.
    pub(super) alignments: Vec<MarkdownTableAlign>,
    /// Widest cell per column in chars, for the monospace row-list rendering.
    pub(super) column_widths: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum MarkdownTableAlign {
    #[default]
    None,
    Left,
    Center,
    Right,
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
    /// A picture alone on its line: a block of its own.
    Image,
    PlainFallback,
    Spacer,
}

impl MarkdownPreviewRow {
    /// Whether this is a spacer a diff inserted to line one side up with the
    /// other. Unlike the gap under a heading, it stands for no source line.
    pub(super) fn is_alignment_padding(&self) -> bool {
        matches!(self.kind, MarkdownPreviewRowKind::Spacer) && self.source_line_range.is_empty()
    }
}

/// Height, in design pixels, a picture reserves when the document says
/// nothing about its size.
pub(super) const MARKDOWN_PREVIEW_IMAGE_DEFAULT_HEIGHT_PX: u32 = 224;

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

mod anchors;
mod document;
mod flatten;
mod html;
mod inline;
mod tables;
mod wrap;

pub(super) use anchors::*;
pub(super) use document::*;
pub(super) use flatten::*;
pub(super) use html::*;
pub(super) use inline::*;
pub(super) use tables::*;
pub(super) use wrap::*;

#[cfg(test)]
mod tests;
