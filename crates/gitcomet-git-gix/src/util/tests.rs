use super::*;

#[test]
fn fnv_preserves_existing_cache_hashes() {
    for (bytes, expected) in [
        (&b""[..], 0xcbf2_9ce4_8422_2325),
        (&b"hello"[..], 0xa430_d846_80aa_bd0b),
        (&b"/tmp/repo\0\xff"[..], 0xa2a3_b27f_0667_4725),
    ] {
        assert_eq!(fnv1a_64(bytes), expected);
    }
}

#[test]
fn activity_progress_deadline_survives_continuous_small_chunks() {
    let start = Instant::now();
    let mut buffer = ActivityOutputBuffer::default();
    for ms in [0, 20, 40, 60, 80] {
        buffer.push(
            GitOutputStream::Stderr,
            "progress\r".into(),
            start + Duration::from_millis(ms),
        );
        assert!(!buffer.ready(start + Duration::from_millis(ms)));
    }
    assert_eq!(
        buffer.wait(start + Duration::from_millis(90)),
        Duration::from_millis(10)
    );
    buffer.push(
        GitOutputStream::Stderr,
        "still transferring\r".into(),
        start + GIT_ACTIVITY_OUTPUT_FLUSH,
    );
    assert!(buffer.ready(start + GIT_ACTIVITY_OUTPUT_FLUSH));
    let chunks = buffer.take();
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].text.ends_with("still transferring\r"));
    assert!(!buffer.ready(start + Duration::from_secs(1)));
    assert_eq!(
        buffer.wait(start + Duration::from_secs(1)),
        GIT_ACTIVITY_OUTPUT_FLUSH
    );
}

#[test]
fn activity_progress_flushes_large_output_and_preserves_stream_order() {
    let start = Instant::now();
    let mut buffer = ActivityOutputBuffer::default();
    buffer.push(GitOutputStream::Stdout, "out".into(), start);
    buffer.push(GitOutputStream::Stderr, "err".into(), start);
    buffer.push(
        GitOutputStream::Stdout,
        "x".repeat(GIT_ACTIVITY_OUTPUT_BATCH_BYTES - 6),
        start,
    );
    assert!(buffer.ready(start));
    let chunks = buffer.take();
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].stream, GitOutputStream::Stdout);
    assert_eq!(chunks[1].stream, GitOutputStream::Stderr);
    assert_eq!(chunks[2].stream, GitOutputStream::Stdout);
}

#[test]
fn activity_output_drains_final_partial_batch_on_disconnect() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&events);
    let context = GitOperationContext::new("output", move |_, event| {
        if let GitOperationEvent::Output { chunks } = event {
            captured.lock().unwrap().extend(chunks);
        }
    });
    let (sender, handle) = start_activity_output_aggregator(Some(&context));
    sender
        .as_ref()
        .unwrap()
        .send((GitOutputStream::Stderr, "final λ\r".into()))
        .unwrap();
    drop(sender);
    join_activity_output_aggregator(handle);
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].text, "final λ\r");
}
use gitcomet_core::auth::askpass::{
    GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG_ENV, GITCOMET_ASKPASS_PROMPT_LOG_ENV, PromptAuth,
};
use gitcomet_core::auth::{
    CachedPassphraseEntry, GITCOMET_AUTH_CACHE_SIZE_ENV, GITCOMET_AUTH_KIND_ENV,
    GITCOMET_AUTH_KIND_HOST_VERIFICATION, GITCOMET_AUTH_KIND_PASSPHRASE,
    GITCOMET_AUTH_KIND_PASSPHRASE_CACHED, GITCOMET_AUTH_KIND_USERNAME_PASSWORD,
    GITCOMET_AUTH_SECRET_ENV, GITCOMET_AUTH_USERNAME_ENV, GitAuthKind, StagedGitAuth,
};
use std::process::Command;
use std::sync::Mutex;

const GITPY_FOR_EACH_REF_WITH_PATH_COMPONENT: &[u8] =
    include_bytes!("../../tests/fixtures/gitpython/for_each_ref_with_path_component");
const GITPY_UNCOMMON_BRANCH_PREFIX_FETCH_HEAD: &str =
    include_str!("../../tests/fixtures/gitpython/uncommon_branch_prefix_FETCH_HEAD");
const GITPY_REV_LIST_SINGLE: &str = include_str!("../../tests/fixtures/gitpython/rev_list_single");
const GITPY_REV_LIST_COMMIT_STATS: &str =
    include_str!("../../tests/fixtures/gitpython/rev_list_commit_stats");

#[cfg(unix)]
#[test]
fn path_buf_from_git_bytes_preserves_non_utf8_bytes() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let raw_path = b"docs/\xff-topic.md";
    let path = path_buf_from_git_bytes(raw_path, "test").expect("path conversion");
    assert_eq!(path.as_os_str(), OsStr::from_bytes(raw_path));
}

#[cfg(unix)]
#[test]
fn git_stage_blob_spec_preserves_non_utf8_path_bytes() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let path = Path::new(OsStr::from_bytes(b"nested/\xff-file.bin"));
    let rev = git_stage_blob_spec(2, path).expect("stage spec");
    assert_eq!(rev.as_os_str().as_bytes(), b":2:nested/\xff-file.bin");
}

#[cfg(windows)]
#[test]
fn git_stage_blob_spec_normalizes_windows_separators() {
    let rev = git_stage_blob_spec(3, Path::new(r"nested\file.bin")).expect("stage spec");
    assert_eq!(
        rev.to_str()
            .expect("ascii revision should be valid unicode"),
        ":3:nested/file.bin"
    );
}

fn gitpython_fetch_head_to_remote_ref_output(fetch_head: &str, remote: &str) -> String {
    let mut out = String::new();
    for line in fetch_head.lines() {
        let Some((sha, rest)) = line.split_once('\t') else {
            continue;
        };
        let sha = sha.trim();
        if sha.is_empty() {
            continue;
        }
        let Some(start) = rest.find("'refs/") else {
            continue;
        };
        let refs_and_after = &rest[start + 1..];
        let Some((full_ref, _)) = refs_and_after.split_once('\'') else {
            continue;
        };
        let short_ref = full_ref.strip_prefix("refs/").unwrap_or(full_ref);
        out.push_str(remote);
        out.push('/');
        out.push_str(short_ref);
        out.push('\t');
        out.push_str(sha);
        out.push('\n');
    }
    out
}

#[cfg(unix)]
fn shell_command(script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(script);
    cmd
}

#[cfg(windows)]
fn shell_command(script: &str) -> Command {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-Command", script]);
    cmd
}

#[cfg(unix)]
fn failing_command_with_streams() -> Command {
    shell_command("printf out; printf err >&2; exit 7")
}

#[cfg(windows)]
fn failing_command_with_streams() -> Command {
    shell_command("[Console]::Out.Write('out'); [Console]::Error.Write('err'); exit 7")
}

#[cfg(unix)]
fn failing_command_with_stdout_only() -> Command {
    shell_command("printf 'stdout only'; exit 9")
}

#[cfg(windows)]
fn failing_command_with_stdout_only() -> Command {
    shell_command("[Console]::Out.Write('stdout only'); exit 9")
}

#[cfg(unix)]
fn failing_command_with_missing_gpg() -> Command {
    shell_command(
        "printf 'error: cannot run gpg: No such file or directory\nerror: gpg failed to sign the data:\nfatal: failed to write commit object\n' >&2; exit 128",
    )
}

