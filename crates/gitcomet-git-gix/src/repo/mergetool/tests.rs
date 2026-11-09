use super::*;

fn open_repo(workdir: &Path) -> gix::Repository {
    gix::open(workdir).unwrap()
}

#[test]
fn test_git_config_get_nonexistent_key_returns_none() {
    // Create a temporary git repo
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    gitcomet_core::test_support::git_fixture::git_command()
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
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .output()
        .unwrap();

    // Set a config value
    gitcomet_core::test_support::git_fixture::git_command()
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
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "extensions.worktreeConfig", "true"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
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
    gitcomet_core::test_support::git_fixture::git_command()
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
fn test_mergetool_path_validation_preserves_relative_components() {
    let normalized = validated_mergetool_path(Path::new("docs/nested/file.txt")).unwrap();
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

#[test]
fn test_build_stage_paths_rejects_paths_outside_worktree_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    for path in ["../outside.txt", "/outside.txt", ".git/config"] {
        assert!(
            build_stage_paths(tmp.path(), Path::new(path), false, false).is_err(),
            "{path:?} must be rejected"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_write_stage_bytes_does_not_follow_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let victim = tmp.path().join("victim");
    std::fs::write(&victim, b"keep").unwrap();
    let stage_path = Path::new("file_BASE_test.txt");
    std::os::unix::fs::symlink(&victim, tmp.path().join(stage_path)).unwrap();

    assert!(write_stage_bytes(tmp.path(), stage_path, b"replace").is_err());
    assert_eq!(std::fs::read(victim).unwrap(), b"keep");
}

#[cfg(windows)]
#[test]
fn test_build_stage_paths_write_to_temp_false_normalizes_windows_separators() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = build_stage_paths(tmp.path(), Path::new("docs/a space.txt"), false, false).unwrap();
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
fn test_run_custom_mergetool_command_windows_executes_quoted_cmd_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    let remote = workdir.join("remote.txt");
    let merged = workdir.join("merged.txt");
    std::fs::write(&remote, b"theirs\n").unwrap();

    let output = run_custom_mergetool_command(
        // Cargo can grow PATH beyond cmd's 8191-character environment
        // limit. Resolve the nested shell independently of that PATH.
        r#""%ComSpec%" /D /S /C "copy /Y "%REMOTE%" "%MERGED%" >NUL""#,
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
        !stdout.contains("copy /Y"),
        "cmd payload should execute, not be echoed as a string expression"
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
    gitcomet_core::test_support::git_fixture::git_command()
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
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .output()
        .unwrap();

    gitcomet_core::test_support::git_fixture::git_command()
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

    gitcomet_core::test_support::git_fixture::git_command()
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
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .output()
        .unwrap();

    gitcomet_core::test_support::git_fixture::git_command()
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
            && message.contains("\"sometimes\"")
    ));
}

#[test]
fn test_git_config_get_bool_bare_key_is_true() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    gitcomet_core::test_support::git_fixture::git_command()
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
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .output()
        .unwrap();

    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "merge.tool", "cli"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "merge.guitool", "gui"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "mergetool.guiDefault", "true"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "mergetool.gui.path", "/opt/fake-gui-tool"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "mergetool.gui.trustExitCode", "yes"])
        .output()
        .unwrap();

    // The repository config names a tool outside git's built-in table, which
    // is gated; this test is about config precedence, so grant consent.
    allow_test_repo_local_mergetool_command(workdir, "gui");
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
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .output()
        .unwrap();

    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "merge.tool", "cli"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "merge.guitool", "gui"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "mergetool.guiDefault", "auto"])
        .output()
        .unwrap();

    // The repository config names a tool outside git's built-in table, which
    // is gated; this test is about config precedence, so grant consent.
    allow_test_repo_local_mergetool_command(workdir, "cli");
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
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .output()
        .unwrap();

    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "merge.tool", "cli"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "mergetool.trustExitCode", "true"])
        .output()
        .unwrap();

    // The repository config names a tool outside git's built-in table, which
    // is gated; this test is about config precedence, so grant consent.
    allow_test_repo_local_mergetool_command(workdir, "cli");
    let repo = open_repo(workdir);
    let cfg = resolve_mergetool_config(&repo, false).unwrap();
    assert!(cfg.trust_exit_code);
}

