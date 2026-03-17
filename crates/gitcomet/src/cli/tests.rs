use super::*;
use rustc_hash::FxHashMap as HashMap;
use std::io::Write;

/// Test-only environment that avoids calling the unsafe `std::env::set_var`.
struct TestEnv {
    vars: HashMap<String, OsString>,
}

impl TestEnv {
    fn new() -> Self {
        Self {
            vars: HashMap::default(),
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

#[test]
fn install_linux_script_does_not_use_invalid_debug_flag() {
    let script_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install-linux.sh");
    let script = std::fs::read_to_string(&script_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", script_path.display()));

    assert!(
        !script.contains("cargo build -p gitcomet --${mode}"),
        "install script should not forward mode directly as a cargo flag"
    );
}

#[test]
fn parse_mode_mergetool_drops_empty_base_value_before_clap() {
    let dir = tempfile::tempdir().unwrap();
    let merged = tmp_file(&dir, "merged.txt", "conflict");
    let local = tmp_file(&dir, "local.txt", "a");
    let remote = tmp_file(&dir, "remote.txt", "b");

    let mode = parse_mode_for_test(
        vec![
            "gitcomet".into(),
            "mergetool".into(),
            "--base".into(),
            "".into(),
            "--local".into(),
            local.into(),
            "--remote".into(),
            remote.into(),
            "--merged".into(),
            merged.into(),
        ],
        &TestEnv::new(),
    )
    .expect("mergetool parse with empty --base");

    match mode {
        AppMode::Mergetool(config) => {
            assert!(config.base.is_none(), "empty --base should be omitted");
        }
        other => panic!("expected mergetool mode, got: {other:?}"),
    }
}

#[test]
fn parse_mode_mergetool_drops_empty_attached_base_value_before_clap() {
    let dir = tempfile::tempdir().unwrap();
    let merged = tmp_file(&dir, "merged.txt", "conflict");
    let local = tmp_file(&dir, "local.txt", "a");
    let remote = tmp_file(&dir, "remote.txt", "b");

    let mode = parse_mode_for_test(
        vec![
            "gitcomet".into(),
            "mergetool".into(),
            "--base=".into(),
            "--local".into(),
            local.into(),
            "--remote".into(),
            remote.into(),
            "--merged".into(),
            merged.into(),
        ],
        &TestEnv::new(),
    )
    .expect("mergetool parse with empty --base=");

    match mode {
        AppMode::Mergetool(config) => {
            assert!(config.base.is_none(), "empty --base= should be omitted");
        }
        other => panic!("expected mergetool mode, got: {other:?}"),
    }
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

#[cfg(unix)]
#[test]
fn difftool_accepts_broken_symlink_inputs() {
    use std::os::unix::fs as unix_fs;

    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left-link");
    let remote = dir.path().join("right-link");

    unix_fs::symlink("missing-left-target", &local).unwrap();
    unix_fs::symlink("missing-right-target", &remote).unwrap();

    let args = DifftoolArgs {
        local: Some(local.clone()),
        remote: Some(remote.clone()),
        path: None,
        label_left: None,
        label_right: None,
        gui: false,
    };

    let config = resolve_difftool_with_env(args, &TestEnv::new()).unwrap();
    assert_eq!(config.local, local);
    assert_eq!(config.remote, remote);
}

#[cfg(unix)]
#[test]
fn difftool_accepts_symlinked_directory_inputs() {
    use std::os::unix::fs as unix_fs;

    let dir = tempfile::tempdir().unwrap();
    let left_dir = dir.path().join("left");
    let right_dir = dir.path().join("right");
    let left_link = dir.path().join("left-link");
    let right_link = dir.path().join("right-link");

    std::fs::create_dir_all(&left_dir).unwrap();
    std::fs::create_dir_all(&right_dir).unwrap();
    unix_fs::symlink(&left_dir, &left_link).unwrap();
    unix_fs::symlink(&right_dir, &right_link).unwrap();

    let args = DifftoolArgs {
        local: Some(left_link.clone()),
        remote: Some(right_link.clone()),
        path: None,
        label_left: None,
        label_right: None,
        gui: false,
    };

    let config = resolve_difftool_with_env(args, &TestEnv::new()).unwrap();
    assert_eq!(config.local, left_link);
    assert_eq!(config.remote, right_link);
}

#[cfg(unix)]
#[test]
fn difftool_rejects_fifo_input() {
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    let local_fifo = dir.path().join("left.fifo");
    let fifo_status = Command::new("mkfifo")
        .arg(&local_fifo)
        .status()
        .expect("run mkfifo");
    assert!(fifo_status.success(), "mkfifo failed: {fifo_status}");
    let remote = tmp_file(&dir, "remote.txt", "remote\n");

    let args = DifftoolArgs {
        local: Some(local_fifo.clone()),
        remote: Some(remote),
        path: None,
        label_left: None,
        label_right: None,
        gui: false,
    };

    let err = resolve_difftool_with_env(args, &TestEnv::new()).unwrap_err();
    assert!(
        err.contains("must be a regular file or directory"),
        "error should explain supported path kinds: {err}"
    );
    assert!(
        err.contains(&local_fifo.display().to_string()),
        "error should include offending path: {err}"
    );
}

#[cfg(unix)]
#[test]
fn difftool_rejects_symlink_to_fifo_input() {
    use std::os::unix::fs as unix_fs;
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    let fifo_target = dir.path().join("target.fifo");
    let local_link = dir.path().join("left-link");
    let fifo_status = Command::new("mkfifo")
        .arg(&fifo_target)
        .status()
        .expect("run mkfifo");
    assert!(fifo_status.success(), "mkfifo failed: {fifo_status}");
    unix_fs::symlink(&fifo_target, &local_link).expect("create symlink to fifo");

    let remote = tmp_file(&dir, "remote.txt", "remote\n");
    let args = DifftoolArgs {
        local: Some(local_link.clone()),
        remote: Some(remote),
        path: None,
        label_left: None,
        label_right: None,
        gui: false,
    };

    let err = resolve_difftool_with_env(args, &TestEnv::new()).unwrap_err();
    assert!(
        err.contains("symlink target must resolve to a regular file or directory"),
        "error should explain unsupported symlink targets: {err}"
    );
    assert!(
        err.contains(&local_link.display().to_string()),
        "error should include offending symlink path: {err}"
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
fn mergetool_empty_base_flag_treated_as_missing() {
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

    let config = resolve_mergetool_with_env(args, &TestEnv::new()).unwrap();
    assert!(
        config.base.is_none(),
        "empty explicit --base should be treated as no-base"
    );
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
    assert!(
        err.contains("Merged path must be a file path"),
        "error: {err}"
    );
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

#[cfg(unix)]
#[test]
fn mergetool_local_fifo_errors() {
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    let merged = dir.path().join("merged.txt");
    let local_fifo = dir.path().join("local.fifo");
    let remote = tmp_file(&dir, "remote.txt", "remote\n");

    let fifo_status = Command::new("mkfifo")
        .arg(&local_fifo)
        .status()
        .expect("run mkfifo");
    assert!(fifo_status.success(), "mkfifo failed: {fifo_status}");

    let args = MergetoolArgs {
        merged: Some(merged),
        local: Some(local_fifo.clone()),
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
    assert!(
        err.contains("Local path must be a regular file"),
        "error should explain regular-file requirement: {err}"
    );
    assert!(
        err.contains(&local_fifo.display().to_string()),
        "error should include offending local path: {err}"
    );
}

#[cfg(unix)]
#[test]
fn mergetool_local_symlink_to_fifo_errors() {
    use std::os::unix::fs as unix_fs;
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    let merged = dir.path().join("merged.txt");
    let fifo_target = dir.path().join("local-target.fifo");
    let local_link = dir.path().join("local-link");
    let remote = tmp_file(&dir, "remote.txt", "remote\n");

    let fifo_status = Command::new("mkfifo")
        .arg(&fifo_target)
        .status()
        .expect("run mkfifo");
    assert!(fifo_status.success(), "mkfifo failed: {fifo_status}");
    unix_fs::symlink(&fifo_target, &local_link).expect("create symlink to fifo");

    let args = MergetoolArgs {
        merged: Some(merged),
        local: Some(local_link.clone()),
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
    assert!(
        err.contains("Local path must be a regular file"),
        "error should explain unsupported symlink target: {err}"
    );
    assert!(
        err.contains(&local_link.display().to_string()),
        "error should include offending symlink path: {err}"
    );
}

#[cfg(unix)]
#[test]
fn mergetool_existing_merged_fifo_errors() {
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    let merged_fifo = dir.path().join("merged.fifo");
    let local = tmp_file(&dir, "local.txt", "local\n");
    let remote = tmp_file(&dir, "remote.txt", "remote\n");

    let fifo_status = Command::new("mkfifo")
        .arg(&merged_fifo)
        .status()
        .expect("run mkfifo");
    assert!(fifo_status.success(), "mkfifo failed: {fifo_status}");

    let args = MergetoolArgs {
        merged: Some(merged_fifo.clone()),
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
    assert!(
        err.contains("Merged path must be a regular file path"),
        "error should explain merged-path regular-file requirement: {err}"
    );
    assert!(
        err.contains(&merged_fifo.display().to_string()),
        "error should include offending merged path: {err}"
    );
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

// ── Subdirectory path resolution ────────────────────────────────

#[test]
fn difftool_resolves_paths_in_nested_subdirectory() {
    let dir = tempfile::tempdir().unwrap();
    let subdir = dir.path().join("src").join("lib");
    std::fs::create_dir_all(&subdir).unwrap();
    let local = {
        let p = subdir.join("module.rs");
        std::fs::write(&p, "fn old() {}").unwrap();
        p
    };
    let remote = {
        let p = subdir.join("module_REMOTE.rs");
        std::fs::write(&p, "fn new() {}").unwrap();
        p
    };
    let env = TestEnv::new();

    let args = DifftoolArgs {
        local: Some(local.clone()),
        remote: Some(remote.clone()),
        path: Some("src/lib/module.rs".into()),
        label_left: None,
        label_right: None,
        gui: false,
    };

    let config = resolve_difftool_with_env(args, &env).unwrap();
    assert_eq!(config.local, local);
    assert_eq!(config.remote, remote);
    assert_eq!(config.display_path.as_deref(), Some("src/lib/module.rs"));
}

#[test]
fn mergetool_resolves_paths_in_nested_subdirectory() {
    let dir = tempfile::tempdir().unwrap();
    let subdir = dir.path().join("packages").join("core").join("src");
    std::fs::create_dir_all(&subdir).unwrap();
    let merged = {
        let p = subdir.join("index.ts");
        std::fs::write(&p, "conflict").unwrap();
        p
    };
    let local = {
        let p = subdir.join("index_LOCAL.ts");
        std::fs::write(&p, "local").unwrap();
        p
    };
    let remote = {
        let p = subdir.join("index_REMOTE.ts");
        std::fs::write(&p, "remote").unwrap();
        p
    };
    let base = {
        let p = subdir.join("index_BASE.ts");
        std::fs::write(&p, "base").unwrap();
        p
    };
    let env = TestEnv::new();

    let args = MergetoolArgs {
        merged: Some(merged.clone()),
        local: Some(local.clone()),
        remote: Some(remote.clone()),
        base: Some(base.clone()),
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
fn mergetool_env_resolution_with_subdirectory_paths() {
    // Simulates `git mergetool` providing paths via environment variables
    // when invoked from a repo subdirectory.
    let dir = tempfile::tempdir().unwrap();
    let subdir = dir.path().join("deep").join("nested");
    std::fs::create_dir_all(&subdir).unwrap();
    let merged = {
        let p = subdir.join("file.txt");
        std::fs::write(&p, "x").unwrap();
        p
    };
    let local = {
        let p = subdir.join("file_LOCAL.txt");
        std::fs::write(&p, "a").unwrap();
        p
    };
    let remote = {
        let p = subdir.join("file_REMOTE.txt");
        std::fs::write(&p, "b").unwrap();
        p
    };
    let base = {
        let p = subdir.join("file_BASE.txt");
        std::fs::write(&p, "o").unwrap();
        p
    };

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
    assert_eq!(config.local, local);
    assert_eq!(config.remote, remote);
    assert_eq!(config.base.as_ref(), Some(&base));
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
        "gitcomet", "difftool", "--local", "/tmp/a", "--remote", "/tmp/b", "--path", "foo.txt",
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
        "gitcomet",
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
            "gitcomet",
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
        "gitcomet",
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
fn clap_parses_setup_subcommand() {
    let cli = Cli::try_parse_from(["gitcomet", "setup", "--dry-run", "--local"]).unwrap();

    match cli.command {
        Some(Command::Setup(args)) => {
            assert!(args.dry_run);
            assert!(args.local);
        }
        other => panic!("expected Setup command, got: {other:?}"),
    }
}

#[test]
fn clap_parses_uninstall_subcommand() {
    let cli = Cli::try_parse_from(["gitcomet", "uninstall", "--dry-run", "--local"]).unwrap();

    match cli.command {
        Some(Command::Uninstall(args)) => {
            assert!(args.dry_run);
            assert!(args.local);
        }
        other => panic!("expected Uninstall command, got: {other:?}"),
    }
}

#[test]
fn uninstall_mode_resolves_into_app_mode() {
    let env = TestEnv::new();
    let mode = parse_mode_for_test(
        vec![
            OsString::from("gitcomet"),
            OsString::from("uninstall"),
            OsString::from("--dry-run"),
            OsString::from("--local"),
        ],
        &env,
    )
    .expect("parse uninstall mode");

    match mode {
        AppMode::Uninstall { dry_run, local } => {
            assert!(dry_run);
            assert!(local);
        }
        other => panic!("expected Uninstall mode, got: {other:?}"),
    }
}

#[test]
fn clap_parses_no_subcommand_as_browser() {
    let cli = Cli::try_parse_from(["gitcomet"]).unwrap();
    assert!(cli.command.is_none());
    assert!(cli.path.is_none());
}

#[test]
fn clap_parses_path_argument() {
    let cli = Cli::try_parse_from(["gitcomet", "/some/repo"]).unwrap();
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
fn compat_parses_kdiff3_style_difftool_short_numbered_equals_labels() {
    let dir = tempfile::tempdir().unwrap();
    let local = tmp_file(&dir, "left.txt", "left\n");
    let remote = tmp_file(&dir, "right.txt", "right\n");
    let env = TestEnv::new();

    let mode = parse_mode_for_test(
        vec![
            OsString::from("gitcomet"),
            OsString::from("-L1=LEFT_LABEL"),
            OsString::from("-L2=RIGHT_LABEL"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
fn compat_parses_kdiff3_style_mergetool_with_short_numbered_label_flags() {
    let dir = tempfile::tempdir().unwrap();
    let base = tmp_file(&dir, "base.txt", "base\n");
    let local = tmp_file(&dir, "local.txt", "local\n");
    let remote = tmp_file(&dir, "remote.txt", "remote\n");
    let merged = dir.path().join("merged.txt");
    let env = TestEnv::new();

    let mode = parse_mode_for_test(
        vec![
            OsString::from("gitcomet"),
            OsString::from("--auto"),
            OsString::from("-L1"),
            OsString::from("BASE_LABEL"),
            OsString::from("-L2"),
            OsString::from("LOCAL_LABEL"),
            OsString::from("-L3"),
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
fn compat_parses_kdiff3_style_mergetool_with_attached_short_numbered_label_flags() {
    let dir = tempfile::tempdir().unwrap();
    let base = tmp_file(&dir, "base.txt", "base\n");
    let local = tmp_file(&dir, "local.txt", "local\n");
    let remote = tmp_file(&dir, "remote.txt", "remote\n");
    let merged = dir.path().join("merged.txt");
    let env = TestEnv::new();

    let mode = parse_mode_for_test(
        vec![
            OsString::from("gitcomet"),
            OsString::from("--auto"),
            OsString::from("-L1BASE_LABEL"),
            OsString::from("-L2=LOCAL_LABEL"),
            OsString::from("-L3REMOTE_LABEL"),
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
fn compat_parses_kdiff3_style_mergetool_with_attached_output_and_base_flags() {
    let dir = tempfile::tempdir().unwrap();
    let base = tmp_file(&dir, "base file.txt", "base\n");
    let local = tmp_file(&dir, "local file.txt", "local\n");
    let remote = tmp_file(&dir, "remote file.txt", "remote\n");
    let merged = dir.path().join("merged output.txt");
    let env = TestEnv::new();

    let mode = parse_mode_for_test(
        vec![
            OsString::from("gitcomet"),
            OsString::from("--auto"),
            OsString::from("--L1=BASE_LABEL"),
            OsString::from("--L2=LOCAL_LABEL"),
            OsString::from("--L3=REMOTE_LABEL"),
            OsString::from(format!("--base={}", base.display())),
            OsString::from(format!("--out={}", merged.display())),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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
            OsString::from("gitcomet"),
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

#[test]
fn compat_direct_missing_values_for_value_flags_error() {
    let env = TestEnv::new();
    let no_config = |_: &str| None;
    let cases: Vec<Vec<OsString>> = vec![
        vec![OsString::from("--L1")],
        vec![OsString::from("-L1")],
        vec![OsString::from("--label")],
        vec![OsString::from("-o")],
        vec![OsString::from("--base")],
    ];

    for raw_args in cases {
        let err = parse_compat_external_mode_with_config(&raw_args, &env, &no_config).unwrap_err();
        assert!(
            err.contains("Missing value for compatibility flag"),
            "unexpected error for args {raw_args:?}: {err}"
        );
    }
}

#[test]
fn compat_direct_unknown_flag_and_empty_input_return_none() {
    let env = TestEnv::new();
    let no_config = |_: &str| None;

    let unknown = vec![OsString::from("--unknown-external-flag")];
    assert!(
        parse_compat_external_mode_with_config(&unknown, &env, &no_config)
            .unwrap()
            .is_none()
    );

    let empty: Vec<OsString> = Vec::new();
    assert!(
        parse_compat_external_mode_with_config(&empty, &env, &no_config)
            .unwrap()
            .is_none()
    );
}

#[test]
fn compat_direct_diff_labels_without_positionals_error() {
    let env = TestEnv::new();
    let no_config = |_: &str| None;
    let raw_args = vec![
        OsString::from("--L1"),
        OsString::from("LEFT_LABEL"),
        OsString::from("--L2"),
        OsString::from("RIGHT_LABEL"),
    ];

    let err = parse_compat_external_mode_with_config(&raw_args, &env, &no_config).unwrap_err();
    assert!(
        err.contains("expected 2 positional paths (LOCAL REMOTE)"),
        "unexpected error: {err}"
    );
}

#[test]
fn compat_parses_attached_short_output_and_label_equals_forms() {
    let dir = tempfile::tempdir().unwrap();
    let base = tmp_file(&dir, "base.txt", "base\n");
    let local = tmp_file(&dir, "local.txt", "local\n");
    let remote = tmp_file(&dir, "remote.txt", "remote\n");
    let merged = dir.path().join("merged.txt");
    let env = TestEnv::new();
    let no_config = |_: &str| None;
    let raw_args = vec![
        OsString::from("-L1=BASE_LABEL"),
        OsString::from("-L2LOCAL_LABEL"),
        OsString::from("-L3=REMOTE_LABEL"),
        OsString::from(format!("-o{}", merged.display())),
        base.clone().into_os_string(),
        local.clone().into_os_string(),
        remote.clone().into_os_string(),
    ];

    let mode = parse_compat_external_mode_with_config(&raw_args, &env, &no_config)
        .expect("parse ok")
        .expect("compat mode should resolve");

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
        other => panic!("expected Mergetool mode, got: {other:?}"),
    }
}

#[test]
fn compat_double_dash_collects_remaining_positionals_for_difftool() {
    let dir = tempfile::tempdir().unwrap();
    let local = tmp_file(&dir, "left.txt", "left\n");
    let remote = tmp_file(&dir, "right.txt", "right\n");
    let env = TestEnv::new();
    let no_config = |_: &str| None;
    let raw_args = vec![
        OsString::from("--"),
        local.clone().into_os_string(),
        remote.clone().into_os_string(),
    ];

    let mode = parse_compat_external_mode_with_config(&raw_args, &env, &no_config)
        .expect("parse ok")
        .expect("compat mode should resolve");

    match mode {
        AppMode::Difftool(config) => {
            assert_eq!(config.local, local);
            assert_eq!(config.remote, remote);
        }
        other => panic!("expected Difftool mode, got: {other:?}"),
    }
}

#[test]
fn compat_base_flag_requires_two_positionals_when_output_present() {
    let dir = tempfile::tempdir().unwrap();
    let base = tmp_file(&dir, "base.txt", "base\n");
    let local = tmp_file(&dir, "local.txt", "local\n");
    let merged = dir.path().join("merged.txt");
    let env = TestEnv::new();

    let err = parse_mode_for_test(
        vec![
            OsString::from("gitcomet"),
            OsString::from("--base"),
            base.into_os_string(),
            OsString::from("--out"),
            merged.into_os_string(),
            local.into_os_string(),
        ],
        &env,
    )
    .unwrap_err();

    assert!(
        err.contains("expected exactly 2 positional paths (LOCAL REMOTE) when --base is provided"),
        "unexpected error: {err}"
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
        "gitcomet",
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
    let mut git_cfg = HashMap::default();
    git_cfg.insert("merge.conflictstyle".into(), "zdiff3".into());

    let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
    assert_eq!(config.conflict_style, ConflictStyle::Zdiff3);
}

#[test]
fn git_config_fallback_reads_merge_conflictstyle_diff3() {
    let dir = tempfile::tempdir().unwrap();
    let args = mergetool_args_no_style(&dir);
    let env = TestEnv::new();
    let mut git_cfg = HashMap::default();
    git_cfg.insert("merge.conflictstyle".into(), "diff3".into());

    let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
    assert_eq!(config.conflict_style, ConflictStyle::Diff3);
}

#[test]
fn git_config_fallback_reads_diff_algorithm_histogram() {
    let dir = tempfile::tempdir().unwrap();
    let args = mergetool_args_no_style(&dir);
    let env = TestEnv::new();
    let mut git_cfg = HashMap::default();
    git_cfg.insert("diff.algorithm".into(), "histogram".into());

    let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
    assert_eq!(config.diff_algorithm, DiffAlgorithm::Histogram);
}

#[test]
fn git_config_fallback_reads_diff_algorithm_patience_as_histogram() {
    let dir = tempfile::tempdir().unwrap();
    let args = mergetool_args_no_style(&dir);
    let env = TestEnv::new();
    let mut git_cfg = HashMap::default();
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
    let mut git_cfg = HashMap::default();
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
    let git_cfg: HashMap<String, String> = HashMap::default();

    let config = resolve_mergetool_with_config(args, &env, &mock_git_config(&git_cfg)).unwrap();
    assert_eq!(config.conflict_style, ConflictStyle::Merge);
    assert_eq!(config.diff_algorithm, DiffAlgorithm::Myers);
}

#[test]
fn git_config_fallback_unknown_values_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let args = mergetool_args_no_style(&dir);
    let env = TestEnv::new();
    let mut git_cfg = HashMap::default();
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
    let mut git_cfg = HashMap::default();
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
        "gitcomet",
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
fn clap_parses_mergetool_auto_merge_alias_flag() {
    let cli = Cli::try_parse_from([
        "gitcomet",
        "mergetool",
        "--merged",
        "/tmp/m",
        "--local",
        "/tmp/l",
        "--remote",
        "/tmp/r",
        "--auto-merge",
    ])
    .unwrap();

    match cli.command {
        Some(Command::Mergetool(args)) => {
            assert!(args.auto, "expected --auto-merge alias to set auto=true");
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

    let mode = parse_compat_external_mode_with_config(&raw_args, &env, &no_config)
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

    let mode = parse_compat_external_mode_with_config(&raw_args, &env, &no_config)
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

#[test]
fn clap_parses_extract_merge_fixtures_subcommand() {
    let cli = Cli::try_parse_from([
        "gitcomet",
        "extract-merge-fixtures",
        "--repo",
        "/tmp/repo",
        "--out",
        "/tmp/out",
        "--max-merges",
        "42",
        "--max-files-per-merge",
        "9",
    ])
    .unwrap();

    match cli.command {
        Some(Command::ExtractMergeFixtures(args)) => {
            assert_eq!(args.repo, PathBuf::from("/tmp/repo"));
            assert_eq!(args.out, PathBuf::from("/tmp/out"));
            assert_eq!(args.max_merges, 42);
            assert_eq!(args.max_files_per_merge, 9);
        }
        other => panic!("expected ExtractMergeFixtures command, got: {other:?}"),
    }
}

#[test]
fn extract_merge_fixtures_mode_resolves_into_app_mode() {
    let env = TestEnv::new();
    let mode = parse_mode_for_test(
        vec![
            "gitcomet".into(),
            "extract-merge-fixtures".into(),
            "--repo".into(),
            "/tmp/repo".into(),
            "--out".into(),
            "/tmp/out".into(),
        ],
        &env,
    )
    .expect("parse extract-merge-fixtures mode");

    match mode {
        AppMode::ExtractMergeFixtures(config) => {
            assert_eq!(config.repo, PathBuf::from("/tmp/repo"));
            assert_eq!(config.output_dir, PathBuf::from("/tmp/out"));
            assert_eq!(config.max_merges, 20);
            assert_eq!(config.max_files_per_merge, 5);
        }
        other => panic!("expected ExtractMergeFixtures mode, got: {other:?}"),
    }
}

#[test]
fn extract_merge_fixtures_rejects_zero_limits() {
    let err = resolve_extract_merge_fixtures(ExtractMergeFixturesArgs {
        repo: PathBuf::from("."),
        out: PathBuf::from("fixtures"),
        max_merges: 0,
        max_files_per_merge: 1,
    })
    .expect_err("zero max-merges should error");
    assert!(err.contains("--max-merges"), "unexpected error: {err}");

    let err = resolve_extract_merge_fixtures(ExtractMergeFixturesArgs {
        repo: PathBuf::from("."),
        out: PathBuf::from("fixtures"),
        max_merges: 1,
        max_files_per_merge: 0,
    })
    .expect_err("zero max-files-per-merge should error");
    assert!(
        err.contains("--max-files-per-merge"),
        "unexpected error: {err}"
    );
}
