use crate::cli::{DifftoolConfig, DifftoolInputKind, classify_difftool_input, exit_code};
use gitcomet_core::process::configure_background_command;
use rustc_hash::FxHashSet as HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{Builder, TempDir};

/// Format a `"Failed to {op} {path}: {err}"` message concisely.
macro_rules! io_err {
    ($op:literal, $err:expr) => {
        format!(concat!("Failed to ", $op, ": {}"), $err)
    };
    ($op:literal, $path:expr, $err:expr) => {
        format!(
            concat!("Failed to ", $op, " {}: {}"),
            ($path).display(),
            $err
        )
    };
    ($op:literal, $src:expr, $prep:literal, $dst:expr, $err:expr) => {
        format!(
            concat!("Failed to ", $op, " {} ", $prep, " {}: {}"),
            ($src).display(),
            ($dst).display(),
            $err
        )
    };
}

/// Result of running the dedicated difftool mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DifftoolRunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Execute difftool mode by delegating to `git diff --no-index`.
///
/// Git exits with code `1` when files differ, which is not an operational
/// failure for a diff tool. We normalize both `0` (no diff) and `1` (diff
/// present) to process success for the app-level contract.
pub fn run_difftool(config: &DifftoolConfig) -> Result<DifftoolRunResult, String> {
    let prepared_inputs = prepare_diff_inputs(config)?;

    let mut cmd = Command::new("git");
    configure_background_command(&mut cmd);
    cmd.arg("diff").arg("--no-index").arg("--no-ext-diff");
    // When launched from `git difftool`, Git sets `GIT_EXTERNAL_DIFF` to its
    // helper. Remove it so this nested `git diff --no-index` cannot recurse.
    cmd.env_remove("GIT_EXTERNAL_DIFF");
    let labels = resolve_labels(config);

    cmd.arg("--")
        .arg(&prepared_inputs.local)
        .arg(&prepared_inputs.remote);

    let output = cmd
        .output()
        .map_err(|e| io_err!("launch `git diff --no-index`", e))?;

    let status_code = output.status.code();
    let mut stdout = bytes_to_text_preserving_utf8(&output.stdout);
    let stderr = bytes_to_text_preserving_utf8(&output.stderr);

    if let Some((left, right)) = labels {
        stdout = apply_labels_to_unified_diff_headers(&stdout, &left, &right);
    }

    match status_code {
        Some(0) => Ok(DifftoolRunResult {
            stdout,
            stderr,
            exit_code: exit_code::SUCCESS,
        }),
        Some(1) if !has_git_error_prefix(&stderr) => Ok(DifftoolRunResult {
            stdout,
            stderr,
            exit_code: exit_code::SUCCESS,
        }),
        Some(code) => {
            let detail = stderr.trim();
            if detail.is_empty() {
                Err(format!(
                    "`git diff --no-index` failed with exit code {code}"
                ))
            } else {
                Err(format!(
                    "`git diff --no-index` failed with exit code {code}: {detail}"
                ))
            }
        }
        None => Err("`git diff --no-index` terminated by signal".to_string()),
    }
}

fn bytes_to_text_preserving_utf8(bytes: &[u8]) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

    let mut out = String::with_capacity(bytes.len());
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match std::str::from_utf8(&bytes[cursor..]) {
            Ok(valid) => {
                out.push_str(valid);
                break;
            }
            Err(err) => {
                let valid_len = err.valid_up_to();
                if valid_len > 0 {
                    let valid = &bytes[cursor..cursor + valid_len];
                    out.push_str(
                        std::str::from_utf8(valid)
                            .expect("slice identified by valid_up_to must be valid UTF-8"),
                    );
                    cursor += valid_len;
                }

                let invalid_len = err.error_len().unwrap_or(1);
                let invalid_end = cursor.saturating_add(invalid_len).min(bytes.len());
                for &byte in &bytes[cursor..invalid_end] {
                    out.push('\\');
                    out.push('x');
                    out.push(HEX_DIGITS[(byte >> 4) as usize] as char);
                    out.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
                }
                cursor = invalid_end;
            }
        }
    }

    out
}

struct PreparedDiffInputs {
    local: PathBuf,
    remote: PathBuf,
    _tempdir: Option<TempDir>,
}

