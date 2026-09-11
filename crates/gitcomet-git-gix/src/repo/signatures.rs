//! Commit signature verification.
//!
//! gix parses the `gpgsig` header but cannot check it, so verification shells
//! out to `git log --format=%G?`. Two filters run first so the subprocess sees
//! as few commits as possible: a per-repo cache of immutable signature formats
//! and a free gix pre-pass that drops unsigned commits outright. Signed verdicts
//! are always rechecked: keys, trust files and verifier configuration can change.

use super::{GixRepo, with_object_cache};
use crate::util::{run_git_with_stdin_capture, validate_hex_commit_id};
use gitcomet_core::domain::{CommitId, CommitSignature, SignatureFormat, SignatureStatus};
use gitcomet_core::error::{ErrorKind, GitFailureId};
use gitcomet_core::services::{CancellationToken, Result};
use gix::bstr::ByteSlice as _;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::time::Duration;

/// Verification is read-only and never prompts, so it must not inherit the
/// 300s default: a wedged `gpg-agent` must not pin its worker for five minutes.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// Evict cold metadata one entry at a time, never flush a page's cache hits.
pub(super) const SIGNATURE_CACHE_LIMIT: usize = 4096;
const VERIFY_BATCH_SIZE: usize = 16;

const FIELD_SEP: u8 = 0x1f;
const RECORD_SEP: u8 = 0x1e;

/// One parsed `%H %G? %GS %GK %GG` record. The caller obtains its signature
/// format from the commit object, since `git log` does not report it.
struct VerifyRecord {
    oid: gix::ObjectId,
    status: Option<SignatureStatus>,
    signer: Option<Arc<str>>,
    key_id: Option<Arc<str>>,
    output: String,
}

fn optional_field(value: &str) -> Option<Arc<str>> {
    let value = value.trim();
    (!value.is_empty()).then(|| Arc::from(value))
}

fn parse_verify_output(output: &[u8]) -> Vec<VerifyRecord> {
    output
        .split_inclusive(|byte| *byte == RECORD_SEP)
        .filter_map(|record| {
            // A timeout may leave an incomplete final record. Never accept it.
            let record = record.strip_suffix(&[RECORD_SEP])?.trim_ascii();
            if record.is_empty() {
                return None;
            }
            let mut fields = record.split(|byte| *byte == FIELD_SEP);
            let oid = gix::ObjectId::from_hex(fields.next()?.trim_ascii()).ok()?;
            let status = fields
                .next()
                .and_then(|code| code.trim_ascii().first().copied())
                .and_then(SignatureStatus::from_git_code);
            let signer = fields
                .next()
                .and_then(|f| optional_field(&f.to_str_lossy()));
            let key_id = fields
                .next()
                .and_then(|f| optional_field(&f.to_str_lossy()));
            let output = fields
                .next()
                .map(|f| f.to_str_lossy().into_owned())
                .unwrap_or_default();
            Some(VerifyRecord {
                oid,
                status,
                signer,
                key_id,
                output,
            })
        })
        .collect()
}

impl GixRepo {
    /// Cache only the immutable signature header, never a trust-dependent verdict.
    fn signature_formats(
        &self,
        oids: &[gix::ObjectId],
    ) -> FxHashMap<gix::ObjectId, SignatureFormat> {
        let repo = with_object_cache(&self.repo());
        let mut cache = self
            .signature_format_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        oids.iter()
            .filter_map(|oid| {
                let format = if let Some(format) = cache.get(oid) {
                    *format
                } else {
                    let format = signature_format_of(&repo, oid);
                    cache.put(*oid, format);
                    format
                };
                format.map(|format| (*oid, format))
            })
            .collect()
    }

    pub(in super::super) fn verify_commit_signatures_impl(
        &self,
        ids: &[CommitId],
    ) -> Result<Vec<(CommitId, CommitSignature)>> {
        self.verify_commit_signatures_cancellable_impl(ids, None)
    }

