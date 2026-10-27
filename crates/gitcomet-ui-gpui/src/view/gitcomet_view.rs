use super::*;

impl GitCometView {
    fn hook_activity_workflow_repo(kind: &PopoverKind) -> Option<RepoId> {
        match kind {
            PopoverKind::HookActivity { repo_id, .. } => Some(*repo_id),
            _ => None,
        }
    }

    pub(in crate::view) fn set_hook_activity_dialog_repo(
        &mut self,
        repo_id: Option<RepoId>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.toast_host.update(cx, |host, cx| {
            host.set_hook_activity_dialog_repo(repo_id, cx)
        });
    }

    pub(in crate::view) fn minimize_hook_activity_chains(
        &mut self,
        chains: impl IntoIterator<Item = (RepoId, GitOperationId)>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.minimized_hook_activity_chains.extend(chains);
        self.pending_hook_activity_open = None;
        self.set_hook_activity_dialog_repo(None, cx);
        cx.notify();
    }

    pub(in crate::view) fn minimize_hook_activity_repo(
        &mut self,
        repo_id: RepoId,
        chains: impl IntoIterator<Item = (RepoId, GitOperationId)>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.minimized_hook_activity_repos.insert(repo_id);
        self.sync_minimized_hook_activity_indicator(cx);
        self.minimize_hook_activity_chains(chains, cx);
    }

    pub(in crate::view) fn resume_hook_activity_auto_open(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        self.minimized_hook_activity_repos.remove(&repo_id);
        self.sync_minimized_hook_activity_indicator(cx);
        self.minimized_hook_activity_chains
            .retain(|(minimized_repo_id, _)| *minimized_repo_id != repo_id);
        cx.notify();
    }

    pub(super) fn sync_minimized_hook_activity_indicator(&mut self, cx: &mut gpui::Context<Self>) {
        self.bottom_status_bar.update(cx, |bar, cx| {
            bar.set_minimized_hook_activity_repos(&self.minimized_hook_activity_repos, cx);
        });
    }

    pub(in crate::view) fn hook_activity_workflow_is_open(&self, cx: &App) -> bool {
        self.popover_host.read(cx).is_hook_activity_workflow_open()
    }

    pub(in crate::view) fn hook_activity_workflow_repo_id(&self, cx: &App) -> Option<RepoId> {
        self.popover_host.read(cx).hook_activity_workflow_repo_id()
    }

    pub(in crate::view) fn open_popover_at(
        &mut self,
        kind: impl Into<PopoverRequest>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut kind: PopoverRequest = kind.into();
        if self.window_gated && !kind.kind.survives_gate() {
            return;
        }
        kind.new_source = true;
        self.set_hook_activity_dialog_repo(Self::hook_activity_workflow_repo(&kind.kind), cx);
        self.history_refs_hover_host
            .update(cx, |host, cx| host.close(cx));
        self.popover_host.update(cx, |host, cx| {
            host.restore_history_binding();
            host.open_popover_at(kind, anchor, window, cx)
        });
    }

    /// Close the submodule trust popover only while it is showing its pending
    /// spinner for `repo_id` (no trust prompt yet). Used when a background trust
    /// check resolves to a silent proceed or an error, so the spinner does not
    /// linger. A no-op if the user already dismissed it or another popover is up.
    pub(in crate::view) fn close_submodule_trust_spinner(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind = PopoverKind::submodule(repo_id, SubmodulePopoverKind::TrustConfirm);
        self.popover_host.update(cx, |host, cx| {
            if host.is_kind_open(&kind) {
                host.close_popover(cx);
            }
        });
    }

    pub(in crate::view) fn open_popover_centered(
        &mut self,
        kind: impl Into<PopoverRequest>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut kind: PopoverRequest = kind.into();
        if self.window_gated && !kind.kind.survives_gate() {
            return;
        }
        kind.new_source = true;
        if let PopoverKind::HookActivity { repo_id, .. } = &kind.kind {
            self.pending_hook_activity_open = None;
            self.minimized_hook_activity_chains
                .retain(|(suppressed_repo_id, _)| suppressed_repo_id != repo_id);
        }
        self.set_hook_activity_dialog_repo(Self::hook_activity_workflow_repo(&kind.kind), cx);
        self.history_refs_hover_host
            .update(cx, |host, cx| host.close(cx));
        self.popover_host.update(cx, |host, cx| {
            host.restore_history_binding();
            host.open_popover_centered(kind, window, cx);
        });
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn open_clone_repository_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover_centered(PopoverKind::CloneRepo, window, cx);
    }

    pub(in crate::view) fn open_popover_for_bounds(
        &mut self,
        kind: impl Into<PopoverRequest>,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut kind: PopoverRequest = kind.into();
        if self.window_gated && !kind.kind.survives_gate() {
            return;
        }
        kind.new_source = true;
        self.set_hook_activity_dialog_repo(Self::hook_activity_workflow_repo(&kind.kind), cx);
        self.history_refs_hover_host
            .update(cx, |host, cx| host.close(cx));
        self.popover_host.update(cx, |host, cx| {
            host.restore_history_binding();
            host.open_popover_for_bounds(kind, anchor_bounds, window, cx)
        });
    }

