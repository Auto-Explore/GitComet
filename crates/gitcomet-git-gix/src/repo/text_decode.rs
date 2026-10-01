//! Turns git-side file content into UTF-8 for the views. Sources that are
//! already plain UTF-8 pass through untouched; anything else is transcoded
//! once into a content-addressed UTF-8 cache file.

use super::diff::{io_err_to_error, persist_worktree_git_cache_file};
use super::{DiskFileStamp, GixRepo, TEMP_FILE_MEMO_LIMIT};
use gitcomet_core::domain::{FileDiffText, FileDiffTextSource};
use gitcomet_core::services::{CancellationToken, Result};
use gitcomet_core::text_format::{
    ContentSniffer, LineEndingStats, SideKind, SideTextFormat, TextAttributes, TextEncoding,
    transcode_to_utf8,
};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SNIFF_READ_BYTES: usize = 64 * 1024;

/// Pool uncertain legacy samples across stages. UTF-8, BOMs and explicit
/// encoding policies remain per-side; a short edit cannot independently
/// change a legacy file's encoding and trigger an implicit UTF-8 conversion.
pub(super) fn decode_conflict_stages(
    mut bytes: [Option<Vec<u8>>; 3],
    attributes: &TextAttributes,
    encoding: Option<TextEncoding>,
) -> [(
    gitcomet_core::conflict_session::ConflictPayload,
    Option<SideTextFormat>,
); 3] {
    use gitcomet_core::text_format::{ContentSniff, FormatSource};
    let mut pooled = ContentSniffer::new();
    let uncertain = bytes.each_ref().map(|bytes| {
        let Some(bytes) = bytes else {
            return false;
        };
        let format = ContentSniff::of(bytes).resolve(SideKind::GitInternal, attributes, encoding);
        let uncertain = matches!(format.source, FormatSource::Detected { confident: false });
        if uncertain {
            pooled.feed(bytes);
            pooled.feed(b"\n");
        }
        uncertain
    });
    let common = pooled
        .finish()
        .resolve(SideKind::GitInternal, attributes, encoding);
    std::array::from_fn(|ix| {
        let (payload, mut format) = gitcomet_core::conflict_session::ConflictPayload::decode(
            bytes[ix].take().map(Arc::from),
            None,
            SideKind::GitInternal,
            attributes,
            if uncertain[ix] {
                Some(common.format.encoding)
            } else {
                encoding
            },
        );
        if uncertain[ix]
            && let Some(format) = format.as_mut()
        {
            format.source = common.source;
        }
        (payload, format)
    })
}

