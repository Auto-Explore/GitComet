use super::{LineEndingCounter, LineEndingStats, TextEncoding, TextFormat};
use crate::error::{Error, ErrorKind};
use crate::services::CancellationToken;
use encoding_rs::{CoderResult, EncoderResult};
use std::borrow::Cow;
use std::io::{Read, Write};

const TRANSCODE_CHUNK_BYTES: usize = 64 * 1024;

/// Decoded text; `malformed` when invalid sequences became U+FFFD.
#[derive(Debug)]
pub struct Decoded<'a> {
    pub text: Cow<'a, str>,
    pub malformed: bool,
}

fn strip_bom(bytes: &[u8], format: TextFormat) -> &[u8] {
    match format.encoding.bom() {
        Some(bom) if format.bom && bytes.starts_with(bom) => &bytes[bom.len()..],
        _ => bytes,
    }
}

/// Decode `bytes` (BOM included when `format.bom`).
pub fn decode(bytes: &[u8], format: TextFormat) -> Decoded<'_> {
    let bytes = strip_bom(bytes, format);
    if format.encoding.is_utf8() {
        return match std::str::from_utf8(bytes) {
            Ok(text) => Decoded {
                text: Cow::Borrowed(text),
                malformed: false,
            },
            Err(_) => Decoded {
                text: String::from_utf8_lossy(bytes),
                malformed: true,
            },
        };
    }
    match format.encoding.whatwg() {
        Some(encoding) => {
            let (text, malformed) = encoding.decode_without_bom_handling(bytes);
            Decoded { text, malformed }
        }
        None => Decoded {
            text: encoding_rs::mem::decode_latin1(bytes),
            malformed: false,
        },
    }
}

/// What a transcode produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TranscodeStats {
    pub malformed: bool,
    /// Decoded cleanly, but encoding the text again gives other bytes.
    pub lossy: bool,
    pub line_endings: LineEndingStats,
    pub utf8_len: u64,
}

fn io_error(error: std::io::Error) -> Error {
    Error::new(ErrorKind::Io(error.kind()))
}

/// Stream `reader` (in `format`) into `writer` as UTF-8 without a BOM.
pub fn transcode_to_utf8(
    mut reader: impl Read,
    mut writer: impl Write,
    format: TextFormat,
    cancellation: &CancellationToken,
) -> crate::services::Result<TranscodeStats> {
    let mut stats = TranscodeStats::default();
    let mut line_endings = LineEndingCounter::default();
    let mut input = vec![0u8; TRANSCODE_CHUNK_BYTES];
    let mut output = Vec::new();
    let mut decoder = format
        .encoding
        .whatwg()
        .map(|encoding| encoding.new_decoder_without_bom_handling());
    let mut round_trip = format
        .encoding
        .whatwg()
        .filter(|_| format.encoding.may_not_round_trip())
        .map(RoundTripCheck::new);
    let mut bom_pending = if format.bom {
        format.encoding.bom().unwrap_or_default()
    } else {
        &[]
    };

    loop {
        cancellation.check_cancelled()?;
        let read = read_full(&mut reader, &mut input).map_err(io_error)?;
        let last = read < input.len();
        let mut chunk = &input[..read];
        if !bom_pending.is_empty() {
            // A BOM is at most 3 bytes and the first read is 64 KiB, so it
            // arrives whole or the file is shorter than its BOM.
            if chunk.starts_with(bom_pending) {
                chunk = &chunk[bom_pending.len()..];
            }
            bom_pending = &[];
        }
        output.clear();
        match decoder.as_mut() {
            Some(decoder) => {
                let needed = decoder
                    .max_utf8_buffer_length(chunk.len())
                    .unwrap_or(chunk.len() * 3 + 16);
                output.resize(needed, 0);
                let mut written_total = 0;
                let mut src = chunk;
                loop {
                    let (result, read, written, had_errors) =
                        decoder.decode_to_utf8(src, &mut output[written_total..], last);
                    written_total += written;
                    stats.malformed |= had_errors;
                    src = &src[read..];
                    match result {
                        CoderResult::InputEmpty => break,
                        CoderResult::OutputFull => {
                            output.resize(output.len() * 2 + 16, 0);
                        }
                    }
                }
                output.truncate(written_total);
            }
            None => match encoding_rs::mem::decode_latin1(chunk) {
                Cow::Borrowed(text) => output.extend_from_slice(text.as_bytes()),
                Cow::Owned(text) => output = text.into_bytes(),
            },
        }
        if let Some(check) = round_trip.as_mut() {
            check.feed(chunk, &output, last);
        }
        line_endings.feed(&output);
        stats.utf8_len += output.len() as u64;
        writer.write_all(&output).map_err(io_error)?;
        if last {
            break;
        }
    }
    writer.flush().map_err(io_error)?;
    stats.line_endings = line_endings.finish();
    stats.lossy = round_trip.is_some_and(|check| check.lossy);
    Ok(stats)
}

