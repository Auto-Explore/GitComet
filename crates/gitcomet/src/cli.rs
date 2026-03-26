//! CLI argument parsing for gitcomet.
//!
//! Supports six modes:
//! - Default (no subcommand): open the full repository browser
//! - `difftool`: focused diff view, compatible with `git difftool`
//! - `mergetool`: focused merge view, compatible with `git mergetool`
//! - `setup`: configure git difftool/mergetool integration
//! - `uninstall`: remove gitcomet difftool/mergetool integration
//! - `extract-merge-fixtures`: generate Phase 3C real-world merge fixtures

use clap::{Parser, Subcommand};
use gitcomet_core::merge::{ConflictStyle, DEFAULT_MARKER_SIZE, DiffAlgorithm};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

mod compat;
mod git_config;

/// Exit codes aligned with Git expectations (see external_usage.md).
pub mod exit_code {
    /// User completed action and result persisted to output target.
    pub const SUCCESS: i32 = 0;
    /// User canceled or closed with unresolved result.
    pub const CANCELED: i32 = 1;
    /// Input/IO/internal error.
    pub const ERROR: i32 = 2;
}

// ── Raw CLI argument structs (clap) ──────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "gitcomet", about = "Git GUI built with GPUI", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Path to a git repository to open (default mode only).
    #[arg(global = false)]
    pub path: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Open a focused diff view (for use as git difftool).
    Difftool(DifftoolArgs),
    /// Open a focused merge view (for use as git mergetool).
    Mergetool(MergetoolArgs),
    /// Configure git to use gitcomet as the global diff/merge tool.
    Setup(SetupArgs),
    /// Remove gitcomet diff/merge tool config entries.
    Uninstall(UninstallArgs),
    /// Extract non-trivial merge cases from git history as fixture files.
    ExtractMergeFixtures(ExtractMergeFixturesArgs),
}

#[derive(clap::Args, Debug)]
pub struct DifftoolArgs {
    /// Path to the local (left) file.
    #[arg(long)]
    pub local: Option<PathBuf>,
    /// Path to the remote (right) file.
    #[arg(long)]
    pub remote: Option<PathBuf>,
    /// Display name for the path being diffed.
    #[arg(long)]
    pub path: Option<String>,
    /// Label for the left pane.
    #[arg(long)]
    pub label_left: Option<String>,
    /// Label for the right pane.
    #[arg(long)]
    pub label_right: Option<String>,
    /// Open an interactive GPUI diff window instead of printing to stdout.
    #[arg(long)]
    pub gui: bool,
}

#[derive(clap::Args, Debug)]
pub struct MergetoolArgs {
    /// Path to the merged output file (required).
    ///
    /// Compatibility aliases:
    /// - `-o`
    /// - `--output`
    /// - `--out`
    #[arg(long, short = 'o', visible_aliases = ["output", "out"])]
    pub merged: Option<PathBuf>,
    /// Path to the local (ours) file (required).
    #[arg(long)]
    pub local: Option<PathBuf>,
    /// Path to the remote (theirs) file (required).
    #[arg(long)]
    pub remote: Option<PathBuf>,
    /// Path to the base (common ancestor) file; optional for add/add conflicts.
    #[arg(long)]
    pub base: Option<PathBuf>,
    /// Label for the base pane.
    ///
    /// Compatibility alias: `--L1` (KDiff3-style).
    #[arg(long, visible_alias = "L1")]
    pub label_base: Option<String>,
    /// Label for the local pane.
    ///
    /// Compatibility alias: `--L2` (KDiff3-style).
    #[arg(long, visible_alias = "L2")]
    pub label_local: Option<String>,
    /// Label for the remote pane.
    ///
    /// Compatibility alias: `--L3` (KDiff3-style).
    #[arg(long, visible_alias = "L3")]
    pub label_remote: Option<String>,
    /// Conflict marker style: merge (default), diff3, or zdiff3.
    #[arg(long, value_name = "STYLE")]
    pub conflict_style: Option<String>,
    /// Diff algorithm: myers (default) or histogram.
    #[arg(long, value_name = "ALGORITHM")]
    pub diff_algorithm: Option<String>,
    /// Conflict marker width (must be > 0). Default: 7.
    #[arg(long, value_name = "N")]
    pub marker_size: Option<usize>,
    /// Auto-resolve mode: attempt to resolve all conflicts automatically.
    ///
    /// When enabled, the mergetool applies heuristic passes after the initial
    /// 3-way merge: identical-side detection, single-side-change detection,
    /// whitespace-only normalization, and subchunk splitting. If ALL conflicts
    /// are resolved, exits 0 with clean output. If any remain, writes conflict
    /// markers and exits 1 as usual.
    ///
    /// For best results, combine with `--conflict-style diff3` or `zdiff3`
    /// to provide base content for heuristic resolution.
    ///
    /// Compatibility alias: `--auto-merge` (Meld-style).
    #[arg(long, visible_alias = "auto-merge")]
    pub auto: bool,
    /// Open an interactive GPUI merge window for conflict resolution.
    #[arg(long)]
    pub gui: bool,
}

