use super::*;
use std::fs;
use std::path::Path;

fn locale_env(
    inherited: &[(&str, &str)],
    utf8: Option<&'static str>,
) -> Vec<(String, Option<String>)> {
    let inherited: Vec<(OsString, OsString)> = inherited
        .iter()
        .map(|(name, value)| ((*name).into(), (*value).into()))
        .collect();
    git_locale_env(&inherited, || utf8)
        .into_iter()
        .map(|(name, value)| {
            (
                name.into_string().unwrap(),
                value.map(|value| value.into_string().unwrap()),
            )
        })
        .collect()
}

fn set(name: &str, value: &str) -> (String, Option<String>) {
    (name.to_string(), Some(value.to_string()))
}

#[test]
fn git_speaks_english_and_keeps_a_utf8_locale_the_user_chose() {
    for user in [
        &[("LC_ALL", "fi_FI.UTF-8")][..],
        &[("LANG", "de_DE.utf8"), ("LC_NUMERIC", "fi_FI.UTF-8")],
        &[("LC_CTYPE", "UTF-8")],
    ] {
        assert_eq!(
            locale_env(user, Some("C.UTF-8")),
            [set("LANGUAGE", "en")],
            "{user:?}"
        );
    }
}

#[test]
fn a_locale_without_utf8_gets_a_utf8_charset_and_keeps_everything_else() {
    // A macOS app opened from Finder inherits no locale at all.
    assert_eq!(
        locale_env(&[], Some("UTF-8")),
        [set("LANGUAGE", "en"), set("LC_CTYPE", "UTF-8")]
    );
    assert_eq!(
        locale_env(
            &[("LANG", "fi_FI.UTF-8"), ("LC_CTYPE", "C")],
            Some("C.UTF-8")
        ),
        [set("LANGUAGE", "en"), set("LC_CTYPE", "C.UTF-8")],
        "LC_CTYPE outranks LANG"
    );
    assert_eq!(
        locale_env(
            &[("LC_ALL", "fi_FI.iso88591"), ("LC_TIME", "de_DE.UTF-8")],
            Some("C.UTF-8")
        ),
        [
            set("LANGUAGE", "en"),
            ("LC_ALL".to_string(), None),
            ("LC_TIME".to_string(), None),
            set("LANG", "fi_FI.iso88591"),
            set("LC_CTYPE", "C.UTF-8"),
        ],
        "LC_ALL hid LC_TIME, so moving LC_ALL into LANG must not expose it"
    );
    assert_eq!(
        locale_env(&[("LANG", "C")], None),
        [set("LANGUAGE", "en")],
        "a UTF-8 locale this machine lacks would only add warnings"
    );
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

struct GitRuntimePreferenceResetGuard {
    original: GitExecutablePreference,
}

impl GitRuntimePreferenceResetGuard {
    fn install(preference: GitExecutablePreference) -> Self {
        let original = current_git_executable_preference();
        let _ = install_git_executable_preference(preference);
        Self { original }
    }
}

impl Drop for GitRuntimePreferenceResetGuard {
    fn drop(&mut self) {
        let _ = install_git_executable_preference(self.original.clone());
    }
}

fn git_runtime_probe_count(probe_log: &Path) -> usize {
    fs::read_to_string(probe_log)
        .unwrap_or_else(|err| {
            panic!(
                "read probe log {}: {err}; runtime: {:?}",
                probe_log.display(),
                current_git_runtime()
            )
        })
        .lines()
        .count()
}

#[test]
fn selecting_and_reading_runtime_never_spawns_and_concurrent_requests_coalesce() {
    let _lock = lock_git_runtime_test();
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("probes");
    let (_script_dir, script) = create_git_probe_script("git version test", "", 0, Some(&log));
    select_git_executable_path(Some(script));
    for _ in 0..100 {
        assert_eq!(
            GitRuntimeState::default().availability,
            GitExecutableAvailability::Checking
        );
        let _ = git_command();
    }
    assert!(!log.exists());
    let request = begin_git_runtime_probe(false).unwrap();
    assert!(begin_git_runtime_probe(true).is_none());
    assert!(!log.exists());
    assert!(request.run().unwrap().is_available());
    for _ in 0..100 {
        assert!(begin_git_runtime_probe(false).is_none());
    }
    assert_eq!(git_runtime_probe_count(&log), 1);
}

#[test]
fn waiters_adopt_a_probe_started_elsewhere_without_probing_again() {
    let _lock = lock_git_runtime_test();
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("probes");
    let (_script_dir, script) = create_git_probe_script("git version early", "", 0, Some(&log));
    select_git_executable_path(Some(script));
    assert!(wait_for_git_runtime_probe(Duration::ZERO).is_none());

    let request = begin_git_runtime_probe(false).unwrap();
    let waiters: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(|| wait_for_git_runtime_probe(Duration::from_secs(30))))
        .collect();
    // The waiters block on the claim until the probe settles.
    std::thread::sleep(Duration::from_millis(20));
    assert!(waiters.iter().all(|waiter| !waiter.is_finished()));
    assert!(request.run().unwrap().is_available());
    for waiter in waiters {
        let adopted = waiter.join().unwrap().expect("settled runtime");
        assert_eq!(adopted.version_output(), Some("git version early"));
    }
    // Settled: adopting returns at once, still without a second probe.
    assert!(wait_for_git_runtime_probe(Duration::ZERO).is_some_and(|r| r.is_available()));
    assert_eq!(git_runtime_probe_count(&log), 1);
}