/// [`round_trips`] over a stream: each decoded chunk is encoded again and
/// must reproduce the bytes it came from.
struct RoundTripCheck {
    encoder: encoding_rs::Encoder,
    /// Input the encoder has not given back yet (a sequence split by a read).
    unmatched: Vec<u8>,
    encoded: Vec<u8>,
    lossy: bool,
}

impl RoundTripCheck {
    fn new(encoding: &'static encoding_rs::Encoding) -> Self {
        Self {
            encoder: encoding.new_encoder(),
            unmatched: Vec::new(),
            encoded: Vec::new(),
            lossy: false,
        }
    }

    fn feed(&mut self, raw: &[u8], decoded_utf8: &[u8], last: bool) {
        if self.lossy {
            return;
        }
        self.unmatched.extend_from_slice(raw);
        let mut src = std::str::from_utf8(decoded_utf8).expect("decoder output is UTF-8");
        loop {
            let needed = self
                .encoder
                .max_buffer_length_from_utf8_without_replacement(src.len())
                .unwrap_or(src.len() * 4);
            self.encoded.resize(needed.max(16), 0);
            let (result, read, written) =
                self.encoder
                    .encode_from_utf8_without_replacement(src, &mut self.encoded, last);
            if !self.unmatched.starts_with(&self.encoded[..written]) {
                self.lossy = true;
                return;
            }
            self.unmatched.drain(..written);
            src = &src[read..];
            match result {
                EncoderResult::InputEmpty => break,
                EncoderResult::OutputFull => {}
                EncoderResult::Unmappable(_) => {
                    self.lossy = true;
                    return;
                }
            }
        }
        if last && !self.unmatched.is_empty() {
            self.lossy = true;
        }
    }
}

fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// A character the target encoding cannot represent.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("‘{ch}’ at line {line}, column {column} cannot be written in {encoding}")]
pub struct Unmappable {
    pub ch: char,
    pub line: u32,
    pub column: u32,
    pub encoding: TextEncoding,
}

impl Unmappable {
    fn at(text: &str, byte_offset: usize, ch: char, encoding: TextEncoding) -> Self {
        let before = &text[..byte_offset];
        let line_start = before.rfind('\n').map_or(0, |pos| pos + 1);
        Self {
            ch,
            line: u32::try_from(memchr::memchr_iter(b'\n', before.as_bytes()).count() + 1)
                .unwrap_or(u32::MAX),
            column: u32::try_from(before[line_start..].chars().count() + 1).unwrap_or(u32::MAX),
            encoding,
        }
    }
}