    pub(in super::super) fn verify_commit_signatures_cancellable_impl(
        &self,
        ids: &[CommitId],
        cancellation: Option<&CancellationToken>,
    ) -> Result<Vec<(CommitId, CommitSignature)>> {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        for id in ids {
            validate_hex_commit_id(id)?;
        }

        // Preserve input order, and keep the caller's `CommitId` spelling so the
        // returned keys match what the UI already holds.
        let mut requested: Vec<(CommitId, gix::ObjectId)> = Vec::with_capacity(ids.len());
        for id in ids {
            if let Ok(oid) = gix::ObjectId::from_hex(id.as_ref().as_bytes()) {
                requested.push((id.clone(), oid));
            }
        }
        if requested.is_empty() {
            return Ok(Vec::new());
        }

        let oids: Vec<gix::ObjectId> = requested.iter().map(|(_, oid)| *oid).collect();
        let formats = self.signature_formats(&oids);
        let mut verdicts = FxHashMap::default();
        let signed: Vec<_> = requested
            .iter()
            .filter_map(|(_, oid)| formats.get(oid).map(|format| (*oid, *format)))
            .collect();
        for chunk in signed.chunks(VERIFY_BATCH_SIZE) {
            if let Some(cancellation) = cancellation {
                cancellation.check_cancelled()?;
            }
            let formats = chunk.iter().copied().collect();
            let verified = match self.run_verify_batch(&formats, VERIFY_TIMEOUT, cancellation) {
                Ok(verified) => verified,
                Err(error) => {
                    let ErrorKind::Git(failure) = error.kind() else {
                        return Err(error);
                    };
                    if failure.id() != GitFailureId::Timeout {
                        return Err(error);
                    }
                    // Keep complete records from the timed-out process, then
                    // retry only unfinished commits individually. A stalled
                    // verifier must not erase progress for the rest of a page.
                    let mut verified = signatures_from_output(&formats, failure.stdout());
                    let completed: std::collections::HashSet<_> =
                        verified.iter().map(|(oid, _)| *oid).collect();
                    for (oid, format) in chunk.iter().filter(|(oid, _)| !completed.contains(oid)) {
                        let single = [(*oid, *format)].into_iter().collect();
                        match self.run_verify_batch(&single, Duration::from_secs(2), cancellation) {
                            Ok(result) => verified.extend(result),
                            Err(error) if matches!(error.kind(), ErrorKind::Git(failure) if failure.id() == GitFailureId::Timeout) =>
                                {}
                            Err(error) => return Err(error),
                        }
                    }
                    verified
                }
            };
            verdicts.extend(
                verified
                    .into_iter()
                    .filter_map(|(oid, signature)| signature.map(|signature| (oid, signature))),
            );
        }

        Ok(requested
            .into_iter()
            .filter_map(|(id, oid)| verdicts.remove(&oid).map(|signature| (id, signature)))
            .collect())
    }

