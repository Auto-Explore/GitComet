//! Reading and writing file text in any encoding and line-ending style.
//!
//! Content is decoded once at the boundary where it is read (backend for git
//! objects, UI for raw working-tree files); everything past that sees UTF-8.

mod attributes;
mod codec;
mod encoding;
mod line_ending;
mod sniff;

pub use attributes::{
    AutoCrlf, EncodingAttr, EolPolicy, EolSource, TabWidth, TabWidthSource, TextAttr,
    TextAttributes, parse_whitespace_tab_width,
};
pub use codec::{
    Decoded, TranscodeStats, Unmappable, decode, encode, round_trips, transcode_to_utf8,
};
pub use encoding::TextEncoding;
pub use line_ending::{LineEnding, LineEndingCounter, LineEndingStats, convert_line_endings};
pub use sniff::{ContentSniff, ContentSniffer};

use std::borrow::Cow;

/// Whether bytes are in git's stored form (blobs, index, `git diff` output,
/// the normalized working-tree side) or as they sit in the working tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SideKind {
    GitInternal,
    Worktree,
}

/// An encoding plus whether the content starts with its byte-order mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextFormat {
    pub encoding: TextEncoding,
    pub bom: bool,
}

impl TextFormat {
    pub const UTF_8: Self = Self {
        encoding: TextEncoding::UTF_8,
        bom: false,
    };

    /// Plain UTF-8, which needs no decoding.
    pub fn is_plain_utf8(self) -> bool {
        self == Self::UTF_8
    }
}

impl Default for TextFormat {
    fn default() -> Self {
        Self::UTF_8
    }
}

/// Why content is read in the encoding it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormatSource {
    /// Chosen by the user in the view.
    Override,
    /// `working-tree-encoding` in `.gitattributes`.
    WorkingTreeEncoding,
    /// Git stores `working-tree-encoding` files as UTF-8.
    GitInternalUtf8,
    /// Byte-order mark.
    Bom,
    /// `encoding` in `.gitattributes`.
    EncodingAttribute,
    /// Valid UTF-8 (including plain ASCII).
    Utf8,
    /// `gui.encoding` in git config.
    GuiEncoding,
    /// Guessed from the bytes.
    Detected { confident: bool },
    /// Not text in any encoding tried.
    Binary,
}

/// How one side of a view was decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SideTextFormat {
    pub format: TextFormat,
    pub source: FormatSource,
    /// Not decodable as text; shown as binary.
    pub binary: bool,
    /// Invalid sequences were replaced with U+FFFD.
    pub malformed: bool,
    /// Writing the decoded text back would not reproduce the original bytes.
    pub lossy: bool,
    pub line_endings: LineEndingStats,
}

impl SideTextFormat {
    /// Plain UTF-8 content that decoded cleanly.
    pub fn utf8(line_endings: LineEndingStats) -> Self {
        Self {
            format: TextFormat::UTF_8,
            source: FormatSource::Utf8,
            binary: false,
            malformed: false,
            lossy: false,
            line_endings,
        }
    }

    /// Choose a save format using the original read's safety information.
    /// A BOM change alone cannot repair a lossy legacy encoding.
    pub fn for_save_as(mut self, format: TextFormat) -> Self {
        self.lossy &= self.format.encoding == format.encoding;
        self.format = format;
        self.source = FormatSource::Override;
        self
    }

    /// Writing the decoded text back is known to reproduce the bytes.
    pub fn is_writable(&self) -> bool {
        !self.binary && !self.malformed && !self.lossy
    }
}

/// The user's per-file choices, all optional.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextOverride {
    pub encoding: Option<TextEncoding>,
    pub tab_size: Option<u8>,
}

impl TextOverride {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Decoded content and how it was read.
#[derive(Debug)]
pub struct DecodedText<'a> {
    pub text: Cow<'a, str>,
    pub format: SideTextFormat,
}

/// Resolve and decode in-memory content in one call. Binary content decodes
/// as lossy UTF-8 with `format.binary` set, so callers decide what to show.
pub fn decode_bytes<'a>(
    bytes: &'a [u8],
    kind: SideKind,
    attributes: &TextAttributes,
    override_encoding: Option<TextEncoding>,
) -> DecodedText<'a> {
    let sniff = ContentSniff::of(bytes);
    decode_in_format(bytes, sniff.resolve(kind, attributes, override_encoding))
}

/// Decode a resolved format and finish its line-ending and round-trip checks.
/// Callers with a streaming sniff or a known source format need no second guess.
pub fn decode_in_format(bytes: &[u8], mut format: SideTextFormat) -> DecodedText<'_> {
    let decoded = decode(bytes, format.format);
    format.malformed = decoded.malformed;
    if !format.format.encoding.is_ascii_compatible() {
        format.line_endings = LineEndingStats::from_bytes(decoded.text.as_bytes());
    }
    if !decoded.malformed && !format.binary {
        format.lossy = !round_trips(bytes, &decoded.text, format.format);
    }
    DecodedText {
        text: decoded.text,
        format,
    }
}

#[cfg(test)]
mod tests;
