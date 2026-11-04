use super::*;

use gitcomet_state::model::submodule_display_name as name;

/// The words the prompt uses: one submodule is named, several are counted.
pub(super) struct LeaveBehindCopy {
    pub(super) title: String,
    pub(super) body: String,
    pub(super) stash_message: String,
    pub(super) shelve_note: String,
    pub(super) commit_only_label: String,
    /// "Open <submodule>" when there is one to commit in.
    pub(super) open_label: Option<String>,
}

pub(super) fn copy(prompt: &SubmoduleLeaveBehindPrompt, parent: &str) -> LeaveBehindCopy {
    let (title, subject) = match prompt.submodules.as_slice() {
        [path] => (
            format!("{} has uncommitted changes", name(path)),
            name(path),
        ),
        paths => (
            format!("{} submodules have uncommitted changes", paths.len()),
            "each submodule".to_string(),
        ),
    };
    let stash_message = if prompt.subject.is_empty() {
        format!("Shelved while committing {parent}")
    } else {
        format!("Shelved while committing {parent}: {}", prompt.subject)
    };
    LeaveBehindCopy {
        title,
        body: format!(
            "Committing {parent} alone leaves them behind: the commit does not record them, and they are easy to forget."
        ),
        shelve_note: format!("Shelving stashes them in {subject} as \"{stash_message}\"."),
        stash_message,
        commit_only_label: format!("Commit {parent} only"),
        open_label: match prompt.submodules.as_slice() {
            [path] => Some(format!("Open {}", name(path))),
            _ => None,
        },
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: SubmoduleLeaveBehindPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let parent_workdir = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == prompt.repo_id)
        .map(|repo| repo.spec.workdir.clone());
    let copy = copy(&prompt, &this.state.repo_name(prompt.repo_id));
    let mut dialog = ConfirmDialog::new(copy.title, DIALOG_440_WIDTH).text(theme, copy.body);
    for path in &prompt.submodules {
        dialog = dialog.mono_value(theme, path.display().to_string());
    }
    dialog = dialog.note(theme, copy.shelve_note);

    let open_path = parent_workdir
        .zip(prompt.submodules.first())
        .map(|(workdir, path)| workdir.join(path));
    let shelve_paths = prompt.submodules.clone();
    let stash_message = copy.stash_message;
    let actions = div()
        .flex()
        .gap_1()
        .when_some(copy.open_label.zip(open_path), |actions, (label, path)| {
            actions.child(
                components::Button::new("submodule_leave_behind_open", label)
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        let root_view = this.root_view.clone();
                        this.close_resolved_popover(cx);
                        let _ = root_view.request(
                            RootRequest::OpenRepoPath(path.clone()),
                            Some(window),
                            cx,
                        );
                    }),
            )
        })
        .child(
            components::Button::new("submodule_leave_behind_commit", copy.commit_only_label)
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    let root_view = this.root_view.clone();
                    this.close_resolved_popover(cx);
                    let _ = root_view.request(
                        RootRequest::SubmitCommitPastSubmoduleGuard { shelve: None },
                        Some(window),
                        cx,
                    );
                }),
        )
        .child(
            components::Button::new("submodule_leave_behind_shelve", "Shelve and commit")
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    // The stash runs in the commit's own step, so a failed
                    // stash leaves nothing committed.
                    let shelve = gitcomet_state::model::SubmoduleShelve {
                        paths: shelve_paths.clone(),
                        message: stash_message.clone(),
                    };
                    let root_view = this.root_view.clone();
                    this.close_resolved_popover(cx);
                    let _ = root_view.request(
                        RootRequest::SubmitCommitPastSubmoduleGuard {
                            shelve: Some(shelve),
                        },
                        Some(window),
                        cx,
                    );
                }),
        );
    dialog.render(
        theme,
        dialog_cancel_button(
            "submodule_leave_behind_cancel",
            "submodule_leave_behind_cancel_hint",
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
    use std::path::PathBuf;

    #[test]
    fn one_submodule_left_behind_is_named_and_can_be_opened() {
        let copy = copy(
            &SubmoduleLeaveBehindPrompt {
                repo_id: RepoId(1),
                submodules: vec![PathBuf::from("fixtures/syntax_test")],
                subject: "test: use new fixtures".into(),
            },
            "GitComet",
        );
        assert_eq!(copy.title, "syntax_test has uncommitted changes");
        assert_eq!(copy.commit_only_label, "Commit GitComet only");
        assert_eq!(copy.open_label.as_deref(), Some("Open syntax_test"));
        assert_eq!(
            copy.stash_message,
            "Shelved while committing GitComet: test: use new fixtures"
        );
        assert_eq!(
            copy.shelve_note,
            "Shelving stashes them in syntax_test as \"Shelved while committing GitComet: test: use new fixtures\"."
        );
    }

    #[test]
    fn several_submodules_left_behind_are_counted() {
        let copy = copy(
            &SubmoduleLeaveBehindPrompt {
                repo_id: RepoId(1),
                submodules: vec![PathBuf::from("a"), PathBuf::from("vendor/b")],
                subject: String::new(),
            },
            "app",
        );
        assert_eq!(copy.title, "2 submodules have uncommitted changes");
        assert_eq!(copy.stash_message, "Shelved while committing app");
        assert!(copy.open_label.is_none());
    }
}