    /// One small `git log` batch; stdin avoids argv limits.
    fn run_verify_batch(
        &self,
        formats: &FxHashMap<gix::ObjectId, SignatureFormat>,
        timeout: Duration,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Vec<(gix::ObjectId, Option<CommitSignature>)>> {
        let mut stdin =
            Vec::with_capacity(formats.len() * (gix::hash::Kind::Sha1.len_in_hex() + 1));
        for oid in formats.keys() {
            stdin.extend_from_slice(oid.to_hex().to_string().as_bytes());
            stdin.push(b'\n');
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.env("GIT_FLUSH", "1")
            .env("LC_ALL", "C")
            .arg("log")
            .arg("--encoding=UTF-8")
            // `log.showSignature=true` makes git print gpg's prose to *stdout*
            // ahead of the records, which would swallow the first oid and make
            // the whole batch parse as "no badge".
            .arg("--no-show-signature")
            .arg("--no-walk=unsorted")
            .arg("--stdin")
            .arg("--format=%H%x1f%G?%x1f%GS%x1f%GK%x1f%GG%x1e");
        let output =
            run_git_with_stdin_capture(cmd, stdin, "git log --stdin", timeout, cancellation)?;

        Ok(signatures_from_output(formats, &output))
    }
}

/// Include only complete records; `None` means Git actually reported no badge.
fn signatures_from_output(
    formats: &FxHashMap<gix::ObjectId, SignatureFormat>,
    output: &[u8],
) -> Vec<(gix::ObjectId, Option<CommitSignature>)> {
    parse_verify_output(output)
        .into_iter()
        .filter_map(|record| {
            let format = *formats.get(&record.oid)?;
            let status = record.status.filter(|status| {
                // Git's SSH parser starts at B even when the helper cannot verify.
                format != SignatureFormat::Ssh
                    || *status != SignatureStatus::Bad
                    || record
                        .output
                        .lines()
                        .any(|line| line == "Could not verify signature.")
            });
            Some((
                record.oid,
                status.map(|status| CommitSignature {
                    status,
                    format,
                    signer: record.signer,
                    key_id: record.key_id,
                }),
            ))
        })
        .collect()
}

/// The signature format of `oid`, or `None` when the commit carries no
/// signature. Reads an already-decoded header, so this costs one object read.
fn signature_format_of(repo: &gix::Repository, oid: &gix::ObjectId) -> Option<SignatureFormat> {
    let commit = repo.find_object(*oid).ok()?.try_into_commit().ok()?;
    let decoded = commit.decode().ok()?;
    let signature = decoded.extra_headers().pgp_signature()?;
    SignatureFormat::from_armor(signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cached_page_does_not_evict_itself_by_counting_hits_as_new_entries() {
        let dir = tempfile::tempdir().unwrap();
        let git = gix::init(dir.path()).unwrap();
        let repo = GixRepo::new(dir.path().to_path_buf(), git.into_sync());
        let ids: Vec<_> = (0..3000)
            .map(|ix| gix::ObjectId::from_hex(format!("{ix:040x}").as_bytes()).unwrap())
            .collect();
        for oid in &ids {
            repo.signature_format_cache
                .lock()
                .unwrap()
                .put(*oid, Some(SignatureFormat::Ssh));
        }
        assert_eq!(
            repo.signature_formats(&ids).len(),
            ids.len(),
            "cache hits must not trigger wholesale eviction"
        );
        assert_eq!(repo.signature_formats(&ids).len(), ids.len());
    }

    #[test]
    fn partial_verifier_output_keeps_completed_no_badge_records_only() {
        let first = gix::ObjectId::from_hex(b"1111111111111111111111111111111111111111").unwrap();
        let second = gix::ObjectId::from_hex(b"2222222222222222222222222222222222222222").unwrap();
        let formats = [
            (first, SignatureFormat::OpenPgp),
            (second, SignatureFormat::OpenPgp),
        ]
        .into_iter()
        .collect();
        let output = format!("{first}\x1fE\x1f\x1f\x1f\x1e\n{second}\x1fG\x1fSigner\x1fKey");
        let records = signatures_from_output(&formats, output.as_bytes());
        assert_eq!(
            records,
            vec![(first, None)],
            "the incomplete record must be retried, and E must count as completed"
        );
    }

    #[test]
    fn format_cache_evicts_cold_entries_without_flushing_hot_ones() {
        let dir = tempfile::tempdir().unwrap();
        let git = gix::init(dir.path()).unwrap();
        let repo = GixRepo::new(dir.path().to_path_buf(), git.into_sync());
        let oid = |ix| gix::ObjectId::from_hex(format!("{ix:040x}").as_bytes()).unwrap();
        {
            let mut cache = repo.signature_format_cache.lock().unwrap();
            for ix in 0..SIGNATURE_CACHE_LIMIT {
                cache.put(oid(ix), Some(SignatureFormat::Ssh));
            }
        }
        assert_eq!(repo.signature_formats(&[oid(0)]).len(), 1);
        // A new unsigned object misses and occupies one slot.
        repo.signature_formats(&[oid(SIGNATURE_CACHE_LIMIT)]);
        let cache = repo.signature_format_cache.lock().unwrap();
        assert_eq!(cache.len(), SIGNATURE_CACHE_LIMIT);
        assert!(cache.contains(&oid(0)), "the hot entry must survive");
        assert!(
            !cache.contains(&oid(1)),
            "only the oldest entry should be evicted"
        );
    }
}
