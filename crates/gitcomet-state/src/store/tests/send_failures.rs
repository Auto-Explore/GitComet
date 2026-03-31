use super::*;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, mpsc};
use std::time::{Duration, Instant};

#[test]
fn dispatch_increments_failure_counter_when_channel_is_disconnected() {
    let before = super::send_diagnostics::send_failure_count(
        super::send_diagnostics::SendFailureKind::StoreDispatch,
    );

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    drop(msg_rx);

    let store = AppStore {
        state: Arc::new(RwLock::new(Arc::new(AppState::default()))),
        msg_tx,
    };

    store.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/repo")));

    let after = super::send_diagnostics::send_failure_count(
        super::send_diagnostics::SendFailureKind::StoreDispatch,
    );
    assert!(after > before);
}

#[test]
fn executor_increments_failure_counter_when_worker_queue_disconnects() {
    let before = super::send_diagnostics::send_failure_count(
        super::send_diagnostics::SendFailureKind::ExecutorQueue,
    );

    let executor = super::executor::TaskExecutor::new(1);
    let (started_tx, started_rx) = mpsc::channel::<()>();
    executor.spawn(move || {
        let _ = started_tx.send(());
        panic!("intentional panic to drop executor worker");
    });

    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("worker task did not start");

    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        // The worker panic may race with this test thread; keep attempting to enqueue
        // until the sender observes the disconnected queue and diagnostics increment.
        executor.spawn(|| {});

        let after = super::send_diagnostics::send_failure_count(
            super::send_diagnostics::SendFailureKind::ExecutorQueue,
        );
        if after > before {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "expected executor queue send failure count to increase"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn effect_message_send_increments_failure_counter_when_disconnected() {
    let before = super::send_diagnostics::send_failure_count(
        super::send_diagnostics::SendFailureKind::EffectMessage,
    );

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    drop(msg_rx);

    super::send_diagnostics::send_or_log(
        &msg_tx,
        Msg::RefreshBranches { repo_id: RepoId(1) },
        super::send_diagnostics::SendFailureKind::EffectMessage,
        "test effect pipeline send",
    );

    let after = super::send_diagnostics::send_failure_count(
        super::send_diagnostics::SendFailureKind::EffectMessage,
    );
    assert!(after > before);
}

#[test]
fn store_event_send_increments_failure_counter_when_receiver_closed() {
    let before = super::send_diagnostics::send_failure_count(
        super::send_diagnostics::SendFailureKind::StoreEvent,
    );

    let (event_tx, event_rx) = smol::channel::bounded::<StoreEvent>(1);
    drop(event_rx);

    super::send_diagnostics::try_send_state_changed_or_log(&event_tx, "test state event send");

    let after = super::send_diagnostics::send_failure_count(
        super::send_diagnostics::SendFailureKind::StoreEvent,
    );
    assert!(after > before);
}