#[test]
fn dropping_an_unrun_probe_releases_its_claim_without_rate_limiting() {
    let _lock = lock_git_runtime_test();
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("probes");
    let (_script_dir, script) = create_git_probe_script("git version late", "", 0, Some(&log));
    select_git_executable_path(Some(script));
    // A worker that never started (a failed thread spawn, a cancelled task).
    drop(begin_git_runtime_probe(false).unwrap());
    assert!(!log.exists());
    let retry = begin_git_runtime_probe(false).expect("nothing was learned, so retry at once");
    assert!(retry.run().unwrap().is_available());
    assert_eq!(git_runtime_probe_count(&log), 1);
    assert!(begin_git_runtime_probe(false).is_none());
}

#[test]
fn waiters_give_up_on_a_probe_abandoned_by_a_new_preference() {
    let _lock = lock_git_runtime_test();
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());
    let (_a, a) = create_git_probe_script("git version old", "", 0, None);
    let (_b, b) = create_git_probe_script("git version new", "", 0, None);
    select_git_executable_path(Some(a));
    let old = begin_git_runtime_probe(false).unwrap();
    let waiter = std::thread::spawn(|| wait_for_git_runtime_probe(Duration::from_secs(30)));
    std::thread::sleep(Duration::from_millis(20));
    select_git_executable_path(Some(b));
    // Nothing is known about the new executable yet.
    assert!(waiter.join().unwrap().is_none());
    assert!(old.run().is_none());
}

#[test]
fn superseded_probe_cannot_overwrite_new_preference_or_release_its_claim() {
    let _lock = lock_git_runtime_test();
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());
    let (_a, a) = create_git_probe_script("git version old", "", 0, None);
    let (_b, b) = create_git_probe_script("git version new", "", 0, None);
    select_git_executable_path(Some(a));
    let old = begin_git_runtime_probe(false).unwrap();
    select_git_executable_path(Some(b.clone()));
    let new = begin_git_runtime_probe(false).unwrap();
    assert!(old.run().is_none());
    assert!(begin_git_runtime_probe(true).is_none());
    assert_eq!(
        current_git_executable_preference().custom_path(),
        Some(b.as_path())
    );
    assert_eq!(new.run().unwrap().version_output(), Some("git version new"));
}

#[test]
fn unavailable_focus_recovery_is_throttled_but_diagnostics_can_recheck() {
    let _lock = lock_git_runtime_test();
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());
    select_git_executable_path(Some(PathBuf::new()));
    assert!(
        !begin_git_runtime_probe(false)
            .unwrap()
            .run()
            .unwrap()
            .is_available()
    );
    assert!(begin_git_runtime_probe(false).is_none());
    assert!(begin_git_runtime_probe(true).is_some());
}

fn create_git_probe_script(
    stdout: &str,
    stderr: &str,
    exit_code: i32,
    probe_log: Option<&Path>,
) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("create temp dir");
    #[cfg(unix)]
    let script = dir.path().join("git");
    #[cfg(windows)]
    let script = dir.path().join("git.cmd");

    write_git_probe_script(&script, stdout, stderr, exit_code, probe_log);
    (dir, script)
}

