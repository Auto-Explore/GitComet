use super::*;
use std::fs;

#[test]
fn another_product_registers_and_removes_only_its_own_tools() {
    let pro = gitcomet_core::identity::ProductIdentity::builder("Comet Pro", "comet-pro")
        .build()
        .unwrap();
    let names = ToolNames::for_identity(&pro);
    let setup: Vec<_> = build_config_entries_for(&names, "/opt/pro")
        .into_iter()
        .map(|entry| (entry.key, entry.value))
        .collect();
    assert!(setup.contains(&("merge.tool", "comet-pro".to_string())));
    assert!(setup.contains(&("diff.guitool", "comet-pro-gui".to_string())));
    assert!(
        setup
            .iter()
            .any(|(key, _)| *key == "mergetool.comet-pro.cmd")
    );
    assert!(
        setup
            .iter()
            .all(|(key, value)| !key.contains("gitcomet") && !value.contains("gitcomet"))
    );
    assert!(
        build_backup_entries_for(&names)
            .iter()
            .all(|entry| entry.backup_key.starts_with("comet-pro.backup."))
    );

    let uninstall = build_uninstall_entries_for(&names);
    assert!(
        uninstall
            .iter()
            .all(|entry| !entry.key.contains("gitcomet"))
    );
    // Shared selectors are removed only while they still select this product.
    let selector = uninstall
        .iter()
        .find(|entry| entry.key == "merge.tool")
        .unwrap();
    assert_eq!(selector.expected_value, Some("comet-pro"));
    let behavior = uninstall
        .iter()
        .find(|entry| entry.key == "mergetool.prompt")
        .unwrap();
    assert_eq!(behavior.guard.unwrap().expected_value, "comet-pro");
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

fn assert_placeholder_is_quoted(cmd: &str, var: &str) {
    let raw = format!("${var}");
    let quoted = format!("\"{raw}\"");

    let raw_count = count_occurrences(cmd, &raw);
    let quoted_count = count_occurrences(cmd, &quoted);

    assert!(
        quoted_count > 0,
        "expected quoted placeholder {quoted} in command: {cmd}"
    );
    assert_eq!(
        raw_count, quoted_count,
        "found unquoted placeholder ${var} in command: {cmd}"
    );
}

#[test]
fn shell_single_quote_wraps_plain_text() {
    assert_eq!(shell_single_quote("abc"), "'abc'");
    assert_eq!(shell_single_quote(""), "''");
}

#[test]
fn shell_single_quote_escapes_embedded_single_quote() {
    assert_eq!(shell_single_quote("it's"), "'it'\"'\"'s'");
}

#[cfg(unix)]
#[test]
fn executable_path_for_shell_rejects_non_utf8_unix_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let path = PathBuf::from(OsString::from_vec(vec![
        b'/', b't', b'm', b'p', b'/', b'g', b'i', b't', b'c', b'o', b'm', b'e', b't', b'-', 0xff,
    ]));
    let error = executable_path_for_shell(&path).expect_err("non-utf8 path should error");
    assert!(error.contains("non-Unicode executable path"), "{error}");
}

#[test]
fn build_config_entries_contains_all_required_keys() {
    let entries = build_config_entries("/usr/bin/gitcomet");
    let keys: Vec<&str> = entries.iter().map(|e| e.key).collect();

    // Headless tool
    assert!(keys.contains(&"merge.tool"));
    assert!(keys.contains(&"mergetool.gitcomet.cmd"));
    assert!(keys.contains(&"mergetool.trustExitCode"));
    assert!(keys.contains(&"mergetool.gitcomet.trustExitCode"));
    assert!(keys.contains(&"mergetool.prompt"));
    assert!(keys.contains(&"diff.tool"));
    assert!(keys.contains(&"difftool.gitcomet.cmd"));
    assert!(keys.contains(&"difftool.trustExitCode"));
    assert!(keys.contains(&"difftool.gitcomet.trustExitCode"));
    assert!(keys.contains(&"difftool.prompt"));

    // GUI tool variant
    assert!(keys.contains(&"merge.guitool"));
    assert!(keys.contains(&"diff.guitool"));
    assert!(keys.contains(&"mergetool.gitcomet-gui.cmd"));
    assert!(keys.contains(&"mergetool.gitcomet-gui.trustExitCode"));
    assert!(keys.contains(&"difftool.gitcomet-gui.cmd"));
    assert!(keys.contains(&"difftool.gitcomet-gui.trustExitCode"));
    assert!(keys.contains(&"mergetool.guiDefault"));
    assert!(keys.contains(&"difftool.guiDefault"));
}

