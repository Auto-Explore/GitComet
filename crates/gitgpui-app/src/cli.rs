//! CLI argument parsing for gitgpui-app.
//!
//! Supports three modes:
//! - Default (no subcommand): open the full repository browser
//! - `difftool`: focused diff view, compatible with `git difftool`
//! - `mergetool`: focused merge view, compatible with `git mergetool`

use clap::{Parser, Subcommand};
use gitgpui_core::merge::{ConflictStyle, DiffAlgorithm};
use std::ffi::OsString;
use std::path::PathBuf;

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
}

#[derive(clap::Args, Debug)]
pub struct MergetoolArgs {
    /// Path to the merged output file (required).
    #[arg(long)]
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
    #[arg(long)]
    pub label_base: Option<String>,
    /// Label for the local pane.
    #[arg(long)]
    pub label_local: Option<String>,
    /// Label for the remote pane.
    #[arg(long)]
    pub label_remote: Option<String>,
    /// Conflict marker style: merge (default), diff3, or zdiff3.
    #[arg(long, value_name = "STYLE")]
    pub conflict_style: Option<String>,
    /// Diff algorithm: myers (default) or histogram.
    #[arg(long, value_name = "ALGORITHM")]
    pub diff_algorithm: Option<String>,
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

/// Resolve and validate difftool arguments.
///
/// Priority: explicit `--local`/`--remote` flags, then `LOCAL`/`REMOTE` env vars.
/// Both local and remote must resolve to existing files or directories.
fn resolve_difftool_with_env(
    args: DifftoolArgs,
    env: &dyn EnvLookup,
) -> Result<DifftoolConfig, String> {
    let local = resolve_path(args.local, "LOCAL", env)
        .ok_or("Missing required input: --local flag or LOCAL environment variable")?;

    let remote = resolve_path(args.remote, "REMOTE", env)
        .ok_or("Missing required input: --remote flag or REMOTE environment variable")?;

    if !local.exists() {
        return Err(format!("Local path does not exist: {}", local.display()));
    }
    if !remote.exists() {
        return Err(format!("Remote path does not exist: {}", remote.display()));
    }

    // Display path: flag > MERGED env (git difftool compat) > None
    let display_path = args.path.or_else(|| env.var("MERGED"));

    Ok(DifftoolConfig {
        local,
        remote,
        display_path,
        label_left: args.label_left,
        label_right: args.label_right,
    })
}

/// Resolve and validate mergetool arguments.
///
/// Priority: explicit flags, then env vars (MERGED, LOCAL, REMOTE, BASE).
/// merged, local, and remote are required. base is optional.
fn resolve_mergetool_with_env(
    args: MergetoolArgs,
    env: &dyn EnvLookup,
) -> Result<MergetoolConfig, String> {
    let merged = resolve_path(args.merged, "MERGED", env)
        .ok_or("Missing required input: --merged flag or MERGED environment variable")?;

    let local = resolve_path(args.local, "LOCAL", env)
        .ok_or("Missing required input: --local flag or LOCAL environment variable")?;

    let remote = resolve_path(args.remote, "REMOTE", env)
        .ok_or("Missing required input: --remote flag or REMOTE environment variable")?;

    let base = resolve_path(args.base, "BASE", env);

    if !merged.exists() {
        return Err(format!(
            "Merged output path does not exist: {}",
            merged.display()
        ));
    }
    if !local.exists() {
        return Err(format!("Local path does not exist: {}", local.display()));
    }
    if !remote.exists() {
        return Err(format!("Remote path does not exist: {}", remote.display()));
    }

    // Base is allowed to be missing (add/add conflicts have no base).
    // But if explicitly provided, it should exist.
    if let Some(ref base_path) = base {
        if !base_path.exists() {
            return Err(format!("Base path does not exist: {}", base_path.display()));
        }
    }

    let conflict_style = match args.conflict_style.as_deref() {
        None | Some("merge") => ConflictStyle::Merge,
        Some("diff3") => ConflictStyle::Diff3,
        Some("zdiff3") => ConflictStyle::Zdiff3,
        Some(other) => {
            return Err(format!(
                "Unknown conflict style '{other}': expected merge, diff3, or zdiff3"
            ))
        }
    };

    let diff_algorithm = match args.diff_algorithm.as_deref() {
        None | Some("myers") => DiffAlgorithm::Myers,
        Some("histogram") => DiffAlgorithm::Histogram,
        Some(other) => {
            return Err(format!(
                "Unknown diff algorithm '{other}': expected myers or histogram"
            ))
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
    })
}

/// Public resolve wrappers that use the real process environment.
pub fn resolve_difftool(args: DifftoolArgs) -> Result<DifftoolConfig, String> {
    resolve_difftool_with_env(args, &ProcessEnv)
}

pub fn resolve_mergetool(args: MergetoolArgs) -> Result<MergetoolConfig, String> {
    resolve_mergetool_with_env(args, &ProcessEnv)
}

/// Parse CLI arguments and resolve into a validated `AppMode`.
pub fn parse_app_mode() -> Result<AppMode, String> {
    let cli = Cli::try_parse().map_err(|e| e.to_string())?;

    match cli.command {
        None => Ok(AppMode::Browser { path: cli.path }),
        Some(Command::Difftool(args)) => resolve_difftool(args).map(AppMode::Difftool),
        Some(Command::Mergetool(args)) => resolve_mergetool(args).map(AppMode::Mergetool),
    }
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
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.local, local);
        assert_eq!(config.remote, remote);
        assert_eq!(config.display_path.as_deref(), Some("file.txt"));
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
        };

        let err = resolve_difftool_with_env(args, &env).unwrap_err();
        assert!(
            err.contains("does not exist"),
            "error should mention nonexistence: {err}"
        );
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
        };

        let config = resolve_difftool_with_env(args, &env).unwrap();
        assert_eq!(config.local, left_dir);
        assert_eq!(config.remote, right_dir);
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
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.merged, merged);
        assert_eq!(config.local, local);
        assert_eq!(config.remote, remote);
        assert!(config.base.is_none());
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
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("REMOTE"), "error should mention REMOTE: {err}");
    }

    #[test]
    fn mergetool_nonexistent_merged_errors() {
        let dir = tempfile::tempdir().unwrap();
        let local = tmp_file(&dir, "local.txt", "a");
        let remote = tmp_file(&dir, "remote.txt", "b");
        let env = TestEnv::new();

        let args = MergetoolArgs {
            merged: Some(dir.path().join("no_such_merged.txt")),
            local: Some(local),
            remote: Some(remote),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: None,
            diff_algorithm: None,
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("does not exist"), "error: {err}");
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
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("Base path does not exist"), "error: {err}");
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
        };

        let config = resolve_mergetool_with_env(args, &env).unwrap();
        assert_eq!(config.conflict_style, ConflictStyle::Merge);
        assert_eq!(config.diff_algorithm, DiffAlgorithm::Myers);
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
        };

        let err = resolve_mergetool_with_env(args, &env).unwrap_err();
        assert!(err.contains("Unknown diff algorithm"), "error: {err}");
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
        ])
        .unwrap();

        match cli.command {
            Some(Command::Mergetool(args)) => {
                assert_eq!(args.conflict_style.as_deref(), Some("zdiff3"));
                assert_eq!(args.diff_algorithm.as_deref(), Some("histogram"));
            }
            _ => panic!("expected Mergetool command"),
        }
    }
}
