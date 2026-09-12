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

pub(super) struct Mainline {
    pub(super) choices: Vec<MainlineChoice>,
    /// The commit object's own parents have not arrived yet.
    pub(super) pending: bool,
}

/// History rows are shaped by the walk (First-parent mode keeps one parent per
/// merge), so the parents come from the commit object: loaded details, or the
/// lookup these dialogs issue when they open. Rows are only the fallback.
fn parent_ids(repo: &RepoState, commit_id: &CommitId) -> (Vec<CommitId>, bool) {
    if let Loadable::Ready(details) = &repo.history_state.commit_details
        && details.id == *commit_id
    {
        return (details.parent_ids.clone(), false);
    }
    let rows = || {
        find_commit(repo, commit_id)
            .map(|commit| commit.parent_ids.iter().cloned().collect())
            .unwrap_or_default()
    };
    let lookup = &repo.history_state.commit_lookup;
    match &lookup.result {
        Loadable::Ready(commit) if lookup.reference.as_ref() == Some(commit_id) => {
            (commit.parent_ids.iter().cloned().collect(), false)
        }
        // The backend re-validates the mainline, so rows are safe to act on.
        Loadable::Error(_) if lookup.reference.as_ref() == Some(commit_id) => (rows(), false),
        _ => (rows(), true),
    }
}

pub(super) fn mainline(this: &PopoverHost, repo_id: RepoId, commit_id: &CommitId) -> Mainline {
    let Some(repo) = this.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return Mainline {
            choices: Vec::new(),
            pending: false,
        };
    };
    let (parent_ids, pending) = parent_ids(repo, commit_id);
    let choices = parent_ids
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
        .collect();
    Mainline { choices, pending }
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
    use gitcomet_core::domain::{Commit, LogPage, RepoSpec};
    use gitcomet_state::model::CommitLookup;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn commit(id: &CommitId, parents: &[&str]) -> Commit {
        Commit {
            id: id.clone(),
            parent_ids: parents.iter().map(|p| CommitId((*p).into())).collect(),
            summary: "Merge branch 'topic'".into(),
            author: "Author".into(),
            time: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn merge_parents_come_from_the_commit_object_not_walk_shaped_rows() {
        let merge = CommitId("cafebabe".into());
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        // First-parent history keeps only the parent the walk followed.
        repo.log = Loadable::Ready(Arc::new(LogPage {
            commits: vec![commit(&merge, &["aaaa"])],
            next_cursor: None,
        }));
        assert_eq!(
            parent_ids(&repo, &merge),
            (vec![CommitId("aaaa".into())], true)
        );

        repo.history_state.commit_lookup = CommitLookup {
            request: 1,
            reference: Some(merge.clone()),
            result: Loadable::Ready(commit(&merge, &["aaaa", "bbbb"])),
        };
        let (parents, pending) = parent_ids(&repo, &merge);
        assert_eq!((parents.len(), pending), (2, false));

        // A failed lookup falls back to the rows; the backend re-validates.
        repo.history_state.commit_lookup.result = Loadable::Error("unsupported".into());
        assert_eq!(
            parent_ids(&repo, &merge),
            (vec![CommitId("aaaa".into())], false)
        );
    }

    #[test]
    fn merge_actions_require_an_explicit_mainline() {
        assert!(mainline_actions_disabled(2, None));
        assert!(!mainline_actions_disabled(2, Some(1)));
        assert!(!mainline_actions_disabled(1, None));
        assert!(!mainline_actions_disabled(0, None));
    }
}