const MAX_STAGED_ENTRY_COUNT: usize = 100_000;
const MAX_STAGED_FILE_COUNT: usize = 50_000;
const MAX_STAGED_BYTE_COUNT: u64 = 512 * 1024 * 1024;
const MAX_STAGING_DEPTH: usize = 128;

struct StagingCopyState {
    allowed_roots: Vec<PathBuf>,
    active_dirs: HashSet<PathBuf>,
    staged_entries: usize,
    staged_files: usize,
    staged_bytes: u64,
}

impl StagingCopyState {
    fn new(allowed_roots: Vec<PathBuf>) -> Self {
        Self {
            allowed_roots,
            active_dirs: HashSet::default(),
            staged_entries: 0,
            staged_files: 0,
            staged_bytes: 0,
        }
    }

    fn ensure_within_allowed_roots(
        &self,
        canonical_path: &Path,
        source_path: &Path,
    ) -> Result<(), String> {
        if self
            .allowed_roots
            .iter()
            .any(|root| canonical_path.starts_with(root))
        {
            return Ok(());
        }

        let allowed_roots = self
            .allowed_roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "Refusing to dereference path outside allowed roots while staging directory diff inputs: {} resolved to {} (allowed roots: {allowed_roots})",
            source_path.display(),
            canonical_path.display()
        ))
    }

    fn record_entry(&mut self, path: &Path) -> Result<(), String> {
        self.staged_entries = self.staged_entries.checked_add(1).ok_or_else(|| {
            "Entry counter overflow while staging directory diff inputs".to_string()
        })?;
        if self.staged_entries > MAX_STAGED_ENTRY_COUNT {
            return Err(format!(
                "Refusing to stage directory diff inputs: exceeded entry limit ({MAX_STAGED_ENTRY_COUNT}) at {}",
                path.display()
            ));
        }
        Ok(())
    }

    fn record_staged_file(&mut self, path: &Path, bytes: u64) -> Result<(), String> {
        self.staged_files = self.staged_files.checked_add(1).ok_or_else(|| {
            "File counter overflow while staging directory diff inputs".to_string()
        })?;
        if self.staged_files > MAX_STAGED_FILE_COUNT {
            return Err(format!(
                "Refusing to stage directory diff inputs: exceeded file limit ({MAX_STAGED_FILE_COUNT}) at {}",
                path.display()
            ));
        }

        self.staged_bytes = self.staged_bytes.checked_add(bytes).ok_or_else(|| {
            "Byte counter overflow while staging directory diff inputs".to_string()
        })?;
        if self.staged_bytes > MAX_STAGED_BYTE_COUNT {
            return Err(format!(
                "Refusing to stage directory diff inputs: exceeded byte limit ({MAX_STAGED_BYTE_COUNT}) at {}",
                path.display()
            ));
        }

        Ok(())
    }
}

fn prepare_diff_inputs(config: &DifftoolConfig) -> Result<PreparedDiffInputs, String> {
    let local_kind = classify_difftool_input(&config.local, "Local")?;
    let remote_kind = classify_difftool_input(&config.remote, "Remote")?;

    if local_kind != remote_kind {
        return Err(format!(
            "Difftool input kind mismatch: local is a {} and remote is a {}. Use two files or two directories.",
            local_kind.display_name(),
            remote_kind.display_name()
        ));
    }

    if local_kind != DifftoolInputKind::Directory {
        return Ok(PreparedDiffInputs {
            local: config.local.clone(),
            remote: config.remote.clone(),
            _tempdir: None,
        });
    }

    let local_contains_symlink = directory_tree_contains_symlink(&config.local)?;
    let remote_contains_symlink = directory_tree_contains_symlink(&config.remote)?;
    if !local_contains_symlink && !remote_contains_symlink {
        return Ok(PreparedDiffInputs {
            local: config.local.clone(),
            remote: config.remote.clone(),
            _tempdir: None,
        });
    }

    let tempdir = Builder::new()
        .prefix("gitcomet-difftool-")
        .tempdir()
        .map_err(|e| io_err!("create temporary directory staging area", e))?;

    let staged_local = tempdir.path().join("left");
    let staged_remote = tempdir.path().join("right");
    let allowed_roots = resolve_allowed_staging_roots(&config.local, &config.remote)?;
    let mut staging_state = StagingCopyState::new(allowed_roots);
    copy_tree_dereferencing_symlinks(&config.local, &staged_local, &mut staging_state)?;
    copy_tree_dereferencing_symlinks(&config.remote, &staged_remote, &mut staging_state)?;

    Ok(PreparedDiffInputs {
        local: staged_local,
        remote: staged_remote,
        _tempdir: Some(tempdir),
    })
}