/// Git's marker file combines original stage bytes. Decode its side ranges
/// before treating the document as a single encoding, retaining the original
/// bytes and any manual text outside the markers.
pub(super) fn decode_mixed_conflict(
    payload: &gitcomet_core::conflict_session::ConflictPayload,
    formats: [Option<SideTextFormat>; 3],
    stages: [&gitcomet_core::conflict_session::ConflictPayload; 3],
    attributes: &TextAttributes,
) -> Option<gitcomet_core::conflict_session::ConflictPayload> {
    use gitcomet_core::conflict_session::{
        ConflictPayload, ParsedConflictSegmentRanges, parse_conflict_marker_ranges_bytes,
    };
    use gitcomet_core::text_format::decode_bytes;

    let bytes = payload.as_bytes()?;
    let segments = parse_conflict_marker_ranges_bytes(bytes);
    if !segments
        .iter()
        .any(|segment| matches!(segment, ParsedConflictSegmentRanges::Conflict(_)))
    {
        return None;
    }
    let mut context_lines = rustc_hash::FxHashMap::default();
    for (stage, format) in stages.into_iter().zip(formats) {
        let (Some(bytes), Some(format)) = (stage.as_bytes(), format) else {
            continue;
        };
        for line in bytes
            .split_inclusive(|&byte| byte == b'\n')
            .filter(|line| !line.is_ascii())
        {
            context_lines
                .entry(line)
                .and_modify(|known| {
                    if *known != Some(format.format.encoding) {
                        *known = None;
                    }
                })
                .or_insert(Some(format.format.encoding));
        }
    }
    let context_encoding = |range: &std::ops::Range<usize>| {
        let line = &bytes[range.clone()];
        // Unchanged context keeps its stage's encoding even if its bytes also
        // happen to form valid UTF-8. Conflicting evidence must remain read-only.
        if let Some(encoding) = context_lines.get(line) {
            return *encoding;
        }
        // Manual edits may already be UTF-8. Otherwise use the known legacy
        // encoding, never a fresh guess from a tiny range.
        if std::str::from_utf8(line).is_ok() {
            return Some(TextEncoding::UTF_8);
        }
        let mut candidates = formats
            .into_iter()
            .flatten()
            .map(|format| format.format.encoding)
            .filter(|encoding| !encoding.is_utf8());
        let first = candidates.next()?;
        candidates
            .all(|encoding| encoding == first)
            .then_some(first)
    };
    let mut out = String::with_capacity(bytes.len());
    let mut append = |range: std::ops::Range<usize>, encoding| -> Option<()> {
        if range.is_empty() {
            return Some(());
        }
        let decoded = decode_bytes(
            &bytes[range],
            SideKind::Worktree,
            attributes,
            Some(encoding?),
        );
        if !decoded.format.is_writable() {
            return None;
        }
        out.push_str(&decoded.text);
        Some(())
    };
    for segment in segments {
        match segment {
            ParsedConflictSegmentRanges::Text(range) => {
                let mut start = range.start;
                for line in bytes[range].split_inclusive(|&byte| byte == b'\n') {
                    let end = start + line.len();
                    let encoding = context_encoding(&(start..end))?;
                    append(start..end, Some(encoding))?;
                    start = end;
                }
            }
            ParsedConflictSegmentRanges::Conflict(block) => {
                append(
                    block.marker_start..block.ours.start,
                    Some(TextEncoding::UTF_8),
                )?;
                append(
                    block.ours.clone(),
                    formats[1].map(|format| format.format.encoding),
                )?;
                let last = if let Some(base) = block.base {
                    append(block.ours.end..base.start, Some(TextEncoding::UTF_8))?;
                    append(
                        base.clone(),
                        formats[0].map(|format| format.format.encoding),
                    )?;
                    base.end
                } else {
                    block.ours.end
                };
                append(last..block.theirs.start, Some(TextEncoding::UTF_8))?;
                append(
                    block.theirs.clone(),
                    formats[2].map(|format| format.format.encoding),
                )?;
                append(
                    block.theirs.end..block.marker_end,
                    Some(TextEncoding::UTF_8),
                )?;
            }
        }
    }
    let raw = match payload {
        ConflictPayload::EncodedText { bytes, .. } | ConflictPayload::Binary(bytes) => {
            Arc::clone(bytes)
        }
        _ => Arc::from(bytes),
    };
    Some(ConflictPayload::EncodedText {
        text: out.into(),
        bytes: raw,
    })
}

/// How a content-addressed source reads under given attributes and choice.
/// Identities are content hashes, so an entry never goes stale; only a
/// transcoded file can disappear or be tampered with, which its stamp catches.
#[derive(Clone, Debug)]
pub(super) struct TextFormatMemoEntry {
    format: SideTextFormat,
    transcoded: Option<(PathBuf, Arc<str>, Option<DiskFileStamp>)>,
}

impl GixRepo {
    /// Both sides of a file diff, pointed at UTF-8 content and tagged with how
    /// they were read.
    pub(super) fn decode_file_diff_text(
        &self,
        text: FileDiffText,
        attributes: &TextAttributes,
        encoding: Option<TextEncoding>,
        cancellation: &CancellationToken,
    ) -> Result<FileDiffText> {
        let path = text.path.clone();
        let decode = |source: Option<FileDiffTextSource>| {
            source
                .map(|source| {
                    self.decode_file_diff_source(source, &path, attributes, encoding, cancellation)
                })
                .transpose()
        };
        let old = decode(text.old_source)?;
        let new = decode(text.new_source)?;
        Ok(FileDiffText::new_sources(path, old, new)
            .with_large_sides(text.old_large, text.new_large))
    }

