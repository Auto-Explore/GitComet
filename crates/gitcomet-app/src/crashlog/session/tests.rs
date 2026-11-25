use super::{ConfirmedCrash, CrashCause, SessionRecord, SessionState, marker_path, recover};
use gitcomet_core::environment::EnvironmentSnapshot;
use gitcomet_core::fs_utils::write_private_file;
use gitcomet_core::startup_exit::{SessionToken, StartupExitReason};

fn fixture(pid: u32, started_unix_ms: u64) -> SessionRecord {
    SessionRecord {
        pid,
        token: SessionToken::new_v4(),
        started_unix_ms,
        version: "0.3.5".into(),
        identity: None,
        state: SessionState::Unfinished,
    }
}

#[test]
fn recovery_keeps_the_newer_crash_after_acknowledging_the_older_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut older = fixture(101, 1_700_000_000_000);
    let mut newer = fixture(102, 1_700_000_001_000);
    for record in [&mut older, &mut newer] {
        record.state = SessionState::Confirmed(Box::new(ConfirmedCrash {
            timestamp_unix_ms: record.started_unix_ms + 2_000,
            cause: CrashCause::NativeFault {
                description: "Access violation".into(),
                time_is_event: true,
            },
            environment: Some(EnvironmentSnapshot {
                git_version: Some("git from the crashed process".into()),
                ..Default::default()
            }),
            operation: None,
        }));
        write_private_file(
            &marker_path(dir.path(), record.pid, record.token),
            &record.encode().unwrap(),
        )
        .unwrap();
    }
    let find_next = || recover(dir.path(), |_, _| panic!("already confirmed")).unwrap();
    let first = find_next().unwrap();
    assert_eq!(first.snapshot.session_token(), Some(older.token));
    assert_eq!(first.snapshot.reason(), StartupExitReason::NativeFault);
    assert!(first.summary.contains("OS event recorded"));
    assert!(
        first
            .issue_url
            .contains("git%20from%20the%20crashed%20process")
    );
    first.snapshot.acknowledge().unwrap();
    let second = find_next().unwrap();
    assert_eq!(second.snapshot.session_token(), Some(newer.token));
    second.snapshot.acknowledge().unwrap();
    assert!(find_next().is_none());
}

#[test]
fn recovery_checks_an_unfinished_dead_session_once_and_leaves_live_sessions_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut dead = fixture(std::process::id(), 1_700_000_000_000);
    dead.identity = super::native::current();
    let live = fixture(dead.pid, 1_700_000_001_000);
    let dead_path = marker_path(dir.path(), dead.pid, dead.token);
    let live_path = marker_path(dir.path(), live.pid, live.token);
    write_private_file(&dead_path, &dead.encode().unwrap()).unwrap();
    write_private_file(&live_path, &live.encode().unwrap()).unwrap();
    let _live_lease = super::lock_private_file(&live_path.with_extension("lease")).unwrap();
    let calls = std::cell::Cell::new(0);
    let recover = || {
        recover(dir.path(), |_, _| {
            calls.set(calls.get() + 1);
            None
        })
        .unwrap()
    };
    assert!(recover().is_none());
    assert!(recover().is_none());
    assert_eq!(calls.get(), usize::from(dead.identity.is_some()));
    assert!(live_path.exists());
    assert!(!dead_path.exists());
    let manual = std::fs::read_to_string(dir.path().join("latest-session.log")).unwrap();
    assert!(manual.contains("Session started (Unix ms): 1700000000000"));
}

#[test]
fn a_failed_manual_diagnostic_does_not_hide_a_confirmed_crash() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("latest-session.log")).unwrap();
    let unfinished = fixture(101, 1_700_000_000_000);
    let mut crashed = fixture(102, 1_700_000_001_000);
    crashed.state = SessionState::Confirmed(Box::new(ConfirmedCrash {
        timestamp_unix_ms: 1_700_000_002_000,
        cause: CrashCause::NativeFault {
            description: "Access violation".into(),
            time_is_event: false,
        },
        environment: None,
        operation: None,
    }));
    for record in [&unfinished, &crashed] {
        write_private_file(
            &marker_path(dir.path(), record.pid, record.token),
            &record.encode().unwrap(),
        )
        .unwrap();
    }
    let report = recover(dir.path(), |_, _| None).unwrap().unwrap();
    assert_eq!(report.snapshot.session_token(), Some(crashed.token));
    assert!(marker_path(dir.path(), unfinished.pid, unfinished.token).exists());
}
