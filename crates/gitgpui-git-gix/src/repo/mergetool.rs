use super::GixRepo;
use gitgpui_core::error::{Error, ErrorKind};
use gitgpui_core::services::{
    validate_conflict_resolution_text, CommandOutput, MergetoolResult, Result,
};
use std::path::{Path, PathBuf};
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
        let MergetoolConfig {
            tool_name,
            tool_cmd,
            tool_path,
            trust_exit_code,
            write_to_temp,
            keep_temporaries,
        } = resolve_mergetool_config(workdir, env_has_display())?;
        let stage_paths =
            materialize_mergetool_stage_files(workdir, path, write_to_temp, keep_temporaries)?;

        let base_path = &stage_paths.base;
        let local_path = &stage_paths.local;
        let remote_path = &stage_paths.remote;
        let merged_path = workdir.join(path);

        // 4. Snapshot merged contents before tool invocation so we can
        //    detect actual content changes when trustExitCode is false.
        let pre_merged_state = if trust_exit_code {
            None
        } else {
            Some(read_merged_file_state(&merged_path)?)
        };

        // Build and invoke the mergetool command
        let output = if let Some(ref custom_cmd) = tool_cmd {
            // Match git-mergetool behavior by providing variables as shell env.
            // This supports both "$VAR" and "${VAR}" templates in config.
            Command::new("sh")
                .arg("-c")
                .arg(custom_cmd)
                .env("BASE", base_path)
                .env("LOCAL", local_path)
                .env("REMOTE", remote_path)
                .env("MERGED", &merged_path)
                .current_dir(workdir)
                .output()
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?
        } else {
            // No custom command — try invoking the tool name directly with
            // the standard argument convention used by many merge tools.
            let tool_executable = tool_path.as_deref().unwrap_or(&tool_name);
            Command::new(tool_executable)
                .arg(local_path)
                .arg(base_path)
                .arg(remote_path)
                .arg(&merged_path)
                .current_dir(workdir)
                .output()
                .map_err(|e| {
                    Error::new(ErrorKind::Backend(format!(
                        "Failed to launch mergetool '{tool_name}' ({tool_executable}): {e}"
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
        let post_merged_state = read_merged_file_state(&merged_path)?;
        let tool_success = if trust_exit_code {
            output.status.success()
        } else {
            // When trustExitCode is false (default), require an actual
            // merged-output delta (bytes change or file deletion/creation).
            pre_merged_state.as_ref() != Some(&post_merged_state)
        };

        if !tool_success {
            return Ok(MergetoolResult {
                tool_name,
                success: false,
                merged_contents: None,
                output: cmd_output,
            });
        }

        // 6. Stage tool output. For deleted output, stage deletion instead
        // of reading/staging file contents.
        let merged_contents = match post_merged_state {
            MergedFileState::Present(bytes) => {
                // Validate textual merged output and refuse staging if conflict
                // markers are still present.
                if let Ok(merged_text) = std::str::from_utf8(&bytes) {
                    let validation = validate_conflict_resolution_text(merged_text);
                    if validation.has_conflict_markers {
                        return Err(Error::new(ErrorKind::Backend(format!(
                            "Mergetool '{tool_name}' left unresolved conflict markers in {} ({} marker lines); refusing to stage",
                            path.display(),
                            validation.marker_lines
                        ))));
                    }
                }

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

                Some(bytes)
            }
            MergedFileState::Missing => {
                let mut rm = Command::new("git");
                rm.arg("-C").arg(workdir).arg("rm").arg("--").arg(path);
                let rm_output = rm
                    .output()
                    .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
                if !rm_output.status.success() {
                    let rm_stderr = String::from_utf8_lossy(&rm_output.stderr);
                    return Err(Error::new(ErrorKind::Backend(format!(
                        "git rm failed after mergetool: {}",
                        rm_stderr.trim()
                    ))));
                }
                None
            }
        };

        Ok(MergetoolResult {
            tool_name,
            success: true,
            merged_contents,
            output: cmd_output,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuiDefault {
    False,
    True,
    Auto,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MergetoolConfig {
    tool_name: String,
    tool_cmd: Option<String>,
    tool_path: Option<String>,
    trust_exit_code: bool,
    write_to_temp: bool,
    keep_temporaries: bool,
}

fn env_has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn parse_gui_default(value: Option<&str>) -> Result<GuiDefault> {
    let Some(value) = value else {
        return Ok(GuiDefault::False);
    };

    if value.eq_ignore_ascii_case("auto") {
        return Ok(GuiDefault::Auto);
    }

    match parse_git_bool(value) {
        Some(true) => Ok(GuiDefault::True),
        Some(false) => Ok(GuiDefault::False),
        None => Err(Error::new(ErrorKind::Backend(format!(
            "Invalid value for mergetool.guiDefault: {:?}. Expected true/false or auto.",
            value
        )))),
    }
}

fn choose_mergetool_name(
    merge_tool: Option<String>,
    merge_guitool: Option<String>,
    gui_default: GuiDefault,
    has_display: bool,
) -> Result<String> {
    let prefer_gui = match gui_default {
        GuiDefault::True => true,
        GuiDefault::False => false,
        GuiDefault::Auto => has_display,
    };

    if prefer_gui {
        if let Some(tool) = merge_guitool {
            return Ok(tool);
        }
        if let Some(tool) = merge_tool {
            return Ok(tool);
        }
    } else if let Some(tool) = merge_tool {
        return Ok(tool);
    }

    if let Some(tool) = merge_guitool {
        return Ok(tool);
    }

    Err(Error::new(ErrorKind::Backend(
        "No merge.tool or merge.guitool configured. Set one with: \
         git config merge.tool <toolname> or git config merge.guitool <toolname>"
            .to_string(),
    )))
}

fn resolve_mergetool_config(workdir: &Path, has_display: bool) -> Result<MergetoolConfig> {
    let merge_tool = git_config_get(workdir, "merge.tool")?;
    let merge_guitool = git_config_get(workdir, "merge.guitool")?;
    let gui_default =
        parse_gui_default(git_config_get(workdir, "mergetool.guiDefault")?.as_deref())?;

    let tool_name = choose_mergetool_name(merge_tool, merge_guitool, gui_default, has_display)?;
    let tool_cmd = git_config_get(workdir, &format!("mergetool.{tool_name}.cmd"))?;
    let tool_path = git_config_get(workdir, &format!("mergetool.{tool_name}.path"))?;
    let trust_exit_code =
        git_config_get_bool(workdir, &format!("mergetool.{tool_name}.trustExitCode"))?
            .unwrap_or(false);
    let write_to_temp = git_config_get_bool(workdir, "mergetool.writeToTemp")?.unwrap_or(false);
    let keep_temporaries =
        git_config_get_bool(workdir, "mergetool.keepTemporaries")?.unwrap_or(false);

    Ok(MergetoolConfig {
        tool_name,
        tool_cmd,
        tool_path,
        trust_exit_code,
        write_to_temp,
        keep_temporaries,
    })
}

#[derive(Debug)]
struct StagePaths {
    workdir: PathBuf,
    base: PathBuf,
    local: PathBuf,
    remote: PathBuf,
    _temp_dir: Option<tempfile::TempDir>,
    cleanup_files: bool,
}

impl Drop for StagePaths {
    fn drop(&mut self) {
        if !self.cleanup_files {
            return;
        }
        for path in [&self.base, &self.local, &self.remote] {
            let path = stage_path_to_fs_path(&self.workdir, path);
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {}
            }
        }
    }
}

fn materialize_mergetool_stage_files(
    workdir: &Path,
    conflict_path: &Path,
    write_to_temp: bool,
    keep_temporaries: bool,
) -> Result<StagePaths> {
    let stage_paths = build_stage_paths(workdir, conflict_path, write_to_temp, keep_temporaries)?;
    write_stage_bytes(
        workdir,
        &stage_paths.base,
        git_show_stage_bytes(workdir, 1, conflict_path)?
            .as_deref()
            .unwrap_or(b""),
    )?;
    write_stage_bytes(
        workdir,
        &stage_paths.local,
        git_show_stage_bytes(workdir, 2, conflict_path)?
            .as_deref()
            .unwrap_or(b""),
    )?;
    write_stage_bytes(
        workdir,
        &stage_paths.remote,
        git_show_stage_bytes(workdir, 3, conflict_path)?
            .as_deref()
            .unwrap_or(b""),
    )?;
    Ok(stage_paths)
}

fn build_stage_paths(
    workdir: &Path,
    conflict_path: &Path,
    write_to_temp: bool,
    keep_temporaries: bool,
) -> Result<StagePaths> {
    let (mut merge_base, ext) = split_merged_path_and_extension(conflict_path);
    let pid = std::process::id();

    if write_to_temp {
        let tmp_dir = tempfile::Builder::new()
            .prefix("gitgpui-mergetool-")
            .tempdir()
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        let (tmp_dir_path, temp_dir_guard) = if keep_temporaries {
            (tmp_dir.keep(), None)
        } else {
            (tmp_dir.path().to_path_buf(), Some(tmp_dir))
        };
        merge_base = PathBuf::from(
            merge_base
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        );

        let merge_base_name = merge_base.to_string_lossy();
        let base = tmp_dir_path.join(format!("{merge_base_name}_BASE_{pid}{ext}"));
        let local = tmp_dir_path.join(format!("{merge_base_name}_LOCAL_{pid}{ext}"));
        let remote = tmp_dir_path.join(format!("{merge_base_name}_REMOTE_{pid}{ext}"));

        return Ok(StagePaths {
            workdir: workdir.to_path_buf(),
            base,
            local,
            remote,
            _temp_dir: temp_dir_guard,
            cleanup_files: false,
        });
    }

    let merge_base = PathBuf::from(".").join(merge_base);
    let parent = merge_base.parent().unwrap_or(Path::new("."));
    let merge_base_name = merge_base
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let base = parent.join(format!("{merge_base_name}_BASE_{pid}{ext}"));
    let local = parent.join(format!("{merge_base_name}_LOCAL_{pid}{ext}"));
    let remote = parent.join(format!("{merge_base_name}_REMOTE_{pid}{ext}"));

    Ok(StagePaths {
        workdir: workdir.to_path_buf(),
        base,
        local,
        remote,
        _temp_dir: None,
        cleanup_files: !keep_temporaries,
    })
}

fn split_merged_path_and_extension(path: &Path) -> (PathBuf, String) {
    let mut merge_base = path.to_path_buf();
    let Some(ext) = path.extension() else {
        return (merge_base, String::new());
    };
    let ext = format!(".{}", ext.to_string_lossy());
    merge_base.set_extension("");
    (merge_base, ext)
}

fn stage_path_to_fs_path(workdir: &Path, stage_path: &Path) -> PathBuf {
    if stage_path.is_absolute() {
        stage_path.to_path_buf()
    } else {
        workdir.join(stage_path)
    }
}

fn write_stage_bytes(workdir: &Path, stage_path: &Path, bytes: &[u8]) -> Result<()> {
    let path = stage_path_to_fs_path(workdir, stage_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    }
    std::fs::write(path, bytes).map_err(|e| Error::new(ErrorKind::Io(e.kind())))
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

/// Read a git config boolean value.
///
/// Supports git-style boolean literals: true/false, yes/no, on/off, 1/0.
fn git_config_get_bool(workdir: &Path, key: &str) -> Result<Option<bool>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workdir)
        .arg("config")
        .arg("--get")
        .arg(key)
        .output()
        .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

    if output.status.success() {
        // In git config files, a bare boolean key (no explicit value) is
        // treated as `true`; `git config --get` returns an empty line for it.
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            return Ok(Some(true));
        }
        parse_git_bool(&value).map(Some).ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "Invalid boolean value for git config {key}: {:?}. Expected true/false, yes/no, on/off, or 1/0.",
                value
            )))
        })
    } else {
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

fn parse_git_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
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

#[derive(Debug, Clone, Eq, PartialEq)]
enum MergedFileState {
    Present(Vec<u8>),
    Missing,
}

fn read_merged_file_state(path: &Path) -> Result<MergedFileState> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(MergedFileState::Present(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(MergedFileState::Missing),
        Err(e) => Err(Error::new(ErrorKind::Io(e.kind()))),
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

    #[test]
    fn test_build_stage_paths_write_to_temp_false_uses_workdir_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = build_stage_paths(tmp.path(), Path::new("dir/a.txt"), false, false).unwrap();

        assert!(paths._temp_dir.is_none());
        assert!(paths.cleanup_files);
        assert_eq!(paths.base.parent(), Some(Path::new("./dir")));
        assert_eq!(paths.local.parent(), Some(Path::new("./dir")));
        assert_eq!(paths.remote.parent(), Some(Path::new("./dir")));

        let base_name = paths
            .base
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let local_name = paths
            .local
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let remote_name = paths
            .remote
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(base_name.starts_with("a_BASE_"), "{base_name}");
        assert!(local_name.starts_with("a_LOCAL_"), "{local_name}");
        assert!(remote_name.starts_with("a_REMOTE_"), "{remote_name}");
        assert!(base_name.ends_with(".txt"));
        assert!(local_name.ends_with(".txt"));
        assert!(remote_name.ends_with(".txt"));
    }

    #[test]
    fn test_build_stage_paths_write_to_temp_true_uses_tempdir() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = build_stage_paths(tmp.path(), Path::new("nested/a.txt"), true, false).unwrap();

        assert!(paths._temp_dir.is_some());
        assert!(!paths.cleanup_files);
        assert!(paths.base.is_absolute());
        assert!(paths.local.is_absolute());
        assert!(paths.remote.is_absolute());

        let base_name = paths
            .base
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let local_name = paths
            .local
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let remote_name = paths
            .remote
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(base_name.starts_with("a_BASE_"), "{base_name}");
        assert!(local_name.starts_with("a_LOCAL_"), "{local_name}");
        assert!(remote_name.starts_with("a_REMOTE_"), "{remote_name}");
        assert!(base_name.ends_with(".txt"));
    }

    #[test]
    fn test_parse_git_bool_true_variants() {
        for value in ["true", "TRUE", "yes", "on", "1", "  YeS  "] {
            assert_eq!(parse_git_bool(value), Some(true), "value={value:?}");
        }
    }

    #[test]
    fn test_parse_git_bool_false_variants() {
        for value in ["false", "FALSE", "no", "off", "0", "  Off  "] {
            assert_eq!(parse_git_bool(value), Some(false), "value={value:?}");
        }
    }

    #[test]
    fn test_parse_gui_default_variants() {
        assert_eq!(parse_gui_default(None).unwrap(), GuiDefault::False);
        assert_eq!(parse_gui_default(Some("auto")).unwrap(), GuiDefault::Auto);
        assert_eq!(parse_gui_default(Some("TRUE")).unwrap(), GuiDefault::True);
        assert_eq!(parse_gui_default(Some("off")).unwrap(), GuiDefault::False);
    }

    #[test]
    fn test_parse_gui_default_invalid_errors() {
        let err = parse_gui_default(Some("sometimes")).unwrap_err();
        assert!(matches!(
            err.kind(),
            ErrorKind::Backend(message) if message.contains("mergetool.guiDefault")
        ));
    }

    #[test]
    fn test_choose_mergetool_name_prefers_guitool_when_enabled() {
        let selected = choose_mergetool_name(
            Some("cli-tool".to_string()),
            Some("gui-tool".to_string()),
            GuiDefault::True,
            false,
        )
        .unwrap();
        assert_eq!(selected, "gui-tool");
    }

    #[test]
    fn test_choose_mergetool_name_auto_without_display_prefers_cli_tool() {
        let selected = choose_mergetool_name(
            Some("cli-tool".to_string()),
            Some("gui-tool".to_string()),
            GuiDefault::Auto,
            false,
        )
        .unwrap();
        assert_eq!(selected, "cli-tool");
    }

    #[test]
    fn test_choose_mergetool_name_auto_with_display_prefers_guitool() {
        let selected = choose_mergetool_name(
            Some("cli-tool".to_string()),
            Some("gui-tool".to_string()),
            GuiDefault::Auto,
            true,
        )
        .unwrap();
        assert_eq!(selected, "gui-tool");
    }

    #[test]
    fn test_choose_mergetool_name_falls_back_to_guitool_if_only_guitool_set() {
        let selected =
            choose_mergetool_name(None, Some("gui-tool".to_string()), GuiDefault::False, false)
                .unwrap();
        assert_eq!(selected, "gui-tool");
    }

    #[test]
    fn test_choose_mergetool_name_errors_when_no_tool_configured() {
        let err = choose_mergetool_name(None, None, GuiDefault::False, false).unwrap_err();
        assert!(matches!(
            err.kind(),
            ErrorKind::Backend(message) if message.contains("merge.tool or merge.guitool")
        ));
    }

    #[test]
    fn test_git_config_get_bool_nonexistent_key_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        let result = git_config_get_bool(workdir, "nonexistent.bool.key").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_git_config_get_bool_parses_variants() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("config")
            .arg("mergetool.test.trustExitCode")
            .arg("yes")
            .output()
            .unwrap();
        assert_eq!(
            git_config_get_bool(workdir, "mergetool.test.trustExitCode").unwrap(),
            Some(true)
        );

        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("config")
            .arg("mergetool.test.trustExitCode")
            .arg("off")
            .output()
            .unwrap();
        assert_eq!(
            git_config_get_bool(workdir, "mergetool.test.trustExitCode").unwrap(),
            Some(false)
        );
    }

    #[test]
    fn test_git_config_get_bool_invalid_value_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("config")
            .arg("mergetool.test.trustExitCode")
            .arg("sometimes")
            .output()
            .unwrap();

        let err = git_config_get_bool(workdir, "mergetool.test.trustExitCode").unwrap_err();
        assert!(matches!(
            err.kind(),
            ErrorKind::Backend(message) if message.contains("Invalid boolean value")
        ));
    }

    #[test]
    fn test_git_config_get_bool_bare_key_is_true() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        let config_path = workdir.join(".git").join("config");
        let mut config = std::fs::read_to_string(&config_path).unwrap();
        config.push_str("\n[mergetool \"test\"]\n\ttrustExitCode\n");
        std::fs::write(config_path, config).unwrap();

        assert_eq!(
            git_config_get_bool(workdir, "mergetool.test.trustExitCode").unwrap(),
            Some(true)
        );
    }

    #[test]
    fn test_resolve_mergetool_config_prefers_guitool_and_reads_path_override() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "merge.tool", "cli"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "merge.guitool", "gui"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "mergetool.guiDefault", "true"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "mergetool.gui.path", "/opt/fake-gui-tool"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "mergetool.gui.trustExitCode", "yes"])
            .output()
            .unwrap();

        let cfg = resolve_mergetool_config(workdir, false).unwrap();
        assert_eq!(cfg.tool_name, "gui");
        assert_eq!(cfg.tool_cmd, None);
        assert_eq!(cfg.tool_path.as_deref(), Some("/opt/fake-gui-tool"));
        assert!(cfg.trust_exit_code);
        assert!(!cfg.write_to_temp);
        assert!(!cfg.keep_temporaries);
    }

    #[test]
    fn test_resolve_mergetool_config_auto_without_display_uses_merge_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "merge.tool", "cli"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "merge.guitool", "gui"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "mergetool.guiDefault", "auto"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "mergetool.cli.cmd", "exit 0"])
            .output()
            .unwrap();

        let cfg = resolve_mergetool_config(workdir, false).unwrap();
        assert_eq!(cfg.tool_name, "cli");
        assert_eq!(cfg.tool_cmd.as_deref(), Some("exit 0"));
        assert!(!cfg.write_to_temp);
        assert!(!cfg.keep_temporaries);
    }

    #[test]
    fn test_resolve_mergetool_config_reads_write_to_temp() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "merge.tool", "cli"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "mergetool.cli.cmd", "exit 0"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "mergetool.writeToTemp", "true"])
            .output()
            .unwrap();

        let cfg = resolve_mergetool_config(workdir, false).unwrap();
        assert!(cfg.write_to_temp);
        assert!(!cfg.keep_temporaries);
    }

    #[test]
    fn test_resolve_mergetool_config_reads_keep_temporaries() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "merge.tool", "cli"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "mergetool.cli.cmd", "exit 0"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "mergetool.keepTemporaries", "true"])
            .output()
            .unwrap();

        let cfg = resolve_mergetool_config(workdir, false).unwrap();
        assert!(!cfg.write_to_temp);
        assert!(cfg.keep_temporaries);
    }
}
