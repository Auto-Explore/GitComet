use super::*;
use crate::view::caches::HistoryListRow;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct ViewportKey {
    repo: RepoId,
    epoch: u64,
    source: usize,
    plan: u64,
    start: usize,
    end: usize,
}

impl HistoryView {
    pub(super) fn sync_signature_viewport(
        &mut self,
        fallback_plan: &HistoryListPlan,
        cx: &mut gpui::Context<Self>,
    ) {
        // Keep the disabled path free of ID collections, queues, and timers.
        if !self.state.git_log_settings.verify_commit_signatures {
            self.signature_viewport = None;
            self.signature_debounce = None;
            return;
        }
        let Some(repo) = self.active_repo() else {
            return;
        };
        let repo_id = repo.id;
        let epoch = repo.history_state.commit_signatures_epoch;
        let scroll = self.scroll_interaction.borrow();
        let (source, plan, range) = if let (Some(shown), Some(logical)) =
            (&self.indexed.presentation, &scroll.logical)
        {
            (
                Arc::as_ptr(&shown.graph) as usize,
                &self.indexed.plan,
                logical.top
                    ..(logical.top
                        + ((logical.viewport + logical.within) / logical.height).ceil() as usize)
                        .min(logical.total),
            )
        } else {
            let Some(cache) = &self.history_cache else {
                return;
            };
            let handle = self.history_scroll.0.borrow();
            let Some(size) = handle.last_item_size else {
                return;
            };
            let height = crate::view::rows::history_row_height(self.ui_scale());
            let offset = (-handle.base_handle.offset().y).max(px(0.0));
            let start = (offset / height).floor() as usize;
            let end = ((offset + size.item.height) / height).ceil() as usize;
            (
                Arc::as_ptr(&cache.page) as usize,
                fallback_plan,
                start..end.min(fallback_plan.list_len(cache.base.visible_indices.len())),
            )
        };
        let key = ViewportKey {
            repo: repo_id,
            epoch,
            source,
            plan: plan.fingerprint(),
            start: range.start,
            end: range.end,
        };
        if self.signature_viewport == Some(key) {
            return;
        }
        let ids: Arc<[CommitId]> = range
            .take(256)
            .filter_map(|row| {
                let HistoryListRow::Commit { visible_ix } = plan.row_at(row)? else {
                    return None;
                };
                if let Some(shown) = &self.indexed.presentation {
                    shown.graph.projection.commit_id(visible_ix)
                } else {
                    let cache = self.history_cache.as_ref()?;
                    Some(
                        cache
                            .page
                            .commits
                            .get(cache.base.visible_indices.get(visible_ix)?)?
                            .id
                            .clone(),
                    )
                }
            })
            .collect();
        drop(scroll);
        self.signature_viewport = Some(key);
        let delay = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(100));
        self.signature_debounce = Some(cx.spawn(async move |view, cx| {
            delay.await;
            let _ = view.update(cx, |this, _cx| {
                if this.signature_viewport == Some(key)
                    && this.state.git_log_settings.verify_commit_signatures
                {
                    this.store.dispatch(Msg::SetCommitSignatureTargets {
                        repo_id,
                        epoch,
                        commit_ids: ids,
                    });
                }
            });
        }));
    }
}