    fn decode_file_diff_source(
        &self,
        source: FileDiffTextSource,
        logical_path: &Path,
        attributes: &TextAttributes,
        encoding: Option<TextEncoding>,
        cancellation: &CancellationToken,
    ) -> Result<FileDiffTextSource> {
        cancellation.check_cancelled()?;
        let key = memo_key(&source.identity, attributes, encoding);
        if let Some(entry) = self.text_format_memo_get(key) {
            match entry.transcoded {
                None => return Ok(source.with_format(entry.format)),
                Some((path, identity, Some(stamp)))
                    if DiskFileStamp::read(&path) == Some(stamp) =>
                {
                    return Ok(FileDiffTextSource::with_identity(path, identity)
                        .with_format(entry.format)
                        .with_raw_path(source.path.clone()));
                }
                Some(_) => {}
            }
        }

        let (sniff, _) = sniff_file(&source.path, cancellation)?;
        let mut format = sniff.resolve(SideKind::GitInternal, attributes, encoding);
        if format.binary || (format.format.is_plain_utf8() && sniff.utf8_valid) {
            self.text_format_memo_put(
                key,
                TextFormatMemoEntry {
                    format,
                    transcoded: None,
                },
            );
            return Ok(source.with_format(format));
        }

        let file = std::fs::File::open(&source.path).map_err(io_err_to_error)?;
        let mut tmp_file =
            tempfile::NamedTempFile::new_in(std::env::temp_dir()).map_err(io_err_to_error)?;
        let stats = transcode_to_utf8(
            std::io::BufReader::with_capacity(SNIFF_READ_BYTES, file),
            std::io::BufWriter::new(tmp_file.as_file_mut()),
            format.format,
            cancellation,
        )?;
        format.malformed = stats.malformed;
        format.lossy = stats.lossy;
        if !format.format.encoding.is_ascii_compatible() {
            format.line_endings = stats.line_endings;
        }
        let identity: Arc<str> = Arc::from(format!(
            "{}@{}{}",
            source.identity,
            format.format.encoding.name(),
            if format.format.bom { "+bom" } else { "" }
        ));
        let cache_path = utf8_cache_path(logical_path, &identity);
        let created = persist_worktree_git_cache_file(tmp_file, &cache_path)?;
        // A file this call created is private to it (0600, content-addressed,
        // never rewritten), so its fresh timestamps cannot hide a later write.
        let stamp = if created {
            DiskFileStamp::read(&cache_path)
        } else {
            DiskFileStamp::read_for_verification_memo(&cache_path)
        };
        self.text_format_memo_put(
            key,
            TextFormatMemoEntry {
                format,
                transcoded: Some((cache_path.clone(), Arc::clone(&identity), stamp)),
            },
        );
        Ok(FileDiffTextSource::with_identity(cache_path, identity)
            .with_format(format)
            .with_raw_path(source.path.clone()))
    }

    fn text_format_memo_get(&self, key: u64) -> Option<TextFormatMemoEntry> {
        self.text_format_memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
            .cloned()
    }

    fn text_format_memo_put(&self, key: u64, entry: TextFormatMemoEntry) {
        let mut memo = self
            .text_format_memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if memo.len() >= TEMP_FILE_MEMO_LIMIT {
            memo.clear();
        }
        memo.insert(key, entry);
    }
}

fn memo_key(identity: &str, attributes: &TextAttributes, encoding: Option<TextEncoding>) -> u64 {
    let mut hasher = FxHasher::default();
    identity.hash(&mut hasher);
    attributes.decoding_encodings().hash(&mut hasher);
    encoding.hash(&mut hasher);
    hasher.finish()
}

/// Stream `path` through a sniffer.
pub(super) fn sniff_file(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<(gitcomet_core::text_format::ContentSniff, LineEndingStats)> {
    let mut file = std::fs::File::open(path).map_err(io_err_to_error)?;
    let mut sniffer = ContentSniffer::new();
    let mut buf = vec![0u8; SNIFF_READ_BYTES];
    loop {
        cancellation.check_cancelled()?;
        let read = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(io_err_to_error(error)),
        };
        sniffer.feed(&buf[..read]);
    }
    let sniff = sniffer.finish();
    let line_endings = sniff.line_endings;
    Ok((sniff, line_endings))
}