#[cfg(unix)]
fn write_executable_script(script_path: &Path, script: &str) {
    // A concurrent test can fork while fs::write holds the script open,
    // inheriting the writable fd until exec even though it is CLOEXEC.
    // Probing the script in that window fails with ETXTBSY. Write in a
    // child instead so the test runner never owns that writable fd, and
    // wait for the writer to exit before making the script executable.
    // https://github.com/rust-lang/rust/issues/114554
    let output = Command::new("/bin/sh")
        .args([
            "-c",
            "umask 077; printf '%s' \"$2\" > \"$1\"",
            "write-git-fixture",
        ])
        .arg(script_path)
        .arg(script)
        .output()
        .expect("run git fixture writer");
    assert!(
        output.status.success(),
        "write git fixture {} failed ({}): {}",
        script_path.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    fs::set_permissions(script_path, fs::Permissions::from_mode(0o700))
        .expect("set git fixture permissions");
}

#[cfg(unix)]
fn write_git_probe_script(
    script_path: &Path,
    stdout: &str,
    stderr: &str,
    exit_code: i32,
    probe_log: Option<&Path>,
) {
    let mut script = String::from("#!/bin/sh\n");
    if let Some(probe_log) = probe_log {
        script.push_str(&format!("printf 'probe\\n' >> '{}'\n", probe_log.display()));
    }
    if !stdout.is_empty() {
        script.push_str(&format!("printf '%s\\n' '{stdout}'\n"));
    }
    if !stderr.is_empty() {
        script.push_str(&format!("printf '%s\\n' '{stderr}' >&2\n"));
    }
    script.push_str(&format!("exit {exit_code}\n"));

    write_executable_script(script_path, &script);
}

#[cfg(windows)]
fn write_git_probe_script(
    script_path: &Path,
    stdout: &str,
    stderr: &str,
    exit_code: i32,
    probe_log: Option<&Path>,
) {
    let mut script = String::from("@echo off\r\n");
    if let Some(probe_log) = probe_log {
        script.push_str(&format!(">>\"{}\" echo probe\r\n", probe_log.display()));
    }
    if !stdout.is_empty() {
        script.push_str(&format!("echo {stdout}\r\n"));
    }
    if !stderr.is_empty() {
        script.push_str(&format!("1>&2 echo {stderr}\r\n"));
    }
    script.push_str(&format!("exit /b {exit_code}\r\n"));
    fs::write(script_path, script).expect("write git probe script");
}

#[test]
fn normalize_git_executable_path_makes_relative_paths_absolute() {
    let path = normalize_git_executable_path(PathBuf::from("test-git"));
    assert!(
        path.is_absolute(),
        "expected absolute path, got {}",
        path.display()
    );
}

#[cfg(unix)]
#[test]
fn normalize_git_executable_path_preserves_absolute_symlink() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let target = dir.path().join("store-git");
    let link = dir.path().join("profile-git");
    fs::write(&target, b"git").expect("write target");
    std::os::unix::fs::symlink(&target, &link).expect("create symlink");

    let normalized = normalize_git_executable_path(link.clone());

    assert_eq!(normalized, link);
    assert_ne!(normalized, target);
}

#[test]
fn git_executable_preference_from_optional_path_covers_all_variants() {
    let relative = PathBuf::from("test-git");

    assert_eq!(
        GitExecutablePreference::from_optional_path(None),
        GitExecutablePreference::SystemPath
    );
    assert_eq!(
        GitExecutablePreference::from_optional_path(Some(PathBuf::new())),
        GitExecutablePreference::Custom(PathBuf::new())
    );
    assert_eq!(
        GitExecutablePreference::from_optional_path(Some(relative.clone())),
        GitExecutablePreference::Custom(normalize_git_executable_path(relative))
    );
}

#[test]
fn git_executable_preference_custom_path_and_display_label_cover_variants() {
    let custom = GitExecutablePreference::Custom(PathBuf::from("/opt/git/bin/git"));
    let empty_custom = GitExecutablePreference::Custom(PathBuf::new());

    assert_eq!(GitExecutablePreference::SystemPath.custom_path(), None);
    assert_eq!(custom.custom_path(), Some(Path::new("/opt/git/bin/git")));
    assert_eq!(empty_custom.custom_path(), Some(Path::new("")));

    assert_eq!(
        GitExecutablePreference::SystemPath.display_label(),
        "System PATH"
    );
    assert_eq!(
        empty_custom.display_label(),
        "Custom executable (not selected)"
    );
    assert_eq!(custom.display_label(), "/opt/git/bin/git");
}

