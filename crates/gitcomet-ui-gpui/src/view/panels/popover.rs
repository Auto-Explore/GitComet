use super::*;

mod app_menu;
mod branch_picker;
mod checkout_remote_branch_prompt;
mod clone_repo;
mod conflict_save_stage_confirm;
pub(in super::super) mod context_menu;
mod create_branch;
mod create_branch_from_ref_prompt;
mod create_tag_prompt;
mod delete_remote_branch_confirm;
mod diff_hunks;
mod discard_changes_confirm;
mod file_history;
mod fingerprint;
mod force_delete_branch_confirm;
mod force_push_confirm;
mod force_remove_worktree_confirm;
mod merge_abort_confirm;
mod open_source_licenses;
mod pull_reconcile_prompt;
mod push_set_upstream_prompt;
mod recent_repo_picker;
mod remote_add_prompt;
mod remote_edit_url_prompt;
mod remote_remove_confirm;
mod repo_picker;
mod reset_prompt;
mod search_inputs;
mod settings;
mod stash_drop_confirm;
mod stash_prompt;
mod submodule_add_prompt;
mod submodule_open_picker;
mod submodule_remove_confirm;
mod submodule_remove_picker;
mod worktree_add_prompt;
mod worktree_open_picker;
mod worktree_remove_confirm;
mod worktree_remove_picker;