#[cfg(windows)]
fn failing_command_with_missing_gpg() -> Command {
    shell_command(
        "[Console]::Error.Write(\"error: cannot run gpg: No such file or directory`nerror: gpg failed to sign the data:`nfatal: failed to write commit object`n\"); exit 128",
    )
}

#[cfg(unix)]
fn failing_command_with_gpg_signing_error() -> Command {
    shell_command(
        "printf 'error: gpg failed to sign the data\nfatal: failed to write commit object\n' >&2; exit 128",
    )
}

#[cfg(windows)]
fn failing_command_with_gpg_signing_error() -> Command {
    shell_command(
        "[Console]::Error.Write(\"error: gpg failed to sign the data`nfatal: failed to write commit object`n\"); exit 128",
    )
}

#[cfg(unix)]
fn failing_command_with_missing_gpg_program_path() -> Command {
    shell_command(
        "printf 'error: cannot run /opt/homebrew/bin/gpg: Datei oder Verzeichnis nicht gefunden\nerror: gpg failed to sign the data:\nfatal: failed to write commit object\n' >&2; exit 128",
    )
}

#[cfg(windows)]
fn failing_command_with_missing_gpg_program_path() -> Command {
    shell_command(
        "[Console]::Error.Write(\"error: cannot run C:/Program Files/Git/usr/bin/gpg.exe: Het systeem kan het opgegeven bestand niet vinden.`nerror: gpg failed to sign the data:`nfatal: failed to write commit object`n\"); exit 128",
    )
}

#[cfg(unix)]
fn sleep_command(seconds: u64) -> Command {
    shell_command(&format!("sleep {seconds}"))
}

#[cfg(windows)]
fn sleep_command(seconds: u64) -> Command {
    shell_command(&format!("Start-Sleep -Seconds {seconds}"))
}

fn run_git_test_setup(workdir: &Path, args: &[&str]) {
    let mut cmd = git_workdir_cmd_for(workdir);
    cmd.args(args);
    let output = cmd.output().expect("test Git command should start");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_test_hook(workdir: &Path, name: &str, script: &str) {
    let hooks = workdir.join(".githooks");
    std::fs::create_dir_all(&hooks).expect("create test hooks directory");
    let path = hooks.join(name);
    std::fs::write(&path, script).expect("write test hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(&path)
            .expect("read test hook metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("make test hook executable");
    }
}

#[test]
fn hook_free_commands_do_not_enable_trace2() {
    let operation = GitOperationContext::new("test", |_, _| {});
    for args in [
        vec![
            "-c",
            "protocol.ext.allow=never",
            "-C",
            "unicode space é",
            "remote",
            "set-url",
            "--",
            "origin",
            "url",
        ],
        vec!["--no-optional-locks", "config", "--get", "core.editor"],
        vec!["config", "get", "user.name"],
    ] {
        let mut cmd = Command::new("git");
        cmd.args(args);
        assert!(Trace2Monitor::start(&mut cmd, Some(&operation), &LivenessClock::new()).is_none());
        assert!(!cmd.get_envs().any(|(key, _)| key == "GIT_TRACE2_EVENT"));
    }
    for args in [
        vec!["commit", "-m", "hook"],
        vec!["status", "--porcelain=v2"],
        vec!["remote", "update"],
        vec!["config", "--edit"],
        vec!["--paginate", "config", "--list"],
        vec!["--unknown", "config", "--list"],
        vec!["custom-alias"],
        vec!["-C"],
    ] {
        let mut cmd = Command::new("git");
        cmd.args(args);
        assert!(!command_is_known_hook_free(&cmd));
        assert!(Trace2Monitor::start(&mut cmd, Some(&operation), &LivenessClock::new()).is_some());
    }
    let mut wrapper = Command::new("custom-git-wrapper");
    wrapper.args(["remote", "set-url", "origin", "url"]);
    assert!(!command_is_known_hook_free(&wrapper));
    let mut custom_git = Command::new("custom/bin/git.exe");
    custom_git.args(["remote", "set-url", "origin", "url"]);
    assert!(!command_is_known_hook_free(&custom_git));
}

#[test]
fn trace2_monitor_finish_and_drop_wake_a_parked_worker() {
    for finish in [false, true] {
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = Arc::clone(&done);
        let (ready_tx, ready_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            while !worker_done.load(Ordering::Acquire) {
                ready_tx.send(()).unwrap();
                // A long poll makes the regression independent of tight
                // elapsed-time assertions on a loaded CI machine.
                thread::park_timeout(Duration::from_secs(30));
            }
        });
        let worker = handle.thread().clone();
        let monitor = Trace2Monitor {
            _path: tempfile::NamedTempFile::new().unwrap().into_temp_path(),
            done,
            handle: Some(handle),
        };
        ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let shutdown = thread::spawn(move || {
            if finish {
                monitor.finish();
            } else {
                drop(monitor);
            }
            stopped_tx.send(()).unwrap();
        });
        let result = stopped_rx.recv_timeout(Duration::from_secs(3));
        // Clean up promptly even if shutdown failed to wake the worker.
        worker.unpark();
        shutdown.join().unwrap();
        result.expect("trace monitor shutdown must wake its worker");
    }
}

#[test]
fn trace2_monitor_finish_and_drop_drain_the_final_unterminated_event() {
    for finish in [false, true] {
        let (sender, receiver) = mpsc::channel();
        let context = GitOperationContext::new("trace drain", move |_, event| {
            sender.send(event).unwrap();
        });
        let mut cmd = Command::new("git");
        let monitor =
            Trace2Monitor::start(&mut cmd, Some(&context), &LivenessClock::new()).unwrap();
        std::fs::write(
            &monitor._path,
            concat!(
                "{\"event\":\"child_start\",\"sid\":\"test\",\"child_id\":1,",
                "\"child_class\":\"hook\",\"hook_name\":\"pre-commit\"}\n",
                "{\"event\":\"child_exit\",\"sid\":\"test\",\"child_id\":1,\"code\":7}"
            ),
        )
        .unwrap();
        if finish {
            monitor.finish();
        } else {
            drop(monitor);
        }
        let events: Vec<_> = receiver.try_iter().collect();
        assert!(matches!(events.as_slice(), [
            GitOperationEvent::HookStarted { name, id },
            GitOperationEvent::HookFinished { name: finished_name, id: finished_id, exit_code: Some(7), .. },
        ] if name == "pre-commit" && finished_name == name && finished_id == id));
    }
}

#[test]
fn operation_context_reports_real_hooks_output_and_exit_codes() {
    let repo = tempfile::tempdir().expect("create test repository");
    run_git_test_setup(repo.path(), &["init", "--quiet"]);
    run_git_test_setup(repo.path(), &["config", "user.name", "GitComet Test"]);
    run_git_test_setup(
        repo.path(),
        &["config", "user.email", "gitcomet@example.invalid"],
    );
    run_git_test_setup(repo.path(), &["config", "commit.gpgsign", "false"]);
    run_git_test_setup(repo.path(), &["config", "core.hooksPath", ".githooks"]);
    write_test_hook(
        repo.path(),
        "pre-commit",
        "#!/bin/sh\nprintf 'pre-commit stdout\\n'\nprintf 'pre-commit stderr\\n' >&2\n",
    );
    write_test_hook(
        repo.path(),
        "post-commit",
        "#!/bin/sh\nprintf 'post-commit stdout\\n'\nprintf 'post-commit stderr\\n' >&2\nexit 7\n",
    );
    std::fs::write(repo.path().join("file.txt"), "content\n").expect("write test file");
    run_git_test_setup(repo.path(), &["add", "--", "file.txt"]);

    let events = Arc::new(Mutex::new(Vec::<GitOperationEvent>::new()));
    let captured = Arc::clone(&events);
    let operation = GitOperationContext::new("Commit", move |_, event| {
        captured
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event);
    });
    let mut cmd = git_workdir_cmd_for(repo.path());
    cmd.args(["commit", "--quiet", "-m", "exercise hooks"]);
    {
        let _scope = git_operation::attach(&operation);
        run_git_simple(cmd, "git commit")
            .expect("post-commit failures must not fail the outer commit");
    }

    let events = events.lock().unwrap_or_else(|error| error.into_inner());
    let started = events
        .iter()
        .filter_map(|event| match event {
            GitOperationEvent::HookStarted { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(started, ["pre-commit", "post-commit"]);

    let finished = events
        .iter()
        .filter_map(|event| match event {
            GitOperationEvent::HookFinished {
                name, exit_code, ..
            } => Some((name.as_str(), *exit_code)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        finished,
        [("pre-commit", Some(0)), ("post-commit", Some(7))]
    );

    let output = events
        .iter()
        .filter_map(|event| match event {
            GitOperationEvent::Output { chunks } => Some(chunks),
            _ => None,
        })
        .flatten()
        .map(|chunk| chunk.text.as_str())
        .collect::<String>();
    assert!(output.contains("pre-commit stdout"));
    assert!(output.contains("pre-commit stderr"));
    assert!(output.contains("post-commit stdout"));
    assert!(output.contains("post-commit stderr"));
}

#[test]
fn run_git_with_output_failure_preserves_structured_details() {
    let err = run_git_with_output(failing_command_with_streams(), "git synthetic")
        .expect_err("expected failing command");

    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), "git synthetic");
            assert_eq!(failure.id(), GitFailureId::CommandFailed);
            assert_eq!(failure.exit_code(), Some(7));
            assert_eq!(failure.stdout(), b"out");
            assert_eq!(failure.stderr(), b"err");
            assert_eq!(failure.detail(), Some("err"));
            assert_eq!(failure.to_string(), "git synthetic failed: err");
        }
        other => panic!("expected structured git failure, got {other:?}"),
    }
}

#[test]
fn run_git_with_output_failure_falls_back_to_stdout_when_stderr_is_empty() {
    let err = run_git_with_output(failing_command_with_stdout_only(), "git synthetic")
        .expect_err("expected failing command");

    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), "git synthetic");
            assert_eq!(failure.id(), GitFailureId::CommandFailed);
            assert_eq!(failure.exit_code(), Some(9));
            assert_eq!(failure.stdout(), b"stdout only");
            assert_eq!(failure.stderr(), b"");
            assert_eq!(failure.detail(), Some("stdout only"));
            assert_eq!(failure.to_string(), "git synthetic failed: stdout only");
        }
        other => panic!("expected structured git failure, got {other:?}"),
    }
}

