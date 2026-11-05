use super::*;
use gitcomet_core::domain::{SubmoduleCheckoutPolicy, SubmoduleCheckoutRisk};
use gitcomet_state::model::{SubmoduleCheckoutOperation, SubmoduleCheckoutPrompt};

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
    let (title, body, keep_label, keep_note) = match prompt.risks.as_slice() {
        [risk] => {
            let (name, target) = (name(risk), short_commit(&risk.target));
            let (title, body, keep_label, keep_note) = match (changes, unreferenced) {
                (true, false) => (
                    format!("{name} has uncommitted changes"),
                    format!(
                        "Checking out {target} would overwrite them. Choose how to keep them safe."
                    ),
                    "Stash and check out",
                    format!("The changes are stashed in {name} first."),
                ),
                (false, true) => (
                    format!("{name} is on a commit no branch holds"),
                    format!(
                        "Checking out {target} would leave that commit reachable only from the reflog. Choose how to keep it safe."
                    ),
                    "Keep on a branch and check out",
                    format!("A new branch in {name} keeps the commit first."),
                ),
                _ => (
                    format!("{name} has work a checkout would lose"),
                    format!(
                        "Checking out {target} would overwrite uncommitted changes and leave a commit no branch holds. Choose how to keep them safe."
                    ),
                    "Keep the work and check out",
                    format!(
                        "Changes are stashed and the commit is kept on a new branch in {name}."
                    ),
                ),
            };
            (title, body, keep_label, keep_note)
        }
        risks => {
            let count = risks.len();
            match (changes, unreferenced) {
                (true, false) => (
                    format!("{count} submodules have uncommitted changes"),
                    "Checking out the restored commits would overwrite them. Choose how to keep them safe.".to_string(),
                    "Stash and check out",
                    "The changes are stashed in each submodule first.".to_string(),
                ),
                (false, true) => (
                    format!("{count} submodules are on commits no branch holds"),
                    "Checking out the restored commits would leave those commits reachable only from the reflog. Choose how to keep them safe.".to_string(),
                    "Keep on branches and check out",
                    "A new branch in each submodule keeps its commit first.".to_string(),
                ),
                _ => (
                    format!("{count} submodules have work a checkout would lose"),
                    "Checking out the restored commits would overwrite uncommitted changes and leave commits no branch holds. Choose how to keep them safe.".to_string(),
                    "Keep the work and check out",
                    "Changes are stashed and commits no branch holds are kept on new branches in each submodule.".to_string(),
                ),
            }
        }
    };
    let keep_label = keep_label.to_string();
    let (leave_label, leave_note) = match &prompt.operation {
        SubmoduleCheckoutOperation::Discard { .. } => (
            Some(format!("Discard in {parent} only")),
            Some(match prompt.risks.as_slice() {
                [risk] => format!(
                    "{} stays at {} and shows as pointer moved.",
                    name(risk),
                    risk.checked_out.as_ref().map_or("its commit", short_commit)
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
    let copy = copy(&prompt, &this.state.repo_name(prompt.repo_id));

    let mut dialog = ConfirmDialog::new(copy.title, DIALOG_420_WIDTH).text(theme, copy.body);
    for risk in &prompt.risks {
        let from = risk
            .checked_out
            .as_ref()
            .map_or("not checked out", short_commit);
        dialog = dialog.mono_value(
            theme,
            format!("{}  {from} → {}", name(risk), short_commit(&risk.target)),
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
    use gitcomet_core::domain::CommitId;
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
    fn several_commits_on_no_branch_are_worded_in_the_plural() {
        let mut other = risk(false, true);
        other.path = PathBuf::from("vendor/lib");
        let copy = copy(&prompt(vec![risk(false, true), other]), "app");
        assert_eq!(copy.title, "2 submodules are on commits no branch holds");
        assert_eq!(
            copy.body,
            "Checking out the restored commits would leave those commits reachable only from the reflog. Choose how to keep them safe."
        );
        assert_eq!(copy.keep_label, "Keep on branches and check out");
        assert_eq!(
            copy.keep_note,
            "A new branch in each submodule keeps its commit first."
        );
        assert_eq!(
            copy.leave_note.as_deref(),
            Some("The submodules stay where they are and show as pointer moved.")
        );

        let changed = copy_for_changes();
        assert_eq!(changed.title, "2 submodules have uncommitted changes");
        assert_eq!(
            changed.keep_note,
            "The changes are stashed in each submodule first."
        );
    }

    fn copy_for_changes() -> SubmoduleCheckoutCopy {
        let mut other = risk(true, false);
        other.path = PathBuf::from("vendor/lib");
        copy(&prompt(vec![risk(true, false), other]), "app")
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
