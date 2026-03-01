//! CLI argument parsing for gitgpui-app.
//!
//! Supports three modes:
//! - Default (no subcommand): open the full repository browser
//! - `difftool`: focused diff view, compatible with `git difftool`
//! - `mergetool`: focused merge view, compatible with `git mergetool`

use clap::{Parser, Subcommand};
use gitgpui_core::merge::{ConflictStyle, DEFAULT_MARKER_SIZE, DiffAlgorithm};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

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
#[command(name = "gitgpui-app", about = "Git GUI built with GPUI", version)]
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
    /// Configure git to use gitgpui as the global diff/merge tool.
    Setup(SetupArgs),
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
    #[arg(long)]
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

    if !local.exists() {
        return Err(format!("Local path does not exist: {}", local.display()));
    }
    if !remote.exists() {
        return Err(format!("Remote path does not exist: {}", remote.display()));
    }
    if local.is_dir() != remote.is_dir() {
        let local_kind = if local.is_dir() { "directory" } else { "file" };
        let remote_kind = if remote.is_dir() { "directory" } else { "file" };
        return Err(format!(
            "Difftool input kind mismatch: local is a {local_kind} and remote is a {remote_kind}. Use two files or two directories."
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
    let base_from_flag = args.base.is_some();

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

    let base = match resolve_path(args.base, "BASE", env) {
        Some(path) if is_empty_path(&path) => {
            if base_from_flag {
                return Err("Invalid base path: value is empty. Use --base with a non-empty path, or omit it for no-base conflicts.".to_string());
            }
            None
        }
        other => other,
    };

    // MERGED is an output target and may not exist yet (e.g. standalone
    // --output/--out usage). Runtime creates parent directories as needed.
    if merged.exists() && merged.is_dir() {
        return Err(format!(
            "Merged path must be a file path, not a directory: {}",
            merged.display()
        ));
    }

    if !local.exists() {
        return Err(format!("Local path does not exist: {}", local.display()));
    }
    if local.is_dir() {
        return Err(format!(
            "Local path must be a file, not a directory: {}",
            local.display()
        ));
    }

    if !remote.exists() {
        return Err(format!("Remote path does not exist: {}", remote.display()));
    }
    if remote.is_dir() {
        return Err(format!(
            "Remote path must be a file, not a directory: {}",
            remote.display()
        ));
    }

    // Base is allowed to be missing (add/add conflicts have no base).
    // But if explicitly provided, it should exist.
    if let Some(ref base_path) = base
        && !base_path.exists()
    {
        return Err(format!("Base path does not exist: {}", base_path.display()));
    }
    if let Some(ref base_path) = base
        && base_path.is_dir()
    {
        return Err(format!(
            "Base path must be a file, not a directory: {}",
            base_path.display()
        ));
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

/// Read a single git config value via `git config --get`.
/// Returns `None` if the key is not set or git is not available.
fn read_git_config(key: &str) -> Option<String> {
    std::process::Command::new("git")
        .args(["config", "--get", key])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Apply git config fallback for `merge.conflictstyle` and `diff.algorithm`
/// when the user did not provide explicit CLI flags.
///
/// This mirrors `git merge-file` behavior: the tool respects the user's
/// configured preferences without requiring them to modify the mergetool
/// command string.
fn apply_git_config_fallback(
    config: &mut MergetoolConfig,
    had_explicit_style: bool,
    had_explicit_algorithm: bool,
    git_config: &dyn Fn(&str) -> Option<String>,
) {
    if !had_explicit_style && let Some(style) = git_config("merge.conflictstyle") {
        match style.as_str() {
            "merge" => config.conflict_style = ConflictStyle::Merge,
            "diff3" => config.conflict_style = ConflictStyle::Diff3,
            "zdiff3" => config.conflict_style = ConflictStyle::Zdiff3,
            _ => {} // ignore unrecognized values, keep default
        }
    }

    if !had_explicit_algorithm && let Some(algo) = git_config("diff.algorithm") {
        match algo.as_str() {
            "histogram" | "patience" => config.diff_algorithm = DiffAlgorithm::Histogram,
            "myers" | "default" | "minimal" => config.diff_algorithm = DiffAlgorithm::Myers,
            _ => {} // ignore unrecognized values, keep default
        }
    }
}

/// Internal: resolve mergetool args with both env and git config fallback.
fn resolve_mergetool_with_config(
    args: MergetoolArgs,
    env: &dyn EnvLookup,
    git_config: &dyn Fn(&str) -> Option<String>,
) -> Result<MergetoolConfig, String> {
    let had_explicit_style = args.conflict_style.is_some();
    let had_explicit_algorithm = args.diff_algorithm.is_some();

    let mut config = resolve_mergetool_with_env(args, env)?;
    apply_git_config_fallback(
        &mut config,
        had_explicit_style,
        had_explicit_algorithm,
        git_config,
    );
    Ok(config)
}

fn assign_next_compat_label(
    label_l1: &mut Option<String>,
    label_l2: &mut Option<String>,
    label_l3: &mut Option<String>,
    value: String,
) -> Result<(), String> {
    if label_l1.is_none() {
        *label_l1 = Some(value);
        return Ok(());
    }
    if label_l2.is_none() {
        *label_l2 = Some(value);
        return Ok(());
    }
    if label_l3.is_none() {
        *label_l3 = Some(value);
        return Ok(());
    }
    Err("Invalid external invocation: too many label flags; expected at most 3 labels across --L1/--L2/--L3 and -L/--label.".to_string())
}

fn parse_compat_external_mode_with_config(
    raw_args: &[OsString],
    env: &dyn EnvLookup,
    git_config: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<AppMode>, String> {
    let mut label_l1: Option<String> = None;
    let mut label_l2: Option<String> = None;
    let mut label_l3: Option<String> = None;
    let mut base_flag: Option<PathBuf> = None;
    let mut merged_output: Option<PathBuf> = None;
    let mut positionals: Vec<PathBuf> = Vec::new();
    let mut has_auto = false;
    let mut has_auto_merge = false;
    let mut has_kdiff3_label_flags = false;

    let mut idx = 0usize;
    while idx < raw_args.len() {
        let arg = &raw_args[idx];
        let token = arg.to_string_lossy();

        if token == "--auto" {
            has_auto = true;
            idx += 1;
            continue;
        }

        if token == "--auto-merge" {
            has_auto_merge = true;
            idx += 1;
            continue;
        }

        if token == "--L1" || token == "--L2" || token == "--L3" {
            let next_idx = idx + 1;
            let value = raw_args.get(next_idx).ok_or_else(|| {
                format!("Missing value for compatibility flag {token} in external tool mode")
            })?;
            let value = value.to_string_lossy().into_owned();
            match token.as_ref() {
                "--L1" => label_l1 = Some(value),
                "--L2" => label_l2 = Some(value),
                "--L3" => label_l3 = Some(value),
                _ => unreachable!(),
            }
            has_kdiff3_label_flags = true;
            idx += 2;
            continue;
        }

        if token == "-L" || token == "--label" {
            let next_idx = idx + 1;
            let value = raw_args.get(next_idx).ok_or_else(|| {
                format!("Missing value for compatibility flag {token} in external tool mode")
            })?;
            assign_next_compat_label(
                &mut label_l1,
                &mut label_l2,
                &mut label_l3,
                value.to_string_lossy().into_owned(),
            )?;
            idx += 2;
            continue;
        }

        if let Some(value) = token.strip_prefix("--L1=") {
            label_l1 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--L2=") {
            label_l2 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--L3=") {
            label_l3 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--label=") {
            assign_next_compat_label(
                &mut label_l1,
                &mut label_l2,
                &mut label_l3,
                value.to_string(),
            )?;
            idx += 1;
            continue;
        }

        if token == "-o" || token == "--output" || token == "--out" {
            let next_idx = idx + 1;
            let value = raw_args.get(next_idx).ok_or_else(|| {
                format!("Missing value for compatibility flag {token} in external tool mode")
            })?;
            merged_output = Some(PathBuf::from(value));
            idx += 2;
            continue;
        }

        if token == "--base" {
            let next_idx = idx + 1;
            let value = raw_args.get(next_idx).ok_or_else(|| {
                "Missing value for compatibility flag --base in external tool mode".to_string()
            })?;
            base_flag = Some(PathBuf::from(value));
            idx += 2;
            continue;
        }

        if let Some(value) = token.strip_prefix("--output=") {
            merged_output = Some(PathBuf::from(value));
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--out=") {
            merged_output = Some(PathBuf::from(value));
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--base=") {
            base_flag = Some(PathBuf::from(value));
            idx += 1;
            continue;
        }
        if token.starts_with("-o") && token.len() > 2 {
            merged_output = Some(PathBuf::from(token[2..].to_string()));
            idx += 1;
            continue;
        }
        if token.starts_with("-L") && token.len() > 2 {
            assign_next_compat_label(
                &mut label_l1,
                &mut label_l2,
                &mut label_l3,
                token[2..].to_string(),
            )?;
            idx += 1;
            continue;
        }

        if token == "--" {
            positionals.extend(raw_args[idx + 1..].iter().map(PathBuf::from));
            idx = raw_args.len();
            continue;
        }

        if token.starts_with('-') {
            return Ok(None);
        }

        positionals.push(PathBuf::from(arg));
        idx += 1;
    }

    if has_auto && merged_output.is_none() {
        return Err(
            "Invalid external merge invocation: --auto requires -o/--output/--out <MERGED>."
                .to_string(),
        );
    }

    if has_auto_merge && merged_output.is_none() {
        return Err(
            "Invalid external merge invocation: --auto-merge requires -o/--output/--out <MERGED>."
                .to_string(),
        );
    }

    if let Some(merged) = merged_output {
        let (base, local, remote, label_base, label_local, label_remote) = if let Some(
            explicit_base,
        ) = base_flag
        {
            match positionals.len() {
                2 => (
                    Some(explicit_base),
                    positionals[0].clone(),
                    positionals[1].clone(),
                    label_l1,
                    label_l2,
                    label_l3,
                ),
                0 | 1 => {
                    return Err("Invalid external merge invocation: expected exactly 2 positional paths (LOCAL REMOTE) when --base is provided.".to_string());
                }
                _ => {
                    return Err("Invalid external merge invocation: --base already supplies BASE; expected exactly 2 positional paths (LOCAL REMOTE).".to_string());
                }
            }
        } else {
            match positionals.len() {
                3 => {
                    // Ambiguous 3-path merge-mode compatibility input:
                    // - KDiff3 style: BASE LOCAL REMOTE
                    // - Meld style:   LOCAL BASE REMOTE
                    //
                    // Prefer KDiff3 order when KDiff3-specific hints are
                    // present (`--auto`/`--L*`). Otherwise default to Meld's
                    // LOCAL BASE REMOTE ordering for broad path-override
                    // compatibility.
                    if has_auto || has_kdiff3_label_flags {
                        (
                            Some(positionals[0].clone()),
                            positionals[1].clone(),
                            positionals[2].clone(),
                            label_l1,
                            label_l2,
                            label_l3,
                        )
                    } else {
                        (
                            Some(positionals[1].clone()),
                            positionals[0].clone(),
                            positionals[2].clone(),
                            label_l2,
                            label_l1,
                            label_l3,
                        )
                    }
                }
                2 => {
                    if label_l3.is_some() {
                        return Err("Invalid external merge invocation: --L3 requires BASE input. Provide --base <BASE> or 3 positional paths (BASE LOCAL REMOTE).".to_string());
                    }
                    (
                        None,
                        positionals[0].clone(),
                        positionals[1].clone(),
                        None,
                        label_l1,
                        label_l2,
                    )
                }
                0 | 1 => {
                    return Err("Invalid external merge invocation: expected 2 positional paths (LOCAL REMOTE) or 3 (BASE LOCAL REMOTE) after -o/--output/--out.".to_string());
                }
                _ => {
                    return Err("Invalid external merge invocation: too many positional paths; expected 2 (LOCAL REMOTE) or 3 (BASE LOCAL REMOTE).".to_string());
                }
            }
        };

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base,
            label_base,
            label_local,
            label_remote,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: has_auto || has_auto_merge,
            gui: false,
        };
        return resolve_mergetool_with_config(args, env, git_config)
            .map(AppMode::Mergetool)
            .map(Some);
    }

    if base_flag.is_some() {
        return Err(
            "Invalid external diff invocation: --base is only valid for merge mode with -o/--output/--out."
                .to_string(),
        );
    }

    if label_l3.is_some() {
        return Err(
            "Invalid external diff invocation: --L3 is only valid for merge mode with -o/--output/--out."
                .to_string(),
        );
    }

    if positionals.is_empty() && (label_l1.is_some() || label_l2.is_some()) {
        return Err(
            "Invalid external diff invocation: expected 2 positional paths (LOCAL REMOTE)."
                .to_string(),
        );
    }

    if positionals.len() == 2 {
        let args = DifftoolArgs {
            local: Some(positionals[0].clone()),
            remote: Some(positionals[1].clone()),
            path: None,
            label_left: label_l1,
            label_right: label_l2,
            gui: false,
        };
        return resolve_difftool_with_env(args, env)
            .map(AppMode::Difftool)
            .map(Some);
    }

    if positionals.len() > 2 {
        return Err("Invalid external diff invocation: too many positional paths; expected exactly 2 (LOCAL REMOTE). Use -o/--output/--out for merge mode.".to_string());
    }

    Ok(None)
}

fn parse_app_mode_from_args_env_and_config(
    args: Vec<OsString>,
    env: &dyn EnvLookup,
    git_config: &dyn Fn(&str) -> Option<String>,
) -> Result<AppMode, String> {
    match Cli::try_parse_from(args.clone()) {
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
        },
        Err(clap_err) => {
            let compat_args = if args.len() > 1 { &args[1..] } else { &[][..] };
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
    parse_app_mode_from_args_env_and_config(args, env, &read_git_config)
}

/// Parse CLI arguments and resolve into a validated `AppMode`.
pub fn parse_app_mode() -> Result<AppMode, String> {
    parse_app_mode_from_args_and_env(std::env::args_os().collect(), &ProcessEnv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;

    /// Test-only environment that avoids calling the unsafe `std::env::set_var`.
    struct TestEnv {
        vars: HashMap<String, OsString>,
    }

    impl TestEnv {
        fn new() -> Self {
            Self {
                vars: HashMap::new(),
            }
        }

        fn set(&mut self, key: &str, value: impl Into<OsString>) -> &mut Self {
            self.vars.insert(key.to_string(), value.into());
            self
        }
    }

    impl EnvLookup for TestEnv {
        fn var_os(&self, key: &str) -> Option<OsString> {
            self.vars.get(key).cloned()
        }
    }

    /// Create a temporary file and return its path.
    fn tmp_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    fn parse_mode_for_test_with_config(
        args: Vec<OsString>,
        env: &dyn EnvLookup,
        git_config: &dyn Fn(&str) -> Option<String>,
    ) -> Result<AppMode, String> {
        parse_app_mode_from_args_env_and_config(args, env, git_config)
    }

    fn parse_mode_for_test(args: Vec<OsString>, env: &dyn EnvLookup) -> Result<AppMode, String> {
        parse_mode_for_test_with_config(args, env, &|_| None)
    }

    // ── DifftoolArgs resolution ──────────────────────────────────────

    #[test]
    fn difftool_resolves_from_explicit_flags() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "left.txt", "left content");
        let remote = tmp_file(&dir, "right.txt", "right content");
        let env = TestEnv::new();

        let args = DifftoolArgs {
            local: Some(local.clone()),
            remote: Some(remote.clone()),
            path: Some("display.txt".into()),
            label_left: Some("Ours".into()),
            label_right: Some("Theirs".into()),
            gui: false,
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.local, local);
        assert_eq!(config.remote, remote);
        assert_eq!(config.display_path.as_deref(), Some("display.txt"));
        assert_eq!(config.label_left.as_deref(), Some("Ours"));
        assert_eq!(config.label_right.as_deref(), Some("Theirs"));
    }

    #[test]
    fn difftool_resolves_from_env_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");

        let mut env = TestEnv::new();
        env.set("LOCAL", &local);
        env.set("REMOTE", &remote);
        env.set("MERGED", "file.txt");

        let args = DifftoolArgs {
            local: None,
            remote: None,
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.local, local);
        assert_eq!(config.remote, remote);
        assert_eq!(config.display_path.as_deref(), Some("file.txt"));
    }

    #[test]
    fn difftool_uses_base_env_as_display_path_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");

        let mut env = TestEnv::new();
        env.set("LOCAL", &local);
        env.set("REMOTE", &remote);
        env.set("BASE", "base-name.txt");

        let args = DifftoolArgs {
            local: None,
            remote: None,
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.display_path.as_deref(), Some("base-name.txt"));
    }

    #[test]
    fn difftool_prefers_merged_over_base_for_display_path() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");

        let mut env = TestEnv::new();
        env.set("LOCAL", &local);
        env.set("REMOTE", &remote);
        env.set("MERGED", "merged-name.txt");
        env.set("BASE", "base-name.txt");

        let args = DifftoolArgs {
            local: None,
            remote: None,
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.display_path.as_deref(), Some("merged-name.txt"));
    }

    #[test]
    fn difftool_path_flag_overrides_merged_and_base_display_env() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");

        let mut env = TestEnv::new();
        env.set("LOCAL", &local);
        env.set("REMOTE", &remote);
        env.set("MERGED", "merged-name.txt");
        env.set("BASE", "base-name.txt");

        let args = DifftoolArgs {
            local: None,
            remote: None,
            path: Some("explicit-name.txt".into()),
            label_left: None,
            label_right: None,
            gui: false,
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.display_path.as_deref(), Some("explicit-name.txt"));
    }

    #[test]
    fn difftool_flags_take_precedence_over_env() {
        let dir = tempfile::tempdir().unwrap();
        let flag_local = tmp_file(&dir, "flag_local.txt", "flag");
        let flag_remote = tmp_file(&dir, "flag_remote.txt", "flag");
        let _env_local = tmp_file(&dir, "env_local.txt", "env");
        let _env_remote = tmp_file(&dir, "env_remote.txt", "env");

        let mut env = TestEnv::new();
        env.set("LOCAL", dir.path().join("env_local.txt"));
        env.set("REMOTE", dir.path().join("env_remote.txt"));

        let args = DifftoolArgs {
            local: Some(flag_local.clone()),
            remote: Some(flag_remote.clone()),
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.local, flag_local);
        assert_eq!(config.remote, flag_remote);
    }

    #[test]
    fn difftool_missing_local_errors() {
        let dir = tempfile::tempdir().unwrap();
        let remote = tmp_file(&dir, "remote.txt", "b");
        let env = TestEnv::new();

        let args = DifftoolArgs {
            local: None,
            remote: Some(remote),
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let err = resolve_difftool_with_env(args, &env).unwrap_err();
        assert!(err.contains("LOCAL"), "error should mention LOCAL: {err}");
    }

    #[test]
    fn difftool_missing_remote_errors() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let env = TestEnv::new();

        let args = DifftoolArgs {
            local: Some(local),
            remote: None,
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let err = resolve_difftool_with_env(args, &env).unwrap_err();
        assert!(err.contains("REMOTE"), "error should mention REMOTE: {err}");
    }

    #[test]
    fn difftool_nonexistent_local_errors() {
        let dir = tempfile::tempdir().unwrap();
        let remote = tmp_file(&dir, "remote.txt", "b");
        let env = TestEnv::new();

        let args = DifftoolArgs {
            local: Some(dir.path().join("no_such_file.txt")),
            remote: Some(remote),
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let err = resolve_difftool_with_env(args, &env).unwrap_err();
        assert!(
            err.contains("does not exist"),
            "error should mention nonexistence: {err}"
        );
    }

    #[test]
    fn difftool_nonexistent_remote_errors() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let env = TestEnv::new();

        let args = DifftoolArgs {
            local: Some(local),
            remote: Some(dir.path().join("no_such_file.txt")),
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let err = resolve_difftool_with_env(args, &env).unwrap_err();
        assert!(
            err.contains("does not exist"),
            "error should mention nonexistence: {err}"
        );
    }

    #[test]
    fn difftool_empty_local_path_errors() {
        let dir = tempfile::tempdir().unwrap();
        let remote = tmp_file(&dir, "remote.txt", "b");

        let args = DifftoolArgs {
            local: Some(PathBuf::new()),
            remote: Some(remote),
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let err = resolve_difftool_with_env(args, &TestEnv::new()).unwrap_err();
        assert!(err.contains("Invalid local path"), "error: {err}");
    }

    #[test]
    fn difftool_empty_merged_env_is_ignored_for_display_path() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");

        let mut env = TestEnv::new();
        env.set("LOCAL", &local);
        env.set("REMOTE", &remote);
        env.set("MERGED", "");
        env.set("BASE", "fallback-name.txt");

        let args = DifftoolArgs {
            local: None,
            remote: None,
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.display_path.as_deref(), Some("fallback-name.txt"));
    }

    #[test]
    fn difftool_accepts_directories() {
        let dir = tempfile::tempdir().unwrap();
        let left_dir = dir.path().join("left");
        let right_dir = dir.path().join("right");
        std::fs::create_dir(&left_dir).unwrap();
        std::fs::create_dir(&right_dir).unwrap();
        let env = TestEnv::new();

        let args = DifftoolArgs {
            local: Some(left_dir.clone()),
            remote: Some(right_dir.clone()),
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.local, left_dir);
        assert_eq!(config.remote, right_dir);
    }

    #[test]
    fn difftool_rejects_file_vs_directory_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = tmp_file(&dir, "left.txt", "left");
        let right_dir = dir.path().join("right");
        std::fs::create_dir(&right_dir).unwrap();

        let args = DifftoolArgs {
            local: Some(file_path),
            remote: Some(right_dir),
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let err = resolve_difftool_with_env(args, &TestEnv::new()).unwrap_err();
        assert!(
            err.contains("input kind mismatch"),
            "error should mention kind mismatch: {err}"
        );
        assert!(
            err.contains("two files or two directories"),
            "error should explain valid combinations: {err}"
        );
    }

    // ── MergetoolArgs resolution ─────────────────────────────────────

    #[test]
    fn mergetool_resolves_from_explicit_flags() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "merged.txt", "<<<<<<< HEAD\na\n=======\nb\n>>>>>>>");
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let base = tmp_file(&dir, "base.txt", "original");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged.clone()),
            local: Some(local.clone()),
            remote: Some(remote.clone()),
            base: Some(base.clone()),
            label_base: Some("Base".into()),
            label_local: Some("Ours".into()),
            label_remote: Some("Theirs".into()),
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.merged, merged);
        assert_eq!(config.local, local);
        assert_eq!(config.remote, remote);
        assert_eq!(config.base.as_ref(), Some(&base));
        assert_eq!(config.label_base.as_deref(), Some("Base"));
        assert_eq!(config.label_local.as_deref(), Some("Ours"));
        assert_eq!(config.label_remote.as_deref(), Some("Theirs"));
    }

    #[test]
    fn mergetool_resolves_from_env_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "merged.txt", "conflict");
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let base = tmp_file(&dir, "base.txt", "original");

        let mut env = TestEnv::new();
        env.set("MERGED", &merged);
        env.set("LOCAL", &local);
        env.set("REMOTE", &remote);
        env.set("BASE", &base);

        let args = MergetoolArgs {
            merged: None,
            local: None,
            remote: None,
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.merged, merged);
        assert_eq!(config.local, local);
        assert_eq!(config.remote, remote);
        assert_eq!(config.base.as_ref(), Some(&base));
    }

    #[test]
    fn mergetool_base_optional() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "merged.txt", "conflict");
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let env = TestEnv::new(); // no BASE in env

        let args = MergetoolArgs {
            merged: Some(merged.clone()),
            local: Some(local.clone()),
            remote: Some(remote.clone()),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.merged, merged);
        assert_eq!(config.local, local);
        assert_eq!(config.remote, remote);
        assert!(config.base.is_none());
    }

    #[test]
    fn mergetool_empty_base_env_treated_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "merged.txt", "conflict");
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");

        let mut env = TestEnv::new();
        env.set("MERGED", &merged);
        env.set("LOCAL", &local);
        env.set("REMOTE", &remote);
        env.set("BASE", "");

        let args = MergetoolArgs {
            merged: None,
            local: None,
            remote: None,
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert!(
            config.base.is_none(),
            "empty BASE should be treated as no-base"
        );
    }

    #[test]
    fn mergetool_empty_base_flag_errors() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "merged.txt", "conflict");
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: Some(PathBuf::new()),
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &TestEnv::new()).unwrap_err();
        assert!(err.contains("Invalid base path"), "error: {err}");
    }

    #[test]
    fn mergetool_empty_merged_path_errors() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");

        let args = MergetoolArgs {
            merged: Some(PathBuf::new()),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &TestEnv::new()).unwrap_err();
        assert!(err.contains("Invalid merged path"), "error: {err}");
    }

    #[test]
    fn mergetool_missing_merged_errors() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: None,
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("MERGED"), "error should mention MERGED: {err}");
    }

    #[test]
    fn mergetool_missing_local_errors() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "merged.txt", "conflict");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: None,
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("LOCAL"), "error should mention LOCAL: {err}");
    }

    #[test]
    fn mergetool_missing_remote_errors() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "merged.txt", "conflict");
        let local = tmp_file(&dir, "local.txt", "a");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: None,
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("REMOTE"), "error should mention REMOTE: {err}");
    }

    #[test]
    fn mergetool_nonexistent_merged_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let env = TestEnv::new();
        let merged = dir.path().join("no_such_merged.txt");

        let args = MergetoolArgs {
            merged: Some(merged.clone()),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.merged, merged);
    }

    #[test]
    fn mergetool_nonexistent_base_errors_when_explicitly_provided() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "merged.txt", "conflict");
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: Some(dir.path().join("no_such_base.txt")),
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("Base path does not exist"), "error: {err}");
    }

    #[test]
    fn mergetool_existing_merged_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        let merged_dir = dir.path().join("merged-dir");
        std::fs::create_dir_all(&merged_dir).unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");

        let args = MergetoolArgs {
            merged: Some(merged_dir),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &TestEnv::new()).unwrap_err();
        assert!(err.contains("Merged path must be a file path"), "error: {err}");
    }

    #[test]
    fn mergetool_local_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        let merged = dir.path().join("merged.txt");
        let local_dir = dir.path().join("local-dir");
        std::fs::create_dir_all(&local_dir).unwrap();
        let remote = tmp_file(&dir, "remote.txt", "b");

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local_dir),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &TestEnv::new()).unwrap_err();
        assert!(err.contains("Local path must be a file"), "error: {err}");
    }

    #[test]
    fn mergetool_remote_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        let merged = dir.path().join("merged.txt");
        let local = tmp_file(&dir, "local.txt", "a");
        let remote_dir = dir.path().join("remote-dir");
        std::fs::create_dir_all(&remote_dir).unwrap();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote_dir),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &TestEnv::new()).unwrap_err();
        assert!(err.contains("Remote path must be a file"), "error: {err}");
    }

    #[test]
    fn mergetool_base_directory_errors_when_explicitly_provided() {
        let dir = tempfile::tempdir().unwrap();
        let merged = dir.path().join("merged.txt");
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let base_dir = dir.path().join("base-dir");
        std::fs::create_dir_all(&base_dir).unwrap();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: Some(base_dir),
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &TestEnv::new()).unwrap_err();
        assert!(err.contains("Base path must be a file"), "error: {err}");
    }

    // ── Exit code constants ──────────────────────────────────────────

    #[test]
    fn exit_code_values_match_design() {
        assert_eq!(exit_code::SUCCESS, 0);
        assert_eq!(exit_code::CANCELED, 1);
        assert_eq!(exit_code::ERROR, 2);
    }

    // ── Paths with spaces ────────────────────────────────────────────

    #[test]
    fn difftool_handles_paths_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "my local file.txt", "left");
        let remote = tmp_file(&dir, "my remote file.txt", "right");
        let env = TestEnv::new();

        let args = DifftoolArgs {
            local: Some(local.clone()),
            remote: Some(remote.clone()),
            path: Some("path with spaces.txt".into()),
            label_left: None,
            label_right: None,
            gui: false,
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.local, local);
        assert_eq!(config.remote, remote);
        assert_eq!(config.display_path.as_deref(), Some("path with spaces.txt"));
    }

    #[test]
    fn mergetool_handles_paths_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "my merged file.txt", "conflict");
        let local = tmp_file(&dir, "my local file.txt", "a");
        let remote = tmp_file(&dir, "my remote file.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged.clone()),
            local: Some(local.clone()),
            remote: Some(remote.clone()),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.merged, merged);
        assert_eq!(config.local, local);
        assert_eq!(config.remote, remote);
    }

    // ── Unicode paths ────────────────────────────────────────────────

    #[test]
    fn difftool_handles_unicode_paths() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "日本語ファイル.txt", "左");
        let remote = tmp_file(&dir, "ファイル名.txt", "右");
        let env = TestEnv::new();

        let args = DifftoolArgs {
            local: Some(local.clone()),
            remote: Some(remote.clone()),
            path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.local, local);
        assert_eq!(config.remote, remote);
    }

    // ── Env-only resolution with no flags ────────────────────────────

    #[test]
    fn mergetool_env_only_resolution_with_all_four_vars() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "m.txt", "x");
        let local = tmp_file(&dir, "l.txt", "a");
        let remote = tmp_file(&dir, "r.txt", "b");
        let base = tmp_file(&dir, "b.txt", "o");

        let mut env = TestEnv::new();
        env.set("MERGED", &merged)
            .set("LOCAL", &local)
            .set("REMOTE", &remote)
            .set("BASE", &base);

        let args = MergetoolArgs {
            merged: None,
            local: None,
            remote: None,
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.merged, merged);
        assert_eq!(config.base.as_ref(), Some(&base));
    }

    #[test]
    fn mergetool_env_only_resolution_without_base() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "m.txt", "x");
        let local = tmp_file(&dir, "l.txt", "a");
        let remote = tmp_file(&dir, "r.txt", "b");

        let mut env = TestEnv::new();
        env.set("MERGED", &merged)
            .set("LOCAL", &local)
            .set("REMOTE", &remote);
        // Deliberately no BASE

        let args = MergetoolArgs {
            merged: None,
            local: None,
            remote: None,
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert!(config.base.is_none());
    }

    // ── Clap argument parsing ────────────────────────────────────────

    #[test]
    fn clap_parses_difftool_subcommand() {
        let cli = Cli::try_parse_from([
            "gitgpui-app",
            "difftool",
            "--local",
            "/tmp/a",
            "--remote",
            "/tmp/b",
            "--path",
            "foo.txt",
        ])
        .unwrap();

        match cli.command {
            Some(Command::Difftool(args)) => {
                assert_eq!(args.local.as_deref(), Some(std::path::Path::new("/tmp/a")));
                assert_eq!(args.remote.as_deref(), Some(std::path::Path::new("/tmp/b")));
                assert_eq!(args.path.as_deref(), Some("foo.txt"));
            }
            _ => panic!("expected Difftool command"),
        }
    }

    #[test]
    fn clap_parses_mergetool_subcommand() {
        let cli = Cli::try_parse_from([
            "gitgpui-app",
            "mergetool",
            "--merged",
            "/tmp/m",
            "--local",
            "/tmp/l",
            "--remote",
            "/tmp/r",
            "--base",
            "/tmp/b",
            "--label-base",
            "Base",
            "--label-local",
            "Ours",
            "--label-remote",
            "Theirs",
        ])
        .unwrap();

        match cli.command {
            Some(Command::Mergetool(args)) => {
                assert_eq!(args.merged.as_deref(), Some(std::path::Path::new("/tmp/m")));
                assert_eq!(args.local.as_deref(), Some(std::path::Path::new("/tmp/l")));
                assert_eq!(args.remote.as_deref(), Some(std::path::Path::new("/tmp/r")));
                assert_eq!(args.base.as_deref(), Some(std::path::Path::new("/tmp/b")));
                assert_eq!(args.label_base.as_deref(), Some("Base"));
                assert_eq!(args.label_local.as_deref(), Some("Ours"));
                assert_eq!(args.label_remote.as_deref(), Some("Theirs"));
            }
            _ => panic!("expected Mergetool command"),
        }
    }

    #[test]
    fn clap_parses_mergetool_output_aliases() {
        for merged_flag in ["-o", "--output", "--out"] {
            let cli = Cli::try_parse_from([
                "gitgpui-app",
                "mergetool",
                merged_flag,
                "/tmp/m",
                "--local",
                "/tmp/l",
                "--remote",
                "/tmp/r",
            ])
            .unwrap();

            match cli.command {
                Some(Command::Mergetool(args)) => {
                    assert_eq!(args.merged.as_deref(), Some(std::path::Path::new("/tmp/m")));
                    assert_eq!(args.local.as_deref(), Some(std::path::Path::new("/tmp/l")));
                    assert_eq!(args.remote.as_deref(), Some(std::path::Path::new("/tmp/r")));
                }
                _ => panic!("expected Mergetool command"),
            }
        }
    }

    #[test]
    fn clap_parses_mergetool_kdiff3_label_aliases() {
        let cli = Cli::try_parse_from([
            "gitgpui-app",
            "mergetool",
            "--merged",
            "/tmp/m",
            "--local",
            "/tmp/l",
            "--remote",
            "/tmp/r",
            "--L1",
            "Base",
            "--L2",
            "Ours",
            "--L3",
            "Theirs",
        ])
        .unwrap();

        match cli.command {
            Some(Command::Mergetool(args)) => {
                assert_eq!(args.label_base.as_deref(), Some("Base"));
                assert_eq!(args.label_local.as_deref(), Some("Ours"));
                assert_eq!(args.label_remote.as_deref(), Some("Theirs"));
            }
            _ => panic!("expected Mergetool command"),
        }
    }

    #[test]
    fn clap_parses_no_subcommand_as_browser() {
        let cli = Cli::try_parse_from(["gitgpui-app"]).unwrap();
        assert!(cli.command.is_none());
        assert!(cli.path.is_none());
    }

    #[test]
    fn clap_parses_path_argument() {
        let cli = Cli::try_parse_from(["gitgpui-app", "/some/repo"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(
            cli.path.as_deref(),
            Some(std::path::Path::new("/some/repo"))
        );
    }

    #[test]
    fn compat_parses_positional_difftool_invocation() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "left.txt", "left\n");
        let remote = tmp_file(&dir, "right.txt", "right\n");
        let env = TestEnv::new();

        let mode = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap();

        match mode {
            AppMode::Difftool(config) => {
                assert!(config.local.ends_with("left.txt"));
                assert!(config.remote.ends_with("right.txt"));
                assert_eq!(config.label_left, None);
                assert_eq!(config.label_right, None);
            }
            _ => panic!("expected Difftool mode"),
        }
    }

    #[test]
    fn compat_parses_kdiff3_style_difftool_labels() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "left.txt", "left\n");
        let remote = tmp_file(&dir, "right.txt", "right\n");
        let env = TestEnv::new();

        let mode = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--L1"),
                OsString::from("LEFT_LABEL"),
                OsString::from("--L2"),
                OsString::from("RIGHT_LABEL"),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap();

        match mode {
            AppMode::Difftool(config) => {
                assert_eq!(config.label_left.as_deref(), Some("LEFT_LABEL"));
                assert_eq!(config.label_right.as_deref(), Some("RIGHT_LABEL"));
            }
            _ => panic!("expected Difftool mode"),
        }
    }

    #[test]
    fn compat_parses_meld_style_difftool_short_labels() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "left.txt", "left\n");
        let remote = tmp_file(&dir, "right.txt", "right\n");
        let env = TestEnv::new();

        let mode = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("-L"),
                OsString::from("LEFT_LABEL"),
                OsString::from("--label"),
                OsString::from("RIGHT_LABEL"),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap();

        match mode {
            AppMode::Difftool(config) => {
                assert_eq!(config.label_left.as_deref(), Some("LEFT_LABEL"));
                assert_eq!(config.label_right.as_deref(), Some("RIGHT_LABEL"));
            }
            _ => panic!("expected Difftool mode"),
        }
    }

    #[test]
    fn compat_parses_meld_style_difftool_attached_labels() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "left.txt", "left\n");
        let remote = tmp_file(&dir, "right.txt", "right\n");
        let env = TestEnv::new();

        let mode = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("-LLEFT_LABEL"),
                OsString::from("--label=RIGHT_LABEL"),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap();

        match mode {
            AppMode::Difftool(config) => {
                assert_eq!(config.label_left.as_deref(), Some("LEFT_LABEL"));
                assert_eq!(config.label_right.as_deref(), Some("RIGHT_LABEL"));
            }
            _ => panic!("expected Difftool mode"),
        }
    }

    #[test]
    fn compat_parses_kdiff3_style_mergetool_with_base() {
        let dir = tempfile::tempdir().unwrap();
        let base = tmp_file(&dir, "base.txt", "base\n");
        let local = tmp_file(&dir, "local.txt", "local\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("nested/out/merged.txt");
        let env = TestEnv::new();

        let mode = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--auto"),
                OsString::from("--L1"),
                OsString::from("BASE_LABEL"),
                OsString::from("--L2"),
                OsString::from("LOCAL_LABEL"),
                OsString::from("--L3"),
                OsString::from("REMOTE_LABEL"),
                OsString::from("-o"),
                merged.clone().into_os_string(),
                base.clone().into_os_string(),
                local.clone().into_os_string(),
                remote.clone().into_os_string(),
            ],
            &env,
        )
        .unwrap();

        match mode {
            AppMode::Mergetool(config) => {
                assert_eq!(config.merged, merged);
                assert_eq!(config.base.as_ref(), Some(&base));
                assert_eq!(config.local, local);
                assert_eq!(config.remote, remote);
                assert_eq!(config.label_base.as_deref(), Some("BASE_LABEL"));
                assert_eq!(config.label_local.as_deref(), Some("LOCAL_LABEL"));
                assert_eq!(config.label_remote.as_deref(), Some("REMOTE_LABEL"));
            }
            _ => panic!("expected Mergetool mode"),
        }
    }

    #[test]
    fn compat_parses_kdiff3_style_mergetool_with_base_flag() {
        let dir = tempfile::tempdir().unwrap();
        let base = tmp_file(&dir, "base.txt", "base\n");
        let local = tmp_file(&dir, "local.txt", "local\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let mode = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--auto"),
                OsString::from("--L1=BASE_LABEL"),
                OsString::from("--L2=LOCAL_LABEL"),
                OsString::from("--L3=REMOTE_LABEL"),
                OsString::from("--base"),
                base.clone().into_os_string(),
                OsString::from("--output"),
                merged.clone().into_os_string(),
                local.clone().into_os_string(),
                remote.clone().into_os_string(),
            ],
            &env,
        )
        .unwrap();

        match mode {
            AppMode::Mergetool(config) => {
                assert_eq!(config.merged, merged);
                assert_eq!(config.base.as_ref(), Some(&base));
                assert_eq!(config.local, local);
                assert_eq!(config.remote, remote);
                assert_eq!(config.label_base.as_deref(), Some("BASE_LABEL"));
                assert_eq!(config.label_local.as_deref(), Some("LOCAL_LABEL"));
                assert_eq!(config.label_remote.as_deref(), Some("REMOTE_LABEL"));
            }
            _ => panic!("expected Mergetool mode"),
        }
    }

    #[test]
    fn compat_parses_kdiff3_style_mergetool_without_base() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "local\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let mode = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--auto"),
                OsString::from("--L1"),
                OsString::from("LOCAL_LABEL"),
                OsString::from("--L2"),
                OsString::from("REMOTE_LABEL"),
                OsString::from("--out"),
                merged.clone().into_os_string(),
                local.clone().into_os_string(),
                remote.clone().into_os_string(),
            ],
            &env,
        )
        .unwrap();

        match mode {
            AppMode::Mergetool(config) => {
                assert_eq!(config.merged, merged);
                assert!(config.base.is_none());
                assert_eq!(config.local, local);
                assert_eq!(config.remote, remote);
                assert_eq!(config.label_base, None);
                assert_eq!(config.label_local.as_deref(), Some("LOCAL_LABEL"));
                assert_eq!(config.label_remote.as_deref(), Some("REMOTE_LABEL"));
            }
            _ => panic!("expected Mergetool mode"),
        }
    }

    #[test]
    fn compat_mergetool_applies_merge_conflictstyle_from_git_config() {
        let dir = tempfile::tempdir().unwrap();
        let base = tmp_file(&dir, "base.txt", "base\n");
        let local = tmp_file(&dir, "local.txt", "local\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let mode = parse_mode_for_test_with_config(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--auto"),
                OsString::from("-o"),
                merged.into_os_string(),
                base.into_os_string(),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
            &|key| match key {
                "merge.conflictstyle" => Some("diff3".to_string()),
                _ => None,
            },
        )
        .unwrap();

        match mode {
            AppMode::Mergetool(config) => {
                assert_eq!(config.conflict_style, ConflictStyle::Diff3);
                assert_eq!(config.diff_algorithm, DiffAlgorithm::Myers);
            }
            _ => panic!("expected Mergetool mode"),
        }
    }

    #[test]
    fn compat_mergetool_applies_diff_algorithm_from_git_config() {
        let dir = tempfile::tempdir().unwrap();
        let base = tmp_file(&dir, "base.txt", "base\n");
        let local = tmp_file(&dir, "local.txt", "local\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let mode = parse_mode_for_test_with_config(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--auto"),
                OsString::from("-o"),
                merged.into_os_string(),
                base.into_os_string(),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
            &|key| match key {
                "diff.algorithm" => Some("histogram".to_string()),
                _ => None,
            },
        )
        .unwrap();

        match mode {
            AppMode::Mergetool(config) => {
                assert_eq!(config.conflict_style, ConflictStyle::Merge);
                assert_eq!(config.diff_algorithm, DiffAlgorithm::Histogram);
            }
            _ => panic!("expected Mergetool mode"),
        }
    }

    #[test]
    fn compat_parses_meld_style_mergetool_with_output() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "local\n");
        let base = tmp_file(&dir, "base.txt", "base\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let mode = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--output"),
                merged.clone().into_os_string(),
                local.clone().into_os_string(),
                base.clone().into_os_string(),
                remote.clone().into_os_string(),
            ],
            &env,
        )
        .unwrap();

        match mode {
            AppMode::Mergetool(config) => {
                assert_eq!(config.merged, merged);
                assert_eq!(config.base.as_ref(), Some(&base));
                assert_eq!(config.local, local);
                assert_eq!(config.remote, remote);
                assert_eq!(config.label_base, None);
                assert_eq!(config.label_local, None);
                assert_eq!(config.label_remote, None);
            }
            _ => panic!("expected Mergetool mode"),
        }
    }

    #[test]
    fn compat_parses_meld_style_mergetool_labels() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "local\n");
        let base = tmp_file(&dir, "base.txt", "base\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let mode = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--output"),
                merged.clone().into_os_string(),
                OsString::from("--label=LOCAL_LABEL"),
                OsString::from("--label"),
                OsString::from("BASE_LABEL"),
                OsString::from("-LREMOTE_LABEL"),
                local.clone().into_os_string(),
                base.clone().into_os_string(),
                remote.clone().into_os_string(),
            ],
            &env,
        )
        .unwrap();

        match mode {
            AppMode::Mergetool(config) => {
                assert_eq!(config.merged, merged);
                assert_eq!(config.base.as_ref(), Some(&base));
                assert_eq!(config.local, local);
                assert_eq!(config.remote, remote);
                assert_eq!(config.label_local.as_deref(), Some("LOCAL_LABEL"));
                assert_eq!(config.label_base.as_deref(), Some("BASE_LABEL"));
                assert_eq!(config.label_remote.as_deref(), Some("REMOTE_LABEL"));
            }
            _ => panic!("expected Mergetool mode"),
        }
    }

    #[test]
    fn compat_parses_meld_style_mergetool_with_auto_merge_flag() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "local\n");
        let base = tmp_file(&dir, "base.txt", "base\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let mode = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--auto-merge"),
                OsString::from("--output"),
                merged.clone().into_os_string(),
                local.clone().into_os_string(),
                base.clone().into_os_string(),
                remote.clone().into_os_string(),
            ],
            &env,
        )
        .unwrap();

        match mode {
            AppMode::Mergetool(config) => {
                assert_eq!(config.merged, merged);
                assert_eq!(config.base.as_ref(), Some(&base));
                assert_eq!(config.local, local);
                assert_eq!(config.remote, remote);
            }
            _ => panic!("expected Mergetool mode"),
        }
    }

    #[test]
    fn compat_auto_merge_requires_output_path() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "local\n");
        let base = tmp_file(&dir, "base.txt", "base\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let env = TestEnv::new();

        let err = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--auto-merge"),
                local.into_os_string(),
                base.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap_err();

        assert!(
            err.contains("--auto-merge requires -o/--output/--out"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compat_rejects_too_many_label_flags() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "local\n");
        let base = tmp_file(&dir, "base.txt", "base\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let err = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--output"),
                merged.into_os_string(),
                OsString::from("--label"),
                OsString::from("L1"),
                OsString::from("--label"),
                OsString::from("L2"),
                OsString::from("--label"),
                OsString::from("L3"),
                OsString::from("--label"),
                OsString::from("L4"),
                local.into_os_string(),
                base.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap_err();

        assert!(
            err.contains("too many label flags"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compat_auto_requires_output_path() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "local\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let env = TestEnv::new();

        let err = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--auto"),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap_err();

        assert!(
            err.contains("--auto requires -o/--output/--out"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compat_merge_requires_two_or_three_positionals_after_output_flag() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "local\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let err = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--output"),
                merged.into_os_string(),
                local.into_os_string(),
            ],
            &env,
        )
        .unwrap_err();

        assert!(
            err.contains("expected 2 positional paths (LOCAL REMOTE) or 3"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compat_merge_rejects_too_many_positionals() {
        let dir = tempfile::tempdir().unwrap();
        let base = tmp_file(&dir, "base.txt", "base\n");
        let local = tmp_file(&dir, "local.txt", "local\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let extra = tmp_file(&dir, "extra.txt", "extra\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let err = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--out"),
                merged.into_os_string(),
                base.into_os_string(),
                local.into_os_string(),
                remote.into_os_string(),
                extra.into_os_string(),
            ],
            &env,
        )
        .unwrap_err();

        assert!(
            err.contains("too many positional paths"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compat_merge_rejects_base_flag_with_extra_positionals() {
        let dir = tempfile::tempdir().unwrap();
        let base = tmp_file(&dir, "base.txt", "base\n");
        let local = tmp_file(&dir, "local.txt", "local\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let err = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--base"),
                base.into_os_string(),
                OsString::from("--out"),
                merged.into_os_string(),
                // Invalid: base is passed both via --base and positional arg.
                tmp_file(&dir, "base-positional.txt", "base\n").into_os_string(),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap_err();

        assert!(
            err.contains("--base already supplies BASE"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compat_merge_without_base_rejects_l3_label() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "local\n");
        let remote = tmp_file(&dir, "remote.txt", "remote\n");
        let merged = dir.path().join("merged.txt");
        let env = TestEnv::new();

        let err = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--out"),
                merged.into_os_string(),
                OsString::from("--L3"),
                OsString::from("REMOTE_LABEL"),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap_err();

        assert!(
            err.contains("--L3 requires BASE input"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compat_diff_rejects_l3_without_output_path() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "left.txt", "left\n");
        let remote = tmp_file(&dir, "right.txt", "right\n");
        let env = TestEnv::new();

        let err = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--L3"),
                OsString::from("REMOTE"),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap_err();

        assert!(
            err.contains("--L3 is only valid for merge mode"),
            "error: {err}"
        );
    }

    #[test]
    fn compat_diff_rejects_base_without_output_path() {
        let dir = tempfile::tempdir().unwrap();
        let base = tmp_file(&dir, "base.txt", "base\n");
        let local = tmp_file(&dir, "left.txt", "left\n");
        let remote = tmp_file(&dir, "right.txt", "right\n");
        let env = TestEnv::new();

        let err = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                OsString::from("--base"),
                base.into_os_string(),
                local.into_os_string(),
                remote.into_os_string(),
            ],
            &env,
        )
        .unwrap_err();

        assert!(
            err.contains("--base is only valid for merge mode"),
            "error: {err}"
        );
    }

    #[test]
    fn compat_diff_rejects_too_many_positionals() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "left.txt", "left\n");
        let remote = tmp_file(&dir, "right.txt", "right\n");
        let extra = tmp_file(&dir, "third.txt", "third\n");
        let env = TestEnv::new();

        let err = parse_mode_for_test(
            vec![
                OsString::from("gitgpui-app"),
                local.into_os_string(),
                remote.into_os_string(),
                extra.into_os_string(),
            ],
            &env,
        )
        .unwrap_err();

        assert!(
            err.contains("too many positional paths; expected exactly 2"),
            "error: {err}"
        );
    }

    // ── Conflict style and diff algorithm ─────────────────────────────

    #[test]
    fn mergetool_conflict_style_defaults_to_merge() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "m.txt", "x");
        let local = tmp_file(&dir, "l.txt", "a");
        let remote = tmp_file(&dir, "r.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.conflict_style, ConflictStyle::Merge);
        assert_eq!(config.diff_algorithm, DiffAlgorithm::Myers);
        assert_eq!(config.marker_size, DEFAULT_MARKER_SIZE);
    }

    #[test]
    fn mergetool_conflict_style_diff3() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "m.txt", "x");
        let local = tmp_file(&dir, "l.txt", "a");
        let remote = tmp_file(&dir, "r.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: Some("diff3".into()),
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.conflict_style, ConflictStyle::Diff3);
    }

    #[test]
    fn mergetool_conflict_style_zdiff3() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "m.txt", "x");
        let local = tmp_file(&dir, "l.txt", "a");
        let remote = tmp_file(&dir, "r.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: Some("zdiff3".into()),
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.conflict_style, ConflictStyle::Zdiff3);
    }

    #[test]
    fn mergetool_conflict_style_invalid_errors() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "m.txt", "x");
        let local = tmp_file(&dir, "l.txt", "a");
        let remote = tmp_file(&dir, "r.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: Some("bad".into()),
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("Unknown conflict style"), "error: {err}");
    }

    #[test]
    fn mergetool_diff_algorithm_histogram() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "m.txt", "x");
        let local = tmp_file(&dir, "l.txt", "a");
        let remote = tmp_file(&dir, "r.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: Some("histogram".into()),
            marker_size: None,
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.diff_algorithm, DiffAlgorithm::Histogram);
    }

    #[test]
    fn mergetool_diff_algorithm_invalid_errors() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "m.txt", "x");
        let local = tmp_file(&dir, "l.txt", "a");
        let remote = tmp_file(&dir, "r.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: Some("patience".into()),
            marker_size: None,
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("Unknown diff algorithm"), "error: {err}");
    }

    #[test]
    fn mergetool_marker_size_custom_value() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "m.txt", "x");
        let local = tmp_file(&dir, "l.txt", "a");
        let remote = tmp_file(&dir, "r.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: Some(10),
            auto: false,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.marker_size, 10);
    }

    #[test]
    fn mergetool_marker_size_zero_errors() {
        let dir = tempfile::tempdir().unwrap();
        let merged = tmp_file(&dir, "m.txt", "x");
        let local = tmp_file(&dir, "l.txt", "a");
        let remote = tmp_file(&dir, "r.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: Some(0),
            auto: false,
            gui: false,
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("Invalid marker size"), "error: {err}");
    }

    #[test]
    fn clap_parses_conflict_style_and_diff_algorithm() {
        let cli = Cli::try_parse_from([
            "gitgpui-app",
            "mergetool",
            "--merged",
            "/tmp/m",
            "--local",
            "/tmp/l",
            "--remote",
            "/tmp/r",
            "--conflict-style",
            "zdiff3",
            "--diff-algorithm",
            "histogram",
            "--marker-size",
            "10",
        ])
        .unwrap();

        match cli.command {
            Some(Command::Mergetool(args)) => {
                assert_eq!(args.conflict_style.as_deref(), Some("zdiff3"));
                assert_eq!(args.diff_algorithm.as_deref(), Some("histogram"));
                assert_eq!(args.marker_size, Some(10));
            }
            _ => panic!("expected Mergetool command"),
        }
    }

    // ── Git config fallback ─────────────────────────────────────────

    /// Helper to build mergetool args with no explicit style/algorithm flags.
    fn mergetool_args_no_style(dir: &tempfile::TempDir) -> MergetoolArgs {
        MergetoolArgs {
            merged: Some(tmp_file(dir, "m.txt", "x")),
            local: Some(tmp_file(dir, "l.txt", "a")),
            remote: Some(tmp_file(dir, "r.txt", "b")),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: false,
            gui: false,
        }
    }

    fn mock_git_config(map: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn git_config_fallback_reads_merge_conflictstyle_zdiff3() {
        let dir = tempfile::tempdir().unwrap();
        let args = mergetool_args_no_style(&dir);
        let env = TestEnv::new();
        let mut git_cfg = HashMap::new();
        git_cfg.insert("merge.conflictstyle".into(), "zdiff3".into());

        let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
        assert_eq!(config.conflict_style, ConflictStyle::Zdiff3);
    }

    #[test]
    fn git_config_fallback_reads_merge_conflictstyle_diff3() {
        let dir = tempfile::tempdir().unwrap();
        let args = mergetool_args_no_style(&dir);
        let env = TestEnv::new();
        let mut git_cfg = HashMap::new();
        git_cfg.insert("merge.conflictstyle".into(), "diff3".into());

        let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
        assert_eq!(config.conflict_style, ConflictStyle::Diff3);
    }

    #[test]
    fn git_config_fallback_reads_diff_algorithm_histogram() {
        let dir = tempfile::tempdir().unwrap();
        let args = mergetool_args_no_style(&dir);
        let env = TestEnv::new();
        let mut git_cfg = HashMap::new();
        git_cfg.insert("diff.algorithm".into(), "histogram".into());

        let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
        assert_eq!(config.diff_algorithm, DiffAlgorithm::Histogram);
    }

    #[test]
    fn git_config_fallback_reads_diff_algorithm_patience_as_histogram() {
        let dir = tempfile::tempdir().unwrap();
        let args = mergetool_args_no_style(&dir);
        let env = TestEnv::new();
        let mut git_cfg = HashMap::new();
        git_cfg.insert("diff.algorithm".into(), "patience".into());

        let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
        assert_eq!(config.diff_algorithm, DiffAlgorithm::Histogram);
    }

    #[test]
    fn git_config_fallback_explicit_cli_overrides_git_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut args = mergetool_args_no_style(&dir);
        args.conflict_style = Some("merge".into()); // explicit CLI flag
        args.diff_algorithm = Some("myers".into()); // explicit CLI flag
        let env = TestEnv::new();
        let mut git_cfg = HashMap::new();
        git_cfg.insert("merge.conflictstyle".into(), "zdiff3".into());
        git_cfg.insert("diff.algorithm".into(), "histogram".into());

        let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
        // CLI flags should win over git config.
        assert_eq!(config.conflict_style, ConflictStyle::Merge);
        assert_eq!(config.diff_algorithm, DiffAlgorithm::Myers);
    }

    #[test]
    fn git_config_fallback_no_git_config_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let args = mergetool_args_no_style(&dir);
        let env = TestEnv::new();
        let git_cfg: HashMap<String, String> = HashMap::new();

        let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
        assert_eq!(config.conflict_style, ConflictStyle::Merge);
        assert_eq!(config.diff_algorithm, DiffAlgorithm::Myers);
    }

    #[test]
    fn git_config_fallback_unknown_values_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let args = mergetool_args_no_style(&dir);
        let env = TestEnv::new();
        let mut git_cfg = HashMap::new();
        git_cfg.insert("merge.conflictstyle".into(), "some_future_style".into());
        git_cfg.insert("diff.algorithm".into(), "some_future_algo".into());

        let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
        // Unknown values should be ignored, keeping defaults.
        assert_eq!(config.conflict_style, ConflictStyle::Merge);
        assert_eq!(config.diff_algorithm, DiffAlgorithm::Myers);
    }

    #[test]
    fn git_config_fallback_combined_style_and_algorithm() {
        let dir = tempfile::tempdir().unwrap();
        let args = mergetool_args_no_style(&dir);
        let env = TestEnv::new();
        let mut git_cfg = HashMap::new();
        git_cfg.insert("merge.conflictstyle".into(), "zdiff3".into());
        git_cfg.insert("diff.algorithm".into(), "histogram".into());

        let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
        assert_eq!(config.conflict_style, ConflictStyle::Zdiff3);
        assert_eq!(config.diff_algorithm, DiffAlgorithm::Histogram);
    }

    // ── Auto flag ─────────────────────────────────────────────────────

    #[test]
    fn clap_parses_mergetool_auto_flag() {
        let cli = Cli::try_parse_from([
            "gitgpui-app",
            "mergetool",
            "--merged",
            "/tmp/m",
            "--local",
            "/tmp/l",
            "--remote",
            "/tmp/r",
            "--auto",
        ])
        .unwrap();

        match cli.command {
            Some(Command::Mergetool(args)) => {
                assert!(args.auto, "expected --auto to be true");
            }
            _ => panic!("expected Mergetool command"),
        }
    }

    #[test]
    fn mergetool_auto_flag_propagates_to_config() {
        let dir = tempfile::tempdir().unwrap();
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(tmp_file(&dir, "m.txt", "x")),
            local: Some(tmp_file(&dir, "l.txt", "a")),
            remote: Some(tmp_file(&dir, "r.txt", "b")),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: true,
            gui: false,
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert!(config.auto, "auto flag should propagate to config");
    }

    #[test]
    fn compat_auto_flag_propagates_to_config() {
        let dir = tempfile::tempdir().unwrap();
        let base = tmp_file(&dir, "base.txt", "orig");
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let merged = tmp_file(&dir, "merged.txt", "x");

        let raw_args: Vec<OsString> = vec![
            OsString::from("--auto"),
            OsString::from("-o"),
            merged.as_os_str().to_owned(),
            base.as_os_str().to_owned(),
            local.as_os_str().to_owned(),
            remote.as_os_str().to_owned(),
        ];

        let env = TestEnv::new();
        let no_config = |_: &str| None;

        let mode =
            parse_compat_external_mode_with_config(&raw_args, &env, &no_config)
                .expect("parse ok")
                .expect("should parse compat mode");

        match mode {
            AppMode::Mergetool(config) => {
                assert!(config.auto, "compat --auto should propagate to config");
            }
            _ => panic!("expected Mergetool mode"),
        }
    }

    #[test]
    fn compat_auto_merge_flag_propagates_to_config() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let base = tmp_file(&dir, "base.txt", "orig");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let merged = tmp_file(&dir, "merged.txt", "x");

        let raw_args: Vec<OsString> = vec![
            OsString::from("--auto-merge"),
            OsString::from("--output"),
            merged.as_os_str().to_owned(),
            local.as_os_str().to_owned(),
            base.as_os_str().to_owned(),
            remote.as_os_str().to_owned(),
        ];

        let env = TestEnv::new();
        let no_config = |_: &str| None;

        let mode =
            parse_compat_external_mode_with_config(&raw_args, &env, &no_config)
                .expect("parse ok")
                .expect("should parse compat mode");

        match mode {
            AppMode::Mergetool(config) => {
                assert!(
                    config.auto,
                    "compat --auto-merge should propagate to config"
                );
            }
            _ => panic!("expected Mergetool mode"),
        }
    }
}
