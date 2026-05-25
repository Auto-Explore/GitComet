use crate::msg::Msg;
use gitcomet_core::services::CancellationToken;
#[cfg(any(test, feature = "test-support"))]
use gitcomet_core::services::GitRepository;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use super::RepoId;
use super::repo_load_trace;
use super::send_diagnostics::{self, SendFailureKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StoreInstanceId(u64);

impl StoreInstanceId {
    pub(super) fn next() -> Self {
        static NEXT_STORE_ID: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_STORE_ID.fetch_add(1, Ordering::Relaxed))
    }

    pub(super) fn get(self) -> u64 {
        self.0
    }
}

pub(super) enum StoreWorkerCommand {
    Msg(Box<Msg>),
    Shutdown,
    #[cfg(any(test, feature = "test-support"))]
    InsertRepoForTest {
        repo_id: RepoId,
        repo: Arc<dyn GitRepository>,
    },
}

#[derive(Clone)]
enum StoreWorkerSenderInner {
    Command(mpsc::Sender<StoreWorkerCommand>),
    #[cfg(test)]
    MsgForTest(mpsc::Sender<Msg>),
}

#[derive(Clone)]
pub(super) struct StoreWorkerSender {
    inner: StoreWorkerSenderInner,
    alive: Arc<AtomicBool>,
    store_id: StoreInstanceId,
    repo_load_guard: Option<RepoLoadGuard>,
    cancellation: Option<CancellationToken>,
}

#[derive(Clone)]
struct RepoLoadGuard {
    repo_id: RepoId,
    load_epoch: u64,
}

impl StoreWorkerSender {
    pub(super) fn new(
        tx: mpsc::Sender<StoreWorkerCommand>,
        alive: Arc<AtomicBool>,
        store_id: StoreInstanceId,
    ) -> Self {
        Self {
            inner: StoreWorkerSenderInner::Command(tx),
            alive,
            store_id,
            repo_load_guard: None,
            cancellation: None,
        }
    }

    #[cfg(test)]
    pub(super) fn for_test_msg_sender(tx: mpsc::Sender<Msg>) -> Self {
        Self {
            inner: StoreWorkerSenderInner::MsgForTest(tx),
            alive: Arc::new(AtomicBool::new(true)),
            store_id: StoreInstanceId(0),
            repo_load_guard: None,
            cancellation: None,
        }
    }

    pub(super) fn store_id(&self) -> StoreInstanceId {
        self.store_id
    }

    pub(super) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }

    pub(super) fn with_repo_load_guard(
        &self,
        repo_id: RepoId,
        load_epoch: u64,
        cancellation: CancellationToken,
    ) -> Self {
        let mut guarded = self.clone();
        guarded.repo_load_guard = Some(RepoLoadGuard {
            repo_id,
            load_epoch,
        });
        guarded.cancellation = Some(cancellation);
        guarded
    }

    pub(super) fn dispatch(&self, msg: Msg) {
        self.send_or_log(
            msg,
            SendFailureKind::StoreDispatch,
            "AppStore::dispatch",
            false,
        );
    }

    pub(super) fn send_effect_or_log(&self, msg: Msg, context: &'static str) {
        if self.is_cancelled() {
            repo_load_trace::trace!(
                "suppress_effect_message_cancelled msg={} context={}",
                repo_load_trace::msg_name(&msg),
                context
            );
            return;
        }
        repo_load_trace::trace!(
            "send_effect_message msg={} context={}",
            repo_load_trace::msg_name(&msg),
            context
        );
        let msg = self.wrap_effect_message(msg);
        self.send_or_log(msg, SendFailureKind::EffectMessage, context, true);
    }

    pub(super) fn send_repo_monitor_or_log(&self, msg: Msg, context: &'static str) {
        self.send_or_log(msg, SendFailureKind::RepoMonitorMessage, context, true);
    }

    fn wrap_effect_message(&self, msg: Msg) -> Msg {
        #[cfg(test)]
        if matches!(&self.inner, StoreWorkerSenderInner::MsgForTest(_)) {
            return msg;
        }

        let Some(guard) = &self.repo_load_guard else {
            return msg;
        };
        match msg {
            Msg::Internal(message) => match message {
                crate::msg::InternalMsg::RepoLoadFinished { .. } => Msg::Internal(message),
                message => {
                    repo_load_trace::trace!(
                        "wrap_repo_load_message repo_id={:?} load_epoch={} inner={}",
                        guard.repo_id,
                        guard.load_epoch,
                        repo_load_trace::internal_msg_name(&message)
                    );
                    Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
                        repo_id: guard.repo_id,
                        load_epoch: guard.load_epoch,
                        message: Box::new(message),
                    })
                }
            },
            msg => msg,
        }
    }

    fn send_or_log(
        &self,
        msg: Msg,
        kind: SendFailureKind,
        context: &'static str,
        suppress_after_shutdown: bool,
    ) {
        if suppress_after_shutdown && !self.is_alive() {
            return;
        }

        match &self.inner {
            StoreWorkerSenderInner::Command(tx) => send_diagnostics::send_or_log(
                tx,
                StoreWorkerCommand::Msg(Box::new(msg)),
                kind,
                context,
            ),
            #[cfg(test)]
            StoreWorkerSenderInner::MsgForTest(tx) => {
                send_diagnostics::send_or_log(tx, msg, kind, context)
            }
        }
    }

    pub(super) fn shutdown(&self) {
        if !self.alive.swap(false, Ordering::AcqRel) {
            return;
        }

        match &self.inner {
            StoreWorkerSenderInner::Command(tx) => {
                let _ = tx.send(StoreWorkerCommand::Shutdown);
            }
            #[cfg(test)]
            StoreWorkerSenderInner::MsgForTest(_) => {}
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn insert_repo_for_test(&self, repo_id: RepoId, repo: Arc<dyn GitRepository>) {
        if !self.is_alive() {
            return;
        }

        match &self.inner {
            StoreWorkerSenderInner::Command(tx) => {
                let _ = tx.send(StoreWorkerCommand::InsertRepoForTest { repo_id, repo });
            }
            #[cfg(test)]
            StoreWorkerSenderInner::MsgForTest(_) => {}
        }
    }
}