    pub(super) fn open_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        // Both are scrim-backed modals over the same overlay layer, so opening
        // one on top of the other stacks two scrims and leaves the lower one
        // behind when the upper is dismissed. Closing first also means the
        // focus captured below is the one the closed modal handed back, not its
        // own input.
        if self.reveal_commit_open {
            self.close_reveal_commit(window, cx);
        }
        self.command_palette_open = true;
        let restore_focus = window
            .focused(cx)
            .or_else(|| self.pre_palette_focus.clone());
        let fallback_focus = self.diff_renderer.read(cx).diff_panel_focus_handle.clone();
        let context = self.command_palette_context(cx);
        self.command_palette.update(cx, |palette, cx| {
            palette.open(restore_focus, fallback_focus, context, window, cx);
        });
    }

    /// The state palette commands are enabled against. Mirrors the conditions
    /// the action bar and menus use, so a command is greyed out exactly when
    /// its button or menu entry would be.
    pub(super) fn command_palette_context(&self, cx: &App) -> command_palette::PaletteContext {
        let repo = self.active_repo();
        let host = self.popover_host.read(cx);
        let annex = repo.and_then(|repo| match &repo.large_file_support {
            gitcomet_state::model::Loadable::Ready(support) => Some(&support.annex),
            _ => None,
        });
        command_palette::PaletteContext {
            has_active_repo: repo.is_some(),
            is_bare: repo.is_some_and(RepoState::is_bare),
            pull_unavailable: repo
                .and_then(|repo| pull_unavailable_reason(repo, &self.state.large_file_settings)),
            external_editor: self.effective_external_editor().is_some(),
            merging: repo.is_some_and(merge_in_progress),
            sequencer: repo.is_some_and(|repo| {
                active_sequencer_state(repo) != gitcomet_core::services::SequencerState::None
            }),
            sequencer_busy: repo.is_some_and(|repo| repo.sequencer_actions_in_flight > 0),
            unresolved_conflicts: repo.is_some_and(|repo| repo.has_unstaged_conflicts),
            push_with_tags_unavailable: gitcomet_core::tag_push::TagPushMode::ALL
                .map(|mode| host.push_with_tags_unavailable(mode)),
            remote_web_page_unavailable: repo
                .and_then(|repo| remote_web_request(repo).unavailable_reason()),
            git_lfs_missing: self.state.large_file_tools.git_lfs.is_not_found(),
            repo_uses_lfs: repo.is_some_and(|repo| {
                matches!(
                    &repo.large_file_support,
                    gitcomet_state::model::Loadable::Ready(support) if support.lfs.in_use()
                )
            }),
            git_annex_missing: self.state.large_file_tools.git_annex.is_not_found(),
            repo_uses_annex: annex.is_some_and(|annex| annex.in_use()),
            annex_initialized: annex.is_some_and(|annex| annex.initialized()),
            annex_adjusted: repo.is_some_and(|repo| repo.annex_adjusted_branch().is_some()),
        }
    }

    pub(super) fn close_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.command_palette_open = false;
        self.command_palette
            .update(cx, |palette, cx| palette.close(window, cx));
    }

    pub(super) fn command_palette_did_close(
        &mut self,
        command: Option<&str>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.command_palette_open = false;
        if let Some(command) = command {
            self.execute_command(command, Some(window), cx);
        }
    }

    pub(crate) fn toggle_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.command_palette_open {
            self.close_command_palette(window, cx);
        } else {
            self.open_command_palette(window, cx);
        }
    }

    pub(super) fn open_reveal_commit(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        // See `open_command_palette`: two stacked modal scrims are never right.
        if self.command_palette_open {
            self.close_command_palette(window, cx);
        }
        self.reveal_commit_open = true;
        let restore_focus = window
            .focused(cx)
            .or_else(|| self.pre_palette_focus.clone());
        let fallback_focus = self.diff_renderer.read(cx).diff_panel_focus_handle.clone();
        let repo_id = self.active_repo_id();
        self.reveal_commit_dialog.update(cx, |dialog, cx| {
            dialog.open(restore_focus, fallback_focus, repo_id, window, cx);
        });
    }

    pub(super) fn close_reveal_commit(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.reveal_commit_open = false;
        self.reveal_commit_dialog
            .update(cx, |dialog, cx| dialog.close(window, cx));
    }

    pub(super) fn reveal_commit_did_close(
        &mut self,
        target: Option<CommitId>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.reveal_commit_open = false;
        let (Some(repo_id), Some(target)) = (self.active_repo_id(), target) else {
            return;
        };
        self.main_pane.update(cx, |main, cx| {
            main.reveal_history_commit(
                repo_id,
                target,
                Some(gitcomet_core::domain::LogScope::AllBranches),
                cx,
            );
        });
    }

    pub(crate) fn toggle_reveal_commit(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.reveal_commit_open {
            self.close_reveal_commit(window, cx);
            return;
        }
        // Nothing to reveal outside a normal window, or without a repository.
        if !command_palette_available(self.view_mode)
            || !self.shell_action_allowed(super::shell_policy::ShellAction::Repository)
        {
            return;
        }
        self.open_reveal_commit(window, cx);
    }

    /// Whether a popover, dialog, prompt, or context menu is currently open
    /// (all are tracked as a `PopoverKind` by the popover host).
    pub(in crate::view) fn is_overlay_open(&self, cx: &App) -> bool {
        // The collapsed-sidebar section popover covers the history view too, so it
        // must suppress ref hovers the same way the popover host does.
        self.popover_host.read(cx).is_open() || self.sidebar_collapsed_popover.is_some()
    }

    pub(in crate::view) fn show_history_refs_hover(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        source_bounds: Bounds<Pixels>,
        items: Arc<[HistoryRefListItem]>,
        pointer: Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Don't surface the refs hover while an overlay (popover, dialog, or
        // context menu) is open on top of the history view — the history canvas
        // handles mouse-move at the window level, so it still fires under the
        // overlay. If the open overlay is the hover's own item menu, leave the
        // existing hover in place.
        if self.is_overlay_open(cx) && !self.history_refs_hover_host.read(cx).is_item_menu_open() {
            self.close_history_refs_hover(cx);
            return;
        }
        self.history_refs_hover_host.update(cx, |host, cx| {
            host.show(repo_id, commit_id, source_bounds, items, pointer, cx)
        });
    }

    pub(in crate::view) fn show_commit_message_hover(
        &mut self,
        next: CommitMessageHoverState,
        pointer: Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Same reasoning as the refs hover: the history canvas listens for
        // mouse-move at the window level, so it still fires under an open
        // overlay and the card would surface on top of it.
        if self.is_overlay_open(cx) {
            self.dismiss_commit_message_hover(cx);
            return;
        }
        self.commit_message_hover_host
            .update(cx, |host, cx| host.show(next, pointer, cx));
    }

    pub(in crate::view) fn dismiss_commit_message_hover(&mut self, cx: &mut gpui::Context<Self>) {
        self.commit_message_hover_host
            .update(cx, |host, cx| host.dismiss(cx));
    }

    pub(in crate::view) fn close_history_refs_hover(&mut self, cx: &mut gpui::Context<Self>) {
        self.history_refs_hover_host
            .update(cx, |host, cx| host.close(cx));
    }

    pub(in crate::view) fn dismiss_history_refs_menus(&mut self, cx: &mut gpui::Context<Self>) {
        self.close_history_refs_hover(cx);

        let history_refs_menu_open =
            self.active_context_menu_invoker
                .as_ref()
                .is_some_and(|invoker| {
                    invoker
                        .as_ref()
                        .starts_with(HISTORY_REFS_HOVER_MENU_INVOKER_PREFIX)
                });
        if history_refs_menu_open {
            self.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
        }
    }

    pub(in crate::view) fn set_history_refs_hover_item_menu_open(
        &mut self,
        open: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_refs_hover_host
            .update(cx, |host, cx| host.set_item_menu_open(open, cx));
    }

    pub(in crate::view) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next.clone();

        let sidebar_pane = self.sidebar_pane.clone();
        let diff_renderer = self.diff_renderer.clone();
        let main_pane = self.main_pane.clone();
        let details_pane = self.details_pane.clone();
        let repo_tabs_bar = self.repo_tabs_bar.clone();
        let action_bar = self.action_bar.clone();
        let bottom_status_bar = self.bottom_status_bar.clone();

        cx.defer(move |cx| {
            sidebar_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            diff_renderer.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            main_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            details_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            repo_tabs_bar.update(cx, |bar, cx| {
                bar.set_active_context_menu_invoker(next.clone(), cx);
            });
            action_bar.update(cx, |bar, cx| {
                bar.set_active_context_menu_invoker(next.clone(), cx);
            });
            bottom_status_bar.update(cx, |bar, cx| {
                bar.set_active_context_menu_invoker(next.clone(), cx);
            });
        });
    }

    #[cfg(test)]
    pub fn new(
        store: AppStore,
        events: smol::channel::Receiver<StoreEvent>,
        initial_path: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let config = match initial_path {
            Some(path) => GitCometViewConfig::normal_with_initial_repository(path, None),
            None => GitCometViewConfig::normal(None),
        };
        Self::new_with_config(store, events, config, window, cx)
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn new_with_config(
        store: AppStore,
        events: smol::channel::Receiver<StoreEvent>,
        config: GitCometViewConfig,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let bootstrap = super::window_bootstrap::PreparedWindowBootstrap::new(&store, config, cx);
        Self::new_with_bootstrap(store, events, bootstrap, window, cx)
    }

    pub(crate) fn new_with_bootstrap(
        store: AppStore,
        events: smol::channel::Receiver<StoreEvent>,
        mut bootstrap: super::window_bootstrap::PreparedWindowBootstrap,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let _stage = gitcomet_core::startup_trace::span("view_init");
        let mut startup_repo_runtime_adoption = bootstrap.attach_git_probe_handoff();
        let super::window_bootstrap::PreparedWindowBootstrap {
            config,
            ui_session,
            mut ui_preferences,
            workspace_id,
            persisted_workspace_repo_paths,
            persisted_workspace_active_repository,
            initial_window_placement,
            workspace_theme_key,
            entry_denials,
            mut startup_repo_bootstrap_pending,
            startup_repo_open_failure_revision,
            deferred_repo_bootstrap,
            pending_repo_open_reservations,
            pending_repo_open_active,
            ..
        } = bootstrap;
        let GitCometViewConfig {
            initial_path: _,
            initial_repository_launch_mode: _,
            view_mode,
            focused_diff,
            focused_mergetool,
            focused_mergetool_exit_code,
            startup_crash_report,
            workspace: _,
        } = config;
        crate::environment::track_window(window, cx);
        let focused_mergetool_labels = focused_mergetool.as_ref().map(|cfg| cfg.labels.clone());
        let focused_mergetool_bootstrap = if view_mode == GitCometViewMode::FocusedMergetool {
            focused_mergetool
                .clone()
                .map(FocusedMergetoolBootstrap::from_view_config)
        } else {
            None
        };
        let store = Arc::new(store);

        image_preview::Preferences {
            mode: image_preview::ImageSamplingMode::parse(
                ui_session.image_sampling_mode.as_deref(),
            ),
            filter: image_preview::ImageSmoothFilter::parse(
                ui_session.image_smooth_filter.as_deref(),
            ),
        }
        .initialize(cx);
        crate::session_ui::initialize_appearance(&ui_session, cx);
        ui_preferences.appearance.metrics = crate::appearance::current(cx);
        crate::session_ui::ui_scale(&ui_session, cx);
        // This window's scale: the default, unless the window has its own zoom.
        let ui_scale = crate::ui_scale::current(cx);
        ui_preferences.appearance.ui_scale_percent = ui_scale.percent;
        let _font_preferences = {
            let _stage = gitcomet_core::startup_trace::span("font_preferences");
            crate::session_ui::font_preferences(window, &ui_session, cx)
        };
        let restored_sidebar_width = ui_preferences.window.sidebar_width;
        let restored_details_width = ui_preferences.window.details_width;
        let restored_sidebar_collapsed = ui_preferences.window.sidebar_collapsed;
        let _ = crate::theme::ensure_user_themes_dir_exists();
        let theme_mode = ui_preferences.appearance.theme_mode.clone();
        let workspace_theme_mode = workspace_theme_key.as_deref().and_then(ThemeMode::from_key);
        let window_appearance = window.appearance();
        let initial_theme = workspace_theme_mode
            .as_ref()
            .unwrap_or(&theme_mode)
            .resolve_theme(window_appearance)
            .with_appearance(crate::appearance::current(cx));
        let date_time_format = ui_preferences.appearance.date_time_format;
        let timezone = ui_preferences.appearance.timezone;
        let show_timezone = ui_preferences.appearance.show_timezone;
        let change_tracking_view = ui_preferences.change_tracking.view;
        let file_list_layout = ui_preferences.file_lists.layout;
        let file_list_sort = ui_preferences.file_lists.sort;
        cx.set_global(crate::view::FileListDefaults {
            layout: file_list_layout,
            sort: file_list_sort,
        });
        let terminal_preferences = ui_preferences.terminal.clone();
        let diff_scroll_sync = ui_preferences.diff.scroll_sync;
        let diff_content_mode = ui_preferences.diff.content_mode;
        let diff_whitespace_mode = ui_preferences.diff.whitespace_mode;
        let diff_view_mode = ui_preferences.diff.view_mode;
        let annotate_enabled = ui_preferences.diff.annotate_enabled;
        let diff_reveal_whitespace_chars = ui_preferences.diff.reveal_whitespace_chars;
        let diff_word_wrap = ui_preferences.diff.word_wrap;
        let diff_tab_size = ui_preferences.diff.tab_size;
        let diff_show_line_numbers = ui_preferences.diff.show_line_numbers;
        let auto_save_file_edits = ui_preferences.file_editing.auto_save;
        let remote_markdown_image_policy = ui_preferences.security.remote_markdown_images;
        let remote_url_policy = ui_preferences.security.remote_url_policy;
        let check_for_updates_on_startup = ui_preferences.security.check_for_updates_on_startup;
        let commit_push_after_enabled = ui_preferences.repository.commit_push_after_enabled;
        let initial_state = store.snapshot();
        // The worker queued Git adoption before the restore. If it is already
        // reflected here, no later snapshot needs to suppress that transition.
        if startup_repo_runtime_adoption.as_ref() == Some(&initial_state.git_runtime) {
            startup_repo_runtime_adoption = None;
        }
        let ui_model = cx.new(|cx| {
            AppUiModel::new_with_preferences(
                Arc::clone(&initial_state),
                ui_preferences.clone(),
                Arc::downgrade(&store),
                cx,
            )
        });
        let tooltip_host = cx.new(|_cx| TooltipHost::new(initial_theme));
        let diff_environment = super::hosted::environment::DiffPaneEnvironment {
            services: super::panes::diff::services::DiffServices {
                store: store.clone().into(),
                commands: super::diff_commands::for_window(cx.weak_entity()),
                documents: ui_model.read(cx).documents.clone(),
                tooltip: tooltip_host.downgrade(),
            },
            preferences: super::hosted::environment::DiffPreferences::Window(ui_model.downgrade()),
            window: window.window_handle(),
        };
        let extension_window = super::extension_host::ExtensionWindow::new(
            window,
            &store,
            Arc::clone(&initial_state),
            initial_theme,
            diff_environment.clone(),
            if view_mode == GitCometViewMode::FocusedMergetool {
                gitcomet_core::identity::WindowKind::FocusedMergetool
            } else if view_mode == GitCometViewMode::FocusedDiff {
                gitcomet_core::identity::WindowKind::FocusedDiff
            } else {
                gitcomet_core::identity::WindowKind::Main
            },
            cx,
        );
        let focused_diff_pane = focused_diff.and_then(|config| {
            let host = extension_window.as_ref()?.host();
            let path = config
                .display_path
                .unwrap_or_else(|| format!("{} vs {}", config.label_left, config.label_right));
            let snapshot = match config.input {
                crate::FocusedDiffInput::FilePair { path, old, new } => {
                    gitcomet_extension_api::DiffSnapshot::new(path, old, new)
                }
                crate::FocusedDiffInput::Patch(text) => {
                    let id = gitcomet_core::domain::CommitId("snapshot".into());
                    let target =
                        gitcomet_core::domain::DiffTarget::commit_range(id.clone(), Some(id), None);
                    let patch = gitcomet_core::domain::Diff::from_unified_owned(target, text);
                    gitcomet_extension_api::DiffSnapshot::from_patch(path, patch)
                }
            };
            let mut policy = gitcomet_extension_api::DiffPanePolicy::default();
            policy.allow_edit = false;
            policy.allow_stage = false;
            policy.allow_annotate = false;
            policy.close_button = false;
            policy.escape_clears_target = false;
            policy.file_navigation = false;
            host.create_snapshot_pane(
                snapshot,
                gitcomet_extension_api::DiffPaneOptions {
                    policy,
                    ..Default::default()
                },
                cx,
            )
            .ok()
            .map(|pane| FocusedDiffPane {
                pane,
                labels: [config.label_left.into(), config.label_right.into()],
            })
        });
        if !initial_state.repos.is_empty()
            || initial_state.repo_open_failure_revision > startup_repo_open_failure_revision
        {
            startup_repo_bootstrap_pending = false;
        }

        let ui_model_subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let should_quit = crate::startup_probe::observe_app_state(next.as_ref());
            let should_notify = this.apply_state_snapshot(next, cx);
            this.process_filesystem_results(cx);
            if should_notify {
                cx.notify();
            }
            if should_quit {
                crate::app::mark_clean_shutdown_from_view(cx);
                cx.quit();
            }
        });

        let weak_view = cx.weak_entity();
        // The receiver retains a coalesced StateChanged even if repository loading
        // finished before this view existed. Its first poll observes startup readiness.
        let poller = Poller::start(Arc::clone(&store), events, ui_model.downgrade(), window, cx);

        let title_bar = cx.new(|cx| {
            TitleBarView::new(
                initial_theme,
                weak_view.clone(),
                titlebar_repo_tab_actions_enabled(view_mode, !initial_state.repos.is_empty()),
                cx,
            )
        });
        let toast_host = cx.new(|_cx| ToastHost::new(initial_theme, weak_view.clone()));
        let history_refs_hover_host =
            cx.new(|_cx| HistoryRefsHoverHost::new(initial_theme, weak_view.clone()));
        let commit_message_hover_host = cx.new(|_cx| {
            CommitMessageHoverHost::new(initial_theme, Arc::clone(&store), ui_model.clone())
        });
        let repo_tabs_bar = cx.new(|cx| {
            RepoTabsBarView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                weak_view.clone(),
                cx,
            )
        });
        let action_bar = cx.new(|cx| {
            ActionBarView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                weak_view.clone(),
                cx,
            )
        });
        let bottom_status_bar = cx.new(|cx| {
            BottomStatusBarView::new(initial_theme, ui_model.clone(), weak_view.clone(), cx)
        });
        panels::bar_items::connect(&action_bar, &bottom_status_bar, cx);

        let file_history_pane = cx.new(|cx| {
            panes::file_history::FileHistoryPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                panes::file_history::FileHistoryPaneInit {
                    theme: initial_theme,
                    root_view: weak_view.clone(),
                },
                window,
                cx,
            )
        });
        let sidebar_pane = cx.new(|cx| {
            let mut pane = SidebarPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                ui_session.repo_sidebar_collapsed_items.clone(),
                ui_session.repo_sidebar_pinned_branches.clone(),
                weak_view.clone(),
                tooltip_host.downgrade(),
                file_history_pane.clone(),
                cx,
            );
            pane.set_expanded_branches_visible(!restored_sidebar_collapsed, cx);
            pane
        });
        let sidebar_header_subscription = cx.subscribe(
            &sidebar_pane,
            |_, _, _: &panes::SidebarHeaderChanged, cx| cx.notify(),
        );
        let diff_renderer = cx.new(|cx| {
            DiffRenderer::new(
                super::panes::diff::input::DiffPaneInput::selected(initial_state.clone()),
                &ui_preferences,
                diff_environment.services.clone(),
                DiffRendererInit {
                    theme: initial_theme,
                    view_mode,
                    focused_mergetool_labels,
                    focused_mergetool_exit_code: focused_mergetool_exit_code.clone(),
                },
                window,
                cx,
            )
        });
        let main_pane = cx.new(|cx| {
            MainPaneView::new(
                store.clone(),
                ui_model.clone(),
                diff_renderer.clone(),
                weak_view.clone(),
                tooltip_host.downgrade(),
                window,
                cx,
            )
        });
        window
            .observe_release(&diff_renderer, cx, |pane, window, cx| {
                if let Some(cancel) = pane.conflict_image_preview_cancel.take() {
                    cancel.store(true, std::sync::atomic::Ordering::Release);
                }
                pane.conflict_image_preview_task = None;
                pane.file_image_preview_timer = Default::default();

                let mut images = Vec::new();
                if let Some(image) = pane.file_image_diff_cache_old.take() {
                    images.push(image);
                }
                if let Some(image) = pane.file_image_diff_cache_new.take()
                    && images
                        .iter()
                        .all(|known: &Arc<gpui::RenderImage>| known.id != image.id)
                {
                    images.push(image);
                }
                for side in ThreeWayColumn::ALL {
                    if let Loadable::Ready(Some(ConflictPreviewImage::Rendered(image))) =
                        pane.conflict_resolver.image_preview.image(side)
                        && images
                            .iter()
                            .all(|known: &Arc<gpui::RenderImage>| known.id != image.id)
                    {
                        images.push(Arc::clone(image));
                    }
                }
                for image in images {
                    cx.drop_image(image, Some(&mut *window));
                }
            })
            .detach();
        let details_pane = cx.new(|cx| {
            DetailsPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                DetailsPaneInit {
                    theme: initial_theme,
                    root_view: weak_view.clone(),
                    diff_renderer: diff_renderer.downgrade(),
                    history_view: main_pane.read(cx).history_view.clone(),
                    tooltip_host: tooltip_host.downgrade(),
                },
                window,
                cx,
            )
        });

        let reflog_pane = cx.new(|cx| {
            ReflogPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                ReflogPaneInit {
                    theme: initial_theme,
                    root_view: weak_view.clone(),
                },
                cx,
            )
        });

        let popover_host = cx.new(|cx| {
            PopoverHost::new(
                Arc::clone(&store),
                ui_model.clone(),
                PopoverHostInit {
                    theme: initial_theme,
                    root_view: weak_view.clone(),
                    root_view_mode: view_mode,
                    tooltip_host: tooltip_host.downgrade(),
                    diff_renderer: diff_renderer.clone(),
                    main_pane: main_pane.clone(),
                    details_pane: details_pane.clone(),
                    reflog_pane: reflog_pane.clone(),
                    sidebar_pane: sidebar_pane.clone(),
                    pinned_branches_by_repo: ui_session.repo_sidebar_pinned_branches.clone(),
                    collapsed_items_by_repo: ui_session.repo_sidebar_collapsed_items.clone(),
                },
                window,
                cx,
            )
        });

        let command_palette = cx.new(|cx| {
            command_palette::CommandPaletteView::new(
                initial_theme,
                initial_state.active_repo.is_some(),
                weak_view.clone(),
                window,
                cx,
            )
        });

        let reveal_commit_dialog = cx.new(|cx| {
            reveal_commit::RevealCommitView::new(
                initial_theme,
                initial_state.active_repo,
                weak_view.clone(),
                Arc::clone(&store),
                window,
                cx,
            )
        });

        let activation_subscription = cx.observe_window_activation(window, |this, window, cx| {
            let now = Instant::now();
            if !window.is_window_active() {
                crate::press_gesture::clear_pointer_press(window, cx);
                // Leaving the app is one of the two moments auto-save has to
                // mean more than "after a pause": the pending timer would
                // otherwise fire against a window the user has already left,
                // and an external edit to the same file could land first.
                this.diff_renderer
                    .update(cx, |pane, cx| pane.flush_file_editor_buffer(cx));
                // Capture the focused element before our blur policy clears it.
                // This is the restore target when opening the palette via a global hotkey while
                // this window is in the background.
                this.pre_palette_focus = window.focused(cx);
                crate::window_focus::reset_on_deactivation(window, cx);
                // A deactivation right after we asked for a move/resize grab is
                // the compositor taking focus for the drag, not the user leaving
                // the app. Remember it so the matching re-activation does not
                // refresh the repo.
                this.window_grab_activation_suppressed_at =
                    crate::kit::window_chrome::take_window_grab_started_within(
                        now,
                        WINDOW_GRAB_DEACTIVATE_GRACE,
                    )
                    .then_some(now);
                return;
            }
            crate::app::mark_gitcomet_window_focused(cx, this.window_handle.window_id());
            #[cfg(target_os = "macos")]
            cx.defer(crate::app::refresh_macos_app_menus);
            crate::workspaces::mark_window_active(cx, this.window_handle.window_id());
            if !this.has_repo_tabs() {
                // Another window may have opened or closed repositories.
                this.home.update(cx, |home, _| home.refresh_repositories());
                cx.notify();
            }
            let self_initiated_grab =
                consume_window_grab_activation(&mut this.window_grab_activation_suppressed_at, now);
            if self_initiated_grab {
                return;
            }
            let git_available = this.state.git_runtime.is_available();
            if !git_available {
                runtime_probe::request(cx, false);
            }
            // Suppressed activations skip `repo_activation_msg` entirely, so its
            // throttle map is not stamped and a genuine alt-tab immediately after
            // a drag still refreshes.
            if !git_available {
                return;
            }
            if let Some(msg) =
                repo_activation_msg(&this.state, &mut this.last_repo_activation_dispatch_at, now)
            {
                // The full refresh already requests the other worktrees' scan.
                // Requesting it here too schedules a redundant trailing scan.
                this.store.dispatch(msg);
            }
        });

        let window_bounds_subscription = cx.observe_window_bounds(window, |this, window, cx| {
            this.window_placement = Some(crate::app::capture_window_placement(
                window,
                cx,
                this.window_placement.as_ref(),
            ));
            if let Some(placement) = this.window_placement.clone() {
                crate::workspaces::record_window_placement(
                    cx,
                    this.window_handle.window_id(),
                    placement,
                );
            }
            if this.view_mode == GitCometViewMode::FocusedMergetool {
                this.schedule_legacy_window_bounds_persist(cx);
            } else {
                this.schedule_workspace_persist(cx);
            }
        });

        let appearance_subscription = {
            let view = cx.weak_entity();
            let mut first = true;
            window.observe_window_appearance(move |window, app| {
                if first {
                    first = false;
                    return;
                }
                let _ = view.update(app, |this, cx| {
                    this.window_appearance = window.appearance();
                    if !this.effective_theme_mode().is_automatic() {
                        return;
                    }
                    let theme = this
                        .effective_theme_mode()
                        .resolve_theme(this.window_appearance);
                    this.set_theme(theme, cx);
                    cx.notify();
                });
            })
        };

        let open_repo_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/repo".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let auth_prompt_username_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Username".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let auth_prompt_secret_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Password / passphrase / confirmation".into(),
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_masked(true, cx);
            input
        });

        let home = cx.new(|cx| {
            let mut home = super::home::HomeView::new(
                window.window_handle(),
                tooltip_host.downgrade(),
                ui_session.pinned_repos.clone(),
                ui_session.recent_repos.clone(),
                window,
                cx,
            );
            home.set_workspace_id(workspace_id);
            home
        });
        let home_subscription = cx.subscribe(&home, |this, _, event, cx| match event {
            super::home::HomeEvent::OpenWorkspace(id) => this.open_workspace_from_home(*id, cx),
            super::home::HomeEvent::OpenRepository(path) => this.open_repo_path(path.clone(), cx),
        });

        let open_repo_input_subscription = cx.observe(&open_repo_input, |this, input, cx| {
            let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
            let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

            if !this.open_repo_panel {
                return;
            }

            if escape_pressed {
                this.open_repo_panel = false;
                cx.notify();
                return;
            }
            if enter_pressed {
                this.submit_open_repo_panel(cx);
                return;
            }
            cx.notify();
        });

        let auth_prompt_username_input_subscription =
            cx.observe(&auth_prompt_username_input, |this, input, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

                if escape_pressed {
                    this.cancel_auth_prompt(cx);
                    return;
                }
                if enter_pressed {
                    this.try_auth_prompt_submit(cx);
                    return;
                }
                cx.notify();
            });

        let auth_prompt_secret_input_subscription =
            cx.observe(&auth_prompt_secret_input, |this, input, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

                if escape_pressed {
                    this.cancel_auth_prompt(cx);
                    return;
                }
                if enter_pressed {
                    this.try_auth_prompt_submit(cx);
                    return;
                }
                cx.notify();
            });

        let scale = ui_scale::UiScale::from_percent(ui_scale.percent);
        let initial_sidebar_width_design =
            ui_scale::design_units_from_stored(restored_sidebar_width)
                .unwrap_or(280.0)
                .max(SIDEBAR_MIN_PX);
        let initial_details_width_design =
            ui_scale::design_units_from_stored(restored_details_width)
                .unwrap_or(420.0)
                .max(DETAILS_MIN_PX);
        let initial_sidebar_width = scale.px(initial_sidebar_width_design);
        let initial_details_width = scale.px(initial_details_width_design);
        // Reopen collapsed if the user quit while collapsed: the render width must
        // also start at the collapsed strip so it doesn't flash open on launch.
        let initial_sidebar_render_width = if restored_sidebar_collapsed {
            scale.px(PANE_COLLAPSED_PX)
        } else {
            initial_sidebar_width
        };

        let terminal_keystroke_interceptor = Self::install_terminal_keystroke_interceptor(cx);
        let documents_root = cx.weak_entity();
        let documents = cx.new(|cx| {
            documents::DocumentsView::new(
                initial_theme,
                documents_root.clone(),
                ui_model.clone(),
                cx,
            )
        });
        let document_picker = {
            let status_bar = bottom_status_bar.entity_id();
            cx.new(|cx| {
                documents::DocumentPicker::new(
                    initial_theme,
                    documents_root,
                    &documents,
                    status_bar,
                    window,
                    cx,
                )
            })
        };

        // The error details dialog shows what the toast host holds.
        let toast_errors_subscription = cx.observe(&toast_host, |this, toast_host, cx| {
            let errors = toast_host.read(cx).error_notices().len();
            this.popover_host
                .update(cx, |host, cx| host.error_notices_changed(errors, cx));
        });

        let closing_view = cx.weak_entity();
        let probe_window = window.window_handle().window_id();
        let large_file_tools_window_closed = cx.on_window_closed(move |cx, closed| {
            if closed == probe_window {
                let _ = closing_view.update(cx, |view, _| view.close_large_file_tools_probe());
            }
        });

        let mut view = Self {
            state: Arc::clone(&initial_state),
            window_handle: window.window_handle(),
            ui_model,
            store,
            _poller: poller,
            _ui_model_subscription: ui_model_subscription,
            _activation_subscription: activation_subscription,
            _window_bounds_subscription: window_bounds_subscription,
            _appearance_subscription: appearance_subscription,
            _terminal_keystroke_interceptor: terminal_keystroke_interceptor,
            _auth_prompt_username_input_subscription: auth_prompt_username_input_subscription,
            _open_repo_input_subscription: open_repo_input_subscription,
            _home_subscription: home_subscription,
            _auth_prompt_secret_input_subscription: auth_prompt_secret_input_subscription,
            _toast_errors_subscription: toast_errors_subscription,
            view_mode,
            workspace_id,
            persisted_workspace_repo_paths,
            persisted_workspace_active_repository,
            window_placement: Some(crate::app::capture_window_placement(
                window,
                cx,
                initial_window_placement.as_ref(),
            )),
            native_window_title: crate::view::product_name().to_string(),
            theme_mode,
            workspace_theme_mode,
            window_appearance,
            theme: initial_theme,
            title_bar,
            sidebar_pane,
            diff_renderer,
            main_pane,
            details_pane,
            repo_tabs_bar,
            action_bar,
            bottom_status_bar,
            window_gates: super::window_gates::WindowGates::new(cx),
            window_gated: false,
            extension_window,
            repository_view_focus: cx.focus_handle(),
            repository_views: Self::repository_view_router(cx),
            details_tabs: Self::details_tab_router(cx),
            sidebar_tabs: Self::sidebar_tab_router(cx),
            sidebar_sections: Self::sidebar_section_router(cx),
            tooltip_host,
            toast_host,
            history_refs_hover_host,
            commit_message_hover_host,
            popover_host,
            command_palette,
            command_palette_open: false,
            reveal_commit_dialog,
            reveal_commit_open: false,
            pre_palette_focus: None,
            focused_diff_pane,
            focused_mergetool_bootstrap,
            submodule_diff_bootstrap: None,
            deferred_repo_bootstrap,
            startup_repo_runtime_adoption,
            pending_repo_open_reservations,
            pending_repo_open_active,
            startup_repo_bootstrap_pending,
            splash_backdrop_image: splash::load_splash_backdrop_image(initial_theme.is_dark),
            last_window_size: size(px(0.0), px(0.0)),
            ui_window_size_last_seen: size(px(0.0), px(0.0)),
            synced_repo_paths: std::sync::Arc::from(Vec::new()),
            ui_settings_persist_seq: 0,
            workspace_persist_seq: 0,
            #[cfg(test)]
            ui_settings_persist_requests_for_test: 0,
            last_repo_activation_dispatch_at: initial_state
                .repos
                .iter()
                .filter(|repo| matches!(repo.open, Loadable::Ready(_)))
                .map(|repo| (repo.id, Instant::now()))
                .collect(),
            window_grab_activation_suppressed_at: None,
            signing_tools_probe_seq: 0,
            signing_tools_probe_in_flight: false,
            signing_tools_probe_cancellation: Default::default(),
            large_file_tools_probe: Default::default(),
            _large_file_tools_window_closed: large_file_tools_window_closed,
            #[cfg(test)]
            large_file_tools_detector_for_test: None,
            date_time_format,
            timezone,
            show_timezone,
            change_tracking_view,
            file_list_layout,
            file_list_sort,
            terminal_preferences,
            terminal_sessions: FxHashMap::default(),
            terminal_panel_height: px(TERMINAL_PANEL_DEFAULT_HEIGHT_PX),
            terminal_panel_resize: None,
            next_terminal_session_seq: 1,
            reflog_pane,
            file_history_pane,
            bottom_panel_providers: super::bottom_panel_providers::registered(cx),
            active_bottom_panel: FxHashMap::default(),
            commit_push_after_enabled,
            diff_scroll_sync,
            diff_content_mode,
            diff_whitespace_mode,
            diff_view_mode,
            annotate_enabled,
            diff_reveal_whitespace_chars,
            diff_word_wrap,
            diff_tab_size,
            diff_show_line_numbers,
            auto_save_file_edits,
            remote_markdown_image_policy,
            remote_url_policy,
            check_for_updates_on_startup,
            update_check_in_flight: false,
            update_check_manual_feedback_requested: false,
            ui_scale_percent: ui_scale.percent,
            zoom_wheel: Default::default(),
            open_repo_panel: false,
            open_repo_input,
            home,
            external_drag_paths: None,
            file_operations: file_operations::FileOperationsUi::default(),
            documents,
            document_picker,
            documents_active: false,
            document_routing: documents::Routing::default(),
            external_drag_payload: None,
            external_drag_classification_seq: 0,
            external_drag_drop_pending: false,
            hover_resize_edge: None,
            frame_decorations: None,
            sidebar_collapsed: restored_sidebar_collapsed,
            sidebar_collapsed_popover: None,
            sidebar_collapsed_popover_closing: None,
            sidebar_collapsed_popover_anim_seq: 0,
            sidebar_collapsed_before_merge_view: None,
            details_collapsed: false,
            sidebar_width_design: initial_sidebar_width_design,
            details_width_design: initial_details_width_design,
            sidebar_width: initial_sidebar_width,
            sidebar_header_minimum: px(0.0),
            sidebar_window_size_request: None,
            _sidebar_header_subscription: sidebar_header_subscription,
            details_width: initial_details_width,
            sidebar_render_width: initial_sidebar_render_width,
            details_render_width: initial_details_width,
            sidebar_width_anim_seq: 0,
            details_width_anim_seq: 0,
            sidebar_width_animating: false,
            details_width_animating: false,
            pane_resize: None,
            last_mouse_pos: point(px(0.0), px(0.0)),
            pending_terminal_shutdown_prompt: None,
            pending_close_guard_prompt: None,
            pending_unsaved_file_edits_prompt: None,
            pending_unsaved_file_edits_flush: None,
            pending_file_edits_action: None,
            pending_quit_other_views: Vec::new(),
            pending_pull_reconcile_prompt: None,
            pending_branch_exists_prompt: initial_state.branch_exists_prompt.clone(),
            pending_force_delete_branch_prompt: None,
            pending_force_delete_branch_centered: false,
            pending_force_remove_worktree_prompt: None,
            pending_submodule_trust_prompt: None,
            pending_submodule_trust_check: None,
            pending_hook_activity_open: None,
            minimized_hook_activity_chains: FxHashSet::default(),
            minimized_hook_activity_repos: FxHashSet::default(),
            startup_crash_report,
            #[cfg(target_os = "macos")]
            recent_repos_menu_fingerprint: ui_session.recent_repos.clone(),
            #[cfg(target_os = "macos")]
            recent_repos_session_stamp: None,
            auth_prompt_username_input,
            auth_prompt_secret_input,
            auth_prompt_key: None,
            auth_prompt_submitted_generation: None,
            auth_prompt_input_error: None,
            auth_prompt_focus_return: None,
            active_context_menu_invoker: None,
        };

        view.set_theme(initial_theme, cx);
        view.sync_terminal_button_target(cx);
        view.consume_initial_store_feedback(cx);
        for reason in entry_denials {
            view.push_toast(components::ToastKind::Warning, reason.to_string(), cx);
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        view.maybe_auto_install_linux_desktop_integration(cx);

        view.drive_focused_mergetool_bootstrap();
        view.drive_submodule_diff_bootstrap();
        runtime_probe::request(cx, false);
        view.refresh_signing_tools(false, cx);
        view.schedule_large_file_tools_after_initial_frame(window, cx);

        view.sync_workspace_and_registry(cx);
        if view.is_home_screen_active() {
            let focus = view.home.read(cx).search_focus_handle(cx);
            window.focus(&focus, cx);
        }

        view
    }

    pub(crate) fn run_process_startup_hooks(&mut self, cx: &mut gpui::Context<Self>) {
        #[cfg(test)]
        crate::app::record_startup_hook_invocation_for_test(cx, self.window_handle.window_id());
        self.maybe_show_user_survey_on_startup(cx);
        self.maybe_check_for_updates_on_startup(cx);
        self.maybe_warn_outdated_git(cx);
    }

    pub(super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        let theme = theme.with_appearance(crate::appearance::current(cx));
        crate::theme::set_window_theme(cx, self.window_handle.window_id(), theme);
        self.documents
            .update(cx, |documents, cx| documents.set_theme(theme, cx));
        self.document_picker
            .update(cx, |picker, cx| picker.set_theme(theme, cx));
        self.splash_backdrop_image = splash::load_splash_backdrop_image(theme.is_dark);
        self.theme = theme;
        if let Some(extension_window) = self.extension_window.as_ref() {
            extension_window.set_theme(theme, cx);
        }
        for session in self.terminal_sessions.values() {
            for instance in &session.instances {
                instance.viewport.update(cx, |viewport, cx| {
                    viewport.set_theme(theme, cx);
                });
            }
        }
        self.title_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.sidebar_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.diff_renderer
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.main_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.details_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.reflog_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.file_history_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.repo_tabs_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.action_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.bottom_status_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.tooltip_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.toast_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.history_refs_hover_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.commit_message_hover_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.popover_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.command_palette
            .update(cx, |palette, cx| palette.set_theme(theme, cx));
        self.reveal_commit_dialog
            .update(cx, |dialog, cx| dialog.set_theme(theme, cx));
        cx.notify();
    }

    /// The footer's zoom button shows whether this window has its own zoom.
    pub(crate) fn refresh_zoom_indicator(&mut self, cx: &mut gpui::Context<Self>) {
        self.bottom_status_bar.update(cx, |_bar, cx| cx.notify());
    }

    pub(super) fn notify_font_preferences_changed(&mut self, cx: &mut gpui::Context<Self>) {
        let metrics = crate::appearance::current(cx);
        self.set_theme(self.theme.with_appearance(metrics), cx);
        self.update_ui_preferences(cx, move |prefs| prefs.appearance.metrics = metrics);
        self.details_pane
            .update(cx, |pane, _| pane.appearance_metrics = metrics);
        self.main_pane.update(cx, |pane, cx| {
            pane.history_view.update(cx, |history, cx| {
                history.set_appearance_metrics(metrics, cx);
            });
        });
        for session in self.terminal_sessions.values() {
            for instance in &session.instances {
                instance.viewport.update(cx, |viewport, cx| {
                    viewport.invalidate_layout(cx);
                });
            }
        }
        self.title_bar.update(cx, |_bar, cx| cx.notify());
        self.sidebar_pane.update(cx, |_pane, cx| cx.notify());
        self.diff_renderer
            .update(cx, |pane, cx| pane.invalidate_font_metrics(cx));
        self.details_pane.update(cx, |_pane, cx| cx.notify());
        self.reflog_pane.update(cx, |_pane, cx| cx.notify());
        self.repo_tabs_bar.update(cx, |_bar, cx| cx.notify());
        self.action_bar.update(cx, |_bar, cx| cx.notify());
        self.bottom_status_bar.update(cx, |_bar, cx| cx.notify());
        self.tooltip_host.update(cx, |_host, cx| cx.notify());
        self.toast_host.update(cx, |_host, cx| cx.notify());
        self.popover_host.update(cx, |_host, cx| cx.notify());
        self.open_repo_input.update(cx, |_input, cx| cx.notify());
        self.auth_prompt_username_input
            .update(cx, |_input, cx| cx.notify());
        self.auth_prompt_secret_input
            .update(cx, |_input, cx| cx.notify());
        cx.notify();
    }

    pub(crate) fn workspace_changed(&mut self, cx: &mut gpui::Context<Self>) {
        self.sync_workspace_theme_override(cx);
        self.sync_native_window_title(cx);
        // Both entities paint against the title-bar fill: the title bar owns
        // the chrome and the tab strip owns transparent labels and hover
        // overlays that must composite over the same color.
        self.title_bar.update(cx, |_bar, cx| cx.notify());
        self.repo_tabs_bar.update(cx, |_bar, cx| cx.notify());
        cx.notify();
    }

    pub(super) fn refresh_main_pane_after_panel_animation(&mut self, cx: &mut gpui::Context<Self>) {
        let diff_renderer = self.diff_renderer.clone();
        cx.defer(move |cx| {
            diff_renderer.update(cx, |pane, cx| {
                pane.sync_layout_snapshot(cx);
                cx.notify();
            });
        });
    }

    /// Evaluate a CSS-style `cubic-bezier(x1, y1, x2, y2)` timing function at
    /// progress `t` in `[0, 1]`. Endpoints P0=(0,0) and P3=(1,1) are implicit.
    ///
    /// The curve is parametric in `s`, so for a given time `t` we first solve
    /// `bezier_x(s) = t` (a few Newton-Raphson steps — the x-curve is monotonic
    /// for the control points we use) and then read off `bezier_y(s)`.
    pub(super) fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }

        // B(s) = 3(1-s)^2 s c1 + 3(1-s) s^2 c2 + s^3, with c0 = 0, c3 = 1.
        let bezier = |c1: f32, c2: f32, s: f32| {
            let inv = 1.0 - s;
            3.0 * inv * inv * s * c1 + 3.0 * inv * s * s * c2 + s * s * s
        };
        // B'(s) = 3(1-s)^2 c1 + 6(1-s) s (c2 - c1) + 3 s^2 (1 - c2).
        let bezier_prime = |c1: f32, c2: f32, s: f32| {
            let inv = 1.0 - s;
            3.0 * inv * inv * c1 + 6.0 * inv * s * (c2 - c1) + 3.0 * s * s * (1.0 - c2)
        };

        let mut s = t;
        for _ in 0..8 {
            let x = bezier(x1, x2, s) - t;
            if x.abs() < 1e-4 {
                break;
            }
            let dx = bezier_prime(x1, x2, s);
            if dx.abs() < 1e-6 {
                break;
            }
            s = (s - x / dx).clamp(0.0, 1.0);
        }

        bezier(y1, y2, s)
    }

    /// Easing for pane collapse/expand: a "fast-out, slow-in" cubic bezier
    /// (the Material standard curve) that reads smoothly in both directions.
    pub(super) fn pane_collapse_ease(t: f32) -> f32 {
        Self::cubic_bezier(0.4, 0.0, 0.2, 1.0, t)
    }

    pub(super) fn animate_sidebar_render_width_to(
        &mut self,
        target: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let start = self.sidebar_render_width;
        let start_f: f32 = start.into();
        let target_f: f32 = target.into();
        self.sidebar_width_anim_seq = self.sidebar_width_anim_seq.wrapping_add(1);
        let seq = self.sidebar_width_anim_seq;
        if (start_f - target_f).abs() <= 0.5 {
            self.sidebar_render_width = target;
            self.sidebar_width_animating = false;
            return;
        }

        if !crate::ui_runtime::current().uses_pane_animations() {
            self.sidebar_render_width = target;
            self.sidebar_width_animating = false;
            self.refresh_main_pane_after_panel_animation(cx);
            cx.notify();
            return;
        }

        self.sidebar_width_animating = true;
        let started = Instant::now();
        let duration = Duration::from_millis(PANE_COLLAPSE_ANIM_MS);

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(16)).await;

                let mut t =
                    started.elapsed().as_secs_f32() / duration.as_secs_f32().max(f32::EPSILON);
                if !t.is_finite() {
                    t = 1.0;
                }
                let t = t.clamp(0.0, 1.0);
                let eased = Self::pane_collapse_ease(t);
                let mut done = t >= 1.0;

                let _ = view.update(cx, |this, cx| {
                    if this.sidebar_width_anim_seq != seq {
                        done = true;
                        return;
                    }

                    let mut changed = false;
                    let next_width = px(start_f + (target_f - start_f) * eased);
                    if this.sidebar_render_width != next_width {
                        this.sidebar_render_width = next_width;
                        changed = true;
                    }
                    if t >= 1.0 {
                        if this.sidebar_render_width != px(target_f) {
                            this.sidebar_render_width = px(target_f);
                        }
                        this.sidebar_width_animating = false;
                        this.refresh_main_pane_after_panel_animation(cx);
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                });

                if done {
                    break;
                }
            },
        )
        .detach();
    }

    pub(super) fn animate_details_render_width_to(
        &mut self,
        target: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let start = self.details_render_width;
        let start_f: f32 = start.into();
        let target_f: f32 = target.into();
        self.details_width_anim_seq = self.details_width_anim_seq.wrapping_add(1);
        let seq = self.details_width_anim_seq;
        if (start_f - target_f).abs() <= 0.5 {
            self.details_render_width = target;
            self.details_width_animating = false;
            return;
        }

        if !crate::ui_runtime::current().uses_pane_animations() {
            self.details_render_width = target;
            self.details_width_animating = false;
            self.refresh_main_pane_after_panel_animation(cx);
            cx.notify();
            return;
        }

        self.details_width_animating = true;
        let started = Instant::now();
        let duration = Duration::from_millis(PANE_COLLAPSE_ANIM_MS);

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(16)).await;

                let mut t =
                    started.elapsed().as_secs_f32() / duration.as_secs_f32().max(f32::EPSILON);
                if !t.is_finite() {
                    t = 1.0;
                }
                let t = t.clamp(0.0, 1.0);
                let eased = Self::pane_collapse_ease(t);
                let mut done = t >= 1.0;

                let _ = view.update(cx, |this, cx| {
                    if this.details_width_anim_seq != seq {
                        done = true;
                        return;
                    }

                    let mut changed = false;
                    let next_width = px(start_f + (target_f - start_f) * eased);
                    if this.details_render_width != next_width {
                        this.details_render_width = next_width;
                        changed = true;
                    }
                    if t >= 1.0 {
                        if this.details_render_width != px(target_f) {
                            this.details_render_width = px(target_f);
                        }
                        this.details_width_animating = false;
                        this.refresh_main_pane_after_panel_animation(cx);
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                });

                if done {
                    break;
                }
            },
        )
        .detach();
    }

    /// Opens the file history tab for `path`; does nothing for an inactive repository.
    pub(in crate::view) fn open_file_history(
        &mut self,
        repo_id: RepoId,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.state.active_repo != Some(repo_id) {
            return;
        }
        self.set_sidebar_collapsed(false, cx);
        self.file_history_pane
            .update(cx, |pane, cx| pane.open(repo_id, path, window, cx));
        self.sidebar_pane
            .update(cx, |pane, cx| pane.show_file_history(window, cx));
    }

    pub(super) fn set_sidebar_collapsed(&mut self, collapsed: bool, cx: &mut gpui::Context<Self>) {
        if self.sidebar_collapsed == collapsed {
            return;
        }

        self.sidebar_collapsed = collapsed;
        self.sidebar_pane.update(cx, |pane, cx| {
            pane.set_expanded_branches_visible(!collapsed, cx)
        });
        // The cached bottom bar draws this toggle's icon. Notified by id: its own
        // toggle calls this from inside the bar's update.
        gpui::AppContext::notify(cx, self.bottom_status_bar.entity_id());
        // The collapsed-rail popover only exists while collapsed; drop it (and any
        // in-flight fade) instantly when the full sidebar comes back so it can't
        // linger over the expanded pane.
        if !collapsed {
            self.sidebar_collapsed_popover = None;
            self.sidebar_collapsed_popover_closing = None;
            self.sidebar_collapsed_popover_anim_seq =
                self.sidebar_collapsed_popover_anim_seq.wrapping_add(1);
        }
        if matches!(
            self.pane_resize,
            Some(PaneResizeState {
                handle: PaneResizeHandle::Sidebar,
                ..
            })
        ) {
            self.pane_resize = None;
        }
        if !collapsed {
            // Mark the sidebar as animating before clamping: the width reconcile
            // in `clamp_pane_widths_to_window` snaps `sidebar_render_width` to the
            // target whenever it isn't animating, which would collapse the open
            // animation to a single frame (start == target). With the flag set it
            // preserves the current (collapsed) render width so the animation below
            // can grow it out.
            self.sidebar_width_animating = true;
            self.clamp_pane_widths_to_window();
        }

        let target = if collapsed {
            self.pane_collapsed_width()
        } else {
            self.sidebar_width
        };
        self.animate_sidebar_render_width_to(target, cx);
        // Persist so the sidebar reopens in the same state next launch.
        self.schedule_ui_settings_persist(cx);
        cx.notify();
    }

    /// Toggle the collapsed-sidebar popover for `section`. Clicking the icon of
    /// the open section closes it; clicking a different one switches to it and
    /// triggers any lazy data load that section needs.
    pub(in crate::view) fn toggle_sidebar_collapsed_popover(
        &mut self,
        section: CollapsedSidebarSection,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.sidebar_collapsed_popover == Some(section) {
            self.close_sidebar_collapsed_popover(cx);
        } else {
            self.open_sidebar_collapsed_popover(section, cx);
        }
    }

    pub(super) fn open_sidebar_collapsed_popover(
        &mut self,
        section: CollapsedSidebarSection,
        cx: &mut gpui::Context<Self>,
    ) {
        self.sidebar_collapsed_popover = Some(section);
        self.sidebar_collapsed_popover_closing = None;
        self.sidebar_collapsed_popover_anim_seq =
            self.sidebar_collapsed_popover_anim_seq.wrapping_add(1);
        self.sidebar_pane.update(cx, |pane, cx| {
            pane.ensure_collapsed_section_data(section, cx);
        });
        cx.notify();
    }

    /// Begin dismissing the popover: hand the section to `..._closing` so it stays
    /// mounted for the fade-out, then clear it after the fade with a seq-guarded
    /// timer so a fresh open during the fade isn't clobbered.
    pub(in crate::view) fn close_sidebar_collapsed_popover(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(section) = self.sidebar_collapsed_popover.take() else {
            return;
        };
        self.sidebar_collapsed_popover_closing = Some(section);
        self.sidebar_collapsed_popover_anim_seq =
            self.sidebar_collapsed_popover_anim_seq.wrapping_add(1);
        let seq = self.sidebar_collapsed_popover_anim_seq;
        cx.notify();

        // Time the fade-out on the app's executor rather than a bare
        // `smol::Timer`, which would arm the global reactor and fire on its own
        // thread — deterministic under test, identical in the running app.
        let fade_out = cx
            .background_executor()
            .timer(Duration::from_millis(COLLAPSED_POPOVER_FADE_MS));
        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                fade_out.await;
                let _ = view.update(cx, |this, cx| {
                    if this.sidebar_collapsed_popover_anim_seq == seq
                        && this.sidebar_collapsed_popover_closing.is_some()
                    {
                        this.sidebar_collapsed_popover_closing = None;
                        cx.notify();
                    }
                });
            },
        )
        .detach();
    }

    /// The cached bottom bar rounds its corners by the frame's tiling, which
    /// it reads at render; a change that leaves its bounds alone would
    /// otherwise replay the old corners.
    pub(super) fn sync_frame_decorations(
        &mut self,
        decorations: gpui::Decorations,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.frame_decorations.replace(decorations) != Some(decorations) {
            gpui::AppContext::notify(cx, self.bottom_status_bar.entity_id());
        }
    }

    pub(super) fn set_details_collapsed(&mut self, collapsed: bool, cx: &mut gpui::Context<Self>) {
        if self.details_collapsed == collapsed {
            return;
        }

        self.details_collapsed = collapsed;
        // The cached bottom bar draws this toggle's icon. Notified by id: its own
        // toggle calls this from inside the bar's update.
        gpui::AppContext::notify(cx, self.bottom_status_bar.entity_id());
        if matches!(
            self.pane_resize,
            Some(PaneResizeState {
                handle: PaneResizeHandle::Details,
                ..
            })
        ) {
            self.pane_resize = None;
        }
        if !collapsed {
            // Same reasoning as the sidebar: flag the animation before clamping so
            // the width reconcile preserves the collapsed render width instead of
            // snapping to the target and cancelling the open animation.
            self.details_width_animating = true;
            self.clamp_pane_widths_to_window();
        }

        let target = if collapsed {
            self.pane_collapsed_width()
        } else {
            self.details_width
        };
        self.animate_details_render_width_to(target, cx);
        cx.notify();
    }

    pub(super) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn active_repository_is_bare(&self) -> bool {
        self.active_repo().is_some_and(RepoState::is_bare)
    }

    pub(super) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|repo| repo.id == repo_id)
    }

    pub(super) fn drive_focused_mergetool_bootstrap(&mut self) {
        if !self.state.git_runtime.is_available() {
            return;
        }

        let Some(bootstrap) = self.focused_mergetool_bootstrap.as_ref() else {
            return;
        };
        let Some(action) = focused_mergetool_bootstrap_action(&self.state, bootstrap) else {
            return;
        };

        match action {
            FocusedMergetoolBootstrapAction::OpenRepo(path) => {
                self.store.dispatch(Msg::OpenRepo(path))
            }
            FocusedMergetoolBootstrapAction::SetActiveRepo(repo_id) => {
                self.store.dispatch(Msg::SetActiveRepo { repo_id });
            }
            FocusedMergetoolBootstrapAction::SelectConflictDiff { repo_id, path } => {
                self.store
                    .dispatch(Msg::SelectConflictDiff { repo_id, path });
            }
            FocusedMergetoolBootstrapAction::LoadConflictFile { repo_id, path } => {
                self.store.dispatch(Msg::LoadConflictFile {
                    repo_id,
                    path,
                    mode: gitcomet_state::model::ConflictFileLoadMode::CurrentOnly,
                });
            }
            FocusedMergetoolBootstrapAction::Complete => {
                self.focused_mergetool_bootstrap = None;
            }
        }
    }

    pub(super) fn drive_submodule_diff_bootstrap(&mut self) {
        if !self.state.git_runtime.is_available() {
            return;
        }

        let Some(bootstrap) = self.submodule_diff_bootstrap.as_ref() else {
            return;
        };
        let Some(action) = submodule_diff_bootstrap_action(&self.state, bootstrap) else {
            return;
        };

        match action {
            SubmoduleDiffBootstrapAction::OpenRepo(path) => {
                self.store.dispatch(Msg::OpenRepo(path))
            }
            SubmoduleDiffBootstrapAction::SetActiveRepo(repo_id) => {
                self.store.dispatch(Msg::SetActiveRepo { repo_id });
            }
            SubmoduleDiffBootstrapAction::SelectDiff { repo_id, target } => {
                self.store.dispatch(Msg::SelectDiff { repo_id, target });
            }
            SubmoduleDiffBootstrapAction::Complete => {
                self.submodule_diff_bootstrap = None;
            }
        }
    }

    #[cfg(test)]
    pub(super) fn remote_rows(repo: &RepoState) -> Vec<RemoteRow> {
        let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();

        if let Loadable::Ready(remote_branches) = &repo.remote_branches {
            for branch in remote_branches.iter() {
                grouped
                    .entry(branch.remote.clone())
                    .or_default()
                    .push(branch.name.clone());
            }
        }

        if grouped.is_empty()
            && let Loadable::Ready(remotes) = &repo.remotes
        {
            for remote in remotes.iter() {
                grouped.entry(remote.name.clone()).or_default();
            }
        }

        let mut rows = Vec::new();
        for (remote, mut branches) in grouped {
            branches.sort_unstable();
            branches.dedup();
            rows.push(RemoteRow::Header(remote.clone()));
            for name in branches {
                rows.push(RemoteRow::Branch {
                    remote: remote.clone(),
                    name,
                });
            }
        }

        rows
    }

    /// Show an error until the user closes it.
    pub(in crate::view) fn report_error(
        &mut self,
        report: ErrorReport,
        cx: &mut gpui::Context<Self>,
    ) {
        self.toast_host
            .update(cx, |host, cx| host.push_error(report, cx));
    }

    pub(super) fn auth_prompt_banner_colors(theme: AppTheme) -> (gpui::Rgba, gpui::Rgba) {
        (
            with_alpha(theme.colors.accent.foreground, 0.15),
            with_alpha(theme.colors.accent.foreground, 0.3),
        )
    }

    pub(super) fn try_auth_prompt_submit(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(prompt) = self.state.auth_prompt.as_ref() else {
            return;
        };
        if prompt.status == AuthPromptStatus::Validating
            || self.auth_prompt_submitted_generation == Some(prompt.generation)
        {
            return;
        }
        let generation = prompt.generation;
        let requires_username = prompt.kind == AuthPromptKind::UsernamePassword;
        let secret_required_message = match prompt.kind {
            AuthPromptKind::UsernamePassword => "Password is required.",
            AuthPromptKind::Passphrase => "Passphrase is required.",
            AuthPromptKind::HostVerification => "Confirmation is required (`yes` or fingerprint).",
        };

        let username = self
            .auth_prompt_username_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        let secret = self.auth_prompt_secret_input.read(cx).text().to_string();

        if requires_username && username.is_empty() {
            self.auth_prompt_input_error = Some("Username is required.".to_string());
            cx.notify();
            return;
        }
        if secret.trim().is_empty() {
            self.auth_prompt_input_error = Some(secret_required_message.to_string());
            cx.notify();
            return;
        }

        // The worker may not have published Validating yet. Latch immediately
        // so a second click/Enter cannot submit the same challenge twice.
        self.auth_prompt_submitted_generation = Some(generation);
        self.auth_prompt_input_error = None;
        self.auth_prompt_secret_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.store.dispatch(Msg::SubmitAuthPrompt {
            generation,
            username: requires_username.then_some(username),
            secret,
        });
        cx.notify();
    }

    pub(super) fn cancel_auth_prompt(&mut self, cx: &mut gpui::Context<Self>) {
        self.auth_prompt_secret_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.auth_prompt_username_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.auth_prompt_input_error = None;
        self.store.dispatch(Msg::CancelAuthPrompt);
        cx.notify();
    }

    fn auth_prompt_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.auth_prompt_username_input
            .read(cx)
            .focus_handle()
            .is_focused(window)
            || self
                .auth_prompt_secret_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
    }

    /// Focuses a new prompt's first field so the user can type at once,
    /// unless they are typing in another field or the terminal, or working in
    /// an overlay: the
    /// prompt then waits for a click rather than swallowing their keystrokes.
    pub(super) fn focus_new_auth_prompt(
        &mut self,
        input: &Entity<components::TextInput>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let prompt_has_focus = self.auth_prompt_has_focus(window, cx);
        let typing_elsewhere = !prompt_has_focus
            && window
                .context_stack()
                .iter()
                .any(|context| context.contains("TextInput") || context.contains("Terminal"));
        if typing_elsewhere
            || self.is_overlay_open(cx)
            || self.command_palette_open
            || self.reveal_commit_open
        {
            return;
        }
        // A retry keeps the place the first prompt of the exchange took.
        if self.auth_prompt_focus_return.is_none() && !prompt_has_focus {
            self.auth_prompt_focus_return = Some(window.focused(cx));
        }
        window.focus(&input.read(cx).focus_handle(), cx);
    }

    /// The prompt took the keyboard by itself, so give it back once the prompt
    /// is gone, unless the user has since moved it elsewhere.
    pub(super) fn restore_focus_after_auth_prompt(&mut self, window: &mut Window, cx: &mut App) {
        let Some(previous) = self.auth_prompt_focus_return.take() else {
            return;
        };
        if !self.auth_prompt_has_focus(window, cx) {
            return;
        }
        match previous {
            Some(focus) => window.focus(&focus, cx),
            None => window.blur(cx),
        }
    }

    pub(super) fn push_toast(
        &mut self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if kind == components::ToastKind::Error {
            self.report_error(ErrorReport::message(self.active_repo_id(), message), cx);
            return;
        }
        self.toast_host
            .update(cx, |host, cx| host.push_toast(kind, message, cx));
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn push_toast_with_link(
        &mut self,
        kind: components::ToastKind,
        message: String,
        link_url: String,
        link_label: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if kind == components::ToastKind::Error {
            let report = ErrorReport::message(self.active_repo_id(), message).with_action(
                ErrorAction::OpenUrl {
                    url: link_url,
                    label: link_label,
                },
            );
            self.report_error(report, cx);
            return;
        }
        self.toast_host.update(cx, |host, cx| {
            host.push_toast_with_link(kind, message, link_url, link_label, cx)
        });
    }

    pub(super) fn active_repo_workdir(&self) -> Option<std::path::PathBuf> {
        let repo_id = self.active_repo_id()?;
        self.state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| repo.spec.workdir.clone())
    }

    pub(crate) fn open_active_repo_in_external_code_editor(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(workdir) = self.active_repo_workdir() else {
            self.push_toast(
                components::ToastKind::Error,
                "No active repository to open in code editor.".to_string(),
                cx,
            );
            return;
        };
        self.open_path_in_external_code_editor(workdir, cx);
    }

    /// Open the active repository's remote in the browser, or let the user pick
    /// one when several remotes have a web page. Pressing it again with the
    /// picker up closes the picker.
    pub(crate) fn open_remote_in_browser(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_remote_in_browser_with(window, cx, |url| platform_open::open_url_blocking(&url));
    }

    pub(super) fn open_remote_in_browser_with(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
        open_url: impl FnOnce(String) -> Result<(), std::io::Error> + Send + 'static,
    ) {
        if !command_palette_available(self.view_mode) {
            return;
        }
        let Some(repo) = self.active_repo() else {
            return;
        };
        let picker = PopoverKind::remote(repo.id, RemotePopoverKind::OpenInBrowserMenu);
        let request = remote_web_request(repo);
        if self.popover_host.read(cx).is_kind_open(&picker) {
            self.popover_host.update(cx, |host, cx| {
                host.close_popover_and_restore_focus(window, cx)
            });
            return;
        }
        match request {
            RemoteWebRequest::Open(page) => {
                platform_open::spawn_launch(
                    cx,
                    move || open_url(page.url),
                    |this, result, cx| {
                        if let Err(err) = result {
                            this.push_toast(
                                components::ToastKind::Error,
                                format!("Failed to open link: {err}"),
                                cx,
                            );
                        }
                    },
                );
            }
            RemoteWebRequest::Choose(_) => {
                // Never stack the picker's scrim on another modal's.
                if self.command_palette_open {
                    self.close_command_palette(window, cx);
                }
                if self.reveal_commit_open {
                    self.close_reveal_commit(window, cx);
                }
                self.open_popover_centered(picker, window, cx);
            }
            unavailable => {
                if let Some(message) = unavailable.unavailable_message() {
                    self.push_toast(components::ToastKind::Warning, message.to_owned(), cx);
                }
            }
        }
    }

    pub(in crate::view) fn effective_external_editor(
        &self,
    ) -> Option<gitcomet_state::session::ExternalCodeEditorSetting> {
        self.extension_window
            .as_ref()
            .and_then(|window| window.external_editor())
            .or_else(crate::external_editor::configured_setting)
    }

    pub(in crate::view) fn open_path_in_external_code_editor(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        if !path.exists() {
            self.push_toast(
                components::ToastKind::Error,
                format!("Path not found: {}", path.display()),
                cx,
            );
            return;
        }

        let command = match self
            .effective_external_editor()
            .ok_or(crate::external_editor::ExternalEditorError::NotConfigured)
            .and_then(|setting| crate::external_editor::launch_command_for_setting(&setting, &path))
        {
            Ok(command) => command,
            Err(err) => {
                self.push_toast(
                    components::ToastKind::Error,
                    format!("Failed to open in code editor: {err}"),
                    cx,
                );
                return;
            }
        };

        platform_open::spawn_launch(
            cx,
            move || crate::external_editor::spawn_launch_command(command),
            |this, result, cx| {
                if let Err(err) = result {
                    this.push_toast(
                        components::ToastKind::Error,
                        format!("Failed to open in code editor: {err}"),
                        cx,
                    );
                }
            },
        );
    }

    pub(in crate::view) fn startup_crash_report_issue_url(&self) -> Option<String> {
        self.startup_crash_report
            .as_ref()
            .map(|report| report.issue_url.clone())
            .filter(|url| !url.is_empty())
    }

    /// The "Report Issue" button's whole body, so the button stays a one-liner
    /// and tests can drive the real sequence through
    /// [`Self::report_startup_crash_report_with`].
    pub(super) fn report_startup_crash_report(&mut self, cx: &mut gpui::Context<Self>) {
        self.report_startup_crash_report_with(cx, |url| platform_open::open_url_blocking(&url));
    }

    pub(super) fn report_startup_crash_report_with(
        &mut self,
        cx: &mut gpui::Context<Self>,
        open_url: impl FnOnce(String) -> Result<(), std::io::Error> + Send + 'static,
    ) {
        let Some(url) = self.startup_crash_report_issue_url() else {
            return;
        };
        platform_open::spawn_launch(
            cx,
            move || open_url(url),
            |this, result, cx| match result {
                Ok(()) => this.push_toast(
                    components::ToastKind::Success,
                    "Opened crash report page in your browser.".to_string(),
                    cx,
                ),
                Err(err) => this.push_toast(
                    components::ToastKind::Error,
                    format!("Failed to open browser: {err}"),
                    cx,
                ),
            },
        );
    }

    pub(super) fn ignore_startup_crash_report(&mut self) -> Result<(), std::io::Error> {
        let Some(report) = self.startup_crash_report.as_ref() else {
            return Ok(());
        };
        match std::fs::remove_file(&report.crash_log_path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        self.startup_crash_report = None;
        Ok(())
    }

    /// The diff the diff keys (F1–F4, F7, Alt+Up/Down) act on: History's,
    /// unless an extension's repository view is selected. Then it is the
    /// hosted diff pane inside that view the user is engaged with, and with
    /// none there the keys do nothing, rather than move History's hidden
    /// selection.
    pub(super) fn diff_shortcut_target(
        &self,
        window: &Window,
        cx: &App,
    ) -> Option<Entity<DiffRenderer>> {
        let extension_view_selected = self
            .repository_views
            .as_ref()
            .zip(self.active_repo())
            .is_some_and(|(router, repo)| router.selected(repo).is_some());
        if !extension_view_selected || self.documents_active {
            return Some(self.diff_renderer.clone());
        }
        self.extension_window
            .as_ref()?
            .diff_pane_for_keys(&self.repository_view_focus, window, cx)
    }

    pub(super) fn defer_search_match_navigation(
        &self,
        forward: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let view = cx.weak_entity();
        let window_handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let Ok((Some(target), main, primary)) = view.read_with(cx, |view, cx| {
                    (
                        view.diff_shortcut_target(window, cx),
                        view.main_pane.clone(),
                        view.diff_renderer.entity_id(),
                    )
                }) else {
                    return;
                };
                let handled = if target.entity_id() == primary {
                    main.update(cx, |main, cx| main.navigate_search_match(forward, cx))
                } else {
                    target.update(cx, |pane, cx| {
                        if forward {
                            pane.navigate_next_search_match_or_diff_change(cx)
                        } else {
                            pane.navigate_prev_search_match_or_diff_change(cx)
                        }
                    })
                };
                if handled {
                    window.refresh();
                }
            });
        });
    }

    pub(super) fn defer_text_input_main_pane_action<F>(
        &self,
        cx: &mut gpui::Context<Self>,
        action: F,
    ) where
        F: FnOnce(&mut DiffRenderer, &mut Window, &mut gpui::Context<DiffRenderer>) -> bool
            + 'static,
    {
        let view = cx.weak_entity();
        let window_handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let Ok(Some(pane)) =
                    view.read_with(cx, |view, cx| view.diff_shortcut_target(window, cx))
                else {
                    return;
                };
                pane.update(cx, |pane, cx| {
                    if action(pane, window, cx) {
                        cx.notify();
                        window.refresh();
                    }
                });
            });
        });
    }

    pub(super) fn defer_text_input_adjacent_diff_file_navigation(
        &self,
        direction: i8,
        cx: &mut gpui::Context<Self>,
    ) {
        self.defer_text_input_main_pane_action(cx, move |pane, window, cx| {
            let Some(repo_id) = pane.active_repo_id() else {
                return false;
            };
            pane.try_select_adjacent_diff_file_preserving_focus(repo_id, direction, window, cx)
        });
    }

    pub(crate) fn defer_adjacent_diff_file_navigation(
        &self,
        direction: i8,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.shell_action_allowed(super::shell_policy::ShellAction::Repository) {
            return;
        }
        self.defer_text_input_main_pane_action(cx, move |pane, window, cx| {
            let Some(repo_id) = pane.active_repo_id() else {
                return false;
            };
            pane.try_select_adjacent_diff_file(repo_id, direction, window, cx)
        });
    }

    pub(crate) fn defer_diff_change_navigation(
        &self,
        previous: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.shell_action_allowed(super::shell_policy::ShellAction::Repository) {
            return;
        }
        self.defer_text_input_main_pane_action(cx, move |pane, _window, cx| {
            if previous {
                pane.navigate_prev_diff_change(cx)
            } else {
                pane.navigate_next_diff_change(cx)
            }
        });
    }

    /// Mouse back/forward side buttons: step the active repo's global navigation
    /// history (diffs, file content, commit selections). Active anywhere in the
    /// window.
    pub(super) fn dispatch_global_nav(&self, forward: bool, cx: &mut gpui::Context<Self>) {
        if !self.shell_action_allowed(super::shell_policy::ShellAction::Repository) {
            return;
        }
        if self.route_extension_navigation(forward, cx) {
            return;
        }
        let Some(repo_id) = self.diff_renderer.read(cx).active_repo_id() else {
            return;
        };
        let msg = if forward {
            Msg::GlobalNavForward { repo_id }
        } else {
            Msg::GlobalNavBack { repo_id }
        };
        self.store.dispatch(msg);
        cx.notify();
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn is_popover_open(&self, app: &App) -> bool {
        self.popover_host.read(app).is_open()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn tooltip_host_for_test(&self) -> Entity<TooltipHost> {
        self.tooltip_host.clone()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn tooltip_text_for_test(&self, app: &App) -> Option<SharedString> {
        self.tooltip_host
            .read(app)
            .tooltip_text_for_test()
            .or_else(tooltip::tooltip_text_for_test)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn open_repo_panel_visible_for_test(&self) -> bool {
        self.open_repo_panel
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn show_timezone_for_test(&self) -> bool {
        self.show_timezone
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(in crate::view) fn change_tracking_view_for_test(&self) -> ChangeTrackingView {
        self.change_tracking_view
    }

    #[cfg(test)]
    pub(in crate::view) fn terminal_preferences_for_test(&self) -> &TerminalPreferences {
        &self.terminal_preferences
    }

    pub(super) fn cancel_signing_tools_probe(&mut self) {
        self.signing_tools_probe_cancellation.cancel();
        self.signing_tools_probe_seq = self.signing_tools_probe_seq.wrapping_add(1);
        self.signing_tools_probe_in_flight = false;
    }

    /// Discover signing tools after opt-in, on opted-in startup, or diagnostics.
    pub(super) fn refresh_signing_tools(&mut self, force: bool, cx: &mut gpui::Context<Self>) {
        if cfg!(test)
            || !self
                .ui_model
                .read(cx)
                .preferences
                .history
                .verify_commit_signatures
            || !current_git_runtime().is_available()
        {
            return;
        }
        if !force && self.signing_tools_probe_in_flight {
            return;
        }
        self.cancel_signing_tools_probe();
        self.signing_tools_probe_cancellation = Default::default();
        let cancellation = self.signing_tools_probe_cancellation.clone();
        let seq = self.signing_tools_probe_seq;
        let runtime = current_git_runtime();
        self.signing_tools_probe_in_flight = true;
        // Explicit diagnostics may follow changes to trust/config files without
        // changing a verifier's name or version. Retire the old trust context.
        self.store
            .dispatch(Msg::SetSigningToolsState(Default::default()));
        let detection = cx.background_spawn(async move {
            gitcomet_core::signing_tools::detect_signing_tools_cancellable(&cancellation)
        });
        cx.spawn(async move |view, cx| {
            let tools = detection.await;
            let _ = view.update(cx, |this, cx| {
                if this.signing_tools_probe_seq != seq
                    || current_git_runtime() != runtime
                    || !this
                        .ui_model
                        .read(cx)
                        .preferences
                        .history
                        .verify_commit_signatures
                {
                    return;
                }
                this.signing_tools_probe_in_flight = false;
                this.store.dispatch(Msg::SetSigningToolsState(tools));
            });
        })
        .detach();
    }

    pub(super) fn resume_after_git_runtime_recovery(&mut self) {
        if let Some(bootstrap) = self.deferred_repo_bootstrap.take() {
            match bootstrap {
                DeferredRepoBootstrap::RestoreSession {
                    open_repos,
                    active_repo,
                } => {
                    self.startup_repo_bootstrap_pending = true;
                    self.store.dispatch(Msg::RestoreSession {
                        open_repos,
                        active_repo,
                    });
                }
                DeferredRepoBootstrap::OpenRepos(paths) => {
                    self.startup_repo_bootstrap_pending = true;
                    for path in paths {
                        self.store.dispatch(Msg::OpenRepo(path));
                    }
                }
            }
            return;
        }

        if !self.state.repos.is_empty() {
            let repo_ids: Vec<_> = self.state.repos.iter().map(|repo| repo.id).collect();
            for repo_id in repo_ids {
                self.store.dispatch(Msg::ReloadRepo { repo_id });
            }
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(in crate::view) fn diff_scroll_sync_for_test(&self) -> DiffScrollSync {
        self.diff_scroll_sync
    }

    pub(super) fn set_external_drag_payload(
        &mut self,
        payload: Option<external_drag::ClassifiedExternalPaths>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.external_drag_payload == payload {
            return;
        }

        let folder_active = payload
            .as_ref()
            .is_some_and(external_drag::ClassifiedExternalPaths::has_directory);
        self.external_drag_payload = payload;
        self.repo_tabs_bar.update(cx, |bar, cx| {
            bar.set_external_folder_drag_active(folder_active, cx);
        });
        cx.notify();
    }

    pub(super) fn begin_external_drag_classification(
        &mut self,
        paths: gpui::ExternalPaths,
        repository_bar_already_cleared: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.external_drag_paths.as_ref() == Some(&paths) {
            return;
        }

        self.external_drag_classification_seq =
            self.external_drag_classification_seq.wrapping_add(1);
        let classification_seq = self.external_drag_classification_seq;
        self.external_drag_paths = Some(paths.clone());
        self.external_drag_drop_pending = false;
        if repository_bar_already_cleared {
            self.external_drag_payload = None;
            cx.notify();
        } else {
            self.external_drag_payload = None;
            // Classification narrows this to payloads with a folder in them.
            let show_drop_zone = !paths.paths().is_empty();
            self.repo_tabs_bar.update(cx, |bar, cx| {
                bar.set_external_folder_drag_active(show_drop_zone, cx);
            });
            cx.notify();
        }

        let expected_paths = paths.clone();
        let classification_task =
            cx.background_spawn(
                async move { external_drag::classify_external_paths_blocking(&paths) },
            );
        cx.spawn(async move |view, cx| {
            let classification = classification_task.await;
            let _ = view.update(cx, |this, cx| {
                if this.external_drag_classification_seq != classification_seq
                    || this.external_drag_paths.as_ref() != Some(&expected_paths)
                {
                    return;
                }

                if this.external_drag_drop_pending {
                    this.external_drag_payload = Some(classification);
                    this.finish_external_drag_drop(false, cx);
                } else {
                    this.set_external_drag_payload(Some(classification), cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn clear_external_drag_state(
        &mut self,
        repository_bar_already_cleared: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let had_paths = self.external_drag_paths.take().is_some();
        let had_payload = self.external_drag_payload.take().is_some();
        let changed = had_paths || had_payload || self.external_drag_drop_pending;
        self.external_drag_drop_pending = false;
        self.external_drag_classification_seq =
            self.external_drag_classification_seq.wrapping_add(1);
        if !repository_bar_already_cleared {
            self.repo_tabs_bar.update(cx, |bar, cx| {
                bar.set_external_folder_drag_active(false, cx);
            });
        }
        if changed {
            cx.notify();
        }
    }

    pub(super) fn finish_external_drag_drop(
        &mut self,
        repository_bar_already_cleared: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let directories = self
            .external_drag_payload
            .as_ref()
            .map(|payload| payload.directories().to_vec())
            .unwrap_or_default();
        self.clear_external_drag_state(repository_bar_already_cleared, cx);
        // Each folder opens (or fails) on its own, in drop order, so the last
        // one dropped ends up as the active tab.
        let window_id = self.window_handle.window_id();
        for path in directories {
            crate::app::open_dropped_repository_from_view(cx, window_id, path);
        }
    }

    pub(super) fn submit_external_drag_payload_from_home(
        &mut self,
        paths: gpui::ExternalPaths,
        cx: &mut gpui::Context<Self>,
    ) {
        self.repo_tabs_bar.update(cx, |bar, cx| {
            bar.set_external_folder_drag_active(false, cx);
        });
        self.submit_external_drag_payload_after_repo_drop(paths, cx);
    }

    /// Called after the drop target has cleared the repository bar's highlight.
    /// The repository bar itself can still be on the entity update stack, so
    /// this must not update that entity re-entrantly from its drop handler.
    pub(in crate::view) fn submit_external_drag_payload_after_repo_drop(
        &mut self,
        paths: gpui::ExternalPaths,
        cx: &mut gpui::Context<Self>,
    ) {
        if paths.paths().is_empty() {
            self.clear_external_drag_state(true, cx);
            return;
        }

        if self.external_drag_paths.as_ref() != Some(&paths) {
            self.begin_external_drag_classification(paths, true, cx);
        }
        self.external_drag_drop_pending = true;
        if self.external_drag_payload.is_some() {
            self.finish_external_drag_drop(true, cx);
        } else {
            cx.notify();
        }
    }
}
