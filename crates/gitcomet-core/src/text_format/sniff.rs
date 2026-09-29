use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};

use super::{
    FormatSource, LineEndingCounter, LineEndingStats, SideKind, SideTextFormat, TextAttributes,
    TextEncoding, TextFormat,
};

/// Bytes git inspects for NUL to call content binary (`buffer_is_binary`).
const BINARY_SNIFF_BYTES: usize = 8000;
/// Most bytes handed to the charset detector, counted from the first byte
/// that is not plain ASCII.
const DETECTION_SAMPLE_BYTES: usize = 256 * 1024;

/// One streaming pass over content that collects everything encoding
/// resolution needs. Feed it the bytes as they are read.
#[derive(Debug)]
pub struct ContentSniffer {
    head: [u8; 4],
    head_len: usize,
    prefix_len: usize,
    prefix_has_nul: bool,
    /// Aligned 2-byte units in the prefix shaped like ASCII in UTF-16LE / BE,
    /// and units that are both zero.
    utf16_le_ascii: u32,
    utf16_be_ascii: u32,
    utf16_units: u32,
    utf16_zero_units: u32,
    pending_unit_byte: Option<u8>,
    utf8_valid: bool,
    utf8_tail: [u8; 4],
    utf8_tail_len: usize,
    ascii_only: bool,
    escape_seen: bool,
    sample_started: bool,
    sample: Vec<u8>,
    line_endings: LineEndingCounter,
    len: u64,
}