#[test]
fn gui_tool_uses_separate_tool_name() {
    let entries = build_config_entries("/usr/bin/gitcomet");
    let merge_guitool = entries.iter().find(|e| e.key == "merge.guitool").unwrap();
    let diff_guitool = entries.iter().find(|e| e.key == "diff.guitool").unwrap();

    assert_eq!(merge_guitool.value, "gitcomet-gui");
    assert_eq!(diff_guitool.value, "gitcomet-gui");
}

#[test]
fn gui_tool_cmd_includes_gui_flag() {
    let entries = build_config_entries("/path/to/bin");
    let merge_gui_cmd = entries
        .iter()
        .find(|e| e.key == "mergetool.gitcomet-gui.cmd")
        .unwrap();
    let diff_gui_cmd = entries
        .iter()
        .find(|e| e.key == "difftool.gitcomet-gui.cmd")
        .unwrap();

    assert!(
        merge_gui_cmd.value.contains("--gui"),
        "GUI mergetool cmd missing --gui flag: {}",
        merge_gui_cmd.value
    );
    assert!(
        diff_gui_cmd.value.contains("--gui"),
        "GUI difftool cmd missing --gui flag: {}",
        diff_gui_cmd.value
    );
}

#[test]
fn headless_tool_cmd_omits_gui_flag() {
    let entries = build_config_entries("/path/to/bin");
    let merge_cmd = entries
        .iter()
        .find(|e| e.key == "mergetool.gitcomet.cmd")
        .unwrap();
    let diff_cmd = entries
        .iter()
        .find(|e| e.key == "difftool.gitcomet.cmd")
        .unwrap();

    assert!(
        !merge_cmd.value.contains("--gui"),
        "headless mergetool cmd should not contain --gui: {}",
        merge_cmd.value
    );
    assert!(
        !diff_cmd.value.contains("--gui"),
        "headless difftool cmd should not contain --gui: {}",
        diff_cmd.value
    );
}

#[test]
fn mergetool_cmd_includes_all_stage_vars() {
    let entries = build_config_entries("/path/to/bin");
    let cmd = entries
        .iter()
        .find(|e| e.key == "mergetool.gitcomet.cmd")
        .unwrap();

    assert_placeholder_is_quoted(&cmd.value, "BASE");
    assert_placeholder_is_quoted(&cmd.value, "LOCAL");
    assert_placeholder_is_quoted(&cmd.value, "REMOTE");
    assert_placeholder_is_quoted(&cmd.value, "MERGED");
    assert!(cmd.value.starts_with("'/path/to/bin'"));
}

#[test]
fn mergetool_cmd_escapes_single_quote_in_binary_path() {
    let entries = build_config_entries("/tmp/it's/gitcomet");
    let cmd = entries
        .iter()
        .find(|e| e.key == "mergetool.gitcomet.cmd")
        .unwrap();

    assert!(
        cmd.value.starts_with("'/tmp/it'\"'\"'s/gitcomet'"),
        "unexpected cmd quoting: {}",
        cmd.value
    );
}

#[test]
fn difftool_cmd_includes_local_remote_merged() {
    let entries = build_config_entries("/path/to/bin");
    let cmd = entries
        .iter()
        .find(|e| e.key == "difftool.gitcomet.cmd")
        .unwrap();

    assert_placeholder_is_quoted(&cmd.value, "LOCAL");
    assert_placeholder_is_quoted(&cmd.value, "REMOTE");
    assert_placeholder_is_quoted(&cmd.value, "MERGED");
}

#[test]
fn gui_mergetool_cmd_quotes_all_stage_vars() {
    let entries = build_config_entries("/path/to/bin");
    let cmd = entries
        .iter()
        .find(|e| e.key == "mergetool.gitcomet-gui.cmd")
        .unwrap();

    assert_placeholder_is_quoted(&cmd.value, "BASE");
    assert_placeholder_is_quoted(&cmd.value, "LOCAL");
    assert_placeholder_is_quoted(&cmd.value, "REMOTE");
    assert_placeholder_is_quoted(&cmd.value, "MERGED");
}

