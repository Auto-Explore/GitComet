use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
struct MainlineChoice {
    number: usize,
    short_id: String,
    summary: Option<String>,
    refs: Vec<String>,
}

fn mainline_actions_disabled(parent_count: usize, selected_mainline: Option<usize>) -> bool {
    parent_count > 1 && selected_mainline.is_none()
}

fn mainline_choices(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
) -> Vec<MainlineChoice> {
    let Some(repo) = this.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    let Loadable::Ready(page) = &repo.log else {
        return Vec::new();
    };
    let Some(commit) = page.commits.iter().find(|commit| commit.id == *commit_id) else {
        return Vec::new();
    };

    commit
        .parent_ids
        .iter()
        .enumerate()
        .map(|(ix, parent_id)| {
            let summary = page
                .commits
                .iter()
                .find(|candidate| candidate.id == *parent_id)
                .map(|candidate| candidate.summary.lines().next().unwrap_or("").trim())
                .filter(|summary| !summary.is_empty());
            let mut refs = Vec::new();
            if let Loadable::Ready(branches) = &repo.branches {
                refs.extend(
                    branches
                        .iter()
                        .filter(|branch| branch.target == *parent_id)
                        .map(|branch| branch.name.clone()),
                );
            }
            if let Loadable::Ready(branches) = &repo.remote_branches {
                refs.extend(
                    branches
                        .iter()
                        .filter(|branch| branch.target == *parent_id)
                        .map(|branch| format!("{}/{}", branch.remote, branch.name)),
                );
            }

            MainlineChoice {
                number: ix + 1,
                short_id: parent_id
                    .as_ref()
                    .get(..8)
                    .unwrap_or(parent_id.as_ref())
                    .to_string(),
                summary: summary.map(str::to_string),
                refs,
            }
        })
        .collect()
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    commit_id: CommitId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let mainline_choices = mainline_choices(this, repo_id, &commit_id);
    let is_merge = mainline_choices.len() > 1;
    let selected_mainline = is_merge.then_some(this.cherry_pick_mainline).flatten();
    let mainline_missing = mainline_actions_disabled(mainline_choices.len(), selected_mainline);
    let sha = commit_id.as_ref();
    let short = sha.get(0..7).unwrap_or(sha).to_string();
    let summary = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| match &repo.log {
            Loadable::Ready(page) => page
                .commits
                .iter()
                .find(|commit| commit.id == commit_id)
                .map(|commit| commit.summary.to_string()),
            _ => None,
        })
        .unwrap_or_default();

    let dispatch =
        move |this: &mut PopoverHost, commit_now: bool, cx: &mut gpui::Context<PopoverHost>| {
            this.store.dispatch(Msg::CherryPickCommit {
                repo_id,
                commit_id: commit_id.clone(),
                commit: commit_now,
                mainline: selected_mainline,
                summary: summary.clone(),
            });
            this.close_popover(cx);
        };

    let mainline_section = is_merge.then(|| {
        div()
            .px_2()
            .pb_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .child("Mainline parent"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .child("Choose the parent Git should treat as the merge's mainline."),
            )
            .children(mainline_choices.into_iter().map(|choice| {
                let number = choice.number;
                let is_selected = selected_mainline == Some(number);
                let outlined_border = crate::theme::with_alpha(
                    theme.colors.text_muted,
                    if theme.is_dark { 0.38 } else { 0.28 },
                );
                let hover_overlay = crate::theme::with_alpha(
                    theme.colors.text,
                    if theme.is_dark { 0.07 } else { 0.05 },
                );
                let top_line = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_sm().child(format!("Parent {number}")))
                    .child(
                        div()
                            .text_xs()
                            .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                            .child(choice.short_id),
                    )
                    .when(!choice.refs.is_empty(), |line| {
                        line.child(div().flex_1()).child(
                            div()
                                .text_xs()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(choice.refs.join(", ")),
                        )
                    });

                div()
                    .id(SharedString::from(format!("cherry_pick_mainline_{number}")))
                    .w_full()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_color(theme.colors.text)
                    .border_1()
                    .border_color(if is_selected {
                        theme.colors.accent
                    } else {
                        outlined_border
                    })
                    .when(is_selected, |row| {
                        row.bg(crate::theme::with_alpha(
                            theme.colors.accent,
                            if theme.is_dark { 0.12 } else { 0.08 },
                        ))
                    })
                    .when(!is_selected, |row| {
                        row.hover(move |style| style.bg(hover_overlay))
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _e: &gpui::ClickEvent, _w, cx| {
                        this.cherry_pick_mainline = Some(number);
                        cx.notify();
                    }))
                    .child(top_line)
                    .when_some(choice.summary, |row, summary| {
                        row.child(
                            div()
                                .text_xs()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(summary),
                        )
                    })
            }))
    });

    let mut dialog = ConfirmDialog::new("Commit cherry-picked commit?", DIALOG_380_WIDTH)
        .text(theme, format!("Apply {short} to the current branch?"))
        .note(theme, "Commit the cherry-picked change immediately?");
    if let Some(section) = mainline_section {
        dialog = dialog.section(section);
    }

    dialog.render(
        theme,
        dialog_cancel_button(
            "cherry_pick_commit_cancel",
            "cherry_pick_commit_cancel_hint",
            theme,
            cx,
        ),
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                components::Button::new("cherry_pick_commit_no", "No")
                    .style(components::ButtonStyle::Outlined)
                    .disabled(mainline_missing)
                    .on_click(theme, cx, {
                        let dispatch = dispatch.clone();
                        move |this, _e, _w, cx| dispatch(this, false, cx)
                    }),
            )
            .child(
                components::Button::new("cherry_pick_commit_yes", "Yes")
                    .style(components::ButtonStyle::Filled)
                    .disabled(mainline_missing)
                    .on_click(theme, cx, move |this, _e, _w, cx| dispatch(this, true, cx)),
            ),
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_actions_require_an_explicit_mainline() {
        assert!(mainline_actions_disabled(2, None));
        assert!(!mainline_actions_disabled(2, Some(1)));
        assert!(!mainline_actions_disabled(1, None));
        assert!(!mainline_actions_disabled(0, None));
    }
}
