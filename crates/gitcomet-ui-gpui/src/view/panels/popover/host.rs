use super::*;

fn missing_checkout_remote_branch(
    state: &AppState,
    popover: Option<&PopoverKind>,
) -> Option<String> {
    let Some(PopoverKind::CheckoutRemoteBranchPrompt {
        repo_id,
        remote,
        branch,
    }) = popover
    else {
        return None;
    };
    let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id) else {
        return Some(format!("{remote}/{branch}"));
    };
    let Loadable::Ready(branches) = &repo.remote_branches else {
        return None;
    };
    (!branches
        .iter()
        .any(|candidate| candidate.remote == *remote && candidate.name == *branch))
    .then(|| format!("{remote}/{branch}"))
}

impl PopoverHost {
    #[cfg(test)]
    pub(in crate::view) fn create_branch_input_focus_handle_for_test(
        &self,
        app: &App,
    ) -> FocusHandle {
        self.create_branch_input.read(app).focus_handle()
    }

    /// The history author filter's search box, once its popover has opened it.
    #[cfg(test)]
    pub(in crate::view) fn history_author_filter_search_input_for_test(
        &self,
    ) -> Option<&Entity<components::TextInput>> {
        self.history_author_filter_search_input.as_ref()
    }

    /// Scrolls the author dropdown to a displayed row exactly as its keyboard
    /// navigation does.
    #[cfg(test)]
    pub(in crate::view) fn scroll_history_author_filter_to_item_for_test(
        &mut self,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        self.scroll_history_author_filter_to_row(ix, cx);
    }

