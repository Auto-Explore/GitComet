use super::GixRepo;
use gitgpui_core::error::{Error, ErrorKind};
use gitgpui_core::services::{CommandOutput, MergetoolResult, Result};
use std::path::Path;
use std::process::Command;

impl GixRepo {
    /// Launch an external mergetool for a conflicted file.
    ///
    /// The implementation:
    /// 1. Reads `merge.tool` from git config to determine the tool name.
    /// 2. Extracts conflict stages (`:1:`, `:2:`, `:3:`) into temp files.
    /// 3. Invokes the tool with BASE, LOCAL, REMOTE, MERGED file paths.
    /// 4. Reads `mergetool.<tool>.trustExitCode` to decide success semantics.
    /// 5. Reads back the merged file and stages it on success.
    pub(super) fn launch_mergetool_impl(&self, path: &Path) -> Result<MergetoolResult> {
        let workdir = &self.spec.workdir;

        // 1. Determine which mergetool to use
        let tool_name = git_config_get(workdir, "merge.tool")?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "No merge.tool configured. Set it with: git config merge.tool <toolname>"
                    .to_string(),
            ))
        })?;

        // 2. Read the tool command template (or fall back to built-in tool name)
        let tool_cmd = git_config_get(workdir, &format!("mergetool.{tool_name}.cmd"))?;
        let trust_exit_code =
            git_config_get(workdir, &format!("mergetool.{tool_name}.trustExitCode"))?
                .map(|v| v.trim().eq_ignore_ascii_case("true"))
                .unwrap_or(false);

        // 3. Materialize temp files for BASE, LOCAL, REMOTE
        let tmp_dir = tempfile::Builder::new()
            .prefix("gitgpui-mergetool-")
            .tempdir()
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let base_path = tmp_dir.path().join(format!("{file_name}.BASE"));
        let local_path = tmp_dir.path().join(format!("{file_name}.LOCAL"));
        let remote_path = tmp_dir.path().join(format!("{file_name}.REMOTE"));
        let merged_path = workdir.join(path);

        // Extract stage :1: (base)
        let base_bytes = git_show_stage_bytes(workdir, 1, path)?;
        std::fs::write(&base_path, base_bytes.as_deref().unwrap_or(b""))
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

        // Extract stage :2: (ours/local)
        let local_bytes = git_show_stage_bytes(workdir, 2, path)?;
        std::fs::write(&local_path, local_bytes.as_deref().unwrap_or(b""))
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

        // Extract stage :3: (theirs/remote)
        let remote_bytes = git_show_stage_bytes(workdir, 3, path)?;
        std::fs::write(&remote_path, remote_bytes.as_deref().unwrap_or(b""))
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

        // 4. Snapshot merged contents before tool invocation so we can
        //    detect actual content changes when trustExitCode is false.
        let pre_merged_contents = if trust_exit_code {
            None
        } else {
            std::fs::read(&merged_path).ok()
        };

        // Build and invoke the mergetool command
        let output = if let Some(ref custom_cmd) = tool_cmd {
            // Custom command template — substitute variables
            let expanded = custom_cmd
                .replace("$BASE", &base_path.to_string_lossy())
                .replace("$LOCAL", &local_path.to_string_lossy())
                .replace("$REMOTE", &remote_path.to_string_lossy())
                .replace("$MERGED", &merged_path.to_string_lossy());

            Command::new("sh")
                .arg("-c")
                .arg(&expanded)
                .current_dir(workdir)
                .output()
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?
        } else {
            // No custom command — try invoking the tool name directly with
            // the standard argument convention used by many merge tools.
            Command::new(&tool_name)
                .arg(&local_path)
                .arg(&base_path)
                .arg(&remote_path)
                .arg(&merged_path)
                .current_dir(workdir)
                .output()
                .map_err(|e| {
                    Error::new(ErrorKind::Backend(format!(
                        "Failed to launch mergetool '{tool_name}': {e}"
                    )))
                })?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code();

        let cmd_output = CommandOutput {
            command: format!("mergetool ({tool_name})"),
            stdout,
            stderr,
            exit_code,
        };

        // 5. Determine success
        let mut post_merged_contents = None;
        let tool_success = if trust_exit_code {
            output.status.success()
        } else {
            // When trustExitCode is false (default), require actual content
            // changes in MERGED (not just metadata changes).
            match std::fs::read(&merged_path) {
                Ok(post) => {
                    let changed = pre_merged_contents.as_ref() != Some(&post);
                    if changed {
                        post_merged_contents = Some(post);
                    }
                    changed
                }
                Err(_) => false, // file doesn't exist or is unreadable
            }
        };

        if !tool_success {
            return Ok(MergetoolResult {
                tool_name,
                success: false,
                merged_contents: None,
                output: cmd_output,
            });
        }

        // 6. Read back merged contents and stage
        let merged_contents = match post_merged_contents {
            Some(bytes) => bytes,
            None => std::fs::read(&merged_path).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?,
        };

        // Stage the file
        let path_ref: &Path = path;
        let mut add = Command::new("git");
        add.arg("-C")
            .arg(workdir)
            .arg("add")
            .arg("--")
            .arg(path_ref);
        let add_output = add
            .output()
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

        if !add_output.status.success() {
            let add_stderr = String::from_utf8_lossy(&add_output.stderr);
            return Err(Error::new(ErrorKind::Backend(format!(
                "git add failed after mergetool: {}",
                add_stderr.trim()
            ))));
        }

        Ok(MergetoolResult {
            tool_name,
            success: true,
            merged_contents: Some(merged_contents),
            output: cmd_output,
        })
    }
}

