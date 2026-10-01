use super::GixRepo;
use crate::util::{run_git_capture_bytes, run_git_with_output};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, Result};
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;

/// `patch` in a temp file for `git apply`, which reads it by path.
pub(super) fn write_patch_file(patch: &[u8]) -> Result<NamedTempFile> {
    let mut file = NamedTempFile::new().map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    file.write_all(patch)
        .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    Ok(file)
}

/// NUL-terminated paths for `--pathspec-from-file` with `--pathspec-file-nul`;
/// a long path list would overflow a Windows command line.
pub(super) fn write_pathspec_file<'a>(
    paths: impl IntoIterator<Item = &'a [u8]>,
) -> Result<NamedTempFile> {
    let mut bytes = Vec::new();
    for path in paths {
        bytes.extend_from_slice(path);
        bytes.push(0);
    }
    write_patch_file(&bytes)
}

impl GixRepo {
    pub(super) fn export_patch_with_output_impl(
        &self,
        commit_id: &CommitId,
        dest: &Path,
    ) -> Result<CommandOutput> {
        let sha = commit_id.as_ref();
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("format-patch")
            .arg("-1")
            .arg(sha)
            .arg("--stdout")
            .arg("--binary");
        // The bytes as git wrote them: file content in any encoding must reach
        // the patch file unchanged, or `git am` cannot apply it.
        let patch = run_git_capture_bytes(cmd, &format!("git format-patch -1 {sha} --stdout"))?;
        std::fs::write(dest, patch).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        Ok(CommandOutput {
            command: format!("Export patch {sha}"),
            stdout: format!("Saved patch to {}", dest.display()),
            stderr: String::new(),
            exit_code: Some(0),
        })
    }

    pub(super) fn apply_patch_with_output_impl(&self, patch: &Path) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("am").arg("--3way").arg("--").arg(patch);
        run_git_with_output(cmd, &format!("git am --3way {}", patch.display()))
    }

    pub(super) fn apply_unified_patch_to_index_with_output_impl(
        &self,
        patch: &[u8],
        reverse: bool,
    ) -> Result<CommandOutput> {
        let tmp_file = write_patch_file(patch)?;
        let tmp_path = tmp_file.path();

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("apply")
            .arg("--cached")
            .arg("--recount")
            .arg("--whitespace=nowarn");
        if reverse {
            cmd.arg("--reverse");
        }
        cmd.arg(tmp_path);

        let label = if reverse {
            format!("git apply --cached --reverse {}", tmp_path.display())
        } else {
            format!("git apply --cached {}", tmp_path.display())
        };

        run_git_with_output(cmd, &label)
    }

    pub(super) fn apply_unified_patch_to_worktree_with_output_impl(
        &self,
        patch: &[u8],
        reverse: bool,
    ) -> Result<CommandOutput> {
        let tmp_file = write_patch_file(patch)?;
        let tmp_path = tmp_file.path();

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("apply").arg("--recount").arg("--whitespace=nowarn");
        if reverse {
            cmd.arg("--reverse");
        }
        cmd.arg(tmp_path);

        let label = if reverse {
            format!("git apply --reverse {}", tmp_path.display())
        } else {
            format!("git apply {}", tmp_path.display())
        };

        run_git_with_output(cmd, &label)
    }
}
