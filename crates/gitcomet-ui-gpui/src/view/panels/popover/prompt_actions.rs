use super::*;

impl PopoverHost {
    pub(in crate::view) fn popover_view(
        &mut self,
        kind: PopoverKind,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let ui_scale = popover_ui_scale(cx);
        let ui_scale_percent = ui_scale.percent();
        let scaled_px = |value: f32| popover_scaled_px(value, ui_scale);
        let anchor_source = self
            .popover_anchor
            .clone()
            .unwrap_or_else(|| PopoverAnchor::Point(point(px(64.0), px(64.0))));
        let anchor_is_bounds = matches!(&anchor_source, PopoverAnchor::Bounds(_));
        let window_bounds = window.window_bounds().get_bounds();
        let window_w = window_bounds.size.width;
        let window_h = window_bounds.size.height;
        let margin_x = scaled_px(16.0);
        let margin_y = scaled_px(16.0);

        let is_app_menu = matches!(&kind, PopoverKind::AppMenu);
        let is_context_menu = popover_is_context_menu(&kind);
        let center_hook_workflow = matches!(&kind, PopoverKind::HookActivity { .. });
        let mut anchor_corner = popover_anchor_corner(&kind);

        let anchor_for_corner = |corner: Anchor| match &anchor_source {
            PopoverAnchor::Point(point) => *point,
            PopoverAnchor::Bounds(bounds) => match corner {
                Anchor::TopRight => bounds.bottom_right(),
                Anchor::BottomLeft => bounds.origin,
                Anchor::BottomRight => bounds.top_right(),
                _ => bounds.bottom_left(),
            },
            PopoverAnchor::Centered => point(px(0.0), px(0.0)),
        };

        // Some popovers have large minimum widths. If the anchor is close to the edge, the popover
        // can end up constrained to a very narrow width (making inputs unusably small). Prefer the
        // side with more horizontal space in those cases.
        let mut anchor = anchor_for_corner(anchor_corner);
        let preferred_width = popover_preferred_anchor_width(&kind, ui_scale);
        let space_left = (anchor.x - margin_x).max(px(0.0));
        let space_right = (window_w - margin_x - anchor.x).max(px(0.0));
        anchor_corner =
            choose_popover_anchor_corner(anchor_corner, space_left, space_right, preferred_width);
        anchor = anchor_for_corner(anchor_corner);

        let panel = match kind {
            PopoverKind::HookActivity {
                repo_id,
                operation_id,
            } => hook_activity::panel(self, repo_id, operation_id, window, cx),
            PopoverKind::RepoPicker => repo_picker::panel(self, cx),
            PopoverKind::BranchPicker { .. } => branch_picker::panel(self, cx),
            PopoverKind::CreateBranchFromRefPrompt {
                repo_id,
                target,
                source_selectable,
                // Consumed when the popover opens, seeding the name input.
                name_prefix: _,
            } => create_branch_from_ref_prompt::panel(
                self,
                repo_id,
                target,
                source_selectable,
                window,
                cx,
            ),
            PopoverKind::RenameBranchPrompt {
                repo_id,
                name,
                is_current_branch,
            } => rename_branch_prompt::panel(self, repo_id, name, is_current_branch, cx),
            PopoverKind::CheckoutRemoteBranchPrompt {
                repo_id,
                remote,
                branch,
            } => checkout_remote_branch_prompt::panel(self, repo_id, remote, branch, cx),
            PopoverKind::StashPrompt => stash_prompt::panel(self, cx),
            PopoverKind::CommitPrompt { repo_id } => commit_prompt::panel(self, repo_id, cx),
            PopoverKind::StashPickerPrompt { repo_id, purpose } => {
                stash_picker_prompt::panel(self, repo_id, purpose, cx)
            }
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
            PopoverKind::SquashPrompt { repo_id } => squash_prompt::panel(self, repo_id, cx),
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
                    RemotePopoverKind::Menu { name } => self.context_menu_view(
                        PopoverKind::remote(repo_id, RemotePopoverKind::Menu { name }),
                        cx,
                    ),
                },
                RepoPopoverKind::Worktree(worktree_kind) => match worktree_kind {
                    WorktreePopoverKind::SectionMenu => self.context_menu_view(
                        PopoverKind::worktree(repo_id, WorktreePopoverKind::SectionMenu),
                        cx,
                    ),
                    WorktreePopoverKind::Menu { path, branch } => self.context_menu_view(
                        PopoverKind::worktree(repo_id, WorktreePopoverKind::Menu { path, branch }),
                        cx,
                    ),
                    WorktreePopoverKind::AddPrompt => {
                        worktree_add_prompt::panel(self, repo_id, window, cx)
                    }
                    WorktreePopoverKind::OpenPicker => {
                        worktree_picker::panel(self, repo_id, false, cx)
                    }
                    WorktreePopoverKind::RemovePicker => {
                        worktree_picker::panel(self, repo_id, true, cx)
                    }
                    WorktreePopoverKind::BadgePicker => workspace_picker::panel(self, repo_id, cx),
                    WorktreePopoverKind::RemoveConfirm { path, branch } => {
                        worktree_remove_confirm::panel(self, repo_id, path, branch, cx)
                    }
                },
                RepoPopoverKind::Submodule(submodule_kind) => match submodule_kind {
                    SubmodulePopoverKind::SectionMenu => self.context_menu_view(
                        PopoverKind::submodule(repo_id, SubmodulePopoverKind::SectionMenu),
                        cx,
                    ),
                    SubmodulePopoverKind::Menu { path } => self.context_menu_view(
                        PopoverKind::submodule(repo_id, SubmodulePopoverKind::Menu { path }),
                        cx,
                    ),
                    SubmodulePopoverKind::AddPrompt => {
                        submodule_add_prompt::panel(self, repo_id, cx)
                    }
                    SubmodulePopoverKind::ChangePointerPrompt { path } => {
                        submodule_change_pointer_prompt::panel(self, repo_id, &path, cx)
                    }
                    SubmodulePopoverKind::TrustConfirm => {
                        submodule_trust_confirm::panel(self, repo_id, cx)
                    }
                    SubmodulePopoverKind::OpenPicker => {
                        submodule_picker::panel(self, repo_id, false, cx)
                    }
                    SubmodulePopoverKind::RemovePicker => {
                        submodule_picker::panel(self, repo_id, true, cx)
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
            PopoverKind::CherryPickCommitConfirm { repo_id, commit_id } => {
                cherry_pick_commit_confirm::panel(self, repo_id, commit_id, cx)
            }
            PopoverKind::MergeCommitConfirm { repo_id, commit_id } => {
                merge_commit_confirm::panel(self, repo_id, commit_id, cx)
            }
            PopoverKind::MergeAbortConfirm { repo_id } => {
                merge_abort_confirm::panel(self, repo_id, cx)
            }
            PopoverKind::ForceDeleteBranchConfirm { repo_id, name } => {
                force_delete_branch_confirm::panel(self, repo_id, name, cx)
            }
            PopoverKind::ForceRemoveWorktreeConfirm {
                repo_id,
                path,
                branch,
            } => force_remove_worktree_confirm::panel(self, repo_id, path, branch, cx),
            PopoverKind::DiscardChangesConfirm {
                repo_id,
                area,
                path,
            } => discard_changes_confirm::panel(self, repo_id, area, path.clone(), cx),
            PopoverKind::AddToGitignorePrompt {
                repo_id,
                area,
                path,
            } => add_to_gitignore_prompt::panel(self, repo_id, area, path.clone(), cx),
            PopoverKind::StageConflictMarkersConfirm {
                repo_id,
                paths,
                unresolved,
                clear_selection,
            } => stage_conflict_markers_confirm::panel(
                self,
                repo_id,
                paths.clone(),
                unresolved.clone(),
                clear_selection,
                cx,
            ),
            PopoverKind::PullReconcilePrompt { repo_id } => {
                pull_reconcile_prompt::panel(self, repo_id, cx)
            }
            PopoverKind::DiffActionMenu => self.context_menu_view(PopoverKind::DiffActionMenu, cx),
            PopoverKind::WebLinkMenu { url } => {
                self.context_menu_view(PopoverKind::WebLinkMenu { url }, cx)
            }
            PopoverKind::CommitShaLinkMenu {
                repo_id,
                commit_id,
                allow_navigate,
            } => self.context_menu_view(
                PopoverKind::CommitShaLinkMenu {
                    repo_id,
                    commit_id,
                    allow_navigate,
                },
                cx,
            ),
            PopoverKind::MergetoolSettingsMenu => {
                self.context_menu_view(PopoverKind::MergetoolSettingsMenu, cx)
            }
            PopoverKind::TerminalMenu { repo_id, context } => {
                self.context_menu_view(PopoverKind::TerminalMenu { repo_id, context }, cx)
            }
            PopoverKind::HistoryBranchFilter { repo_id } => {
                self.context_menu_view(PopoverKind::HistoryBranchFilter { repo_id }, cx)
            }
            PopoverKind::HistoryAuthorFilter { repo_id } => author_filter::panel(self, repo_id, cx),
            PopoverKind::DiffContentModeSettings => {
                self.context_menu_view(PopoverKind::DiffContentModeSettings, cx)
            }
            PopoverKind::ChangeTrackingSettings => {
                self.context_menu_view(PopoverKind::ChangeTrackingSettings, cx)
            }
            PopoverKind::UiScalePicker => self.context_menu_view(PopoverKind::UiScalePicker, cx),
            PopoverKind::PullPicker => self.context_menu_view(PopoverKind::PullPicker, cx),
            PopoverKind::PushPicker => self.context_menu_view(PopoverKind::PushPicker, cx),
            PopoverKind::CommitOptionsMenu { repo_id } => {
                self.context_menu_view(PopoverKind::CommitOptionsMenu { repo_id }, cx)
            }
            PopoverKind::PreviousCommitMessagesMenu { repo_id } => {
                self.context_menu_view(PopoverKind::PreviousCommitMessagesMenu { repo_id }, cx)
            }
            PopoverKind::RepoTabMenu { repo_id } => {
                self.context_menu_view(PopoverKind::RepoTabMenu { repo_id }, cx)
            }
            PopoverKind::CommitMenu { repo_id, commit_id } => {
                self.context_menu_view(PopoverKind::CommitMenu { repo_id, commit_id }, cx)
            }
            PopoverKind::ReflogEntryMenu {
                repo_id,
                target,
                selector,
            } => self.context_menu_view(
                PopoverKind::ReflogEntryMenu {
                    repo_id,
                    target,
                    selector,
                },
                cx,
            ),
            PopoverKind::TagMenu { repo_id, commit_id } => {
                self.context_menu_view(PopoverKind::TagMenu { repo_id, commit_id }, cx)
            }
            PopoverKind::TagRefMenu {
                repo_id,
                commit_id,
                name,
            } => self.context_menu_view(
                PopoverKind::TagRefMenu {
                    repo_id,
                    commit_id,
                    name,
                },
                cx,
            ),
            PopoverKind::DiffHunkMenu { repo_id, src_ix } => {
                self.context_menu_view(PopoverKind::DiffHunkMenu { repo_id, src_ix }, cx)
            }
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
                copy_target,
            } => self.context_menu_view(
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
                    copy_target,
                },
                cx,
            ),
            PopoverKind::ConflictResolverInputRowMenu {
                line_label,
                line_target,
                chunk_label,
                chunk_target,
            } => self.context_menu_view(
                PopoverKind::ConflictResolverInputRowMenu {
                    line_label,
                    line_target,
                    chunk_label,
                    chunk_target,
                },
                cx,
            ),
            PopoverKind::ConflictResolverChunkMenu {
                conflict_ix,
                has_base,
                is_three_way,
                selected_choices,
                output_line_ix,
                split_selection_rows,
                join_previous_region,
                join_next_region,
                alignment_marked_columns,
                has_manual_alignments,
                output_is_protected,
            } => self.context_menu_view(
                PopoverKind::ConflictResolverChunkMenu {
                    conflict_ix,
                    has_base,
                    is_three_way,
                    selected_choices,
                    output_line_ix,
                    split_selection_rows,
                    join_previous_region,
                    join_next_region,
                    alignment_marked_columns,
                    has_manual_alignments,
                    output_is_protected,
                },
                cx,
            ),
            PopoverKind::ConflictResolverOutputMenu {
                cursor_line,
                selected_text,
                has_source_a,
                has_source_b,
                has_source_c,
                is_three_way,
            } => self.context_menu_view(
                PopoverKind::ConflictResolverOutputMenu {
                    cursor_line,
                    selected_text,
                    has_source_a,
                    has_source_b,
                    has_source_c,
                    is_three_way,
                },
                cx,
            ),
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
            PopoverKind::BranchRefsMenu {
                repo_id,
                display_name,
                targets,
            } => self.context_menu_view(
                PopoverKind::BranchRefsMenu {
                    repo_id,
                    display_name,
                    targets,
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
            } => self.context_menu_view(
                PopoverKind::StashMenu {
                    repo_id,
                    index,
                    message,
                },
                cx,
            ),
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
            PopoverKind::FileBrowserFileMenu { repo_id, path } => {
                self.context_menu_view(PopoverKind::FileBrowserFileMenu { repo_id, path }, cx)
            }
            PopoverKind::FileBrowserFolderMenu { repo_id, path } => {
                self.context_menu_view(PopoverKind::FileBrowserFolderMenu { repo_id, path }, cx)
            }
            PopoverKind::BranchGroupMenu {
                repo_id,
                section,
                remote,
                path,
            } => self.context_menu_view(
                PopoverKind::BranchGroupMenu {
                    repo_id,
                    section,
                    remote,
                    path,
                },
                cx,
            ),
            PopoverKind::PinnedSectionMenu { repo_id, section } => {
                self.context_menu_view(PopoverKind::PinnedSectionMenu { repo_id, section }, cx)
            }
            PopoverKind::DeleteBranchesConfirm {
                repo_id,
                section,
                remote,
                group_label,
                names,
            } => delete_branches_confirm::panel(
                self,
                repo_id,
                section,
                remote,
                group_label,
                names,
                cx,
            ),
            PopoverKind::BrowseHistoryMenu { repo_id } => {
                self.context_menu_view(PopoverKind::BrowseHistoryMenu { repo_id }, cx)
            }
            PopoverKind::SubmoduleInnerDiffMenu {
                repo_id,
                submodule_repo_path,
                target,
            } => self.context_menu_view(
                PopoverKind::SubmoduleInnerDiffMenu {
                    repo_id,
                    submodule_repo_path,
                    target,
                },
                cx,
            ),
            kind @ (PopoverKind::AppMenu | PopoverKind::AddRepoMenu) => {
                self.context_menu_view(kind, cx)
            }
            PopoverKind::RebaseOntoConfirm { repo_id, onto } => {
                rebase_onto_confirm::panel(self, repo_id, onto, cx)
            }
            PopoverKind::InteractiveRebaseActionMenu { .. }
            | PopoverKind::InteractiveRebaseAutosquashMenu => {
                self.context_menu_view(kind.clone(), cx)
            }
            PopoverKind::RebaseReword {
                ix,
                original_action,
                original_message: _,
            } => {
                let theme = self.theme;
                let submit_button_id = "reword_save";
                let main_pane = self.main_pane.clone();
                let submit = cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                    let subject = this
                        .rebase_reword_input
                        .read_with(cx, |input, _| input.text().to_string());
                    let body = this
                        .rebase_reword_description_input
                        .read_with(cx, |input, _| input.text().to_string());
                    let new_message = if body.trim().is_empty() {
                        subject.clone()
                    } else {
                        format!("{subject}\n\n{body}")
                    };
                    main_pane.update(cx, |pane, cx| {
                        if subject.is_empty() {
                            // Empty subject → discard any previous override and revert
                            // the action. Use set_rebase_action so side-effects
                            // (squash-target cleanup, notify) are handled consistently.
                            if let Some(entry) = pane
                                .active_irebase_mut()
                                .and_then(|st| st.entries.get_mut(ix))
                            {
                                entry.new_message = None;
                            }
                            pane.set_rebase_action(ix, original_action, cx);
                        } else if let Some(entry) = pane
                            .active_irebase_mut()
                            .and_then(|st| st.entries.get_mut(ix))
                        {
                            entry.action = InteractiveRebaseAction::Reword;
                            entry.new_message = Some(new_message);
                            cx.notify();
                        }
                    });
                    this.close_popover_and_restore_focus(window, cx);
                });
                let cancel = cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        pane.set_rebase_action(ix, original_action, cx);
                    });
                    this.close_popover_and_restore_focus(window, cx);
                });

                div()
                    .flex()
                    .flex_col()
                    .w(scaled_px(440.0))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child("Reword commit message"),
                    )
                    .child(div().border_t_1().border_color(theme.colors.stroke.default))
                    .child(
                        div()
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child("Commit message"),
                            )
                            .child(self.rebase_reword_input.clone()),
                    )
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .pb_2()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child("Description"),
                            )
                            .child(
                                components::ScrollContainer::vertical(
                                    "rebase_reword_description_scroll_surface",
                                    "rebase_reword_description_scrollbar",
                                    self.rebase_reword_description_scroll.clone(),
                                    scaled_px(180.0),
                                )
                                .debug_selector("rebase_reword_description_scroll_surface")
                                .render(theme, self.rebase_reword_description_input.clone()),
                            ),
                    )
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .text_xs()
                            .text_color(theme.colors.foreground.secondary)
                            .child(
                                "Clear the message and save to keep the original commit message.",
                            ),
                    )
                    .child(div().border_t_1().border_color(theme.colors.stroke.default))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                components::Button::new("reword_cancel", "Cancel")
                                    .separated_end_slot(hotkey_hint(
                                        theme,
                                        "reword_cancel_hint",
                                        "Esc",
                                    ))
                                    .style(components::ButtonStyle::Outlined)
                                    .render(theme, ui_scale_percent)
                                    .on_click(cancel),
                            )
                            .child(
                                components::Button::new(submit_button_id, "Save message")
                                    .style(components::ButtonStyle::Filled)
                                    .render(theme, ui_scale_percent)
                                    .on_click(submit),
                            ),
                    )
            }
            PopoverKind::TerminalShutdownConfirm(prompt) => {
                terminal_shutdown_confirm::panel(self, prompt, cx)
            }
            PopoverKind::UnsavedFileEditsConfirm(prompt) => {
                unsaved_file_edits_confirm::panel(self, prompt, cx)
            }
        };

        let is_right = matches!(anchor_corner, Anchor::TopRight | Anchor::BottomRight);
        let popover_border_color = theme.colors.stroke.default;
        let gap_y = if is_app_menu {
            crate::view::chrome::title_bar_height(ui_scale_percent)
        } else if anchor_is_bounds {
            px(1.0)
        } else if is_right {
            scaled_px(10.0)
        } else {
            scaled_px(8.0)
        };

        let mut context_menu_max_panel_h: Option<Pixels> = None;
        if is_context_menu {
            let (below_anchor_y, above_anchor_y) = match &anchor_source {
                PopoverAnchor::Point(_) => (anchor.y, anchor.y),
                PopoverAnchor::Bounds(bounds) => (bounds.bottom_left().y, bounds.origin.y),
                PopoverAnchor::Centered => (anchor.y, anchor.y),
            };
            let below = (window_h - margin_y) - (below_anchor_y + gap_y);
            let above = (above_anchor_y - gap_y) - margin_y;
            if below < scaled_px(240.0) && above > below {
                anchor_corner = match anchor_corner {
                    Anchor::TopLeft => Anchor::BottomLeft,
                    Anchor::TopRight => Anchor::BottomRight,
                    corner => corner,
                };
            }
            if anchor_is_bounds {
                anchor = anchor_for_corner(anchor_corner);
            }

            let popover_edge_y = match anchor_corner {
                Anchor::BottomLeft | Anchor::BottomRight => anchor.y - gap_y,
                _ => anchor.y + gap_y,
            };
            let max_popover_h = match anchor_corner {
                Anchor::BottomLeft | Anchor::BottomRight => popover_edge_y - margin_y,
                _ => (window_h - margin_y) - popover_edge_y,
            }
            .max(px(0.0));
            let max_panel_h = (max_popover_h - scaled_px(12.0)).max(px(0.0));
            context_menu_max_panel_h = Some(max_panel_h);
        }

        let offset_y = match anchor_corner {
            Anchor::BottomLeft | Anchor::BottomRight => -gap_y,
            _ => gap_y,
        };

        let panel = if let Some(max_panel_h) = context_menu_max_panel_h {
            restrict_scroll_to_vertical_axis(
                div()
                    .id("context_menu_scroll")
                    .min_h(px(0.0))
                    .max_h(max_panel_h)
                    .overflow_y_scroll(),
            )
            .child(panel)
            .into_any_element()
        } else {
            panel.into_any_element()
        };

        let prompt_tab_navigation_enabled = self.prompt_tab_navigation_enabled();
        let panel = if prompt_tab_navigation_enabled {
            div()
                .track_focus(&self.prompt_tab_group_focus_handle)
                .tab_group()
                .child(panel)
                .child(
                    div()
                        .track_focus(&self.prompt_tab_wrap_end_focus_handle)
                        .w(px(0.0))
                        .h(px(0.0)),
                )
                .into_any_element()
        } else {
            panel
        };

        // Centered prompts are modal dialogs; anchored popovers (menus,
        // pickers) float just above the content and take the lighter lift.
        let is_centered = matches!(self.popover_anchor, Some(PopoverAnchor::Centered));
        let popover_surface = if is_centered {
            components::modal_surface(theme)
        } else {
            components::popover_surface(theme).border_color(popover_border_color)
        };
        let mut popover_container = popover_surface
            .id("app_popover")
            .debug_selector(|| "app_popover".to_string())
            .on_any_mouse_down(|_e, _w, cx| cx.stop_propagation())
            // `occlude` keeps the root view's mouse-move listener from firing
            // over the popover, so the tooltip host would otherwise anchor
            // truncated-text tooltips to wherever the pointer was before the
            // popover opened. Feed it positions from inside the popover.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _window, cx| {
                let _ = this
                    .tooltip_host
                    .update(cx, |host, cx| host.on_mouse_moved(e.position, cx));
            }))
            .occlude()
            .p_1()
            .child(panel);

        if prompt_tab_navigation_enabled || center_hook_workflow {
            popover_container = popover_container
                .key_context("PopoverPrompt")
                .on_action(cx.listener(Self::dismiss_prompt))
                .on_action(cx.listener(Self::focus_next_prompt_field))
                .on_action(cx.listener(Self::focus_prev_prompt_field));
        }

        if is_centered {
            let top_offset = scaled_px(80.0);
            let scrim_close = cx.listener(|this, _: &MouseDownEvent, window, cx| {
                if !this.dismiss_hook_activity_workflow(window, cx) {
                    this.close_popover_and_restore_focus(window, cx);
                }
            });
            let placement = div()
                .absolute()
                .left_0()
                .w_full()
                .flex()
                .justify_center()
                .when(center_hook_workflow, |placement| {
                    placement.top_0().h_full().items_center()
                })
                .when(!center_hook_workflow, |placement| placement.top(top_offset))
                .child(div().child(popover_container));
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(components::modal_scrim(theme).on_mouse_down(MouseButton::Left, scrim_close))
                .child(placement)
                .into_any_element()
        } else {
            anchored()
                .position(anchor)
                .anchor(anchor_corner)
                .offset(point(px(0.0), offset_y))
                .child(popover_container)
                .into_any_element()
        }
    }
}