#[derive(Clone, Debug)]
enum PopoverAnchor {
    Point(Point<Pixels>),
    Bounds(Bounds<Pixels>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsSubmenu {
    Theme,
    DateFormat,
    Timezone,
}

pub(in super::super) struct PopoverHost {
    store: Arc<AppStore>,
    state: Arc<AppState>,
    theme: AppTheme,
    theme_mode: ThemeMode,
    date_time_format: DateTimeFormat,
    timezone: Timezone,
    show_timezone: bool,
    settings_submenu: Option<SettingsSubmenu>,
    settings_submenu_top: Option<Pixels>,
    settings_submenu_left: Option<Pixels>,
    settings_submenu_width: Option<Pixels>,
    settings_submenu_max_h: Option<Pixels>,
    settings_runtime_info: settings::SettingsRuntimeInfo,
    _ui_model_subscription: gpui::Subscription,
    _clone_repo_url_input_subscription: gpui::Subscription,
    _clone_repo_parent_dir_input_subscription: gpui::Subscription,
    _create_tag_input_subscription: gpui::Subscription,
    _repo_picker_search_input_subscription: Option<gpui::Subscription>,
    _branch_picker_search_input_subscription: Option<gpui::Subscription>,
    _create_branch_input_subscription: gpui::Subscription,
    _stash_message_input_subscription: gpui::Subscription,
    notify_fingerprint: u64,
    root_view: WeakEntity<GitCometView>,
    main_pane: Entity<MainPaneView>,
    details_pane: Entity<DetailsPaneView>,

    popover: Option<PopoverKind>,
    popover_anchor: Option<PopoverAnchor>,
    context_menu_focus_handle: FocusHandle,
    context_menu_selected_ix: Option<usize>,

    repo_picker_search_input: Option<Entity<components::TextInput>>,
    recent_repo_picker_search_input: Option<Entity<components::TextInput>>,
    branch_picker_search_input: Option<Entity<components::TextInput>>,
    remote_picker_search_input: Option<Entity<components::TextInput>>,
    file_history_search_input: Option<Entity<components::TextInput>>,
    worktree_picker_search_input: Option<Entity<components::TextInput>>,
    submodule_picker_search_input: Option<Entity<components::TextInput>>,
    diff_hunk_picker_search_input: Option<Entity<components::TextInput>>,
    picker_prompt_scroll: ScrollHandle,

    clone_repo_url_input: Entity<components::TextInput>,
    clone_repo_parent_dir_input: Entity<components::TextInput>,
    rebase_onto_input: Entity<components::TextInput>,
    create_tag_input: Entity<components::TextInput>,
    remote_name_input: Entity<components::TextInput>,
    remote_url_input: Entity<components::TextInput>,
    remote_url_edit_input: Entity<components::TextInput>,
    create_branch_input: Entity<components::TextInput>,
    create_branch_checkout_enabled: bool,
    stash_message_input: Entity<components::TextInput>,
    push_upstream_branch_input: Entity<components::TextInput>,
    worktree_path_input: Entity<components::TextInput>,
    worktree_ref_input: Entity<components::TextInput>,
    submodule_url_input: Entity<components::TextInput>,
    submodule_path_input: Entity<components::TextInput>,

    open_source_licenses_scroll: ScrollHandle,
}

impl PopoverHost {
    fn sync_titlebar_app_menu_state(&self, cx: &mut gpui::Context<Self>) {
        let root_view = self.root_view.clone();
        let app_menu_open = matches!(
            self.popover,
            Some(
                PopoverKind::AppMenu
                    | PopoverKind::Settings
                    | PopoverKind::OpenSourceLicenses
                    | PopoverKind::SettingsThemeMenu
                    | PopoverKind::SettingsDateFormatMenu
                    | PopoverKind::SettingsTimezoneMenu
            )
        );
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.title_bar.update(cx, |title_bar, cx| {
                    title_bar.set_app_menu_open(app_menu_open, cx);
                });
            });
        });
    }

    fn clear_active_context_menu_invoker(&self, cx: &mut gpui::Context<Self>) {
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.set_active_context_menu_invoker(None, cx);
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        theme_mode: ThemeMode,
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        show_timezone: bool,
        root_view: WeakEntity<GitCometView>,
        main_pane: Entity<MainPaneView>,
        details_pane: Entity<DetailsPaneView>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            this.state = Arc::clone(&model.read(cx).state);

            let Some(popover) = this.popover.as_ref() else {
                return;
            };

            let next_fingerprint = fingerprint::notify_fingerprint(&this.state, popover);
            if next_fingerprint != this.notify_fingerprint {
                this.notify_fingerprint = next_fingerprint;
                cx.notify();
            }
        });

        let clone_repo_url_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let clone_repo_parent_dir_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/parent/folder".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let clone_repo_url_input_subscription =
            cx.observe(&clone_repo_url_input, |this, input, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

                if !matches!(this.popover, Some(PopoverKind::CloneRepo)) {
                    return;
                }

                if escape_pressed {
                    this.close_popover(cx);
                    return;
                }

                if enter_pressed {
                    this.submit_clone_repo(cx);
                    return;
                }

                cx.notify();
            });

        let clone_repo_parent_dir_input_subscription =
            cx.observe(&clone_repo_parent_dir_input, |this, input, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

                if !matches!(this.popover, Some(PopoverKind::CloneRepo)) {
                    return;
                }

                if escape_pressed {
                    this.close_popover(cx);
                    return;
                }

                if enter_pressed {
                    this.submit_clone_repo(cx);
                    return;
                }

                cx.notify();
            });

        let rebase_onto_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "origin/main".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let create_tag_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "v1.0.0".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let remote_name_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "origin".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let remote_url_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let remote_url_edit_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let create_branch_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-name".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let stash_message_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Stash message".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let create_tag_input_subscription = cx.observe(&create_tag_input, |this, input, cx| {
            let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
            let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

            if !matches!(this.popover, Some(PopoverKind::CreateTagPrompt { .. })) {
                return;
            }

            if escape_pressed {
                this.close_popover(cx);
                return;
            }

            if enter_pressed {
                this.submit_create_tag(cx);
                return;
            }

            cx.notify();
        });

        let create_branch_input_subscription =
            cx.observe_in(&create_branch_input, window, |this, input, window, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());
                let is_create_branch_prompt = matches!(
                    this.popover,
                    Some(PopoverKind::CreateBranch)
                        | Some(PopoverKind::CreateBranchFromRefPrompt { .. })
                );

                if !is_create_branch_prompt {
                    return;
                }

                if escape_pressed {
                    this.dismiss_inline_popover(window, cx);
                    return;
                }

                if enter_pressed {
                    this.submit_create_branch(window, cx);
                    return;
                }

                cx.notify();
            });

        let stash_message_input_subscription =
            cx.observe_in(&stash_message_input, window, |this, input, window, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

                if !matches!(this.popover, Some(PopoverKind::StashPrompt)) {
                    return;
                }

                if escape_pressed {
                    this.dismiss_inline_popover(window, cx);
                    return;
                }

                if enter_pressed {
                    this.submit_stash(window, cx);
                    return;
                }

                cx.notify();
            });

        let push_upstream_branch_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-name".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let worktree_path_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/worktree".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let worktree_ref_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-or-commit".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let submodule_url_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let submodule_path_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "path/in/repo".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let context_menu_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);

        Self {
            store,
            state,
            theme,
            theme_mode,
            date_time_format,
            timezone,
            show_timezone,
            settings_submenu: None,
            settings_submenu_top: None,
            settings_submenu_left: None,
            settings_submenu_width: None,
            settings_submenu_max_h: None,
            settings_runtime_info: settings::SettingsRuntimeInfo::detect(),
            _ui_model_subscription: subscription,
            _clone_repo_url_input_subscription: clone_repo_url_input_subscription,
            _clone_repo_parent_dir_input_subscription: clone_repo_parent_dir_input_subscription,
            _create_tag_input_subscription: create_tag_input_subscription,
            _repo_picker_search_input_subscription: None,
            _branch_picker_search_input_subscription: None,
            _create_branch_input_subscription: create_branch_input_subscription,
            _stash_message_input_subscription: stash_message_input_subscription,
            notify_fingerprint: 0,
            root_view,
            main_pane,
            details_pane,
            popover: None,
            popover_anchor: None,
            context_menu_focus_handle,
            context_menu_selected_ix: None,
            repo_picker_search_input: None,
            recent_repo_picker_search_input: None,
            branch_picker_search_input: None,
            remote_picker_search_input: None,
            file_history_search_input: None,
            worktree_picker_search_input: None,
            submodule_picker_search_input: None,
            diff_hunk_picker_search_input: None,
            picker_prompt_scroll: ScrollHandle::new(),
            clone_repo_url_input,
            clone_repo_parent_dir_input,
            rebase_onto_input,
            create_tag_input,
            remote_name_input,
            remote_url_input,
            remote_url_edit_input,
            create_branch_input,
            create_branch_checkout_enabled: true,
            stash_message_input,
            push_upstream_branch_input,
            worktree_path_input,
            worktree_ref_input,
            submodule_url_input,
            submodule_path_input,
            open_source_licenses_scroll: ScrollHandle::new(),
        }
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;

        self.clone_repo_url_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.clone_repo_parent_dir_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.rebase_onto_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.create_tag_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.remote_name_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.remote_url_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.remote_url_edit_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.create_branch_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.stash_message_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.push_upstream_branch_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.worktree_path_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.worktree_ref_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.submodule_url_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.submodule_path_input
            .update(cx, |input, cx| input.set_theme(theme, cx));

        if let Some(input) = &self.repo_picker_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        if let Some(input) = &self.recent_repo_picker_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        if let Some(input) = &self.branch_picker_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        if let Some(input) = &self.remote_picker_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        if let Some(input) = &self.file_history_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        if let Some(input) = &self.worktree_picker_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        if let Some(input) = &self.submodule_picker_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        if let Some(input) = &self.diff_hunk_picker_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }

        cx.notify();
    }

    #[cfg(test)]
    pub(in super::super) fn set_state_for_test(&mut self, state: Arc<AppState>) {
        self.state = state;
    }

    pub(in super::super) fn close_popover(&mut self, cx: &mut gpui::Context<Self>) {
        self.popover = None;
        self.popover_anchor = None;
        self.context_menu_selected_ix = None;
        self.notify_fingerprint = 0;
        self.settings_submenu = None;
        self.settings_submenu_top = None;
        self.settings_submenu_left = None;
        self.settings_submenu_width = None;
        self.settings_submenu_max_h = None;
        self.sync_titlebar_app_menu_state(cx);
        self.clear_active_context_menu_invoker(cx);
        cx.notify();
    }

    #[cfg(test)]
    pub(in super::super) fn is_open(&self) -> bool {
        self.popover.is_some()
    }

    fn dismiss_inline_popover(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.popover = None;
        self.popover_anchor = None;
        self.clear_active_context_menu_invoker(cx);
        let focus = self.main_pane.read(cx).diff_panel_focus_handle.clone();
        window.focus(&focus);
        cx.notify();
    }

    fn can_submit_create_tag(&self, cx: &mut gpui::Context<Self>) -> bool {
        matches!(self.popover, Some(PopoverKind::CreateTagPrompt { .. }))
            && self
                .create_tag_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    fn can_submit_clone_repo(&self, cx: &mut gpui::Context<Self>) -> bool {
        matches!(self.popover, Some(PopoverKind::CloneRepo))
            && self
                .clone_repo_url_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
            && self
                .clone_repo_parent_dir_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    fn submit_create_tag(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::CreateTagPrompt { repo_id, target }) = self.popover.clone() else {
            return;
        };

        let name = self
            .create_tag_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if name.is_empty() {
            return;
        }

        self.store.dispatch(Msg::CreateTag {
            repo_id,
            name,
            target,
        });
        self.close_popover(cx);
    }

    fn submit_clone_repo(&mut self, cx: &mut gpui::Context<Self>) {
        if !matches!(self.popover, Some(PopoverKind::CloneRepo)) {
            return;
        }

        let url = self
            .clone_repo_url_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        let parent = self
            .clone_repo_parent_dir_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if url.is_empty() || parent.is_empty() {
            return;
        }

        let repo_name = clone_repo_name_from_url(&url);
        let dest = std::path::PathBuf::from(parent).join(repo_name);
        self.store.dispatch(Msg::CloneRepo { url, dest });
        self.close_popover(cx);
    }

    fn can_submit_create_branch(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.create_branch_prompt_repo_and_target().is_some()
            && self
                .create_branch_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    fn create_branch_prompt_repo_and_target(&self) -> Option<(RepoId, String)> {
        match &self.popover {
            Some(PopoverKind::CreateBranch) => self
                .active_repo_id()
                .map(|repo_id| (repo_id, "HEAD".to_string())),
            Some(PopoverKind::CreateBranchFromRefPrompt { repo_id, target }) => {
                Some((*repo_id, target.clone()))
            }
            _ => None,
        }
    }

    fn submit_create_branch(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, target)) = self.create_branch_prompt_repo_and_target() else {
            return;
        };
        let name = self
            .create_branch_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if name.is_empty() {
            return;
        }

        let checkout = match self.popover {
            Some(PopoverKind::CreateBranch) => true,
            Some(PopoverKind::CreateBranchFromRefPrompt { .. }) => {
                self.create_branch_checkout_enabled
            }
            _ => return,
        };

        if checkout {
            self.store.dispatch(Msg::CreateBranchAndCheckout {
                repo_id,
                name,
                target,
            });
        } else {
            self.store.dispatch(Msg::CreateBranch {
                repo_id,
                name,
                target,
            });
        }
        self.dismiss_inline_popover(window, cx);
    }

    fn can_submit_stash(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.active_repo_id().is_some()
            && self
                .stash_message_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    fn submit_stash(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let message = self
            .stash_message_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if message.is_empty() {
            return;
        }

        self.store.dispatch(Msg::Stash {
            repo_id,
            message,
            include_untracked: true,
        });
        self.dismiss_inline_popover(window, cx);
    }

    pub(in super::super) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover(kind, PopoverAnchor::Point(anchor), window, cx);
    }

    pub(in super::super) fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover(kind, PopoverAnchor::Bounds(anchor_bounds), window, cx);
    }

    fn open_popover(
        &mut self,
        kind: PopoverKind,
        anchor: PopoverAnchor,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !matches!(kind, PopoverKind::Settings) {
            self.settings_submenu = None;
            self.settings_submenu_top = None;
            self.settings_submenu_left = None;
            self.settings_submenu_width = None;
            self.settings_submenu_max_h = None;
        }

        let is_context_menu = matches!(
            &kind,
            PopoverKind::PullPicker
                | PopoverKind::PushPicker
                | PopoverKind::HistoryBranchFilter { .. }
                | PopoverKind::HistoryColumnSettings
                | PopoverKind::SettingsThemeMenu
                | PopoverKind::SettingsDateFormatMenu
                | PopoverKind::SettingsTimezoneMenu
                | PopoverKind::DiffHunkMenu { .. }
                | PopoverKind::DiffEditorMenu { .. }
                | PopoverKind::ConflictResolverInputRowMenu { .. }
                | PopoverKind::ConflictResolverChunkMenu { .. }
                | PopoverKind::ConflictResolverOutputMenu { .. }
                | PopoverKind::CommitMenu { .. }
                | PopoverKind::StatusFileMenu { .. }
                | PopoverKind::BranchMenu { .. }
                | PopoverKind::BranchSectionMenu { .. }
                | PopoverKind::Repo {
                    kind: RepoPopoverKind::Remote(RemotePopoverKind::Menu { .. }),
                    ..
                }
                | PopoverKind::StashMenu { .. }
                | PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(
                        WorktreePopoverKind::SectionMenu | WorktreePopoverKind::Menu { .. },
                    ),
                    ..
                }
                | PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(
                        SubmodulePopoverKind::SectionMenu | SubmodulePopoverKind::Menu { .. },
                    ),
                    ..
                }
                | PopoverKind::CommitFileMenu { .. }
                | PopoverKind::TagMenu { .. }
        );
        let keep_active_invoker = is_context_menu
            || matches!(
                &kind,
                PopoverKind::CreateBranch
                    | PopoverKind::CreateBranchFromRefPrompt { .. }
                    | PopoverKind::StashPrompt
            );
        if !keep_active_invoker {
            self.clear_active_context_menu_invoker(cx);
        }

        self.popover_anchor = Some(anchor);
        self.context_menu_selected_ix = None;
        if is_context_menu {
            self.popover = Some(kind);
            self.context_menu_selected_ix = self
                .popover
                .as_ref()
                .and_then(|kind| self.context_menu_model(kind, cx))
                .and_then(|m| m.first_selectable());
            window.focus(&self.context_menu_focus_handle);
        } else {
            match &kind {
                PopoverKind::RepoPicker => {
                    let _ = self.ensure_repo_picker_search_input(window, cx);
                }
                PopoverKind::RecentRepositoryPicker => {
                    let _ = self.ensure_recent_repo_picker_search_input(window, cx);
                }
                PopoverKind::BranchPicker => {
                    let _ = self.ensure_branch_picker_search_input(window, cx);
                }
                PopoverKind::CreateBranch => {
                    let theme = self.theme;
                    self.create_branch_checkout_enabled = true;
                    self.create_branch_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .create_branch_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::CreateBranchFromRefPrompt { .. } => {
                    let theme = self.theme;
                    self.create_branch_checkout_enabled = true;
                    self.create_branch_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .create_branch_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::CheckoutRemoteBranchPrompt { branch, .. } => {
                    let theme = self.theme;
                    self.create_branch_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(branch.clone(), cx);
                        cx.notify();
                    });
                    let focus = self
                        .create_branch_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::StashPrompt => {
                    let theme = self.theme;
                    self.stash_message_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .stash_message_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::CloneRepo => {
                    let theme = self.theme;
                    let url_text = self
                        .clone_repo_url_input
                        .read_with(cx, |i, _| i.text().to_string());
                    let parent_text = self
                        .clone_repo_parent_dir_input
                        .read_with(cx, |i, _| i.text().to_string());
                    self.clone_repo_url_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(url_text, cx);
                        cx.notify();
                    });
                    self.clone_repo_parent_dir_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(parent_text, cx);
                        cx.notify();
                    });
                    let focus = self
                        .clone_repo_url_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::CreateTagPrompt { .. } => {
                    let theme = self.theme;
                    self.create_tag_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self.create_tag_input.read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::Repo {
                    kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
                    ..
                } => {
                    let theme = self.theme;
                    self.remote_name_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.remote_url_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .remote_name_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::Repo {
                    repo_id,
                    kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { name, .. }),
                } => {
                    let theme = self.theme;
                    let text = self
                        .state
                        .repos
                        .iter()
                        .find(|r| r.id == *repo_id)
                        .and_then(|r| match &r.remotes {
                            Loadable::Ready(remotes) => remotes
                                .iter()
                                .find(|remote| remote.name.as_str() == name.as_str())
                                .and_then(|remote| remote.url.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    self.remote_url_edit_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(text, cx);
                        cx.notify();
                    });
                    let focus = self
                        .remote_url_edit_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                    ..
                } => {
                    let theme = self.theme;
                    self.worktree_path_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.worktree_ref_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .worktree_path_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::Repo {
                    repo_id,
                    kind:
                        RepoPopoverKind::Worktree(
                            WorktreePopoverKind::OpenPicker | WorktreePopoverKind::RemovePicker,
                        ),
                } => {
                    let _ = self.ensure_worktree_picker_search_input(window, cx);
                    self.store
                        .dispatch(Msg::LoadWorktrees { repo_id: *repo_id });
                }
                PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
                    ..
                } => {
                    let theme = self.theme;
                    self.submodule_url_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.submodule_path_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .submodule_url_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::Repo {
                    repo_id,
                    kind:
                        RepoPopoverKind::Submodule(
                            SubmodulePopoverKind::OpenPicker | SubmodulePopoverKind::RemovePicker,
                        ),
                } => {
                    let _ = self.ensure_submodule_picker_search_input(window, cx);
                    self.store
                        .dispatch(Msg::LoadSubmodules { repo_id: *repo_id });
                }
                PopoverKind::FileHistory { repo_id, path } => {
                    self.ensure_file_history_search_input(window, cx);
                    self.store.dispatch(Msg::LoadFileHistory {
                        repo_id: *repo_id,
                        path: path.clone(),
                        limit: 200,
                    });
                }
                PopoverKind::PushSetUpstreamPrompt { repo_id, .. } => {
                    let theme = self.theme;
                    let current_text = self
                        .push_upstream_branch_input
                        .read_with(cx, |i, _| i.text().to_string());
                    let text = self
                        .state
                        .repos
                        .iter()
                        .find(|r| r.id == *repo_id)
                        .and_then(|repo| match &repo.head_branch {
                            Loadable::Ready(head) if !head.is_empty() => Some(head.clone()),
                            _ => None,
                        })
                        .unwrap_or(current_text);
                    self.push_upstream_branch_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(text, cx);
                        cx.notify();
                    });
                    let focus = self
                        .push_upstream_branch_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus);
                }
                PopoverKind::DiffHunks => {
                    let _ = self.ensure_diff_hunk_picker_search_input(window, cx);
                }
                PopoverKind::OpenSourceLicenses => {
                    self.open_source_licenses_scroll
                        .set_offset(point(px(0.0), px(0.0)));
                }
                _ => {}
            }
            self.popover = Some(kind);
        }
        if let Some(popover) = self.popover.as_ref() {
            self.notify_fingerprint = fingerprint::notify_fingerprint(&self.state, popover);
        }
        self.sync_titlebar_app_menu_state(cx);
        cx.notify();
    }

    fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in super::super) fn set_date_time_format(
        &mut self,
        next: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == next {
            return;
        }
        self.date_time_format = next;
        self.main_pane
            .update(cx, |pane, cx| pane.set_date_time_format(next, cx));
        self.schedule_ui_settings_persist(cx);
    }

    pub(in super::super) fn set_timezone(&mut self, next: Timezone, cx: &mut gpui::Context<Self>) {
        if self.timezone == next {
            return;
        }
        self.timezone = next;
        self.main_pane
            .update(cx, |pane, cx| pane.set_timezone(next, cx));
        self.schedule_ui_settings_persist(cx);
    }

    pub(in super::super) fn set_show_timezone(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.show_timezone == enabled {
            return;
        }
        self.show_timezone = enabled;
        self.main_pane
            .update(cx, |pane, cx| pane.set_show_timezone(enabled, cx));
        self.schedule_ui_settings_persist(cx);
    }

    pub(in super::super) fn set_theme_mode(
        &mut self,
        next: ThemeMode,
        appearance: gpui::WindowAppearance,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.theme_mode == next {
            return;
        }

        self.theme_mode = next;
        self.set_theme(next.resolve_theme(appearance), cx);
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.set_theme_mode(next, appearance, cx);
            });
        });
    }

    fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        let mode = self.theme_mode;
        let fmt = self.date_time_format;
        let tz = self.timezone;
        let show_tz = self.show_timezone;
        let root_view = self.root_view.clone();
        cx.spawn(
            async move |_host: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let _ = root_view.update(cx, |root, cx| {
                    root.theme_mode = mode;
                    root.date_time_format = fmt;
                    root.timezone = tz;
                    root.show_timezone = show_tz;
                    root.schedule_ui_settings_persist(cx);
                });
            },
        )
        .detach();
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn install_linux_desktop_integration(&mut self, cx: &mut gpui::Context<Self>) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.install_linux_desktop_integration(cx);
        });
    }

    fn push_toast(
        &mut self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.push_toast(kind, message, cx);
        });
    }

    fn open_external_url(&mut self, url: &str) -> Result<(), std::io::Error> {
        super::super::platform_open::open_url(url)
    }
}