// ── Validated configuration types ────────────────────────────────────

/// Validated difftool configuration ready for the UI layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DifftoolConfig {
    pub local: PathBuf,
    pub remote: PathBuf,
    pub display_path: Option<String>,
    pub label_left: Option<String>,
    pub label_right: Option<String>,
    pub gui: bool,
}

/// Validated mergetool configuration ready for the UI layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergetoolConfig {
    pub merged: PathBuf,
    pub local: PathBuf,
    pub remote: PathBuf,
    pub base: Option<PathBuf>,
    pub label_base: Option<String>,
    pub label_local: Option<String>,
    pub label_remote: Option<String>,
    pub conflict_style: ConflictStyle,
    pub diff_algorithm: DiffAlgorithm,
    pub marker_size: usize,
    pub auto: bool,
    pub gui: bool,
}

#[derive(clap::Args, Debug)]
pub struct SetupArgs {
    /// Only print the git config commands without running them.
    #[arg(long)]
    pub dry_run: bool,
    /// Apply config to the local repository instead of global.
    #[arg(long)]
    pub local: bool,
}

#[derive(clap::Args, Debug)]
pub struct UninstallArgs {
    /// Only print the git config commands that would be run.
    #[arg(long)]
    pub dry_run: bool,
    /// Remove config from the local repository instead of global.
    #[arg(long)]
    pub local: bool,
}

#[derive(clap::Args, Debug)]
pub struct ExtractMergeFixturesArgs {
    /// Repository to scan for merge commits (default: current directory).
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
    /// Destination directory for generated fixture files.
    #[arg(long)]
    pub out: PathBuf,
    /// Maximum number of merge commits to scan.
    #[arg(long, default_value_t = 20)]
    pub max_merges: usize,
    /// Maximum number of files extracted per merge commit.
    #[arg(long, default_value_t = 5)]
    pub max_files_per_merge: usize,
}

/// Validated extraction configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractMergeFixturesConfig {
    pub repo: PathBuf,
    pub output_dir: PathBuf,
    pub max_merges: usize,
    pub max_files_per_merge: usize,
}

/// Which mode the application was launched in.
#[derive(Clone, Debug)]
pub enum AppMode {
    /// Full repository browser (default).
    Browser { path: Option<PathBuf> },
    /// Focused diff view.
    Difftool(DifftoolConfig),
    /// Focused merge view.
    Mergetool(MergetoolConfig),
    /// Write git config for difftool/mergetool integration.
    Setup { dry_run: bool, local: bool },
    /// Remove gitcomet-specific difftool/mergetool integration.
    Uninstall { dry_run: bool, local: bool },
    /// Generate merge fixtures from repository history.
    ExtractMergeFixtures(ExtractMergeFixturesConfig),
}

// ── Environment lookup trait for testability ─────────────────────────

