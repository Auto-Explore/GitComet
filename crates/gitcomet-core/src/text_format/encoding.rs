use encoding_rs::Encoding;
use std::fmt;

/// A text encoding file content can be decoded from and written back in.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextEncoding(Repr);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Repr {
    /// A WHATWG encoding, UTF-8 and UTF-16 included.
    Whatwg(&'static Encoding),
    /// True ISO-8859-1 (byte value = code point), which is what git and iconv
    /// mean by `latin-1`. WHATWG maps that label to windows-1252 instead.
    Latin1,
}

struct CatalogEntry {
    encoding: TextEncoding,
    name: &'static str,
    group: &'static str,
    /// iconv spelling, written to `working-tree-encoding`.
    git_label: &'static str,
}

macro_rules! entry {
    ($enc:expr, $name:literal, $group:literal, $git:literal) => {
        CatalogEntry {
            encoding: $enc,
            name: $name,
            group: $group,
            git_label: $git,
        }
    };
}

const fn whatwg(encoding: &'static Encoding) -> TextEncoding {
    TextEncoding(Repr::Whatwg(encoding))
}

/// Every encoding offered to the user, in picker order.
static CATALOG: &[CatalogEntry] = &[
    entry!(TextEncoding::UTF_8, "UTF-8", "Unicode", "UTF-8"),
    entry!(TextEncoding::UTF_16LE, "UTF-16LE", "Unicode", "UTF-16LE"),
    entry!(TextEncoding::UTF_16BE, "UTF-16BE", "Unicode", "UTF-16BE"),
    entry!(
        TextEncoding::WINDOWS_1252,
        "Windows-1252",
        "Western",
        "WINDOWS-1252"
    ),
    entry!(
        TextEncoding::ISO_8859_1,
        "ISO-8859-1",
        "Western",
        "ISO-8859-1"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_15_INIT),
        "ISO-8859-15",
        "Western",
        "ISO-8859-15"
    ),
    entry!(
        whatwg(&encoding_rs::MACINTOSH_INIT),
        "Mac Roman",
        "Western",
        "MACINTOSH"
    ),
    entry!(
        whatwg(&encoding_rs::WINDOWS_1250_INIT),
        "Windows-1250",
        "Central European",
        "WINDOWS-1250"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_2_INIT),
        "ISO-8859-2",
        "Central European",
        "ISO-8859-2"
    ),
    entry!(
        whatwg(&encoding_rs::WINDOWS_1251_INIT),
        "Windows-1251",
        "Cyrillic",
        "WINDOWS-1251"
    ),
    entry!(
        whatwg(&encoding_rs::KOI8_R_INIT),
        "KOI8-R",
        "Cyrillic",
        "KOI8-R"
    ),
    entry!(
        whatwg(&encoding_rs::KOI8_U_INIT),
        "KOI8-U",
        "Cyrillic",
        "KOI8-U"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_5_INIT),
        "ISO-8859-5",
        "Cyrillic",
        "ISO-8859-5"
    ),
    entry!(
        whatwg(&encoding_rs::IBM866_INIT),
        "IBM866",
        "Cyrillic",
        "IBM866"
    ),
    entry!(
        whatwg(&encoding_rs::X_MAC_CYRILLIC_INIT),
        "Mac Cyrillic",
        "Cyrillic",
        "MAC-CYRILLIC"
    ),
    entry!(
        whatwg(&encoding_rs::WINDOWS_1253_INIT),
        "Windows-1253",
        "Greek",
        "WINDOWS-1253"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_7_INIT),
        "ISO-8859-7",
        "Greek",
        "ISO-8859-7"
    ),
    entry!(
        whatwg(&encoding_rs::WINDOWS_1254_INIT),
        "Windows-1254",
        "Turkish",
        "WINDOWS-1254"
    ),
    entry!(
        whatwg(&encoding_rs::WINDOWS_1255_INIT),
        "Windows-1255",
        "Hebrew",
        "WINDOWS-1255"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_8_INIT),
        "ISO-8859-8",
        "Hebrew",
        "ISO-8859-8"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_8_I_INIT),
        "ISO-8859-8-I",
        "Hebrew",
        "ISO-8859-8"
    ),
    entry!(
        whatwg(&encoding_rs::WINDOWS_1256_INIT),
        "Windows-1256",
        "Arabic",
        "WINDOWS-1256"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_6_INIT),
        "ISO-8859-6",
        "Arabic",
        "ISO-8859-6"
    ),
    entry!(
        whatwg(&encoding_rs::WINDOWS_1257_INIT),
        "Windows-1257",
        "Baltic",
        "WINDOWS-1257"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_4_INIT),
        "ISO-8859-4",
        "Baltic",
        "ISO-8859-4"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_13_INIT),
        "ISO-8859-13",
        "Baltic",
        "ISO-8859-13"
    ),
    entry!(
        whatwg(&encoding_rs::WINDOWS_1258_INIT),
        "Windows-1258",
        "Vietnamese",
        "WINDOWS-1258"
    ),
    entry!(
        whatwg(&encoding_rs::WINDOWS_874_INIT),
        "Windows-874",
        "Thai",
        "WINDOWS-874"
    ),
    entry!(
        whatwg(&encoding_rs::SHIFT_JIS_INIT),
        "Shift_JIS",
        "Japanese",
        "SHIFT_JIS"
    ),
    entry!(
        whatwg(&encoding_rs::EUC_JP_INIT),
        "EUC-JP",
        "Japanese",
        "EUC-JP"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_2022_JP_INIT),
        "ISO-2022-JP",
        "Japanese",
        "ISO-2022-JP"
    ),
    entry!(
        whatwg(&encoding_rs::GBK_INIT),
        "GBK",
        "Chinese Simplified",
        "GBK"
    ),
    entry!(
        whatwg(&encoding_rs::GB18030_INIT),
        "GB18030",
        "Chinese Simplified",
        "GB18030"
    ),
    entry!(
        whatwg(&encoding_rs::BIG5_INIT),
        "Big5",
        "Chinese Traditional",
        "BIG5"
    ),
    entry!(
        whatwg(&encoding_rs::EUC_KR_INIT),
        "EUC-KR",
        "Korean",
        "EUC-KR"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_3_INIT),
        "ISO-8859-3",
        "Other",
        "ISO-8859-3"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_10_INIT),
        "ISO-8859-10",
        "Other",
        "ISO-8859-10"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_14_INIT),
        "ISO-8859-14",
        "Other",
        "ISO-8859-14"
    ),
    entry!(
        whatwg(&encoding_rs::ISO_8859_16_INIT),
        "ISO-8859-16",
        "Other",
        "ISO-8859-16"
    ),
];