#[test]
fn install_git_executable_preference_reports_missing_custom_path() {
    let _lock = lock_git_runtime_test();
    let missing = std::env::temp_dir().join("gitcomet-missing-git-executable");
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

    let state = install_git_executable_preference(GitExecutablePreference::Custom(missing.clone()));

    assert!(!state.is_available());
    assert_eq!(
        state.preference,
        GitExecutablePreference::Custom(missing.clone())
    );
    assert!(
        state
            .unavailable_detail()
            .expect("expected unavailable detail")
            .contains(&missing.display().to_string())
    );
}

#[test]
fn install_git_executable_preference_uses_custom_executable_stdout() {
    let _lock = lock_git_runtime_test();
    let (_dir, script) = create_git_probe_script("git version 9.9.9-test", "", 0, None);
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

    let state = install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));
    assert!(state.is_available());
    assert_eq!(state.version_output(), Some("git version 9.9.9-test"));
}

#[test]
fn install_git_executable_preference_uses_stderr_when_stdout_is_empty() {
    let _lock = lock_git_runtime_test();
    let (_dir, script) = create_git_probe_script("", "git version 8.8.8-test", 0, None);
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

    let state = install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));

    assert!(state.is_available());
    assert_eq!(state.version_output(), Some("git version 8.8.8-test"));
}

#[test]
fn install_git_executable_preference_reports_empty_version_output() {
    let _lock = lock_git_runtime_test();
    let (_dir, script) = create_git_probe_script("", "", 0, None);
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

    let state = install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));

    assert!(!state.is_available());
    assert_eq!(
        state.unavailable_detail(),
        Some(
            &format!(
                "Git executable at {} returned no version text.",
                script.display()
            )[..]
        )
    );
}

#[test]
fn install_git_executable_preference_reports_process_failure_with_stderr() {
    let _lock = lock_git_runtime_test();
    let (_dir, script) = create_git_probe_script("", "fatal: not a git executable", 7, None);
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

    let state = install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));

    assert!(!state.is_available());
    assert_eq!(
        state.unavailable_detail(),
        Some(
            &format!(
                "Git executable at {} failed: fatal: not a git executable",
                script.display()
            )[..]
        )
    );
}

#[test]
fn install_git_executable_preference_reports_process_failure_without_stderr() {
    let _lock = lock_git_runtime_test();
    let (_dir, script) = create_git_probe_script("", "", 7, None);
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

    let state = install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));

    assert!(!state.is_available());
    let detail = state
        .unavailable_detail()
        .expect("expected unavailable detail");
    assert!(detail.contains(&script.display().to_string()));
    assert!(detail.contains("exited with"));
}

#[test]
fn install_git_executable_preference_reports_missing_custom_selection() {
    let _lock = lock_git_runtime_test();
    let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

    let state = install_git_executable_preference(GitExecutablePreference::Custom(PathBuf::new()));

    assert!(!state.is_available());
    assert_eq!(
        state.preference,
        GitExecutablePreference::Custom(PathBuf::new())
    );
    assert_eq!(
        state.unavailable_detail(),
        Some(
            "Custom Git executable is not configured. Choose an executable or switch back to System PATH."
        )
    );
}

#[test]
fn git_command_uses_installed_preference() {
    let _lock = lock_git_runtime_test();
    let temp = tempfile::tempdir().expect("create temp dir");
    let probe_log = temp.path().join("git-probes.log");
    let (_dir, script) = create_git_probe_script("git version 7.7.7-test", "", 0, Some(&probe_log));
    let _restore = GitRuntimePreferenceResetGuard::install(GitExecutablePreference::Custom(script));

    assert_eq!(
        git_runtime_probe_count(&probe_log),
        1,
        "installing a custom executable should probe exactly once"
    );

    let output = git_command()
        .arg("--version")
        .output()
        .expect("run configured git command");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "git version 7.7.7-test"
    );
    assert_eq!(
        git_runtime_probe_count(&probe_log),
        2,
        "running the returned command should invoke the configured executable"
    );
}

