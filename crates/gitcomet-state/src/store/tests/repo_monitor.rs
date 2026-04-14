use super::super::repo_monitor as monitor_impl;
use super::*;

#[test]
fn repo_monitor_start_failures_are_recorded_for_missing_workdir() {
    let before = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Start);

    let mut monitors = monitor_impl::RepoMonitorManager::new();
    let missing_workdir = std::env::temp_dir().join(format!(
        "gitcomet-repo-monitor-missing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&missing_workdir);
    let _ = std::fs::remove_dir_all(&missing_workdir);
    let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();

    monitors.start(
        RepoId(1),
        missing_workdir,
        msg_tx,
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
    );
    monitors.stop(RepoId(1));

    let after = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Start);
    assert_eq!(after, before + 1);
}

#[test]
fn repo_monitor_stop_send_failures_are_recorded() {
    let before = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Stop);

    monitor_impl::record_stop_send_failure(RepoId(77), "repo monitor test stop send");

    let after = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Stop);
    assert_eq!(after, before + 1);
}

#[test]
fn repo_monitor_join_failures_are_recorded() {
    let before = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Join);

    let join = std::thread::spawn(|| panic!("monitor panic test"));
    monitor_impl::join_monitor_or_log(join, RepoId(88), "repo monitor test join");

    let after = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Join);
    assert_eq!(after, before + 1);
}
