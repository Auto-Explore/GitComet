//! Branch prompts: create, rename, check out a remote branch, and the
//! inline branch picker; each validates its fields and dispatches.

use super::*;

impl PopoverHost {
    pub(in crate::view::panels::popover) fn resolve_open_branch_exists_prompt(
        &self,
        choice: BranchExistsChoice,
    ) -> bool {
        let Some(PopoverKind::BranchExistsPrompt {
            repo_id,
            name,
            target,
            operation,
        }) = self.popover.as_ref()
        else {
            return false;
        };

        self.store.dispatch(Msg::ResolveBranchExistsPrompt {
            prompt: BranchExistsPromptState {
                repo_id: *repo_id,
                name: name.clone(),
                target: target.clone(),
                operation: operation.clone(),
            },
            choice,
        });
        true
    }

    pub(in crate::view::panels::popover) fn inline_branch_picker_active(&self) -> bool {
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

    pub(in crate::view::panels::popover) fn handle_inline_branch_picker_escape(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
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

    pub(in crate::view::panels::popover) fn handle_inline_branch_picker_select(
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

    pub(in crate::view::panels::popover) fn can_submit_create_branch(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.create_branch_prompt_repo_and_target().is_some()
            && self
                .create_branch_input
                .read_with(cx, |input, _| is_submittable_branch_name(input.text()))
    }

    pub(in crate::view::panels::popover) fn create_branch_prompt_repo_and_target(
        &self,
    ) -> Option<(RepoId, String)> {
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

    pub(in crate::view::panels::popover) fn submit_create_branch(
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
                force: false,
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

    pub(in crate::view::panels::popover) fn can_submit_rename_branch(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(PopoverKind::RenameBranchPrompt { name, .. }) = &self.popover else {
            return false;
        };
        self.create_branch_input.read_with(cx, |input, _| {
            let new_name = input.text().trim();
            is_submittable_branch_name(new_name) && new_name != name
        })
    }

    pub(in crate::view::panels::popover) fn submit_rename_branch(
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
        if !is_submittable_branch_name(&new_name) || new_name == name {
            return;
        }
        self.store.dispatch(Msg::RenameBranch {
            repo_id,
            old_name: name,
            new_name,
            force: false,
        });
        self.dismiss_inline_popover(window, cx);
    }

    pub(in crate::view::panels::popover) fn can_submit_checkout_remote_branch(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.create_branch_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn submit_checkout_remote_branch(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
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
            self.store.dispatch(Msg::ShowBranchExistsPrompt {
                prompt: BranchExistsPromptState {
                    repo_id,
                    name: local_branch,
                    target: format!("{remote}/{branch}"),
                    operation: BranchExistsPromptOperation::CheckoutRemoteBranch { remote, branch },
                },
            });
            return;
        }

        self.dispatch_checkout_remote_branch(
            repo_id,
            remote,
            branch,
            local_branch,
            CheckoutRemoteBranchMode::Create,
            cx,
        );
    }

    pub(in crate::view::panels::popover) fn dispatch_checkout_remote_branch(
        &mut self,
        repo_id: RepoId,
        remote: String,
        branch: String,
        local_branch: String,
        mode: CheckoutRemoteBranchMode,
        cx: &mut gpui::Context<Self>,
    ) {
        self.store.dispatch(Msg::CheckoutRemoteBranch {
            repo_id,
            remote,
            branch,
            local_branch,
            mode,
        });
        self.main_pane.update(cx, |pane, cx| {
            pane.rebuild_diff_cache(cx);
            cx.notify();
        });
        self.close_popover(cx);
    }
}