#[test]
fn run_git_failure_adds_gpg_signing_hint() {
    let err = run_git_with_output(failing_command_with_missing_gpg(), "git commit")
        .expect_err("expected failing command");

    match err.kind() {
        ErrorKind::Git(failure) => {
            let detail = failure.detail().expect("expected failure detail");
            assert!(detail.contains("cannot run gpg"));
            assert!(detail.contains("GUI app PATH"));
            assert!(detail.contains("git config --global gpg.program /path/to/gpg"));
        }
        other => panic!("expected structured git failure, got {other:?}"),
    }
}

#[test]
fn run_git_failure_does_not_add_path_hint_for_other_gpg_signing_errors() {
    let err = run_git_with_output(failing_command_with_gpg_signing_error(), "git commit")
        .expect_err("expected failing command");

    match err.kind() {
        ErrorKind::Git(failure) => {
            let detail = failure.detail().expect("expected failure detail");
            assert!(detail.contains("gpg failed to sign the data"));
            assert!(!detail.contains("GUI app PATH"));
            assert!(!detail.contains("git config --global gpg.program /path/to/gpg"));
        }
        other => panic!("expected structured git failure, got {other:?}"),
    }
}

#[test]
fn run_git_failure_adds_gpg_signing_hint_for_missing_gpg_program_path() {
    let err = run_git_with_output(
        failing_command_with_missing_gpg_program_path(),
        "git commit",
    )
    .expect_err("expected failing command");

    match err.kind() {
        ErrorKind::Git(failure) => {
            let detail = failure.detail().expect("expected failure detail");
            assert!(detail.contains("cannot run"));
            assert!(detail.contains("gpg"));
            assert!(detail.contains("GUI app PATH"));
            assert!(detail.contains("git config --global gpg.program /path/to/gpg"));
        }
        other => panic!("expected structured git failure, got {other:?}"),
    }
}

#[test]
fn classifies_git_lfs_failures_from_stderr() {
    let cases = [
        (
            "git-lfs filter-process: line 1: git-lfs: command not found\nfatal: a.bin: smudge filter lfs failed",
            GitFailureId::LfsNotInstalled,
        ),
        (
            "git: 'lfs' is not a git command. See 'git --help'.",
            GitFailureId::LfsNotInstalled,
        ),
        (
            "Error downloading object: a.bin (6667b2d): Smudge error: Error downloading a.bin",
            GitFailureId::LfsObjectMissing,
        ),
        (
            "fatal: a.bin: smudge filter lfs failed",
            GitFailureId::LfsObjectMissing,
        ),
        (
            "Unable to push locked files:\n* art/hero.psd - alice\nerror: failed to push",
            GitFailureId::LfsLocked,
        ),
        (
            "LFS upload failed:\n  (missing) a.bin",
            GitFailureId::LfsUploadFailed,
        ),
        ("fatal: not a git repository", GitFailureId::CommandFailed),
        (
            "error: cannot run gpg: No such file or directory",
            GitFailureId::CommandFailed,
        ),
    ];
    for (stderr, expected) in cases {
        assert_eq!(classify_git_failure(stderr), expected, "{stderr}");
    }
    let detail = add_lfs_failure_hint("boom".to_string(), GitFailureId::LfsNotInstalled);
    assert!(detail.starts_with("boom\n\nHint: Git LFS is not installed"));
    assert_eq!(
        add_lfs_failure_hint("boom".to_string(), GitFailureId::CommandFailed),
        "boom"
    );
}

/// Real stderr: the LFS hooks' own message, dash's (Debian/Ubuntu `sh`)
/// missing-command form, and a present git-lfs reporting a missing object,
/// whose "no such file or directory" is about the object, not the binary.
#[test]
fn missing_git_lfs_is_told_apart_from_missing_lfs_objects() {
    for stderr in [
        "\nThis repository is configured for Git LFS but 'git-lfs' was not found on your path. If you no longer wish to use Git LFS, remove this hook by deleting the 'pre-push' file in the hooks directory (set by 'core.hookspath'; usually '.git/hooks').\n\nerror: failed to push some refs to '../r2.git'",
        "git-lfs filter-process: 1: git-lfs: not found\nerror: could not read greeting from subprocess 'git-lfs filter-process'\nfatal: a.bin: clean filter 'lfs' failed",
        "'git-lfs' is not recognized as an internal or external command,\noperable program or batch file.",
    ] {
        assert_eq!(
            classify_git_failure(stderr),
            GitFailureId::LfsNotInstalled,
            "{stderr}"
        );
    }
    let missing_object = "Error downloading object: a.bin (6667b2d): Smudge error: Error reading from media file: open /r/.git/lfs/objects/66/67/6667b2d: no such file or directory\n\nerror: external filter 'git-lfs filter-process' failed\nfatal: a.bin: smudge filter lfs failed";
    assert_eq!(
        classify_git_failure(missing_object),
        GitFailureId::LfsObjectMissing
    );
    assert_ne!(
        classify_git_failure(
            "git-lfs: open .git/lfs/objects/66/67/6667b2d: no such file or directory"
        ),
        GitFailureId::LfsNotInstalled
    );
}