/// Labels git (iconv) or git-gui (Tcl) accept that WHATWG does not, or maps
/// differently. Keys are lowercase.
static EXTRA_LABELS: &[(&str, TextEncoding)] = &[
    ("latin-1", TextEncoding::ISO_8859_1),
    ("latin1", TextEncoding::ISO_8859_1),
    ("l1", TextEncoding::ISO_8859_1),
    ("iso-8859-1", TextEncoding::ISO_8859_1),
    ("iso8859-1", TextEncoding::ISO_8859_1),
    ("iso88591", TextEncoding::ISO_8859_1),
    ("iso_8859-1", TextEncoding::ISO_8859_1),
    ("iso_8859-1:1987", TextEncoding::ISO_8859_1),
    ("iso-ir-100", TextEncoding::ISO_8859_1),
    ("cp819", TextEncoding::ISO_8859_1),
    ("ibm819", TextEncoding::ISO_8859_1),
    ("csisolatin1", TextEncoding::ISO_8859_1),
    ("utf-16le-bom", TextEncoding::UTF_16LE),
    ("utf-16be-bom", TextEncoding::UTF_16BE),
    ("ucs-2le", TextEncoding::UTF_16LE),
    ("ucs-2be", TextEncoding::UTF_16BE),
    ("utf16", TextEncoding::UTF_16LE),
    ("utf16le", TextEncoding::UTF_16LE),
    ("utf16be", TextEncoding::UTF_16BE),
    ("utf8", TextEncoding::UTF_8),
    ("cp932", whatwg(&encoding_rs::SHIFT_JIS_INIT)),
    ("shiftjis", whatwg(&encoding_rs::SHIFT_JIS_INIT)),
    ("cp936", whatwg(&encoding_rs::GBK_INIT)),
    ("euc-cn", whatwg(&encoding_rs::GBK_INIT)),
    ("cp949", whatwg(&encoding_rs::EUC_KR_INIT)),
    ("cp950", whatwg(&encoding_rs::BIG5_INIT)),
    ("cp874", whatwg(&encoding_rs::WINDOWS_874_INIT)),
    ("macroman", whatwg(&encoding_rs::MACINTOSH_INIT)),
    ("mac-cyrillic", whatwg(&encoding_rs::X_MAC_CYRILLIC_INIT)),
    ("maccyrillic", whatwg(&encoding_rs::X_MAC_CYRILLIC_INIT)),
];

