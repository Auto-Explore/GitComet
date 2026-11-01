//! Repository prompts: tags, clones, stashes, commits, squashes, worktrees,
//! and submodules.

use super::*;
use crate::view::pending_commit::PendingCommit;

impl PopoverHost {
    /// Validates the repo's current multi-selection against its loaded log and
    /// HEAD, returning a squash plan when the selection is eligible. Shared by
    /// the squash prompt's render, prefill, and submit paths so they always
    /// agree on the range.
    pub(in crate::view) fn squash_plan_for_repo_id(
        &self,
        repo_id: RepoId,
    ) -> Option<gitcomet_core::squash::SquashPlan> {
        let repo = self.state.repos.iter().find(|r| r.id == repo_id)?;
        repo.history_squash_plan()
    }

    /// Populates the squash prompt's inputs from the loaded message preview.
    /// Only fires when the preview matches the live plan's range (never a stale
    /// preview from an earlier selection) and only while both inputs are still
    /// empty for a range not yet prefilled (never over the user's own text).
    pub(in crate::view::panels::popover) fn sync_squash_prompt_prefill(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
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
    pub(in crate::view::panels::popover) fn submit_squash(&mut self, cx: &mut gpui::Context<Self>) {
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

    pub(in crate::view::panels::popover) fn can_submit_create_tag(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        matches!(self.popover, Some(PopoverKind::CreateTagPrompt { .. }))
            && self
                .create_tag_input
                .read_with(cx, |input, _| is_submittable_branch_name(input.text()))
    }

    pub(in crate::view::panels::popover) fn can_submit_clone_repo(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        matches!(self.popover, Some(PopoverKind::CloneRepo))
            && self
                .clone_repo_url_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
            && self
                .clone_repo_parent_dir_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn can_submit_submodule_change_pointer(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
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

    pub(in crate::view::panels::popover) fn submit_create_tag(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
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

    pub(in crate::view::panels::popover) fn submit_clone_repo(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
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

    pub(in crate::view::panels::popover) fn submit_annex_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Annex(AnnexPopoverKind::Prompt(prompt)),
        }) = self.popover.clone()
        else {
            return;
        };
        let text = self
            .submodule_ref_input
            .read_with(cx, |input, _| input.text().to_string());
        let unused =
            annex_prompt::droppable_unused(self.state.repos.iter().find(|r| r.id == repo_id));
        let Ok(command) = annex_prompt::prompt_command(&prompt, &text, unused) else {
            return;
        };
        self.store
            .dispatch(Msg::RunLargeFileCommand { repo_id, command });
        self.dismiss_inline_popover(window, cx);
    }

    pub(in crate::view::panels::popover) fn submit_submodule_change_pointer(
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

    pub(in crate::view::panels::popover) fn can_submit_stash(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.active_repo_id().is_some()
            && self
                .stash_message_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn submit_commit_prompt(
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
        let pending = PendingCommit::new(&self.state, repo_id, false, message.clone().into());
        self.store.dispatch(Msg::Commit {
            repo_id,
            message,
            push_after_commit: false,
        });
        self.pending_commit_prompts.insert(repo_id, pending);
        self.dismiss_inline_popover(window, cx);
    }

    pub(in crate::view::panels::popover) fn advance_pending_commit_prompts(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let state = Arc::clone(&self.state);
        let mut pending_commits = std::mem::take(&mut self.pending_commit_prompts);
        let before = pending_commits.len();
        pending_commits.retain(|repo_id, pending| {
            // Without git the store drops the commit and never logs a result.
            if !state.git_runtime.is_available()
                || !state.repos.iter().any(|repo| repo.id == *repo_id)
            {
                return false;
            }
            let result = pending.advance(&state);
            if result == Some(true) {
                if self
                    .commit_prompt_message_drafts
                    .get(repo_id)
                    .is_some_and(|draft| draft.trim() == pending.message.as_ref())
                {
                    self.commit_prompt_message_drafts.remove(repo_id);
                }
                if self.popover == Some(PopoverKind::CommitPrompt { repo_id: *repo_id })
                    && self.commit_prompt_message_input.read(cx).text().trim()
                        == pending.message.as_ref()
                {
                    self.commit_prompt_message_input
                        .update(cx, |input, cx| input.set_text(String::new(), cx));
                    self.commit_prompt_message_scroll
                        .set_offset(point(px(0.0), px(0.0)));
                }
            }
            !(result.is_some() && pending.settled())
        });
        // The host fingerprint does not cover the submit gate, so re-render
        // the Commit button once a commit settles.
        if pending_commits.len() != before {
            cx.notify();
        }
        self.pending_commit_prompts = pending_commits;
    }

    pub(in crate::view::panels::popover) fn save_commit_prompt_draft(
        &mut self,
        cx: &gpui::Context<Self>,
    ) {
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

    pub(in crate::view::panels::popover) fn can_submit_commit_prompt(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.active_repo().is_some_and(|repo| {
            repo.commit_in_flight == 0
                && !self.pending_commit_prompts.contains_key(&repo.id)
                && (repo
                    .staged_status_entries()
                    .is_some_and(|entries| !entries.is_empty())
                    || matches!(repo.merge_commit_message, Loadable::Ready(Some(_))))
        }) && self
            .commit_prompt_message_input
            .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn submit_stash(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
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

    /// The `git worktree add` the open dialog's fields describe, or why Add is off.
    pub(in crate::view::panels::popover) fn worktree_add_request(
        &self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> Result<
        (
            std::path::PathBuf,
            gitcomet_core::services::WorktreeCheckout,
        ),
        worktree_add_prompt::WorktreeAddBlocked,
    > {
        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            return Err(worktree_add_prompt::WorktreeAddBlocked::Incomplete);
        };
        let folder = self.worktree_path_input.read(cx).text().to_string();
        let new_branch = self.worktree_new_branch_input.read(cx).text().to_string();
        worktree_add_prompt::worktree_add_request(
            repo,
            &worktree_add_prompt::WorktreeAddFields {
                folder: &folder,
                source: &self.worktree_ref_source_target,
                create_branch: self.worktree_add_create_branch,
                new_branch: &new_branch,
            },
        )
    }

    pub(in crate::view::panels::popover) fn can_submit_worktree_add(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
        }) = self.popover
        else {
            return false;
        };
        matches!(
            self.worktree_add_request(repo_id, cx),
            Ok(_) | Err(worktree_add_prompt::WorktreeAddBlocked::CheckedOut(_))
        )
    }

    pub(in crate::view::panels::popover) fn submit_worktree_add(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.suppress_worktree_submit_after_ref_enter {
            return;
        }
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
        }) = self.popover
        else {
            return;
        };
        match self.worktree_add_request(repo_id, cx) {
            Ok((path, checkout)) => {
                self.store.dispatch(Msg::AddWorktree {
                    repo_id,
                    path,
                    checkout,
                    open_after: self.worktree_add_open_after,
                });
            }
            Err(worktree_add_prompt::WorktreeAddBlocked::CheckedOut(path)) => {
                let is_current = self
                    .state
                    .repos
                    .iter()
                    .any(|repo| repo.id == repo_id && repo.spec.workdir == path);
                // The worktree the dialog started in is open already.
                if !is_current {
                    let _ = self
                        .root_view
                        .request(RootRequest::OpenRepoPath(path), None, cx);
                }
            }
            Err(_) => return,
        }
        self.close_popover(cx);
    }

    pub(in crate::view::panels::popover) fn can_submit_submodule_add(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.submodule_url_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
            && self
                .submodule_path_input
                .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn submit_submodule_add(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
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
}
