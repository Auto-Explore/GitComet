use super::GixRepo;
use crate::util::{run_git_capture_bytes_cancellable, run_git_parsed_stdout_cancellable};
use gitcomet_core::domain::{DiffArea, DiffTarget, FileDiffTextSource};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::patch_action::{DiffActionSnapshot, SOURCE_BYTE_LIMIT};
use gitcomet_core::services::{CancellationToken, Result};
use std::io::{Read, Write};
use std::sync::Arc;

fn source_text(source: Option<&FileDiffTextSource>, text: Option<Arc<str>>) -> Result<Arc<str>> {
    if let Some(source) = source {
        return read_bounded(&source.path).and_then(|bytes| {
            String::from_utf8(bytes).map(Arc::from).map_err(|_| {
                Error::new(ErrorKind::Unsupported(
                    "partial patches require UTF-8 sources",
                ))
            })
        });
    }
    let text = text.unwrap_or_else(|| Arc::from(""));
    if text.len() > SOURCE_BYTE_LIMIT {
        return Err(source_limit_error());
    }
    Ok(text)
}

fn source_limit_error() -> Error {
    Error::new(ErrorKind::Unsupported(
        "partial structural patches require sources of at most 1,000,000 bytes",
    ))
}

fn read_bounded(path: &std::path::Path) -> Result<Vec<u8>> {
    let file =
        std::fs::File::open(path).map_err(|error| Error::new(ErrorKind::Io(error.kind())))?;
    let mut bytes = Vec::new();
    file.take((SOURCE_BYTE_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::new(ErrorKind::Io(error.kind())))?;
    if bytes.len() > SOURCE_BYTE_LIMIT {
        return Err(source_limit_error());
    }
    Ok(bytes)
}

fn append_policy(policy: &mut Vec<u8>, bytes: &[u8]) {
    policy.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    policy.extend_from_slice(bytes);
}

fn append_worktree_copy(policy: &mut Vec<u8>, path: &std::path::Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            let bytes = if metadata.is_symlink() {
                std::fs::read_link(path)
                    .map_err(|error| Error::new(ErrorKind::Io(error.kind())))?
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec()
            } else {
                read_bounded(path)?
            };
            policy.push(1);
            append_policy(policy, &bytes);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => policy.push(0),
        Err(error) => return Err(Error::new(ErrorKind::Io(error.kind()))),
    }
    Ok(())
}

impl GixRepo {
    pub(super) fn diff_action_snapshot_impl(
        &self,
        target: &DiffTarget,
        cancellation: &CancellationToken,
    ) -> Result<DiffActionSnapshot> {
        if target.worktree().is_some() {
            return Err(Error::new(ErrorKind::Unsupported(
                "partial actions are unavailable for linked-worktree comparisons",
            )));
        }
        let DiffTarget::WorkingTree { path, area, .. } = target else {
            return Err(Error::new(ErrorKind::Unsupported(
                "partial patches require a working-tree comparison",
            )));
        };
        let source = self
            .diff_file_text_impl_cancellable(target, cancellation)?
            .ok_or_else(|| {
                Error::new(ErrorKind::Unsupported(
                    "partial patches require complete text sources",
                ))
            })?;
        if source.old_large.is_some() || source.new_large.is_some() {
            return Err(Error::new(ErrorKind::Unsupported(
                "partial patches are unavailable for managed payloads",
            )));
        }
        let absent_old = source.old_source.is_none() && source.old.is_none();
        let old = source_text(source.old_source.as_ref(), source.old)?;
        let new = source_text(source.new_source.as_ref(), source.new)?;
        let mut bytes = run_git_capture_bytes_cancellable(
            self.build_unified_diff_command_with_policy(target, true),
            "git diff (exact patch)",
            cancellation,
        )?;
        if bytes.is_empty() && absent_old && !new.is_empty() {
            let mut file = tempfile::NamedTempFile::new()
                .map_err(|error| Error::new(ErrorKind::Io(error.kind())))?;
            file.write_all(new.as_bytes())
                .map_err(|error| Error::new(ErrorKind::Io(error.kind())))?;
            let metadata = std::fs::metadata(self.spec.workdir.join(path))
                .map_err(|error| Error::new(ErrorKind::Io(error.kind())))?;
            file.as_file()
                .set_permissions(metadata.permissions())
                .map_err(|error| Error::new(ErrorKind::Io(error.kind())))?;
            let mut command = self.git_workdir_cmd();
            command
                .args([
                    "diff",
                    "--no-index",
                    "--no-color",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--",
                    "/dev/null",
                ])
                .arg(file.path());
            let raw = run_git_parsed_stdout_cancellable(
                command,
                "git diff (untracked exact patch)",
                true,
                cancellation,
                |mut stdout| {
                    let mut bytes = Vec::new();
                    stdout
                        .read_to_end(&mut bytes)
                        .map_err(|error| Error::new(ErrorKind::Io(error.kind())))?;
                    Ok(bytes)
                },
            )?;
            bytes = super::apply_change::retarget_patch(&raw, path)?;
        }
        let mut attributes = self.git_workdir_cmd();
        attributes
            .args(["check-attr", "-z", "--all", "--"])
            .arg(path);
        let attributes =
            run_git_capture_bytes_cancellable(attributes, "git check-attr", cancellation)?;
        let mut config = self.git_workdir_cmd();
        config.args([
            "config",
            "--null",
            "--get-regexp",
            "^(core\\.(autocrlf|eol|safecrlf|attributesfile)|filter\\.)",
        ]);
        let config = run_git_parsed_stdout_cancellable(
            config,
            "git config (patch policy)",
            true,
            cancellation,
            |mut stdout| {
                let mut bytes = Vec::new();
                stdout
                    .read_to_end(&mut bytes)
                    .map_err(|error| Error::new(ErrorKind::Io(error.kind())))?;
                Ok(bytes)
            },
        )?;
        let mut policy = Vec::new();
        append_policy(&mut policy, &attributes);
        append_policy(&mut policy, &config);
        // Unstaging moves lines between HEAD and the index; the worktree copy
        // plays no part in it, so it must not make the snapshot stale.
        if *area == DiffArea::Unstaged {
            append_worktree_copy(&mut policy, &self.spec.workdir.join(path))?;
        }
        Ok(DiffActionSnapshot {
            diff: gitcomet_core::patch_action::parse_exact_diff(target.clone(), bytes)
                .map_err(|error| Error::new(ErrorKind::Backend(error.to_string())))?,
            old,
            new,
            policy: policy.into(),
        })
    }
}
