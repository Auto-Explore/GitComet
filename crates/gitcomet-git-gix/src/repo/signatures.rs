//! Commit signature verification.
//!
//! gix parses the `gpgsig` header but cannot check it, so verification shells
//! out to `git log --format=%G?`. Two filters run first so the subprocess sees
//! as few commits as possible: a per-repo cache of immutable signature formats
//! and a free gix pre-pass that drops unsigned commits outright. Signed verdicts
//! are always rechecked: keys, trust files and verifier configuration can change.

use super::{GixRepo, with_object_cache};
use crate::util::{
    git_command_failed_error, path_buf_from_git_bytes, run_git_preview_output,
    run_git_with_stdin_capture, validate_hex_commit_id,
};
use gitcomet_core::domain::{
    CommitId, CommitSignature, SignatureFormat, SignatureFormats, SignatureStatus,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::{CancellationToken, Result};
use gix::bstr::ByteSlice as _;
use rustc_hash::FxHashMap;
use std::ffi::OsString;
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
const VERIFIER_PROGRAMS: [(&str, &str); 3] = [
    ("gpg.openpgp.program", "gpg"),
    ("gpg.x509.program", "gpgsm"),
    ("gpg.ssh.program", "ssh-keygen"),
];

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

/// Records of exactly the five requested fields, at most one per commit.
///
/// `%GS` and `%GG` repeat the verifier's text, which quotes the signer's key
/// user id. Should a separator byte ever reach it, the record splits: an
/// extra field, or a forged record for another commit. Git prints exactly one
/// record per requested commit, so either sign voids what it touches.
fn parse_verify_output(output: &[u8]) -> Vec<VerifyRecord> {
    let mut records: Vec<VerifyRecord> = output
        .split_inclusive(|byte| *byte == RECORD_SEP)
        .filter_map(|record| {
            // A timeout may leave an incomplete final record. Never accept it.
            let record = record.strip_suffix(&[RECORD_SEP])?.trim_ascii();
            if record.is_empty() {
                return None;
            }
            let fields: Vec<&[u8]> = record.split(|byte| *byte == FIELD_SEP).collect();
            let [oid, code, signer, key_id, output] = fields.as_slice() else {
                return None;
            };
            let oid = gix::ObjectId::from_hex(oid.trim_ascii()).ok()?;
            let status = code
                .trim_ascii()
                .first()
                .copied()
                .and_then(SignatureStatus::from_git_code);
            Some(VerifyRecord {
                oid,
                status,
                signer: optional_field(&signer.to_str_lossy()),
                key_id: optional_field(&key_id.to_str_lossy()),
                output: output.to_str_lossy().into_owned(),
            })
        })
        .collect();
    let mut seen = FxHashMap::<gix::ObjectId, usize>::default();
    for record in &records {
        *seen.entry(record.oid).or_default() += 1;
    }
    records.retain(|record| seen.get(&record.oid) == Some(&1));
    records
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

    pub(in super::super) fn verify_commit_signatures_cancellable_impl(
        &self,
        ids: &[CommitId],
        verifiable: SignatureFormats,
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
        if requested.is_empty() || verifiable.is_empty() {
            return Ok(Vec::new());
        }

        let oids: Vec<gix::ObjectId> = requested.iter().map(|(_, oid)| *oid).collect();
        let formats = self.signature_formats(&oids);
        let mut verdicts = FxHashMap::default();
        let signed: Vec<_> = requested
            .iter()
            .filter_map(|(_, oid)| formats.get(oid).map(|format| (*oid, *format)))
            // A format without an installed verifier would only yield `N`.
            .filter(|(_, format)| verifiable.contains(*format))
            .collect();
        if signed.is_empty() {
            return Ok(Vec::new());
        }
        let programs = self.signature_verifier_programs(cancellation)?;
        for chunk in signed.chunks(VERIFY_BATCH_SIZE) {
            if let Some(cancellation) = cancellation {
                cancellation.check_cancelled()?;
            }
            let formats = chunk.iter().copied().collect();
            let batch = self.run_verify_batch(&formats, &programs, VERIFY_TIMEOUT, cancellation);
            let verified = match batch {
                Ok(verified) => verified,
                Err(error) => {
                    let Some(failure) = interrupted_verification(&error) else {
                        return Err(error);
                    };
                    // Keep complete records from the interrupted process, then
                    // retry only unfinished commits individually. One stalled
                    // or crashing verifier must not erase progress for the rest
                    // of a page.
                    let mut verified = signatures_from_output(&formats, failure.stdout());
                    let completed: std::collections::HashSet<_> =
                        verified.iter().map(|(oid, _)| *oid).collect();
                    for (oid, format) in chunk.iter().filter(|(oid, _)| !completed.contains(oid)) {
                        let single = [(*oid, *format)].into_iter().collect();
                        match self.run_verify_batch(
                            &single,
                            &programs,
                            Duration::from_secs(2),
                            cancellation,
                        ) {
                            Ok(result) => verified.extend(result),
                            Err(error) if interrupted_verification(&error).is_some() => {}
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
        programs: &[OsString; 3],
        timeout: Duration,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Vec<(gix::ObjectId, Option<CommitSignature>)>> {
        let mut stdin =
            Vec::with_capacity(formats.keys().map(|oid| oid.kind().len_in_hex() + 1).sum());
        for oid in formats.keys() {
            stdin.extend_from_slice(oid.to_hex().to_string().as_bytes());
            stdin.push(b'\n');
        }

        let cmd = self.signature_verification_command(programs);
        let output =
            run_git_with_stdin_capture(cmd, stdin, "git log --stdin", timeout, cancellation)?;

        Ok(signatures_from_output(formats, &output))
    }

    fn signature_verifier_programs(
        &self,
        cancellation: Option<&CancellationToken>,
    ) -> Result<[OsString; 3]> {
        let mut config = self.git_workdir_cmd();
        config.args([
            "--no-pager",
            "config",
            "--null",
            "--show-scope",
            "--get-regexp",
            r"^gpg\.(program|(openpgp|x509|ssh)\.program)$",
        ]);
        let fallback = CancellationToken::new();
        let label = "git config (signature verifier programs)";
        let output = run_git_preview_output(config, label, cancellation.unwrap_or(&fallback))?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(git_command_failed_error(label, output));
        }
        scoped_verifier_programs(&output.stdout)
    }

    fn signature_verification_command(&self, programs: &[OsString; 3]) -> std::process::Command {
        let mut cmd = self.git_workdir_cmd();
        for program in programs {
            cmd.arg("-c").arg(program);
        }
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
        cmd
    }
}

fn scoped_verifier_programs(listing: &[u8]) -> Result<[OsString; 3]> {
    let mut programs = VERIFIER_PROGRAMS.map(|(_, program)| OsString::from(program));
    let invalid_listing = || {
        Error::new(ErrorKind::Backend(
            "invalid Git verifier configuration listing".into(),
        ))
    };
    if !listing.is_empty() {
        let mut fields = listing
            .strip_suffix(b"\0")
            .ok_or_else(invalid_listing)?
            .split(|&byte| byte == 0);
        while let Some(scope) = fields.next() {
            let entry = fields.next().ok_or_else(invalid_listing)?;
            if !matches!(scope, b"system" | b"global" | b"command") {
                continue;
            }
            let separator = entry.find_byte(b'\n').ok_or_else(invalid_listing)?;
            let (key, program) = (&entry[..separator], &entry[separator + 1..]);
            let key = if key == b"gpg.program" {
                b"gpg.openpgp.program"
            } else {
                key
            };
            if let Some(index) = VERIFIER_PROGRAMS
                .iter()
                .position(|(name, _)| name.as_bytes() == key)
            {
                programs[index] =
                    path_buf_from_git_bytes(program, "Git signature verifier program")?
                        .into_os_string();
            }
        }
    }
    Ok(std::array::from_fn(|index| {
        let mut config = OsString::from(VERIFIER_PROGRAMS[index].0);
        config.push("=");
        config.push(&programs[index]);
        config
    }))
}

/// A verifier that stalls times out. One that exits without reading its payload
/// kills git with SIGPIPE (no exit code): git's SSH `check-novalidate` fallback
/// does not ignore it, and git resets SIGPIPE at startup, so we cannot ignore
/// it on git's behalf.
fn interrupted_verification(error: &Error) -> Option<&GitFailure> {
    let ErrorKind::Git(failure) = error.kind() else {
        return None;
    };
    let killed_by_signal =
        failure.id() == GitFailureId::CommandFailed && failure.exit_code().is_none();
    (failure.id() == GitFailureId::Timeout || killed_by_signal).then_some(failure)
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
    fn verification_command_uses_defaults_instead_of_repository_programs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        crate::test_support::init_test_repo(root);
        crate::test_support::git_success(root, &["config", "gpg.program", "inert-local"]);
        crate::test_support::git_success(root, &["config", "gpg.ssh.program", "inert-local"]);
        let repo = crate::test_support::open_repo(root);
        let programs = repo.signature_verifier_programs(None).unwrap();
        let command = repo.signature_verification_command(&programs);
        for expected in [
            "gpg.openpgp.program=gpg",
            "gpg.x509.program=gpgsm",
            "gpg.ssh.program=ssh-keygen",
        ] {
            assert!(command.get_args().any(|arg| arg == expected), "{expected}");
        }
    }

    #[test]
    fn verifier_programs_preserve_trusted_config_and_alias_precedence() {
        let programs = scoped_verifier_programs(
            b"global\0gpg.openpgp.program\nlegacy-default\0local\0gpg.openpgp.program\nlocal-placeholder\0global\0gpg.program\nuser-verifier\0global\0gpg.ssh.program\nssh verifier\0worktree\0gpg.ssh.program\nworktree-placeholder\0",
        )
        .unwrap();
        assert_eq!(
            programs,
            [
                "gpg.openpgp.program=user-verifier",
                "gpg.x509.program=gpgsm",
                "gpg.ssh.program=ssh verifier",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn a_cached_page_does_not_evict_itself_by_counting_hits_as_new_entries() {
        let dir = tempfile::tempdir().unwrap();
        let git = gix::init(dir.path()).unwrap();
        let repo = GixRepo::new(dir.path().to_path_buf(), git.into_sync()).unwrap();
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

    /// `%GG` is the verifier's own text, which quotes the signer's key user
    /// id. A record with more than the five requested fields means a
    /// separator reached the output from there; none of it can be trusted.
    #[test]
    fn a_record_with_extra_fields_gets_no_verdict() {
        let first = gix::ObjectId::from_hex(b"1111111111111111111111111111111111111111").unwrap();
        let formats = [(first, SignatureFormat::OpenPgp)].into_iter().collect();
        let output = format!("{first}\x1fG\x1fSigner\x1fKey\x1fgpg: Good signature\x1fsurplus\x1e");
        let records = signatures_from_output(&formats, output.as_bytes());
        assert!(
            records.iter().all(|(_, signature)| signature.is_none()),
            "{records:?}"
        );
    }

    /// A record separator smuggled into one commit's `%GG` could append a
    /// forged record for another commit in the same batch. Git prints exactly
    /// one record per requested commit, so a second one voids both.
    #[test]
    fn a_second_record_for_one_commit_cannot_override_its_verdict() {
        let first = gix::ObjectId::from_hex(b"1111111111111111111111111111111111111111").unwrap();
        let second = gix::ObjectId::from_hex(b"2222222222222222222222222222222222222222").unwrap();
        let formats = [
            (first, SignatureFormat::OpenPgp),
            (second, SignatureFormat::OpenPgp),
        ]
        .into_iter()
        .collect();
        let output = format!(
            "{second}\x1fN\x1f\x1f\x1f\x1e\
             {first}\x1fG\x1fAttacker\x1fK1\x1fgpg: Good signature from Attacker\x1e\
             {second}\x1fG\x1fTrusted Maintainer\x1fK2\x1fforged\x1e"
        );
        let records = signatures_from_output(&formats, output.as_bytes());
        assert!(
            !records.iter().any(|(oid, signature)| *oid == second
                && signature.as_ref().is_some_and(|s| s.status.is_verified())),
            "a forged record marked the commit as verified: {records:?}"
        );
    }

    #[test]
    fn format_cache_evicts_cold_entries_without_flushing_hot_ones() {
        let dir = tempfile::tempdir().unwrap();
        let git = gix::init(dir.path()).unwrap();
        let repo = GixRepo::new(dir.path().to_path_buf(), git.into_sync()).unwrap();
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
