use super::*;
use crate::view::caches::HistoryListRow;
use rustc_hash::FxHashMap as HashMap;
use std::time::{Duration, Instant};

const SKELETON_DELAY: Duration = Duration::from_millis(300);
const STATUS_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum RowIdentity {
    #[cfg(test)]
    Commit(CommitId),
    Indexed(usize),
    Worktree(PathBuf),
}

#[derive(Default)]
pub(in crate::view) struct HistoryLoading {
    query: Option<(RepoId, LogScope, Option<String>)>,
    presentation: usize,
    missing: HashMap<RowIdentity, Instant>,
    rows: HashMap<usize, Instant>,
    spare_missing: HashMap<RowIdentity, Instant>,
    initial_since: Option<Instant>,
    busy_since: Option<Instant>,
    background_busy: bool,
    deadline: Option<Instant>,
    timer: Option<gpui::Task<()>>,
    generation: u64,
}

impl HistoryLoading {
    pub fn skeleton_visible(&self, row: usize, now: Instant) -> bool {
        self.rows
            .get(&row)
            .is_some_and(|since| now >= *since + SKELETON_DELAY)
    }

    pub fn initial_skeleton_visible(&self, now: Instant) -> bool {
        self.initial_since
            .is_some_and(|since| now >= since + SKELETON_DELAY)
    }

    pub fn status_visible(&self, now: Instant) -> bool {
        self.busy_since
            .is_some_and(|since| now >= since + STATUS_DELAY)
    }

    fn sync_rows(&mut self, rows: Vec<(usize, RowIdentity)>, now: Instant) {
        std::mem::swap(&mut self.missing, &mut self.spare_missing);
        self.missing.clear();
        let old = &self.spare_missing;
        self.rows.clear();
        for (row, id) in rows {
            let since = old.get(&id).copied().unwrap_or(now);
            self.rows.insert(row, since);
            self.missing.insert(id, since);
        }
        self.sync_busy(now);
    }

    fn sync_busy(&mut self, now: Instant) {
        if self.background_busy || self.initial_since.is_some() || !self.missing.is_empty() {
            self.busy_since.get_or_insert(now);
        } else {
            self.busy_since = None;
        }
    }

    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.missing
            .values()
            .copied()
            .chain(self.initial_since)
            .map(|since| since + SKELETON_DELAY)
            .chain(self.busy_since.map(|since| since + STATUS_DELAY))
            .filter(|deadline| *deadline > now)
            .min()
    }
}

impl HistoryView {
    /// An empty result and an as-yet unpainted first page are different states.
    pub(super) fn history_initial_loading(&self) -> bool {
        self.indexed.presentation.is_none()
            && self.history_cache.is_none()
            && self.active_repo().is_some_and(|repo| match &repo.log {
                Loadable::NotLoaded | Loadable::Loading => true,
                Loadable::Ready(page) => !page.commits.is_empty(),
                Loadable::Error(_) => false,
            })
    }

    pub(super) fn sync_history_loading(&mut self, cx: &mut gpui::Context<Self>) {
        let now = cx.background_executor().now();
        let query = self.active_repo().map(|repo| {
            (
                repo.id,
                repo.history_state.history_scope,
                repo.history_state.history_author_filter.clone(),
            )
        });
        if self.loading.query != query {
            self.loading = HistoryLoading {
                query,
                ..Default::default()
            };
        }
        let presentation = self
            .indexed
            .presentation
            .as_ref()
            .map_or(0, |shown| Arc::as_ptr(shown) as usize);
        if self.loading.presentation != presentation {
            self.loading.presentation = presentation;
            self.loading.missing.clear();
            self.loading.rows.clear();
        }
        if self.history_initial_loading() {
            self.loading.initial_since.get_or_insert(now);
        } else {
            self.loading.initial_since = None;
        }
        self.loading.background_busy = self
            .active_repo()
            .is_some_and(|repo| repo.history_state.indexed.loading)
            || self.indexed_is_building();
        self.loading.sync_busy(now);
        self.arm_history_loading_timer(cx);
    }