fn utf8_cache_path(logical_path: &Path, identity: &str) -> PathBuf {
    let mut hasher = FxHasher::default();
    identity.hash(&mut hasher);
    let suffix = logical_path
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty())
        .map(|ext| format!(".{ext}"))
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "gitcomet-diff-utf8-{:016x}{suffix}",
        hasher.finish()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::conflict_session::ConflictPayload;
    use gitcomet_core::text_format::decode_bytes;

    #[test]
    fn review_mixed_conflict_context_uses_known_stage_encoding() {
        use gitcomet_core::text_format::{TextFormat, encode};
        let attributes = TextAttributes::default();
        let encoding = TextEncoding::from_label("windows-1250").unwrap();
        let format = TextFormat {
            encoding,
            bom: false,
        };
        // ą alone is guessed incorrectly; Âą's legacy bytes also form valid
        // UTF-8, so unchanged context must win over UTF-8 sniffing as well.
        for context_text in ["ą\n", "Âą\n"] {
            let context = encode(context_text, format).unwrap();
            let ours =
                decode_bytes(&context, SideKind::GitInternal, &attributes, Some(encoding)).format;
            let utf8 = SideTextFormat::utf8(LineEndingStats::default());
            let raw = [
                "manual 日本語\n".as_bytes(),
                context.as_ref(),
                b"<<<<<<< ours\nlocal\n=======\n",
                "日本語\n>>>>>>> theirs\n".as_bytes(),
            ]
            .concat();
            let stage = ConflictPayload::Binary(context.as_ref().into());
            let utf8_stage = ConflictPayload::Text("日本語\n".into());
            let decoded = decode_mixed_conflict(
                &ConflictPayload::Binary(raw.into()),
                [Some(ours), Some(ours), Some(utf8)],
                [&stage, &stage, &utf8_stage],
                &attributes,
            )
            .expect("known encodings decode the marker file");
            assert!(
                decoded
                    .as_text()
                    .unwrap()
                    .starts_with(&format!("manual 日本語\n{context_text}")),
                "{:?}",
                decoded.as_text()
            );
        }
    }

    #[test]
    fn mixed_markers_preserve_manual_text_and_original_bytes() {
        let attributes = TextAttributes::default();
        let latin1 = decode_bytes(
            b"caf\xe9\n",
            SideKind::GitInternal,
            &attributes,
            Some(TextEncoding::WINDOWS_1252),
        )
        .format;
        let utf8 = SideTextFormat::utf8(LineEndingStats::default());
        let latin1_stage = ConflictPayload::Binary(Arc::from(b"caf\xe9\n".as_slice()));
        let utf8_stage = ConflictPayload::Text("café remote 日本語\n".into());
        for base in [
            b"".as_slice(),
            b"||||||| base\ncaf\xe9\n",
            b"||||||| base\n",
        ] {
            let raw: Arc<[u8]> = [
                "my manual edit 日本語\n<<<<<<< ours\n".as_bytes(),
                b"caf\xe9 local\n",
                base,
                b"=======\n",
                "café remote 日本語\n>>>>>>> theirs\nmanual tail\n".as_bytes(),
            ]
            .concat()
            .into();
            let payload = ConflictPayload::Binary(Arc::clone(&raw));
            let decoded = decode_mixed_conflict(
                &payload,
                [
                    (base != b"||||||| base\n").then_some(latin1),
                    Some(latin1),
                    Some(utf8),
                ],
                [&latin1_stage, &latin1_stage, &utf8_stage],
                &attributes,
            )
            .unwrap();
            assert!(
                decoded
                    .as_text()
                    .unwrap()
                    .starts_with("my manual edit 日本語\n")
            );
            assert!(decoded.as_text().unwrap().contains("café local\n"));
            assert!(decoded.as_text().unwrap().contains("café remote 日本語\n"));
            assert!(decoded.as_text().unwrap().ends_with("manual tail\n"));
            let ConflictPayload::EncodedText { bytes, .. } = decoded else {
                panic!("keep original bytes")
            };
            assert!(Arc::ptr_eq(&raw, &bytes));
        }
        let resolved = ConflictPayload::Text("already resolved 日本語\n".into());
        assert!(
            decode_mixed_conflict(
                &resolved,
                [Some(latin1), Some(latin1), Some(utf8)],
                [&latin1_stage, &latin1_stage, &utf8_stage],
                &attributes
            )
            .is_none()
        );
    }
}