fn push_unique_root(roots: &mut Vec<PathBuf>, root: PathBuf) {
    if !roots.iter().any(|existing| existing == &root) {
        roots.push(root);
    }
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    let start_dir = if start.is_dir() {
        start
    } else {
        start.parent()?
    };
    for candidate in start_dir.ancestors() {
        if candidate.join(".git").exists() {
            return Some(fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf()));
        }
    }
    None
}

fn resolve_allowed_staging_roots(local: &Path, remote: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = Vec::new();

    for (label, path) in [("local", local), ("remote", remote)] {
        let canonical = fs::canonicalize(path).map_err(|e| {
            format!(
                "Failed to resolve {label} directory {} while preparing directory diff staging boundaries: {e}",
                path.display()
            )
        })?;
        push_unique_root(&mut roots, canonical.clone());
        if let Some(repo_root) = find_git_root(&canonical) {
            push_unique_root(&mut roots, repo_root);
        }
    }

    if let Ok(cwd) = std::env::current_dir()
        && let Some(repo_root) = find_git_root(&cwd)
    {
        push_unique_root(&mut roots, repo_root);
    }

    Ok(roots)
}

fn directory_tree_contains_symlink(path: &Path) -> Result<bool, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| io_err!("read metadata for directory diff input", path, e))?;
    if metadata.file_type().is_symlink() {
        return Ok(true);
    }
    directory_tree_contains_symlink_inner(path, 0)
}

fn directory_tree_contains_symlink_inner(path: &Path, depth: usize) -> Result<bool, String> {
    if depth > MAX_STAGING_DEPTH {
        return Err(format!(
            "Refusing to inspect directory diff input for symlinks: exceeded recursion depth limit ({MAX_STAGING_DEPTH}) at {}",
            path.display()
        ));
    }

    let entries = fs::read_dir(path).map_err(|e| {
        format!(
            "Failed to read directory {} while inspecting for symlinks: {e}",
            path.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| io_err!("read entry in", path, e))?;
        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| io_err!("read file type for", entry_path, e))?;

        if file_type.is_symlink() {
            return Ok(true);
        }

        if file_type.is_dir() && directory_tree_contains_symlink_inner(&entry_path, depth + 1)? {
            return Ok(true);
        }
    }

    Ok(false)
}

fn copy_tree_dereferencing_symlinks(
    src: &Path,
    dst: &Path,
    staging_state: &mut StagingCopyState,
) -> Result<(), String> {
    copy_tree_dereferencing_symlinks_inner(src, dst, staging_state, 0)
}

fn copy_tree_dereferencing_symlinks_inner(
    src: &Path,
    dst: &Path,
    staging_state: &mut StagingCopyState,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_STAGING_DEPTH {
        return Err(format!(
            "Refusing to stage directory diff inputs: exceeded recursion depth limit ({MAX_STAGING_DEPTH}) at {}",
            src.display()
        ));
    }

    let canonical_src = fs::canonicalize(src).map_err(|e| {
        format!(
            "Failed to resolve directory {} while staging directory diff inputs: {e}",
            src.display()
        )
    })?;
    staging_state.ensure_within_allowed_roots(&canonical_src, src)?;
    if !staging_state.active_dirs.insert(canonical_src.clone()) {
        return Err(format!(
            "Detected symlink cycle while staging directory diff inputs at {}",
            src.display()
        ));
    }

    let result = copy_tree_dereferencing_symlinks_impl(src, dst, staging_state, depth);
    staging_state.active_dirs.remove(&canonical_src);
    result
}