/// Read a git config value. Returns `Ok(None)` if the key is not set.
fn git_config_get(workdir: &Path, key: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workdir)
        .arg("config")
        .arg("--get")
        .arg(key)
        .output()
        .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    } else {
        // Exit code 1 means key not found; other codes are errors
        let code = output.status.code().unwrap_or(-1);
        if code == 1 {
            Ok(None)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::new(ErrorKind::Backend(format!(
                "git config --get {key} failed: {}",
                stderr.trim()
            ))))
        }
    }
}

/// Read the content of a conflict stage as raw bytes.
/// Stage 1 = base, 2 = ours, 3 = theirs.
/// Returns `Ok(None)` if the stage doesn't exist for this file.
fn git_show_stage_bytes(workdir: &Path, stage: u8, path: &Path) -> Result<Option<Vec<u8>>> {
    let rev = format!(":{stage}:{}", path.to_string_lossy());
    let output = Command::new("git")
        .arg("-C")
        .arg(workdir)
        .arg("show")
        .arg(&rev)
        .output()
        .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

    if output.status.success() {
        Ok(Some(output.stdout))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.to_string();
        // Stage might not exist (e.g. add/add conflict has no base)
        if stderr.contains("does not exist")
            || stderr.contains("not at stage")
            || stderr.contains("bad revision")
            || stderr.contains("invalid object")
        {
            Ok(None)
        } else {
            Err(Error::new(ErrorKind::Backend(format!(
                "git show :{stage}:{} failed: {}",
                path.display(),
                stderr.trim()
            ))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_config_get_nonexistent_key_returns_none() {
        // Create a temporary git repo
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        let result = git_config_get(workdir, "nonexistent.key.xyz").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_git_config_get_existing_key() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        // Set a config value
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("config")
            .arg("merge.tool")
            .arg("vimdiff")
            .output()
            .unwrap();

        let result = git_config_get(workdir, "merge.tool").unwrap();
        assert_eq!(result, Some("vimdiff".to_string()));
    }

    #[test]
    fn test_git_show_stage_bytes_no_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        // No conflict stages exist
        let result = git_show_stage_bytes(workdir, 1, Path::new("nonexistent.txt")).unwrap();
        assert_eq!(result, None);
    }
}