#[test]
fn test_resolve_mergetool_config_tool_specific_trust_exit_overrides_global() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .output()
        .unwrap();

    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "merge.tool", "cli"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "mergetool.trustExitCode", "true"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "mergetool.cli.trustExitCode", "false"])
        .output()
        .unwrap();

    // The repository config names a tool outside git's built-in table, which
    // is gated; this test is about config precedence, so grant consent.
    allow_test_repo_local_mergetool_command(workdir, "cli");
    let repo = open_repo(workdir);
    let cfg = resolve_mergetool_config(&repo, false).unwrap();
    assert!(!cfg.trust_exit_code);
}

#[test]
fn test_resolve_mergetool_config_reads_write_to_temp() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .output()
        .unwrap();

    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "merge.tool", "cli"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "mergetool.writeToTemp", "true"])
        .output()
        .unwrap();

    // The repository config names a tool outside git's built-in table, which
    // is gated; this test is about config precedence, so grant consent.
    allow_test_repo_local_mergetool_command(workdir, "cli");
    let repo = open_repo(workdir);
    let cfg = resolve_mergetool_config(&repo, false).unwrap();
    assert!(cfg.write_to_temp);
    assert!(!cfg.keep_temporaries);
}

#[test]
fn test_resolve_mergetool_config_reads_keep_temporaries() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .output()
        .unwrap();

    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "merge.tool", "cli"])
        .output()
        .unwrap();
    gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["config", "mergetool.keepTemporaries", "true"])
        .output()
        .unwrap();

    // The repository config names a tool outside git's built-in table, which
    // is gated; this test is about config precedence, so grant consent.
    allow_test_repo_local_mergetool_command(workdir, "cli");
    let repo = open_repo(workdir);
    let cfg = resolve_mergetool_config(&repo, false).unwrap();
    assert!(!cfg.write_to_temp);
    assert!(cfg.keep_temporaries);
}

fn init_repo(workdir: &Path) {
    let status = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("init")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

fn git_config(workdir: &Path, args: &[&str]) {
    let status = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .arg("config")
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git config {args:?} failed");
}

/// Open without the developer's own global config, so the only trusted
/// values are `overrides` (gix source `Api`, a non-repository scope) and
/// everything in `.git/config` is repository-controlled.
fn open_repo_with_trusted_config(workdir: &Path, overrides: &[&str]) -> gix::Repository {
    gix::open_opts(
        workdir,
        gix::open::Options::isolated().config_overrides(overrides.iter().copied()),
    )
    .unwrap()
}

#[test]
fn repo_local_tool_path_is_refused_without_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_repo(workdir);
    git_config(
        workdir,
        &["mergetool.fake.path", "/opt/repo-controlled-tool"],
    );

    let repo = open_repo_with_trusted_config(workdir, &["merge.tool=fake"]);
    let message = resolve_mergetool_config(&repo, false)
        .unwrap_err()
        .to_string();
    assert!(message.contains("mergetool.fake.path"), "{message}");
    assert!(message.contains("/opt/repo-controlled-tool"), "{message}");
    assert!(
        message.contains(&repo_local_mergetool_command_consent_key(workdir, "fake")),
        "{message}"
    );
}

#[test]
fn repo_local_tool_path_is_used_with_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_repo(workdir);
    git_config(
        workdir,
        &["mergetool.fake.path", "/opt/repo-controlled-tool"],
    );
    allow_test_repo_local_mergetool_command(workdir, "fake");

    let repo = open_repo_with_trusted_config(workdir, &["merge.tool=fake"]);
    let cfg = resolve_mergetool_config(&repo, false).unwrap();
    assert_eq!(cfg.tool_name, "fake");
    assert_eq!(cfg.tool_path.as_deref(), Some("/opt/repo-controlled-tool"));
}

