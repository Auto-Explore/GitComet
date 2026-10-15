//! The markdown preview model: parsing into preview rows, diff alignment,
//! anchors, tables, HTML blocks and wrapping. Renderers style and paint rows;
//! each row offers a cache slot for the renderer's styled text.

use gpui_shared_string::SharedString;
use std::ops::Range;
use std::sync::{Arc, Mutex};

/// Maximum source size (bytes) for a single markdown preview document.
pub const MAX_PREVIEW_SOURCE_BYTES: usize = 1_024 * 1_024; // 1 MiB

/// Maximum combined source size (bytes) for a two-sided diff preview.
pub const MAX_DIFF_PREVIEW_SOURCE_BYTES: usize = 2 * 1_024 * 1_024; // 2 MiB

/// Maximum number of preview rows per document.
pub const MAX_PREVIEW_ROWS: usize = 20_000;

/// Maximum number of inline spans per row before degrading to plain text.
const MAX_INLINE_SPANS_PER_ROW: usize = 512;

// ── Core types ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownPreviewDocument {
    pub rows: Vec<MarkdownPreviewRow>,
    pub anchors: MarkdownAnchorIndexCell,
}

impl MarkdownPreviewDocument {
    pub fn new(rows: Vec<MarkdownPreviewRow>) -> Self {
        Self {
            rows,
            anchors: MarkdownAnchorIndexCell::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownPreviewDiff {
    /// Both sides padded with spacer rows to the same aligned row indices.
    pub old: MarkdownPreviewDocument,
    pub new: MarkdownPreviewDocument,
    pub inline: MarkdownPreviewDocument,
    /// Which rows of `inline` show the old version. A modified paragraph is
    /// drawn twice, both copies marked modified, so the change hint alone
    /// cannot tell the old copy — whose links open the file before the change —
    /// from the new one.
    pub inline_old: Vec<bool>,
    /// How the flowing renderer groups each document's rows.
    pub old_blocks: Vec<MarkdownBlock>,
    pub new_blocks: Vec<MarkdownBlock>,
    pub inline_blocks: Vec<MarkdownBlock>,
    /// Side-by-side slices of `old`/`new` for the split view.
    pub bands: Vec<MarkdownDiffBand>,
    /// What each side's file held, which says why a side shows no block.
    pub old_source: MarkdownDiffSideSource,
    pub new_source: MarkdownDiffSideSource,
}

/// What one side of a diff held.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MarkdownDiffSideSource {
    /// No file on this side: it was added, or deleted.
    Missing,
    #[default]
    Empty,
    /// Text, though perhaps none that renders.
    Text,
}

impl MarkdownDiffSideSource {
    pub fn of(source: Option<&str>) -> Self {
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
    pub fn new(
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
    pub fn old_empty_notice(&self) -> &'static str {
        self.old_source.notice("File added.")
    }

    /// The notice standing in for the new side when it has no block.
    pub fn new_empty_notice(&self) -> &'static str {
        self.new_source.notice("File deleted.")
    }

    /// The notice standing in for a diff with no block on either side.
    pub fn empty_notice(&self) -> &'static str {
        if [self.old_source, self.new_source].contains(&MarkdownDiffSideSource::Text) {
            "Nothing to render."
        } else {
            "Empty file."
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownDiagramSource {
    pub kind: gitcomet_diagrams::DiagramKind,
    pub info: Arc<str>,
    pub source: Arc<str>,
    pub fence_lines: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownPreviewRow {
    pub kind: MarkdownPreviewRowKind,
    pub text: SharedString,
    pub inline_spans: Arc<Vec<MarkdownInlineSpan>>,
    pub code_language: Option<gitcomet_syntax::DiffSyntaxLanguage>,
    pub diagram: Option<Arc<MarkdownDiagramSource>>,
    pub source_line_range: Range<usize>,
    pub change_hint: MarkdownChangeHint,
    pub indent_level: u8,
    pub blockquote_level: u8,
    pub footnote_label: Option<SharedString>,
    pub alert_kind: Option<MarkdownAlertKind>,
    pub starts_alert: bool,
    /// The image an [`MarkdownPreviewRowKind::Image`] row paints.
    pub image: Option<Arc<MarkdownImage>>,
    /// Pictures that share this row's line with its text, in document order.
    pub inline_images: Arc<[MarkdownInlineImage]>,
    pub styled_text_cache: MarkdownPreviewRowStyledTextCache,
    /// Cell layout of a [`MarkdownPreviewRowKind::TableRow`]; `None` otherwise.
    pub table: Option<MarkdownTableRow>,
    /// The `[ ]`/`[x]` a task-list item opens with; `None` otherwise.
    pub task: Option<MarkdownTaskMarker>,
    /// A later row of a list item (another paragraph, a line after a hard
    /// break): it keeps the item's indent but draws no second bullet.
    pub continues_item: bool,
    /// The `align` of the HTML block around it (`<p align="center">`).
    pub align: MarkdownTextAlign,
}

impl Default for MarkdownPreviewRow {
    /// An empty spacer row; fixtures override what they need.
    fn default() -> Self {
        Self {
            kind: MarkdownPreviewRowKind::Spacer,
            text: SharedString::default(),
            inline_spans: Arc::default(),
            code_language: None,
            diagram: None,
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
            align: MarkdownTextAlign::None,
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
pub struct MarkdownTaskMarker {
    pub checked: bool,
    pub line: usize,
    pub column: usize,
    /// The marker's line as parsed, so a line that moved there since — another
    /// item, its `[ ]` in the same column — is not mistaken for it.
    pub line_hash: u64,
}

impl MarkdownTaskMarker {
    /// Where the marker's `[` falls in `text`, if that line is still the one
    /// the marker was parsed from.
    pub fn byte_offset(&self, text: &[u8]) -> Option<usize> {
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
pub fn task_line_hash(line: &[u8], column: usize) -> u64 {
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
pub struct MarkdownTableRow {
    /// One entry per column; a row shorter than the table gets empty cells.
    pub cells: Arc<[MarkdownTableCell]>,
    /// Table boundaries are independent of whether a row contains headers.
    pub starts_table: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownTableCell {
    pub range: Range<usize>,
    pub is_header: bool,
    /// Resolved alignment, including default centering for headers.
    pub align: MarkdownTextAlign,
    /// Empty for text-only cells; mixed content retains document order.
    pub content: Vec<MarkdownTableCellPart>,
}

impl MarkdownTableCell {
    fn new(range: Range<usize>) -> Self {
        Self {
            range,
            is_header: false,
            align: MarkdownTextAlign::None,
            content: Vec::new(),
        }
    }
}

/// Ranges address the row's logical text, including each image's alt text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownTableCellPart {
    Text(Range<usize>),
    Image { index: usize, range: Range<usize> },
}

/// A table column's alignment, or an HTML block's `align`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MarkdownTextAlign {
    #[default]
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkdownPreviewRowKind {
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
    pub fn is_alignment_padding(&self) -> bool {
        matches!(self.kind, MarkdownPreviewRowKind::Spacer) && self.source_line_range.is_empty()
    }
}

/// Height, in design pixels, a picture reserves when the document says
/// nothing about its size.
pub const MARKDOWN_PREVIEW_IMAGE_DEFAULT_HEIGHT_PX: u32 = 224;

/// Combine the inline style stack into a single effective style.
fn resolve_style_stack(
    stack: impl IntoIterator<Item = MarkdownInlineStyle>,
) -> MarkdownInlineStyle {
    let mut has_bold = false;
    let mut has_italic = false;
    let mut has_strikethrough = false;
    let mut has_link = false;
    let mut has_code = false;
    let mut has_underline = false;

    for s in stack {
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

pub use anchors::*;
pub use document::*;
use flatten::*;
pub use html::*;
use inline::*;
use tables::*;
pub use wrap::*;

#[cfg(test)]
mod diagram_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod table_tests;
