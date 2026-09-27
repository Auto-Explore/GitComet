//! Turns git-side file content into UTF-8 for the views. Sources that are
//! already plain UTF-8 pass through untouched; anything else is transcoded
//! once into a content-addressed UTF-8 cache file.

use super::diff::{io_err_to_error, persist_worktree_git_cache_file};
use super::{DiskFileStamp, GixRepo, TEMP_FILE_MEMO_LIMIT};
use gitcomet_core::domain::{DiffSectionFormats, FileDiffText, FileDiffTextSource};
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

    /// Per-side formats for a single-file patch, read from the same sources
    /// (and memo) as the file view, so both views decode alike.
    pub(super) fn patch_section_formats(
        &self,
        file_text: Option<&FileDiffText>,
    ) -> DiffSectionFormats {
        let side = |source: Option<&FileDiffTextSource>| {
            source.and_then(|source| source.format).map(|format| {
                gitcomet_core::text_format::TextFormat {
                    bom: false,
                    ..format.format
                }
            })
        };
        let old = file_text.and_then(|text| side(text.old_source.as_ref()));
        let new = file_text.and_then(|text| side(text.new_source.as_ref()));
        match (old, new) {
            (None, None) => DiffSectionFormats::UTF_8,
            (old, new) => DiffSectionFormats {
                old: old.or(new).unwrap_or_default(),
                new: new.or(old).unwrap_or_default(),
            },
        }
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
    attributes.hash(&mut hasher);
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
