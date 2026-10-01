//! Original bytes indexed by decoded file line. Loaded off the UI thread.
use gitcomet_core::domain::{FileDiffText, FileDiffTextSource};
use gitcomet_core::text_format::TextEncoding;
use std::sync::Arc;

pub(crate) struct RawLines {
    bytes: Arc<[u8]>,
    starts: Vec<usize>,
}

impl RawLines {
    pub(crate) fn new(bytes: Arc<[u8]>, encoding: TextEncoding) -> Self {
        let bom = TextEncoding::for_bom(&bytes).map_or(0, |(_, size)| size);
        let mut starts = vec![bom];
        if encoding == TextEncoding::UTF_16LE || encoding == TextEncoding::UTF_16BE {
            for (ix, unit) in bytes[bom..].as_chunks::<2>().0.iter().enumerate() {
                let newline = if encoding == TextEncoding::UTF_16LE {
                    [10, 0]
                } else {
                    [0, 10]
                };
                if *unit == newline {
                    starts.push(bom + (ix + 1) * 2);
                }
            }
        } else {
            // Every other supported codec keeps LF as one byte, including
            // stateful ISO-2022-JP. Never re-encode lossy decoded text.
            starts.extend(
                bytes
                    .iter()
                    .enumerate()
                    .filter_map(|(ix, byte)| (*byte == b'\n').then_some(ix + 1)),
            );
        }
        if starts.last() == Some(&bytes.len()) {
            starts.pop();
        }
        Self { bytes, starts }
    }

    pub(crate) fn range(&self, start: u32, end: u32) -> Option<Arc<[u8]>> {
        if start == 0 || end < start {
            return None;
        }
        let from = *self.starts.get(start as usize - 1)?;
        let to = self
            .starts
            .get(end as usize)
            .copied()
            .unwrap_or(self.bytes.len());
        (end as usize <= self.starts.len()).then(|| Arc::from(&self.bytes[from..to]))
    }

    pub(crate) fn file(text: &FileDiffText) -> [Option<Self>; 2] {
        fn side(
            inline: &Option<Arc<str>>,
            source: &Option<FileDiffTextSource>,
        ) -> Option<RawLines> {
            if let Some(source) = source {
                let path = source.raw_path.as_ref().unwrap_or(&source.path);
                let bytes = std::fs::read(path).ok()?;
                let encoding = source
                    .format
                    .map_or(TextEncoding::UTF_8, |format| format.format.encoding);
                Some(RawLines::new(bytes.into(), encoding))
            } else {
                inline
                    .as_ref()
                    .map(|text| RawLines::new(Arc::from(text.as_bytes()), TextEncoding::UTF_8))
            }
        }
        [
            side(&text.old, &text.old_source),
            side(&text.new, &text.new_source),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_original_malformed_bytes_and_crlf() {
        let lines = RawLines::new(Arc::from(&b"a\r\n\xffb\r\nc"[..]), TextEncoding::UTF_8);
        assert_eq!(&*lines.range(2, 2).unwrap(), b"\xffb\r\n");
        assert_eq!(&*lines.range(2, 3).unwrap(), b"\xffb\r\nc");
        assert!(lines.range(0, 1).is_none());
        assert!(lines.range(1, 4).is_none());
    }
    #[test]
    fn utf16_lines_respect_units_and_omit_bom() {
        let lines = RawLines::new(
            Arc::from(&b"\xff\xfea\0\r\0\n\0b\0"[..]),
            TextEncoding::UTF_16LE,
        );
        assert_eq!(&*lines.range(1, 1).unwrap(), b"a\0\r\0\n\0");
        assert_eq!(&*lines.range(2, 2).unwrap(), b"b\0");
    }
}
