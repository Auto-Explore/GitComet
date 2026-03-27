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
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

struct BlameCommitMetadata {
    commit_id_text: Arc<str>,
    author: Arc<str>,
    author_time_unix: Option<i64>,
    summary: Arc<str>,
}

fn blame_commit_metadata(
    repo: &gix::Repository,
    cache: &mut HashMap<gix::ObjectId, Rc<BlameCommitMetadata>>,
    commit_id: gix::ObjectId,
) -> Result<Rc<BlameCommitMetadata>> {
    if let Some(metadata) = cache.get(&commit_id) {
        return Ok(Rc::clone(metadata));
    }

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

    let metadata = Rc::new(BlameCommitMetadata {
        commit_id_text: oid_to_arc_str(&commit_id),
        author,
        author_time_unix,
        summary,
    });
    cache.insert(commit_id, Rc::clone(&metadata));
    Ok(metadata)
}

fn blame_line_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes_to_text_preserving_utf8(bytes),
    }
}

impl GixRepo {
    pub(super) fn blame_file_impl(&self, path: &Path, rev: Option<&str>) -> Result<Vec<BlameLine>> {
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
        let mut lines = Vec::new();
        for (entry, entry_lines) in outcome.entries_with_lines() {
            let metadata = blame_commit_metadata(&repo, &mut metadata_cache, entry.commit_id)?;
            for line in entry_lines {
                lines.push(BlameLine {
                    commit_id: metadata.commit_id_text.clone(),
                    author: metadata.author.clone(),
                    author_time_unix: metadata.author_time_unix,
                    summary: metadata.summary.clone(),
                    line: blame_line_text(line.as_ref()),
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
}