#[test]
fn git_command_prepends_custom_git_parent_to_path() {
    let _lock = lock_git_runtime_test();
    let (_dir, script) = create_git_probe_script("git version 5.5.5-test", "", 0, None);
    let _restore =
        GitRuntimePreferenceResetGuard::install(GitExecutablePreference::Custom(script.clone()));

    let command = git_command();
    let path_env = command
        .get_envs()
        .find(|(name, _)| *name == OsStr::new("PATH"))
        .and_then(|(_, value)| value.map(ToOwned::to_owned))
        .expect("custom git command should set PATH");
    let paths: Vec<_> = std::env::split_paths(&path_env).collect();

    assert_eq!(
        paths.first().map(PathBuf::as_path),
        script.parent(),
        "custom git parent should be first in PATH"
    );
}

#[cfg(unix)]
#[test]
fn custom_git_can_resolve_sibling_helper_from_path() {
    let _lock = lock_git_runtime_test();
    let dir = tempfile::tempdir().expect("create temp dir");
    let git = dir.path().join("git");
    let gpg = dir.path().join("gpg");

    write_executable_script(&git, "#!/bin/sh\ngpg --version\n");
    write_executable_script(&gpg, "#!/bin/sh\nprintf 'sibling gpg 1.0\\n'\n");

    let _restore = GitRuntimePreferenceResetGuard::install(GitExecutablePreference::Custom(git));

    assert_eq!(
        current_git_runtime().version_output(),
        Some("sibling gpg 1.0")
    );
}

#[test]
fn refresh_git_runtime_reprobes_current_preference() {
    let _lock = lock_git_runtime_test();
    let temp = tempfile::tempdir().expect("create temp dir");
    let probe_log = temp.path().join("git-probes.log");
    let (_dir, script) = create_git_probe_script("git version 6.6.6-test", "", 0, Some(&probe_log));
    let _restore = GitRuntimePreferenceResetGuard::install(GitExecutablePreference::Custom(script));

    assert_eq!(git_runtime_probe_count(&probe_log), 1);
    let refreshed = refresh_git_runtime();

    assert!(refreshed.is_available());
    assert_eq!(refreshed.version_output(), Some("git version 6.6.6-test"));
    assert_eq!(
        git_runtime_probe_count(&probe_log),
        2,
        "refresh should re-run the current runtime probe"
    );
}

#[test]
fn bytes_to_text_preserving_utf8_preserves_valid_utf8() {
    assert_eq!(
        bytes_to_text_preserving_utf8("cafe 日本語".as_bytes()),
        "cafe 日本語"
    );
}

#[test]
fn bytes_to_text_preserving_utf8_escapes_invalid_bytes_without_losing_valid_segments() {
    let input = b"ok\xffstill-valid\xc3\xa9\x80done";
    assert_eq!(
        bytes_to_text_preserving_utf8(input),
        "ok\\xffstill-validé\\x80done"
    );
}

#[test]
fn bytes_to_text_preserving_utf8_escapes_truncated_multibyte_tail() {
    assert_eq!(
        bytes_to_text_preserving_utf8(b"snowman \xe2\x98"),
        "snowman \\xe2\\x98"
    );
}

#[test]
fn bytes_to_text_preserving_utf8_handles_empty_input() {
    assert_eq!(bytes_to_text_preserving_utf8(b""), "");
}

#[test]
fn git_version_parses_the_first_version_token() {
    assert_eq!(
        GitVersion::parse("git version 2.53.1"),
        Some(GitVersion {
            major: 2,
            minor: 53
        })
    );
    assert_eq!(
        GitVersion::parse("git version 2.45.1.windows.1"),
        Some(GitVersion {
            major: 2,
            minor: 45
        })
    );
    assert_eq!(GitVersion::parse("git version v2.45.1"), None);
    assert_eq!(
        GitVersion::parse_token("2.53rc1"),
        GitVersion::parse("2.53")
    );
    assert_eq!(parse_u32_prefix("rc53"), None);
}

#[test]
fn supported_git_version_starts_at_2_53() {
    let version = |major, minor| GitVersion { major, minor };
    assert!(version(2, 53).is_supported());
    assert!(version(2, 54).is_supported());
    assert!(version(3, 0).is_supported());
    assert!(!version(2, 52).is_supported());
    assert!(!version(1, 99).is_supported());
    assert_eq!(GitVersion::MINIMUM.to_string(), "2.53");
}