fn copy_tree_dereferencing_symlinks_impl(
    src: &Path,
    dst: &Path,
    staging_state: &mut StagingCopyState,
    depth: usize,
) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| io_err!("create staged directory", dst, e))?;

    let entries = fs::read_dir(src).map_err(|e| io_err!("read directory", src, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_err!("read entry in", src, e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        staging_state.record_entry(&src_path)?;
        let file_type = entry
            .file_type()
            .map_err(|e| io_err!("read file type for", src_path, e))?;

        if file_type.is_dir() {
            copy_tree_dereferencing_symlinks_inner(&src_path, &dst_path, staging_state, depth + 1)?;
            continue;
        }

        if file_type.is_symlink() {
            copy_symlink_target_contents(&src_path, &dst_path, staging_state, depth + 1)?;
            continue;
        }

        if file_type.is_file() {
            let metadata = entry
                .metadata()
                .map_err(|e| io_err!("read metadata for", src_path, e))?;
            staging_state.record_staged_file(&src_path, metadata.len())?;
            fs::copy(&src_path, &dst_path)
                .map_err(|e| io_err!("stage file", src_path, "to", dst_path, e))?;
            continue;
        }

        return Err(format!(
            "Unsupported entry type at {} while staging directory diff inputs",
            src_path.display()
        ));
    }

    Ok(())
}

fn copy_symlink_target_contents(
    link_path: &Path,
    dst_path: &Path,
    staging_state: &mut StagingCopyState,
    depth: usize,
) -> Result<(), String> {
    let target =
        fs::read_link(link_path).map_err(|e| io_err!("read symlink target", link_path, e))?;
    let resolved_target = if target.is_absolute() {
        target.clone()
    } else {
        link_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(&target)
    };

    let canonical_target = match fs::canonicalize(&resolved_target) {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            staging_state
                .record_staged_file(link_path, serialized_symlink_target_byte_len(&target))?;
            write_symlink_target(dst_path, &target).map_err(|e| {
                io_err!(
                    "materialize unresolved symlink",
                    link_path,
                    "into",
                    dst_path,
                    e
                )
            })?;
            return Ok(());
        }
        Err(e) => {
            return Err(io_err!("resolve symlink target", link_path, e));
        }
    };
    staging_state.ensure_within_allowed_roots(&canonical_target, link_path)?;

    match fs::metadata(&canonical_target) {
        Ok(meta) if meta.is_file() => {
            staging_state.record_staged_file(&canonical_target, meta.len())?;
            fs::copy(&canonical_target, dst_path).map_err(|e| {
                io_err!(
                    "stage symlink target file",
                    canonical_target,
                    "to",
                    dst_path,
                    e
                )
            })?;
        }
        Ok(meta) if meta.is_dir() => {
            copy_tree_dereferencing_symlinks_inner(
                &canonical_target,
                dst_path,
                staging_state,
                depth,
            )?;
        }
        _ => {
            staging_state
                .record_staged_file(link_path, serialized_symlink_target_byte_len(&target))?;
            write_symlink_target(dst_path, &target).map_err(|e| {
                io_err!(
                    "materialize unresolved symlink",
                    link_path,
                    "into",
                    dst_path,
                    e
                )
            })?;
        }
    }

    Ok(())
}

fn serialized_symlink_target_byte_len(target: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        target.as_os_str().as_bytes().len() as u64
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        target
            .as_os_str()
            .encode_wide()
            .count()
            .saturating_mul(std::mem::size_of::<u16>()) as u64
    }
    #[cfg(not(any(unix, windows)))]
    {
        target.as_os_str().len() as u64
    }
}

/// Write symlink target path bytes to a file, preserving non-UTF-8 content on Unix.
fn write_symlink_target(dst: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        fs::write(dst, target.as_os_str().as_bytes())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut bytes = Vec::new();
        for unit in target.as_os_str().encode_wide() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(dst, bytes)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let Some(path_text) = target.to_str() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "symlink target path is not valid Unicode on this platform",
            ));
        };
        fs::write(dst, path_text.as_bytes())
    }
}

fn has_git_error_prefix(stderr: &str) -> bool {
    stderr
        .lines()
        .map(str::trim_start)
        .any(|line| line.starts_with("error:") || line.starts_with("fatal:"))
}

fn push_labeled_header_line(out: &mut String, prefix: &str, label: &str, segment: &str) {
    out.push_str(prefix);
    out.push(' ');
    out.push_str(label);
    if segment.ends_with('\n') {
        out.push('\n');
    }
}

