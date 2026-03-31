use super::{
    GixRepo, bstr_to_arc_str,
    conflict_stages::{gix_index_stage_blob_bytes_optional, gix_index_stage_exists},
    oid_to_arc_str,
};
use crate::util::{bytes_to_text_preserving_utf8, run_git_with_output};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{BlameLine, CommandOutput, ConflictSide, Result};
use gix::bstr::ByteSlice as _;
use rustc_hash::FxHashMap as HashMap;
use std::collections::hash_map::Entry;
use std::fs;
use std::path::Path;
use std::sync::Arc;

struct BlameCommitMetadata {
    commit_id_text: Arc<str>,
    author: Arc<str>,
    author_time_unix: Option<i64>,
    summary: Arc<str>,
}

fn blame_commit_metadata<'a>(
    repo: &gix::Repository,
    cache: &'a mut HashMap<gix::ObjectId, BlameCommitMetadata>,
    commit_id: gix::ObjectId,
) -> Result<&'a BlameCommitMetadata> {
    match cache.entry(commit_id) {
        Entry::Occupied(entry) => Ok(entry.into_mut()),
        Entry::Vacant(entry) => {
            let commit = repo.find_commit(commit_id).map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "gix find_commit {commit_id}: {e}"
                )))
            })?;

            let (author, author_time_unix) = match commit.author() {
                Ok(signature) => (
                    bstr_to_arc_str(signature.name.as_ref()),
                    signature.time().ok().map(|time| time.seconds),
                ),
                Err(_) => (Arc::<str>::default(), None),
            };
            let summary_bytes = commit
                .message_raw_sloppy()
                .lines()
                .next()
                .unwrap_or_default();
            let summary = bstr_to_arc_str(summary_bytes);

            Ok(entry.insert(BlameCommitMetadata {
                commit_id_text: oid_to_arc_str(&commit_id),
                author,
                author_time_unix,
                summary,
            }))
        }
    }
}

fn blame_line_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes_to_text_preserving_utf8(bytes),
    }
}

struct BlameBlobLines<'a> {
    blob: &'a [u8],
    cursor: usize,
}

impl<'a> Iterator for BlameBlobLines<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.blob.len() {
            return None;
        }

        let start = self.cursor;
        let remaining = &self.blob[start..];
        if let Some(offset) = remaining.iter().position(|byte| *byte == b'\n') {
            let end = start + offset + 1;
            self.cursor = end;
            Some(&self.blob[start..end])
        } else {
            self.cursor = self.blob.len();
            Some(&self.blob[start..])
        }
    }
}

fn blame_blob_lines(blob: &[u8]) -> BlameBlobLines<'_> {
    BlameBlobLines { blob, cursor: 0 }
}

