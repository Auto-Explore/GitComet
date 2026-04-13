use super::*;

impl PopoverHost {
    pub(super) fn ensure_repo_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.repo_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter repositories".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        if self._repo_picker_search_input_subscription.is_none() {
            self._repo_picker_search_input_subscription =
                Some(cx.observe(input, |this, input, cx| {
                    let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

                    if !matches!(this.popover, Some(PopoverKind::RepoPicker)) {
                        return;
                    }

                    if escape_pressed {
                        this.close_popover(cx);
                        return;
                    }

                    cx.notify();
                }));
        }
        input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_recent_repo_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.recent_repo_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter recent repositories".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_branch_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.branch_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter branches".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        if self._branch_picker_search_input_subscription.is_none() {
            self._branch_picker_search_input_subscription =
                Some(cx.observe(input, |this, input, cx| {
                    let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

                    if !matches!(this.popover, Some(PopoverKind::BranchPicker)) {
                        return;
                    }

                    if escape_pressed {
                        this.close_popover(cx);
                        return;
                    }

                    cx.notify();
                }));
        }
        input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_worktree_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.worktree_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter worktrees".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_submodule_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.submodule_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter submodules".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_subtree_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.subtree_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter subtrees".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_file_history_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.file_history_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter commits".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_diff_hunk_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.diff_hunk_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter hunks".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }
}
