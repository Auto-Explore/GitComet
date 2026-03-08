//! CLI argument parsing for gitcomet-app.
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
#[command(name = "gitcomet-app", about = "Git GUI built with GPUI", version)]
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
    fn display_name(self) -> &'static str {
        match self {
            DifftoolInputKind::Directory => "directory",
            DifftoolInputKind::FileLike => "file",
        }
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
    let metadata = std::fs::symlink_metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{role_name} path does not exist: {}", path.display())
        } else {
            format!(
                "Failed to read metadata for {role_name} path {}: {e}",
                path.display()
            )
        }
    })?;

    if metadata.is_dir() {
        return Ok(DifftoolInputKind::Directory);
    }

    if metadata.file_type().is_symlink() {
        match std::fs::metadata(path) {
            Ok(target_meta) if target_meta.is_dir() => return Ok(DifftoolInputKind::Directory),
            Ok(target_meta) if target_meta.is_file() => return Ok(DifftoolInputKind::FileLike),
            Ok(_) => {
                return Err(format!(
                    "{role_name} path symlink target must resolve to a regular file or directory: {}",
                    path.display()
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DifftoolInputKind::FileLike);
            }
            Err(e) => {
                return Err(format!(
                    "Failed to resolve {role_name} path {}: {e}",
                    path.display()
                ));
            }
        }
    }

    if metadata.is_file() {
        return Ok(DifftoolInputKind::FileLike);
    }

    Err(format!(
        "{role_name} path must be a regular file or directory: {}",
        path.display()
    ))
}

fn resolve_regular_file_metadata(
    path: &Path,
    role_name: &str,
) -> Result<std::fs::Metadata, String> {
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
        Ok(metadata)
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

fn validate_existing_merged_output_path(path: &Path) -> Result<(), String> {
    let symlink_meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(format!(
                "Failed to read metadata for merged path {}: {e}",
                path.display()
            ));
        }
    };

    if symlink_meta.is_dir() {
        return Err(format!(
            "Merged path must be a file path, not a directory: {}",
            path.display()
        ));
    }

    let followed = std::fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "Merged path must resolve to an existing file target: {}",
                path.display()
            )
        } else {
            format!("Failed to resolve merged path {}: {e}", path.display())
        }
    })?;

    if followed.is_dir() {
        return Err(format!(
            "Merged path must be a file path, not a directory: {}",
            path.display()
        ));
    }

    if !followed.is_file() {
        return Err(format!(
            "Merged path must be a regular file path: {}",
            path.display()
        ));
    }

    Ok(())
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

/// Resolve and validate mergetool arguments.
///
/// Priority: explicit flags, then env vars (MERGED, LOCAL, REMOTE, BASE).
/// merged, local, and remote are required. base is optional.
fn resolve_mergetool_with_env(
    args: MergetoolArgs,
    env: &dyn EnvLookup,
) -> Result<MergetoolConfig, String> {
    let marker_size = parse_marker_size(args.marker_size)?;

    let merged = require_non_empty_path(
        resolve_path(args.merged, "MERGED", env)
            .ok_or("Missing required input: --merged flag or MERGED environment variable")?,
        "merged",
    )?;

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

    // Treat an empty BASE value as "no base" for compatibility with
    // shell-expanded custom tool commands like `--base "$BASE"`.
    let base = match resolve_path(args.base, "BASE", env) {
        Some(path) if is_empty_path(&path) => None,
        other => other,
    };

    // MERGED is an output target and may not exist yet (e.g. standalone
    // --output/--out usage). If it already exists, it must resolve to a
    // regular file target.
    validate_existing_merged_output_path(&merged)?;

    resolve_regular_file_metadata(&local, "Local")?;
    resolve_regular_file_metadata(&remote, "Remote")?;

    // Base is allowed to be missing (add/add conflicts have no base).
    // But if explicitly provided, it must resolve to a regular file.
    if let Some(ref base_path) = base {
        resolve_regular_file_metadata(base_path, "Base")?;
    }

    let conflict_style = match args.conflict_style.as_deref() {
        None | Some("merge") => ConflictStyle::Merge,
        Some("diff3") => ConflictStyle::Diff3,
        Some("zdiff3") => ConflictStyle::Zdiff3,
        Some(other) => {
            return Err(format!(
                "Unknown conflict style '{other}': expected merge, diff3, or zdiff3"
            ));
        }
    };

    let diff_algorithm = match args.diff_algorithm.as_deref() {
        None | Some("myers") => DiffAlgorithm::Myers,
        Some("histogram") => DiffAlgorithm::Histogram,
        Some(other) => {
            return Err(format!(
                "Unknown diff algorithm '{other}': expected myers or histogram"
            ));
        }
    };

    Ok(MergetoolConfig {
        merged,
        local,
        remote,
        base,
        label_base: args.label_base,
        label_local: args.label_local,
        label_remote: args.label_remote,
        conflict_style,
        diff_algorithm,
        marker_size,
        auto: args.auto,
        gui: args.gui,
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
