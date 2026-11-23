use super::{PreparedSyntaxChunkBuildRequest, PreparedSyntaxChunkKey};
use std::cell::RefCell;
use std::cmp::Reverse;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread::ThreadId;

fn next_sequence() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Default)]
pub struct PreparedSyntaxViewport {
    current: Option<Arc<()>>,
}

impl PreparedSyntaxViewport {
    /// Scope all token requests made while building one column's visible rows.
    pub fn begin(&mut self) -> PreparedSyntaxViewportPass {
        let next = Arc::new(());
        let current = PreparedSyntaxViewportRequest(Some((next_sequence(), Arc::downgrade(&next))));
        PreparedSyntaxViewportPass {
            previous: CURRENT.replace(current),
            // Keep overlapping work live until this pass has registered its rows.
            _previous_generation: self.current.replace(next),
            _thread: PhantomData,
        }
    }
}

thread_local! {
    static CURRENT: RefCell<PreparedSyntaxViewportRequest> = const { RefCell::new(PreparedSyntaxViewportRequest(None)) };
}

#[derive(Clone, Debug, Default)]
pub struct PreparedSyntaxViewportRequest(Option<(u64, Weak<()>)>);

impl PreparedSyntaxViewportRequest {
    pub fn current() -> Self {
        CURRENT.with_borrow(Clone::clone)
    }

    pub(super) fn current_sequence() -> Option<u64> {
        CURRENT.with_borrow(Self::sequence)
    }

    /// Streamed rows request tokens at paint time, after the row builder returns.
    pub fn enter(&self) -> PreparedSyntaxViewportPass {
        PreparedSyntaxViewportPass {
            previous: CURRENT.replace(self.clone()),
            _previous_generation: None,
            _thread: PhantomData,
        }
    }

    pub(super) fn sequence(&self) -> Option<u64> {
        self.0.as_ref().map(|(sequence, _)| *sequence)
    }

    fn is_live(&self) -> bool {
        self.0
            .as_ref()
            .is_none_or(|(_, owner)| owner.strong_count() != 0)
    }
}

#[must_use]
pub struct PreparedSyntaxViewportPass {
    previous: PreparedSyntaxViewportRequest,
    _previous_generation: Option<Arc<()>>,
    // Restores the thread-local scope on the same thread that entered it.
    _thread: PhantomData<Rc<()>>,
}