impl TextEncoding {
    pub const UTF_8: Self = whatwg(&encoding_rs::UTF_8_INIT);
    pub const UTF_16LE: Self = whatwg(&encoding_rs::UTF_16LE_INIT);
    pub const UTF_16BE: Self = whatwg(&encoding_rs::UTF_16BE_INIT);
    pub const WINDOWS_1252: Self = whatwg(&encoding_rs::WINDOWS_1252_INIT);
    pub const ISO_8859_1: Self = TextEncoding(Repr::Latin1);

    /// Resolve a label from `.gitattributes` or git config: WHATWG names plus
    /// the iconv and Tcl spellings git and git-gui accept. `None` for labels
    /// that name nothing decodable here (UTF-32, `binary`, typos).
    pub fn from_label(label: &str) -> Option<Self> {
        let label = label.trim();
        if label.is_empty() || label.len() > 64 {
            return None;
        }
        let lower = label.to_ascii_lowercase();
        if let Some((_, encoding)) = EXTRA_LABELS.iter().find(|(name, _)| *name == lower) {
            return Some(*encoding);
        }
        Encoding::for_label_no_replacement(lower.as_bytes()).map(whatwg)
    }

    /// Every encoding the UI offers, grouped in picker order.
    pub fn all() -> impl Iterator<Item = Self> {
        CATALOG.iter().map(|entry| entry.encoding)
    }