#[test]
fn run_command_with_timeout_returns_structured_timeout_failure() {
    let err = run_command_with_timeout(
        sleep_command(2),
        "git synthetic",
        Duration::from_millis(50),
        None,
    )
    .expect_err("expected timed out command");

    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), "git synthetic");
            assert_eq!(failure.id(), GitFailureId::Timeout);
            assert!(failure.detail().is_some_and(|detail| {
                detail.contains("set GITCOMET_GIT_COMMAND_TIMEOUT_SECS to override")
            }));
            assert!(
                failure
                    .to_string()
                    .starts_with("git synthetic timed out after")
            );
        }
        other => panic!("expected structured git timeout, got {other:?}"),
    }
}

#[test]
fn git_command_wait_poll_is_short_for_fast_commands_and_capped_for_slow_ones() {
    assert_eq!(
        git_command_wait_poll(Duration::from_micros(500), Duration::from_secs(1)),
        Some(Duration::from_micros(250))
    );
    assert_eq!(
        git_command_wait_poll(Duration::from_millis(10), Duration::from_secs(1)),
        Some(Duration::from_millis(1))
    );
    assert_eq!(
        git_command_wait_poll(Duration::from_millis(50), Duration::from_secs(1)),
        Some(Duration::from_millis(5))
    );
    assert_eq!(
        git_command_wait_poll(Duration::from_millis(50), Duration::from_millis(2)),
        Some(Duration::from_millis(2))
    );
    assert_eq!(
        git_command_wait_poll(Duration::from_millis(50), Duration::ZERO),
        None
    );
}

#[cfg(unix)]
#[test]
fn chatty_command_outlives_a_silence_deadline_shorter_than_its_runtime() {
    let output = run_command_with_timeout(
        shell_command("for i in 1 2 3 4 5 6; do echo tick; sleep 0.25; done"),
        "git synthetic",
        Duration::from_millis(600),
        None,
    )
    .expect("regular output keeps the command alive");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .matches("tick")
            .count(),
        6
    );
}

/// git-lfs prints nothing without a TTY, so its progress file is the only
/// sign of life. A checkout (or a new worktree's) can bring LFS into a
/// repository whose current commit has none, so it is monitored all the same.
#[cfg(unix)]
#[test]
fn checkout_that_brings_in_lfs_stays_alive_through_its_progress_file() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    let git = |args: &[&str]| {
        let mut cmd = Command::new("git");
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-C"])
            .arg(workdir)
            .args(args);
        assert!(cmd.status().unwrap().success(), "{args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "-q", "--allow-empty", "-m", "no lfs"]);
    git(&["switch", "-q", "-c", "assets"]);
    std::fs::write(workdir.join(".gitattributes"), "*.bin filter=lfs\n").unwrap();
    std::fs::write(workdir.join("a.bin"), "content\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "lfs"]);
    git(&["switch", "-q", "main"]);
    git(&[
        "config",
        "filter.lfs.smudge",
        "for i in 1 2 3 4 5 6; do if [ -n \"$GIT_LFS_PROGRESS\" ]; then \
         echo \"download $i/6 1/1 a.bin\" >> \"$GIT_LFS_PROGRESS\"; fi; sleep 0.25; done; cat",
    ]);
    git(&["config", "filter.lfs.required", "true"]);
    let repo = crate::repo::GixRepo::new(
        workdir.to_path_buf(),
        gix::open(workdir).unwrap().into_sync(),
    );
    let worktree = dir.path().join("wt");
    let worktree_arg = worktree.to_str().unwrap();
    for (label, args, checked_out) in [
        (
            "git worktree add",
            vec!["worktree", "add", "-q", "--detach", worktree_arg, "assets"],
            worktree.as_path(),
        ),
        ("git checkout", vec!["checkout", "-q", "assets"], workdir),
    ] {
        let mut cmd = repo.git_workdir_cmd();
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(&args);
        run_command_with_timeout(cmd, label, Duration::from_millis(600), None)
            .unwrap_or_else(|e| panic!("progress lines keep `{label}` alive: {e}"));
        assert_eq!(
            std::fs::read_to_string(checked_out.join("a.bin")).unwrap(),
            "content\n"
        );
    }
}

#[test]
fn submodule_and_worktree_checkouts_may_transfer_lfs_content() {
    for args in [["submodule", "update"], ["worktree", "add"]] {
        let mut cmd = Command::new("git");
        cmd.args(args);
        assert!(may_transfer_lfs_content(&cmd), "{args:?}");
    }
}

#[test]
fn lfs_progress_drains_a_line_written_during_the_last_emit() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "download 1/2 500/1000 a.bin\n").unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let writer_done = Arc::clone(&done);
    let path = file.path().to_path_buf();
    let (sender, receiver) = mpsc::channel();
    let context = GitOperationContext::new("lfs progress", move |_, event| {
        if !writer_done.load(Ordering::Acquire) {
            use std::io::Write;
            let mut writer = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(writer, "download 2/2 1000/1000 a.bin").unwrap();
            writer_done.store(true, Ordering::Release);
        }
        sender.send(event).unwrap();
    });
    lfs_progress_tail_loop(file.path(), Some(&context), &done, &LivenessClock::new());
    let events: Vec<_> = receiver.try_iter().collect();
    assert!(
        matches!(events.last(), Some(GitOperationEvent::TransferProgress(p)) if p.bytes_done == 1000),
        "{events:?}"
    );
}

#[test]
fn lfs_progress_monitor_reports_lines_and_keeps_the_command_alive() {
    let (sender, receiver) = mpsc::channel();
    let context = GitOperationContext::new("lfs progress", move |_, event| {
        sender.send(event).unwrap();
    });
    let liveness = LivenessClock::new();
    let mut cmd = Command::new("git");
    cmd.args(["lfs", "pull"]);
    let mut monitor = LfsProgressMonitor::start(&mut cmd, Some(&context), &liveness)
        .expect("lfs commands are monitored");
    thread::sleep(Duration::from_millis(40));
    std::fs::write(&monitor._path, "download 1/2 500/1000 a.bin\n").unwrap();
    monitor.stop();
    assert!(
        liveness.idle() < Duration::from_millis(40),
        "a line counts as activity"
    );
    let events: Vec<_> = receiver.try_iter().collect();
    assert!(
        events.iter().any(|event| matches!(
            event,
            GitOperationEvent::TransferProgress(progress) if progress.files_done == 1 && progress.bytes_total == 1000
        )),
        "{events:?}"
    );

    let mut status = Command::new("git");
    status.arg("status");
    assert!(LfsProgressMonitor::start(&mut status, None, &liveness).is_none());
}

#[test]
fn liveness_clock_measures_time_since_last_touch() {
    let clock = LivenessClock::new();
    thread::sleep(Duration::from_millis(30));
    assert!(clock.idle() >= Duration::from_millis(30));
    clock.touch();
    assert!(clock.idle() < Duration::from_millis(30));
}