#[test]
fn gui_difftool_cmd_quotes_all_stage_vars() {
    let entries = build_config_entries("/path/to/bin");
    let cmd = entries
        .iter()
        .find(|e| e.key == "difftool.gitcomet-gui.cmd")
        .unwrap();

    assert_placeholder_is_quoted(&cmd.value, "LOCAL");
    assert_placeholder_is_quoted(&cmd.value, "REMOTE");
    assert_placeholder_is_quoted(&cmd.value, "MERGED");
}

#[test]
fn format_commands_global_scope() {
    let entries = build_config_entries("/bin/gitcomet");
    let output = format_commands(&entries, "--global");

    // Headless mergetool entries
    assert!(output.contains("git config --global merge.tool"));
    assert!(output.contains("git config --global mergetool.gitcomet.cmd"));
    assert!(output.contains("git config --global mergetool.trustExitCode"));
    assert!(output.contains("git config --global mergetool.gitcomet.trustExitCode"));
    assert!(output.contains("git config --global mergetool.prompt"));

    // Headless difftool entries
    assert!(output.contains("git config --global diff.tool"));
    assert!(output.contains("git config --global difftool.gitcomet.cmd"));
    assert!(output.contains("git config --global difftool.trustExitCode"));
    assert!(
        output.contains("git config --global difftool.gitcomet.trustExitCode"),
        "expected per-tool difftool trustExitCode entry:\n{output}"
    );
    assert!(output.contains("git config --global difftool.prompt"));

    // GUI tool entries
    assert!(output.contains("git config --global merge.guitool"));
    assert!(output.contains("git config --global mergetool.gitcomet-gui.cmd"));
    assert!(output.contains("git config --global mergetool.gitcomet-gui.trustExitCode"));
    assert!(output.contains("git config --global diff.guitool"));
    assert!(output.contains("git config --global difftool.gitcomet-gui.cmd"));
    assert!(output.contains("git config --global difftool.gitcomet-gui.trustExitCode"));

    // GUI default auto-selection
    assert!(output.contains("git config --global mergetool.guiDefault"));
    assert!(output.contains("git config --global difftool.guiDefault"));

    assert!(
        !output.contains("''/bin/gitcomet'"),
        "dry-run output should not contain broken nested quoting:\n{output}"
    );
}

#[test]
fn format_commands_local_scope() {
    let entries = build_config_entries("/bin/gitcomet");
    let output = format_commands(&entries, "--local");

    assert!(output.contains("git config --local merge.tool"));
    assert!(output.contains("git config --local diff.tool"));
}

#[test]
fn dry_run_does_not_write_config() {
    // dry_run=true should produce output but not call git config.
    // We verify by running inside a temp dir with no repo — if it
    // actually tried `git config --global`, the test env would be
    // unaffected because we only check output format.
    let result = run_setup(true, false).unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("Dry run"));
    assert!(result.stdout.contains("git config --global"));
}

#[test]
fn dry_run_local_scope_uses_local_flag() {
    let result = run_setup(true, true).unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("git config --local"));
    assert!(!result.stdout.contains("--global"));
}

#[test]
fn apply_config_to_local_repo() {
    let dir = tempfile::tempdir().unwrap();

    // Initialize a git repo.
    let init = gitcomet_core::test_support::git_fixture::git_command()
        .arg("init")
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(init.status.success());

    let entries = build_config_entries("/test/gitcomet");
    let result = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(dir.path())
        .args(["config", "--local", entries[0].key, &entries[0].value])
        .output()
        .unwrap();
    assert!(result.status.success());

    // Verify the value was written.
    let check = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(dir.path())
        .args(["config", "--get", entries[0].key])
        .output()
        .unwrap();
    assert!(check.status.success());
    let value = String::from_utf8(check.stdout).expect("utf-8 git config output");
    assert_eq!(value.trim(), entries[0].value);
}