    pub(in crate::view) fn sync_history_loading_rows(
        &mut self,
        range: Range<usize>,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut missing = Vec::new();
        if let Some(shown) = &self.indexed.presentation {
            let visible_end =
                self.scroll_interaction
                    .borrow()
                    .logical
                    .as_ref()
                    .map_or(range.end, |logical| {
                        logical.top
                            + ((logical.viewport + logical.within) / logical.height).ceil() as usize
                    });
            for row in range {
                if row >= visible_end {
                    break;
                }
                let (visible_ix, worktree_ix) = match self.indexed.plan.row_at(row) {
                    Some(HistoryListRow::Commit { visible_ix }) => (visible_ix, None),
                    Some(HistoryListRow::WorktreeUncommitted {
                        visible_ix,
                        worktree_ix,
                    }) => (visible_ix, Some(worktree_ix)),
                    _ => continue,
                };
                let loaded = self.indexed.window.as_ref().is_some_and(|window| {
                    visible_ix.checked_sub(window.start).is_some_and(|ix| {
                        if worktree_ix.is_some() {
                            window.cache.base.graph_rows.get(ix).is_some()
                        } else {
                            window.loaded.get(ix).copied().unwrap_or(false)
                        }
                    })
                });
                if loaded {
                    continue;
                }
                let failed = shown
                    .graph
                    .projection
                    .raw_position(visible_ix)
                    .is_some_and(|raw| {
                        let block = raw / gitcomet_core::history_index::HISTORY_BLOCK_SIZE
                            * gitcomet_core::history_index::HISTORY_BLOCK_SIZE;
                        self.active_repo().is_some_and(|repo| {
                            repo.history_state.indexed.range_errors.contains_key(&block)
                        })
                    });
                if !failed {
                    let identity = match worktree_ix {
                        Some(ix) => {
                            let Some(worktree) = self.indexed.worktrees.get(ix) else {
                                continue;
                            };
                            RowIdentity::Worktree(worktree.path.clone())
                        }
                        None => RowIdentity::Indexed(visible_ix),
                    };
                    missing.push((row, identity));
                }
            }
        }
        self.loading
            .sync_rows(missing, cx.background_executor().now());
        self.arm_history_loading_timer(cx);
    }

    fn arm_history_loading_timer(&mut self, cx: &mut gpui::Context<Self>) {
        let now = cx.background_executor().now();
        let deadline = self.loading.next_deadline(now);
        if self.loading.deadline == deadline {
            return;
        }
        self.loading.timer = None;
        self.loading.deadline = deadline;
        self.loading.generation = self.loading.generation.wrapping_add(1);
        let generation = self.loading.generation;
        if let Some(deadline) = deadline {
            let timer = cx
                .background_executor()
                .timer(deadline.saturating_duration_since(now));
            self.loading.timer = Some(cx.spawn(async move |view, cx| {
                timer.await;
                let _ = view.update(cx, |this, cx| {
                    if this.loading.generation == generation
                        && this.loading.deadline == Some(deadline)
                    {
                        this.loading.deadline = None;
                        cx.notify();
                    }
                });
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> RowIdentity {
        RowIdentity::Commit(CommitId(id.into()))
    }

    #[test]
    fn history_skeleton_delay_never_holds_ready_content() {
        let start = Instant::now();
        for elapsed in [50, 299, 300, 301, 2000] {
            let mut state = HistoryLoading::default();
            state.sync_rows(vec![(4, row("a"))], start);
            let now = start + Duration::from_millis(elapsed);
            assert_eq!(state.skeleton_visible(4, now), elapsed >= 300);
            state.sync_rows(Vec::new(), now);
            assert!(!state.skeleton_visible(4, now));
            assert!(!state.status_visible(now));
            assert_eq!(state.next_deadline(now), None);
        }
    }

    #[test]
    fn history_skeleton_deadlines_follow_visible_identities() {
        let start = Instant::now();
        let mut state = HistoryLoading::default();
        state.sync_rows(vec![(4, row("a")), (5, row("b"))], start);
        let moved = start + Duration::from_millis(200);
        state.sync_rows(vec![(8, row("b")), (9, row("c"))], moved);
        let now = start + Duration::from_millis(300);
        assert!(!state.skeleton_visible(4, now));
        assert!(state.skeleton_visible(8, now));
        assert!(!state.skeleton_visible(9, now));
        assert_eq!(
            state.next_deadline(now),
            Some(start + Duration::from_millis(500))
        );
        state.sync_rows(vec![(4, row("a"))], now);
        assert!(
            !state.skeleton_visible(4, now),
            "revisiting starts a new wait"
        );
    }

    #[test]
    fn history_loading_status_spans_continuous_work_without_repeating_timers() {
        let start = Instant::now();
        let mut state = HistoryLoading {
            initial_since: Some(start),
            ..Default::default()
        };
        state.sync_busy(start);
        assert!(!state.initial_skeleton_visible(start + Duration::from_millis(299)));
        assert!(state.initial_skeleton_visible(start + Duration::from_millis(300)));
        state.initial_since = None;
        state.background_busy = true;
        state.sync_busy(start + Duration::from_millis(400));
        assert!(!state.status_visible(start + Duration::from_millis(999)));
        assert!(state.status_visible(start + STATUS_DELAY));
        assert_eq!(state.next_deadline(start + STATUS_DELAY), None);
        state.background_busy = false;
        state.sync_busy(start + STATUS_DELAY);
        assert!(!state.status_visible(start + STATUS_DELAY));
    }
}