impl Default for ContentSniffer {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentSniffer {
    pub fn new() -> Self {
        Self {
            head: [0; 4],
            head_len: 0,
            prefix_len: 0,
            prefix_has_nul: false,
            utf16_le_ascii: 0,
            utf16_be_ascii: 0,
            utf16_units: 0,
            utf16_zero_units: 0,
            pending_unit_byte: None,
            utf8_valid: true,
            utf8_tail: [0; 4],
            utf8_tail_len: 0,
            ascii_only: true,
            escape_seen: false,
            sample_started: false,
            sample: Vec::new(),
            line_endings: LineEndingCounter::default(),
            len: 0,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        self.len = self.len.saturating_add(chunk.len() as u64);
        if self.head_len < self.head.len() {
            let take = (self.head.len() - self.head_len).min(chunk.len());
            self.head[self.head_len..self.head_len + take].copy_from_slice(&chunk[..take]);
            self.head_len += take;
        }
        self.feed_prefix(chunk);
        self.feed_utf8(chunk);
        self.feed_sample(chunk);
        self.line_endings.feed(chunk);
    }

    fn feed_prefix(&mut self, chunk: &[u8]) {
        if self.prefix_len >= BINARY_SNIFF_BYTES {
            return;
        }
        let take = (BINARY_SNIFF_BYTES - self.prefix_len).min(chunk.len());
        let bytes = &chunk[..take];
        self.prefix_len += take;
        if !self.prefix_has_nul && memchr::memchr(0, bytes).is_some() {
            self.prefix_has_nul = true;
        }
        let mut bytes = bytes;
        if let Some(first) = self.pending_unit_byte.take() {
            self.count_unit(first, bytes[0]);
            bytes = &bytes[1..];
        }
        let (units, rest) = bytes.as_chunks::<2>();
        for [first, second] in units {
            self.count_unit(*first, *second);
        }
        if let [last] = rest {
            self.pending_unit_byte = Some(*last);
        }
    }

    fn count_unit(&mut self, first: u8, second: u8) {
        fn texty(byte: u8) -> bool {
            matches!(byte, b'\t' | b'\n' | b'\r' | 0x20..=0x7e)
        }
        self.utf16_units += 1;
        match (first, second) {
            (0, 0) => self.utf16_zero_units += 1,
            (lo, 0) if texty(lo) => self.utf16_le_ascii += 1,
            (0, lo) if texty(lo) => self.utf16_be_ascii += 1,
            _ => {}
        }
    }

    fn feed_utf8(&mut self, mut chunk: &[u8]) {
        if !self.utf8_valid {
            return;
        }
        if self.utf8_tail_len > 0 {
            let need = utf8_sequence_len(self.utf8_tail[0]);
            let take = (need - self.utf8_tail_len).min(chunk.len());
            self.utf8_tail[self.utf8_tail_len..self.utf8_tail_len + take]
                .copy_from_slice(&chunk[..take]);
            self.utf8_tail_len += take;
            chunk = &chunk[take..];
            if self.utf8_tail_len < need {
                return;
            }
            if std::str::from_utf8(&self.utf8_tail[..need]).is_err() {
                self.utf8_valid = false;
                return;
            }
            self.utf8_tail_len = 0;
        }
        let valid_up_to = encoding_rs::Encoding::utf8_valid_up_to(chunk);
        if valid_up_to == chunk.len() {
            return;
        }
        let rest = &chunk[valid_up_to..];
        match std::str::from_utf8(rest) {
            Err(error) if error.valid_up_to() == 0 && error.error_len().is_none() => {
                self.utf8_tail[..rest.len()].copy_from_slice(rest);
                self.utf8_tail_len = rest.len();
            }
            _ => self.utf8_valid = false,
        }
    }

    fn feed_sample(&mut self, chunk: &[u8]) {
        let ascii = chunk.is_ascii();
        if !ascii {
            self.ascii_only = false;
        }
        if !self.escape_seen && memchr::memchr(0x1b, chunk).is_some() {
            self.escape_seen = true;
        }
        if self.sample.len() >= DETECTION_SAMPLE_BYTES {
            return;
        }
        let start = if self.sample_started {
            0
        } else {
            let first_non_ascii = if ascii {
                None
            } else {
                chunk.iter().position(|&byte| byte >= 0x80)
            };
            let first_escape = if self.escape_seen {
                memchr::memchr(0x1b, chunk)
            } else {
                None
            };
            match (first_non_ascii, first_escape) {
                (Some(a), Some(b)) => a.min(b),
                (Some(pos), None) | (None, Some(pos)) => pos,
                (None, None) => return,
            }
        };
        self.sample_started = true;
        let take = (DETECTION_SAMPLE_BYTES - self.sample.len()).min(chunk.len() - start);
        self.sample.extend_from_slice(&chunk[start..start + take]);
    }

    /// Whether the completed binary prefix already rules out text. Wait for
    /// the full prefix (UTF-16 detection needs it) and a definite UTF-8 error,
    /// not a character split across read buffers.
    pub fn is_binary(
        &self,
        kind: SideKind,
        attributes: &TextAttributes,
        encoding: Option<TextEncoding>,
    ) -> bool {
        if self.prefix_len < BINARY_SNIFF_BYTES || self.utf8_valid || !self.prefix_has_nul {
            return false;
        }
        self.snapshot(Vec::new())
            .resolve(kind, attributes, encoding)
            .binary
    }

    pub fn finish(mut self) -> ContentSniff {
        let sample = std::mem::take(&mut self.sample);
        self.snapshot(sample)
    }

    fn snapshot(&self, sample: Vec<u8>) -> ContentSniff {
        let head = &self.head[..self.head_len];
        ContentSniff {
            bom: TextEncoding::for_bom(head),
            unsupported_bom: TextEncoding::has_unsupported_bom(head),
            utf16: self.utf16_guess(),
            has_nul: self.prefix_has_nul,
            utf8_valid: self.utf8_valid && self.utf8_tail_len == 0,
            ascii_only: self.ascii_only,
            escape_seen: self.escape_seen,
            sample,
            line_endings: self.line_endings.finish(),
            len: self.len,
        }
    }

    /// BOM-less UTF-16: mostly ASCII-shaped units on one side, and no units
    /// that are both zero (U+0000 does not appear in text).
    fn utf16_guess(&self) -> Option<TextEncoding> {
        if !self.prefix_has_nul || self.utf16_units < 2 {
            return None;
        }
        let units = self.utf16_units;
        if self.utf16_zero_units * 100 > units {
            return None;
        }
        if self.utf16_le_ascii * 2 >= units && self.utf16_be_ascii * 20 <= units {
            Some(TextEncoding::UTF_16LE)
        } else if self.utf16_be_ascii * 2 >= units && self.utf16_le_ascii * 20 <= units {
            Some(TextEncoding::UTF_16BE)
        } else {
            None
        }
    }
}

fn utf8_sequence_len(lead: u8) -> usize {
    match lead {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

/// What a [`ContentSniffer`] saw.
#[derive(Clone, Debug)]
pub struct ContentSniff {
    pub bom: Option<(TextEncoding, usize)>,
    unsupported_bom: bool,
    pub utf16: Option<TextEncoding>,
    pub has_nul: bool,
    pub utf8_valid: bool,
    pub ascii_only: bool,
    escape_seen: bool,
    sample: Vec<u8>,
    /// Byte-level counts; only meaningful for ASCII-compatible encodings.
    pub line_endings: LineEndingStats,
    pub len: u64,
}

impl ContentSniff {
    pub fn of(bytes: &[u8]) -> Self {
        let mut sniffer = ContentSniffer::new();
        sniffer.feed(bytes);
        sniffer.finish()
    }

    /// Decide how to read these bytes. `malformed` and `lossy` are left false:
    /// they are only known after decoding.
    pub fn resolve(
        &self,
        kind: SideKind,
        attributes: &TextAttributes,
        override_encoding: Option<TextEncoding>,
    ) -> SideTextFormat {
        let working_tree_encoding = attributes.working_tree_encoding();
        let with = |encoding: TextEncoding, source| self.format(encoding, source);

        if self.unsupported_bom {
            return with(TextEncoding::UTF_8, FormatSource::Binary);
        }
        if let Some(encoding) = override_encoding
            && !(kind == SideKind::GitInternal && working_tree_encoding.is_some())
        {
            return with(encoding, FormatSource::Override);
        }
        if kind == SideKind::Worktree
            && let Some(encoding) = working_tree_encoding
        {
            // A UTF-16 label's endianness yields to the file's own BOM.
            let encoding = match self.bom {
                Some((bom, _)) if encoding.is_utf16() && bom.is_utf16() => bom,
                _ => encoding,
            };
            return with(encoding, FormatSource::WorkingTreeEncoding);
        }
        if let Some((encoding, _)) = self.bom {
            return with(encoding, FormatSource::Bom);
        }
        if kind == SideKind::GitInternal
            && working_tree_encoding.is_some()
            && self.utf8_valid
            && self.utf16.is_none()
        {
            return with(TextEncoding::UTF_8, FormatSource::GitInternalUtf8);
        }
        // A display hint, not a claim of text: NULs still mean binary unless
        // the attribute names UTF-16, whose text is full of them.
        if let Some(encoding) = attributes.encoding.as_ref().and_then(|attr| attr.encoding)
            && (!self.has_nul || encoding.is_utf16())
        {
            return with(encoding, FormatSource::EncodingAttribute);
        }
        if let Some(encoding) = self.utf16 {
            return with(encoding, FormatSource::Detected { confident: true });
        }
        if self.utf8_valid {
            if self.ascii_only && self.escape_seen {
                let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
                detector.feed(&self.sample, true);
                if detector.guess(None, Utf8Detection::Allow) == encoding_rs::ISO_2022_JP {
                    return with(
                        TextEncoding::from_whatwg(encoding_rs::ISO_2022_JP),
                        FormatSource::Detected { confident: true },
                    );
                }
            }
            return with(TextEncoding::UTF_8, FormatSource::Utf8);
        }
        if self.has_nul {
            return with(TextEncoding::UTF_8, FormatSource::Binary);
        }
        if let Some(encoding) = attributes
            .gui_encoding
            .as_ref()
            .and_then(|attr| attr.encoding)
            .filter(|encoding| !encoding.is_utf8())
        {
            return with(encoding, FormatSource::GuiEncoding);
        }
        // chardetng no longer reports confidence, so keep heuristic guesses uncertain.
        with(self.guess(), FormatSource::Detected { confident: false })
    }

    fn guess(&self) -> TextEncoding {
        let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
        // End the sample on an ASCII byte so a sequence cut at the budget is
        // not scored as an error.
        let sample = match self.sample.iter().rposition(|&byte| byte < 0x80) {
            Some(end) if self.sample.len() >= DETECTION_SAMPLE_BYTES => &self.sample[..=end],
            _ => &self.sample[..],
        };
        detector.feed(sample, true);
        TextEncoding::from_whatwg(detector.guess(None, Utf8Detection::Deny))
    }

    fn format(&self, encoding: TextEncoding, source: FormatSource) -> SideTextFormat {
        let bom = self.bom.is_some_and(|(bom, _)| bom == encoding);
        SideTextFormat {
            format: TextFormat { encoding, bom },
            source,
            binary: source == FormatSource::Binary,
            malformed: false,
            lossy: false,
            line_endings: if encoding.is_ascii_compatible() {
                self.line_endings
            } else {
                LineEndingStats::default()
            },
        }
    }
}
