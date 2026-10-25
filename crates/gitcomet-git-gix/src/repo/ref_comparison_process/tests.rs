use super::*;

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
        empty_graph: None,
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
        empty_graph: None,
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
        empty_graph: None,
    };
    let mut command = Command::new("sh");
    command.args(["-c", "exit 1", "merge-base"]);
    assert_eq!(
        analysis.run(command, &[], |_| Ok(())),
        Err(RefComparisonReason::IncompleteHistory)
    );
}