/// Abstraction over environment variable lookup. Production code uses
/// `ProcessEnv`; tests supply a closure-based implementation to avoid
/// calling the unsafe `set_var`/`remove_var` in edition 2024.
trait EnvLookup {
    fn var_os(&self, key: &str) -> Option<OsString>;
    fn var(&self, key: &str) -> Option<String> {
        self.var_os(key).and_then(|v| v.into_string().ok())
    }
}

/// Reads environment variables from the actual process environment.
struct ProcessEnv;

impl EnvLookup for ProcessEnv {
    fn var_os(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }
}

// ── Resolution + validation ──────────────────────────────────────────

/// Resolve a path from an explicit flag, falling back to an environment
/// variable. Returns `None` if neither is set.
fn resolve_path(flag: Option<PathBuf>, env_key: &str, env: &dyn EnvLookup) -> Option<PathBuf> {
    flag.or_else(|| env.var_os(env_key).map(PathBuf::from))
}

fn is_empty_path(path: &Path) -> bool {
    path.as_os_str().is_empty()
}

fn require_non_empty_path(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    if is_empty_path(&path) {
        return Err(format!(
            "Invalid {label} path: value is empty. Use a non-empty path."
        ));
    }
    Ok(path)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DifftoolInputKind {
    Directory,
    FileLike,
}

impl DifftoolInputKind {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            DifftoolInputKind::Directory => "directory",
            DifftoolInputKind::FileLike => "file",
        }
    }
}

/// Result of probing a filesystem path, distinguishing regular files, directories,
/// symlinks, and absent paths without repeating metadata lookup logic.
enum ResolvedPathKind {
    File,
    Directory,
    /// Exists but is neither file nor directory (e.g. socket, FIFO).
    /// `is_symlink` is true when the path itself is a symlink to such an entry.
    Special {
        is_symlink: bool,
    },
    /// Symlink whose target does not exist.
    BrokenSymlink,
    /// Path does not exist (not even as a symlink).
    NotFound,
    /// Failed to read metadata.
    Error(std::io::Error),
}

/// Probe a filesystem path once, returning a high-level classification.
/// Follows symlinks via `fs::metadata` first, then falls back to
/// `fs::symlink_metadata` when the target is missing or special.
fn classify_path(path: &Path) -> ResolvedPathKind {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => ResolvedPathKind::File,
        Ok(meta) if meta.is_dir() => ResolvedPathKind::Directory,
        Ok(_) => {
            let is_symlink = std::fs::symlink_metadata(path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            ResolvedPathKind::Special { is_symlink }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(path) {
                Ok(m) if m.file_type().is_symlink() => ResolvedPathKind::BrokenSymlink,
                Ok(m) if m.is_dir() => ResolvedPathKind::Directory,
                Ok(m) if m.is_file() => ResolvedPathKind::File,
                Ok(_) => ResolvedPathKind::Special { is_symlink: false },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => ResolvedPathKind::NotFound,
                Err(e) => ResolvedPathKind::Error(e),
            }
        }
        Err(e) => ResolvedPathKind::Error(e),
    }
}

/// Classify a difftool path as directory or file-like.
///
/// Symlink handling rules:
/// - symlink to directory => directory
/// - symlink to file => file-like
/// - broken symlink => file-like (for symlink conflict diffs)
pub(crate) fn classify_difftool_input(
    path: &Path,
    role_name: &str,
) -> Result<DifftoolInputKind, String> {
    match classify_path(path) {
        ResolvedPathKind::File | ResolvedPathKind::BrokenSymlink => Ok(DifftoolInputKind::FileLike),
        ResolvedPathKind::Directory => Ok(DifftoolInputKind::Directory),
        ResolvedPathKind::Special { is_symlink: true } => Err(format!(
            "{role_name} path symlink target must resolve to a regular file or directory: {}",
            path.display()
        )),
        ResolvedPathKind::Special { is_symlink: false } => Err(format!(
            "{role_name} path must be a regular file or directory: {}",
            path.display()
        )),
        ResolvedPathKind::NotFound => Err(format!(
            "{role_name} path does not exist: {}",
            path.display()
        )),
        ResolvedPathKind::Error(e) => Err(format!(
            "Failed to read metadata for {role_name} path {}: {e}",
            path.display()
        )),
    }
}

