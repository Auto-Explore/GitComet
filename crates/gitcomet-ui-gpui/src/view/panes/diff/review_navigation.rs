use super::*;

/// A file boundary is asynchronous. Keep its destination until the new diff
/// has been laid out, so repeated keys cannot navigate the previous file twice.
pub(in crate::view) struct ReviewFileJump {
    pub repo: RepoId,
    pub from: DiffTarget,
    pub to: DiffTarget,
    pub last: bool,
}

pub(in crate::view) struct ReviewPosition {
    repo: RepoId,
    target: DiffTarget,
    anchor: Option<usize>,
    offset: Point<Pixels>,
}

impl DiffRenderer {
    pub(in crate::view) fn remember_review_position(&mut self) {
        self.review_position = self
            .active_repo_id()
            .zip(self.rendered_diff_target().cloned())
            .map(|(repo, target)| ReviewPosition {
                repo,
                target,
                anchor: self.diff_selection_anchor,
                offset: self.diff_scroll.0.borrow().base_handle.offset(),
            });
    }

    pub(in crate::view) fn restore_review_position(&mut self) -> bool {
        if !self.review_position.as_ref().is_some_and(|position| {
            self.active_repo_id() == Some(position.repo)
                && self.rendered_diff_target() == Some(&position.target)
        }) {
            return false;
        }
        let position = self.review_position.take().unwrap();
        if let Some(anchor) = position
            .anchor
            .filter(|&anchor| anchor < self.diff_visible_len())
        {
            self.focus_review_change(anchor);
        }
        self.diff_scroll
            .0
            .borrow()
            .base_handle
            .set_offset(position.offset);
        self.diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .set_offset(position.offset);
        self.diff_autoscroll_pending = false;
        true
    }
    pub(in crate::view) fn prepare_review_file_jump(
        &mut self,
        repo: RepoId,
        to: DiffTarget,
        last: bool,
    ) {
        if let Some(from) = self.rendered_diff_target().cloned() {
            self.review_file_jump = Some(ReviewFileJump {
                repo,
                from,
                to,
                last,
            });
        }
    }

    pub(in crate::view) fn review_diff_is_loading(&self) -> bool {
        matches!(self.rendered_patch_diff_loadable(), Some(Loadable::Loading))
            || matches!(self.rendered_file_diff_loadable(), Some(Loadable::Loading))
            || self.file_diff_cache_inflight.is_some()
    }

    pub(in crate::view) fn consume_pending_review_jump(&mut self) -> bool {
        let Some(jump) = &self.review_file_jump else {
            return false;
        };
        let target = self.rendered_diff_target();
        if self.active_repo_id() != Some(jump.repo)
            || (target != Some(&jump.from) && target != Some(&jump.to))
        {
            self.review_file_jump = None;
            return false;
        }
        if target != Some(&jump.to) || self.review_diff_is_loading() {
            return true;
        }
        // Text diffs land in maybe_autoscroll_diff_to_first_change. A binary,
        // image or empty diff is itself one review stop, with no block to select.
        if self.diff_nav_entries().is_empty() {
            self.review_file_jump = None;
            self.diff_autoscroll_pending = false;
            return true;
        }
        self.maybe_autoscroll_diff_to_first_change();
        true
    }

    pub(in crate::view) fn review_adjacent_file(
        &mut self,
        direction: i8,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.consume_pending_review_jump() || self.review_diff_is_loading() {
            return true;
        }
        let Some(repo) = self.active_repo_id() else {
            return false;
        };
        self.try_select_review_file(repo, direction, window, cx)
    }

    pub(in crate::view) fn review_adjacent_change(
        &mut self,
        previous: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.rendered_diff_target().is_none() {
            return false;
        }
        if self.is_conflict_resolver_active() {
            // The resolver retains its own editing and conflict workflow.
            return if previous {
                self.navigate_prev_diff_change(cx)
            } else {
                self.navigate_next_diff_change(cx)
            };
        }
        if self.consume_pending_review_jump() || self.review_diff_is_loading() {
            return true;
        }
        let entries = self.diff_nav_entries();
        let target = if self.diff_focus_visible_range().is_none() {
            if previous {
                entries.last()
            } else {
                entries.first()
            }
            .copied()
        } else if previous {
            self.diff_nav_prev_target_ix(&entries)
        } else {
            self.diff_nav_next_target_ix(&entries)
        };
        if let Some(target) = target {
            self.focus_review_change(target);
        } else {
            self.review_adjacent_file(if previous { -1 } else { 1 }, window, cx);
        }
        // Consume even at the outer edge of the list.
        true
    }

    pub(in crate::view) fn focus_review_change(&mut self, target: usize) {
        self.clear_diff_text_selection();
        self.diff_selection_range = None;
        self.diff_selection_anchor = Some(target);
        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.focus_change_block_at(target);
    }

    pub(in crate::view) fn review_scroll_key(&mut self, key: &str) -> bool {
        match key {
            "home" | "end" => {
                let entries = self.diff_nav_entries();
                let target = if key == "home" {
                    entries.first()
                } else {
                    entries.last()
                };
                if let Some(&target) = target {
                    self.focus_review_change(target);
                }
            }
            "pageup" | "pagedown" => {
                let preview = self.is_file_preview_active();
                let scroll = if preview {
                    self.worktree_preview_scroll.0.borrow().base_handle.clone()
                } else {
                    self.diff_scroll.0.borrow().base_handle.clone()
                };
                let offset = scroll.offset();
                let direction = if key == "pageup" { 1.0 } else { -1.0 };
                let y = (offset.y + scroll.bounds().size.height * 0.9 * direction)
                    .max(-scroll.max_offset().y)
                    .min(px(0.0));
                scroll.set_offset(point(offset.x, y));
                if !preview {
                    let right = self.diff_split_right_scroll.0.borrow().base_handle.clone();
                    right.set_offset(point(right.offset().x, y));
                }
            }
            _ => return false,
        }
        true
    }
}
