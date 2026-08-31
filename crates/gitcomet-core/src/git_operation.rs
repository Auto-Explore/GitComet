//! Per-user-operation context for observing Git subprocess activity.
//!
//! The context is deliberately thread-local: repository work is dispatched to a
//! worker thread, and every Git command started while the scope is attached
//! belongs to that one user action.  The event sink itself is shared so the
//! stdout/stderr and Trace2 reader threads can report live progress too.

use crate::services::CancellationToken;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GitOperationId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitOutputStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HookExecutionId {
    pub sid: Arc<str>,
    pub child_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitOutputChunk {
    pub stream: GitOutputStream,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitOperationEvent {
    Output {
        chunks: Vec<GitOutputChunk>,
    },
    HookStarted {
        id: HookExecutionId,
        name: String,
    },
    HookFinished {
        id: HookExecutionId,
        name: String,
        exit_code: Option<i32>,
        duration: Duration,
    },
}

type EventSink = dyn Fn(GitOperationId, GitOperationEvent) + Send + Sync + 'static;

struct GitOperationInner {
    id: GitOperationId,
    label: Arc<str>,
    cancellation: CancellationToken,
    sink: Arc<EventSink>,
}

impl Drop for GitOperationInner {
    fn drop(&mut self) {
        operation_tokens()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
    }
}

#[derive(Clone)]
pub struct GitOperationContext {
    inner: Arc<GitOperationInner>,
}

impl fmt::Debug for GitOperationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitOperationContext")
            .field("id", &self.id())
            .field("label", &self.label())
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl GitOperationContext {
    pub fn new(
        label: impl Into<Arc<str>>,
        sink: impl Fn(GitOperationId, GitOperationEvent) + Send + Sync + 'static,
    ) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);

        let id = GitOperationId(NEXT_ID.fetch_add(1, Ordering::Relaxed).max(1));
        let cancellation = CancellationToken::new();
        operation_tokens()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id, cancellation.clone());

        Self {
            inner: Arc::new(GitOperationInner {
                id,
                label: label.into(),
                cancellation,
                sink: Arc::new(sink),
            }),
        }
    }

    pub fn id(&self) -> GitOperationId {
        self.inner.id
    }

    pub fn label(&self) -> &str {
        &self.inner.label
    }

    pub fn cancellation(&self) -> &CancellationToken {
        &self.inner.cancellation
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancellation.is_cancelled()
    }

    pub fn emit(&self, event: GitOperationEvent) {
        (self.inner.sink)(self.id(), event);
    }
}

fn operation_tokens() -> &'static Mutex<HashMap<GitOperationId, CancellationToken>> {
    static TOKENS: LazyLock<Mutex<HashMap<GitOperationId, CancellationToken>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    &TOKENS
}

/// Requests cancellation of a live user operation. Returns `false` when the
/// operation already completed (and therefore left the registry).
pub fn cancel(id: GitOperationId) -> bool {
    let token = operation_tokens()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&id)
        .cloned();
    if let Some(token) = token {
        token.cancel();
        true
    } else {
        false
    }
}

thread_local! {
    static CURRENT_OPERATION: RefCell<Option<GitOperationContext>> = const { RefCell::new(None) };
}

pub struct GitOperationScope {
    previous: Option<GitOperationContext>,
}

impl Drop for GitOperationScope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT_OPERATION.with(|slot| {
            slot.replace(previous);
        });
    }
}

pub fn attach(context: &GitOperationContext) -> GitOperationScope {
    let previous = CURRENT_OPERATION.with(|slot| slot.replace(Some(context.clone())));
    GitOperationScope { previous }
}

pub fn current() -> Option<GitOperationContext> {
    CURRENT_OPERATION.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn attach_restores_the_previous_context() {
        let outer = GitOperationContext::new("outer", |_, _| {});
        let inner = GitOperationContext::new("inner", |_, _| {});
        let _outer = attach(&outer);
        assert_eq!(current().map(|context| context.id()), Some(outer.id()));
        {
            let _inner = attach(&inner);
            assert_eq!(current().map(|context| context.id()), Some(inner.id()));
        }
        assert_eq!(current().map(|context| context.id()), Some(outer.id()));
    }

    #[test]
    fn sink_receives_events_and_registry_cancels_live_operation() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let context = GitOperationContext::new("commit", move |_, event| {
            captured
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(event);
        });
        context.emit(GitOperationEvent::Output {
            chunks: vec![GitOutputChunk {
                stream: GitOutputStream::Stdout,
                text: "checking\n".to_string(),
            }],
        });
        assert!(cancel(context.id()));
        assert!(context.is_cancelled());
        assert_eq!(events.lock().unwrap().len(), 1);
    }
}