impl Render for PopoverHost {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let Some(kind) = self.popover.clone() else {
            return div().into_any_element();
        };

        let close = cx.listener(|this, _e: &MouseDownEvent, _w, cx| this.close_popover(cx));
        let scrim = div()
            .id("popover_scrim")
            .debug_selector(|| "repo_popover_close".to_string())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(gpui::rgba(0x00000000))
            .occlude()
            .on_any_mouse_down(close);

        let popover = self.popover_view(kind, window, cx).into_any_element();

        div()
            .id("popover_layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(scrim)
            .child(popover)
            .into_any_element()
    }
}
impl PopoverHost {
    pub(in super::super) fn popover_view(
        &mut self,
        kind: PopoverKind,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let anchor_source = self
            .popover_anchor
            .clone()
            .unwrap_or_else(|| PopoverAnchor::Point(point(px(64.0), px(64.0))));
        let anchor_is_bounds = matches!(&anchor_source, PopoverAnchor::Bounds(_));
        let window_bounds = window.window_bounds().get_bounds();
        let window_w = window_bounds.size.width;
        let window_h = window_bounds.size.height;
        let margin_x = px(16.0);
        let margin_y = px(16.0);

        let is_app_menu = matches!(&kind, PopoverKind::AppMenu);
        let is_settings = matches!(
            &kind,
            PopoverKind::Settings | PopoverKind::OpenSourceLicenses
        );
        let is_create_branch_or_stash_prompt = matches!(
            &kind,
            PopoverKind::CreateBranch
                | PopoverKind::CreateBranchFromRefPrompt { .. }
                | PopoverKind::StashPrompt
        );
        let is_context_menu = matches!(
            &kind,
            PopoverKind::PullPicker
                | PopoverKind::PushPicker
                | PopoverKind::HistoryBranchFilter { .. }
                | PopoverKind::HistoryColumnSettings
                | PopoverKind::DiffHunkMenu { .. }
                | PopoverKind::DiffEditorMenu { .. }
                | PopoverKind::ConflictResolverInputRowMenu { .. }
                | PopoverKind::ConflictResolverChunkMenu { .. }
                | PopoverKind::ConflictResolverOutputMenu { .. }
                | PopoverKind::CommitMenu { .. }
                | PopoverKind::TagMenu { .. }
                | PopoverKind::StatusFileMenu { .. }
                | PopoverKind::BranchMenu { .. }
                | PopoverKind::BranchSectionMenu { .. }
                | PopoverKind::Repo {
                    kind: RepoPopoverKind::Remote(RemotePopoverKind::Menu { .. }),
                    ..
                }
                | PopoverKind::StashMenu { .. }
                | PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(
                        WorktreePopoverKind::SectionMenu | WorktreePopoverKind::Menu { .. },
                    ),
                    ..
                }
                | PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(
                        SubmodulePopoverKind::SectionMenu | SubmodulePopoverKind::Menu { .. },
                    ),
                    ..
                }
                | PopoverKind::CommitFileMenu { .. }
        );

        let mut anchor_corner = match &kind {
            PopoverKind::PullPicker
            | PopoverKind::PushPicker
            | PopoverKind::CreateBranch
            | PopoverKind::CreateBranchFromRefPrompt { .. }
            | PopoverKind::StashPrompt
            | PopoverKind::StashDropConfirm { .. }
            | PopoverKind::CloneRepo
            | PopoverKind::ResetPrompt { .. }
            | PopoverKind::CreateTagPrompt { .. }
            | PopoverKind::Repo {
                kind:
                    RepoPopoverKind::Remote(
                        RemotePopoverKind::AddPrompt
                        | RemotePopoverKind::EditUrlPrompt { .. }
                        | RemotePopoverKind::RemoveConfirm { .. },
                    ),
                ..
            }
            | PopoverKind::Repo {
                kind:
                    RepoPopoverKind::Worktree(
                        WorktreePopoverKind::AddPrompt
                        | WorktreePopoverKind::OpenPicker
                        | WorktreePopoverKind::RemovePicker
                        | WorktreePopoverKind::RemoveConfirm { .. },
                    ),
                ..
            }
            | PopoverKind::Repo {
                kind:
                    RepoPopoverKind::Submodule(
                        SubmodulePopoverKind::AddPrompt
                        | SubmodulePopoverKind::OpenPicker
                        | SubmodulePopoverKind::RemovePicker
                        | SubmodulePopoverKind::RemoveConfirm { .. },
                    ),
                ..
            }
            | PopoverKind::PushSetUpstreamPrompt { .. }
            | PopoverKind::ForcePushConfirm { .. }
            | PopoverKind::MergeAbortConfirm { .. }
            | PopoverKind::ConflictSaveStageConfirm { .. }
            | PopoverKind::ForceDeleteBranchConfirm { .. }
            | PopoverKind::ForceRemoveWorktreeConfirm { .. }
            | PopoverKind::PullReconcilePrompt { .. }
            | PopoverKind::HistoryBranchFilter { .. }
            | PopoverKind::HistoryColumnSettings
            | PopoverKind::SettingsThemeMenu
            | PopoverKind::SettingsDateFormatMenu
            | PopoverKind::SettingsTimezoneMenu => Corner::TopRight,
            _ => Corner::TopLeft,
        };

        let anchor_for_corner = |corner: Corner| match &anchor_source {
            PopoverAnchor::Point(point) => *point,
            PopoverAnchor::Bounds(bounds) => match corner {
                Corner::TopLeft => bounds.bottom_left(),
                Corner::TopRight => bounds.bottom_right(),
                Corner::BottomLeft => bounds.origin,
                Corner::BottomRight => bounds.top_right(),
            },
        };

        // Some popovers have large minimum widths. If the anchor is close to the edge, the popover
        // can end up constrained to a very narrow width (making inputs unusably small). Prefer the
        // side with more horizontal space in those cases.
        let mut anchor = anchor_for_corner(anchor_corner);
        let min_preferred_w = px(640.0);
        let space_left = (anchor.x - margin_x).max(px(0.0));
        let space_right = (window_w - margin_x - anchor.x).max(px(0.0));
        match anchor_corner {
            Corner::TopRight if space_left < min_preferred_w && space_right > space_left => {
                anchor_corner = Corner::TopLeft;
            }
            Corner::BottomRight if space_left < min_preferred_w && space_right > space_left => {
                anchor_corner = Corner::BottomLeft;
            }
            Corner::TopLeft if space_right < min_preferred_w && space_left > space_right => {
                anchor_corner = Corner::TopRight;
            }
            Corner::BottomLeft if space_right < min_preferred_w && space_left > space_right => {
                anchor_corner = Corner::BottomRight;
            }
            _ => {}
        }
        anchor = anchor_for_corner(anchor_corner);

        let panel = match kind {
            PopoverKind::RepoPicker => repo_picker::panel(self, cx),
            PopoverKind::RecentRepositoryPicker => recent_repo_picker::panel(self, cx),
            PopoverKind::Settings => settings::panel(self, cx),
            PopoverKind::OpenSourceLicenses => open_source_licenses::panel(self, cx),
            PopoverKind::BranchPicker => branch_picker::panel(self, cx),
            PopoverKind::CreateBranch => create_branch::panel(self, cx),
            PopoverKind::CreateBranchFromRefPrompt { repo_id, target } => {
                create_branch_from_ref_prompt::panel(self, repo_id, target, cx)
            }
            PopoverKind::CheckoutRemoteBranchPrompt {
                repo_id,
                remote,
                branch,
            } => checkout_remote_branch_prompt::panel(self, repo_id, remote, branch, cx),
            PopoverKind::StashPrompt => stash_prompt::panel(self, cx),
            PopoverKind::StashDropConfirm {
                repo_id,
                index,
                message,
            } => stash_drop_confirm::panel(self, repo_id, index, message, cx),
            PopoverKind::CloneRepo => clone_repo::panel(self, cx),
            PopoverKind::ResetPrompt {
                repo_id,
                target,
                mode,
            } => reset_prompt::panel(self, repo_id, target, mode, cx),
            PopoverKind::CreateTagPrompt { repo_id, target } => {
                create_tag_prompt::panel(self, repo_id, target, cx)
            }
            PopoverKind::Repo { repo_id, kind } => match kind {
                RepoPopoverKind::Remote(remote_kind) => match remote_kind {
                    RemotePopoverKind::AddPrompt => remote_add_prompt::panel(self, repo_id, cx),
                    RemotePopoverKind::EditUrlPrompt { name, kind } => {
                        remote_edit_url_prompt::panel(self, repo_id, name, kind, cx)
                    }
                    RemotePopoverKind::RemoveConfirm { name } => {
                        remote_remove_confirm::panel(self, repo_id, name, cx)
                    }
                    RemotePopoverKind::DeleteBranchConfirm { remote, branch } => {
                        delete_remote_branch_confirm::panel(self, repo_id, remote, branch, cx)
                    }
                    RemotePopoverKind::Menu { name } => self
                        .context_menu_view(
                            PopoverKind::remote(repo_id, RemotePopoverKind::Menu { name }),
                            cx,
                        )
                        .min_w(px(160.0))
                        .max_w(px(320.0)),
                },
                RepoPopoverKind::Worktree(worktree_kind) => match worktree_kind {
                    WorktreePopoverKind::SectionMenu => self
                        .context_menu_view(
                            PopoverKind::worktree(repo_id, WorktreePopoverKind::SectionMenu),
                            cx,
                        )
                        .min_w(px(160.0))
                        .max_w(px(320.0)),
                    WorktreePopoverKind::Menu { path } => self
                        .context_menu_view(
                            PopoverKind::worktree(repo_id, WorktreePopoverKind::Menu { path }),
                            cx,
                        )
                        .min_w(px(160.0))
                        .max_w(px(320.0)),
                    WorktreePopoverKind::AddPrompt => worktree_add_prompt::panel(self, repo_id, cx),
                    WorktreePopoverKind::OpenPicker => {
                        worktree_open_picker::panel(self, repo_id, cx)
                    }
                    WorktreePopoverKind::RemovePicker => {
                        worktree_remove_picker::panel(self, repo_id, cx)
                    }
                    WorktreePopoverKind::RemoveConfirm { path } => {
                        worktree_remove_confirm::panel(self, repo_id, path, cx)
                    }
                },
                RepoPopoverKind::Submodule(submodule_kind) => match submodule_kind {
                    SubmodulePopoverKind::SectionMenu => self
                        .context_menu_view(
                            PopoverKind::submodule(repo_id, SubmodulePopoverKind::SectionMenu),
                            cx,
                        )
                        .min_w(px(160.0))
                        .max_w(px(320.0)),
                    SubmodulePopoverKind::Menu { path } => self
                        .context_menu_view(
                            PopoverKind::submodule(repo_id, SubmodulePopoverKind::Menu { path }),
                            cx,
                        )
                        .min_w(px(160.0))
                        .max_w(px(320.0)),
                    SubmodulePopoverKind::AddPrompt => {
                        submodule_add_prompt::panel(self, repo_id, cx)
                    }
                    SubmodulePopoverKind::OpenPicker => {
                        submodule_open_picker::panel(self, repo_id, cx)
                    }
                    SubmodulePopoverKind::RemovePicker => {
                        submodule_remove_picker::panel(self, repo_id, cx)
                    }
                    SubmodulePopoverKind::RemoveConfirm { path } => {
                        submodule_remove_confirm::panel(self, repo_id, path, cx)
                    }
                },
            },
            PopoverKind::FileHistory { repo_id, path } => {
                file_history::panel(self, repo_id, path, cx)
            }
            PopoverKind::PushSetUpstreamPrompt { repo_id, remote } => {
                push_set_upstream_prompt::panel(self, repo_id, remote, cx)
            }
            PopoverKind::ForcePushConfirm { repo_id } => {
                force_push_confirm::panel(self, repo_id, cx)
            }
            PopoverKind::MergeAbortConfirm { repo_id } => {
                merge_abort_confirm::panel(self, repo_id, cx)
            }
            PopoverKind::ConflictSaveStageConfirm {
                repo_id,
                path,
                has_conflict_markers,
                unresolved_blocks,
            } => conflict_save_stage_confirm::panel(
                self,
                repo_id,
                &path,
                has_conflict_markers,
                unresolved_blocks,
                cx,
            ),
            PopoverKind::ForceDeleteBranchConfirm { repo_id, name } => {
                force_delete_branch_confirm::panel(self, repo_id, name, cx)
            }
            PopoverKind::ForceRemoveWorktreeConfirm { repo_id, path } => {
                force_remove_worktree_confirm::panel(self, repo_id, path, cx)
            }
            PopoverKind::DiscardChangesConfirm {
                repo_id,
                area,
                path,
            } => discard_changes_confirm::panel(self, repo_id, area, path.clone(), cx),
            PopoverKind::PullReconcilePrompt { repo_id } => {
                pull_reconcile_prompt::panel(self, repo_id, cx)
            }
            PopoverKind::HistoryBranchFilter { repo_id } => self
                .context_menu_view(PopoverKind::HistoryBranchFilter { repo_id }, cx)
                .min_w(px(160.0))
                .max_w(px(220.0)),
            PopoverKind::HistoryColumnSettings => self
                .context_menu_view(PopoverKind::HistoryColumnSettings, cx)
                .min_w(px(160.0))
                .max_w(px(220.0)),
            PopoverKind::SettingsThemeMenu => self
                .context_menu_view(PopoverKind::SettingsThemeMenu, cx)
                .min_w(px(180.0))
                .max_w(px(260.0)),
            PopoverKind::SettingsDateFormatMenu => self
                .context_menu_view(PopoverKind::SettingsDateFormatMenu, cx)
                .min_w(px(220.0))
                .max_w(px(320.0)),
            PopoverKind::SettingsTimezoneMenu => self
                .context_menu_view(PopoverKind::SettingsTimezoneMenu, cx)
                .min_w(px(260.0))
                .max_w(px(420.0)),
            PopoverKind::PullPicker => self.context_menu_view(PopoverKind::PullPicker, cx),
            PopoverKind::PushPicker => self.context_menu_view(PopoverKind::PushPicker, cx),
            PopoverKind::DiffHunks => diff_hunks::panel(self, cx),
            PopoverKind::CommitMenu { repo_id, commit_id } => self
                .context_menu_view(PopoverKind::CommitMenu { repo_id, commit_id }, cx)
                .min_w(px(160.0))
                .max_w(px(320.0)),
            PopoverKind::TagMenu { repo_id, commit_id } => self
                .context_menu_view(PopoverKind::TagMenu { repo_id, commit_id }, cx)
                .min_w(px(160.0))
                .max_w(px(320.0)),
            PopoverKind::DiffHunkMenu { repo_id, src_ix } => self
                .context_menu_view(PopoverKind::DiffHunkMenu { repo_id, src_ix }, cx)
                .min_w(px(160.0))
                .max_w(px(220.0)),
            PopoverKind::DiffEditorMenu {
                repo_id,
                area,
                path,
                hunk_patch,
                hunks_count,
                lines_patch,
                discard_lines_patch,
                lines_count,
                copy_text,
            } => self
                .context_menu_view(
                    PopoverKind::DiffEditorMenu {
                        repo_id,
                        area,
                        path,
                        hunk_patch,
                        hunks_count,
                        lines_patch,
                        discard_lines_patch,
                        lines_count,
                        copy_text,
                    },
                    cx,
                )
                .w(px(220.0))
                .min_w(px(160.0))
                .max_w(px(260.0)),
            PopoverKind::ConflictResolverInputRowMenu {
                line_label,
                line_target,
                chunk_label,
                chunk_target,
            } => self
                .context_menu_view(
                    PopoverKind::ConflictResolverInputRowMenu {
                        line_label,
                        line_target,
                        chunk_label,
                        chunk_target,
                    },
                    cx,
                )
                .min_w(px(180.0))
                .max_w(px(280.0)),
            PopoverKind::ConflictResolverChunkMenu {
                conflict_ix,
                has_base,
                is_three_way,
                selected_choices,
                output_line_ix,
            } => self
                .context_menu_view(
                    PopoverKind::ConflictResolverChunkMenu {
                        conflict_ix,
                        has_base,
                        is_three_way,
                        selected_choices,
                        output_line_ix,
                    },
                    cx,
                )
                .w(px(220.0))
                .min_w(px(190.0))
                .max_w(px(280.0)),
            PopoverKind::ConflictResolverOutputMenu {
                cursor_line,
                selected_text,
                has_source_a,
                has_source_b,
                has_source_c,
                is_three_way,
            } => self
                .context_menu_view(
                    PopoverKind::ConflictResolverOutputMenu {
                        cursor_line,
                        selected_text,
                        has_source_a,
                        has_source_b,
                        has_source_c,
                        is_three_way,
                    },
                    cx,
                )
                .w(px(240.0))
                .min_w(px(200.0))
                .max_w(px(300.0)),
            PopoverKind::StatusFileMenu {
                repo_id,
                area,
                path,
            } => self.context_menu_view(
                PopoverKind::StatusFileMenu {
                    repo_id,
                    area,
                    path,
                },
                cx,
            ),
            PopoverKind::BranchMenu {
                repo_id,
                section,
                name,
            } => self.context_menu_view(
                PopoverKind::BranchMenu {
                    repo_id,
                    section,
                    name,
                },
                cx,
            ),
            PopoverKind::BranchSectionMenu { repo_id, section } => {
                self.context_menu_view(PopoverKind::BranchSectionMenu { repo_id, section }, cx)
            }
            PopoverKind::StashMenu {
                repo_id,
                index,
                message,
            } => self
                .context_menu_view(
                    PopoverKind::StashMenu {
                        repo_id,
                        index,
                        message,
                    },
                    cx,
                )
                .min_w(px(180.0))
                .max_w(px(360.0)),
            PopoverKind::CommitFileMenu {
                repo_id,
                commit_id,
                path,
            } => self.context_menu_view(
                PopoverKind::CommitFileMenu {
                    repo_id,
                    commit_id,
                    path,
                },
                cx,
            ),
            PopoverKind::AppMenu => app_menu::panel(self, cx),
        };

        let is_right = matches!(anchor_corner, Corner::TopRight | Corner::BottomRight);
        let use_accent_border =
            is_context_menu || is_app_menu || is_create_branch_or_stash_prompt || is_settings;
        let popover_border_color = if use_accent_border {
            with_alpha(theme.colors.accent, 0.90)
        } else {
            gpui::rgba(crate::view::chrome::WINDOW_OUTLINE_RGBA)
        };
        let gap_y = if is_app_menu || is_settings {
            crate::view::chrome::TITLE_BAR_HEIGHT
        } else if anchor_is_bounds {
            px(1.0)
        } else if is_right {
            px(10.0)
        } else {
            px(8.0)
        };

        let mut context_menu_max_panel_h: Option<Pixels> = None;
        if is_context_menu || is_settings {
            let (below_anchor_y, above_anchor_y) = match &anchor_source {
                PopoverAnchor::Point(_) => (anchor.y, anchor.y),
                PopoverAnchor::Bounds(bounds) => (bounds.bottom_left().y, bounds.origin.y),
            };
            let below = (window_h - margin_y) - (below_anchor_y + gap_y);
            let above = (above_anchor_y - gap_y) - margin_y;
            if below < px(240.0) && above > below {
                anchor_corner = match anchor_corner {
                    Corner::TopLeft => Corner::BottomLeft,
                    Corner::TopRight => Corner::BottomRight,
                    corner => corner,
                };
            }
            if anchor_is_bounds {
                anchor = anchor_for_corner(anchor_corner);
            }

            let popover_edge_y = match anchor_corner {
                Corner::TopLeft | Corner::TopRight => anchor.y + gap_y,
                Corner::BottomLeft | Corner::BottomRight => anchor.y - gap_y,
            };
            let max_popover_h = match anchor_corner {
                Corner::TopLeft | Corner::TopRight => (window_h - margin_y) - popover_edge_y,
                Corner::BottomLeft | Corner::BottomRight => popover_edge_y - margin_y,
            }
            .max(px(0.0));
            let max_panel_h = (max_popover_h - px(12.0)).max(px(0.0));
            context_menu_max_panel_h = Some(max_panel_h);
        }

        let offset_y = match anchor_corner {
            Corner::TopLeft | Corner::TopRight => gap_y,
            Corner::BottomLeft | Corner::BottomRight => -gap_y,
        };

        let panel = if let Some(max_panel_h) = context_menu_max_panel_h {
            div()
                .id("context_menu_scroll")
                .min_h(px(0.0))
                .max_h(max_panel_h)
                .overflow_y_scroll()
                .child(panel)
                .into_any_element()
        } else {
            panel.into_any_element()
        };

        anchored()
            .position(anchor)
            .anchor(anchor_corner)
            .offset(point(px(0.0), offset_y))
            .child(
                div()
                    .id("app_popover")
                    .debug_selector(|| "app_popover".to_string())
                    .on_any_mouse_down(|_e, _w, cx| cx.stop_propagation())
                    .occlude()
                    .bg(theme.colors.surface_bg_elevated)
                    .border_1()
                    .border_color(popover_border_color)
                    .rounded(px(theme.radii.panel))
                    .shadow_lg()
                    .overflow_hidden()
                    .p_1()
                    .child(panel),
            )
    }
}

fn clone_repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    let name = last.strip_suffix(".git").unwrap_or(last).trim();
    if name.is_empty() {
        "repo".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests;