fn apply_labels_to_unified_diff_headers(diff: &str, left: &str, right: &str) -> String {
    let mut out = String::with_capacity(diff.len());
    // Rewrite only file header lines (`---` / `+++`) and never hunk payload.
    // Hunk lines can legally begin with `--- ` / `+++ ` when content itself
    // starts with `-- ` / `++ `, and those must remain untouched.
    let mut in_hunk = false;
    let mut awaiting_old_header = true;
    let mut awaiting_new_header = false;

    for segment in diff.split_inclusive('\n') {
        if segment.starts_with("diff --git ") {
            in_hunk = false;
            awaiting_old_header = true;
            awaiting_new_header = false;
            out.push_str(segment);
            continue;
        }

        if segment.starts_with("@@ ") {
            in_hunk = true;
            awaiting_old_header = false;
            awaiting_new_header = false;
            out.push_str(segment);
            continue;
        }

        if !in_hunk && awaiting_old_header && segment.starts_with("--- ") {
            push_labeled_header_line(&mut out, "---", left, segment);
            awaiting_old_header = false;
            awaiting_new_header = true;
            continue;
        }

        if !in_hunk && awaiting_new_header && segment.starts_with("+++ ") {
            push_labeled_header_line(&mut out, "+++", right, segment);
            awaiting_new_header = false;
            continue;
        }

        out.push_str(segment);
    }

    if !diff.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }

    out
}

