use super::*;
use crate::view::rows::FileListId;
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
        hash.finish()
    }

    pub(in crate::view) fn open_review_search(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo) = self.active_repo_id() else {
            return false;
        };
        let list = self
            .visible_review_lists()
            .into_iter()
            .find(|&list| self.review_file_focus(list).contains_focused(window, cx));
        let Some(list) = list else {
            return false;
        };
        if self.review_search_list != Some((repo, list)) {
            self.reset_review_search(cx);
            self.review_search_list = Some((repo, list));
        }
        let focus = self.review_search_input.read(cx).focus_handle();
        window.focus(&focus, cx);
        cx.notify();
        true
    }

    pub(super) fn reset_review_search(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some((repo, list)) = self.review_search_list.take() {
            self.file_controller(repo, list).borrow_mut().query = SharedString::default();
            self.review_search_input
                .update(cx, |input, cx| input.set_text("", cx));
            self.notify_commit_file_projection_dependents(cx);
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
            self.reset_review_search(cx);
            window.focus(self.review_file_focus(list), cx);
            cx.notify();
            return true;
        }
        if matches!(key.key.as_str(), "enter" | "down")
            && self
                .review_search_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
        {
            // Keep the filter active while reviewing its results.
            self.focus_review_files(window, cx);
            return true;
        }
        false
    }

    pub(super) fn review_search_bar(
        &self,
        list: FileListId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        if self.review_search_list != self.active_repo_id().map(|id| (id, list)) {
            return None;
        }
        let scale = self.ui_scale();
        Some(
            div()
                .id("review_file_search")
                .debug_selector(|| "review_file_search".to_owned())
                .flex()
                .items_center()
                .gap(scale.px(4.0))
                .p(scale.px(6.0))
                .flex_shrink_0()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(self.review_search_input.clone()),
                )
                .child(
                    components::Button::new("review_search_close", "")
                        .style(components::ButtonStyle::Transparent)
                        .start_slot(icons::svg_icon(
                            "icons/generic_close.svg",
                            self.theme.colors.foreground.secondary,
                            scale.px(14.0),
                        ))
                        .on_click(
                            self.theme,
                            cx,
                            move |this: &mut DetailsPaneView, _, window, cx| {
                                this.reset_review_search(cx);
                                window.focus(this.review_file_focus(list), cx);
                                cx.notify();
                            },
                        )
                        .gitcomet_tooltip(self.theme, "Close file search (Esc)".into()),
                )
                .into_any_element(),
        )
    }
}
