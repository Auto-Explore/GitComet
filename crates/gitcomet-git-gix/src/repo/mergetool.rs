use super::{GixRepo, conflict_stages::gix_index_stage_blob_bytes_optional};
use crate::util::{bytes_to_text_preserving_utf8, run_git_simple};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CommandOutput, MergetoolResult, Result, validate_conflict_resolution_text,
};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

impl GixRepo {
    /// Launch an external mergetool for a conflicted file.
    ///
    /// The implementation:
    /// 1. Reads `merge.tool` from git config to determine the tool name.
    ///    Repository-local `mergetool.<tool>.cmd` is blocked by default unless
    ///    explicitly trusted via a GitComet global consent key.
    /// 2. Extracts conflict stages (`:1:`, `:2:`, `:3:`) into temp files.
    /// 3. Invokes the tool with BASE, LOCAL, REMOTE, MERGED file paths.
    /// 4. Reads trust-exit config to decide success semantics:
    ///    `mergetool.<tool>.trustExitCode`, then `mergetool.trustExitCode`.
    /// 5. Reads back the merged file and stages it on success.
    pub(super) fn launch_mergetool_impl(&self, path: &Path) -> Result<MergetoolResult> {
        let workdir = &self.spec.workdir;
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
        let merged_path = workdir.join(normalize_path_for_platform(path));

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
                let mut add = self.git_workdir_cmd();
                add.arg("add").arg("--").arg(path);
                run_git_simple(add, "git add (after mergetool)")?;

                Some(bytes)
            }
            MergedFileState::Missing => {
                let mut rm = self.git_workdir_cmd();
                rm.arg("rm").arg("--").arg(path);
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
}

fn env_has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(all(windows, test))]
fn shell_command(custom_cmd: &str) -> Command {
    let mut command = Command::new("cmd");
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
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        let script_path = script_dir.path().join("run-mergetool.cmd");
        let mut script = String::from("@echo off\r\n");
        script.push_str(custom_cmd);
        script.push_str("\r\n");
        std::fs::write(&script_path, script).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

        let mut command = Command::new("cmd");
        command.arg("/C").arg(&script_path);
        command
            .env("BASE", base_path)
            .env("LOCAL", local_path)
            .env("REMOTE", remote_path)
            .env("MERGED", merged_path)
            .current_dir(workdir);
        command
            .output()
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))
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
        command
            .output()
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))
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
    let merge_tool = git_config_get(repo, "merge.tool")?;
    let merge_guitool = git_config_get(repo, "merge.guitool")?;
    let gui_default = parse_gui_default(git_config_get(repo, "mergetool.guiDefault")?.as_deref())?;

    let tool_name = choose_mergetool_name(merge_tool, merge_guitool, gui_default, has_display)?;
    let tool_cmd = resolve_mergetool_command_with_trust_mode(repo, &tool_name)?;
    let tool_path = git_config_get(repo, &format!("mergetool.{tool_name}.path"))?;
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

    let consent_key =
        repo_local_mergetool_command_consent_key(repo_workdir_for_mergetool(repo), tool_name);
    Err(Error::new(ErrorKind::Backend(format!(
        "Refusing to execute repository-local mergetool command for '{tool_name}' without explicit consent.\n\
         Blocked command from repository config:\n\
         {local_cmd}\n\
         To allow this command for this repository and tool, run:\n\
         git config --global {} true",
        consent_key,
    ))))
}

fn repo_local_mergetool_command_allowed(repo: &gix::Repository, tool_name: &str) -> Result<bool> {
    let consent_key =
        repo_local_mergetool_command_consent_key(repo_workdir_for_mergetool(repo), tool_name);
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

static TEST_ALLOWED_REPO_LOCAL_MERGETOOL_COMMANDS: OnceLock<Mutex<HashSet<String>>> =
    OnceLock::new();

pub(crate) fn allow_test_repo_local_mergetool_command(workdir: &Path, tool_name: &str) {
    let consent_key = repo_local_mergetool_command_consent_key(workdir, tool_name);
    let allowed =
        TEST_ALLOWED_REPO_LOCAL_MERGETOOL_COMMANDS.get_or_init(|| Mutex::new(HashSet::new()));
    allowed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(consent_key);
}

fn test_repo_local_mergetool_command_allowed(consent_key: &str) -> bool {
    let Some(allowed) = TEST_ALLOWED_REPO_LOCAL_MERGETOOL_COMMANDS.get() else {
        return false;
    };
    allowed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(consent_key)
}

fn stable_path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        path.as_os_str().as_bytes().to_vec()
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let mut bytes = Vec::new();
        for unit in path.as_os_str().encode_wide() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[cfg(not(any(unix, windows)))]
    {
        path.to_str()
            .map(|text| text.as_bytes().to_vec())
            .unwrap_or_else(|| format!("{path:?}").into_bytes())
    }
}