    fn catalog_entry(self) -> Option<&'static CatalogEntry> {
        CATALOG.iter().find(|entry| entry.encoding == self)
    }

    /// Short display name, e.g. `Windows-1252`.
    pub fn name(self) -> &'static str {
        match self.catalog_entry() {
            Some(entry) => entry.name,
            None => match self.0 {
                Repr::Whatwg(encoding) => encoding.name(),
                Repr::Latin1 => "ISO-8859-1",
            },
        }
    }

    /// Script or region the encoding is used for, e.g. `Cyrillic`.
    pub fn group(self) -> &'static str {
        self.catalog_entry().map_or("Other", |entry| entry.group)
    }

    /// The spelling git's iconv accepts for `working-tree-encoding`.
    pub fn git_label(self) -> &'static str {
        match self.catalog_entry() {
            Some(entry) => entry.git_label,
            None => self.name(),
        }
    }

    pub fn is_utf8(self) -> bool {
        self == Self::UTF_8
    }

    pub fn is_utf16(self) -> bool {
        self == Self::UTF_16LE || self == Self::UTF_16BE
    }

    /// Whether ASCII bytes, `\n` included, always stand for themselves, so
    /// lines can be split on raw bytes.
    pub fn is_ascii_compatible(self) -> bool {
        match self.0 {
            Repr::Whatwg(encoding) => encoding.is_ascii_compatible(),
            Repr::Latin1 => true,
        }
    }

    /// Encodings where decoding and re-encoding can change bytes that were
    /// decoded without error (duplicate mappings), so a round trip is checked.
    pub(super) fn may_not_round_trip(self) -> bool {
        match self.0 {
            Repr::Whatwg(encoding) => {
                !encoding.is_single_byte()
                    && encoding != encoding_rs::UTF_8
                    && encoding != encoding_rs::UTF_16LE
                    && encoding != encoding_rs::UTF_16BE
            }
            Repr::Latin1 => false,
        }
    }

    /// The byte-order mark this encoding writes, if it has one.
    pub fn bom(self) -> Option<&'static [u8]> {
        if self == Self::UTF_8 {
            Some(b"\xEF\xBB\xBF")
        } else if self == Self::UTF_16LE {
            Some(b"\xFF\xFE")
        } else if self == Self::UTF_16BE {
            Some(b"\xFE\xFF")
        } else {
            None
        }
    }

    /// The encoding a leading byte-order mark announces, and its length.
    pub fn for_bom(bytes: &[u8]) -> Option<(Self, usize)> {
        if Self::has_unsupported_bom(bytes) {
            return None;
        }
        Encoding::for_bom(bytes).map(|(encoding, len)| (whatwg(encoding), len))
    }

    /// UTF-32 is unsupported; its little-endian BOM starts with UTF-16LE's.
    pub(super) fn has_unsupported_bom(bytes: &[u8]) -> bool {
        bytes.starts_with(b"\xFF\xFE\x00\x00") || bytes.starts_with(b"\x00\x00\xFE\xFF")
    }

    pub(super) fn from_whatwg(encoding: &'static Encoding) -> Self {
        whatwg(encoding)
    }

    pub(super) fn whatwg(self) -> Option<&'static Encoding> {
        match self.0 {
            Repr::Whatwg(encoding) => Some(encoding),
            Repr::Latin1 => None,
        }
    }
}

impl fmt::Debug for TextEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TextEncoding({})", self.name())
    }
}

impl fmt::Display for TextEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_resolve_git_and_iconv_spellings() {
        for (label, expected) in [
            ("UTF-8", TextEncoding::UTF_8),
            ("utf8", TextEncoding::UTF_8),
            ("latin-1", TextEncoding::ISO_8859_1),
            ("ISO-8859-1", TextEncoding::ISO_8859_1),
            ("cp1252", TextEncoding::WINDOWS_1252),
            ("windows-1252", TextEncoding::WINDOWS_1252),
            ("UTF-16LE-BOM", TextEncoding::UTF_16LE),
            ("UTF-16", TextEncoding::UTF_16LE),
            ("utf-16be", TextEncoding::UTF_16BE),
        ] {
            assert_eq!(TextEncoding::from_label(label), Some(expected), "{label}");
        }
        assert_eq!(
            TextEncoding::from_label("cp932").map(TextEncoding::name),
            Some("Shift_JIS")
        );
        assert_eq!(
            TextEncoding::from_label(" KOI8-R ").map(TextEncoding::name),
            Some("KOI8-R")
        );
    }

    #[test]
    fn unknown_and_unsupported_labels_resolve_to_none() {
        for label in [
            "",
            "UTF-32",
            "utf-32le",
            "binary",
            "no-such-encoding",
            "replacement",
        ] {
            assert_eq!(TextEncoding::from_label(label), None, "{label}");
        }
    }

    #[test]
    fn every_catalog_entry_round_trips_through_its_git_label() {
        for encoding in TextEncoding::all() {
            // ISO-8859-8-I shares iconv's ISO-8859-8 spelling.
            if encoding.name() == "ISO-8859-8-I" {
                continue;
            }
            assert_eq!(
                TextEncoding::from_label(encoding.git_label()),
                Some(encoding),
                "{}",
                encoding.name()
            );
        }
    }

    #[test]
    fn catalog_has_no_duplicates() {
        let all: Vec<_> = TextEncoding::all().collect();
        for (ix, encoding) in all.iter().enumerate() {
            assert!(!all[ix + 1..].contains(encoding), "{encoding}");
        }
    }
}