fn gitpython_rev_list_fixture_to_pretty_record(fixture: &str) -> String {
    let id = fixture
        .lines()
        .find_map(|line| line.strip_prefix("commit "))
        .expect("rev-list fixture should contain commit id")
        .trim();

    let parents = fixture
        .lines()
        .filter_map(|line| line.strip_prefix("parent "))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");

    let author_line = fixture
        .lines()
        .find(|line| line.starts_with("author "))
        .expect("rev-list fixture should contain author line");
    let author = author_line
        .strip_prefix("author ")
        .and_then(|line| line.split_once(" <").map(|(name, _)| name))
        .expect("author line should include actor and email");
    let time = author_line
        .split_whitespace()
        .rev()
        .nth(1)
        .expect("author line should contain unix timestamp")
        .trim();

    let summary = fixture
        .lines()
        .find_map(|line| line.strip_prefix("    "))
        .unwrap_or_default()
        .trim();

    format!("{id}\x1f{parents}\x1f{author}\x1f{time}\x1f{summary}\x1e")
}

#[test]
fn parse_remote_branches_splits_and_skips_head() {
    let output = "origin/HEAD\tdeadbeef\norigin/main\t1111111\nupstream/feature/foo\t2222222\n\n";
    let branches = parse_remote_branches(output);
    assert_eq!(
        branches,
        vec![
            RemoteBranch {
                remote: "origin".to_string(),
                name: "main".to_string(),
                target: CommitId("1111111".into())
            },
            RemoteBranch {
                remote: "upstream".to_string(),
                name: "feature/foo".to_string(),
                target: CommitId("2222222".into())
            },
        ]
    );
}

#[test]
fn unix_seconds_to_system_time_clamps_negative_to_epoch() {
    assert_eq!(
        unix_seconds_to_system_time_or_epoch(-1),
        SystemTime::UNIX_EPOCH
    );
    assert_eq!(
        unix_seconds_to_system_time_or_epoch(1),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1)
    );
}

#[test]
fn parse_remote_branches_handles_path_components_from_gitpython_fixture() {
    let raw = std::str::from_utf8(GITPY_FOR_EACH_REF_WITH_PATH_COMPONENT)
        .expect("fixture should be valid UTF-8");
    let mut fields = raw.trim().split('\0');
    let full_ref = fields.next().expect("refname field");
    let oid = fields.next().expect("object id field");
    let short = full_ref
        .strip_prefix("refs/heads/")
        .expect("heads ref prefix in fixture");

    let output = format!("origin/{short}\t{oid}\norigin/HEAD\tdeadbeef\n");
    let branches = parse_remote_branches(&output);

    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0].remote, "origin");
    assert_eq!(branches[0].name, "refactoring/feature1");
    assert_eq!(branches[0].target, CommitId(oid.to_string().into()));
}

#[test]
fn parse_git_log_pretty_records_parses_single_commit_from_gitpython_fixture() {
    let output = gitpython_rev_list_fixture_to_pretty_record(GITPY_REV_LIST_SINGLE);
    let page = parse_git_log_pretty_records(&output);

    assert_eq!(page.commits.len(), 1);
    assert!(page.next_cursor.is_none());
    let commit = &page.commits[0];
    assert_eq!(
        commit.id,
        CommitId("4c8124ffcf4039d292442eeccabdeca5af5c5017".into())
    );
    assert_eq!(
        commit.parent_ids.as_slice(),
        &[CommitId("634396b2f541a9f2d58b00be1a07f0c358b999b3".into())]
    );
    assert_eq!(&*commit.author, "Tom Preston-Werner");
    assert_eq!(&*commit.summary, "implement Grit#heads");
    assert_eq!(
        commit.time,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_191_999_972)
    );
}

#[test]
fn parse_git_log_pretty_records_parses_multiple_gitpython_fixtures() {
    let output = format!(
        "{}{}",
        gitpython_rev_list_fixture_to_pretty_record(GITPY_REV_LIST_SINGLE),
        gitpython_rev_list_fixture_to_pretty_record(GITPY_REV_LIST_COMMIT_STATS)
    );
    let page = parse_git_log_pretty_records(&output);

    assert_eq!(page.commits.len(), 2);
    assert!(page.next_cursor.is_none());

    assert_eq!(
        page.commits[1].id,
        CommitId("634396b2f541a9f2d58b00be1a07f0c358b999b3".into())
    );
    assert!(page.commits[1].parent_ids.is_empty());
    assert_eq!(&*page.commits[1].author, "Tom Preston-Werner");
    assert_eq!(&*page.commits[1].summary, "initial grit setup");
    assert!(Arc::ptr_eq(
        &page.commits[0].author,
        &page.commits[1].author
    ));
    assert!(Arc::ptr_eq(
        &page.commits[0].parent_ids[0].0,
        &page.commits[1].id.0
    ));
    assert_eq!(
        page.commits[1].time,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_191_997_100)
    );
}

#[test]
fn parse_git_log_pretty_records_from_reader_matches_string_parser() {
    let output = format!(
        "{}{}",
        gitpython_rev_list_fixture_to_pretty_record(GITPY_REV_LIST_SINGLE),
        gitpython_rev_list_fixture_to_pretty_record(GITPY_REV_LIST_COMMIT_STATS)
    );

    let from_string = parse_git_log_pretty_records(&output);
    let from_reader =
        parse_git_log_pretty_records_from_reader(std::io::Cursor::new(output.as_bytes()))
            .expect("streaming parser");

    assert_eq!(from_reader, from_string);
    assert!(Arc::ptr_eq(
        &from_reader.commits[0].author,
        &from_reader.commits[1].author
    ));
    assert!(Arc::ptr_eq(
        &from_reader.commits[0].parent_ids[0].0,
        &from_reader.commits[1].id.0
    ));
}

#[test]
fn parse_remote_branches_handles_pull_ref_prefixes_from_gitpython_fixture() {
    let mut output = gitpython_fetch_head_to_remote_ref_output(
        GITPY_UNCOMMON_BRANCH_PREFIX_FETCH_HEAD,
        "origin",
    );
    output.push_str("origin/HEAD\tdeadbeef\n");
    let branches = parse_remote_branches(&output);

    let names = branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "pull/1/head",
            "pull/1/merge",
            "pull/2/head",
            "pull/2/merge",
            "pull/3/head",
            "pull/3/merge",
        ]
    );
    assert_eq!(branches.len(), 6);
    assert_eq!(
        branches[0].target,
        CommitId("c2e3c20affa3e2b61a05fdc9ee3061dd416d915e".into())
    );
}

#[test]
fn command_may_require_auth_detects_auth_related_git_commands() {
    let mut push = Command::new("git");
    push.args(["-C", "/tmp/repo", "push", "origin", "main"]);
    assert!(command_may_require_auth(&push));

    let mut fetch = Command::new("git");
    fetch.args(["-c", "color.ui=false", "fetch", "--all"]);
    assert!(command_may_require_auth(&fetch));

    let mut ls_remote = Command::new("git");
    ls_remote.args(["ls-remote", "origin"]);
    assert!(command_may_require_auth(&ls_remote));

    let mut commit = Command::new("git");
    commit.args(["commit", "-m", "msg"]);
    assert!(command_may_require_auth(&commit));

    let mut status = Command::new("git");
    status.args(["-C", "/tmp/repo", "status", "--short"]);
    assert!(!command_may_require_auth(&status));

    let mut log = Command::new("git");
    log.args(["log", "--oneline", "-n", "1"]);
    assert!(!command_may_require_auth(&log));
}