fn resolve_labels(config: &DifftoolConfig) -> Option<(String, String)> {
    let has_custom_labels = config.label_left.is_some() || config.label_right.is_some();
    let has_display_path = config.display_path.is_some();
    if !has_custom_labels && !has_display_path {
        return None;
    }

    let left_default = config
        .display_path
        .as_ref()
        .map(|path| format!("a/{path}"))
        .unwrap_or_else(|| config.local.display().to_string());
    let right_default = config
        .display_path
        .as_ref()
        .map(|path| format!("b/{path}"))
        .unwrap_or_else(|| config.remote.display().to_string());

    let left = config.label_left.clone().unwrap_or(left_default);
    let right = config.label_right.clone().unwrap_or(right_default);
    Some((left, right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_file(path: &std::path::Path, content: &str) {
        std::fs::write(path, content).expect("write fixture file");
    }

    fn write_bytes(path: &std::path::Path, content: &[u8]) {
        std::fs::write(path, content).expect("write fixture bytes");
    }

    fn config(local: PathBuf, remote: PathBuf) -> DifftoolConfig {
        DifftoolConfig {
            local,
            remote,
            display_path: None,
            label_left: None,
            label_right: None,
            gui: false,
        }
    }

    #[test]
    fn run_difftool_identical_files_returns_success_with_no_diff() {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left.txt");
        let right = tmp.path().join("right.txt");
        write_file(&left, "same\n");
        write_file(&right, "same\n");

        let result = run_difftool(&config(left, right)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.trim().is_empty(),
            "identical files should produce no stdout diff, got: {}",
            result.stdout
        );
    }

    #[test]
    fn run_difftool_changed_files_maps_git_exit_1_to_success() {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left.txt");
        let right = tmp.path().join("right.txt");
        write_file(&left, "left\n");
        write_file(&right, "right\n");

        let result = run_difftool(&config(left, right)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.contains("@@"),
            "expected a hunk in diff output"
        );
        assert!(result.stdout.contains("-left"));
        assert!(result.stdout.contains("+right"));
    }

    #[test]
    fn run_difftool_uses_display_path_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left.txt");
        let right = tmp.path().join("right.txt");
        write_file(&left, "left\n");
        write_file(&right, "right\n");

        let mut cfg = config(left, right);
        cfg.display_path = Some("src/lib.rs".to_string());

        let result = run_difftool(&cfg).expect("difftool run");
        assert!(result.stdout.contains("--- a/src/lib.rs"));
        assert!(result.stdout.contains("+++ b/src/lib.rs"));
    }

    #[test]
    fn run_difftool_uses_explicit_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left.txt");
        let right = tmp.path().join("right.txt");
        write_file(&left, "left\n");
        write_file(&right, "right\n");

        let mut cfg = config(left, right);
        cfg.label_left = Some("OURS".to_string());
        cfg.label_right = Some("THEIRS".to_string());

        let result = run_difftool(&cfg).expect("difftool run");
        assert!(result.stdout.contains("--- OURS"));
        assert!(result.stdout.contains("+++ THEIRS"));
    }

    #[test]
    fn run_difftool_nonexistent_input_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("missing.txt");
        let right = tmp.path().join("right.txt");
        write_file(&right, "right\n");

        let err = run_difftool(&config(left, right)).expect_err("expected error");
        assert!(
            err.contains("Local path does not exist")
                || err.contains("Failed to read metadata for local path")
                || err.contains("failed with exit code"),
            "unexpected error message: {err}"
        );
    }

    #[test]
    fn run_difftool_directory_diff_returns_success() {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        write_file(&left.join("a.txt"), "left\n");
        write_file(&right.join("a.txt"), "right\n");

        let result = run_difftool(&config(left, right)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.contains("a.txt"),
            "expected filename in dir diff output, got: {}",
            result.stdout
        );
    }

    #[test]
    fn prepare_diff_inputs_directory_without_symlinks_skips_staging_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        write_file(&left.join("a.txt"), "left\n");
        write_file(&right.join("a.txt"), "right\n");

        let prepared =
            prepare_diff_inputs(&config(left.clone(), right.clone())).expect("prepare inputs");
        assert_eq!(prepared.local, left);
        assert_eq!(prepared.remote, right);
        assert!(prepared._tempdir.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn prepare_diff_inputs_directory_with_symlink_uses_staging_copy() {
        use std::os::unix::fs as unix_fs;

        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        write_file(&left.join("a.txt"), "before\n");
        write_file(&right.join("target.txt"), "after\n");
        unix_fs::symlink(right.join("target.txt"), right.join("a.txt"))
            .expect("create symlink in right dir");

        let prepared =
            prepare_diff_inputs(&config(left.clone(), right.clone())).expect("prepare inputs");
        assert_ne!(prepared.local, left);
        assert_ne!(prepared.remote, right);
        assert!(prepared._tempdir.is_some());

        let tempdir = prepared._tempdir.as_ref().unwrap();
        assert!(prepared.local.starts_with(tempdir.path()));
        assert!(prepared.remote.starts_with(tempdir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn run_difftool_directory_diff_dereferences_symlinked_files() {
        use std::os::unix::fs as unix_fs;

        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();

        write_file(&left.join("a.txt"), "before\n");
        write_file(&right.join("target.txt"), "after\n");
        unix_fs::symlink(right.join("target.txt"), right.join("a.txt"))
            .expect("create symlink in right dir");

        let result = run_difftool(&config(left, right)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.contains("-before") && result.stdout.contains("+after"),
            "expected dereferenced content diff, got: {}",
            result.stdout
        );
        assert!(
            !result.stdout.contains("new file mode 120000"),
            "did not expect symlink mode-only diff, got: {}",
            result.stdout
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_difftool_directory_symlink_inputs_use_directory_content_diff() {
        use std::os::unix::fs as unix_fs;

        let tmp = tempfile::tempdir().unwrap();
        let left_dir = tmp.path().join("left");
        let right_dir = tmp.path().join("right");
        let left_link = tmp.path().join("left-link");
        let right_link = tmp.path().join("right-link");

        std::fs::create_dir_all(&left_dir).unwrap();
        std::fs::create_dir_all(&right_dir).unwrap();
        write_file(&left_dir.join("a.txt"), "before\n");
        write_file(&right_dir.join("a.txt"), "after\n");
        unix_fs::symlink(&left_dir, &left_link).expect("create symlink to left directory");
        unix_fs::symlink(&right_dir, &right_link).expect("create symlink to right directory");

        let result = run_difftool(&config(left_link, right_link)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.contains("-before") && result.stdout.contains("+after"),
            "expected staged directory content diff, got: {}",
            result.stdout
        );
        assert!(
            !result.stdout.contains("new file mode 120000"),
            "did not expect top-level symlink-mode-only diff, got: {}",
            result.stdout
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_difftool_directory_diff_rejects_symlink_cycles() {
        use std::os::unix::fs as unix_fs;

        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();

        write_file(&left.join("a.txt"), "left\n");
        write_file(&right.join("a.txt"), "right\n");
        unix_fs::symlink(".", right.join("loop")).expect("create self-referential symlink");

        let err = run_difftool(&config(left, right)).expect_err("expected symlink cycle error");
        assert!(
            err.contains("symlink cycle"),
            "expected cycle-specific error, got: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_difftool_directory_diff_rejects_symlink_target_outside_allowed_roots() {
        use std::os::unix::fs as unix_fs;

        let tmp = tempfile::tempdir().unwrap();
        let outside_tmp = tempfile::Builder::new()
            .prefix("gitcomet-difftool-outside-")
            .tempdir()
            .expect("create outside temp dir");
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        let outside = outside_tmp.path().join("outside");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        write_file(&left.join("a.txt"), "left\n");
        write_file(&outside.join("secret.txt"), "outside\n");
        unix_fs::symlink(outside.join("secret.txt"), right.join("a.txt"))
            .expect("create out-of-bound symlink");

        let err = run_difftool(&config(left, right)).expect_err("expected allowed-root error");
        assert!(
            err.contains("outside allowed roots"),
            "expected allowed-root-specific error, got: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_difftool_directory_diff_allows_symlink_target_within_detected_git_root() {
        use std::os::unix::fs as unix_fs;

        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("repo");
        let left = repo_root.join("left");
        let right = repo_root.join("right");
        let shared = repo_root.join("shared");
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        std::fs::create_dir_all(&shared).unwrap();

        write_file(&left.join("a.txt"), "before\n");
        write_file(&shared.join("target.txt"), "after\n");
        unix_fs::symlink(shared.join("target.txt"), right.join("a.txt"))
            .expect("create in-repo symlink");

        let result = run_difftool(&config(left, right)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.contains("-before") && result.stdout.contains("+after"),
            "expected dereferenced in-repo symlink content diff, got: {}",
            result.stdout
        );
    }

    #[test]
    fn run_difftool_binary_content_returns_success() {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left.bin");
        let right = tmp.path().join("right.bin");
        write_bytes(&left, &[0x00, 0x01, 0x02, 0x03]);
        write_bytes(&right, &[0x00, 0x01, 0xFF, 0x03]);

        let result = run_difftool(&config(left, right)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.contains("Binary files")
                || result.stdout.contains("GIT binary patch")
                || result.stdout.contains("differ"),
            "expected binary diff output, got: {}",
            result.stdout
        );
    }

    #[test]
    fn run_difftool_non_utf8_text_content_returns_success() {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left.dat");
        let right = tmp.path().join("right.dat");
        write_bytes(&left, b"prefix\n\xFF\n");
        write_bytes(&right, b"prefix\n\xFE\n");

        let result = run_difftool(&config(left, right)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            !result.stdout.trim().is_empty(),
            "expected non-empty diff output for non-UTF8 content"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_difftool_directory_diff_preserves_non_utf8_broken_symlink_target_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs as unix_fs;

        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();

        // Non-UTF-8 bytes that to_string_lossy() would corrupt.
        let non_utf8_bytes = b"target-\xff-\xfe";
        let non_utf8_target = OsStr::from_bytes(non_utf8_bytes);

        // Left side: broken symlink with non-UTF-8 target (materialized as target bytes).
        unix_fs::symlink(non_utf8_target, left.join("entry")).expect("create non-UTF-8 symlink");

        // Right side: file containing the exact same raw bytes.
        write_bytes(&right.join("entry"), non_utf8_bytes);

        let result = run_difftool(&config(left.clone(), right)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.trim().is_empty(),
            "expected no diff when broken symlink target bytes match file content, got: {}",
            result.stdout
        );
    }

    #[test]
    fn apply_labels_rewrites_unified_headers_only() {
        let input = "diff --git a/l b/r\n--- a/l\n+++ b/r\n@@ -1 +1 @@\n-a\n+b\n";
        let got = apply_labels_to_unified_diff_headers(input, "LEFT", "RIGHT");
        assert!(got.contains("diff --git a/l b/r"));
        assert!(got.contains("--- LEFT"));
        assert!(got.contains("+++ RIGHT"));
        assert!(got.contains("@@ -1 +1 @@"));
    }

    #[test]
    fn apply_labels_does_not_rewrite_hunk_content_that_looks_like_headers() {
        let input = "diff --git a/l b/r\n--- a/l\n+++ b/r\n@@ -1 +1 @@\n--- content\n+++ content\n";
        let got = apply_labels_to_unified_diff_headers(input, "LEFT", "RIGHT");
        assert!(got.contains("--- LEFT\n+++ RIGHT\n"));
        assert!(got.contains("@@ -1 +1 @@\n--- content\n+++ content\n"));
    }

    #[test]
    fn apply_labels_rewrites_each_file_header_pair_in_multi_file_diff() {
        let input = concat!(
            "diff --git a/l b/l\n",
            "--- a/l\n",
            "+++ b/l\n",
            "@@ -1 +1 @@\n",
            "-a\n",
            "+b\n",
            "diff --git a/r b/r\n",
            "--- a/r\n",
            "+++ b/r\n",
            "@@ -1 +1 @@\n",
            "-c\n",
            "+d\n",
        );
        let got = apply_labels_to_unified_diff_headers(input, "LEFT", "RIGHT");
        assert_eq!(got.matches("--- LEFT").count(), 2);
        assert_eq!(got.matches("+++ RIGHT").count(), 2);
    }

    #[test]
    fn has_git_error_prefix_detects_error_and_fatal_diagnostics() {
        assert!(has_git_error_prefix("error: unable to read file"));
        assert!(has_git_error_prefix(
            "warning: context\nfatal: cannot stat '/tmp/missing'"
        ));
    }

    #[test]
    fn has_git_error_prefix_ignores_non_error_output() {
        assert!(!has_git_error_prefix(
            "diff --git a/file b/file\nindex 1..2 100644"
        ));
        assert!(!has_git_error_prefix("warning: textconv failed"));
    }

    // ── Subdirectory invocation ─────────────────────────────────────

    #[test]
    fn run_difftool_files_in_nested_subdirectory() {
        // Simulates `git difftool` invoked from a subdirectory: diff inputs
        // are in a nested path, not the root temp directory.
        let tmp = tempfile::tempdir().unwrap();
        let subdir = tmp.path().join("src").join("components");
        std::fs::create_dir_all(&subdir).unwrap();

        let left = subdir.join("widget_LOCAL.txt");
        let right = subdir.join("widget_REMOTE.txt");
        write_file(&left, "original line\n");
        write_file(&right, "changed line\n");

        let result = run_difftool(&config(left, right)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.contains("original line") || result.stdout.contains("changed line"),
            "diff output should contain file content: {}",
            result.stdout
        );
    }

    #[test]
    fn run_difftool_files_in_different_directories() {
        // Input files are in different directories, simulating writeToTemp
        // mode where Git places stage files in a temp directory.
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = tmp.path().join("stages");
        let dir_b = tmp.path().join("workdir").join("src");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let left = dir_a.join("file_LOCAL_12345.txt");
        let right = dir_b.join("file.txt");
        write_file(&left, "before\n");
        write_file(&right, "after\n");

        let result = run_difftool(&config(left, right)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            !result.stdout.is_empty(),
            "diff should produce output for different files"
        );
    }

    #[test]
    fn run_difftool_directory_diff_in_nested_subdirectories() {
        // Directory diff mode with directories nested inside subdirectories.
        let tmp = tempfile::tempdir().unwrap();
        let left_dir = tmp.path().join("workspace").join("left");
        let right_dir = tmp.path().join("workspace").join("right");
        std::fs::create_dir_all(&left_dir).unwrap();
        std::fs::create_dir_all(&right_dir).unwrap();

        write_file(&left_dir.join("main.rs"), "fn main() {}\n");
        write_file(
            &right_dir.join("main.rs"),
            "fn main() { println!(\"hello\"); }\n",
        );

        let result = run_difftool(&config(left_dir, right_dir)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.contains("main.rs"),
            "directory diff output should mention the changed file: {}",
            result.stdout
        );
    }

    #[test]
    fn bytes_to_text_preserving_utf8_handles_pure_ascii() {
        assert_eq!(bytes_to_text_preserving_utf8(b"hello world"), "hello world");
    }

    #[test]
    fn bytes_to_text_preserving_utf8_handles_valid_utf8() {
        let input = "café ñ 日本語".as_bytes();
        assert_eq!(bytes_to_text_preserving_utf8(input), "café ñ 日本語");
    }

    #[test]
    fn bytes_to_text_preserving_utf8_escapes_invalid_bytes() {
        let input = b"good\xff\xfebad";
        let result = bytes_to_text_preserving_utf8(input);
        assert_eq!(result, "good\\xff\\xfebad");
    }

    #[test]
    fn bytes_to_text_preserving_utf8_handles_empty() {
        assert_eq!(bytes_to_text_preserving_utf8(b""), "");
    }

    #[test]
    fn bytes_to_text_preserving_utf8_escapes_leading_invalid() {
        let result = bytes_to_text_preserving_utf8(b"\x80rest");
        assert_eq!(result, "\\x80rest");
    }
}