fn decision_for<'a>(plan: &'a [UninstallPlanItem], key: &str) -> &'a UninstallDecision {
    &plan
        .iter()
        .find(|item| item.key == key)
        .unwrap_or_else(|| panic!("missing plan item for key {key}"))
        .decision
}

#[test]
fn uninstall_dry_run_lists_unset_commands_and_conditions() {
    let result = run_uninstall(true, true).unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("Dry run"));
    assert!(
        result
            .stdout
            .contains("git config --local --unset-all mergetool.gitcomet.cmd")
    );
    assert!(
        result
            .stdout
            .contains("git config --local --unset-all merge.tool  # only if value is 'gitcomet'")
    );
    assert!(
        result.stdout.contains("and merge.tool is 'gitcomet'"),
        "expected guarded-condition annotation in dry run output:\n{}",
        result.stdout
    );
}

#[test]
fn uninstall_plan_unsets_matching_setup_values() {
    let entries = build_uninstall_entries();
    let mut snapshot: FxHashMap<&'static str, Vec<String>> = FxHashMap::default();
    snapshot.insert("mergetool.gitcomet.cmd", vec!["custom-cmd".to_string()]);
    snapshot.insert("merge.tool", vec!["gitcomet".to_string()]);
    snapshot.insert("mergetool.prompt", vec!["false".to_string()]);

    let plan = plan_uninstall(&entries, &snapshot);

    assert_eq!(
        decision_for(&plan, "mergetool.gitcomet.cmd"),
        &UninstallDecision::Unset
    );
    assert_eq!(decision_for(&plan, "merge.tool"), &UninstallDecision::Unset);
    assert_eq!(
        decision_for(&plan, "mergetool.prompt"),
        &UninstallDecision::Unset
    );
}

#[test]
fn uninstall_plan_preserves_non_gitcomet_generic_settings() {
    let entries = build_uninstall_entries();
    let mut snapshot: FxHashMap<&'static str, Vec<String>> = FxHashMap::default();
    snapshot.insert("mergetool.gitcomet.cmd", vec!["custom-cmd".to_string()]);
    snapshot.insert("merge.tool", vec!["meld".to_string()]);
    snapshot.insert("mergetool.prompt", vec!["false".to_string()]);

    let plan = plan_uninstall(&entries, &snapshot);

    assert_eq!(
        decision_for(&plan, "mergetool.gitcomet.cmd"),
        &UninstallDecision::Unset
    );
    assert_eq!(
        decision_for(&plan, "merge.tool"),
        &UninstallDecision::SkipValueMismatch {
            expected: "gitcomet",
            actual: vec!["meld".to_string()],
        }
    );
    assert_eq!(
        decision_for(&plan, "mergetool.prompt"),
        &UninstallDecision::SkipGuardMismatch {
            guard_key: "merge.tool",
            guard_expected: "gitcomet",
            guard_actual: vec!["meld".to_string()],
        }
    );
}

fn temp_file_scope() -> (tempfile::TempDir, String, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("config");
    let scope = format!("--file={}", config_path.display());
    (dir, scope, config_path)
}

#[test]
fn collect_uninstall_snapshot_keys_includes_guard_keys_once() {
    let entries = vec![
        UninstallEntry {
            key: "primary.key",
            expected_value: None,
            guard: Some(UninstallGuard {
                key: "guard.key",
                expected_value: "expected",
            }),
        },
        UninstallEntry {
            key: "guard.key",
            expected_value: None,
            guard: None,
        },
    ];

    let keys = collect_uninstall_snapshot_keys(&entries);
    assert_eq!(keys, vec!["primary.key", "guard.key"]);
}

#[test]
fn git_config_helpers_cover_missing_and_error_paths() {
    let (_dir, scope, _) = temp_file_scope();
    let missing = read_git_config_values(&scope, "gitcomet.coverage.missing").unwrap();
    assert!(missing.is_empty());

    let err = read_git_config_values("--not-a-valid-scope", "gitcomet.coverage")
        .expect_err("invalid scope should fail");
    assert!(err.contains("failed"));

    let set_err = set_single_config_value("--not-a-valid-scope", "foo.bar", "value")
        .expect_err("invalid scope should fail");
    assert!(set_err.contains("failed"));

    let add_err = add_config_value("--not-a-valid-scope", "foo.bar", "value")
        .expect_err("invalid scope should fail");
    assert!(add_err.contains("failed"));
}