/// Encode `text` for writing in `format`, prefixed by the BOM when
/// `format.bom`. Never substitutes: a character the encoding lacks is an error.
pub fn encode(text: &str, format: TextFormat) -> Result<Cow<'_, [u8]>, Unmappable> {
    let encoding = format.encoding;
    let bom = if format.bom { encoding.bom() } else { None };
    if encoding.is_utf8() {
        return Ok(match bom {
            Some(bom) => {
                let mut out = Vec::with_capacity(bom.len() + text.len());
                out.extend_from_slice(bom);
                out.extend_from_slice(text.as_bytes());
                Cow::Owned(out)
            }
            None => Cow::Borrowed(text.as_bytes()),
        });
    }
    if encoding.is_utf16() {
        let little_endian = encoding == TextEncoding::UTF_16LE;
        let mut out = Vec::with_capacity(text.len() * 2 + 2);
        out.extend_from_slice(bom.unwrap_or_default());
        for unit in text.encode_utf16() {
            out.extend_from_slice(&if little_endian {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        return Ok(Cow::Owned(out));
    }
    let Some(whatwg) = encoding.whatwg() else {
        // True ISO-8859-1.
        if encoding_rs::mem::is_str_latin1(text) {
            return Ok(encoding_rs::mem::encode_latin1_lossy(text));
        }
        let (offset, ch) = text
            .char_indices()
            .find(|(_, ch)| u32::from(*ch) > 0xff)
            .unwrap_or((0, '\u{fffd}'));
        return Err(Unmappable::at(text, offset, ch, encoding));
    };
    if encoding.is_ascii_compatible() && text.is_ascii() {
        return Ok(Cow::Borrowed(text.as_bytes()));
    }
    let mut encoder = whatwg.new_encoder();
    let mut out = Vec::with_capacity(
        encoder
            .max_buffer_length_from_utf8_without_replacement(text.len())
            .unwrap_or(text.len() * 2 + 16),
    );
    let mut consumed = 0;
    loop {
        let (result, read) =
            encoder.encode_from_utf8_to_vec_without_replacement(&text[consumed..], &mut out, true);
        consumed += read;
        match result {
            EncoderResult::InputEmpty => return Ok(Cow::Owned(out)),
            EncoderResult::OutputFull => out.reserve(out.capacity().max(64)),
            EncoderResult::Unmappable(ch) => {
                return Err(Unmappable::at(text, consumed - ch.len_utf8(), ch, encoding));
            }
        }
    }
}

/// Whether writing `decoded` back in `format` reproduces `raw` exactly. Only
/// encodings with duplicate mappings can fail; the rest are skipped.
pub fn round_trips(raw: &[u8], decoded: &str, format: TextFormat) -> bool {
    if !format.encoding.may_not_round_trip() {
        return true;
    }
    matches!(encode(decoded, format), Ok(bytes) if bytes.as_ref() == raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(encoding: TextEncoding, bom: bool) -> TextFormat {
        TextFormat { encoding, bom }
    }

    #[test]
    fn every_byte_round_trips_in_windows_1252_and_latin1() {
        let raw: Vec<u8> = (0..=255).collect();
        for encoding in [TextEncoding::WINDOWS_1252, TextEncoding::ISO_8859_1] {
            let decoded = decode(&raw, fmt(encoding, false));
            assert!(!decoded.malformed, "{encoding}");
            let encoded = encode(&decoded.text, fmt(encoding, false)).unwrap();
            assert_eq!(encoded.as_ref(), raw.as_slice(), "{encoding}");
        }
    }

    #[test]
    fn latin1_and_windows_1252_differ_only_in_c1() {
        let latin1 = decode(b"\x80\xe9", fmt(TextEncoding::ISO_8859_1, false));
        let cp1252 = decode(b"\x80\xe9", fmt(TextEncoding::WINDOWS_1252, false));
        assert_eq!(latin1.text, "\u{80}é");
        assert_eq!(cp1252.text, "€é");
    }

    #[test]
    fn utf16_round_trips_with_bom_and_surrogates() {
        let text = "a\r\n😀é";
        for encoding in [TextEncoding::UTF_16LE, TextEncoding::UTF_16BE] {
            let bytes = encode(text, fmt(encoding, true)).unwrap();
            assert_eq!(&bytes[..2], encoding.bom().unwrap());
            let decoded = decode(&bytes, fmt(encoding, true));
            assert_eq!(decoded.text, text);
            assert!(!decoded.malformed);
        }
    }

    #[test]
    fn utf8_bom_is_stripped_and_restored() {
        let decoded = decode(b"\xEF\xBB\xBFhi", fmt(TextEncoding::UTF_8, true));
        assert_eq!(decoded.text, "hi");
        assert_eq!(
            encode("hi", fmt(TextEncoding::UTF_8, true))
                .unwrap()
                .as_ref(),
            b"\xEF\xBB\xBFhi"
        );
    }

    #[test]
    fn unmappable_reports_line_and_column() {
        let koi8 = TextEncoding::from_label("koi8-r").unwrap();
        let error = encode("ok\nпривет ü", fmt(koi8, false)).unwrap_err();
        assert_eq!((error.ch, error.line, error.column), ('ü', 2, 8));
        let error = encode("ÿĀ", fmt(TextEncoding::ISO_8859_1, false)).unwrap_err();
        assert_eq!((error.ch, error.line, error.column), ('Ā', 1, 2));
    }

    #[test]
    fn invalid_input_is_marked_malformed() {
        assert!(decode(b"caf\xe9", fmt(TextEncoding::UTF_8, false)).malformed);
        // Odd byte count in UTF-16.
        assert!(decode(b"a\x00b", fmt(TextEncoding::UTF_16LE, false)).malformed);
    }

    #[test]
    fn shift_jis_round_trip_detects_duplicate_mappings() {
        let sjis = TextEncoding::from_label("shift_jis").unwrap();
        let clean = encode("日本語テキスト", fmt(sjis, false))
            .unwrap()
            .into_owned();
        let decoded = decode(&clean, fmt(sjis, false));
        assert!(round_trips(&clean, &decoded.text, fmt(sjis, false)));
        // NEC row 13 0x8790 decodes to U+2252, which the encoder writes as
        // the JIS X 0208 0x81E0 — an unedited save would change the bytes.
        let nec = b"\x87\x90";
        let decoded = decode(nec, fmt(sjis, false));
        assert!(!decoded.malformed);
        assert_eq!(decoded.text, "\u{2252}");
        assert!(!round_trips(nec, &decoded.text, fmt(sjis, false)));
    }

    #[test]
    fn transcode_matches_whole_decode_across_chunk_edges() {
        // Multi-byte sequences and CRLF straddle the 64 KiB read size.
        let mut text = "x".repeat(TRANSCODE_CHUNK_BYTES - 1);
        text.push_str("Ω\r\nこんにちは\r\n");
        text.push_str(&"y".repeat(10));
        for encoding in [
            TextEncoding::UTF_16LE,
            TextEncoding::from_label("shift_jis").unwrap(),
            TextEncoding::UTF_8,
        ] {
            let format = fmt(encoding, encoding.is_utf16());
            let raw = encode(&text, format).unwrap().into_owned();
            let mut out = Vec::new();
            let stats =
                transcode_to_utf8(raw.as_slice(), &mut out, format, &CancellationToken::new())
                    .unwrap();
            assert_eq!(String::from_utf8(out).unwrap(), text, "{encoding}");
            assert!(!stats.malformed);
            assert_eq!(stats.line_endings.crlf, 2);
            assert_eq!(stats.utf8_len, text.len() as u64);
        }
    }

    #[test]
    fn transcode_honours_cancellation() {
        let token = CancellationToken::new();
        token.cancel();
        let result = transcode_to_utf8(
            &b"abc"[..],
            Vec::new(),
            fmt(TextEncoding::WINDOWS_1252, false),
            &token,
        );
        assert!(matches!(
            result.as_ref().map_err(|e| e.kind()),
            Err(ErrorKind::Cancelled)
        ));
    }
}