fn validate_regular_file_path(path: &Path, role_name: &str) -> Result<(), String> {
    let metadata = std::fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{role_name} path does not exist: {}", path.display())
        } else {
            format!(
                "Failed to read metadata for {role_name} path {}: {e}",
                path.display()
            )
        }
    })?;

    if metadata.is_file() {
        Ok(())
    } else if metadata.is_dir() {
        Err(format!(
            "{role_name} path must be a file, not a directory: {}",
            path.display()
        ))
    } else {
        Err(format!(
            "{role_name} path must be a regular file: {}",
            path.display()
        ))
    }
}

struct MergetoolPathArgs {
    merged: Option<PathBuf>,
    local: Option<PathBuf>,
    remote: Option<PathBuf>,
    base: Option<PathBuf>,
}

struct ResolvedMergetoolPaths {
    merged: PathBuf,
    local: PathBuf,
    remote: PathBuf,
    base: Option<PathBuf>,
}

fn resolve_required_mergetool_path(
    flag: Option<PathBuf>,
    flag_name: &'static str,
    env_key: &'static str,
    env: &dyn EnvLookup,
) -> Result<PathBuf, String> {
    let path = resolve_path(flag, env_key, env).ok_or_else(|| {
        format!("Missing required input: --{flag_name} flag or {env_key} environment variable")
    })?;
    require_non_empty_path(path, flag_name)
}

fn resolve_optional_base_path(base: Option<PathBuf>, env: &dyn EnvLookup) -> Option<PathBuf> {
    // Treat an empty BASE value as "no base" for compatibility with
    // shell-expanded custom tool commands like `--base "$BASE"`.
    match resolve_path(base, "BASE", env) {
        Some(path) if is_empty_path(&path) => None,
        other => other,
    }
}

fn resolve_and_validate_mergetool_paths(
    args: MergetoolPathArgs,
    env: &dyn EnvLookup,
) -> Result<ResolvedMergetoolPaths, String> {
    let paths = ResolvedMergetoolPaths {
        merged: resolve_required_mergetool_path(args.merged, "merged", "MERGED", env)?,
        local: resolve_required_mergetool_path(args.local, "local", "LOCAL", env)?,
        remote: resolve_required_mergetool_path(args.remote, "remote", "REMOTE", env)?,
        base: resolve_optional_base_path(args.base, env),
    };

    // MERGED is an output target and may not exist yet (e.g. standalone
    // --output/--out usage). If it already exists, it must resolve to a
    // regular file target.
    validate_existing_merged_output_path(&paths.merged)?;
    validate_regular_file_path(&paths.local, "Local")?;
    validate_regular_file_path(&paths.remote, "Remote")?;

    // Base is allowed to be missing (add/add conflicts have no base).
    // But if explicitly provided, it must resolve to a regular file.
    if let Some(base_path) = paths.base.as_deref() {
        validate_regular_file_path(base_path, "Base")?;
    }

    Ok(paths)
}

fn validate_existing_merged_output_path(path: &Path) -> Result<(), String> {
    match classify_path(path) {
        ResolvedPathKind::File | ResolvedPathKind::NotFound => Ok(()),
        ResolvedPathKind::Directory => Err(format!(
            "Merged path must be a file path, not a directory: {}",
            path.display()
        )),
        ResolvedPathKind::BrokenSymlink => Err(format!(
            "Merged path must resolve to an existing file target: {}",
            path.display()
        )),
        ResolvedPathKind::Special { .. } => Err(format!(
            "Merged path must be a regular file path: {}",
            path.display()
        )),
        ResolvedPathKind::Error(e) => Err(format!(
            "Failed to read metadata for merged path {}: {e}",
            path.display()
        )),
    }
}

