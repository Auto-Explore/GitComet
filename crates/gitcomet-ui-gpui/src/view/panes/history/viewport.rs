use super::*;
use crate::view::caches::HistoryListRow;

/// The source and row mapping used by the previous list layout. Keeping these
/// together lets a new status result or graph build move rows without changing
/// what a scroll offset meant before that result arrived.
pub(super) struct PresentedHistory {
    request: HistoryBaseCacheRequest,
    page: Arc<LogPage>,
    visible_indices: HistoryVisibleIndices,
    plan: HistoryListPlan,
    worktree_paths: Vec<PathBuf>,
    worktree_dirty_rev: u64,
    load_epoch: u64,
    ready: bool,
    height: Pixels,
}

impl HistoryView {
    pub(super) fn sync_history_viewport(&mut self, plan: &HistoryListPlan) {
        let Some(repo) = self.active_repo() else {
            self.presented_history = None;
            return;
        };
        let Some(cache) = self
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request.repo_id == repo.id)
        else {
            self.presented_history = None;
            return;
        };
        let height = crate::view::rows::history_row_height(self.ui_scale());
        let ready = matches!(repo.log, Loadable::Ready(_));
        if self.presented_history.as_ref().is_some_and(|old| {
            old.request == cache.base.request
                && old.plan.fingerprint() == plan.fingerprint()
                && old.worktree_dirty_rev == repo.worktree_dirty_rev
                && old.load_epoch == repo.load_epoch
                && old.height == height
                && old.ready == ready
        }) {
            return;
        }
        let paths = match &repo.worktree_dirty {
            Loadable::Ready(dirty) => dirty.iter().map(|summary| summary.path.clone()).collect(),
            _ => Vec::new(),
        };
        let next = PresentedHistory {
            request: cache.base.request.clone(),
            page: Arc::clone(&cache.page),
            visible_indices: cache.base.visible_indices.clone(),
            plan: plan.clone(),
            worktree_paths: paths,
            worktree_dirty_rev: repo.worktree_dirty_rev,
            load_epoch: repo.load_epoch,
            ready,
            height,
        };
        let scroll = self.history_scroll.0.borrow();
        let offset = scroll.base_handle.offset();
        let restored = self.presented_history.as_ref().and_then(|old| {
            if !old.ready
                || !next.ready
                || old.load_epoch != next.load_epoch
                || old.request.repo_id != next.request.repo_id
                || old.request.history_scope != next.request.history_scope
                || old.request.history_author_filter != next.request.history_author_filter
                || self.pending_history_reveal.is_some()
                || scroll.deferred_scroll_to_item.is_some()
                || offset.y >= px(0.0)
                || old.height <= px(0.0)
                || height <= px(0.0)
            {
                return None;
            }
            let len = old.plan.list_len(old.visible_indices.len());
            if len == 0 {
                return None;
            }
            let top = ((-offset.y / old.height).floor() as usize).min(len - 1);
            let relocate = |ix| {
                let next_ix = match old.plan.row_at(ix)? {
                    HistoryListRow::WorkingTreeSummary => {
                        next.plan.show_working_tree_summary_row().then_some(0)?
                    }
                    HistoryListRow::WorktreeUncommitted { worktree_ix, .. } => {
                        let path = old.worktree_paths.get(worktree_ix)?;
                        let next_worktree = next
                            .worktree_paths
                            .iter()
                            .position(|candidate| candidate == path)?;
                        next.plan.list_ix_for_worktree(next_worktree)?
                    }
                    HistoryListRow::Commit { visible_ix } => {
                        let commit = old.page.commits.get(old.visible_indices.get(visible_ix)?)?;
                        next.plan
                            .list_ix_for_visible(*cache.base.visible_ix_by_commit.get(&commit.id)?)
                    }
                };
                let relative_y = offset.y + old.height * ix as f32;
                Some(point(
                    offset.x,
                    (relative_y - height * next_ix as f32).min(px(0.0)),
                ))
            };
            // Normally just one lookup. If that row was removed, find the
            // nearest survivor in the old order, preferring the following row.
            for distance in 0..len {
                if top + distance < len
                    && let Some(offset) = relocate(top + distance)
                {
                    return Some(offset);
                }
                if distance > 0
                    && let Some(ix) = top.checked_sub(distance)
                    && let Some(offset) = relocate(ix)
                {
                    return Some(offset);
                }
            }
            None
        });
        if let Some(offset) = restored {
            // Set this before uniform_list prepaint clamps against the new size.
            scroll.base_handle.set_offset(offset);
        }
        drop(scroll);
        self.presented_history = Some(next);
    }
}
