use super::*;
use gitcomet_state::model::{LinkedPushChoice, LinkedPushPrompt, submodule_display_name};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StepState {
    Done,
    Refused,
    HeldBack,
}

pub(super) struct LinkedPushStep {
    pub(super) state: StepState,
    pub(super) label: String,
    pub(super) detail: Option<String>,
}

/// The words the prompt uses, kept apart from the drawing so tests can read
/// them.
pub(super) struct LinkedPushCopy {
    pub(super) title: String,
    pub(super) body: String,
    pub(super) steps: Vec<LinkedPushStep>,
    pub(super) note: String,
    pub(super) rebase_label: String,
}

pub(super) fn copy(state: &AppState, prompt: &LinkedPushPrompt) -> LinkedPushCopy {
    let child = submodule_display_name(&prompt.path);
    let parent = state.repo_name(prompt.parent);
    let plan = state
        .linked_commit
        .as_ref()
        .filter(|plan| plan.parent == prompt.parent);
    let paths = plan
        .map(|plan| plan.paths())
        .filter(|paths| paths.contains(&prompt.path))
        .unwrap_or_else(|| vec![prompt.path.clone()]);
    let recorded = plan
        .and_then(|plan| plan.parent_commit.as_ref())
        .map(short_commit);
    let recorded_names = paths
        .iter()
        .map(|path| submodule_display_name(path))
        .collect::<Vec<_>>()
        .join(", ");
    let mut steps = paths
        .iter()
        .map(|path| LinkedPushStep {
            state: StepState::Done,
            label: format!("Commit {}", path.display()),
            detail: None,
        })
        .collect::<Vec<_>>();
    steps.push(LinkedPushStep {
        state: StepState::Done,
        label: format!("Commit {parent}"),
        detail: Some(match &recorded {
            Some(commit) => format!("{commit} · records {recorded_names}"),
            None => format!("records {recorded_names}"),
        }),
    });
    let refused = paths
        .iter()
        .position(|path| *path == prompt.path)
        .unwrap_or_default();
    for (ix, path) in paths.iter().enumerate() {
        let (state, detail) = match ix.cmp(&refused) {
            std::cmp::Ordering::Less => (StepState::Done, None),
            std::cmp::Ordering::Equal => (
                StepState::Refused,
                prompt
                    .error
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(str::to_string),
            ),
            std::cmp::Ordering::Greater => (StepState::HeldBack, None),
        };
        steps.push(LinkedPushStep {
            state,
            label: format!("Push {}", path.display()),
            detail,
        });
    }
    steps.push(LinkedPushStep {
        state: StepState::HeldBack,
        label: format!("Push {parent}"),
        detail: None,
    });
    LinkedPushCopy {
        title: format!("Push stopped at {child}"),
        body: format!(
            "Submodules first, parent last, so {parent} never records a commit others cannot fetch."
        ),
        steps,
        note: format!(
            "Nothing was pushed for {parent}. Rebasing {child} changes its commit id, so the commit in {parent} is updated to the new id before pushing again."
        ),
        rebase_label: format!("Rebase {child} and retry"),
    }
}

