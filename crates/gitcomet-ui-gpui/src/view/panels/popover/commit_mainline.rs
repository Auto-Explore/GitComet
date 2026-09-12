//! Mainline-parent picker shared by the cherry-pick and revert confirmations.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MainlineChoice {
    number: usize,
    short_id: String,
    summary: Option<String>,
    refs: Vec<String>,
}

pub(super) fn mainline_actions_disabled(
    parent_count: usize,
    selected_mainline: Option<usize>,
) -> bool {
    parent_count > 1 && selected_mainline.is_none()
}

/// Looks in the loaded history page, then in file history: both dialogs can be
/// opened from a file-history row whose commit is not on the history page.
fn find_commit<'a>(
    repo: &'a RepoState,
    commit_id: &CommitId,
) -> Option<&'a gitcomet_core::domain::Commit> {
    [repo.log.ready(), repo.history_state.file_history.ready()]
        .into_iter()
        .flatten()
        .find_map(|page| page.commits.iter().find(|commit| commit.id == *commit_id))
}

pub(super) fn commit_summary(this: &PopoverHost, repo_id: RepoId, commit_id: &CommitId) -> String {
    this.state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| find_commit(repo, commit_id))
        .map(|commit| commit.summary.to_string())
        .unwrap_or_default()
}

pub(super) fn mainline_choices(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
) -> Vec<MainlineChoice> {
    let Some(repo) = this.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    let Some(commit) = find_commit(repo, commit_id) else {
        return Vec::new();
    };

    commit
        .parent_ids
        .iter()
        .enumerate()
        .map(|(ix, parent_id)| {
            let summary = find_commit(repo, parent_id)
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

/// Parent rows; clicking one stores its number in `PopoverHost::commit_mainline`.
pub(super) fn mainline_section(
    theme: AppTheme,
    choices: Vec<MainlineChoice>,
    selected_mainline: Option<usize>,
    id_prefix: &'static str,
    description: &'static str,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    div()
        .px_2()
        .pb_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child("Mainline parent"),
        )
        .child(
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(description),
        )
        .children(choices.into_iter().map(|choice| {
            let number = choice.number;
            let is_selected = selected_mainline == Some(number);
            let outlined_border = crate::theme::with_alpha(
                theme.colors.foreground.secondary,
                if theme.is_dark { 0.38 } else { 0.28 },
            );
            let hover_overlay = crate::theme::with_alpha(
                theme.colors.foreground.primary,
                if theme.is_dark { 0.07 } else { 0.05 },
            );
            let top_line = div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(theme.ui_text(14.0))
                        .child(format!("Parent {number}")),
                )
                .child(
                    div()
                        .text_size(theme.ui_text(12.0))
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .child(choice.short_id),
                )
                .when(!choice.refs.is_empty(), |line| {
                    line.child(div().flex_1()).child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(choice.refs.join(", ")),
                    )
                });

            div()
                .id(SharedString::from(format!("{id_prefix}_{number}")))
                .w_full()
                .px_2()
                .py_1()
                .rounded_md()
                .text_color(theme.colors.foreground.primary)
                .border_1()
                .border_color(if is_selected {
                    theme.colors.accent.foreground
                } else {
                    outlined_border
                })
                .when(is_selected, |row| {
                    row.bg(crate::theme::with_alpha(
                        theme.colors.accent.foreground,
                        if theme.is_dark { 0.12 } else { 0.08 },
                    ))
                })
                .when(!is_selected, |row| {
                    row.hover(move |style| style.bg(hover_overlay))
                })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _e: &gpui::ClickEvent, _w, cx| {
                    this.commit_mainline = Some(number);
                    cx.notify();
                }))
                .child(top_line)
                .when_some(choice.summary, |row, summary| {
                    row.child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(summary),
                    )
                })
        }))
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
