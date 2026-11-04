use super::*;

/// The words the prompt uses, kept apart from the drawing so tests can read
/// them.
pub(super) struct SubmoduleCloseCopy {
    pub(super) title: String,
    pub(super) body: String,
    pub(super) submodules: Vec<String>,
}

pub(super) fn copy(state: &AppState, prompt: &SubmoduleClosePrompt) -> SubmoduleCloseCopy {
    let parent = prompt
        .repo_ids
        .first()
        .map(|repo_id| state.repo_name(*repo_id))
        .unwrap_or_else(|| "the repository".to_string());
    let submodules = prompt
        .submodules
        .iter()
        .map(|repo_id| state.repo_name(*repo_id))
        .collect::<Vec<_>>();
    let body = match submodules.as_slice() {
        [submodule] => format!(
            "{submodule} is open as a submodule of {parent}. Close it too, or keep it open on its own."
        ),
        _ => format!(
            "{} submodules of {parent} are open. Close them too, or keep them open on their own.",
            submodules.len()
        ),
    };
    SubmoduleCloseCopy {
        title: format!("Close {parent}?"),
        body,
        submodules,
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: SubmoduleClosePrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let copy = copy(this.state.as_ref(), &prompt);
    let names = copy
        .submodules
        .iter()
        .map(|name| SharedString::from(name.clone()))
        .collect::<Vec<_>>();
    let dialog = ConfirmDialog::new(copy.title, DIALOG_420_WIDTH)
        .text(theme, copy.body)
        .file_list(theme, &names);

    let keep_prompt = prompt.clone();
    let all_prompt = prompt;
    let actions = div()
        .flex()
        .gap_1()
        .child(
            components::Button::new("submodule_close_keep", "Keep submodules open")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    let request = RootRequest::CloseReposPastSubmodulePrompt {
                        repo_ids: keep_prompt.repo_ids.clone(),
                        activate_after: keep_prompt.activate_after,
                    };
                    let root_view = this.root_view.clone();
                    this.close_resolved_popover(cx);
                    let _ = root_view.request(request, Some(window), cx);
                }),
        )
        .child(
            components::Button::new("submodule_close_all", "Close all")
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    let mut repo_ids = all_prompt.repo_ids.clone();
                    repo_ids.extend(all_prompt.submodules.iter().copied());
                    let request = RootRequest::CloseReposPastSubmodulePrompt {
                        repo_ids,
                        activate_after: all_prompt.activate_after,
                    };
                    let root_view = this.root_view.clone();
                    this.close_resolved_popover(cx);
                    let _ = root_view.request(request, Some(window), cx);
                }),
        );
    dialog.render(
        theme,
        dialog_cancel_button(
            "submodule_close_cancel",
            "submodule_close_cancel_hint",
            theme,
            cx,
        ),
        actions,
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_state::model::RepoState;
    use std::path::PathBuf;

    fn state(paths: &[&str]) -> AppState {
        let mut state = AppState::default();
        for (ix, path) in paths.iter().enumerate() {
            state.repos.push(RepoState::new_opening(
                RepoId(ix as u64 + 1),
                RepoSpec {
                    workdir: PathBuf::from(path),
                },
            ));
        }
        state
    }

    #[test]
    fn one_open_submodule_is_named() {
        let state = state(&["/w/GitComet", "/w/GitComet/fixtures/syntax_test"]);
        let copy = copy(
            &state,
            &SubmoduleClosePrompt {
                repo_ids: vec![RepoId(1)],
                activate_after: None,
                submodules: vec![RepoId(2)],
            },
        );
        assert_eq!(copy.title, "Close GitComet?");
        assert_eq!(
            copy.body,
            "syntax_test is open as a submodule of GitComet. Close it too, or keep it open on its own."
        );
    }

    #[test]
    fn several_open_submodules_are_counted() {
        let state = state(&["/w/app", "/w/app/a", "/w/app/vendor/b"]);
        let copy = copy(
            &state,
            &SubmoduleClosePrompt {
                repo_ids: vec![RepoId(1)],
                activate_after: None,
                submodules: vec![RepoId(2), RepoId(3)],
            },
        );
        assert!(copy.body.starts_with("2 submodules of app are open."));
        assert_eq!(copy.submodules, vec!["a", "b"]);
    }
}