fn stable_repo_tool_fingerprint(workdir: &Path, tool_name: &str) -> String {
    let repo_path = std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf());
    let mut bytes = stable_path_bytes(&repo_path);
    bytes.push(0);
    bytes.extend_from_slice(tool_name.as_bytes());
    format!("{:016x}", fnv1a_64(&bytes))
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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
    repo: &gix::Repository,
    workdir: &Path,
    conflict_path: &Path,
    write_to_temp: bool,
    keep_temporaries: bool,
) -> Result<StagePaths> {
    let stage_paths = build_stage_paths(workdir, conflict_path, write_to_temp, keep_temporaries)?;
    write_stage_bytes(
        workdir,
        &stage_paths.base,
        gix_index_stage_blob_bytes_optional(repo, conflict_path, 1)?
            .as_deref()
            .unwrap_or(b""),
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
    let normalized_conflict_path = normalize_path_for_platform(conflict_path);
    let (mut merge_base, ext) = split_merged_path_and_extension(&normalized_conflict_path);
    let pid = std::process::id();

    if write_to_temp {
        let tmp_dir = tempfile::Builder::new()
            .prefix("gitcomet-mergetool-")
            .tempdir()
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
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

fn normalize_path_for_platform(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        normalized.push(component.as_os_str());
    }
    normalized
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
        let value = bytes_to_text_preserving_utf8(value.as_ref().as_ref());
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
        Ok(value) => Ok(value),
        Err(err) => {
            let value = bytes_to_text_preserving_utf8(err.input.as_ref());
            Err(Error::new(ErrorKind::Backend(format!(
                "Invalid boolean value for git config {key}: {:?}. Expected true/false, yes/no, on/off, or 1/0.",
                value
            ))))
        }
    }
}

type GitConfigFilter = fn(&gix::config::file::Metadata) -> bool;

fn git_config_scope_filter(scope: GitConfigScope) -> Option<GitConfigFilter> {
    match scope {
        GitConfigScope::Any => None,
        GitConfigScope::Global => Some(config_value_is_global),
        GitConfigScope::Local => Some(config_value_is_local),
    }
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
        Err(e) => Err(Error::new(ErrorKind::Io(e.kind()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_repo(workdir: &Path) -> gix::Repository {
        gix::open(workdir).unwrap()
    }

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

        let repo = open_repo(workdir);
        let result = git_config_get(&repo, "nonexistent.key.xyz").unwrap();
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

        let repo = open_repo(workdir);
        let result = git_config_get(&repo, "merge.tool").unwrap();
        assert_eq!(result, Some("vimdiff".to_string()));
    }

    #[test]
    fn test_git_config_get_with_scope_local_ignores_worktree_config() {
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
            .args(["config", "extensions.worktreeConfig", "true"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "--worktree", "mergetool.fake.cmd", "worktree-cmd"])
            .output()
            .unwrap();

        let repo = open_repo(workdir);
        assert_eq!(
            git_config_get(&repo, "mergetool.fake.cmd").unwrap(),
            Some("worktree-cmd".to_string())
        );
        assert_eq!(
            git_config_get_with_scope(&repo, "mergetool.fake.cmd", GitConfigScope::Local).unwrap(),
            None
        );
    }

    #[test]
    fn test_read_index_stage_bytes_optional_no_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .arg("init")
            .output()
            .unwrap();

        // No conflict stages exist
        let repo = gix::open(workdir).unwrap();
        let result =
            gix_index_stage_blob_bytes_optional(&repo, Path::new("nonexistent.txt"), 1).unwrap();
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
            .and_then(|name| name.to_str())
            .expect("generated stage filename should be valid unicode")
            .to_string();
        let local_name = paths
            .local
            .file_name()
            .and_then(|name| name.to_str())
            .expect("generated stage filename should be valid unicode")
            .to_string();
        let remote_name = paths
            .remote
            .file_name()
            .and_then(|name| name.to_str())
            .expect("generated stage filename should be valid unicode")
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
            .and_then(|name| name.to_str())
            .expect("generated stage filename should be valid unicode")
            .to_string();
        let local_name = paths
            .local
            .file_name()
            .and_then(|name| name.to_str())
            .expect("generated stage filename should be valid unicode")
            .to_string();
        let remote_name = paths
            .remote
            .file_name()
            .and_then(|name| name.to_str())
            .expect("generated stage filename should be valid unicode")
            .to_string();
        assert!(base_name.starts_with("a_BASE_"), "{base_name}");
        assert!(local_name.starts_with("a_LOCAL_"), "{local_name}");
        assert!(remote_name.starts_with("a_REMOTE_"), "{remote_name}");
        assert!(base_name.ends_with(".txt"));
    }

    #[test]
    fn test_normalize_path_for_platform_preserves_components() {
        let normalized = normalize_path_for_platform(Path::new("docs/nested/file.txt"));
        let components: Vec<String> = normalized
            .components()
            .map(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .expect("test path components should be unicode")
                    .to_string()
            })
            .collect();
        assert_eq!(
            components,
            vec![
                "docs".to_string(),
                "nested".to_string(),
                "file.txt".to_string()
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_build_stage_paths_write_to_temp_false_normalizes_windows_separators() {
        let tmp = tempfile::tempdir().unwrap();
        let paths =
            build_stage_paths(tmp.path(), Path::new("docs/a space.txt"), false, false).unwrap();
        let base = paths.base.to_str().expect("path should be unicode");
        let local = paths.local.to_str().expect("path should be unicode");
        let remote = paths.remote.to_str().expect("path should be unicode");
        assert!(
            !base.contains('/'),
            "stage path should avoid mixed separators on Windows: {base}"
        );
        assert!(
            !local.contains('/'),
            "stage path should avoid mixed separators on Windows: {local}"
        );
        assert!(
            !remote.contains('/'),
            "stage path should avoid mixed separators on Windows: {remote}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_run_custom_mergetool_command_windows_executes_quoted_powershell_payload() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        let remote = workdir.join("remote.txt");
        let merged = workdir.join("merged.txt");
        std::fs::write(&remote, b"theirs\n").unwrap();

        let output = run_custom_mergetool_command(
            r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllBytes($env:MERGED, [System.IO.File]::ReadAllBytes($env:REMOTE))""#,
            workdir,
            Path::new("base.txt"),
            Path::new("local.txt"),
            Path::new("remote.txt"),
            &merged,
        )
        .unwrap();

        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8(output.stderr.clone())
                .unwrap_or_else(|_| "<non-utf8 stderr>".to_string())
        );
        assert_eq!(std::fs::read(&merged).unwrap(), b"theirs\n");
        let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
        assert!(
            !stdout.contains("[System.IO.File]"),
            "powershell payload should execute, not be echoed as a string expression"
        );
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

    #[cfg(windows)]
    #[test]
    fn test_shell_command_windows_uses_cmd_percent_expansion() {
        let output = shell_command("echo %GITCOMET_MERGETOOL_TEST_TOKEN%")
            .env("GITCOMET_MERGETOOL_TEST_TOKEN", "from-cmd")
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
        assert!(
            stdout.contains("from-cmd"),
            "expected cmd percent expansion in stdout"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn test_shell_command_unix_uses_sh_dollar_expansion() {
        let output = shell_command("echo \"$GITCOMET_MERGETOOL_TEST_TOKEN\"")
            .env("GITCOMET_MERGETOOL_TEST_TOKEN", "from-sh")
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
        assert!(
            stdout.contains("from-sh"),
            "expected sh dollar expansion in stdout"
        );
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

        let repo = open_repo(workdir);
        let result = git_config_get_bool(&repo, "nonexistent.bool.key").unwrap();
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
        let repo = open_repo(workdir);
        assert_eq!(
            git_config_get_bool(&repo, "mergetool.test.trustExitCode").unwrap(),
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
        let repo = open_repo(workdir);
        assert_eq!(
            git_config_get_bool(&repo, "mergetool.test.trustExitCode").unwrap(),
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

        let repo = open_repo(workdir);
        let err = git_config_get_bool(&repo, "mergetool.test.trustExitCode").unwrap_err();
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

        let repo = open_repo(workdir);
        assert_eq!(
            git_config_get_bool(&repo, "mergetool.test.trustExitCode").unwrap(),
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

        let repo = open_repo(workdir);
        let cfg = resolve_mergetool_config(&repo, false).unwrap();
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

        let repo = open_repo(workdir);
        let cfg = resolve_mergetool_config(&repo, false).unwrap();
        assert_eq!(cfg.tool_name, "cli");
        assert_eq!(cfg.tool_cmd, None);
        assert!(!cfg.write_to_temp);
        assert!(!cfg.keep_temporaries);
    }

    #[test]
    fn test_resolve_mergetool_config_trust_exit_code_falls_back_to_global_setting() {
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
            .args(["config", "mergetool.trustExitCode", "true"])
            .output()
            .unwrap();

        let repo = open_repo(workdir);
        let cfg = resolve_mergetool_config(&repo, false).unwrap();
        assert!(cfg.trust_exit_code);
    }

    #[test]
    fn test_resolve_mergetool_config_tool_specific_trust_exit_overrides_global() {
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
            .args(["config", "mergetool.trustExitCode", "true"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["config", "mergetool.cli.trustExitCode", "false"])
            .output()
            .unwrap();

        let repo = open_repo(workdir);
        let cfg = resolve_mergetool_config(&repo, false).unwrap();
        assert!(!cfg.trust_exit_code);
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
            .args(["config", "mergetool.writeToTemp", "true"])
            .output()
            .unwrap();

        let repo = open_repo(workdir);
        let cfg = resolve_mergetool_config(&repo, false).unwrap();
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
            .args(["config", "mergetool.keepTemporaries", "true"])
            .output()
            .unwrap();

        let repo = open_repo(workdir);
        let cfg = resolve_mergetool_config(&repo, false).unwrap();
        assert!(!cfg.write_to_temp);
        assert!(cfg.keep_temporaries);
    }
}