#[test]
fn command_may_require_auth_covers_ssh_signing_commands() {
    for args in [
        vec!["commit-tree", "-S", "HEAD^{tree}", "-m", "message"],
        vec!["tag", "-s", "v1", "HEAD"],
        vec!["merge", "--no-ff", "topic"],
        vec!["rebase", "--continue"],
        vec!["cherry-pick", "abc123"],
        vec!["revert", "--no-edit", "abc123"],
        vec!["revert", "--continue"],
        vec!["commit", "--no-verify", "-F", "MERGE_MSG"],
        vec!["am", "--3way"],
    ] {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg("/tmp/repo").args(&args);
        assert!(
            command_may_require_auth(&cmd),
            "expected askpass for `git {}`",
            args.join(" ")
        );
    }
}

#[test]
fn create_askpass_script_writes_expected_content_and_permissions() {
    let askpass = create_askpass_script().expect("askpass script creation");
    assert!(askpass.path().exists());

    let contents =
        std::fs::read_to_string(askpass.path()).expect("askpass script should be readable");
    assert!(contents.contains("GITCOMET_AUTH_SECRET"));
    assert!(contents.contains("GITCOMET_AUTH_KIND"));
    assert!(contents.contains("host_verification"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let mode = std::fs::metadata(askpass.path())
            .expect("askpass metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }
}

#[test]
fn append_host_prompt_to_stderr_includes_logged_prompt_with_fingerprint() {
    let askpass = create_askpass_script().expect("askpass script creation");
    std::fs::write(
        askpass.host_prompt_log_path(),
        "The authenticity of host 'github.com (140.82.121.3)' can't be established.\nED25519 key fingerprint is: SHA256:+DiY...\nAre you sure you want to continue connecting (yes/no/[fingerprint])?",
    )
    .expect("write prompt log");

    let mut stderr = b"Host key verification failed.\n".to_vec();
    append_host_prompt_to_stderr(&mut stderr, &askpass);

    let rendered = String::from_utf8(stderr).expect("stderr should be utf-8 for test");
    assert!(rendered.contains("SSH host verification prompt:"));
    assert!(rendered.contains("ED25519 key fingerprint is: SHA256:+DiY..."));
    assert!(rendered.contains("yes/no/[fingerprint]"));
}

#[test]
fn append_host_prompt_to_stderr_skips_when_prompt_already_present() {
    let askpass = create_askpass_script().expect("askpass script creation");
    let prompt = "Are you sure you want to continue connecting (yes/no/[fingerprint])?";
    std::fs::write(askpass.host_prompt_log_path(), prompt).expect("write prompt log");

    let mut stderr = format!("Host key verification failed.\n{prompt}\n").into_bytes();
    append_host_prompt_to_stderr(&mut stderr, &askpass);

    let rendered = String::from_utf8(stderr).expect("stderr should be utf-8 for test");
    assert_eq!(rendered.matches("SSH host verification prompt:").count(), 0);
    assert_eq!(rendered.matches(prompt).count(), 1);
}

fn command_env_value(cmd: &Command, key: &str) -> Option<String> {
    use std::ffi::OsStr;

    cmd.get_envs().find_map(|(k, v)| {
        if k == OsStr::new(key) {
            v.and_then(|value| value.to_str().map(ToOwned::to_owned))
        } else {
            None
        }
    })
}

fn command_env_removed(cmd: &Command, key: &str) -> bool {
    use std::ffi::OsStr;

    cmd.get_envs()
        .any(|(k, v)| k == OsStr::new(key) && v.is_none())
}

#[test]
fn repository_git_commands_disable_the_ext_protocol() {
    let cmd = git_workdir_cmd_for(Path::new("repo"));
    let args = cmd.get_args().collect::<Vec<_>>();
    assert!(args.windows(2).any(|args| {
        args == [
            std::ffi::OsStr::new("-c"),
            std::ffi::OsStr::new("protocol.ext.allow=never"),
        ]
    }));
}

#[test]
fn repository_git_commands_do_not_start_auto_maintenance() {
    let repo = tempfile::tempdir().expect("tempdir");
    let trace_dir = tempfile::tempdir().expect("trace tempdir");
    let trace = trace_dir.path().join("trace2.json");
    run_git_test_setup(repo.path(), &["init", "--quiet"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
    ] {
        run_git_test_setup(repo.path(), &["config", key, value]);
    }

    let output = git_workdir_cmd_for(repo.path())
        .env("GIT_TRACE2_EVENT", &trace)
        .args(["commit", "--allow-empty", "--quiet", "-m", "c"])
        .output()
        .expect("run git commit");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Git starts `maintenance run --auto` after every commit unless
    // `maintenance.auto` is false; GitComet only recommends maintenance.
    let events = std::fs::read_to_string(&trace).expect("trace2 events");
    assert!(events.contains("\"name\":\"commit\""), "{events}");
    assert!(!events.contains("\"maintenance\""), "{events}");
}

#[cfg(unix)]
#[test]
fn repository_config_cannot_reenable_the_ext_protocol() {
    let repo = tempfile::tempdir().expect("tempdir");
    run_git_test_setup(repo.path(), &["init", "--quiet"]);
    run_git_test_setup(repo.path(), &["config", "protocol.ext.allow", "always"]);

    let mut cmd = git_workdir_cmd_for(repo.path());
    // Git's refusal text is translated; assert on the exit status.
    let output = cmd
        .env("LC_ALL", "C")
        .args(["ls-remote", "ext::printf invoked"])
        .output()
        .expect("run git");
    assert!(
        !output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn configure_git_auth_prompt_sets_username_password_env() {
    let askpass = create_askpass_script().expect("askpass script creation");
    let mut cmd = Command::new("git");
    let auth = PromptAuth::Explicit(StagedGitAuth {
        kind: GitAuthKind::UsernamePassword,
        username: Some("alice".to_string()),
        secret: "secret-token".to_string(),
    });

    configure_git_auth_prompt(&mut cmd, Some(&auth), &askpass);

    let askpass_path = askpass
        .path()
        .to_str()
        .expect("temporary askpass path should be unicode")
        .to_string();
    assert_eq!(
        command_env_value(&cmd, "GIT_ASKPASS").as_deref(),
        Some(askpass_path.as_str())
    );
    assert_eq!(
        command_env_value(&cmd, "SSH_ASKPASS").as_deref(),
        Some(askpass_path.as_str())
    );
    assert_eq!(
        command_env_value(&cmd, "SSH_ASKPASS_REQUIRE").as_deref(),
        Some("force")
    );
    assert_eq!(
        command_env_value(&cmd, GITCOMET_ASKPASS_PROMPT_LOG_ENV).as_deref(),
        askpass.host_prompt_log_path().to_str()
    );
    assert_eq!(
        command_env_value(&cmd, GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG_ENV).as_deref(),
        askpass.passphrase_prompt_log_path().to_str()
    );
    assert_eq!(
        command_env_value(&cmd, GITCOMET_AUTH_KIND_ENV).as_deref(),
        Some(GITCOMET_AUTH_KIND_USERNAME_PASSWORD)
    );
    assert_eq!(
        command_env_value(&cmd, GITCOMET_AUTH_USERNAME_ENV).as_deref(),
        Some("alice")
    );
    assert_eq!(
        command_env_value(&cmd, GITCOMET_AUTH_SECRET_ENV).as_deref(),
        Some("secret-token")
    );
    assert_eq!(
        command_env_value(&cmd, GITCOMET_AUTH_CACHE_SIZE_ENV).as_deref(),
        Some("0")
    );

    if cfg!(all(unix, not(target_os = "macos"))) && std::env::var_os("DISPLAY").is_none() {
        assert_eq!(
            command_env_value(&cmd, "DISPLAY").as_deref(),
            Some("gitcomet:0")
        );
    }
}

#[test]
fn configure_git_auth_prompt_sets_passphrase_env_and_removes_username() {
    let askpass = create_askpass_script().expect("askpass script creation");
    let mut cmd = Command::new("git");
    cmd.env(GITCOMET_AUTH_USERNAME_ENV, "legacy-user");
    let auth = PromptAuth::Explicit(StagedGitAuth {
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: "ssh-passphrase".to_string(),
    });

    configure_git_auth_prompt(&mut cmd, Some(&auth), &askpass);

    assert_eq!(
        command_env_value(&cmd, GITCOMET_AUTH_KIND_ENV).as_deref(),
        Some(GITCOMET_AUTH_KIND_PASSPHRASE)
    );
    assert!(command_env_removed(&cmd, GITCOMET_AUTH_USERNAME_ENV));
    assert_eq!(
        command_env_value(&cmd, GITCOMET_AUTH_SECRET_ENV).as_deref(),
        Some("ssh-passphrase")
    );
}

#[test]
fn configure_git_auth_prompt_sets_cached_passphrase_env_and_removes_username() {
    let askpass = create_askpass_script().expect("askpass script creation");
    let mut cmd = Command::new("git");
    cmd.env(GITCOMET_AUTH_USERNAME_ENV, "legacy-user");
    let auth = PromptAuth::CachedPassphrases(vec![
        CachedPassphraseEntry {
            prompt: "Enter passphrase for key '/tmp/key-a':".to_string(),
            secret: "ssh-passphrase-a".to_string(),
        },
        CachedPassphraseEntry {
            prompt: "Enter passphrase for key '/tmp/key-b':".to_string(),
            secret: "ssh-passphrase-b".to_string(),
        },
    ]);

    configure_git_auth_prompt(&mut cmd, Some(&auth), &askpass);

    assert_eq!(
        command_env_value(&cmd, GITCOMET_AUTH_KIND_ENV).as_deref(),
        Some(GITCOMET_AUTH_KIND_PASSPHRASE_CACHED)
    );
    assert!(command_env_removed(&cmd, GITCOMET_AUTH_USERNAME_ENV));
    assert!(command_env_removed(&cmd, GITCOMET_AUTH_SECRET_ENV));
    assert_eq!(
        command_env_value(&cmd, GITCOMET_AUTH_CACHE_SIZE_ENV).as_deref(),
        Some("2")
    );
    assert_eq!(
        command_env_value(&cmd, "GITCOMET_AUTH_CACHE_PROMPT_0").as_deref(),
        Some("Enter passphrase for key '/tmp/key-a':")
    );
    assert_eq!(
        command_env_value(&cmd, "GITCOMET_AUTH_CACHE_SECRET_0").as_deref(),
        Some("ssh-passphrase-a")
    );
    assert_eq!(
        command_env_value(&cmd, "GITCOMET_AUTH_CACHE_PROMPT_1").as_deref(),
        Some("Enter passphrase for key '/tmp/key-b':")
    );
    assert_eq!(
        command_env_value(&cmd, "GITCOMET_AUTH_CACHE_SECRET_1").as_deref(),
        Some("ssh-passphrase-b")
    );
}

#[test]
fn configure_git_auth_prompt_sets_host_verification_env_and_removes_username() {
    let askpass = create_askpass_script().expect("askpass script creation");
    let mut cmd = Command::new("git");
    cmd.env(GITCOMET_AUTH_USERNAME_ENV, "legacy-user");
    let auth = PromptAuth::Explicit(StagedGitAuth {
        kind: GitAuthKind::HostVerification,
        username: None,
        secret: "yes".to_string(),
    });

    configure_git_auth_prompt(&mut cmd, Some(&auth), &askpass);

    assert_eq!(
        command_env_value(&cmd, GITCOMET_AUTH_KIND_ENV).as_deref(),
        Some(GITCOMET_AUTH_KIND_HOST_VERIFICATION)
    );
    assert!(command_env_removed(&cmd, GITCOMET_AUTH_USERNAME_ENV));
    assert_eq!(
        command_env_value(&cmd, GITCOMET_AUTH_SECRET_ENV).as_deref(),
        Some("yes")
    );
}

#[test]
fn configure_git_auth_prompt_without_staged_auth_clears_auth_env() {
    let askpass = create_askpass_script().expect("askpass script creation");
    let mut cmd = Command::new("git");
    cmd.env(GITCOMET_AUTH_KIND_ENV, "legacy-kind");
    cmd.env(GITCOMET_AUTH_USERNAME_ENV, "legacy-user");
    cmd.env(GITCOMET_AUTH_SECRET_ENV, "legacy-secret");

    configure_git_auth_prompt(&mut cmd, None, &askpass);

    let askpass_path = askpass
        .path()
        .to_str()
        .expect("temporary askpass path should be unicode")
        .to_string();
    assert_eq!(
        command_env_value(&cmd, "GIT_ASKPASS").as_deref(),
        Some(askpass_path.as_str())
    );
    assert!(command_env_removed(&cmd, GITCOMET_AUTH_KIND_ENV));
    assert!(command_env_removed(&cmd, GITCOMET_AUTH_USERNAME_ENV));
    assert!(command_env_removed(&cmd, GITCOMET_AUTH_SECRET_ENV));
}

#[test]
fn run_git_with_stdin_capture_times_out_when_command_exceeds_timeout() {
    let err = run_git_with_stdin_capture(
        sleep_command(2),
        vec![],
        "git synthetic",
        Duration::from_millis(50),
        None,
    )
    .expect_err("expected timed out command");

    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), "git synthetic");
            assert_eq!(failure.id(), GitFailureId::Timeout);
        }
        other => panic!("expected structured git timeout, got {other:?}"),
    }
}

#[test]
fn run_git_with_stdin_capture_respects_cancellation() {
    let token = CancellationToken::new();
    let child_token = token.clone();

    let handle = thread::spawn(move || {
        run_git_with_stdin_capture(
            sleep_command(10),
            vec![],
            "git synthetic",
            Duration::from_secs(30),
            Some(&child_token),
        )
    });

    thread::sleep(Duration::from_millis(50));
    token.cancel();

    let result = handle.join().expect("thread should not panic");
    match result {
        Err(err) => match err.kind() {
            ErrorKind::Cancelled => {}
            other => panic!("expected cancellation error, got {other:?}"),
        },
        Ok(_) => panic!("expected cancellation error, but command succeeded"),
    }
}

#[test]
fn pre_cancelled_command_returns_cancelled_before_spawn() {
    let token = CancellationToken::new();
    token.cancel();
    let missing_command = Command::new("gitcomet-command-that-must-not-be-spawned");

    let error = run_command_with_timeout(
        missing_command,
        "git synthetic",
        Duration::from_secs(1),
        Some(&token),
    )
    .expect_err("an already-cancelled operation must reject the command");

    assert!(
        matches!(error.kind(), ErrorKind::Cancelled),
        "cancellation must win before spawn, got {error:?}"
    );
}

#[test]
fn submodule_byte_capture_stops_an_in_flight_command() {
    let token = CancellationToken::new();
    let child_token = token.clone();
    let handle = thread::spawn(move || {
        run_git_capture_bytes_cancellable(
            sleep_command(10),
            "git synthetic submodule numstat",
            &child_token,
        )
    });
    thread::sleep(Duration::from_millis(50));
    let cancelled_at = Instant::now();
    token.cancel();
    let error = handle
        .join()
        .expect("capture worker")
        .expect_err("cancelled capture");
    assert!(matches!(error.kind(), ErrorKind::Cancelled));
    assert!(cancelled_at.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn cancellation_after_leader_exit_stops_pipe_holding_descendant() {
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let handle = thread::spawn(move || {
        run_command_with_timeout(
            shell_command("sleep 3 &"),
            "git synthetic",
            Duration::from_secs(10),
            Some(&worker_token),
        )
    });

    // The shell leader exits immediately; its background child keeps the
    // captured stdout/stderr descriptors open.
    thread::sleep(Duration::from_millis(150));
    let cancelled_at = Instant::now();
    token.cancel();
    let result = handle.join().expect("command thread should not panic");

    assert!(
        matches!(result, Err(ref error) if matches!(error.kind(), ErrorKind::Cancelled)),
        "Stop must remain observable while output readers drain, got {result:?}"
    );
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(1),
        "the pipe-holding descendant was not terminated promptly"
    );
}

#[test]
fn operation_registry_cancellation_stops_the_attached_process() {
    let operation = GitOperationContext::new("synthetic", |_, _| {});
    let worker_operation = operation.clone();
    let handle = thread::spawn(move || {
        let _scope = git_operation::attach(&worker_operation);
        run_git_with_stdin_capture(
            sleep_command(10),
            vec![],
            "git synthetic",
            Duration::from_secs(30),
            None,
        )
    });

    thread::sleep(Duration::from_millis(50));
    assert!(git_operation::cancel(operation.id()));

    let result = handle.join().expect("thread should not panic");
    match result {
        Err(err) => match err.kind() {
            ErrorKind::Cancelled => {}
            other => panic!("expected cancellation error, got {other:?}"),
        },
        Ok(_) => panic!("expected cancellation error, but command succeeded"),
    }
}

/// Spawn a member of `group_id` that exits immediately and is never
/// reaped, reproducing the orphan a container init such as
/// `tail -f /dev/null` adopts and then leaves as a permanent zombie.
#[cfg(target_os = "linux")]
fn spawn_unreaped_zombie_in_group(group_id: u32) -> std::process::Child {
    use std::os::unix::process::CommandExt as _;

    let mut cmd = shell_command("exit 0");
    cmd.process_group(group_id as i32);
    let mut child = cmd.spawn().expect("group member should start");

    let pid = child.id() as i32;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if proc_process_state_and_group(pid).is_some_and(|(state, _)| state == 'Z') {
            return child;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("group member {pid} never became a zombie");
}

#[cfg(target_os = "linux")]
#[test]
fn process_group_liveness_ignores_unreaped_zombies() {
    use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};

    // Replace the shell so waiting for the leader also waits for sleep.
    // A separate sleep child could still be exiting after the shell is reaped.
    let mut cmd = shell_command("exec sleep 10");
    configure_git_process_tree(&mut cmd);
    let mut leader = cmd.spawn().expect("synthetic process group should start");
    let group_id = leader.id();
    let pid = Pid::from_raw(group_id as i32).expect("group id should be a valid pid");

    let mut zombie = spawn_unreaped_zombie_in_group(group_id);
    let leader_was_live = process_group_has_live_member(pid);

    let kill_result = kill_process_group(pid, Signal::KILL);
    let leader_wait_result = leader.wait();

    // The zombie still answers the signal probe, which is exactly why the
    // probe alone cannot decide when a process group is finished.
    let zombie_kept_group_addressable = test_kill_process_group(pid).is_ok();
    let zombie_group_was_live = process_group_has_live_member(pid);

    // Reap both owned children before assertions so failures do not leak them.
    let zombie_wait_result = zombie.wait();
    kill_result.expect("process group should receive KILL");
    leader_wait_result.expect("process-group leader should be reaped");
    zombie_wait_result.expect("unreaped zombie should be reaped after observation");
    assert!(
        leader_was_live,
        "a running leader must count as a live group member"
    );
    assert!(
        zombie_kept_group_addressable,
        "the unreaped zombie should keep the process group addressable"
    );
    assert!(
        !zombie_group_was_live,
        "a group holding only zombies has nothing left to terminate"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn process_tree_termination_returns_promptly_past_unreaped_zombies() {
    use rustix::process::{Pid, Signal, kill_process_group};

    // Keep the leader as the only live member so this measures zombie handling.
    let mut cmd = shell_command("exec sleep 10");
    configure_git_process_tree(&mut cmd);
    let mut child = cmd.spawn().expect("synthetic process group should start");
    let group_id = child.id();
    let pid = Pid::from_raw(group_id as i32).expect("group id should be a valid pid");
    let mut zombie = spawn_unreaped_zombie_in_group(group_id);

    let started = Instant::now();
    let result = terminate_process_tree_and_wait(&mut child);
    let elapsed = started.elapsed();

    let cleanup_result = kill_process_group(pid, Signal::KILL);
    let leader_wait_result = child.wait();
    let zombie_wait_result = zombie.wait();
    cleanup_result.expect("process group should receive cleanup KILL");
    leader_wait_result.expect("process-group leader should be reaped");
    zombie_wait_result.expect("unreaped zombie should be reaped after observation");
    result.expect("process-group termination should reap the leader");

    assert!(
        elapsed < GIT_PROCESS_TERMINATE_GRACE,
        "termination waited {elapsed:?} on a zombie that had already released its descriptors"
    );
}

#[cfg(unix)]
#[test]
fn process_tree_termination_keeps_the_group_grace_when_the_leader_exits() {
    use rustix::process::{Pid, Signal, kill_process_group};

    let mut cmd =
        shell_command("trap 'exit 0' TERM; (trap '' TERM; sleep 10) & while :; do sleep 1; done");
    configure_git_process_tree(&mut cmd);
    let mut child = cmd.spawn().expect("synthetic process group should start");
    let group_id = child.id();
    thread::sleep(Duration::from_millis(100));

    let started = Instant::now();
    let result = terminate_process_tree_and_wait(&mut child);
    let elapsed = started.elapsed();

    // Always clean up the intentionally TERM-resistant descendant when
    // this regression fails against the old implementation.
    if let Some(pid) = Pid::from_raw(group_id as i32) {
        let _ = kill_process_group(pid, Signal::KILL);
    }
    result.expect("process-group termination should reap the leader");

    assert!(
        elapsed >= GIT_PROCESS_TERMINATE_GRACE.saturating_sub(Duration::from_millis(100)),
        "termination returned after {elapsed:?}, before TERM-resistant descendants received the final KILL"
    );
}

#[test]
fn run_git_with_stdin_capture_forwards_stdin_and_captures_stdout() {
    #[cfg(unix)]
    let cmd = shell_command("cat");
    #[cfg(windows)]
    let cmd = shell_command("[Console]::Out.Write([Console]::In.ReadToEnd())");

    let input = b"hello stdin\nline two\n".to_vec();

    let output = run_git_with_stdin_capture(
        cmd,
        input.clone(),
        "cat stdin",
        Duration::from_secs(5),
        None,
    )
    .expect("stdin forwarding should succeed");

    assert_eq!(output, input);
}
