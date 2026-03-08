use crate::msg::StoreEvent;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SendFailureKind {
    StoreDispatch,
    ExecutorQueue,
    EffectMessage,
    RepoMonitorMessage,
    StoreEvent,
}

static STORE_DISPATCH_FAILURES: AtomicU64 = AtomicU64::new(0);
static EXECUTOR_QUEUE_FAILURES: AtomicU64 = AtomicU64::new(0);
static EFFECT_MESSAGE_FAILURES: AtomicU64 = AtomicU64::new(0);
static REPO_MONITOR_MESSAGE_FAILURES: AtomicU64 = AtomicU64::new(0);
static STORE_EVENT_FAILURES: AtomicU64 = AtomicU64::new(0);

fn failure_counter(kind: SendFailureKind) -> &'static AtomicU64 {
    match kind {
        SendFailureKind::StoreDispatch => &STORE_DISPATCH_FAILURES,
        SendFailureKind::ExecutorQueue => &EXECUTOR_QUEUE_FAILURES,
        SendFailureKind::EffectMessage => &EFFECT_MESSAGE_FAILURES,
        SendFailureKind::RepoMonitorMessage => &REPO_MONITOR_MESSAGE_FAILURES,
        SendFailureKind::StoreEvent => &STORE_EVENT_FAILURES,
    }
}

fn record_send_failure(kind: SendFailureKind, context: &'static str) {
    let count = failure_counter(kind).fetch_add(1, Ordering::Relaxed) + 1;
    eprintln!(
        "gitcomet-state: channel send failed ({kind:?}) in {context}; total_failures={count}"
    );
}

pub(super) fn send_or_log<T>(
    tx: &mpsc::Sender<T>,
    message: T,
    kind: SendFailureKind,
    context: &'static str,
) {
    match tx.send(message) {
        Ok(()) => {}
        Err(_) => {
            record_send_failure(kind, context);
        }
    }
}

pub(super) fn try_send_state_changed_or_log(
    tx: &smol::channel::Sender<StoreEvent>,
    context: &'static str,
) {
    match tx.try_send(StoreEvent::StateChanged) {
        Ok(()) | Err(smol::channel::TrySendError::Full(_)) => {}
        Err(smol::channel::TrySendError::Closed(_)) => {
            record_send_failure(SendFailureKind::StoreEvent, context);
        }
    }
}

#[cfg(test)]
pub(super) fn send_failure_count(kind: SendFailureKind) -> u64 {
    failure_counter(kind).load(Ordering::Relaxed)
}