impl GixRepo {
    pub(super) fn blame_file_impl(&self, path: &Path, rev: Option<&str>) -> Result<Vec<BlameLine>> {
        const BLOB_LINE_MISMATCH: &str = "gix blame blob line count did not match blame entries";

        let repo = self._repo.to_thread_local();
        let spec = rev.unwrap_or("HEAD");
        let suspect = repo
            .rev_parse_single(spec)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix rev-parse {spec}: {e}"))))?
            .detach();
        let git_path = gix::path::os_str_into_bstr(path.as_os_str())
            .map(gix::path::to_unix_separators_on_windows)
            .map_err(|_| Error::new(ErrorKind::Unsupported("path is not valid UTF-8")))?;
        let outcome = repo
            .blame_file(git_path.as_ref(), suspect, Default::default())
            .map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "gix blame {}: {e}",
                    path.display()
                )))
            })?;

        let mut metadata_cache = HashMap::default();
        let total_lines = outcome
            .entries
            .last()
            .map(|entry| entry.start_in_blamed_file as usize + entry.len.get() as usize)
            .unwrap_or_default();
        let mut lines = Vec::with_capacity(total_lines);
        let mut blob_lines = blame_blob_lines(&outcome.blob);
        let mut blob_line_ix = 0usize;
        for entry in &outcome.entries {
            let entry_start = entry.start_in_blamed_file as usize;
            let entry_len = entry.len.get() as usize;
            while blob_line_ix < entry_start {
                if blob_lines.next().is_none() {
                    return Err(Error::new(ErrorKind::Backend(
                        BLOB_LINE_MISMATCH.to_string(),
                    )));
                }
                blob_line_ix += 1;
            }
            let metadata = blame_commit_metadata(&repo, &mut metadata_cache, entry.commit_id)?;
            for _ in 0..entry_len {
                let Some(line) = blob_lines.next() else {
                    return Err(Error::new(ErrorKind::Backend(
                        BLOB_LINE_MISMATCH.to_string(),
                    )));
                };
                blob_line_ix += 1;
                lines.push(BlameLine {
                    commit_id: metadata.commit_id_text.clone(),
                    author: metadata.author.clone(),
                    author_time_unix: metadata.author_time_unix,
                    summary: metadata.summary.clone(),
                    line: blame_line_text(line),
                });
            }
        }
        Ok(lines)
    }

    pub(super) fn checkout_conflict_side_impl(
        &self,
        path: &Path,
        side: ConflictSide,
    ) -> Result<CommandOutput> {
        let desired_stage = match side {
            ConflictSide::Ours => 2,
            ConflictSide::Theirs => 3,
        };

        let repo = self._repo.to_thread_local();

        if !gix_index_stage_exists(&repo, path, desired_stage)? {
            let mut rm = self.git_workdir_cmd();
            rm.arg("rm").arg("--").arg(path);
            return run_git_with_output(rm, "git rm --");
        }

        let mut checkout = self.git_workdir_cmd();
        checkout.arg("checkout");
        match side {
            ConflictSide::Ours => {
                checkout.arg("--ours");
            }
            ConflictSide::Theirs => {
                checkout.arg("--theirs");
            }
        }
        checkout.arg("--").arg(path);
        let checkout_out = run_git_with_output(checkout, "git checkout --ours/--theirs")?;

        let mut add = self.git_workdir_cmd();
        add.arg("add").arg("--").arg(path);
        let add_out = run_git_with_output(add, "git add --")?;

        Ok(CommandOutput {
            command: checkout_out.command,
            stdout: [checkout_out.stdout, add_out.stdout]
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            stderr: [checkout_out.stderr, add_out.stderr]
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            exit_code: add_out.exit_code.or(checkout_out.exit_code),
        })
    }

    pub(super) fn accept_conflict_deletion_impl(&self, path: &Path) -> Result<CommandOutput> {
        let mut rm = self.git_workdir_cmd();
        rm.arg("rm").arg("--").arg(path);
        run_git_with_output(rm, "git rm --")
    }

    pub(super) fn checkout_conflict_base_impl(&self, path: &Path) -> Result<CommandOutput> {
        let repo = self._repo.to_thread_local();
        let base_bytes = gix_index_stage_blob_bytes_optional(&repo, path, 1)?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "base conflict stage is not available for {}",
                path.display()
            )))
        })?;
        let abs_path = self.spec.workdir.join(path);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        }
        fs::write(&abs_path, base_bytes).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

        let mut add = self.git_workdir_cmd();
        add.arg("add").arg("--").arg(path);
        let add_out = run_git_with_output(add, "git add --")?;

        Ok(CommandOutput {
            command: format!("git show :1:{} + git add --", path.display()),
            stdout: add_out.stdout,
            stderr: add_out.stderr,
            exit_code: add_out.exit_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blame_line_text_trims_crlf_and_lf() {
        assert_eq!(blame_line_text(b"hello\n"), "hello");
        assert_eq!(blame_line_text(b"hello\r\n"), "hello");
        assert_eq!(blame_line_text(b"hello"), "hello");
    }

    #[test]
    fn blame_blob_lines_preserves_terminators_and_final_line() {
        let blob = b"first\nsecond\r\nthird";
        let lines = blame_blob_lines(blob).collect::<Vec<_>>();
        assert_eq!(
            lines,
            vec![&b"first\n"[..], &b"second\r\n"[..], &b"third"[..]]
        );
    }

    #[test]
    fn blame_blob_lines_is_empty_for_empty_blob() {
        assert_eq!(blame_blob_lines(b"").count(), 0);
    }
}
