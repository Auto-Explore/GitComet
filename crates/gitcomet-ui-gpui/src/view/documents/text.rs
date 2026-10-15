use super::disk::PendingWrite;
use crate::kit::text_model::TextModelSnapshot;
use gitcomet_core::text_format::SideTextFormat;
use gpui::SharedString;
use rustc_hash::FxHasher;
use std::sync::Arc;

/// Why text cannot be written in its file's encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::view) enum SaveEncodeError {
    /// The file did not read back byte for byte, so writing it changes bytes.
    Unwritable(gitcomet_core::text_format::TextEncoding),
    /// A character has no bytes in the encoding.
    Unmappable(gitcomet_core::text_format::Unmappable),
}

impl std::fmt::Display for SaveEncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unwritable(encoding) => write!(
                f,
                "It can't be written in {encoding} without changing bytes it already has."
            ),
            Self::Unmappable(unmappable) => write!(f, "{unmappable}."),
        }
    }
}

/// The bytes to write for `text` in the file's encoding, and the same bytes as
/// the disk check will see them.
pub(in crate::view) fn encode_for_save(
    text: SharedString,
    format: Option<SideTextFormat>,
) -> Result<(gitcomet_state::msg::ContentBytes, PendingWrite), SaveEncodeError> {
    if let Some(format) = format
        && !format.is_writable()
    {
        return Err(SaveEncodeError::Unwritable(format.format.encoding));
    }
    let Some(format) = format.filter(|format| !format.format.is_plain_utf8()) else {
        return Ok((text.to_string().into(), PendingWrite::Text(text)));
    };
    let bytes: Arc<[u8]> = Arc::from(
        gitcomet_core::text_format::encode(&text, format.format)
            .map_err(SaveEncodeError::Unmappable)?
            .as_ref(),
    );
    Ok((Arc::clone(&bytes).into(), PendingWrite::Encoded(bytes)))
}

/// Identity of the text a save or a dirty check was computed over.
///
/// A hash rather than the text: the comparison runs on every keystroke, and
/// keeping a second copy of the file around to `==` against it would double the
/// buffer's memory for a question that a `u64` answers.
///
/// Fed the raw bytes rather than `str::hash`, which writes a `0xff` terminator
/// per call. Chunk boundaries are an artifact of the rope's edit history — the
/// same text reached by typing a character and deleting it again is split
/// differently — so hashing chunk-wise made a byte-identical buffer read as
/// permanently modified.
pub(in crate::view) fn file_editor_text_fingerprint(snapshot: &TextModelSnapshot) -> u64 {
    use std::hash::Hasher;

    #[cfg(test)]
    TEXT_FINGERPRINTS.with(|count| count.set(count.get() + 1));

    let mut hasher = FxHasher::default();
    hasher.write_usize(snapshot.len());
    // Chunk-wise, so a large file is never flattened into one string just to be
    // fingerprinted. `TextModelSnapshot::rope()` is an `Arc` bump.
    for chunk in snapshot.rope().chunks() {
        hasher.write(chunk.as_bytes());
    }
    hasher.finish()
}

#[cfg(test)]
thread_local! {
    static TEXT_FINGERPRINTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Buffers fingerprinted on this thread since the last call.
#[cfg(test)]
pub(in crate::view) fn take_text_fingerprints_for_tests() -> usize {
    TEXT_FINGERPRINTS.with(|count| count.replace(0))
}

#[cfg(test)]
mod encode_for_save_tests {
    use super::*;
    use gitcomet_core::text_format::{
        FormatSource, LineEndingStats, SideTextFormat, TextEncoding, TextFormat,
    };

    fn read_as(encoding: TextEncoding, bom: bool) -> SideTextFormat {
        SideTextFormat {
            format: TextFormat { encoding, bom },
            source: FormatSource::Detected { confident: true },
            binary: false,
            malformed: false,
            lossy: false,
            line_endings: LineEndingStats::default(),
        }
    }

    #[test]
    fn utf8_is_written_as_the_buffer_text() {
        for format in [None, Some(read_as(TextEncoding::UTF_8, false))] {
            let (bytes, pending) = encode_for_save("é\n".into(), format).unwrap();
            assert_eq!(bytes.as_bytes(), "é\n".as_bytes());
            assert!(matches!(pending, PendingWrite::Text(_)));
        }
    }

    #[test]
    fn utf8_fast_path_rejects_unwritable_formats() {
        for bom in [false, true] {
            let format = read_as(TextEncoding::UTF_8, bom);
            for unwritable in [
                SideTextFormat {
                    malformed: true,
                    ..format
                },
                SideTextFormat {
                    lossy: true,
                    ..format
                },
                SideTextFormat {
                    binary: true,
                    ..format
                },
            ] {
                assert!(encode_for_save("caf�\n".into(), Some(unwritable)).is_err());
            }
        }
    }

    #[test]
    fn other_encodings_write_their_own_bytes_and_bom() {
        let (bytes, pending) = encode_for_save(
            "café\n".into(),
            Some(read_as(TextEncoding::WINDOWS_1252, false)),
        )
        .unwrap();
        assert_eq!(bytes.as_bytes(), b"caf\xe9\n");
        assert_eq!(pending, PendingWrite::Encoded(Arc::from(&b"caf\xe9\n"[..])));

        let (bytes, _) =
            encode_for_save("a".into(), Some(read_as(TextEncoding::UTF_16LE, true))).unwrap();
        assert_eq!(bytes.as_bytes(), b"\xff\xfea\x00");
        let (bytes, _) =
            encode_for_save("a".into(), Some(read_as(TextEncoding::UTF_8, true))).unwrap();
        assert_eq!(bytes.as_bytes(), b"\xef\xbb\xbfa");
    }

    #[test]
    fn unwritable_text_or_lossy_files_are_refused() {
        let error = encode_for_save(
            "ok\nĀ".into(),
            Some(read_as(TextEncoding::WINDOWS_1252, false)),
        )
        .unwrap_err();
        assert!(error.to_string().contains("line 2, column 1"), "{error}");

        let mut lossy = read_as(TextEncoding::from_label("shift_jis").unwrap(), false);
        lossy.lossy = true;
        assert!(encode_for_save("x".into(), Some(lossy)).is_err());
    }
}