fn step_row(theme: AppTheme, icon_size: Pixels, ix: usize, step: LinkedPushStep) -> gpui::Div {
    let (icon, color, status) = match step.state {
        StepState::Done => (
            Some("icons/check.svg"),
            theme.colors.status.success.foreground,
            None,
        ),
        StepState::Refused => (
            Some("icons/warning.svg"),
            theme.colors.status.warning.foreground,
            Some("refused"),
        ),
        StepState::HeldBack => (None, theme.colors.foreground.secondary, Some("held back")),
    };
    div()
        .debug_selector(move || format!("linked_push_step_{ix}"))
        .flex()
        .items_start()
        .gap_2()
        .px_2()
        .py_1()
        .child(
            div()
                .flex_none()
                .size(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .when_some(icon, |slot, icon| {
                    slot.child(svg_icon(icon, color, icon_size))
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(theme.ui_text(14.0))
                        .text_color(theme.colors.foreground.primary)
                        .child(step.label),
                )
                .when_some(step.detail, |column, detail| {
                    column.child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                            .text_color(theme.colors.foreground.secondary)
                            .truncate()
                            .child(detail),
                    )
                }),
        )
        .when_some(status, |row, status| {
            row.child(
                div()
                    .flex_none()
                    .text_size(theme.ui_text(12.0))
                    .text_color(color)
                    .child(status),
            )
        })
}

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: LinkedPushPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let copy = copy(this.state.as_ref(), &prompt);
    let icon_size = popover_scaled_px_fn(cx)(14.0);
    let steps = div().flex().flex_col().children(
        copy.steps
            .into_iter()
            .enumerate()
            .map(|(ix, step)| step_row(theme, icon_size, ix, step)),
    );
    let dialog = ConfirmDialog::new(copy.title, DIALOG_440_WIDTH)
        .text(theme, copy.body)
        .section(steps)
        .note(theme, copy.note);

    let keep_prompt = prompt.clone();
    let merge_prompt = prompt.clone();
    let rebase_prompt = prompt;
    let keep = cancel_button_labeled(
        "linked_push_keep_local",
        "linked_push_keep_local_hint",
        "Keep local, push later",
        theme,
    )
    .on_click(theme, cx, move |this, _e, _w, cx| {
        this.store.dispatch(Msg::ResolveLinkedPushPrompt {
            prompt: keep_prompt.clone(),
            choice: LinkedPushChoice::KeepLocal,
        });
        this.close_resolved_popover(cx);
    });
    let actions = div()
        .flex()
        .gap_1()
        .child(
            components::Button::new("linked_push_merge", "Merge and retry")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::ResolveLinkedPushPrompt {
                        prompt: merge_prompt.clone(),
                        choice: LinkedPushChoice::Merge,
                    });
                    this.close_resolved_popover(cx);
                }),
        )
        .child(
            components::Button::new("linked_push_rebase", copy.rebase_label)
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::ResolveLinkedPushPrompt {
                        prompt: rebase_prompt.clone(),
                        choice: LinkedPushChoice::Rebase,
                    });
                    this.close_resolved_popover(cx);
                }),
        );
    dialog.render(theme, keep, actions, cx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{CommitId, RepoSpec};
    use gitcomet_state::model::{LinkedChild, LinkedCommit, LinkedCommitStage, RepoState};
    use std::path::PathBuf;

    fn state() -> AppState {
        let mut state = AppState::default();
        state.repos.push(RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/w/GitComet"),
            },
        ));
        state.repos.push(RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: PathBuf::from("/w/GitComet/fixtures/syntax_test"),
            },
        ));
        state.linked_commit = Some(LinkedCommit {
            parent: RepoId(1),
            child_tab: Some(RepoId(2)),
            children: vec![LinkedChild {
                path: PathBuf::from("fixtures/syntax_test"),
                message: "Add fixtures".into(),
            }],
            parent_message: "Update fixtures/syntax_test".into(),
            push: true,
            stage: LinkedCommitStage::PushStopped {
                path: PathBuf::from("fixtures/syntax_test"),
            },
            parent_commit: Some(CommitId("7d0a5b1ffff".into())),
        });
        state
    }

    #[test]
    fn a_refused_submodule_push_holds_the_parent_back() {
        let state = state();
        let prompt = LinkedPushPrompt {
            generation: 1,
            parent: RepoId(1),
            path: PathBuf::from("fixtures/syntax_test"),
            child_tab: Some(RepoId(2)),
            error: "\n ! [rejected] main -> main (fetch first)\n".into(),
        };
        let copy = copy(&state, &prompt);
        assert_eq!(copy.title, "Push stopped at syntax_test");
        assert_eq!(copy.rebase_label, "Rebase syntax_test and retry");
        let labels = copy
            .steps
            .iter()
            .map(|step| (step.state, step.label.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![
                (StepState::Done, "Commit fixtures/syntax_test"),
                (StepState::Done, "Commit GitComet"),
                (StepState::Refused, "Push fixtures/syntax_test"),
                (StepState::HeldBack, "Push GitComet"),
            ]
        );
        assert_eq!(
            copy.steps[1].detail.as_deref(),
            Some("7d0a5b1 · records syntax_test")
        );
        assert_eq!(
            copy.steps[2].detail.as_deref(),
            Some("! [rejected] main -> main (fetch first)")
        );
        assert!(copy.note.starts_with("Nothing was pushed for GitComet."));
    }

    #[test]
    fn a_refused_push_among_several_submodules_marks_the_ones_after_it_held_back() {
        let mut state = state();
        state.linked_commit = Some(LinkedCommit {
            parent: RepoId(1),
            child_tab: None,
            children: vec![
                LinkedChild {
                    path: PathBuf::from("fixtures/syntax_test"),
                    message: "a".into(),
                },
                LinkedChild {
                    path: PathBuf::from("vendor/ui-kit"),
                    message: "b".into(),
                },
                LinkedChild {
                    path: PathBuf::from("vendor/icons"),
                    message: "c".into(),
                },
            ],
            parent_message: "Update".into(),
            push: true,
            stage: LinkedCommitStage::PushStopped {
                path: PathBuf::from("vendor/ui-kit"),
            },
            parent_commit: None,
        });
        let prompt = LinkedPushPrompt {
            generation: 1,
            parent: RepoId(1),
            path: PathBuf::from("vendor/ui-kit"),
            child_tab: None,
            error: "rejected".into(),
        };
        let copy = copy(&state, &prompt);
        assert_eq!(copy.title, "Push stopped at ui-kit");
        let labels = copy
            .steps
            .iter()
            .map(|step| (step.state, step.label.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![
                (StepState::Done, "Commit fixtures/syntax_test"),
                (StepState::Done, "Commit vendor/ui-kit"),
                (StepState::Done, "Commit vendor/icons"),
                (StepState::Done, "Commit GitComet"),
                (StepState::Done, "Push fixtures/syntax_test"),
                (StepState::Refused, "Push vendor/ui-kit"),
                (StepState::HeldBack, "Push vendor/icons"),
                (StepState::HeldBack, "Push GitComet"),
            ]
        );
        assert_eq!(
            copy.steps[3].detail.as_deref(),
            Some("records syntax_test, ui-kit, icons")
        );
    }
}