    pub(super) fn sync_titlebar_app_menu_state(&self, cx: &mut gpui::Context<Self>) {
        let root_view = self.root_view.clone();
        let app_menu_open = matches!(self.popover, Some(PopoverKind::AppMenu));
        let repo_picker_open = matches!(self.popover, Some(PopoverKind::RepoPicker));
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.title_bar.update(cx, |title_bar, cx| {
                    title_bar.set_app_menu_open(app_menu_open, cx);
                    title_bar.set_repo_picker_open(repo_picker_open, cx);
                });
            });
        });
    }

    pub(super) fn clear_active_context_menu_invoker(&self, cx: &mut gpui::Context<Self>) {
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.set_active_context_menu_invoker(None, cx);
            });
        });
    }

    pub(super) fn history_refs_menu_active(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.root_view
            .update(cx, |root, _cx| {
                root.active_context_menu_invoker
                    .as_ref()
                    .is_some_and(|invoker| {
                        invoker.as_ref().starts_with(
                            crate::view::history_refs_hover::HISTORY_REFS_HOVER_MENU_INVOKER_PREFIX,
                        )
                    })
            })
            .unwrap_or(false)
    }

    /// Subscription that submits a prompt when Enter is pressed in one of its
    /// inputs. Escape is consumed here; prompt dismissal is handled by the
    /// PopoverPrompt key context.
    pub(super) fn prompt_enter_subscription(
        input: &Entity<components::TextInput>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
        is_active: fn(&Self) -> bool,
        submit: fn(&mut Self, &mut Window, &mut gpui::Context<Self>),
    ) -> gpui::Subscription {
        cx.observe_in(input, window, move |this, input, window, cx| {
            let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
            let _ = input.update(cx, |input, _| input.take_escape_pressed());

            if !is_active(this) {
                return;
            }

            if enter_pressed {
                submit(this, window, cx);
                return;
            }

            cx.notify();
        })
    }

    pub(in crate::view) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        init: PopoverHostInit,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let PopoverHostInit {
            theme,
            root_view,
            root_view_mode,
            tooltip_host,
            main_pane,
            details_pane,
            reflog_pane,
            sidebar_pane,
            pinned_branches_by_repo,
            collapsed_items_by_repo,
        } = init;
        let preferences = ui_model.read(cx).preferences.clone();
        let theme_mode = preferences.appearance.theme_mode;
        let date_time_format = preferences.appearance.date_time_format;
        let timezone = preferences.appearance.timezone;
        let show_timezone = preferences.appearance.show_timezone;
        let change_tracking_view = preferences.change_tracking.view;
        let commit_push_after_enabled = preferences.repository.commit_push_after_enabled;
        let diff_content_mode = preferences.diff.content_mode;
        let diff_whitespace_mode = preferences.diff.whitespace_mode;
        let diff_reveal_whitespace_chars = preferences.diff.reveal_whitespace_chars;
        let diff_word_wrap = preferences.diff.word_wrap;
        let diff_show_line_numbers = preferences.diff.show_line_numbers;
        let state = Arc::clone(&ui_model.read(cx).state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let hook_activity_repo_id = match this.popover.as_ref() {
                Some(PopoverKind::HookActivity { repo_id, .. }) => Some(*repo_id),
                _ => None,
            };
            let previous_hook_activity_rev = hook_activity_repo_id.and_then(|repo_id| {
                this.state
                    .repos
                    .iter()
                    .find(|repo| repo.id == repo_id)
                    .map(|repo| repo.feedback.hook_activity_rev)
            });
            let follow_hook_output = hook_activity_repo_id.is_some()
                && scroll_is_near_bottom(&this.hook_activity_output_scroll, px(24.0));
            let follow_hook_list = hook_activity_repo_id.is_some()
                && scroll_is_near_bottom(&this.hook_activity_hooks_scroll, px(24.0));

            let next_state = Arc::clone(&model.read(cx).state);
            let next_hook_activity_rev = hook_activity_repo_id.and_then(|repo_id| {
                next_state
                    .repos
                    .iter()
                    .find(|repo| repo.id == repo_id)
                    .map(|repo| repo.feedback.hook_activity_rev)
            });
            this.state = next_state;
            if follow_hook_output
                && previous_hook_activity_rev.is_some()
                && next_hook_activity_rev != previous_hook_activity_rev
            {
                this.hook_activity_output_scroll.scroll_to_bottom();
            }
            if follow_hook_list
                && previous_hook_activity_rev.is_some()
                && next_hook_activity_rev != previous_hook_activity_rev
            {
                this.hook_activity_hooks_scroll.scroll_to_bottom();
            }
            this.commit_prompt_message_drafts
                .retain(|repo_id, _| this.state.repos.iter().any(|repo| repo.id == *repo_id));

            if let Some(branch) = missing_checkout_remote_branch(&this.state, this.popover.as_ref())
            {
                this.close_popover(cx);
                this.push_toast(
                    components::ToastKind::Warning,
                    format!("Remote branch {branch} no longer exists."),
                    cx,
                );
                return;
            }

            // Prefill the squash prompt from the message preview when it lands,
            // rather than in the render path, so the generated message never
            // clobbers text the user typed while it was loading.
            this.sync_squash_prompt_prefill(cx);

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
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let clone_repo_parent_dir_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/parent/folder".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let rebase_onto_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "origin/main".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let create_tag_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "v1.0.0".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let create_tag_message_scroll = ScrollHandle::new();
        let create_tag_message_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Annotation message (optional)".into(),
                    multiline: true,
                    soft_wrap: true,
                    min_lines: 3,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(create_tag_message_scroll.clone()));
            input
        });

        let gitignore_patterns_scroll = ScrollHandle::new();
        let gitignore_patterns_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/file".into(),
                    multiline: true,
                    soft_wrap: true,
                    min_lines: 3,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(gitignore_patterns_scroll.clone()));
            input
        });

        let squash_message_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Commit message".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let squash_description_scroll = ScrollHandle::new();
        let squash_description_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Description (optional)".into(),
                    multiline: true,
                    soft_wrap: true,
                    min_lines: 4,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(squash_description_scroll.clone()));
            input
        });

        let remote_name_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "origin".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let remote_url_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let remote_url_edit_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let create_branch_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-name".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let stash_message_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Stash message".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        // The subject input re-renders the host on every keystroke so the
        // Squash button's disabled state (driven by whether the message is
        // empty) stays current, and submits on Enter.
        let squash_message_input_subscription =
            cx.observe(&squash_message_input, |this, input, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let _ = input.update(cx, |input, _| input.take_escape_pressed());

                if !matches!(this.popover, Some(PopoverKind::SquashPrompt { .. })) {
                    return;
                }

                if enter_pressed {
                    this.submit_squash(cx);
                    return;
                }

                cx.notify();
            });

        // The multiline description input only needs to re-render the host (it
        // does not affect the button state, and Enter inserts a newline).
        let squash_description_input_subscription =
            cx.observe(&squash_description_input, |this, _input, cx| {
                if !matches!(this.popover, Some(PopoverKind::SquashPrompt { .. })) {
                    return;
                }
                cx.notify();
            });

        let commit_prompt_message_scroll = ScrollHandle::new();
        let commit_prompt_message_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Commit message".into(),
                    multiline: true,
                    soft_wrap: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(commit_prompt_message_scroll.clone()));
            input
        });

        let push_upstream_branch_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-name".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let worktree_path_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/worktree".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let worktree_ref_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-or-commit".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let submodule_url_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let submodule_path_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "path/in/repo".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let submodule_ref_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-or-commit".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let submodule_name_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "submodule-logical-name".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let submodule_branch_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "feature".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let rebase_reword_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Commit subject".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        let rebase_reword_description_scroll = ScrollHandle::new();
        let rebase_reword_description_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Description (optional)".into(),
                    multiline: true,
                    soft_wrap: true,
                    min_lines: 4,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(rebase_reword_description_scroll.clone()));
            input
        });

        let mut prompt_input_subscriptions = Vec::new();
        prompt_input_subscriptions.push(cx.observe(
            &commit_prompt_message_input,
            |this, _input, cx| {
                if matches!(this.popover, Some(PopoverKind::CommitPrompt { .. })) {
                    cx.notify();
                }
            },
        ));
        for input in [&clone_repo_url_input, &clone_repo_parent_dir_input] {
            prompt_input_subscriptions.push(Self::prompt_enter_subscription(
                input,
                window,
                cx,
                |this| matches!(this.popover, Some(PopoverKind::CloneRepo)),
                |this, _window, cx| this.submit_clone_repo(cx),
            ));
        }
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &create_tag_input,
            window,
            cx,
            |this| matches!(this.popover, Some(PopoverKind::CreateTagPrompt { .. })),
            |this, _window, cx| this.submit_create_tag(cx),
        ));
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &create_branch_input,
            window,
            cx,
            |this| {
                matches!(
                    this.popover,
                    Some(PopoverKind::CreateBranchFromRefPrompt { .. })
                        | Some(PopoverKind::RenameBranchPrompt { .. })
                        | Some(PopoverKind::CheckoutRemoteBranchPrompt { .. })
                )
            },
            |this, window, cx| {
                if matches!(
                    this.popover,
                    Some(PopoverKind::CreateBranchFromRefPrompt { .. })
                ) {
                    this.submit_create_branch(window, cx);
                } else if matches!(this.popover, Some(PopoverKind::RenameBranchPrompt { .. })) {
                    this.submit_rename_branch(window, cx);
                } else {
                    this.submit_checkout_remote_branch(cx);
                }
            },
        ));
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &stash_message_input,
            window,
            cx,
            |this| matches!(this.popover, Some(PopoverKind::StashPrompt)),
            |this, window, cx| this.submit_stash(window, cx),
        ));
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &submodule_ref_input,
            window,
            cx,
            |this| {
                matches!(
                    this.popover,
                    Some(PopoverKind::Repo {
                        kind: RepoPopoverKind::Submodule(
                            SubmodulePopoverKind::ChangePointerPrompt { .. }
                        ),
                        ..
                    })
                )
            },
            |this, window, cx| this.submit_submodule_change_pointer(window, cx),
        ));
        for input in [&remote_name_input, &remote_url_input] {
            prompt_input_subscriptions.push(Self::prompt_enter_subscription(
                input,
                window,
                cx,
                |this| {
                    matches!(
                        this.popover,
                        Some(PopoverKind::Repo {
                            kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
                            ..
                        })
                    )
                },
                |this, _window, cx| this.submit_remote_add(cx),
            ));
        }
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &remote_url_edit_input,
            window,
            cx,
            |this| {
                matches!(
                    this.popover,
                    Some(PopoverKind::Repo {
                        kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { .. }),
                        ..
                    })
                )
            },
            |this, _window, cx| this.submit_remote_edit_url(cx),
        ));
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &push_upstream_branch_input,
            window,
            cx,
            |this| {
                matches!(
                    this.popover,
                    Some(PopoverKind::PushSetUpstreamPrompt { .. })
                )
            },
            |this, _window, cx| this.submit_push_set_upstream(cx),
        ));
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &worktree_path_input,
            window,
            cx,
            |this| {
                matches!(
                    this.popover,
                    Some(PopoverKind::Repo {
                        kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                        ..
                    })
                )
            },
            |this, _window, cx| this.submit_worktree_add(cx),
        ));
        for input in [
            &submodule_url_input,
            &submodule_path_input,
            &submodule_branch_input,
            &submodule_name_input,
        ] {
            prompt_input_subscriptions.push(Self::prompt_enter_subscription(
                input,
                window,
                cx,
                |this| {
                    matches!(
                        this.popover,
                        Some(PopoverKind::Repo {
                            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
                            ..
                        })
                    )
                },
                |this, _window, cx| this.submit_submodule_add(cx),
            ));
        }

        let context_menu_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
        let prompt_tab_group_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
        let prompt_tab_wrap_end_focus_handle = cx.focus_handle().tab_index(1).tab_stop(false);
        let create_branch_from_ref_checkout_focus_handle =
            cx.focus_handle().tab_index(0).tab_stop(true);
        let create_branch_from_ref_focus = DialogFocus::new(cx);
        let checkout_remote_branch_focus = DialogFocus::new(cx);
        let stash_focus = DialogFocus::new(cx);
        let commit_prompt_focus = DialogFocus::new(cx);
        let clone_repo_browse_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let squash_cancel_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let squash_submit_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let rebase_onto_submit_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let clone_repo_focus = DialogFocus::new(cx);
        let create_tag_focus = DialogFocus::new(cx);
        let create_tag_annotated_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let remote_add_focus = DialogFocus::new(cx);
        let remote_edit_focus = DialogFocus::new(cx);
        let push_upstream_focus = DialogFocus::new(cx);
        let worktree_browse_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let worktree_focus = DialogFocus::new(cx);
        let submodule_advanced_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let submodule_force_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let submodule_focus = DialogFocus::new(cx);

        Self {
            store,
            state,
            theme,
            theme_mode,
            date_time_format,
            timezone,
            show_timezone,
            change_tracking_view,
            commit_amend_enabled: false,
            commit_push_after_enabled,
            diff_content_mode,
            diff_whitespace_mode,
            diff_reveal_whitespace_chars,
            diff_word_wrap,
            diff_show_line_numbers,
            _ui_model_subscription: subscription,
            _repo_picker_search_input_subscription: None,
            _branch_picker_search_input_subscription: None,
            _worktree_picker_search_input_subscription: None,
            _workspace_picker_search_input_subscription: None,
            _submodule_picker_search_input_subscription: None,
            _file_history_search_input_subscription: None,
            _history_author_filter_search_input_subscription: None,
            _stash_picker_search_input_subscription: None,
            _squash_message_input_subscription: squash_message_input_subscription,
            _squash_description_input_subscription: squash_description_input_subscription,
            _prompt_input_subscriptions: prompt_input_subscriptions,
            notify_fingerprint: 0,
            root_view,
            root_view_mode,
            tooltip_host,
            main_pane,
            details_pane,
            reflog_pane,
            sidebar_pane,
            pinned_branches_by_repo,
            collapsed_items_by_repo,
            branch_filter_query: String::new(),
            popover: None,
            popover_anchor: None,
            hook_activity_selected: None,
            hook_activity_history_scroll: ScrollHandle::new(),
            hook_activity_hooks_scroll: ScrollHandle::new(),
            hook_activity_output_scroll: ScrollHandle::new(),
            cherry_pick_mainline: None,
            context_menu_focus_handle,
            menu_invoker_focus: None,
            popover_opened_from_diff_panel: false,
            prompt_tab_group_focus_handle,
            prompt_tab_wrap_end_focus_handle,
            context_menu_selected_ix: None,
            repo_picker_selected_index: None,
            repo_picker_search_query: String::new(),
            cached_recent_repos: Vec::new(),
            cached_pinned_repos: Vec::new(),
            cached_collapsed_picker_sections: std::collections::BTreeSet::new(),
            repo_picker_sort: repo_picker::RepoPickerSort::default(),
            repo_picker_sort_menu_open: false,
            picker_row_menu: None,
            branch_picker_selected_index: None,
            worktree_picker_selected_index: None,
            workspace_picker_selected_index: None,
            pending_worktree_add_prefill: None,
            submodule_picker_selected_index: None,
            file_history_selected_index: None,
            history_author_filter_selected_index: None,
            history_author_suggestions: None,
            branch_picker_rows_cache: rows_cache::RowsCache::default(),
            workspace_picker_rows_cache: rows_cache::RowsCache::default(),
            repo_picker_rows_cache: rows_cache::RowsCache::default(),
            stash_picker_rows_cache: rows_cache::RowsCache::default(),
            file_history_rows_cache: rows_cache::RowsCache::default(),
            submodule_picker_rows_cache: rows_cache::RowsCache::default(),
            worktree_picker_rows_cache: rows_cache::RowsCache::default(),
            branch_ref_rows_cache: rows_cache::RowsCache::default(),
            repo_picker_search_input: None,
            branch_picker_search_input: None,
            remote_picker_search_input: None,
            file_history_search_input: None,
            history_author_filter_search_input: None,
            worktree_picker_search_input: None,
            workspace_picker_search_input: None,
            submodule_picker_search_input: None,
            picker_prompt_scroll: ScrollHandle::new(),
            clone_repo_url_input,
            clone_repo_parent_dir_input,
            rebase_onto_input,
            create_tag_input,
            create_tag_message_input,
            create_tag_message_scroll,
            gitignore_patterns_input,
            gitignore_patterns_scroll,
            gitignore_scope: gitcomet_core::gitignore::GitignoreScope::File,
            gitignore_suggestions: None,
            gitignore_paths: Vec::new(),
            squash_message_input,
            squash_description_input,
            squash_description_scroll,
            squash_prompt_prefilled_range: None,
            remote_name_input,
            remote_url_input,
            remote_url_edit_input,
            create_branch_input,
            create_branch_checkout_enabled: true,
            create_branch_source_target: String::new(),
            worktree_ref_source_target: String::new(),
            suppress_worktree_submit_after_ref_enter: false,
            suppress_popover_close_after_action: false,
            create_branch_from_ref_checkout_focus_handle,
            create_branch_from_ref_focus,
            create_tag_annotated: false,
            create_tag_annotated_focus_handle,
            checkout_remote_branch_focus,
            stash_message_input,
            stash_focus,
            stash_picker_prompt_selected_index: None,
            stash_picker_search_input: None,
            commit_prompt_message_drafts: FxHashMap::default(),
            commit_prompt_message_input,
            commit_prompt_message_scroll,
            commit_prompt_focus,
            clone_repo_browse_focus_handle,
            squash_cancel_focus_handle,
            squash_submit_focus_handle,
            rebase_onto_submit_focus_handle,
            clone_repo_focus,
            create_tag_focus,
            remote_add_focus,
            remote_edit_focus,
            push_upstream_focus,
            worktree_browse_focus_handle,
            worktree_focus,
            submodule_advanced_focus_handle,
            submodule_force_focus_handle,
            submodule_focus,
            push_upstream_branch_input,
            worktree_path_input,
            worktree_ref_input,
            submodule_url_input,
            submodule_path_input,
            submodule_ref_input,
            submodule_branch_input,
            submodule_name_input,
            submodule_add_advanced_expanded: false,
            submodule_force_enabled: false,
            rebase_reword_input,
            rebase_reword_description_input,
            rebase_reword_description_scroll,
        }
    }

    /// Every text input owned by the host, including the lazily created
    /// picker search inputs that currently exist.
    pub(super) fn all_text_inputs(&self) -> impl Iterator<Item = &Entity<components::TextInput>> {
        [
            &self.clone_repo_url_input,
            &self.clone_repo_parent_dir_input,
            &self.rebase_onto_input,
            &self.create_tag_input,
            &self.create_tag_message_input,
            &self.gitignore_patterns_input,
            &self.squash_message_input,
            &self.squash_description_input,
            &self.remote_name_input,
            &self.remote_url_input,
            &self.remote_url_edit_input,
            &self.create_branch_input,
            &self.stash_message_input,
            &self.commit_prompt_message_input,
            &self.push_upstream_branch_input,
            &self.worktree_path_input,
            &self.worktree_ref_input,
            &self.submodule_url_input,
            &self.submodule_path_input,
            &self.submodule_ref_input,
            &self.submodule_branch_input,
            &self.submodule_name_input,
            &self.rebase_reword_input,
            &self.rebase_reword_description_input,
        ]
        .into_iter()
        .chain(
            [
                &self.repo_picker_search_input,
                &self.branch_picker_search_input,
                &self.remote_picker_search_input,
                &self.file_history_search_input,
                &self.history_author_filter_search_input,
                &self.worktree_picker_search_input,
                &self.workspace_picker_search_input,
                &self.submodule_picker_search_input,
                &self.stash_picker_search_input,
            ]
            .into_iter()
            .flatten(),
        )
    }

    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;

        let inputs: Vec<_> = self.all_text_inputs().cloned().collect();
        for input in inputs {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }

        cx.notify();
    }

    pub(in crate::view) fn is_kind_open(&self, kind: &PopoverKind) -> bool {
        self.popover.as_ref() == Some(kind)
    }

    pub(in crate::view) fn hook_activity_workflow_repo_id(&self) -> Option<RepoId> {
        match self.popover.as_ref() {
            Some(PopoverKind::HookActivity { repo_id, .. }) => Some(*repo_id),
            _ => None,
        }
    }

    pub(in crate::view) fn is_hook_activity_workflow_open(&self) -> bool {
        self.hook_activity_workflow_repo_id().is_some()
    }

    #[cfg(test)]
    pub(in crate::view) fn hook_activity_output_is_near_bottom_for_test(&self) -> bool {
        scroll_is_near_bottom(&self.hook_activity_output_scroll, px(24.0))
    }

    #[cfg(test)]
    pub(in crate::view) fn hook_activity_hooks_are_near_bottom_for_test(&self) -> bool {
        scroll_is_near_bottom(&self.hook_activity_hooks_scroll, px(24.0))
    }

    #[cfg(test)]
    pub(in crate::view) fn scroll_hook_activity_hooks_to_top_for_test(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.hook_activity_hooks_scroll
            .set_offset(point(px(0.0), px(0.0)));
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::view) fn scroll_hook_activity_output_to_top_for_test(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.hook_activity_output_scroll
            .set_offset(point(px(0.0), px(0.0)));
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::view) fn scroll_hook_activity_output_to_bottom_for_test(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.hook_activity_output_scroll.scroll_to_bottom();
        cx.notify();
    }

    pub(super) fn minimize_hook_activity(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::HookActivity { repo_id, .. }) = self.popover.as_ref() else {
            return;
        };
        let repo_id = *repo_id;
        let active_chains = self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .into_iter()
            .flat_map(|repo| repo.feedback.hook_activity.iter())
            .filter(|operation| operation.has_hooks() && operation.status.is_active())
            .map(|operation| (repo_id, operation.id))
            .collect::<Vec<_>>();
        let _ = self.root_view.update(cx, |root, cx| {
            root.minimize_hook_activity_repo(repo_id, active_chains, cx);
        });
        self.close_popover(cx);
    }

    pub(super) fn close_hook_activity(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::HookActivity { repo_id, .. }) = self.popover.as_ref() else {
            return;
        };
        let repo_id = *repo_id;
        let _ = self.root_view.update(cx, |root, cx| {
            root.resume_hook_activity_auto_open(repo_id, cx);
        });
        self.close_popover(cx);
    }

    pub(super) fn dismiss_hook_activity_workflow(
        &mut self,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        match self.popover.clone() {
            Some(PopoverKind::HookActivity { .. }) => {
                self.minimize_hook_activity(cx);
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    pub(in crate::view) fn popover_kind_for_tests(&self) -> Option<PopoverKind> {
        self.popover.clone()
    }

    #[cfg(test)]
    pub(in crate::view) fn popover_opened_from_diff_panel_for_tests(&self) -> bool {
        self.popover_opened_from_diff_panel
    }

    /// The box the open popover hangs off, when it was anchored to one.
    #[cfg(test)]
    pub(in crate::view) fn popover_anchor_bounds_for_tests(&self) -> Option<Bounds<Pixels>> {
        match self.popover_anchor {
            Some(PopoverAnchor::Bounds(bounds)) => Some(bounds),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(in crate::view) fn worktree_path_input_text_for_tests(&self, app: &gpui::App) -> String {
        self.worktree_path_input.read(app).text().to_string()
    }

    #[cfg(test)]
    pub(in crate::view) fn worktree_ref_source_target_for_tests(&self) -> &str {
        &self.worktree_ref_source_target
    }

    /// Whether the unsaved-edits confirmation is the popover on screen.
    ///
    /// Asked by the close/quit path instead of a mirrored bool: that dialog
    /// blocks every further close while it is up, and a mirror that missed a
    /// dismissal wedged the window shut for the rest of the session.
    pub(in crate::view) fn showing_unsaved_file_edits_prompt(&self) -> bool {
        matches!(self.popover, Some(PopoverKind::UnsavedFileEditsConfirm(_)))
    }

    pub(in crate::view) fn close_popover(&mut self, cx: &mut gpui::Context<Self>) {
        let dismissing_unsaved_prompt = self.showing_unsaved_file_edits_prompt();
        let dismissing_hook_activity = self.is_hook_activity_workflow_open();
        self.save_commit_prompt_draft(cx);
        self.clear_truncated_tooltip(cx);
        crate::view::tooltip::set_tooltips_suppressed_by_overlay(false, cx);
        self.popover = None;
        self.popover_anchor = None;
        self.context_menu_selected_ix = None;
        self.picker_row_menu = None;
        self.menu_invoker_focus = None;
        self.notify_fingerprint = 0;
        self.sync_titlebar_app_menu_state(cx);
        self.clear_active_context_menu_invoker(cx);
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                if dismissing_unsaved_prompt {
                    root.clear_pending_unsaved_file_edits_prompt(cx);
                }
                if dismissing_hook_activity {
                    root.set_hook_activity_dialog_repo(None, cx);
                }
                root.set_history_refs_hover_item_menu_open(false, cx);
            });
        });
        cx.notify();
    }

    /// Validates the repo's current multi-selection against its loaded log and
    /// HEAD, returning a squash plan when the selection is eligible. Shared by
    /// the squash prompt's render, prefill, and submit paths so they always
    /// agree on the range.
    pub(in crate::view) fn squash_plan_for_repo_id(
        &self,
        repo_id: RepoId,
    ) -> Option<gitcomet_core::squash::SquashPlan> {
        let repo = self.state.repos.iter().find(|r| r.id == repo_id)?;
        let Loadable::Ready(page) = &repo.log else {
            return None;
        };
        let head = repo.head_commit_id()?;
        gitcomet_core::squash::squash_eligibility(
            &page.commits,
            &repo.history_state.multi_selection.commits,
            &head,
        )
    }

    /// Populates the squash prompt's inputs from the loaded message preview.
    /// Only fires when the preview matches the live plan's range (never a stale
    /// preview from an earlier selection) and only while both inputs are still
    /// empty for a range not yet prefilled (never over the user's own text).
    pub(super) fn sync_squash_prompt_prefill(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::SquashPrompt { repo_id }) = self.popover else {
            return;
        };
        let Some(plan) = self.squash_plan_for_repo_id(repo_id) else {
            return;
        };
        let repo = self.state.repos.iter().find(|r| r.id == repo_id);
        let Some(Loadable::Ready(preview)) = repo.map(|repo| &repo.history_state.squash_preview)
        else {
            return;
        };
        // The preview must belong to the range currently planned, not a leftover
        // from a previous prompt whose PrepareSquash dispatch has not landed yet.
        if preview.oldest != plan.oldest || preview.head != plan.head {
            return;
        }
        let range = (plan.oldest.clone(), plan.head.clone());
        if self.squash_prompt_prefilled_range.as_ref() == Some(&range) {
            return;
        }
        // Empty inputs mean the user has not typed anything for this range yet;
        // if they had, we must not overwrite it.
        let inputs_empty = self
            .squash_message_input
            .read_with(cx, |input, _| input.text().is_empty())
            && self
                .squash_description_input
                .read_with(cx, |input, _| input.text().is_empty());
        if !inputs_empty {
            return;
        }

        let subject = preview.subject.clone();
        let body = preview.body.clone();
        self.squash_prompt_prefilled_range = Some(range);
        self.squash_message_input.update(cx, |input, cx| {
            input.set_text(subject, cx);
            cx.notify();
        });
        self.squash_description_input.update(cx, |input, cx| {
            input.set_text(body, cx);
            cx.notify();
        });
    }

    /// Reads the squash prompt inputs, builds the final message, and dispatches
    /// the squash against the live plan. No-ops if the selection is no longer
    /// eligible or the subject is empty.
    pub(super) fn submit_squash(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::SquashPrompt { repo_id }) = self.popover else {
            return;
        };
        let Some(plan) = self.squash_plan_for_repo_id(repo_id) else {
            return;
        };
        let subject = self
            .squash_message_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if subject.is_empty() {
            return;
        }
        let body = self
            .squash_description_input
            .read_with(cx, |input, _| input.text().to_string());
        let message = if body.trim().is_empty() {
            subject
        } else {
            format!("{subject}\n\n{}", body.trim_end())
        };
        self.store.dispatch(Msg::SquashCommits {
            repo_id,
            oldest: plan.oldest,
            expected_head: plan.head,
            message,
            count: plan.commit_count,
        });
        self.close_popover(cx);
    }

    pub(in crate::view) fn close_popover_and_restore_focus(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let menu_invoker_focus = self.menu_invoker_focus.take();
        let restore_diff_panel_focus = matches!(
            self.popover,
            Some(
                PopoverKind::ChangeTrackingSettings
                    | PopoverKind::DiffContentModeSettings
                    | PopoverKind::WebLinkMenu { .. }
                    | PopoverKind::DiffActionMenu
                    | PopoverKind::MergetoolSettingsMenu
                    | PopoverKind::DiffHunkMenu { .. }
                    | PopoverKind::DiffEditorMenu { .. }
            ) // A web link menu can also be opened from a commit message in the
              // details pane, and handing that click's focus to the diff panel would
              // move the keyboard somewhere the user never was.
        ) && self.popover_opened_from_diff_panel;
        self.close_popover(cx);
        if restore_diff_panel_focus {
            let focus = self.main_pane.read(cx).diff_panel_focus_handle.clone();
            window.focus(&focus, cx);
        } else if let Some(focus) = menu_invoker_focus {
            window.focus(&focus, cx);
        }
    }

    pub(in crate::view) fn is_open(&self) -> bool {
        self.popover.is_some()
    }

    pub(super) fn prompt_tab_navigation_enabled(&self) -> bool {
        matches!(
            self.popover,
            Some(PopoverKind::CreateBranchFromRefPrompt { .. })
                | Some(PopoverKind::RenameBranchPrompt { .. })
                | Some(PopoverKind::CheckoutRemoteBranchPrompt { .. })
                | Some(PopoverKind::StashPrompt)
                | Some(PopoverKind::CommitPrompt { .. })
                | Some(PopoverKind::CloneRepo)
                | Some(PopoverKind::CreateTagPrompt { .. })
                | Some(PopoverKind::SquashPrompt { .. })
                | Some(PopoverKind::PushSetUpstreamPrompt { .. })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
                    ..
                })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { .. }),
                    ..
                })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                    ..
                })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
                    ..
                })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(
                        SubmodulePopoverKind::ChangePointerPrompt { .. }
                    ),
                    ..
                })
        ) || self.popover.as_ref().is_some_and(popover_is_confirm_dialog)
    }

    pub(super) fn wrap_prompt_focus(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if forward {
            window.focus(&self.prompt_tab_group_focus_handle, cx);
            window.focus_next(cx);
        } else {
            window.focus(&self.prompt_tab_wrap_end_focus_handle, cx);
            window.focus_prev(cx);
        }
    }

    pub(super) fn focus_next_prompt_field(
        &mut self,
        _: &crate::view::PopoverPromptTabNext,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.prompt_tab_navigation_enabled() {
            return;
        }

        window.focus_next(cx);
        if !self
            .prompt_tab_group_focus_handle
            .contains_focused(window, cx)
        {
            self.wrap_prompt_focus(true, window, cx);
        }
        cx.stop_propagation();
    }

    pub(super) fn focus_prev_prompt_field(
        &mut self,
        _: &crate::view::PopoverPromptTabPrev,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.prompt_tab_navigation_enabled() {
            return;
        }

        window.focus_prev(cx);
        if !self
            .prompt_tab_group_focus_handle
            .contains_focused(window, cx)
        {
            self.wrap_prompt_focus(false, window, cx);
        }
        cx.stop_propagation();
    }

    pub(in crate::view) fn dismiss_prompt_popover(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.popover.as_ref().is_some_and(popover_is_confirm_dialog) {
            self.close_popover(cx);
            return;
        }
        match self.popover.as_ref() {
            Some(PopoverKind::CreateBranchFromRefPrompt { .. })
            | Some(PopoverKind::RenameBranchPrompt { .. })
            | Some(PopoverKind::StashPrompt)
            | Some(PopoverKind::CommitPrompt { .. })
            | Some(PopoverKind::StashPickerPrompt { .. })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::ChangePointerPrompt { .. }),
                ..
            }) => self.dismiss_inline_popover(window, cx),
            Some(PopoverKind::CloneRepo)
            | Some(PopoverKind::CreateTagPrompt { .. })
            | Some(PopoverKind::SquashPrompt { .. })
            | Some(PopoverKind::CheckoutRemoteBranchPrompt { .. })
            | Some(PopoverKind::PushSetUpstreamPrompt { .. })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
                ..
            })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { .. }),
                ..
            })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                ..
            })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
                ..
            }) => self.close_popover(cx),
            _ => {}
        }
    }

    pub(super) fn dismiss_prompt(
        &mut self,
        _: &crate::view::PopoverPromptDismiss,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.dismiss_hook_activity_workflow(window, cx) {
            cx.stop_propagation();
            return;
        }
        if !self.prompt_tab_navigation_enabled() {
            return;
        }

        self.dismiss_prompt_popover(window, cx);
        cx.stop_propagation();
    }

    pub(super) fn dismiss_inline_popover(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.save_commit_prompt_draft(cx);
        self.clear_truncated_tooltip(cx);
        self.popover = None;
        self.popover_anchor = None;
        self.clear_active_context_menu_invoker(cx);
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.set_history_refs_hover_item_menu_open(false, cx);
            });
        });
        let focus = self.main_pane.read(cx).diff_panel_focus_handle.clone();
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(super) fn clear_truncated_tooltip(&self, cx: &mut gpui::Context<Self>) {
        let _ = self.tooltip_host.update(cx, |host, cx| {
            host.clear_tooltip(cx);
        });
    }

    pub(super) fn can_submit_create_tag(&self, cx: &mut gpui::Context<Self>) -> bool {
        matches!(self.popover, Some(PopoverKind::CreateTagPrompt { .. }))
            && self
                .create_tag_input
                .read_with(cx, |input, _| is_submittable_branch_name(input.text()))
    }

    pub(super) fn can_submit_clone_repo(&self, cx: &mut gpui::Context<Self>) -> bool {
        matches!(self.popover, Some(PopoverKind::CloneRepo))
            && self
                .clone_repo_url_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
            && self
                .clone_repo_parent_dir_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    pub(super) fn can_submit_submodule_change_pointer(&self, cx: &mut gpui::Context<Self>) -> bool {
        matches!(
            self.popover,
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::ChangePointerPrompt { .. }),
                ..
            })
        ) && self
            .submodule_ref_input
            .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    pub(super) fn submit_create_tag(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::CreateTagPrompt { repo_id, target }) = self.popover.clone() else {
            return;
        };

        let name = self
            .create_tag_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if !is_submittable_branch_name(&name) {
            return;
        }

        let annotated = self.create_tag_annotated;
        let message = if annotated {
            let msg = self
                .create_tag_message_input
                .read_with(cx, |input, _| input.text().trim().to_string());
            Some(msg)
        } else {
            None
        };

        self.store.dispatch(Msg::CreateTag {
            repo_id,
            name,
            target,
            message,
            annotated,
        });
        self.close_popover(cx);
    }

    pub(super) fn submit_clone_repo(&mut self, cx: &mut gpui::Context<Self>) {
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

    pub(super) fn submit_submodule_change_pointer(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::ChangePointerPrompt { path }),
        }) = self.popover.clone()
        else {
            return;
        };

        let reference = self
            .submodule_ref_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if reference.is_empty() {
            return;
        }

        self.store.dispatch(Msg::ChangeSubmodulePointer {
            repo_id,
            path,
            reference,
        });
        self.dismiss_inline_popover(window, cx);
    }

    pub(super) fn inline_branch_picker_active(&self) -> bool {
        matches!(
            self.popover,
            Some(PopoverKind::BranchPicker { .. })
                | Some(PopoverKind::CreateBranchFromRefPrompt {
                    source_selectable: true,
                    ..
                })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                    ..
                })
        )
    }

    pub(super) fn handle_inline_branch_picker_escape(&mut self, cx: &mut gpui::Context<Self>) {
        match &self.popover {
            Some(PopoverKind::CreateBranchFromRefPrompt { .. }) => {
                self.branch_picker_selected_index = None;
                if let Some(input) = &self.branch_picker_search_input {
                    let target = self.create_branch_source_target.clone();
                    let theme = self.theme;
                    input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(target, cx);
                        cx.notify();
                    });
                }
                cx.notify();
            }
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                ..
            }) => {
                self.branch_picker_selected_index = None;
                if let Some(input) = &self.branch_picker_search_input {
                    let target = self.worktree_ref_source_target.clone();
                    let theme = self.theme;
                    input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(target, cx);
                        cx.notify();
                    });
                }
                cx.notify();
            }
            _ => {
                self.close_popover(cx);
            }
        }
    }

    pub(super) fn handle_inline_branch_picker_select(
        &mut self,
        name: String,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match &self.popover {
            Some(PopoverKind::CreateBranchFromRefPrompt { .. }) => {
                self.create_branch_source_target = name;
                if let Some(input) = &self.branch_picker_search_input {
                    let theme = self.theme;
                    input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(self.create_branch_source_target.clone(), cx);
                        cx.notify();
                    });
                }
                self.branch_picker_selected_index = None;
                cx.defer_in(window, |this, window, cx| {
                    if matches!(
                        this.popover,
                        Some(PopoverKind::CreateBranchFromRefPrompt { .. })
                    ) {
                        let focus = this
                            .create_branch_input
                            .read_with(cx, |input, _| input.focus_handle());
                        window.focus(&focus, cx);
                        cx.notify();
                    }
                });
                cx.notify();
            }
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                ..
            }) => {
                self.worktree_ref_source_target = name;
                if let Some(input) = &self.branch_picker_search_input {
                    let theme = self.theme;
                    input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(self.worktree_ref_source_target.clone(), cx);
                        cx.notify();
                    });
                }
                self.branch_picker_selected_index = None;
                // Hand focus to Add once the keystroke that picked the ref has
                // finished dispatching, so it cannot land on the button it just
                // moved to; `suppress_worktree_submit_after_ref_enter` covers
                // the same Enter until the next frame is on screen.
                cx.defer_in(window, |this, window, cx| {
                    if matches!(
                        this.popover,
                        Some(PopoverKind::Repo {
                            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                            ..
                        })
                    ) {
                        let focus = if this.can_submit_worktree_add(cx) {
                            this.worktree_focus.submit.clone()
                        } else {
                            this.worktree_path_input
                                .read_with(cx, |input, _| input.focus_handle())
                        };
                        window.focus(&focus, cx);
                        cx.notify();
                    }
                    cx.on_next_frame(window, |this, _window, cx| {
                        this.suppress_worktree_submit_after_ref_enter = false;
                        cx.notify();
                    });
                });
                cx.notify();
            }
            Some(PopoverKind::BranchPicker {
                purpose: BranchPickerPurpose::Delete,
            }) => {
                let is_centered = matches!(self.popover_anchor, Some(PopoverAnchor::Centered));
                let _ = self.root_view.update(cx, |root, _| {
                    root.pending_force_delete_branch_centered = is_centered;
                });
                self.store.dispatch(Msg::DeleteBranch { repo_id, name });
                self.close_popover(cx);
            }
            Some(PopoverKind::BranchPicker {
                purpose: BranchPickerPurpose::RebaseOnto,
            }) => {
                self.open_popover_centered(
                    PopoverKind::RebaseOntoConfirm {
                        repo_id,
                        onto: name,
                    },
                    window,
                    cx,
                );
            }
            _ => {
                self.store.dispatch(Msg::CheckoutBranch { repo_id, name });
                self.close_popover(cx);
            }
        }
    }

    pub(super) fn can_submit_create_branch(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.create_branch_prompt_repo_and_target().is_some()
            && self
                .create_branch_input
                .read_with(cx, |input, _| is_submittable_branch_name(input.text()))
    }

    pub(super) fn create_branch_prompt_repo_and_target(&self) -> Option<(RepoId, String)> {
        match &self.popover {
            Some(PopoverKind::CreateBranchFromRefPrompt {
                repo_id,
                source_selectable: true,
                ..
            }) => {
                let target = self.create_branch_source_target.clone();
                if target.is_empty() {
                    None
                } else {
                    Some((*repo_id, target))
                }
            }
            Some(PopoverKind::CreateBranchFromRefPrompt {
                repo_id, target, ..
            }) => Some((*repo_id, target.clone())),
            _ => None,
        }
    }

    pub(super) fn submit_create_branch(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((repo_id, target)) = self.create_branch_prompt_repo_and_target() else {
            return;
        };
        let name = self
            .create_branch_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if !is_submittable_branch_name(&name) {
            return;
        }

        let checkout = match self.popover {
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

    pub(super) fn can_submit_rename_branch(&self, cx: &mut gpui::Context<Self>) -> bool {
        let Some(PopoverKind::RenameBranchPrompt { name, .. }) = &self.popover else {
            return false;
        };
        self.create_branch_input.read_with(cx, |input, _| {
            let new_name = input.text().trim();
            !new_name.is_empty() && new_name != name
        })
    }

    pub(super) fn submit_rename_branch(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::RenameBranchPrompt { repo_id, name, .. }) = self.popover.clone()
        else {
            return;
        };
        let new_name = self
            .create_branch_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if new_name.is_empty() || new_name == name {
            return;
        }
        self.store.dispatch(Msg::RenameBranch {
            repo_id,
            old_name: name,
            new_name,
        });
        self.dismiss_inline_popover(window, cx);
    }

    pub(super) fn can_submit_stash(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.active_repo_id().is_some()
            && self
                .stash_message_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    pub(super) fn submit_commit_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.can_submit_commit_prompt(cx) {
            return;
        }
        let Some(PopoverKind::CommitPrompt { repo_id }) = self.popover.clone() else {
            return;
        };
        let message = self
            .commit_prompt_message_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if message.is_empty() {
            return;
        }
        self.store.dispatch(Msg::Commit {
            repo_id,
            message,
            push_after_commit: false,
        });
        self.commit_prompt_message_drafts.remove(&repo_id);
        self.commit_prompt_message_input
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self.commit_prompt_message_scroll
            .set_offset(point(px(0.0), px(0.0)));
        self.dismiss_inline_popover(window, cx);
    }

    pub(super) fn save_commit_prompt_draft(&mut self, cx: &gpui::Context<Self>) {
        let Some(PopoverKind::CommitPrompt { repo_id }) = self.popover else {
            return;
        };
        let draft: SharedString = self
            .commit_prompt_message_input
            .read(cx)
            .text()
            .to_string()
            .into();
        if draft.is_empty() {
            self.commit_prompt_message_drafts.remove(&repo_id);
        } else {
            self.commit_prompt_message_drafts.insert(repo_id, draft);
        }
    }

    pub(super) fn can_submit_commit_prompt(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.active_repo().is_some_and(|repo| {
            repo.staged_status_entries()
                .is_some_and(|entries| !entries.is_empty())
                || matches!(repo.merge_commit_message, Loadable::Ready(Some(_)))
        }) && self
            .commit_prompt_message_input
            .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    pub(super) fn submit_stash(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
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

    pub(super) fn can_submit_remote_add(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.remote_name_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
            && self
                .remote_url_input
                .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_remote_add(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_remote_add(cx) {
            return;
        }
        let name = self
            .remote_name_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let url = self
            .remote_url_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        self.store.dispatch(Msg::AddRemote { repo_id, name, url });
        self.close_popover(cx);
    }

    pub(super) fn can_submit_remote_edit_url(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.remote_url_edit_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_remote_edit_url(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { name, kind }),
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_remote_edit_url(cx) {
            return;
        }
        let url = self
            .remote_url_edit_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        self.store.dispatch(Msg::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        });
        self.close_popover(cx);
    }

    pub(super) fn can_submit_push_set_upstream(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.push_upstream_branch_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_push_set_upstream(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::PushSetUpstreamPrompt { repo_id, remote }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_push_set_upstream(cx) {
            return;
        }
        let branch = self
            .push_upstream_branch_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        self.store.dispatch(Msg::PushSetUpstream {
            repo_id,
            remote,
            branch,
        });
        self.close_popover(cx);
    }

    pub(super) fn can_submit_checkout_remote_branch(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.create_branch_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_checkout_remote_branch(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::CheckoutRemoteBranchPrompt {
            repo_id,
            remote,
            branch,
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_checkout_remote_branch(cx) {
            return;
        }
        let local_branch = self
            .create_branch_input
            .read_with(cx, |i, _| i.text().trim().to_string());

        let local_branch_exists = self
            .state
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .and_then(|repo| match &repo.branches {
                Loadable::Ready(branches) => {
                    Some(branches.iter().any(|b| b.name == local_branch.as_str()))
                }
                _ => None,
            })
            .unwrap_or(false);
        if local_branch_exists {
            self.push_toast(
                components::ToastKind::Error,
                format!("Branch already exists: {local_branch}"),
                cx,
            );
            return;
        }

        self.store.dispatch(Msg::CheckoutRemoteBranch {
            repo_id,
            remote,
            branch,
            local_branch,
        });
        self.main_pane.update(cx, |pane, cx| {
            pane.rebuild_diff_cache(cx);
            cx.notify();
        });
        self.close_popover(cx);
    }

    pub(super) fn can_submit_worktree_add(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.worktree_path_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_worktree_add(&mut self, cx: &mut gpui::Context<Self>) {
        if self.suppress_worktree_submit_after_ref_enter {
            return;
        }
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_worktree_add(cx) {
            return;
        }
        let folder = self
            .worktree_path_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let reference = self.worktree_ref_source_target.trim().to_string();
        let reference = (!reference.is_empty()).then_some(reference);
        self.store.dispatch(Msg::AddWorktree {
            repo_id,
            path: std::path::PathBuf::from(folder),
            reference,
        });
        self.close_popover(cx);
    }

    pub(super) fn can_submit_submodule_add(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.submodule_url_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
            && self
                .submodule_path_input
                .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_submodule_add(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_submodule_add(cx) {
            return;
        }
        let url = self
            .submodule_url_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let path_text = self
            .submodule_path_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let branch = self.submodule_branch_input.read_with(cx, |i, _| {
            let text = i.text().trim().to_string();
            if text.is_empty() { None } else { Some(text) }
        });
        let name = self.submodule_name_input.read_with(cx, |i, _| {
            let text = i.text().trim().to_string();
            if text.is_empty() { None } else { Some(text) }
        });
        let force = self.submodule_force_enabled;
        self.store.dispatch(Msg::AddSubmodule {
            repo_id,
            url,
            path: std::path::PathBuf::from(path_text),
            branch,
            name,
            force,
        });
        self.close_popover(cx);
    }

    pub(in crate::view) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover(kind, PopoverAnchor::Point(anchor), window, cx);
    }

    pub(in crate::view) fn open_popover_centered(
        &mut self,
        kind: PopoverKind,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover(kind, PopoverAnchor::Centered, window, cx);
    }

    pub(in crate::view) fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover(kind, PopoverAnchor::Bounds(anchor_bounds), window, cx);
    }

    pub(super) fn request_lazy_popover_repo_data(&self, kind: &PopoverKind) {
        let repo_id = match kind {
            PopoverKind::TagMenu { repo_id, .. } | PopoverKind::TagRefMenu { repo_id, .. } => {
                Some(*repo_id)
            }
            PopoverKind::PreviousCommitMessagesMenu { repo_id } => Some(*repo_id),
            PopoverKind::CommitOptionsMenu { repo_id } => Some(*repo_id),
            PopoverKind::BranchPicker { .. } => self.state.active_repo,
            _ => None,
        };
        let Some(repo_id) = repo_id else {
            return;
        };
        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            return;
        };

        if matches!(kind, PopoverKind::BranchPicker { .. }) {
            // Decorates the checkout picker's rows; load once, retry on error.
            if matches!(repo.ref_metadata, Loadable::NotLoaded | Loadable::Error(_)) {
                self.store.dispatch(Msg::LoadRefMetadata { repo_id });
            }
            // Remote branches arrive with the repo's normal refresh; the picker
            // just omits the Remote section until they do.
            return;
        }

        if matches!(
            kind,
            PopoverKind::PreviousCommitMessagesMenu { .. } | PopoverKind::CommitOptionsMenu { .. }
        ) {
            if matches!(
                repo.recent_commit_messages,
                Loadable::NotLoaded | Loadable::Error(_)
            ) {
                self.store
                    .dispatch(Msg::LoadRecentCommitMessages { repo_id, limit: 10 });
            }
            return;
        }

        if matches!(repo.tags, Loadable::NotLoaded | Loadable::Error(_)) {
            self.store.dispatch(Msg::LoadTags { repo_id });
        }
        if matches!(repo.remote_tags, Loadable::NotLoaded | Loadable::Error(_)) {
            self.store.dispatch(Msg::LoadRemoteTags { repo_id });
        }
    }

    pub(super) fn open_popover(
        &mut self,
        kind: PopoverKind,
        anchor: PopoverAnchor,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.save_commit_prompt_draft(cx);
        self.clear_truncated_tooltip(cx);
        // The anchor stays hovered behind the opened surface; keep its
        // tooltip from re-showing on top of the popover.
        crate::view::tooltip::set_tooltips_suppressed_by_overlay(true, cx);
        self.request_lazy_popover_repo_data(&kind);
        if matches!(&kind, PopoverKind::CherryPickCommitConfirm { .. }) {
            self.cherry_pick_mainline = None;
        }
        self.menu_invoker_focus =
            if matches!(&kind, PopoverKind::AppMenu | PopoverKind::AddRepoMenu) {
                window.focused(cx)
            } else {
                None
            };
        // The diff panel takes focus on any left press inside it, so its focus
        // state at open time is a faithful record of where the click landed.
        self.popover_opened_from_diff_panel = self
            .main_pane
            .read(cx)
            .diff_panel_focus_handle
            .is_focused(window);
        let is_context_menu = popover_is_context_menu(&kind);
        let keep_active_invoker = is_context_menu
            || matches!(
                &kind,
                PopoverKind::CreateBranchFromRefPrompt { .. }
                    | PopoverKind::RenameBranchPrompt { .. }
                    | PopoverKind::StashPrompt
                    | PopoverKind::CommitPrompt { .. }
                    | PopoverKind::StashPickerPrompt { .. }
                    // Opened from the AUTHOR column header, which stays
                    // highlighted while its dropdown is up.
                    | PopoverKind::HistoryAuthorFilter { .. }
                    // Action-bar badges stay lit while their picker is open.
                    // Scoped to Checkout: the Delete picker is opened from the
                    // sidebar context menu, whose invoker must still be cleared.
                    | PopoverKind::BranchPicker {
                        purpose: BranchPickerPurpose::Checkout,
                    }
                    | PopoverKind::Repo {
                        kind: RepoPopoverKind::Worktree(WorktreePopoverKind::BadgePicker),
                        ..
                    }
            );
        if !keep_active_invoker {
            self.clear_active_context_menu_invoker(cx);
        }

        self.popover_anchor = Some(anchor);
        self.context_menu_selected_ix = None;
        self.repo_picker_selected_index = None;
        self.repo_picker_search_query.clear();
        // Belongs with the reset above, not with the RepoPicker arm below: every
        // popover kind draws `row_menu_layer`, so a menu left over from a closed
        // picker would spread its occluding scrim over an unrelated popover.
        self.picker_row_menu = None;
        self.branch_picker_selected_index = None;
        self.worktree_picker_selected_index = None;
        self.workspace_picker_selected_index = None;
        self.submodule_picker_selected_index = None;
        self.file_history_selected_index = None;
        self.history_author_filter_selected_index = None;
        // Rows are keyed by the data they were built from, so a stale slot can
        // only be reused when that data is unchanged. Dropping them on open still
        // keeps the memory from outliving the picker that needed it.
        self.branch_picker_rows_cache.clear();
        self.workspace_picker_rows_cache.clear();
        self.repo_picker_rows_cache.clear();
        self.stash_picker_rows_cache.clear();
        self.file_history_rows_cache.clear();
        self.submodule_picker_rows_cache.clear();
        self.worktree_picker_rows_cache.clear();
        self.branch_ref_rows_cache.clear();
        if is_context_menu {
            self.popover = Some(kind);
            self.context_menu_selected_ix = self
                .popover
                .as_ref()
                .and_then(|kind| self.context_menu_model(kind, cx))
                .and_then(|m| m.first_selectable());
            window.focus(&self.context_menu_focus_handle, cx);
        } else {
            match &kind {
                PopoverKind::HookActivity {
                    repo_id,
                    operation_id,
                } => {
                    let operations = self
                        .state
                        .repos
                        .iter()
                        .find(|repo| repo.id == *repo_id)
                        .map(|repo| repo.feedback.hook_activity.as_slice())
                        .unwrap_or_default();
                    let selected = operation_id
                        .filter(|requested| {
                            operations.iter().any(|operation| {
                                operation.id == *requested && operation.has_hooks()
                            })
                        })
                        .or_else(|| {
                            operations
                                .iter()
                                .rev()
                                .find(|operation| operation.has_hooks())
                                .map(|operation| operation.id)
                        });
                    self.hook_activity_selected = selected;
                    self.hook_activity_history_scroll = ScrollHandle::new();
                    self.hook_activity_hooks_scroll = ScrollHandle::new();
                    self.hook_activity_hooks_scroll.scroll_to_bottom();
                    self.hook_activity_output_scroll = ScrollHandle::new();
                    self.hook_activity_output_scroll.scroll_to_bottom();
                }
                PopoverKind::RepoPicker => {
                    let ui_session = session::load();
                    self.repo_picker_sort = repo_picker::sort_from_session(&ui_session);
                    self.cached_recent_repos = ui_session.recent_repos;
                    self.cached_pinned_repos = ui_session.pinned_repos;
                    self.cached_collapsed_picker_sections =
                        ui_session.repo_picker_collapsed_sections;
                    self.repo_picker_sort_menu_open = false;
                    let _ = self.ensure_repo_picker_search_input(window, cx);
                }
                PopoverKind::BranchPicker { .. } => {
                    let _ = self.ensure_branch_picker_search_input(window, cx);
                }
                PopoverKind::CreateBranchFromRefPrompt {
                    source_selectable,
                    target,
                    name_prefix,
                    ..
                } => {
                    let theme = self.theme;
                    self.create_branch_checkout_enabled = true;
                    self.create_branch_source_target = target.clone();
                    if *source_selectable {
                        let _ = self.ensure_branch_picker_search_input(window, cx);
                        if let Some(input) = &self.branch_picker_search_input {
                            input.update(cx, |input, cx| {
                                input.set_text(target.clone(), cx);
                            });
                        }
                    }
                    let name_prefix = name_prefix.clone();
                    self.create_branch_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(name_prefix, cx);
                        cx.notify();
                    });
                    let focus = self
                        .create_branch_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::RenameBranchPrompt { name, .. } => {
                    let theme = self.theme;
                    self.create_branch_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(name.clone(), cx);
                        cx.notify();
                    });
                    let focus = self
                        .create_branch_input
                        .read_with(cx, |input, _| input.focus_handle());
                    window.focus(&focus, cx);
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
                    window.focus(&focus, cx);
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
                    window.focus(&focus, cx);
                }
                PopoverKind::CommitPrompt { repo_id } => {
                    let theme = self.theme;
                    let draft = self
                        .commit_prompt_message_drafts
                        .get(repo_id)
                        .cloned()
                        .unwrap_or_default();
                    self.commit_prompt_message_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(draft.to_string(), cx);
                        cx.notify();
                    });
                    self.commit_prompt_message_scroll
                        .set_offset(point(px(0.0), px(0.0)));
                    let focus = self
                        .commit_prompt_message_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::StashPickerPrompt { .. } => {
                    let _ = self.ensure_stash_picker_search_input(window, cx);
                    self.stash_picker_prompt_selected_index = Some(0);
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
                    window.focus(&focus, cx);
                }
                PopoverKind::SquashPrompt { .. } => {
                    let theme = self.theme;
                    self.squash_prompt_prefilled_range = None;
                    self.squash_message_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.squash_description_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    // The preview may already be Ready (e.g. reopening the same
                    // range); prefill immediately rather than waiting for the
                    // next model update.
                    self.sync_squash_prompt_prefill(cx);
                    let focus = self
                        .squash_message_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::CreateTagPrompt { .. } => {
                    let theme = self.theme;
                    self.create_tag_annotated =
                        matches!(self.state.default_tag_type, DefaultTagType::Annotated);
                    self.create_tag_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.create_tag_message_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self.create_tag_input.read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
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
                    window.focus(&focus, cx);
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
                    window.focus(&focus, cx);
                }
                PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                    ..
                } => {
                    let theme = self.theme;
                    let (path_prefill, reference_prefill) =
                        self.pending_worktree_add_prefill.take().unwrap_or_default();
                    self.worktree_path_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(path_prefill, cx);
                        cx.notify();
                    });
                    self.worktree_ref_source_target = reference_prefill.clone();
                    self.suppress_worktree_submit_after_ref_enter = false;
                    let ref_input = self.ensure_branch_picker_search_input(window, cx);
                    // `ensure_*` blanks the input, so the prefilled ref has to be
                    // written back afterwards or the box would read empty while
                    // submit still used the reference.
                    if !reference_prefill.is_empty() {
                        ref_input.update(cx, |input, cx| {
                            input.set_text(reference_prefill, cx);
                            cx.notify();
                        });
                    }
                    let focus = self
                        .worktree_path_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
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
                    repo_id,
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::BadgePicker),
                } => {
                    let _ = self.ensure_workspace_picker_search_input(window, cx);
                    self.store
                        .dispatch(Msg::LoadWorktrees { repo_id: *repo_id });
                }
                PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
                    ..
                } => {
                    let theme = self.theme;
                    self.submodule_add_advanced_expanded = false;
                    self.submodule_force_enabled = false;
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
                    self.submodule_branch_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.submodule_name_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .submodule_url_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::Repo {
                    kind:
                        RepoPopoverKind::Submodule(SubmodulePopoverKind::ChangePointerPrompt { .. }),
                    ..
                } => {
                    let theme = self.theme;
                    self.submodule_ref_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .submodule_ref_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::TrustConfirm),
                    ..
                } => {}
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
                PopoverKind::HistoryAuthorFilter { .. } => {
                    self.ensure_history_author_filter_search_input(window, cx);
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
                    window.focus(&focus, cx);
                }
                PopoverKind::RebaseReword {
                    ix: _,
                    original_action: _,
                    original_message,
                } => {
                    let theme = self.theme;
                    let (subject, body) = original_message
                        .split_once("\n\n")
                        .map(|(s, b)| (s.to_owned(), b.to_owned()))
                        .unwrap_or_else(|| (original_message.clone(), String::new()));
                    self.rebase_reword_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(subject, cx);
                        cx.notify();
                    });
                    self.rebase_reword_description_input
                        .update(cx, |input, cx| {
                            input.clear_transient_key_presses();
                            input.set_theme(theme, cx);
                            input.set_text(body, cx);
                            cx.notify();
                        });
                    self.rebase_reword_description_scroll
                        .set_offset(point(px(0.0), px(0.0)));
                    let focus = self
                        .rebase_reword_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::RebaseOntoConfirm { .. } => {
                    // Focus the primary (Rebase) button so Enter confirms and
                    // Tab/Esc still reach Cancel.
                    window.focus(&self.rebase_onto_submit_focus_handle, cx);
                }
                // Must sit above the generic confirm-dialog arm below, which
                // would otherwise swallow it and park focus on the tab group
                // instead of the pattern field.
                PopoverKind::AddToGitignorePrompt {
                    repo_id,
                    area,
                    path,
                } => {
                    let (repo_id, area, path) = (*repo_id, *area, path.clone());
                    self.prepare_add_to_gitignore(repo_id, area, &path, window, cx);
                }
                k if popover_is_confirm_dialog(k) => {
                    window.focus(&self.prompt_tab_group_focus_handle, cx);
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

    pub(super) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(super) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in crate::view) fn set_pinned_branches(
        &mut self,
        pinned: std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeSet<String>>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.pinned_branches_by_repo == pinned {
            return;
        }
        self.pinned_branches_by_repo = pinned;
        cx.notify();
    }

    pub(in crate::view) fn set_branch_filter_query(
        &mut self,
        query: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.branch_filter_query == query {
            return;
        }
        self.branch_filter_query = query;
        cx.notify();
    }

    /// The active branch filter, or `None` when it matches everything.
    ///
    /// Mirrors `matches_branch_filter`, which treats a blank query as "no
    /// filter" — so a lone space must not read as a filter that hides
    /// everything.
    pub(in crate::view) fn active_branch_filter(&self) -> Option<&str> {
        let query = self.branch_filter_query.trim();
        (!query.is_empty()).then_some(query)
    }

    pub(in crate::view) fn set_collapsed_items(
        &mut self,
        collapsed: std::collections::BTreeMap<
            std::path::PathBuf,
            std::collections::BTreeSet<String>,
        >,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.collapsed_items_by_repo == collapsed {
            return;
        }
        self.collapsed_items_by_repo = collapsed;
        cx.notify();
    }

    /// Whether a sidebar collapse key is currently collapsed, going through
    /// `branch_sidebar::is_collapsed` so default-collapsed sections and the
    /// inverted `expanded:` storage are read the same way the tree reads them.
    pub(in crate::view) fn sidebar_collapse_key_is_collapsed(
        &self,
        repo_id: RepoId,
        collapse_key: &str,
    ) -> bool {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return false;
        };
        // A repo with nothing stored reads the same as one with an empty set:
        // `is_collapsed` answers from the key's own default in both cases.
        static EMPTY: std::sync::LazyLock<std::collections::BTreeSet<String>> =
            std::sync::LazyLock::new(std::collections::BTreeSet::new);
        let items = self
            .collapsed_items_by_repo
            .get(&repo.spec.workdir)
            .unwrap_or(&EMPTY);
        crate::view::branch_sidebar::is_collapsed(items, collapse_key)
    }

    /// How many pinned branches the section is actually showing, for the pinned
    /// header's "Unpin all (N)".
    ///
    /// Counting raw pin keys would overcount: the row builder skips a pin whose
    /// branch no longer exists, and skips one filtered out by the branch
    /// filter, so "Unpin all (3)" could sit above a single row.
    pub(in crate::view) fn pinned_branch_count(
        &self,
        repo_id: RepoId,
        section: BranchSection,
    ) -> usize {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return 0;
        };
        let filter = self.active_branch_filter().unwrap_or_default();
        self.pinned_branches_by_repo
            .get(&repo.spec.workdir)
            .map_or(0, |items| {
                items
                    .iter()
                    .filter(|key| {
                        crate::view::branch_sidebar::pinned_branch_renders(
                            repo, key, section, filter,
                        )
                    })
                    .count()
            })
    }

    pub(in crate::view) fn is_branch_pinned(
        &self,
        repo_id: RepoId,
        section: BranchSection,
        name: &str,
    ) -> bool {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return false;
        };
        let key = crate::view::branch_sidebar::branch_pin_storage_key(section, name);
        self.pinned_branches_by_repo
            .get(&repo.spec.workdir)
            .is_some_and(|items| items.contains(&key))
    }

    pub(in crate::view) fn set_date_time_format(
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
        self.sync_pane_date_settings(cx);
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_timezone(&mut self, next: Timezone, cx: &mut gpui::Context<Self>) {
        if self.timezone == next {
            return;
        }
        self.timezone = next;
        self.main_pane
            .update(cx, |pane, cx| pane.set_timezone(next, cx));
        self.sync_pane_date_settings(cx);
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_show_timezone(
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
        self.sync_pane_date_settings(cx);
        self.schedule_ui_settings_persist(cx);
    }

    pub(super) fn sync_pane_date_settings(&mut self, cx: &mut gpui::Context<Self>) {
        let (format, timezone, show_timezone) =
            (self.date_time_format, self.timezone, self.show_timezone);
        self.details_pane.update(cx, |pane, cx| {
            pane.set_date_settings(format, timezone, show_timezone, cx);
        });
        self.reflog_pane.update(cx, |pane, cx| {
            pane.set_date_settings(format, timezone, show_timezone, cx);
        });
    }

    pub(in crate::view) fn set_theme_mode(
        &mut self,
        next: ThemeMode,
        appearance: gpui::WindowAppearance,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.theme_mode == next {
            return;
        }

        self.theme_mode = next.clone();
        self.set_theme(next.resolve_theme(appearance), cx);
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.set_theme_mode(next.clone(), appearance, cx);
            });
        });
    }

    pub(super) fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        let mode = self.theme_mode.clone();
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

    pub(in crate::view) fn sync_change_tracking_view(
        &mut self,
        next: ChangeTrackingView,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.change_tracking_view == next {
            return;
        }

        self.change_tracking_view = next;
        if matches!(self.popover, Some(PopoverKind::ChangeTrackingSettings)) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_commit_push_after_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_push_after_enabled == enabled {
            return;
        }

        self.commit_push_after_enabled = enabled;
        if matches!(self.popover, Some(PopoverKind::CommitOptionsMenu { .. })) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_commit_amend_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_amend_enabled == enabled {
            return;
        }

        self.commit_amend_enabled = enabled;
        if matches!(self.popover, Some(PopoverKind::CommitOptionsMenu { .. })) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_diff_content_mode(
        &mut self,
        next: DiffContentMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_content_mode == next {
            return;
        }

        self.diff_content_mode = next;
        if matches!(self.popover, Some(PopoverKind::DiffContentModeSettings)) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_diff_whitespace_mode(
        &mut self,
        next: DiffWhitespaceMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_whitespace_mode == next {
            return;
        }

        self.diff_whitespace_mode = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_diff_reveal_whitespace_chars(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_reveal_whitespace_chars == next {
            return;
        }

        self.diff_reveal_whitespace_chars = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_diff_word_wrap(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_word_wrap == next {
            return;
        }

        self.diff_word_wrap = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_diff_show_line_numbers(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_show_line_numbers == next {
            return;
        }

        self.diff_show_line_numbers = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub(super) fn install_linux_desktop_integration(&mut self, cx: &mut gpui::Context<Self>) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.install_linux_desktop_integration(cx);
        });
    }

    /// The search input of whichever picker is open, so a row menu floating over
    /// it can read the filter without knowing which picker it is over.
    pub(super) fn open_picker_search_input(&self) -> Option<&Entity<components::TextInput>> {
        match &self.popover {
            Some(PopoverKind::RepoPicker) => self.repo_picker_search_input.as_ref(),
            Some(PopoverKind::BranchPicker { .. }) => self.branch_picker_search_input.as_ref(),
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::BadgePicker),
                ..
            }) => self.workspace_picker_search_input.as_ref(),
            _ => None,
        }
    }

    /// The selection index of whichever picker is open. A row menu parks it while
    /// it is up — the arrow keys walk the menu then — and restores it on the way
    /// out. **Every picker kind that can host a row menu has to be here**; a
    /// missing arm parks the wrong picker's selection with nothing on screen to
    /// say so.
    pub(super) fn open_picker_selected_index(&mut self) -> Option<&mut Option<usize>> {
        match &self.popover {
            Some(PopoverKind::RepoPicker) => Some(&mut self.repo_picker_selected_index),
            Some(PopoverKind::BranchPicker { .. }) => Some(&mut self.branch_picker_selected_index),
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::BadgePicker),
                ..
            }) => Some(&mut self.workspace_picker_selected_index),
            _ => None,
        }
    }

    pub(super) fn open_picker_selected_index_value(&self) -> Option<usize> {
        match &self.popover {
            Some(PopoverKind::RepoPicker) => self.repo_picker_selected_index,
            Some(PopoverKind::BranchPicker { .. }) => self.branch_picker_selected_index,
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::BadgePicker),
                ..
            }) => self.workspace_picker_selected_index,
            _ => None,
        }
    }

    pub(super) fn push_toast(
        &mut self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.push_toast(kind, message, cx);
        });
    }
}
