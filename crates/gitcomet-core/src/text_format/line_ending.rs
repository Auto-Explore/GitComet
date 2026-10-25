use std::borrow::Cow;

/// A line terminator style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
            Self::Cr => "CR",
        }
    }

    pub fn platform() -> Self {
        if cfg!(windows) { Self::CrLf } else { Self::Lf }
    }
}

/// How many line breaks of each style a text contains.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LineEndingStats {
    pub lf: u64,
    pub crlf: u64,
    pub cr: u64,
}

impl LineEndingStats {
    /// Count line breaks in ASCII-compatible bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut counter = LineEndingCounter::default();
        counter.feed(bytes);
        counter.finish()
    }

    pub fn total(&self) -> u64 {
        self.lf + self.crlf + self.cr
    }

    /// The most common style; `None` for text without line breaks. Ties go to
    /// LF, then CRLF.
    pub fn dominant(&self) -> Option<LineEnding> {
        if self.total() == 0 {
            return None;
        }
        Some(if self.lf >= self.crlf && self.lf >= self.cr {
            LineEnding::Lf
        } else if self.crlf >= self.cr {
            LineEnding::CrLf
        } else {
            LineEnding::Cr
        })
    }

    /// More than one style is present.
    pub fn is_mixed(&self) -> bool {
        [self.lf, self.crlf, self.cr]
            .into_iter()
            .filter(|count| *count > 0)
            .count()
            > 1
    }

    /// The single style used throughout; `None` when mixed or when there are no
    /// line breaks.
    pub fn uniform(&self) -> Option<LineEnding> {
        if self.is_mixed() {
            None
        } else {
            self.dominant()
        }
    }
}

/// Counts line breaks across chunks; a CR at a chunk end waits for the next
/// chunk to decide between CR and CRLF.
#[derive(Clone, Copy, Debug, Default)]
pub struct LineEndingCounter {
    stats: LineEndingStats,
    pending_cr: bool,
}

impl LineEndingCounter {
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        if self.pending_cr {
            self.pending_cr = false;
            if rest.first() == Some(&b'\n') {
                self.stats.crlf += 1;
                rest = &rest[1..];
            } else {
                self.stats.cr += 1;
            }
        }
        let mut last_cr = None;
        for pos in memchr::memchr2_iter(b'\r', b'\n', rest) {
            match rest[pos] {
                b'\n' => {
                    match last_cr {
                        Some(cr) if cr + 1 == pos => self.stats.crlf += 1,
                        Some(_) => {
                            self.stats.cr += 1;
                            self.stats.lf += 1;
                        }
                        None => self.stats.lf += 1,
                    }
                    last_cr = None;
                }
                _ => {
                    if last_cr.is_some() {
                        self.stats.cr += 1;
                    }
                    last_cr = Some(pos);
                }
            }
        }
        if let Some(cr) = last_cr {
            if cr + 1 == rest.len() {
                self.pending_cr = true;
            } else {
                self.stats.cr += 1;
            }
        }
    }

    pub fn finish(mut self) -> LineEndingStats {
        if self.pending_cr {
            self.stats.cr += 1;
        }
        self.stats
    }
}

/// Rewrite every line break (LF, CRLF or lone CR) as `to`.
pub fn convert_line_endings(text: &str, to: LineEnding) -> Cow<'_, str> {
    let stats = LineEndingStats::from_bytes(text.as_bytes());
    if stats.total() == 0 || stats.uniform() == Some(to) {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + stats.lf as usize + stats.cr as usize);
    let mut start = 0;
    let mut ix = 0;
    while let Some(offset) = memchr::memchr2(b'\r', b'\n', &bytes[ix..]) {
        let pos = ix + offset;
        out.push_str(&text[start..pos]);
        out.push_str(to.as_str());
        ix = if bytes[pos] == b'\r' && bytes.get(pos + 1) == Some(&b'\n') {
            pos + 2
        } else {
            pos + 1
        };
        start = ix;
    }
    out.push_str(&text[start..]);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_each_style() {
        let stats = LineEndingStats::from_bytes(b"a\nb\r\nc\rd\r\r\n");
        assert_eq!(
            stats,
            LineEndingStats {
                lf: 1,
                crlf: 2,
                cr: 2
            }
        );
        assert!(stats.is_mixed());
        assert_eq!(stats.dominant(), Some(LineEnding::CrLf));
    }

    #[test]
    fn crlf_split_across_chunks_counts_once() {
        let mut counter = LineEndingCounter::default();
        counter.feed(b"one\r");
        counter.feed(b"\ntwo\r");
        counter.feed(b"three");
        assert_eq!(
            counter.finish(),
            LineEndingStats {
                lf: 0,
                crlf: 1,
                cr: 1
            }
        );
    }

    #[test]
    fn lone_cr_before_a_later_lf_is_counted() {
        assert_eq!(
            LineEndingStats::from_bytes(b"a\rb\n"),
            LineEndingStats {
                lf: 1,
                crlf: 0,
                cr: 1
            }
        );
    }

    #[test]
    fn trailing_cr_is_a_cr() {
        assert_eq!(LineEndingStats::from_bytes(b"x\r").cr, 1);
        assert_eq!(LineEndingStats::from_bytes(b"").dominant(), None);
    }

    #[test]
    fn convert_rewrites_all_styles() {
        assert_eq!(
            convert_line_endings("a\nb\r\nc\rd", LineEnding::CrLf),
            "a\r\nb\r\nc\r\nd"
        );
        assert_eq!(convert_line_endings("a\r\nb\r\n", LineEnding::Lf), "a\nb\n");
        assert!(matches!(
            convert_line_endings("a\nb\n", LineEnding::Lf),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            convert_line_endings("no breaks", LineEnding::CrLf),
            Cow::Borrowed(_)
        ));
    }
}
