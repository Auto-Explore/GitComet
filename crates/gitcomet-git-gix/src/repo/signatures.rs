//! Commit signature verification.
//!
//! gix parses the `gpgsig` header but cannot check it, so verification shells
//! out to `git log --format=%G?`. Two filters run first so the subprocess sees
//! as few commits as possible: a per-repo cache (a signature is immutable for
//! its oid) and a free gix pre-pass that drops unsigned commits outright.

use super::{GixRepo, with_object_cache};
use crate::util::{run_git_with_stdin_capture, validate_hex_commit_id};
use gitcomet_core::domain::{CommitId, CommitSignature, SignatureFormat, SignatureStatus};
use gitcomet_core::services::Result;
use gix::bstr::ByteSlice as _;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::time::Duration;

/// Verification is read-only and never prompts, so it must not inherit the
/// 300s default: a wedged `gpg-agent` would otherwise pin a repo-load worker
/// for five minutes.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// Cleared wholesale past this many entries, like [`super::DIVERGENCE_CACHE_LIMIT`].
const SIGNATURE_CACHE_LIMIT: usize = 4096;

const FIELD_SEP: u8 = 0x1f;
const RECORD_SEP: u8 = 0x1e;

/// One parsed `%H %G? %GS %GK` record. `format` is filled in by the caller from
/// the commit object, since `git log` does not report it.
struct VerifyRecord {
    oid: gix::ObjectId,
    status: Option<SignatureStatus>,
    signer: Option<Arc<str>>,
    key_id: Option<Arc<str>>,
}

fn optional_field(value: &str) -> Option<Arc<str>> {
    let value = value.trim();
    (!value.is_empty()).then(|| Arc::from(value))
}

fn parse_verify_output(output: &[u8]) -> Vec<VerifyRecord> {
    output
        .split(|byte| *byte == RECORD_SEP)
        .filter_map(|record| {
            let record = record.trim_ascii();
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
            Some(VerifyRecord {
                oid,
                status,
                signer,
                key_id,
            })
        })
        .collect()
}

impl GixRepo {
    /// Cached verdicts for `oids`, and the oids still needing verification.
    ///
    /// `None` in the cache means "checked, earns no badge" — it is a hit, not a
    /// miss, so an unsigned commit is never re-examined.
    fn cached_signatures(
        &self,
        oids: &[gix::ObjectId],
    ) -> (
        FxHashMap<gix::ObjectId, CommitSignature>,
        Vec<gix::ObjectId>,
    ) {
        let mut hits = FxHashMap::default();
        let mut misses = Vec::new();
        let cache = self
            .signature_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for oid in oids {
            match cache.get(oid) {
                Some(Some(signature)) => {
                    hits.insert(*oid, signature.clone());
                }
                Some(None) => {}
                None => misses.push(*oid),
            }
        }
        (hits, misses)
    }

    fn store_signatures(&self, entries: &[(gix::ObjectId, Option<CommitSignature>)]) {
        let mut cache = self
            .signature_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cache.len().saturating_add(entries.len()) > SIGNATURE_CACHE_LIMIT {
            cache.clear();
        }
        for (oid, signature) in entries {
            cache.insert(*oid, signature.clone());
        }
    }

    pub(in super::super) fn verify_commit_signatures_impl(
        &self,
        ids: &[CommitId],
    ) -> Result<Vec<(CommitId, CommitSignature)>> {
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
        let (mut verdicts, misses) = self.cached_signatures(&oids);

        if !misses.is_empty() {
            // Free pre-pass: read the signature header straight off the commit
            // object. Unsigned commits are settled here, with no subprocess.
            let repo = with_object_cache(&self.repo());
            let mut signed: Vec<(gix::ObjectId, SignatureFormat)> = Vec::new();
            let mut unsigned: Vec<(gix::ObjectId, Option<CommitSignature>)> = Vec::new();
            for oid in misses {
                match signature_format_of(&repo, &oid) {
                    Some(format) => signed.push((oid, format)),
                    None => unsigned.push((oid, None)),
                }
            }
            self.store_signatures(&unsigned);

            if !signed.is_empty() {
                let formats: FxHashMap<gix::ObjectId, SignatureFormat> =
                    signed.iter().copied().collect();
                let verified = self.run_verify_batch(&formats)?;
                self.store_signatures(&verified);
                for (oid, signature) in verified {
                    if let Some(signature) = signature {
                        verdicts.insert(oid, signature);
                    }
                }
            }
        }

        Ok(requested
            .into_iter()
            .filter_map(|(id, oid)| verdicts.remove(&oid).map(|signature| (id, signature)))
            .collect())
    }

    /// One `git log` for the whole batch. Oids go in on stdin, so there is no
    /// argv length limit and no need to chunk.
    fn run_verify_batch(
        &self,
        formats: &FxHashMap<gix::ObjectId, SignatureFormat>,
    ) -> Result<Vec<(gix::ObjectId, Option<CommitSignature>)>> {
        let mut stdin =
            Vec::with_capacity(formats.len() * (gix::hash::Kind::Sha1.len_in_hex() + 1));
        for oid in formats.keys() {
            stdin.extend_from_slice(oid.to_hex().to_string().as_bytes());
            stdin.push(b'\n');
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("log")
            .arg("--no-walk=unsorted")
            .arg("--stdin")
            .arg("--format=%H%x1f%G?%x1f%GS%x1f%GK%x1e");
        let output =
            run_git_with_stdin_capture(cmd, stdin, "git log --stdin", VERIFY_TIMEOUT, None)?;

        // Every requested oid gets an entry, so a commit git said nothing about
        // is cached as "no badge" instead of being re-verified forever.
        let mut results: Vec<(gix::ObjectId, Option<CommitSignature>)> =
            formats.keys().map(|oid| (*oid, None)).collect();
        let mut index: FxHashMap<gix::ObjectId, usize> = results
            .iter()
            .enumerate()
            .map(|(position, (oid, _))| (*oid, position))
            .collect();

        for record in parse_verify_output(&output) {
            let Some(position) = index.remove(&record.oid) else {
                continue;
            };
            let Some(status) = record.status else {
                continue;
            };
            let Some(format) = formats.get(&record.oid).copied() else {
                continue;
            };
            results[position].1 = Some(CommitSignature {
                status,
                format,
                signer: record.signer,
                key_id: record.key_id,
            });
        }

        Ok(results)
    }
}

/// The signature format of `oid`, or `None` when the commit carries no
/// signature. Reads an already-decoded header, so this costs one object read.
fn signature_format_of(repo: &gix::Repository, oid: &gix::ObjectId) -> Option<SignatureFormat> {
    let commit = repo.find_object(*oid).ok()?.try_into_commit().ok()?;
    let decoded = commit.decode().ok()?;
    let signature = decoded.extra_headers().pgp_signature()?;
    SignatureFormat::from_armor(signature)
}
