use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RepoTabDirection {
    Previous,
    Next,
}

fn adjacent_repo_tab_id(
    repo_ids: &[RepoId],
    active_repo: Option<RepoId>,
    direction: RepoTabDirection,
) -> Option<RepoId> {
    if repo_ids.is_empty() {
        return None;
    }

    let Some(active_ix) = active_repo.and_then(|repo_id| {
        repo_ids
            .iter()
            .position(|candidate_repo_id| *candidate_repo_id == repo_id)
    }) else {
        return repo_ids.first().copied();
    };

    if repo_ids.len() == 1 {
        return None;
    }

    let next_ix = match direction {
        RepoTabDirection::Previous => {
            if active_ix == 0 {
                repo_ids.len() - 1
            } else {
                active_ix - 1
            }
        }
        RepoTabDirection::Next => (active_ix + 1) % repo_ids.len(),
    };
    repo_ids.get(next_ix).copied()
}

impl GitCometView {
    pub(crate) fn open_recent_repository_picker(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let window_bounds = window.window_bounds().get_bounds();
        let preferred_width = px(480.0);
        let margin = px(24.0);
        let anchor_x = ((window_bounds.size.width - preferred_width) * 0.5).max(margin);
        let anchor_y = px(72.0);
        self.open_popover_at(
            PopoverKind::RecentRepositoryPicker,
            point(anchor_x, anchor_y),
            window,
            cx,
        );
    }

    pub(crate) fn show_open_repo_panel_fallback(
        &mut self,
        window: Option<&mut Window>,
        show_notice: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_repo_panel = true;
        self.open_repo_input
            .update(cx, |input, cx| input.set_text("", cx));
        if let Some(window) = window {
            let focus = self
                .open_repo_input
                .read_with(cx, |input, _| input.focus_handle());
            window.focus(&focus, cx);
        }
        if show_notice {
            self.push_toast(
                components::ToastKind::Warning,
                "Native folder picker unavailable. Enter a repository path manually.".to_string(),
                cx,
            );
        }
        cx.notify();
    }

    pub(crate) fn activate_repo_path(
        &mut self,
        path: &std::path::Path,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo_id) = self.repo_id_for_path(path) else {
            return false;
        };
        if self.state.active_repo == Some(repo_id) {
            return false;
        }

        self.store.dispatch(Msg::SetActiveRepo { repo_id });
        cx.notify();
        true
    }

    pub(crate) fn close_active_repo_tab(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };

        self.store.dispatch(Msg::CloseRepo { repo_id });
        cx.notify();
        true
    }

    pub(crate) fn activate_previous_repo_tab(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        self.activate_repo_tab_in_direction(RepoTabDirection::Previous, cx)
    }

    pub(crate) fn activate_next_repo_tab(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        self.activate_repo_tab_in_direction(RepoTabDirection::Next, cx)
    }

    fn repo_id_for_path(&self, path: &std::path::Path) -> Option<RepoId> {
        self.state
            .repos
            .iter()
            .find(|repo| repo.spec.workdir == path)
            .map(|repo| repo.id)
    }

    fn activate_repo_tab_in_direction(
        &mut self,
        direction: RepoTabDirection,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let repo_ids: Vec<RepoId> = self.state.repos.iter().map(|repo| repo.id).collect();
        let Some(next_repo_id) = adjacent_repo_tab_id(&repo_ids, self.state.active_repo, direction)
        else {
            return false;
        };

        if self.state.active_repo == Some(next_repo_id) {
            return false;
        }

        self.store.dispatch(Msg::SetActiveRepo {
            repo_id: next_repo_id,
        });
        cx.notify();
        true
    }

    pub(crate) fn open_repo_path(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        self.store.dispatch(Msg::OpenRepo(path));
        self.open_repo_panel = false;
        cx.notify();
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn apply_patch_from_file(
        &mut self,
        patch: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo_id) = self.state.active_repo else {
            return;
        };
        self.store.dispatch(Msg::ApplyPatch { repo_id, patch });
        cx.notify();
    }

    pub(crate) fn prompt_open_repo(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let view = cx.weak_entity();

        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Git Repository".into()),
        });

        window
            .spawn(cx, async move |cx| {
                let result = rx.await;
                let paths = match result {
                    Ok(Ok(Some(paths))) => paths,
                    Ok(Ok(None)) => return,
                    Ok(Err(_)) | Err(_) => {
                        let _ = view.update(cx, |this, cx| {
                            this.show_open_repo_panel_fallback(None, false, cx);
                        });
                        return;
                    }
                };

                let Some(path) = paths.into_iter().next() else {
                    return;
                };

                // Let the backend decide whether the path is a repository.
                // Frontend checks are brittle across bare repos/worktrees/submodules.
                let _ = view.update(cx, |this, cx| this.open_repo_path(path, cx));
            })
            .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_repo_tab_id_wraps_left_from_first_repo() {
        let repo_ids = [RepoId(1), RepoId(2), RepoId(3)];

        let target = adjacent_repo_tab_id(&repo_ids, Some(RepoId(1)), RepoTabDirection::Previous);

        assert_eq!(target, Some(RepoId(3)));
    }

    #[test]
    fn adjacent_repo_tab_id_wraps_right_from_last_repo() {
        let repo_ids = [RepoId(1), RepoId(2), RepoId(3)];

        let target = adjacent_repo_tab_id(&repo_ids, Some(RepoId(3)), RepoTabDirection::Next);

        assert_eq!(target, Some(RepoId(1)));
    }

    #[test]
    fn adjacent_repo_tab_id_defaults_to_first_when_no_repo_is_active() {
        let repo_ids = [RepoId(4), RepoId(5)];

        let target = adjacent_repo_tab_id(&repo_ids, None, RepoTabDirection::Next);

        assert_eq!(target, Some(RepoId(4)));
    }

    #[test]
    fn adjacent_repo_tab_id_noops_for_single_active_repo() {
        let repo_ids = [RepoId(9)];

        let target = adjacent_repo_tab_id(&repo_ids, Some(RepoId(9)), RepoTabDirection::Next);

        assert_eq!(target, None);
    }
}
