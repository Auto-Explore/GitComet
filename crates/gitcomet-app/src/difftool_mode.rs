use crate::cli::{DifftoolConfig, DifftoolInputKind, classify_difftool_input, exit_code};
use crate::git_root::find_git_root;
use gitcomet_core::process::{bytes_to_text_preserving_utf8, git_command};
use rustc_hash::FxHashSet;
use std::fs;
use std::path::{Path, PathBuf};
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

#[cfg(feature = "ui-gpui-runtime")]
pub(crate) fn focused_input(
    config: &DifftoolConfig,
    patch: String,
) -> gitcomet_ui_gpui::FocusedDiffInput {
    use gitcomet_ui_gpui::FocusedDiffInput;
    if patch
        .lines()
        .any(|line| line.starts_with("old mode ") || line.starts_with("new mode "))
    {
        return FocusedDiffInput::Patch(patch);
    }
    match (
        focused_source(&config.local),
        focused_source(&config.remote),
    ) {
        (Ok(Some(old)), Ok(Some(new)))
            if old.starts_with('\u{feff}') == new.starts_with('\u{feff}') =>
        {
            FocusedDiffInput::FilePair {
                path: config
                    .display_path
                    .as_ref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| config.remote.clone()),
                old: old.into(),
                new: new.into(),
            }
        }
        (old, new) => {
            for error in [old, new].into_iter().filter_map(Result::err) {
                log::warn!("Focused diff uses patch input: {error}");
            }
            FocusedDiffInput::Patch(patch)
        }
    }
}

#[cfg(any(feature = "ui-gpui-runtime", test))]
fn focused_source(path: &Path) -> std::io::Result<Option<String>> {
    use std::io::Read;
    let limit = gitcomet_core::patch_action::SOURCE_BYTE_LIMIT;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit || bytes.contains(&0) {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok())
}

/// Execute difftool mode by delegating to `git diff --no-index`.
///
/// Git exits with code `1` when files differ, which is not an operational
/// failure for a diff tool. We normalize both `0` (no diff) and `1` (diff
/// present) to process success for the app-level contract.
pub fn run_difftool(config: &DifftoolConfig) -> Result<DifftoolRunResult, String> {
    let prepared_inputs = prepare_diff_inputs(config)?;

    let mut cmd = git_command();
    cmd.arg("diff")
        .arg("--no-index")
        .arg("--no-ext-diff")
        .arg("--no-color");
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
    // `--no-index` exits 1 both when the inputs differ and when it refuses a
    // path before diffing. A refusal prints no diff and says why on stderr; a
    // difference prints a diff, or nothing when textconv makes both sides equal.
    let differs = !output.stdout.is_empty() || output.stderr.is_empty();
    let mut stdout = decode_unified_diff_output(output.stdout);
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
        Some(1) if differs => Ok(DifftoolRunResult {
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

/// `git diff` output as text, each file section decoded in the encoding its
/// bytes read as; UTF-8 passes through untouched.
fn decode_unified_diff_output(bytes: Vec<u8>) -> String {
    let bytes = match String::from_utf8(bytes) {
        Ok(text) => return text,
        Err(err) => err.into_bytes(),
    };
    let terminated = bytes.ends_with(b"\n");
    let diff = gitcomet_core::domain::Diff::from_unified_bytes(
        gitcomet_core::domain::DiffTarget::working_tree(
            PathBuf::new(),
            gitcomet_core::domain::DiffArea::Unstaged,
        ),
        bytes,
        gitcomet_core::domain::DiffSectionFormats::sniff,
    );
    // Parsed lines drop their CR and newline; put back what the bytes had.
    let mut text = String::new();
    for (ix, line) in diff.lines.iter().enumerate() {
        text.push_str(line.text.as_ref());
        if line.text.raw_bytes().ends_with(b"\r") {
            text.push('\r');
        }
        if terminated || ix + 1 < diff.lines.len() {
            text.push('\n');
        }
    }
    text
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
    active_dirs: FxHashSet<PathBuf>,
    staged_entries: usize,
    staged_files: usize,
    staged_bytes: u64,
}

impl StagingCopyState {
    fn new(allowed_roots: Vec<PathBuf>) -> Self {
        Self {
            allowed_roots,
            active_dirs: FxHashSet::default(),
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
mod tests;
