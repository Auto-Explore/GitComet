use super::*;

#[test]
fn newest_hundred_executions_are_retained_per_repository() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let a = RepoLogKey::new(&root.path().join("a"));
    let b = RepoLogKey::new(&root.path().join("b"));
    drop(log.begin(&b, ExecutionKind::Gix, "other repository", "test"));
    for i in 0..101 {
        drop(log.begin(&a, ExecutionKind::Gix, &i.to_string(), "test"));
    }
    let entries = log.snapshot(&a);
    assert_eq!(entries.len(), 100);
    assert_eq!(entries.first().unwrap().command, "100");
    assert_eq!(entries.last().unwrap().command, "1");
    assert_eq!(log.snapshot(&b)[0].command, "other repository");
}

#[test]
fn clearing_and_eviction_drop_late_output_and_completion() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    let mut old = log.begin(&key, ExecutionKind::Git, "old", "test");
    let output = old.output().unwrap();
    for _ in 0..100 {
        drop(log.begin(&key, ExecutionKind::Gix, "new", "test"));
    }
    output.append(GitOutputStream::Stdout, b"late");
    old.finish(ExecutionOutcome::Success);
    assert!(
        log.snapshot(&key)
            .iter()
            .all(|entry| entry.command != "old")
    );
    let mut active = log.begin(&key, ExecutionKind::Git, "active", "test");
    let output = active.output().unwrap();
    log.clear(&key);
    output.append(GitOutputStream::Stderr, b"late error");
    active.finish(ExecutionOutcome::Failed("late".into()));
    assert!(log.snapshot(&key).is_empty());
}

#[test]
fn completion_order_does_not_reorder_entries_and_callbacks_can_read() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    let notifications = Arc::new(AtomicU64::new(0));
    let observed = Arc::clone(&notifications);
    let read_log = log.clone();
    let read_key = key.clone();
    let subscription = log.subscribe(&key, move || {
        let _ = read_log.snapshot(&read_key);
        observed.fetch_add(1, Ordering::Relaxed);
    });
    let mut a = log.begin(&key, ExecutionKind::Gix, "a", "test");
    let mut b = log.begin(&key, ExecutionKind::Gix, "b", "test");
    b.finish(ExecutionOutcome::Success);
    a.finish(ExecutionOutcome::Success);
    assert_eq!(
        log.snapshot(&key)
            .iter()
            .map(|e| e.command.as_str())
            .collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(notifications.load(Ordering::Relaxed), 4);
    drop(subscription);
    log.clear(&key);
    assert_eq!(notifications.load(Ordering::Relaxed), 4);
}

#[test]
fn streamed_previews_keep_head_and_tail_with_exact_byte_counts() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    let mut guard = log.begin(&key, ExecutionKind::Git, "large", "test");
    let capture = guard.output().unwrap();
    capture.append(GitOutputStream::Stdout, &vec![b'a'; 4096]);
    capture.append(GitOutputStream::Stdout, &vec![b'b'; 100_000]);
    capture.append(GitOutputStream::Stdout, b"final diagnostic");
    guard.finish(ExecutionOutcome::Success);
    let entry = log.snapshot(&key).remove(0);
    assert!(entry.stdout.text.starts_with(&"a".repeat(4096)));
    assert!(entry.stdout.text.ends_with("final diagnostic"));
    assert_eq!(entry.stdout.total_bytes, 104_112);
    assert_eq!(entry.stdout.omitted_bytes, 104_112 - 16_384);
    assert!(entry.stdout.text.len() <= 16_384);
}

#[test]
fn previews_redact_credentials_across_reads_and_escape_binary_data() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    let guard = log.begin(&key, ExecutionKind::Git, "read", "test");
    let capture = guard.output().unwrap();
    capture.append(GitOutputStream::Stderr, b"https://user:test-");
    assert!(!log.snapshot(&key)[0].stderr.text.contains("test-"));
    capture.append(
        GitOutputStream::Stderr,
        b"password@example.invalid/repo\nAuthorization: Bearer test-token\n\x00\xff",
    );
    let entry = log.snapshot(&key).remove(0);
    assert!(!entry.stderr.text.contains("test-password"));
    assert!(!entry.stderr.text.contains("test-token"));
    assert!(entry.stderr.text.contains("example.invalid/repo"));
    assert!(entry.stderr.text.contains("\\u{0}") || entry.stderr.text.contains("\\x00"));
    assert!(entry.stderr.text.contains("\\xFF") || entry.stderr.text.contains("\\xff"));
}

#[test]
fn credentials_spanning_retention_boundaries_never_reappear_in_tail() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    for prefix in ["https://user:", "Authorization: Bearer ", "password="] {
        let mut guard = log.begin(&key, ExecutionKind::Git, "read", "test");
        let output = guard.output().unwrap();
        output.append(GitOutputStream::Stdout, prefix.as_bytes());
        for _ in 0..40 {
            output.append(GitOutputStream::Stdout, &vec![b'z'; 1000]);
        }
        output.append(
            GitOutputStream::Stdout,
            b"@example.invalid/repo\nnext line\n",
        );
        guard.finish(ExecutionOutcome::Success);
        let text = log.snapshot(&key)[0].stdout.text.clone();
        assert!(!text.contains("zzzz"), "{text}");
        assert!(text.contains("next line"));
    }
}