/// Resolve and validate difftool arguments.
///
/// Priority: explicit `--local`/`--remote` flags, then `LOCAL`/`REMOTE` env vars.
/// Both local and remote must resolve to existing files or directories.
fn resolve_difftool_with_env(
    args: DifftoolArgs,
    env: &dyn EnvLookup,
) -> Result<DifftoolConfig, String> {
    let local = require_non_empty_path(
        resolve_path(args.local, "LOCAL", env)
            .ok_or("Missing required input: --local flag or LOCAL environment variable")?,
        "local",
    )?;

    let remote = require_non_empty_path(
        resolve_path(args.remote, "REMOTE", env)
            .ok_or("Missing required input: --remote flag or REMOTE environment variable")?,
        "remote",
    )?;

    let local_kind = classify_difftool_input(&local, "Local")?;
    let remote_kind = classify_difftool_input(&remote, "Remote")?;
    if local_kind != remote_kind {
        return Err(format!(
            "Difftool input kind mismatch: local is a {} and remote is a {}. Use two files or two directories.",
            local_kind.display_name(),
            remote_kind.display_name(),
        ));
    }

    // Display path: flag > MERGED env > BASE env (git difftool compat) > None.
    // Git custom difftool contracts historically pass MERGED and/or BASE as
    // optional compatibility variables; prefer MERGED when both are present.
    let display_path = args.path.filter(|value| !value.is_empty()).or_else(|| {
        env.var("MERGED")
            .filter(|value| !value.is_empty())
            .or_else(|| env.var("BASE").filter(|value| !value.is_empty()))
    });

    Ok(DifftoolConfig {
        local,
        remote,
        display_path,
        label_left: args.label_left,
        label_right: args.label_right,
        gui: args.gui,
    })
}

fn parse_marker_size(marker_size: Option<usize>) -> Result<usize, String> {
    match marker_size {
        None => Ok(DEFAULT_MARKER_SIZE),
        Some(0) => Err("Invalid marker size '0': expected a positive integer.".to_string()),
        Some(value) => Ok(value),
    }
}

fn parse_conflict_style(value: Option<&str>) -> Result<ConflictStyle, String> {
    match value {
        None | Some("merge") => Ok(ConflictStyle::Merge),
        Some("diff3") => Ok(ConflictStyle::Diff3),
        Some("zdiff3") => Ok(ConflictStyle::Zdiff3),
        Some(other) => Err(format!(
            "Unknown conflict style '{other}': expected merge, diff3, or zdiff3"
        )),
    }
}

fn parse_diff_algorithm(value: Option<&str>) -> Result<DiffAlgorithm, String> {
    match value {
        None | Some("myers") => Ok(DiffAlgorithm::Myers),
        Some("histogram") => Ok(DiffAlgorithm::Histogram),
        Some(other) => Err(format!(
            "Unknown diff algorithm '{other}': expected myers or histogram"
        )),
    }
}

/// Resolve and validate mergetool arguments.
///
/// Priority: explicit flags, then env vars (MERGED, LOCAL, REMOTE, BASE).
/// merged, local, and remote are required. base is optional.
fn resolve_mergetool_with_env(
    args: MergetoolArgs,
    env: &dyn EnvLookup,
) -> Result<MergetoolConfig, String> {
    let MergetoolArgs {
        merged,
        local,
        remote,
        base,
        label_base,
        label_local,
        label_remote,
        conflict_style,
        diff_algorithm,
        marker_size,
        auto,
        gui,
    } = args;
    let marker_size = parse_marker_size(marker_size)?;
    let ResolvedMergetoolPaths {
        merged,
        local,
        remote,
        base,
    } = resolve_and_validate_mergetool_paths(
        MergetoolPathArgs {
            merged,
            local,
            remote,
            base,
        },
        env,
    )?;
    let conflict_style = parse_conflict_style(conflict_style.as_deref())?;
    let diff_algorithm = parse_diff_algorithm(diff_algorithm.as_deref())?;

    Ok(MergetoolConfig {
        merged,
        local,
        remote,
        base,
        label_base,
        label_local,
        label_remote,
        conflict_style,
        diff_algorithm,
        marker_size,
        auto,
        gui,
    })
}

