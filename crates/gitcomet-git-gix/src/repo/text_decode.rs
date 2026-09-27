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

/// Git's marker file combines original stage bytes. Decode its side ranges
/// before treating the document as a single encoding, retaining the original
/// bytes and any manual text outside the markers.
pub(super) fn decode_mixed_conflict(
    payload: &gitcomet_core::conflict_session::ConflictPayload,
    formats: [Option<SideTextFormat>; 3],
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
    let mut out = String::with_capacity(bytes.len());
    let mut append = |range: std::ops::Range<usize>, encoding| -> Option<()> {
        let decoded = decode_bytes(&bytes[range], SideKind::Worktree, attributes, encoding);
        if !decoded.format.is_writable() {
            return None;
        }
        out.push_str(&decoded.text);
        Some(())
    };
    for segment in segments {
        match segment {
            ParsedConflictSegmentRanges::Text(range) => append(range, None)?,
            ParsedConflictSegmentRanges::Conflict(block) => {
                append(block.marker_start..block.ours.start, None)?;
                append(
                    block.ours.clone(),
                    formats[1].map(|format| format.format.encoding),
                )?;
                let last = if let Some(base) = block.base {
                    append(block.ours.end..base.start, None)?;
                    append(
                        base.clone(),
                        formats[0].map(|format| format.format.encoding),
                    )?;
                    base.end
                } else {
                    block.ours.end
                };
                append(last..block.theirs.start, None)?;
                append(
                    block.theirs.clone(),
                    formats[2].map(|format| format.format.encoding),
                )?;
                append(block.theirs.end..block.marker_end, None)?;
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
        Ok(FileDiffText::new_sources(path, old, new))
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
                    return Ok(
                        FileDiffTextSource::with_identity(path, identity).with_format(entry.format)
                    );
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
        Ok(FileDiffTextSource::with_identity(cache_path, identity).with_format(format))
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
        for base in [b"".as_slice(), b"||||||| base\ncaf\xe9\n"] {
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
                [Some(latin1), Some(latin1), Some(utf8)],
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
                &attributes
            )
            .is_none()
        );
    }
}
