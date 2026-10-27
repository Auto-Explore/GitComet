use super::*;

#[cfg(unix)]
#[test]
fn native_memory_exhaustion_stops_output_and_reaps_the_process() {
    let tmp = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let owner =
        std::sync::Arc::new(gitcomet_core::ref_comparison_memory::ComparisonMemory::default());
    let _reservation = owner.reserve(16 * 1024 * 1024).unwrap();
    let limit = 8 * 1024 * 1024;
    let native = memory::NativeMemory::for_test(owner.clone(), limit);
    let analysis = Analysis {
        workdir: tmp.path(),
        deadline: Instant::now() + Duration::from_secs(5),
        cancellation: &cancellation,
        memory: 1024 * 1024,
        history_overrides: None,
        native: Some(&native),
    };
    let mut command = Command::new("sh");
    command.args(["-c", "exec awk 'BEGIN { print \"ok\" }'"]);
    let bytes = analysis
        .run(command, &[], |mut reader| {
            let mut bytes = Vec::new();
            reader
                .read_to_end(&mut bytes)
                .map_err(|_| RefComparisonReason::IncompleteHistory)?;
            Ok(bytes)
        })
        .unwrap();
    assert_eq!(bytes, b"ok\n");
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "exec awk 'BEGIN { for (i=0;i<1000000;i++) values[i]=i; print \"finished\" }'",
    ]);
    let started = Instant::now();
    let result = analysis.run(command, &[], |mut reader| {
        std::io::copy(&mut reader, &mut std::io::sink())
            .map_err(|_| RefComparisonReason::IncompleteHistory)?;
        Ok(())
    });
    assert_eq!(result, Err(RefComparisonReason::Budget));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(owner.native_peak_bytes() > limit);
}

#[cfg(unix)]
#[test]
fn elapsed_deadline_stops_a_chatty_process_and_reaps_its_pipes() {
    let tmp = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let analysis = Analysis {
        workdir: tmp.path(),
        deadline: Instant::now() + Duration::from_millis(40),
        cancellation: &cancellation,
        memory: 128 * 1024,
        history_overrides: None,
        native: None,
    };
    let mut command = Command::new("sh");
    command.args(["-c", "while :; do printf x; done"]);
    let started = Instant::now();
    let result = analysis.run(command, &[], |mut reader| {
        std::io::copy(&mut reader, &mut std::io::sink())
            .map_err(|_| RefComparisonReason::IncompleteHistory)?;
        Ok(())
    });
    assert_eq!(result, Err(RefComparisonReason::Deadline));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[cfg(unix)]
#[test]
fn elapsed_deadline_reaps_a_descendant_after_the_leader_exits() {
    let tmp = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let analysis = Analysis {
        workdir: tmp.path(),
        deadline: Instant::now() + Duration::from_millis(40),
        cancellation: &cancellation,
        memory: 128 * 1024,
        history_overrides: None,
        native: None,
    };
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30 & exit 0"]);
    let started = Instant::now();
    let result = analysis.run(command, &[], |mut reader| {
        std::io::copy(&mut reader, &mut std::io::sink())
            .map_err(|_| RefComparisonReason::IncompleteHistory)?;
        Ok(())
    });
    assert_eq!(result, Err(RefComparisonReason::Deadline));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[cfg(unix)]
#[test]
fn a_merge_base_argument_does_not_change_another_commands_exit_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let analysis = Analysis {
        workdir: tmp.path(),
        deadline: Instant::now() + Duration::from_secs(5),
        cancellation: &cancellation,
        memory: 128 * 1024,
        history_overrides: None,
        native: None,
    };
    let mut command = Command::new("sh");
    command.args(["-c", "exit 1", "merge-base"]);
    assert_eq!(
        analysis.run(command, &[], |_| Ok(())),
        Err(RefComparisonReason::IncompleteHistory)
    );
}

/// Git treats any existing shallow file, even an empty one, as a shallow
/// repository and then never reads the commit-graph. An empty override file
/// made every comparison walk on a large repository 10-80x slower.
#[test]
fn analysis_commands_keep_the_commit_graph() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    crate::test_support::init_test_repo(dir);
    crate::test_support::write_file(dir, "a", "a\n");
    crate::test_support::git_success(dir, &["add", "."]);
    crate::test_support::git_success(dir, &["commit", "-m", "a"]);
    let cancellation = CancellationToken::new();
    let overrides = HistoryOverrides::new().unwrap();
    let analysis = Analysis {
        workdir: dir,
        deadline: Instant::now() + Duration::from_secs(5),
        cancellation: &cancellation,
        memory: 128 * 1024,
        history_overrides: Some(overrides.path()),
        native: None,
    };
    let shallow = analysis
        .capture(&["rev-parse", "--is-shallow-repository"], &[])
        .unwrap();
    assert_eq!(shallow.trim_ascii(), b"false");
}

#[test]
fn history_overrides_ignore_graft_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    crate::test_support::init_test_repo(dir);
    for name in ["a", "b"] {
        crate::test_support::write_file(dir, name, "x\n");
        crate::test_support::git_success(dir, &["add", "."]);
        crate::test_support::git_success(dir, &["commit", "-m", name]);
    }
    let head = crate::util::git_workdir_cmd_for(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    // A graft that cuts HEAD off from its parent.
    std::fs::write(dir.join(".git/info/grafts"), &head.stdout).unwrap();
    let cancellation = CancellationToken::new();
    let overrides = HistoryOverrides::new().unwrap();
    let analysis = Analysis {
        workdir: dir,
        deadline: Instant::now() + Duration::from_secs(5),
        cancellation: &cancellation,
        memory: 128 * 1024,
        history_overrides: Some(overrides.path()),
        native: None,
    };
    let count = analysis
        .capture(&["rev-list", "--count", "HEAD"], &[])
        .unwrap();
    assert_eq!(count.trim_ascii(), b"2");
    let analysis = Analysis {
        history_overrides: None,
        ..analysis
    };
    let count = analysis
        .capture(&["rev-list", "--count", "HEAD"], &[])
        .unwrap();
    assert_eq!(
        count.trim_ascii(),
        b"1",
        "the graft must apply without overrides"
    );
}

/// Git maps its packs, commit-graph and the files it hashes, and those pages
/// count toward resident memory. Measuring resident memory killed walks on
/// any large repository, so the native limit counts anonymous memory only.
#[cfg(target_os = "linux")]
#[test]
fn mapped_file_pages_do_not_count_toward_the_native_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    crate::test_support::init_test_repo(dir);
    std::fs::write(dir.join("big"), vec![0x5a_u8; 32 * 1024 * 1024]).unwrap();
    let cancellation = CancellationToken::new();
    let owner =
        std::sync::Arc::new(gitcomet_core::ref_comparison_memory::ComparisonMemory::default());
    let limit = 16 * 1024 * 1024;
    let native = memory::NativeMemory::for_test(owner.clone(), limit);
    let analysis = Analysis {
        workdir: dir,
        deadline: Instant::now() + Duration::from_secs(10),
        cancellation: &cancellation,
        memory: 128 * 1024,
        history_overrides: None,
        native: Some(&native),
    };
    // hash-object mmaps files above 32 KiB and reads every page.
    let oid = analysis.capture(&["hash-object", "big"], &[]);
    assert!(oid.is_ok(), "git was stopped: {oid:?}");
    assert!(owner.native_peak_bytes() < limit);
}