impl Drop for PreparedSyntaxViewportPass {
    fn drop(&mut self) {
        CURRENT.set(std::mem::take(&mut self.previous));
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ChunkPriority {
    Prefetch,
    Visible,
}

pub(super) enum ChunkCommand {
    Build(
        PreparedSyntaxChunkBuildRequest,
        PreparedSyntaxViewportRequest,
        ChunkPriority,
    ),
    Subscribe(
        PreparedSyntaxChunkKey,
        ThreadId,
        PreparedSyntaxViewportRequest,
        ChunkPriority,
    ),
}

struct Interest {
    viewport: PreparedSyntaxViewportRequest,
    sequence: u64,
    priority: ChunkPriority,
}

struct WaitingChunk {
    request: PreparedSyntaxChunkBuildRequest,
    interests: Vec<Interest>,
    queued_rank: Option<(ChunkPriority, u64)>,
    insertion: u64,
}

impl WaitingChunk {
    fn subscribe(&mut self, viewport: PreparedSyntaxViewportRequest, priority: ChunkPriority) {
        self.interests
            .retain(|interest| interest.viewport.is_live());
        if let Some(interest) = self
            .interests
            .iter_mut()
            .find(|interest| interest.viewport.sequence() == viewport.sequence())
        {
            interest.priority = interest.priority.max(priority);
        } else {
            let sequence = viewport.sequence().unwrap_or_else(next_sequence);
            self.interests.push(Interest {
                viewport,
                sequence,
                priority,
            });
        }
    }

    fn rank(&self) -> Option<(ChunkPriority, u64)> {
        self.interests
            .iter()
            .filter(|interest| interest.viewport.is_live())
            .map(|interest| (interest.priority, interest.sequence))
            .max()
    }

    fn order(&self) -> (bool, Option<(ChunkPriority, u64)>, Reverse<u64>) {
        (
            self.queued_rank.is_none(),
            self.queued_rank,
            Reverse(self.insertion),
        )
    }
}

#[derive(Default)]
pub(super) struct ChunkQueue {
    pending: Vec<WaitingChunk>,
    changed: bool,
    pops_since_oldest_visible: u8,
}

impl ChunkQueue {
    pub(super) fn apply(&mut self, command: ChunkCommand) {
        match command {
            ChunkCommand::Build(request, viewport, priority) => {
                self.push(request, viewport, priority)
            }
            ChunkCommand::Subscribe(key, thread, viewport, priority) => {
                self.subscribe(key, thread, viewport, priority)
            }
        }
    }

    fn push(
        &mut self,
        request: PreparedSyntaxChunkBuildRequest,
        viewport: PreparedSyntaxViewportRequest,
        priority: ChunkPriority,
    ) {
        let mut chunk = WaitingChunk {
            request,
            interests: Vec::new(),
            queued_rank: None,
            insertion: next_sequence(),
        };
        chunk.subscribe(viewport, priority);
        self.pending.push(chunk);
        self.changed = true;
    }

    fn subscribe(
        &mut self,
        key: PreparedSyntaxChunkKey,
        thread: ThreadId,
        viewport: PreparedSyntaxViewportRequest,
        priority: ChunkPriority,
    ) {
        if let Some(chunk) = self
            .pending
            .iter_mut()
            .find(|chunk| chunk.request.chunk_key == key && chunk.request.thread_id == thread)
        {
            chunk.subscribe(viewport, priority);
            self.changed = true;
        }
    }

    pub(super) fn take(&mut self) -> Option<(PreparedSyntaxChunkBuildRequest, bool)> {
        if self.pops_since_oldest_visible == 8 {
            self.pops_since_oldest_visible = 0;
            // Keep a stationary window moving during sustained scrolling in another.
            let oldest_visible = self
                .pending
                .iter()
                .enumerate()
                .filter(|(_, chunk)| matches!(chunk.rank(), Some((ChunkPriority::Visible, _))))
                .min_by_key(|(_, chunk)| chunk.insertion)
                .map(|(at, _)| at);
            if let Some(at) = oldest_visible {
                return Some((self.pending.remove(at).request, true));
            }
        }
        if std::mem::take(&mut self.changed) {
            for chunk in &mut self.pending {
                chunk.queued_rank = chunk.rank();
            }
            // Sort once for the whole input batch. Retiring a burst of stale
            // chunks then costs one pop per acknowledgment.
            self.pending.sort_unstable_by_key(WaitingChunk::order);
        }
        while let Some(mut chunk) = self.pending.pop() {
            let rank = chunk.rank();
            if rank.is_none() || rank == chunk.queued_rank {
                self.pops_since_oldest_visible += 1;
                return Some((chunk.request, rank.is_some()));
            }
            // One owner left since sorting, but another still needs this chunk.
            chunk.queued_rank = rank;
            let position = self
                .pending
                .partition_point(|other| other.order() < chunk.order());
            self.pending.insert(position, chunk);
        }
        self.pops_since_oldest_visible = 0;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiffSyntaxLanguage, PreparedSyntaxDocument, prepared_document_tree_state};

    #[test]
    fn a_scroll_stream_leaves_time_for_another_visible_window() {
        let doc = crate::tests::prepare_test_document(DiffSyntaxLanguage::Rust, "fn test() {}");
        let request = |chunk_ix| {
            ChunkCommand::Build(
                PreparedSyntaxChunkBuildRequest {
                    chunk_key: PreparedSyntaxChunkKey {
                        cache_key: doc.cache_key,
                        chunk_ix,
                    },
                    line_count: 256,
                    tree_state: Arc::new(prepared_document_tree_state(doc).unwrap()),
                    thread_id: std::thread::current().id(),
                },
                PreparedSyntaxViewportRequest::current(),
                ChunkPriority::Visible,
            )
        };
        let mut queue = ChunkQueue::default();
        let mut idle_window = PreparedSyntaxViewport::default();
        let mut scrolling_window = PreparedSyntaxViewport::default();
        {
            let _pass = idle_window.begin();
            queue.apply(request(0));
        }
        for ix in 1..=9 {
            let _pass = scrolling_window.begin();
            queue.apply(request(ix));
            let (next, wanted) = queue.take().unwrap();
            assert!(wanted);
            if next.chunk_key.chunk_ix == 0 {
                return;
            }
        }
        panic!("the stationary window is still waiting after nine visible chunks");
    }

    #[test]
    fn latest_visible_work_precedes_other_windows_and_prefetch_and_preserves_shared_interests() {
        let file = crate::tests::prepare_test_document(DiffSyntaxLanguage::Rust, "fn first() {}");
        let other_file =
            crate::tests::prepare_test_document(DiffSyntaxLanguage::Rust, "fn second() {}");
        let thread = std::thread::current().id();
        let key = |doc: PreparedSyntaxDocument, chunk_ix| PreparedSyntaxChunkKey {
            cache_key: doc.cache_key,
            chunk_ix,
        };
        let build = |doc, chunk_ix, priority| {
            ChunkCommand::Build(
                PreparedSyntaxChunkBuildRequest {
                    chunk_key: key(doc, chunk_ix),
                    line_count: 256,
                    tree_state: Arc::new(prepared_document_tree_state(doc).unwrap()),
                    thread_id: thread,
                },
                PreparedSyntaxViewportRequest::current(),
                priority,
            )
        };
        let subscribe = |chunk_ix, priority| {
            ChunkCommand::Subscribe(
                key(file, chunk_ix),
                thread,
                PreparedSyntaxViewportRequest::current(),
                priority,
            )
        };
        let mut queue = ChunkQueue::default();
        let mut first = PreparedSyntaxViewport::default();
        let mut second = PreparedSyntaxViewport::default();
        {
            let _pass = first.begin();
            queue.apply(build(file, 0, ChunkPriority::Prefetch));
            queue.apply(build(file, 1, ChunkPriority::Visible));
            queue.apply(build(file, 2, ChunkPriority::Visible));
        }
        {
            let _pass = second.begin();
            queue.apply(build(other_file, 0, ChunkPriority::Visible));
            queue.apply(subscribe(1, ChunkPriority::Prefetch));
        }
        {
            let _pass = first.begin();
            queue.apply(subscribe(0, ChunkPriority::Visible));
            queue.apply(build(file, 3, ChunkPriority::Prefetch));
        }
        let actual: Vec<_> = std::iter::from_fn(|| queue.take())
            .map(|(request, wanted)| (request.chunk_key, wanted))
            .collect();
        assert_eq!(
            actual,
            [
                (key(file, 2), false),
                (key(file, 0), true),
                (key(other_file, 0), true),
                (key(file, 3), true),
                (key(file, 1), true),
            ]
        );
    }
}
