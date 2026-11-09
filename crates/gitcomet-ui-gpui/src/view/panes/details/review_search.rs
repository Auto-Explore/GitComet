use super::*;
use crate::view::rows::{FileListId, FileListRow, RowIx};
use std::hash::{Hash, Hasher};

impl DetailsPaneView {
    pub(super) fn review_projection_key(&self, repo: RepoId, list: FileListId, key: u64) -> u64 {
        let controller = self.file_controller(repo, list);
        let controller = controller.borrow();
        if controller.query.is_empty() {
            return key;
        }
        let mut hash = rustc_hash::FxHasher::default();
        key.hash(&mut hash);
        controller.query.hash(&mut hash);
        controller.search_options.hash(&mut hash);
        hash.finish()
    }

    /// Cmd/Ctrl-F in the details pane: filter the focused file list, or the
    /// one last filtered, or the first one shown. Reopening keeps the query
    /// and selects it.
    pub(in crate::view) fn open_review_search(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo) = self.active_repo_id() else {
            return false;
        };
        let lists = self.visible_review_lists();
        let list = lists
            .iter()
            .copied()
            .find(|&list| self.review_file_focus(list).contains_focused(window, cx))
            .or_else(|| {
                self.review_search_list
                    .filter(|(id, list)| *id == repo && lists.contains(list))
                    .map(|(_, list)| list)
            })
            .or_else(|| lists.first().copied());
        let Some(list) = list else {
            return false;
        };
        if self.review_search_list != Some((repo, list)) {
            self.reset_review_search(cx);
            self.review_search_list = Some((repo, list));
        }
        self.review_search_input
            .update(cx, |input, cx| input.select_all_text(window, cx));
        let focus = self.review_search_input.read(cx).focus_handle();
        window.focus(&focus, cx);
        cx.notify();
        true
    }

    pub(super) fn reset_review_search(&mut self, cx: &mut gpui::Context<Self>) {
        self.review_search_bar.dismiss_menu();
        if let Some((repo, list)) = self.review_search_list.take() {
            self.file_controller(repo, list).borrow_mut().query = SharedString::default();
            self.review_search_input
                .update(cx, |input, cx| input.set_text("", cx));
            self.notify_commit_file_projection_dependents(cx);
        }
    }

    /// Close the filter and hand focus back to its list.
    fn close_review_search(
        &mut self,
        list: FileListId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.reset_review_search(cx);
        window.focus(self.review_file_focus(list), cx);
        cx.notify();
    }

    /// Escape: close the bar's overflow menu first, then the bar.
    pub(super) fn review_search_escape(
        &mut self,
        list: FileListId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.review_search_bar.dismiss_menu() {
            cx.notify();
        } else {
            self.close_review_search(list, window, cx);
        }
    }

    pub(super) fn handle_review_search_key(
        &mut self,
        list: FileListId,
        key: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if key.modifiers.modified()
            || self.review_search_list != self.active_repo_id().map(|id| (id, list))
        {
            return false;
        }
        if key.key == "escape" {
            self.review_search_escape(list, window, cx);
            return true;
        }
        false
    }

    /// The shown file rows of `list`, in display order.
    fn review_file_rows(&self, list: FileListId) -> Vec<usize> {
        let Some(data) = self.review_list(list) else {
            return Vec::new();
        };
        (0..data.plan.row_len())
            .filter(|&row| matches!(data.plan.row_at(RowIx(row)), Some(FileListRow::File { .. })))
            .collect()
    }

    /// ↑/↓ (and Enter) in the bar: select the previous or next file the
    /// filter shows, wrapping around, as a click on it would.
    pub(super) fn review_search_step(
        &mut self,
        list: FileListId,
        forward: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(data) = self.review_list(list) else {
            return;
        };
        let files = self.review_file_rows(list);
        if files.is_empty() {
            return;
        }
        let current = self
            .review_current_row(list, &data)
            .and_then(|row| files.iter().position(|&file| file == row));
        let next = match (current, forward) {
            (None, true) => 0,
            (None, false) => files.len() - 1,
            (Some(ix), true) => (ix + 1) % files.len(),
            (Some(ix), false) => (ix + files.len() - 1) % files.len(),
        };
        self.select_review_row(list, &data, files[next], cx);
        let focus = self.review_search_input.read(cx).focus_handle();
        window.focus(&focus, cx);
        cx.notify();
    }

    /// F2/F3 with focus in the details pane: step its open file filter.
    /// Returns false when it is closed.
    pub(in crate::view) fn step_review_search(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some((repo, list)) = self.review_search_list else {
            return false;
        };
        if self.active_repo_id() != Some(repo) {
            return false;
        }
        self.review_search_step(list, forward, window, cx);
        true
    }

    fn review_search_status(&self, list: FileListId) -> components::QuickSearchStatus {
        use components::QuickSearchStatus;
        let Some(repo) = self.active_repo_id() else {
            return QuickSearchStatus::Empty;
        };
        let (empty, invalid) = {
            let controller = self.file_controller(repo, list);
            let controller = controller.borrow();
            let matcher =
                crate::view::rows::path_query_matcher(&controller.query, controller.search_options);
            (
                matcher.is_none(),
                matcher.is_some_and(|matcher| matcher.regex_error().is_some()),
            )
        };
        if invalid {
            return QuickSearchStatus::InvalidRegex;
        }
        if empty {
            return QuickSearchStatus::Empty;
        }
        let files = self.review_file_rows(list);
        if files.is_empty() {
            return QuickSearchStatus::NoMatches;
        }
        let current = self.review_list(list).and_then(|data| {
            self.review_current_row(list, &data)
                .and_then(|row| files.iter().position(|&file| file == row))
        });
        QuickSearchStatus::Position {
            current,
            total: files.len(),
            complete: true,
        }
    }

    pub(super) fn review_search_bar(
        &self,
        list: FileListId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let repo = self.active_repo_id()?;
        if self.review_search_list != Some((repo, list)) {
            return None;
        }
        let scale = self.ui_scale();
        let status = self.review_search_status(list);
        let options = self.file_controller(repo, list).borrow().search_options;
        let can_step = matches!(
            status,
            components::QuickSearchStatus::Position { total, .. } if total > 0
        );
        let step = move |forward: bool| {
            move |this: &mut Self, window: &mut Window, cx: &mut gpui::Context<Self>| {
                this.review_search_step(list, forward, window, cx);
            }
        };
        let bar = components::QuickSearchBar::new("review_search", &self.review_search_bar, status)
            .input(self.review_search_input.clone())
            .options(
                components::search_toggles(options),
                move |this: &mut Self, toggles, window, cx| {
                    let Some(repo) = this.active_repo_id() else {
                        return;
                    };
                    this.file_controller(repo, list).borrow_mut().search_options =
                        components::search_options(toggles);
                    this.review_cursor = None;
                    this.notify_commit_file_projection_dependents(cx);
                    let focus = this.review_search_input.read(cx).focus_handle();
                    window.focus(&focus, cx);
                    cx.notify();
                },
            )
            .navigation(can_step, step(false), step(true))
            .on_close(move |this, window, cx| this.close_review_search(list, window, cx))
            .render(self.theme, scale, cx);
        Some(
            div()
                .id("review_file_search")
                .debug_selector(|| "review_file_search".to_owned())
                .flex_shrink_0()
                .w_full()
                .min_w(px(0.0))
                .p(scale.px(6.0))
                .child(bar)
                .into_any_element(),
        )
    }
}
