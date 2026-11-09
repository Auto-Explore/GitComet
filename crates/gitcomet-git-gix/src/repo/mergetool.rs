use super::mergetool_builtin::{
    BuiltinMergeCommand, MergetoolFiles, builtin_merge_command, builtin_tool_key,
    builtin_tool_program,
};
use super::{GixRepo, conflict_stages::gix_index_stage_blob_bytes_optional};
use crate::util::{
    bytes_to_text_preserving_utf8, fnv1a_64, push_literal_paths, run_git_simple, stable_path_bytes,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::{
    canonicalize_or_original, symlink_free_write_target, validated_repo_relative_path,
};
use gitcomet_core::process::background_command as no_window_command;
use gitcomet_core::services::{
    CommandOutput, MergetoolResult, Result, validate_conflict_resolution_text,
};
#[cfg(any(test, feature = "test-support"))]
use rustc_hash::FxHashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
#[cfg(any(not(windows), test))]
use std::process::Command;
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Mutex, OnceLock};

impl GixRepo {
    /// Launch an external mergetool for a conflicted file.
    ///
    /// The implementation:
    /// 1. Reads `merge.tool` from git config to determine the tool name.
    ///    Repository-local configuration that decides *what gets executed* —
    ///    `merge.tool`/`merge.guitool` naming a tool outside git's built-in
    ///    table, `mergetool.<tool>.path`, and `mergetool.<tool>.cmd` — is
    ///    blocked by default unless explicitly trusted via a GitComet global
    ///    consent key.
    /// 2. Extracts conflict stages (`:1:`, `:2:`, `:3:`) into temp files.
    /// 3. Invokes the tool with the BASE, LOCAL, REMOTE and MERGED files, using
    ///    git's built-in argument convention for the tool (see
    ///    [`super::mergetool_builtin`]) so it opens in merge mode rather than as
    ///    a read-only diff.
    /// 4. Reads trust-exit config to decide success semantics:
    ///    `mergetool.<tool>.trustExitCode`, then `mergetool.trustExitCode`.
    /// 5. Reads back the merged file and stages it on success.
    pub(super) fn launch_mergetool_impl(&self, path: &Path) -> Result<MergetoolResult> {
        let workdir = &self.spec.workdir;
        // Keep the caller's slash-separated repository path for index and Git
        // operations. Rebuilding it is only for filesystem access, where
        // Windows needs native separators.
        let filesystem_path = validated_mergetool_path(path)?;
        reject_symlink_conflict(workdir, &filesystem_path, path)?;
        let merged_path = mergetool_worktree_target(workdir, &filesystem_path)?;
        let repo = self.reopen_repo()?;
        let MergetoolConfig {
            tool_name,
            tool_cmd,
            tool_path,
            trust_exit_code,
            write_to_temp,
            keep_temporaries,
        } = resolve_mergetool_config(&repo, env_has_display())?;
        let stage_paths = materialize_mergetool_stage_files(
            &repo,
            workdir,
            path,
            write_to_temp,
            keep_temporaries,
        )?;

        let base_path = &stage_paths.base;
        let local_path = &stage_paths.local;
        let remote_path = &stage_paths.remote;
        // 4. Snapshot merged contents before tool invocation so we can
        //    detect actual content changes when trustExitCode is false.
        let pre_merged_state = if trust_exit_code {
            None
        } else {
            Some(read_merged_file_state(&merged_path)?)
        };

        // Build and invoke the mergetool command
        let output = if let Some(ref custom_cmd) = tool_cmd {
            run_custom_mergetool_command(
                custom_cmd,
                workdir,
                base_path,
                local_path,
                remote_path,
                &merged_path,
            )?
        } else {
            // No custom command — use the argument convention git's built-in
            // definition for this tool would use. Passing bare positional paths
            // leaves tools such as KDiff3 in read-only 3-way diff mode because
            // nothing tells them where to write the merge result.
            // `mergetool.<tool>.path` wins; otherwise a few built-ins are
            // invoked through a command that is not the tool name (`vscode` runs
            // `code`, `bc` runs `bcomp`, ...), exactly as git translates them.
            let builtin_program = match tool_path {
                Some(_) => None,
                None => builtin_tool_program(&tool_name),
            };
            let tool_executable = tool_path
                .as_deref()
                .or(builtin_program.as_deref())
                .unwrap_or(&tool_name);
            let args = match builtin_merge_command(
                &tool_name,
                &MergetoolFiles {
                    base: base_path,
                    local: local_path,
                    remote: remote_path,
                    merged: &merged_path,
                    merged_label: path,
                    base_present: stage_paths.base_present,
                },
            ) {
                BuiltinMergeCommand::Args(args) => args,
                BuiltinMergeCommand::Unsupported(message) => {
                    return Err(Error::new(ErrorKind::Backend(message)));
                }
                // Not a git built-in: keep the generic convention, which is what
                // a tool configured only through `mergetool.<tool>.path` gets.
                BuiltinMergeCommand::Unknown => vec![
                    local_path.into(),
                    base_path.into(),
                    remote_path.into(),
                    merged_path.clone().into_os_string(),
                ],
            };

            no_window_command(tool_executable)
                .args(&args)
                .current_dir(workdir)
                .output()
                .map_err(|e| {
                    Error::new(ErrorKind::Backend(format!(
                        "Failed to launch mergetool '{tool_name}' ({tool_executable}): {e}"
                    )))
                    .with_source(e)
                    .with_context("launch mergetool", Some(Path::new(tool_executable)))
                })?
        };

        let stdout = bytes_to_text_preserving_utf8(&output.stdout);
        let stderr = bytes_to_text_preserving_utf8(&output.stderr);
        let exit_code = output.status.code();

        let cmd_output = CommandOutput {
            command: format!("mergetool ({tool_name})"),
            stdout,
            stderr,
            exit_code,
        };

        // 5. Determine success
        // The tool may leave a symlink here, so re-check before reading. It has
        // already run: report that rather than losing its output.
        let post_merged_state = match mergetool_worktree_target(workdir, &filesystem_path) {
            Ok(merged_path) => read_merged_file_state(&merged_path)?,
            Err(err) => {
                return Ok(MergetoolResult {
                    tool_name,
                    success: false,
                    merged_contents: None,
                    output: append_stderr_note(cmd_output, &err.to_string()),
                });
            }
        };
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
                let mut add = self.git_workdir_cmd();
                add.arg("add");
                push_literal_paths(&mut add, [path]);
                run_git_simple(add, "git add (after mergetool)")?;

                Some(bytes)
            }
            MergedFileState::Missing => {
                let mut rm = self.git_workdir_cmd();
                rm.arg("rm");
                push_literal_paths(&mut rm, [path]);
                run_git_simple(rm, "git rm (after mergetool)")?;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GitConfigScope {
    Any,
    Global,
    Local,
    /// Every scope the repository cannot write: installation, system, global
    /// and process-level overrides.
    Trusted,
    /// `.git/config` and `.git/config.worktree`: whoever produced the
    /// repository controls these.
    Repository,
}

fn env_has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(all(windows, test))]
fn shell_command(custom_cmd: &str) -> Command {
    let mut command = no_window_command("cmd");
    command.arg("/C").arg(custom_cmd);
    command
}

#[cfg(not(windows))]
fn shell_command(custom_cmd: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(custom_cmd);
    command
}

fn run_custom_mergetool_command(
    custom_cmd: &str,
    workdir: &Path,
    base_path: &Path,
    local_path: &Path,
    remote_path: &Path,
    merged_path: &Path,
) -> Result<std::process::Output> {
    #[cfg(windows)]
    {
        let script_dir = tempfile::Builder::new()
            .prefix("gitcomet-mergetool-shell-")
            .tempdir()
            .map_err(Error::from)?;
        let script_path = script_dir.path().join("run-mergetool.cmd");
        let mut script = String::from("@echo off\r\n");
        script.push_str(custom_cmd);
        script.push_str("\r\n");
        std::fs::write(&script_path, script).map_err(Error::from)?;

        let mut command = no_window_command("cmd");
        command.arg("/C").arg(&script_path);
        command
            .env("BASE", base_path)
            .env("LOCAL", local_path)
            .env("REMOTE", remote_path)
            .env("MERGED", merged_path)
            .current_dir(workdir);
        command.output().map_err(Error::from)
    }
    #[cfg(not(windows))]
    {
        let mut command = shell_command(custom_cmd);
        command
            .env("BASE", base_path)
            .env("LOCAL", local_path)
            .env("REMOTE", remote_path)
            .env("MERGED", merged_path)
            .current_dir(workdir);
        command.output().map_err(Error::from)
    }
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

fn resolve_mergetool_config(repo: &gix::Repository, has_display: bool) -> Result<MergetoolConfig> {
    let gui_default = parse_gui_default(git_config_get(repo, "mergetool.guiDefault")?.as_deref())?;

    let tool_name = resolve_mergetool_name_with_trust_mode(repo, gui_default, has_display)?;
    let tool_cmd = resolve_mergetool_command_with_trust_mode(repo, &tool_name)?;
    let tool_path = resolve_mergetool_path_with_trust_mode(repo, &tool_name)?;
    let trust_exit_code =
        match git_config_get_bool(repo, &format!("mergetool.{tool_name}.trustExitCode"))? {
            Some(value) => value,
            None => git_config_get_bool(repo, "mergetool.trustExitCode")?.unwrap_or(false),
        };
    let write_to_temp = git_config_get_bool(repo, "mergetool.writeToTemp")?.unwrap_or(false);
    let keep_temporaries = git_config_get_bool(repo, "mergetool.keepTemporaries")?.unwrap_or(false);

    Ok(MergetoolConfig {
        tool_name,
        tool_cmd,
        tool_path,
        trust_exit_code,
        write_to_temp,
        keep_temporaries,
    })
}

fn resolve_mergetool_command_with_trust_mode(
    repo: &gix::Repository,
    tool_name: &str,
) -> Result<Option<String>> {
    let cmd_key = format!("mergetool.{tool_name}.cmd");
    let global_cmd = git_config_get_with_scope(repo, &cmd_key, GitConfigScope::Global)?;
    let local_cmd = git_config_get_with_scope(repo, &cmd_key, GitConfigScope::Local)?;

    let Some(local_cmd) = local_cmd else {
        return Ok(global_cmd);
    };

    if repo_local_mergetool_command_allowed(repo, tool_name)? {
        return Ok(Some(local_cmd));
    }

    if global_cmd.is_some() {
        return Ok(global_cmd);
    }

    Err(untrusted_repo_local_mergetool_config_error(
        repo, tool_name, &cmd_key, &local_cmd,
    ))
}

/// Pick the merge tool the way git does — repository config overrides the
/// trusted scopes — then refuse a repository-controlled choice unless it is a
/// git built-in or the user has consented to this repository's tool.
///
/// The tool name is the program: a name outside git's built-in table is
/// resolved from `PATH` verbatim and handed the conflict stage files as
/// arguments, so `merge.tool = sh` in a hostile `.git/config` would execute the
/// attacker's stage-2 file. A built-in name only ever runs the well-known
/// program git maps it to, which the repository cannot influence, so it stays
/// allowed without consent.
fn resolve_mergetool_name_with_trust_mode(
    repo: &gix::Repository,
    gui_default: GuiDefault,
    has_display: bool,
) -> Result<String> {
    let vetted_tool = git_config_get_with_scope(repo, "merge.tool", GitConfigScope::Trusted)?;
    let vetted_guitool = git_config_get_with_scope(repo, "merge.guitool", GitConfigScope::Trusted)?;
    let local_tool = git_config_get_with_scope(repo, "merge.tool", GitConfigScope::Repository)?;
    let local_guitool =
        git_config_get_with_scope(repo, "merge.guitool", GitConfigScope::Repository)?;

    let vetted_choice = choose_mergetool_name(
        vetted_tool.clone(),
        vetted_guitool.clone(),
        gui_default,
        has_display,
    )
    .ok();
    let tool_name = choose_mergetool_name(
        local_tool.clone().or(vetted_tool),
        local_guitool.clone().or(vetted_guitool),
        gui_default,
        has_display,
    )?;

    if vetted_choice.as_deref() == Some(tool_name.as_str())
        || builtin_tool_key(&tool_name).is_some()
        || repo_local_mergetool_command_allowed(repo, &tool_name)?
    {
        return Ok(tool_name);
    }
    if let Some(vetted) = vetted_choice {
        return Ok(vetted);
    }

    let key = if local_guitool.as_deref() == Some(tool_name.as_str())
        && local_tool.as_deref() != Some(tool_name.as_str())
    {
        "merge.guitool"
    } else {
        "merge.tool"
    };
    Err(untrusted_repo_local_mergetool_config_error(
        repo, &tool_name, key, &tool_name,
    ))
}

/// `mergetool.<tool>.path` names the executable outright, so a
/// repository-local value is gated exactly like a repository-local `.cmd`.
fn resolve_mergetool_path_with_trust_mode(
    repo: &gix::Repository,
    tool_name: &str,
) -> Result<Option<String>> {
    let path_key = format!("mergetool.{tool_name}.path");
    let trusted_path = git_config_get_with_scope(repo, &path_key, GitConfigScope::Trusted)?;
    let local_path = git_config_get_with_scope(repo, &path_key, GitConfigScope::Repository)?;

    let Some(local_path) = local_path else {
        return Ok(trusted_path);
    };

    if repo_local_mergetool_command_allowed(repo, tool_name)? {
        return Ok(Some(local_path));
    }

    if trusted_path.is_some() {
        return Ok(trusted_path);
    }

    Err(untrusted_repo_local_mergetool_config_error(
        repo,
        tool_name,
        &path_key,
        &local_path,
    ))
}

fn untrusted_repo_local_mergetool_config_error(
    repo: &gix::Repository,
    tool_name: &str,
    key: &str,
    blocked_value: &str,
) -> Error {
    let consent_key =
        repo_local_mergetool_command_consent_key(repo_workdir_for_mergetool(repo), tool_name);
    Error::new(ErrorKind::Backend(format!(
        "Refusing to use repository-local {key} for '{tool_name}' without explicit consent.\n\
         Blocked value from repository config:\n\
         {blocked_value}\n\
         To allow repository-local mergetool configuration for this repository and tool, run:\n\
         git config --global {consent_key} true"
    )))
}

fn repo_local_mergetool_command_allowed(repo: &gix::Repository, tool_name: &str) -> Result<bool> {
    let consent_key =
        repo_local_mergetool_command_consent_key(repo_workdir_for_mergetool(repo), tool_name);
    #[cfg(any(test, feature = "test-support"))]
    if test_repo_local_mergetool_command_allowed(&consent_key) {
        return Ok(true);
    }
    Ok(
        git_config_get_bool_with_scope(repo, &consent_key, GitConfigScope::Global)?
            .unwrap_or(false),
    )
}

fn repo_workdir_for_mergetool(repo: &gix::Repository) -> &Path {
    repo.workdir().unwrap_or_else(|| repo.git_dir())
}

fn repo_local_mergetool_command_consent_key(workdir: &Path, tool_name: &str) -> String {
    let repo_tool_fingerprint = stable_repo_tool_fingerprint(workdir, tool_name);
    format!("gitcomet.mergetool.allowrepolocalcmd-{repo_tool_fingerprint}")
}

#[cfg(any(test, feature = "test-support"))]
static TEST_ALLOWED_REPO_LOCAL_MERGETOOL_COMMANDS: OnceLock<Mutex<FxHashSet<String>>> =
    OnceLock::new();

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn allow_test_repo_local_mergetool_command(workdir: &Path, tool_name: &str) {
    let consent_key = repo_local_mergetool_command_consent_key(workdir, tool_name);
    let allowed =
        TEST_ALLOWED_REPO_LOCAL_MERGETOOL_COMMANDS.get_or_init(|| Mutex::new(FxHashSet::default()));
    allowed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(consent_key);
}

#[cfg(any(test, feature = "test-support"))]
fn test_repo_local_mergetool_command_allowed(consent_key: &str) -> bool {
    let Some(allowed) = TEST_ALLOWED_REPO_LOCAL_MERGETOOL_COMMANDS.get() else {
        return false;
    };
    allowed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(consent_key)
}

fn stable_repo_tool_fingerprint(workdir: &Path, tool_name: &str) -> String {
    let repo_path = canonicalize_or_original(workdir.to_path_buf());
    let mut bytes = stable_path_bytes(&repo_path);
    bytes.push(0);
    bytes.extend_from_slice(tool_name.as_bytes());
    format!("{:016x}", fnv1a_64(&bytes))
}

#[derive(Debug)]
struct StagePaths {
    workdir: PathBuf,
    base: PathBuf,
    local: PathBuf,
    remote: PathBuf,
    /// Whether the index carried a stage 1 entry. Built-in merge tools take a
    /// different (two-way) command line when there is no merge base, so the
    /// empty placeholder file written for `base` must not be mistaken for one.
    base_present: bool,
    _temp_dir: Option<tempfile::TempDir>,
    cleanup_files: bool,
}

impl Drop for StagePaths {
    fn drop(&mut self) {
        if !self.cleanup_files {
            return;
        }
        for path in [&self.base, &self.local, &self.remote] {
            if let Ok(path) = stage_path_to_fs_path(&self.workdir, path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn materialize_mergetool_stage_files(
    repo: &gix::Repository,
    workdir: &Path,
    conflict_path: &Path,
    write_to_temp: bool,
    keep_temporaries: bool,
) -> Result<StagePaths> {
    let mut stage_paths =
        build_stage_paths(workdir, conflict_path, write_to_temp, keep_temporaries)?;
    let base_bytes = gix_index_stage_blob_bytes_optional(repo, conflict_path, 1)?;
    stage_paths.base_present = base_bytes.is_some();
    write_stage_bytes(
        workdir,
        &stage_paths.base,
        base_bytes.as_deref().unwrap_or(b""),
    )?;
    write_stage_bytes(
        workdir,
        &stage_paths.local,
        gix_index_stage_blob_bytes_optional(repo, conflict_path, 2)?
            .as_deref()
            .unwrap_or(b""),
    )?;
    write_stage_bytes(
        workdir,
        &stage_paths.remote,
        gix_index_stage_blob_bytes_optional(repo, conflict_path, 3)?
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
    let normalized_conflict_path = validated_mergetool_path(conflict_path)?;
    let (mut merge_base, ext) = split_merged_path_and_extension(&normalized_conflict_path);
    let pid = std::process::id();

    if write_to_temp {
        let tmp_dir = tempfile::Builder::new()
            .prefix("gitcomet-mergetool-")
            .tempdir()
            .map_err(Error::from)?;
        let (tmp_dir_path, temp_dir_guard) = if keep_temporaries {
            (tmp_dir.keep(), None)
        } else {
            (tmp_dir.path().to_path_buf(), Some(tmp_dir))
        };
        merge_base = PathBuf::from(merge_base.file_name().unwrap_or_default());

        let merge_base_name = merge_base.file_name().unwrap_or_default();
        let base = tmp_dir_path.join(build_stage_variant_file_name(
            merge_base_name,
            "BASE",
            pid,
            &ext,
        ));
        let local = tmp_dir_path.join(build_stage_variant_file_name(
            merge_base_name,
            "LOCAL",
            pid,
            &ext,
        ));
        let remote = tmp_dir_path.join(build_stage_variant_file_name(
            merge_base_name,
            "REMOTE",
            pid,
            &ext,
        ));

        return Ok(StagePaths {
            workdir: workdir.to_path_buf(),
            base,
            local,
            remote,
            base_present: false,
            _temp_dir: temp_dir_guard,
            cleanup_files: false,
        });
    }

    let merge_base = PathBuf::from(".").join(merge_base);
    let parent = merge_base.parent().unwrap_or(Path::new("."));
    let merge_base_name = merge_base.file_name().unwrap_or_default();
    let base = parent.join(build_stage_variant_file_name(
        merge_base_name,
        "BASE",
        pid,
        &ext,
    ));
    let local = parent.join(build_stage_variant_file_name(
        merge_base_name,
        "LOCAL",
        pid,
        &ext,
    ));
    let remote = parent.join(build_stage_variant_file_name(
        merge_base_name,
        "REMOTE",
        pid,
        &ext,
    ));

    Ok(StagePaths {
        workdir: workdir.to_path_buf(),
        base,
        local,
        remote,
        base_present: false,
        _temp_dir: None,
        cleanup_files: !keep_temporaries,
    })
}

fn split_merged_path_and_extension(path: &Path) -> (PathBuf, Option<OsString>) {
    let mut merge_base = path.to_path_buf();
    let ext = path.extension().map(OsStr::to_os_string);
    if ext.is_some() {
        merge_base.set_extension("");
    }
    (merge_base, ext)
}

fn build_stage_variant_file_name(
    base_name: &OsStr,
    role: &str,
    pid: u32,
    ext: &Option<OsString>,
) -> OsString {
    let mut name = base_name.to_os_string();
    name.push(format!("_{role}_{pid}"));
    if let Some(ext) = ext.as_deref() {
        name.push(".");
        name.push(ext);
    }
    name
}

fn validated_mergetool_path(path: &Path) -> Result<PathBuf> {
    validated_repo_relative_path(path).map_err(|err| {
        Error::new(ErrorKind::Backend(format!(
            "invalid mergetool conflict path '{}': {err}",
            path.display()
        )))
        .with_source(err)
        .with_context("validate mergetool conflict path", Some(path))
    })
}

/// Git's mergetool never runs a tool on a symlink conflict; it asks for a side.
/// Say so rather than reporting the generic containment refusal.
fn reject_symlink_conflict(workdir: &Path, path: &Path, label: &Path) -> Result<()> {
    let is_symlink = std::fs::symlink_metadata(workdir.join(path))
        .is_ok_and(|metadata| metadata.file_type().is_symlink());
    if is_symlink {
        return Err(Error::new(ErrorKind::Backend(format!(
            "'{}' is a symbolic-link conflict; a merge tool cannot merge a symlink. \
             Resolve it by taking the local or remote side.",
            label.display()
        ))));
    }
    Ok(())
}

fn append_stderr_note(mut output: CommandOutput, note: &str) -> CommandOutput {
    if !output.stderr.is_empty() && !output.stderr.ends_with('\n') {
        output.stderr.push('\n');
    }
    output.stderr.push_str(note);
    output.stderr.push('\n');
    output
}

fn mergetool_worktree_target(workdir: &Path, path: &Path) -> Result<PathBuf> {
    symlink_free_write_target(workdir, path).map_err(|err| {
        Error::new(ErrorKind::Backend(format!(
            "unsafe mergetool conflict path '{}': {err}",
            path.display()
        )))
        .with_source(err)
        .with_context("resolve mergetool conflict path", Some(&workdir.join(path)))
    })
}

/// Where a stage temporary lives: `mergetool.writeToTemp` gives an absolute
/// path, a relative one goes through the worktree guard.
fn stage_path_to_fs_path(workdir: &Path, stage_path: &Path) -> Result<PathBuf> {
    if stage_path.is_absolute() {
        Ok(stage_path.to_path_buf())
    } else {
        mergetool_worktree_target(workdir, stage_path)
    }
}

fn write_stage_bytes(workdir: &Path, stage_path: &Path, bytes: &[u8]) -> Result<()> {
    let path = stage_path_to_fs_path(workdir, stage_path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(Error::from)?;
    }
    std::fs::write(path, bytes).map_err(Error::from)
}

/// Read a git config value. Returns `Ok(None)` if the key is not set.
fn git_config_get(repo: &gix::Repository, key: &str) -> Result<Option<String>> {
    git_config_get_with_scope(repo, key, GitConfigScope::Any)
}

/// Read a git config value from a specific scope. Returns `Ok(None)` if the key is not set.
fn git_config_get_with_scope(
    repo: &gix::Repository,
    key: &str,
    scope: GitConfigScope,
) -> Result<Option<String>> {
    let config = repo.config_snapshot();
    let value = match git_config_scope_filter(scope) {
        Some(filter) => config.plumbing().string_filter(key, filter),
        None => config.plumbing().string(key),
    };

    Ok(value.and_then(|value| {
        let value = bytes_to_text_preserving_utf8(&value);
        (!value.is_empty()).then_some(value)
    }))
}

/// Read a git config boolean value.
///
/// Supports git-style boolean literals: true/false, yes/no, on/off, 1/0.
fn git_config_get_bool(repo: &gix::Repository, key: &str) -> Result<Option<bool>> {
    git_config_get_bool_with_scope(repo, key, GitConfigScope::Any)
}

/// Read a git config boolean value from a specific scope.
///
/// Supports git-style boolean literals: true/false, yes/no, on/off, 1/0.
fn git_config_get_bool_with_scope(
    repo: &gix::Repository,
    key: &str,
    scope: GitConfigScope,
) -> Result<Option<bool>> {
    let config = repo.config_snapshot();
    let value = match git_config_scope_filter(scope) {
        Some(filter) => config.plumbing().boolean_filter(key, filter),
        None => config.plumbing().boolean(key),
    };

    match value.transpose() {
        Some(Ok(value)) => Ok(Some(value)),
        Some(Err(err)) => {
            let input = err.metadata().find_map(|values| match values.get("input") {
                Some(gix::error::MetadataValue::Bytes(input)) => Some(input.as_slice()),
                _ => None,
            });
            let value = bytes_to_text_preserving_utf8(input.unwrap_or_default());
            Err(Error::new(ErrorKind::Backend(format!(
                "Invalid boolean value for git config {key}: {:?}. Expected true/false, yes/no, on/off, or 1/0.",
                value
            ))))
        }
        None => Ok(None),
    }
}

type GitConfigFilter = fn(&gix::config::file::Metadata) -> bool;

fn git_config_scope_filter(scope: GitConfigScope) -> Option<GitConfigFilter> {
    match scope {
        GitConfigScope::Any => None,
        GitConfigScope::Global => Some(config_value_is_global),
        GitConfigScope::Local => Some(config_value_is_local),
        GitConfigScope::Trusted => Some(config_value_is_trusted),
        GitConfigScope::Repository => Some(config_value_is_repository),
    }
}

fn config_value_is_repository(meta: &gix::config::file::Metadata) -> bool {
    meta.source.kind() == gix::config::source::Kind::Repository
}

fn config_value_is_trusted(meta: &gix::config::file::Metadata) -> bool {
    !config_value_is_repository(meta)
}

fn config_value_is_global(meta: &gix::config::file::Metadata) -> bool {
    meta.source.kind() == gix::config::source::Kind::Global
}

fn config_value_is_local(meta: &gix::config::file::Metadata) -> bool {
    meta.source == gix::config::Source::Local
}

fn parse_git_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
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
        Err(e) => Err(Error::from(e)),
    }
}

#[cfg(test)]
mod tests;
