//! Remote prompts: add a remote, edit its URL, and push with or set an
//! upstream.

use super::*;

pub(in crate::view::panels::popover) fn upstream_prompt_submission(
    kind: &PopoverKind,
    remote: String,
    remote_branch: String,
) -> Option<Msg> {
    let PopoverKind::PushSetUpstreamPrompt {
        repo_id,
        configure_only_for,
        ..
    } = kind
    else {
        return None;
    };
    Some(match configure_only_for {
        Some(local_branch) => Msg::SetUpstreamBranch {
            repo_id: *repo_id,
            branch: local_branch.clone(),
            upstream: Upstream {
                remote,
                branch: remote_branch,
            },
        },
        None => Msg::PushSetUpstream {
            repo_id: *repo_id,
            remote,
            branch: remote_branch,
        },
    })
}

impl PopoverHost {
    pub(in crate::view::panels::popover) fn can_submit_remote_add(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.remote_name_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
            && self
                .remote_url_input
                .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn submit_remote_add(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
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

    pub(in crate::view::panels::popover) fn can_submit_remote_edit_url(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.remote_url_edit_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn submit_remote_edit_url(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
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

    pub(in crate::view::panels::popover) fn can_submit_push_set_upstream(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let local_branch_is_current = match self.popover.as_ref() {
            Some(PopoverKind::PushSetUpstreamPrompt {
                repo_id,
                configure_only_for: Some(branch),
                ..
            }) => self
                .state
                .repos
                .iter()
                .find(|repo| repo.id == *repo_id)
                .is_some_and(|repo| {
                    matches!(&repo.head_branch, Loadable::Ready(head) if head == branch)
                        && repo.branches.ready().is_some_and(|branches| {
                            branches.iter().any(|candidate| candidate.name == *branch)
                        })
                }),
            Some(PopoverKind::PushSetUpstreamPrompt { .. }) => true,
            _ => false,
        };
        local_branch_is_current
            && self.selected_push_upstream_remote().is_some()
            && self
                .push_upstream_branch_input
                .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn selected_push_upstream_remote(&self) -> Option<String> {
        let PopoverKind::PushSetUpstreamPrompt {
            repo_id, remote, ..
        } = self.popover.as_ref()?
        else {
            return None;
        };
        let repo = self.state.repos.iter().find(|repo| repo.id == *repo_id)?;
        push_set_upstream_prompt::selected_remote(repo, remote)
    }

    pub(in crate::view::panels::popover) fn select_push_upstream_remote(
        &mut self,
        selected: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::PushSetUpstreamPrompt {
            repo_id, remote, ..
        }) = self.popover.as_mut()
        else {
            return;
        };
        let is_configured = self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == *repo_id)
            .map(push_set_upstream_prompt::remote_names)
            .is_some_and(|names| names.contains(&selected));
        if is_configured {
            *remote = selected;
        }
        self.sync_tag_push_previews(cx);
        self.push_upstream_remote_menu_open = false;
        self.push_upstream_remote_selected_index = None;
        cx.notify();
    }

    pub(in crate::view::panels::popover) fn submit_push_set_upstream(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(kind @ PopoverKind::PushSetUpstreamPrompt { .. }) = self.popover.clone() else {
            return;
        };
        if !self.can_submit_push_set_upstream(cx) {
            return;
        }
        let Some(remote) = self.selected_push_upstream_remote() else {
            return;
        };
        let branch = self
            .push_upstream_branch_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let message = if let Some(mode) = self.push_upstream_tag_mode {
            let PopoverKind::PushSetUpstreamPrompt { repo_id, .. } = &kind else {
                return;
            };
            let Some(repo) = self.state.repos.iter().find(|repo| repo.id == *repo_id) else {
                return;
            };
            let Some(mut request) = tag_push::request(repo, mode, &self.state.large_file_settings)
            else {
                return;
            };
            request.remote = remote;
            request.branch = branch;
            request.set_upstream = true;
            Msg::PushWithTags {
                repo_id: *repo_id,
                request,
            }
        } else {
            let Some(message) = upstream_prompt_submission(&kind, remote, branch) else {
                return;
            };
            message
        };
        self.store.dispatch(message);
        self.close_popover(cx);
    }
}
