use super::*;
use gitcomet_core::domain::{CommitId, SubmoduleCheckoutPolicy, SubmoduleCheckoutRisk};
use gitcomet_state::model::{SubmoduleCheckoutOperation, SubmoduleCheckoutPrompt};

fn short(commit: &CommitId) -> String {
    gitcomet_core::domain::short_commit_id(commit.as_ref()).to_string()
}

fn name(risk: &SubmoduleCheckoutRisk) -> String {
    gitcomet_state::model::submodule_display_name(&risk.path)
}

/// The words the prompt uses: one submodule is named, several are counted.
pub(super) struct SubmoduleCheckoutCopy {
    pub(super) title: String,
    pub(super) body: String,
    pub(super) keep_label: String,
    pub(super) keep_note: String,
    pub(super) leave_label: Option<String>,
    pub(super) leave_note: Option<String>,
}

pub(super) fn copy(prompt: &SubmoduleCheckoutPrompt, parent: &str) -> SubmoduleCheckoutCopy {
    let changes = prompt.risks.iter().any(|risk| risk.has_inner_changes);
    let unreferenced = prompt.risks.iter().any(|risk| risk.unreferenced);
    let (title, target, subject) = match prompt.risks.as_slice() {
        [risk] => {
            let title = match (risk.has_inner_changes, risk.unreferenced) {
                (true, false) => format!("{} has uncommitted changes", name(risk)),
                (false, true) => format!("{} is on a commit no branch holds", name(risk)),
                _ => format!("{} has work a checkout would lose", name(risk)),
            };
            (title, short(&risk.target), name(risk))
        }
        risks => (
            format!("{} submodules have work a checkout would lose", risks.len()),
            "the restored commits".to_string(),
            "the submodules".to_string(),
        ),
    };
    let body = match (changes, unreferenced) {
        (true, false) => {
            format!("Checking out {target} would overwrite them. Choose how to keep them safe.")
        }
        (false, true) => format!(
            "Checking out {target} would leave that commit reachable only from the reflog. Choose how to keep it safe."
        ),
        _ => format!(
            "Checking out {target} would overwrite uncommitted changes and leave a commit no branch holds. Choose how to keep them safe."
        ),
    };
    let keep_label = match (changes, unreferenced) {
        (true, false) => "Stash and check out",
        (false, true) => "Keep on a branch and check out",
        _ => "Keep the work and check out",
    }
    .to_string();
    let keep_note = match (changes, unreferenced) {
        (true, false) => format!("The changes are stashed in {subject} first."),
        (false, true) => format!("A new branch in {subject} keeps the commit first."),
        _ => format!("Changes are stashed and the commit is kept on a new branch in {subject}."),
    };
    let (leave_label, leave_note) = match &prompt.operation {
        SubmoduleCheckoutOperation::Discard { .. } => (
            Some(format!("Discard in {parent} only")),
            Some(match prompt.risks.as_slice() {
                [risk] => format!(
                    "{} stays at {} and shows as pointer moved.",
                    name(risk),
                    risk.checked_out
                        .as_ref()
                        .map(short)
                        .unwrap_or_else(|| "its commit".to_string())
                ),
                _ => "The submodules stay where they are and show as pointer moved.".to_string(),
            }),
        ),
        SubmoduleCheckoutOperation::UpdateToRecorded { .. } => (None, None),
    };
    SubmoduleCheckoutCopy {
        title,
        body,
        keep_label,
        keep_note,
        leave_label,
        leave_note,
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: SubmoduleCheckoutPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let parent = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == prompt.repo_id)
        .and_then(|repo| repo.spec.workdir.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the parent".to_string());
    let copy = copy(&prompt, &parent);

    let mut dialog = ConfirmDialog::new(copy.title, DIALOG_420_WIDTH).text(theme, copy.body);
    for risk in &prompt.risks {
        let from = risk
            .checked_out
            .as_ref()
            .map(short)
            .unwrap_or_else(|| "not checked out".to_string());
        dialog = dialog.mono_value(
            theme,
            format!("{}  {from} → {}", name(risk), short(&risk.target)),
        );
    }
    dialog = dialog.note(theme, copy.keep_note);
    if let Some(note) = copy.leave_note.clone() {
        dialog = dialog.note(theme, note);
    }

    let keep_prompt = prompt.clone();
    let leave_prompt = prompt;
    let actions = div()
        .flex()
        .gap_1()
        .when_some(copy.leave_label, |actions, label| {
            actions.child(
                components::Button::new("submodule_checkout_leave", label)
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        this.store.dispatch(Msg::ResolveSubmoduleCheckoutPrompt {
                            prompt: leave_prompt.clone(),
                            policy: SubmoduleCheckoutPolicy::KeepCheckout,
                        });
                        this.close_resolved_popover(cx);
                    }),
            )
        })
        .child(
            components::Button::new("submodule_checkout_keep", copy.keep_label)
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::ResolveSubmoduleCheckoutPrompt {
                        prompt: keep_prompt.clone(),
                        policy: SubmoduleCheckoutPolicy::StashAndUpdate,
                    });
                    this.close_resolved_popover(cx);
                }),
        );

    dialog.render(
        theme,
        dialog_cancel_button(
            "submodule_checkout_cancel",
            "submodule_checkout_cancel_hint",
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
    use gitcomet_state::model::RepoId;
    use std::path::PathBuf;

    fn risk(changes: bool, unreferenced: bool) -> SubmoduleCheckoutRisk {
        SubmoduleCheckoutRisk {
            path: PathBuf::from("fixtures/syntax_test"),
            checked_out: Some(CommitId("c41f9e2aaaa".into())),
            target: CommitId("a1b2c3dbbbb".into()),
            has_inner_changes: changes,
            unreferenced,
        }
    }

    fn prompt(risks: Vec<SubmoduleCheckoutRisk>) -> SubmoduleCheckoutPrompt {
        SubmoduleCheckoutPrompt {
            generation: 1,
            repo_id: RepoId(1),
            operation: SubmoduleCheckoutOperation::Discard {
                paths: vec![PathBuf::from("fixtures/syntax_test")],
            },
            risks,
        }
    }

    #[test]
    fn uncommitted_changes_offer_a_stash_or_a_parent_only_discard() {
        let copy = copy(&prompt(vec![risk(true, false)]), "GitComet");
        assert_eq!(copy.title, "syntax_test has uncommitted changes");
        assert_eq!(
            copy.body,
            "Checking out a1b2c3d would overwrite them. Choose how to keep them safe."
        );
        assert_eq!(copy.keep_label, "Stash and check out");
        assert_eq!(
            copy.leave_label.as_deref(),
            Some("Discard in GitComet only")
        );
        assert_eq!(
            copy.leave_note.as_deref(),
            Some("syntax_test stays at c41f9e2 and shows as pointer moved.")
        );
    }

    #[test]
    fn a_commit_on_no_branch_offers_to_keep_it_on_one() {
        let copy = copy(&prompt(vec![risk(false, true)]), "app");
        assert_eq!(copy.title, "syntax_test is on a commit no branch holds");
        assert_eq!(copy.keep_label, "Keep on a branch and check out");
    }

    #[test]
    fn an_update_has_no_parent_only_choice() {
        let mut update = prompt(vec![risk(true, true)]);
        update.operation = SubmoduleCheckoutOperation::UpdateToRecorded {
            path: PathBuf::from("fixtures/syntax_test"),
        };
        let copy = copy(&update, "app");
        assert_eq!(copy.title, "syntax_test has work a checkout would lose");
        assert!(copy.leave_label.is_none());
    }
}