#[test]
fn repo_local_tool_path_yields_to_trusted_path_without_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_repo(workdir);
    git_config(
        workdir,
        &["mergetool.fake.path", "/opt/repo-controlled-tool"],
    );

    let repo = open_repo_with_trusted_config(
        workdir,
        &["merge.tool=fake", "mergetool.fake.path=/opt/trusted-tool"],
    );
    let cfg = resolve_mergetool_config(&repo, false).unwrap();
    assert_eq!(cfg.tool_path.as_deref(), Some("/opt/trusted-tool"));
}

#[test]
fn worktree_scoped_tool_path_counts_as_repository_config() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_repo(workdir);
    git_config(workdir, &["extensions.worktreeConfig", "true"]);
    git_config(
        workdir,
        &["--worktree", "mergetool.fake.path", "/opt/worktree-tool"],
    );

    let repo = open_repo_with_trusted_config(workdir, &["merge.tool=fake"]);
    let message = resolve_mergetool_config(&repo, false)
        .unwrap_err()
        .to_string();
    assert!(message.contains("mergetool.fake.path"), "{message}");
    assert!(message.contains("/opt/worktree-tool"), "{message}");
}

#[test]
fn repo_local_unknown_tool_name_is_refused_without_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_repo(workdir);
    // `sh` is not a git built-in, so it would run verbatim from PATH with the
    // conflict stage files as its arguments.
    git_config(workdir, &["merge.tool", "sh"]);

    let repo = open_repo_with_trusted_config(workdir, &[]);
    let message = resolve_mergetool_config(&repo, false)
        .unwrap_err()
        .to_string();
    assert!(message.contains("merge.tool"), "{message}");
    assert!(message.contains("'sh'"), "{message}");
    assert!(
        message.contains(&repo_local_mergetool_command_consent_key(workdir, "sh")),
        "{message}"
    );
}

#[test]
fn repo_local_guitool_name_is_refused_without_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_repo(workdir);
    git_config(workdir, &["merge.guitool", "sh"]);
    git_config(workdir, &["mergetool.guiDefault", "true"]);

    let repo = open_repo_with_trusted_config(workdir, &[]);
    let message = resolve_mergetool_config(&repo, false)
        .unwrap_err()
        .to_string();
    assert!(message.contains("merge.guitool"), "{message}");
}

#[test]
fn repo_local_builtin_tool_name_is_allowed_without_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_repo(workdir);
    git_config(workdir, &["merge.tool", "kdiff3"]);

    let repo = open_repo_with_trusted_config(workdir, &[]);
    let cfg = resolve_mergetool_config(&repo, false).unwrap();
    assert_eq!(cfg.tool_name, "kdiff3");
    assert_eq!(cfg.tool_cmd, None);
    assert_eq!(cfg.tool_path, None);
}

#[test]
fn repo_local_tool_name_yields_to_trusted_tool_without_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_repo(workdir);
    git_config(workdir, &["merge.tool", "sh"]);

    let repo = open_repo_with_trusted_config(workdir, &["merge.tool=trusted-tool"]);
    let cfg = resolve_mergetool_config(&repo, false).unwrap();
    assert_eq!(cfg.tool_name, "trusted-tool");
}

#[test]
fn repo_local_tool_name_is_used_with_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_repo(workdir);
    git_config(workdir, &["merge.tool", "custom"]);
    allow_test_repo_local_mergetool_command(workdir, "custom");

    let repo = open_repo_with_trusted_config(workdir, &[]);
    let cfg = resolve_mergetool_config(&repo, false).unwrap();
    assert_eq!(cfg.tool_name, "custom");
}

#[test]
fn trusted_tool_selection_is_not_gated_when_repository_repeats_it() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_repo(workdir);
    git_config(workdir, &["merge.tool", "custom"]);

    let repo = open_repo_with_trusted_config(workdir, &["merge.tool=custom"]);
    let cfg = resolve_mergetool_config(&repo, false).unwrap();
    assert_eq!(cfg.tool_name, "custom");
}