#[test]
fn token_only_userinfo_is_withheld_until_authority_is_complete() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    let guard = log.begin(&key, ExecutionKind::Git, "read", "test");
    let output = guard.output().unwrap();
    output.append(GitOutputStream::Stderr, b"https://test-token");
    assert!(!log.snapshot(&key)[0].stderr.text.contains("test-token"));
    output.append(GitOutputStream::Stderr, b"@example.invalid/repo\n");
    let preview = &log.snapshot(&key)[0].stderr;
    assert!(!preview.text.contains("test-token"));
    assert_eq!(preview.omitted_bytes, 0);
    assert!(!preview.truncated);
    let unfinished = log.begin(&key, ExecutionKind::Git, "unfinished", "test");
    unfinished
        .output()
        .unwrap()
        .append(GitOutputStream::Stderr, b"https://test-token");
    drop(unfinished);
    assert!(!log.snapshot(&key)[0].stderr.text.contains("test-token"));
}

#[test]
fn escaped_output_reports_display_truncation_independently_of_raw_bytes() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    let guard = log.begin(&key, ExecutionKind::Git, "binary", "test");
    guard
        .output()
        .unwrap()
        .append(GitOutputStream::Stdout, &vec![0; 4000]);
    let preview = &log.snapshot(&key)[0].stdout;
    assert_eq!(preview.total_bytes, 4000);
    assert_eq!(preview.omitted_bytes, 0);
    assert!(preview.truncated);
    assert!(preview.text.len() <= 16_384);
}

#[test]
fn redaction_flush_preserves_utf8_at_its_lookahead_boundary() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    let guard = log.begin(&key, ExecutionKind::Git, "unicode", "test");
    let text = format!("{}é{}", "a".repeat(767), "b".repeat(1000));
    guard
        .output()
        .unwrap()
        .append(GitOutputStream::Stdout, text.as_bytes());
    assert_eq!(log.snapshot(&key)[0].stdout.text, text);
}

#[test]
fn direct_completion_and_origins_are_bounded_and_redacted() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    let _scope = ExecutionContext::ui(
        context::UiSurface::Button,
        format!(
            "https://u:test-password@example.invalid/{}",
            "x".repeat(20_000)
        ),
    )
    .attach();
    let mut guard = log.begin(&key, ExecutionKind::Gix, "read", "test");
    guard.finish(ExecutionOutcome::Failed(format!(
        "https://u:test-password@example.invalid/{}",
        "e".repeat(20_000)
    )));
    let entry = log.snapshot(&key).remove(0);
    let details = entry.details();
    assert!(!details.contains("test-password"));
    assert!(entry.context.origin.to_string().len() < 1100);
    match entry.state {
        ExecutionState::Finished {
            outcome: ExecutionOutcome::Failed(detail),
            ..
        } => assert!(detail.len() <= 4096),
        other => panic!("{other:?}"),
    }
}

#[test]
fn subcommand_options_and_filenames_do_not_change_repository_identity() {
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    for args in [["diff", "-C", "--", "file"], ["add", "--", "-C", "file"]] {
        let mut command = Command::new("git");
        command.current_dir(root.path()).args(args);
        let guard = ExecutionGuard::command(&command, "test");
        assert_eq!(guard.context.repository.as_ref(), Some(&key));
    }
}

#[test]
fn concurrent_starts_are_ordered_by_their_assigned_sequence() {
    let log = GitExecutionLog::default();
    let root = tempfile::tempdir().unwrap();
    let key = RepoLogKey::new(root.path());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let log = &log;
            let key = &key;
            scope.spawn(move || {
                for _ in 0..40 {
                    drop(log.begin(key, ExecutionKind::Gix, "read", "test"));
                }
            });
        }
    });
    let entries = log.snapshot(&key);
    assert_eq!(entries.len(), 100);
    assert!(entries.windows(2).all(|pair| pair[0].id > pair[1].id));
}

#[test]
fn worker_context_restores_nested_origins_and_clears_reused_threads() {
    use context::{ExecutionContext, UiSurface};
    let a = ExecutionContext::ui(UiSurface::Button, "push");
    let b = ExecutionContext::ui(UiSurface::CommandPalette, "fetch");
    let empty = context::wrap_task(|| assert!(context::current().is_none()));
    let scope = a.attach();
    let task = context::wrap_task(|| context::current().unwrap());
    {
        let _scope = b.attach();
        assert_eq!(context::current().unwrap().action_id, b.action_id);
        empty();
        assert_eq!(context::current().unwrap().action_id, b.action_id);
    }
    assert_eq!(context::current().unwrap().action_id, a.action_id);
    drop(scope);
    let captured = std::thread::spawn(task).join().unwrap();
    assert_eq!(captured, a);
    assert!(context::current().is_none());
}