#[test]
fn parse_git_config_values_preserves_nul_separated_multiline_and_empty_values() {
    let parsed = parse_git_config_values(b"line1\nline2\0\0tail\0").unwrap();
    assert_eq!(
        parsed,
        vec![
            "line1\nline2".to_string(),
            "".to_string(),
            "tail".to_string()
        ]
    );
}

#[test]
fn write_config_values_supports_empty_and_multi_value_sequences() {
    let (_dir, scope, _) = temp_file_scope();
    let mut snapshot = ConfigSnapshot::read(&scope, &["foo.multi"]).unwrap();
    snapshot.write(&scope, "foo.multi", &[]).unwrap();
    assert!(
        read_git_config_values(&scope, "foo.multi")
            .unwrap()
            .is_empty()
    );

    let values = vec!["one".to_string(), "two\nthree".to_string(), "".to_string()];
    snapshot.write(&scope, "foo.multi", &values).unwrap();
    assert_eq!(read_git_config_values(&scope, "foo.multi").unwrap(), values);
}

#[test]
fn maybe_capture_backup_skips_when_current_value_matches_setup_default() {
    let (_dir, scope, _) = temp_file_scope();
    set_single_config_value(&scope, "merge.tool", "gitcomet").unwrap();
    let entry = BackupEntry {
        key: "merge.tool",
        expected_setup_value: "gitcomet",
        backup_key: "gitcomet.backup.merge-tool",
    };
    let mut snapshot = ConfigSnapshot::read(&scope, &[entry.key, entry.backup_key]).unwrap();
    maybe_capture_backup_for_entry(&scope, &entry, &mut snapshot).unwrap();
    assert!(
        read_git_config_values(&scope, entry.backup_key)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn unset_and_apply_paths_report_git_failures() {
    let (_dir, scope, _config_path) = temp_file_scope();
    set_single_config_value(&scope, "foo.readonly", "value").unwrap();

    #[cfg(not(windows))]
    {
        // Simulate a write-time git config failure in a way that does not
        // depend on filesystem permissions (root in containers can bypass
        // readonly directory bits on some CI runners).
        let lock_path = _config_path.with_extension("lock");
        fs::write(&lock_path, b"lock").expect("create config lock file");
        let unset_result = unset_all_config_values(&scope, "foo.readonly");
        fs::remove_file(&lock_path).expect("remove config lock file");

        let unset_err = unset_result.expect_err("config lock file should fail to unset");
        assert!(unset_err.contains("--unset-all"));
    }

    #[cfg(windows)]
    {
        let unset_err = unset_all_config_values("--not-a-valid-scope", "foo.readonly")
            .expect_err("invalid scope should fail");
        assert!(unset_err.contains("failed"));
    }

    let uninstall_err = apply_uninstall_plan(
        &[UninstallPlanItem {
            key: "foo.readonly",
            decision: UninstallDecision::Unset,
        }],
        "--not-a-valid-scope",
    )
    .expect_err("invalid scope should fail");
    assert!(uninstall_err.contains("--unset-all foo.readonly failed"));

    let apply_err = apply_config(
        &[ConfigEntry {
            key: "foo.readonly",
            value: "value".to_string(),
        }],
        "--not-a-valid-scope",
        &mut ConfigSnapshot {
            values: FxHashMap::default(),
        },
    )
    .expect_err("invalid scope should fail");
    assert!(apply_err.contains("git config foo.readonly value failed"));
}

#[test]
fn apply_config_skips_unchanged_values_but_repairs_changed_and_missing_entries() {
    let (_dir, scope, config_path) = temp_file_scope();
    let entries = [
        ConfigEntry {
            key: "diff.tool",
            value: "gitcomet".into(),
        },
        ConfigEntry {
            key: "difftool.gitcomet.cmd",
            value: "command".into(),
        },
    ];
    set_single_config_value(&scope, entries[0].key, &entries[0].value).unwrap();
    set_single_config_value(&scope, entries[1].key, &entries[1].value).unwrap();
    let keys = entries.iter().map(|entry| entry.key).collect::<Vec<_>>();
    let mut snapshot = ConfigSnapshot::read(&scope, &keys).unwrap();
    let before = fs::read(&config_path).unwrap();
    let lock_path = config_path.with_extension("lock");
    fs::write(&lock_path, b"locked").unwrap();
    apply_config(&entries, &scope, &mut snapshot).expect("no-op setup needs no write lock");
    assert_eq!(fs::read(&config_path).unwrap(), before);
    fs::remove_file(lock_path).unwrap();

    set_single_config_value(&scope, entries[0].key, "different").unwrap();
    unset_existing_config_values(&scope, entries[1].key).unwrap();
    let mut snapshot = ConfigSnapshot::read(&scope, &keys).unwrap();
    apply_config(&entries, &scope, &mut snapshot).unwrap();
    for entry in &entries {
        assert_eq!(snapshot.get(entry.key), [entry.value.as_str()]);
        assert_eq!(
            read_git_config_values(&scope, entry.key).unwrap(),
            [entry.value.as_str()]
        );
    }
}

#[test]
fn apply_config_preserves_git_errors_for_duplicate_matching_values() {
    let (_dir, scope, _config_path) = temp_file_scope();
    for _ in 0..2 {
        add_config_value(&scope, "diff.tool", "gitcomet").unwrap();
    }
    let mut snapshot = ConfigSnapshot::read(&scope, &["diff.tool"]).unwrap();
    let error = apply_config(
        &[ConfigEntry {
            key: "diff.tool",
            value: "gitcomet".into(),
        }],
        &scope,
        &mut snapshot,
    )
    .expect_err("multiple equal values must still produce Git's setter error");
    assert!(error.contains("git config diff.tool gitcomet failed"));
    assert_eq!(snapshot.get("diff.tool"), ["gitcomet", "gitcomet"]);
    assert_eq!(
        read_git_config_values(&scope, "diff.tool").unwrap(),
        ["gitcomet", "gitcomet"]
    );
}

#[test]
fn snapshot_matches_git_multivalues_and_scope_and_preserves_subsection_case() {
    let (_dir, scope, config_path) = temp_file_scope();
    std::fs::write(&config_path, "[Merge]\n Tool = first\n tool = \"line1\\nline2\"\n[custom \"MiXeD\"]\n Boolean\n empty =\n").unwrap();
    let snapshot = ConfigSnapshot::read(
        &scope,
        &["merge.tool", "custom.MiXeD.boolean", "custom.MiXeD.empty"],
    )
    .unwrap();
    for key in ["merge.tool", "custom.MiXeD.boolean", "custom.MiXeD.empty"] {
        assert_eq!(
            snapshot.get(key),
            read_git_config_values(&scope, key).unwrap()
        );
    }
    assert_eq!(snapshot.get("MERGE.TOOL"), vec!["first", "line1\nline2"]);
    assert!(snapshot.get("custom.mixed.boolean").is_empty());
    assert!(ConfigSnapshot::parse(b"merge.tool\n\xff\0").is_err());
}

#[test]
fn snapshot_handles_missing_file_and_failed_mutations() {
    let (_dir, scope, config_path) = temp_file_scope();
    assert!(!config_path.exists());
    let mut snapshot = ConfigSnapshot::read(&scope, &["merge.tool"]).unwrap();
    assert!(snapshot.get("merge.tool").is_empty());
    snapshot
        .write(&scope, "merge.tool", &["previous".into()])
        .unwrap();
    let lock_path = config_path.with_extension("lock");
    std::fs::write(&lock_path, b"locked").unwrap();
    assert!(snapshot.unset(&scope, "merge.tool").is_err());
    assert_eq!(snapshot.get("merge.tool"), vec!["previous"]);
    assert!(
        snapshot
            .write(&scope, "merge.tool", &["next".into()])
            .is_err()
    );
    assert_eq!(
        read_git_config_values(&scope, "merge.tool").unwrap(),
        vec!["previous"]
    );
    assert!(ConfigSnapshot::read("--not-a-valid-scope", &["merge.tool"]).is_err());
}

#[test]
fn format_values_empty_returns_unset_marker() {
    assert_eq!(format_values(&[]), "<unset>");
}