// ── Git config fallback ──────────────────────────────────────────────

/// Internal: resolve mergetool args with both env and git config fallback.
fn resolve_mergetool_with_config(
    args: MergetoolArgs,
    env: &dyn EnvLookup,
    git_config: &dyn Fn(&str) -> Option<String>,
) -> Result<MergetoolConfig, String> {
    git_config::resolve_mergetool_with_config(args, env, git_config)
}

fn resolve_extract_merge_fixtures(
    args: ExtractMergeFixturesArgs,
) -> Result<ExtractMergeFixturesConfig, String> {
    let repo = require_non_empty_path(args.repo, "repository")?;
    let output_dir = require_non_empty_path(args.out, "output directory")?;

    if args.max_merges == 0 {
        return Err("Invalid --max-merges value '0': expected a positive integer.".to_string());
    }
    if args.max_files_per_merge == 0 {
        return Err(
            "Invalid --max-files-per-merge value '0': expected a positive integer.".to_string(),
        );
    }

    Ok(ExtractMergeFixturesConfig {
        repo,
        output_dir,
        max_merges: args.max_merges,
        max_files_per_merge: args.max_files_per_merge,
    })
}

fn parse_compat_external_mode_with_config(
    raw_args: &[OsString],
    env: &dyn EnvLookup,
    git_config: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<AppMode>, String> {
    compat::parse_compat_external_mode_with_config(raw_args, env, git_config)
}

fn normalize_empty_mergetool_base_arg(args: &[OsString]) -> Vec<OsString> {
    compat::normalize_empty_mergetool_base_arg(args)
}

fn parse_app_mode_from_args_env_and_config(
    args: Vec<OsString>,
    env: &dyn EnvLookup,
    git_config: &dyn Fn(&str) -> Option<String>,
) -> Result<AppMode, String> {
    let normalized_args = normalize_empty_mergetool_base_arg(&args);

    match Cli::try_parse_from(normalized_args.clone()) {
        Ok(cli) => match cli.command {
            None => Ok(AppMode::Browser { path: cli.path }),
            Some(Command::Difftool(args)) => {
                resolve_difftool_with_env(args, env).map(AppMode::Difftool)
            }
            Some(Command::Mergetool(args)) => {
                resolve_mergetool_with_config(args, env, git_config).map(AppMode::Mergetool)
            }
            Some(Command::Setup(args)) => Ok(AppMode::Setup {
                dry_run: args.dry_run,
                local: args.local,
            }),
            Some(Command::Uninstall(args)) => Ok(AppMode::Uninstall {
                dry_run: args.dry_run,
                local: args.local,
            }),
            Some(Command::ExtractMergeFixtures(args)) => {
                resolve_extract_merge_fixtures(args).map(AppMode::ExtractMergeFixtures)
            }
        },
        Err(clap_err) => {
            // --help and --version produce informational clap errors that
            // should print to stdout and exit 0, not fall through to the
            // compat parser and be treated as real errors (exit 2).
            use clap::error::ErrorKind;
            match clap_err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    clap_err.exit();
                }
                _ => {}
            }

            let compat_args = if normalized_args.len() > 1 {
                &normalized_args[1..]
            } else {
                &[][..]
            };
            if let Some(mode) =
                parse_compat_external_mode_with_config(compat_args, env, git_config)?
            {
                return Ok(mode);
            }
            Err(clap_err.to_string())
        }
    }
}

fn parse_app_mode_from_args_and_env(
    args: Vec<OsString>,
    env: &dyn EnvLookup,
) -> Result<AppMode, String> {
    // Use only repo-scoped lookups resolved from mergetool file paths.
    parse_app_mode_from_args_env_and_config(args, env, &|_| None)
}

/// Parse CLI arguments and resolve into a validated `AppMode`.
pub fn parse_app_mode() -> Result<AppMode, String> {
    parse_app_mode_from_args_and_env(std::env::args_os().collect(), &ProcessEnv)
}

#[cfg(test)]
mod tests;
