//! The workspace edit prompt: one dialog for creating, restacking, re-parenting
//! and removing a virtual branch, and for re-pointing the workspace target.
//!
//! The dialog has two steps. Opened from a branch row it first offers that
//! branch's actions, because stacking, re-parenting, joining another stack and
//! removing are the same four questions about the same branch and a row has
//! no room for four buttons. Picking one narrows the dialog to a single text
//! field, which is where every other prompt in the app puts its input.

use super::*;

/// The line above the field, explaining what confirming will do.
///
/// Worth the space: restacking rewrites the branch's history and moving a
/// branch into a stack does too, and neither is obvious from the button label.
fn detail_line(
    kind: WorkspacePromptKind,
    branch: &str,
    path: Option<&std::path::Path>,
    value: &str,
) -> SharedString {
    match kind {
        WorkspacePromptKind::Create => format!(
            "The new branch starts from the workspace target, with no commits of its own yet."
        )
        .into(),
        WorkspacePromptKind::CreateStacked => {
            format!("The new branch is created on {branch} and stacks above it.").into()
        }
        WorkspacePromptKind::SetTarget => format!(
            "Every branch is rebased onto {value} and the workspace branch is rebuilt."
        )
        .into(),
        WorkspacePromptKind::SetParent => {
            if value.is_empty() {
                format!("{branch} becomes independent, sitting directly on the target.").into()
            } else {
                format!("{branch} is rebased onto {value}; its history is rewritten.").into()
            }
        }
        WorkspacePromptKind::MoveToStack => {
            format!("{branch} is rebased so it sits directly above {value}.").into()
        }
        // Neither of these has a field, so the detail line is the whole body.
        WorkspacePromptKind::Remove => format!(
            "{branch} stops contributing to the workspace branch. The Git branch itself is kept."
        )
        .into(),
        WorkspacePromptKind::BranchActions => format!("Actions for {branch}.").into(),
        WorkspacePromptKind::CommitBranch => format!(
            "Every file assigned to {branch} will be committed to it, and only those."
        )
        .into(),
        WorkspacePromptKind::CommitMessage => match path {
            Some(path) => format!(
                "{} will be committed on its own, to the branch it is assigned to.",
                path.display()
            )
            .into(),
            None => String::new().into(),
        },
        WorkspacePromptKind::AssignFile => match path {
            Some(path) if value.is_empty() => format!(
                "{} will not be committed to any branch. Clear the field to leave it as it is.",
                path.display()
            )
            .into(),
            Some(path) => format!(
                "{} will be committed to {value}.",
                path.display()
            )
            .into(),
            // The panel is only reachable from a file row, so a prompt with no
            // path is a wiring bug rather than something to explain.
            None => String::new().into(),
        },
    }
}

/// The actions offered for a branch, in the order they are listed.
fn branch_actions() -> [WorkspacePromptKind; 5] {
    [
        WorkspacePromptKind::CreateStacked,
        WorkspacePromptKind::SetParent,
        WorkspacePromptKind::MoveToStack,
        WorkspacePromptKind::CommitBranch,
        WorkspacePromptKind::Remove,
    ]
}

fn action_label(kind: WorkspacePromptKind) -> &'static str {
    match kind {
        WorkspacePromptKind::CreateStacked => "Stack a new branch on this",
        WorkspacePromptKind::SetParent => "Change its base branch",
        WorkspacePromptKind::MoveToStack => "Move it into another stack",
        WorkspacePromptKind::CommitBranch => "Commit its assigned files",
        WorkspacePromptKind::Remove => "Remove it from the workspace",
        _ => "",
    }
}

/// The dialog the file row opens, given where the file currently sits.
///
/// Separate from the panel because the answer is two-sided — the prompt is
/// about a file, and the field is seeded with the assignment it is replacing —
/// and both sides have to agree or the row would describe one thing and edit
/// another.
pub(in crate::view) fn assign_file_prompt(
    path: &std::path::Path,
    branch: Option<&str>,
) -> WorkspacePrompt {
    WorkspacePrompt {
        kind: WorkspacePromptKind::AssignFile,
        branch: String::new(),
        path: Some(path.to_path_buf()),
        value: branch.unwrap_or_default().to_string(),
    }
}

/// The dialog the file row opens for a commit, opened on the message the
/// quick commit used to synthesise.
///
/// Prefilling rather than starting empty keeps the one-press commit the row
/// had, while letting the same field be replaced with something the user
/// actually means.
pub(in crate::view) fn commit_file_prompt(
    path: &std::path::Path,
    message: &str,
) -> WorkspacePrompt {
    WorkspacePrompt {
        kind: WorkspacePromptKind::CommitMessage,
        branch: String::new(),
        path: Some(path.to_path_buf()),
        value: message.to_string(),
    }
}

/// The message the quick commit falls back on: the same one the reducer used to
/// synthesise, so nothing about the default changes.
pub(in crate::view) fn default_commit_message(path: &std::path::Path) -> String {
    format!("Update {}", path.display())
}

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: WorkspacePrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
    // The step is host state rather than part of the kind so that narrowing
    // the dialog does not change which popover is open: a kind swap would look
    // to the fingerprint like a different dialog replacing this one.
    let kind = this.workspace_prompt_kind.unwrap_or(prompt.kind);
    let detail = detail_line(kind, &prompt.branch, prompt.path.as_deref(), &prompt.value);

    let mut body = div().flex().flex_col().w(scaled_px(540.0));
    body = body
        .child(popover_title(theme, kind.title()))
        .child(super::popover_rule(theme))
        .child(super::popover_detail(theme, detail));

    if kind == WorkspacePromptKind::BranchActions {
        for action in branch_actions() {
            let action_id = format!("workspace_action_{}", action_label(action).replace(' ', "_"));
            body = body.child(
                components::Button::new(action_id, action_label(action))
                    .style(components::ButtonStyle::Subtle)
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        this.open_workspace_prompt_step(action, window, cx);
                    })
                    // Full width so the list reads as a menu rather than as five
                    // differently sized buttons stacked in a column. Sizing is
                    // a property of the rendered element, so it has to come
                    // after `on_click` — `Button` has no builder method for it.
                    .w_full(),
            );
        }
    } else if kind.asks_for_text() {
        body = body
            .child(input_label(theme, kind.field_label()))
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .w_full()
                    .min_w(px(0.0))
                    .child(this.create_branch_input.clone()),
            );
    }

    body.child(super::popover_rule(theme))
        .child(
            super::prompt_footer_row()
                .child(
                    cancel_button("workspace_prompt_cancel", "workspace_prompt_cancel_hint", theme)
                        .focus_handle(this.create_branch_from_ref_focus.cancel.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("workspace_prompt_go", kind.confirm_label())
                        .focus_handle(this.create_branch_from_ref_focus.submit.clone())
                        .separated_end_slot(hotkey_hint(
                            theme,
                            "workspace_prompt_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!this.can_submit_workspace_prompt(cx))
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.submit_workspace_prompt(window, cx);
                        }),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_action_list_covers_every_branch_only_edit() {
        let actions = branch_actions();
        for kind in [
            WorkspacePromptKind::CreateStacked,
            WorkspacePromptKind::SetParent,
            WorkspacePromptKind::MoveToStack,
            WorkspacePromptKind::Remove,
        ] {
            assert!(actions.contains(&kind), "{kind:?} is not offered");
        }
        // Creating a branch has no subject, so it is not on a branch's list.
        assert!(!actions.contains(&WorkspacePromptKind::Create));
        assert!(!actions.contains(&WorkspacePromptKind::SetTarget));
    }

    #[test]
    fn every_listed_action_has_a_label() {
        for kind in branch_actions() {
            assert!(!action_label(kind).is_empty(), "{kind:?} has no label");
        }
    }

    #[test]
    fn committing_a_branch_is_offered_and_creating_one_is_not() {
        let actions = branch_actions();
        assert!(actions.contains(&WorkspacePromptKind::CommitBranch));
        // A whole-branch commit needs no subject of its own, so it belongs on
        // the branch's list; creating a branch does.
        assert!(!actions.contains(&WorkspacePromptKind::CommitMessage));
        assert!(!actions.contains(&WorkspacePromptKind::Create));
    }

    #[test]
    fn the_branch_commit_line_says_only_assigned_files_go() {
        let line = detail_line(
            WorkspacePromptKind::CommitBranch,
            "feature/api",
            None,
            "Add the client",
        );
        assert!(line.contains("feature/api"), "{line}");
        assert!(line.contains("only those"), "{line}");
    }

    #[test]
    fn only_the_text_edits_ask_for_a_field() {
        assert!(!WorkspacePromptKind::BranchActions.asks_for_text());
        assert!(!WorkspacePromptKind::Remove.asks_for_text());
        assert!(WorkspacePromptKind::Create.asks_for_text());
        assert!(WorkspacePromptKind::CreateStacked.asks_for_text());
        assert!(WorkspacePromptKind::SetTarget.asks_for_text());
        assert!(WorkspacePromptKind::SetParent.asks_for_text());
        assert!(WorkspacePromptKind::MoveToStack.asks_for_text());
        assert!(WorkspacePromptKind::AssignFile.asks_for_text());
    }#[test]
    fn the_commit_prompt_opens_on_a_message_the_user_can_keep() {
        let path = std::path::Path::new("src/lib.rs");
        let prompt = commit_file_prompt(path, &default_commit_message(path));
        assert_eq!(prompt.kind, WorkspacePromptKind::CommitMessage);
        assert_eq!(prompt.path.as_deref(), Some(path));
        assert_eq!(prompt.value, "Update src/lib.rs");
        assert!(
            !prompt.value.trim().is_empty(),
            "an empty default would make the confirm button dead on open"
        );
    }

#[test]
    fn the_commit_line_says_the_commit_is_just_this_file() {
        let line = detail_line(
            WorkspacePromptKind::CommitMessage,
            "",
            Some(std::path::Path::new("src/lib.rs")),
            "Update src/lib.rs",
        );
        assert!(line.contains("on its own"), "{line}");
    }

#[test]
    fn the_assign_prompt_carries_the_file_and_its_current_branch() {
        let prompt = assign_file_prompt(std::path::Path::new("src/lib.rs"), Some("feature/api"));
        assert_eq!(prompt.kind, WorkspacePromptKind::AssignFile);
        assert_eq!(prompt.path.as_deref(), Some(std::path::Path::new("src/lib.rs")));
        assert_eq!(prompt.value, "feature/api");
        assert!(prompt.branch.is_empty(), "the branch is typed, not captured");

        // An unassigned file opens on an empty field, which is also how it is
        // put back.
        let prompt = assign_file_prompt(std::path::Path::new("src/lib.rs"), None);
        assert!(prompt.value.is_empty());
    }

    #[test]
    fn the_assign_line_names_the_file_and_says_where_it_goes() {
        let line = detail_line(
            WorkspacePromptKind::AssignFile,
            "",
            Some(std::path::Path::new("src/lib.rs")),
            "feature/api",
        );
        assert!(line.contains("src/lib.rs"), "{line}");
        assert!(line.contains("feature/api"), "{line}");
    }

    #[test]
    fn an_empty_assign_line_says_the_file_stays_where_it_is() {
        let line = detail_line(
            WorkspacePromptKind::AssignFile,
            "",
            Some(std::path::Path::new("src/lib.rs")),
            "",
        );
        assert!(line.contains("not be committed to any branch"), "{line}");
    }

    #[test]
    fn every_kind_has_a_distinct_title() {
        let kinds = [
            WorkspacePromptKind::BranchActions,
            WorkspacePromptKind::Create,
            WorkspacePromptKind::CreateStacked,
            WorkspacePromptKind::SetTarget,
            WorkspacePromptKind::SetParent,
            WorkspacePromptKind::MoveToStack,
            WorkspacePromptKind::Remove,
            WorkspacePromptKind::AssignFile,
            WorkspacePromptKind::CommitMessage,
            WorkspacePromptKind::CommitBranch,
        ];
        for (i, kind) in kinds.iter().enumerate() {
            for other in &kinds[i + 1..] {
                // The title is what tells two open dialogs apart, so it has to
                // separate every pair. The confirm label need not: committing
                // one file and committing a whole branch are both "Commit", and
                // calling them anything else would describe the plumbing.
                assert_ne!(kind.title(), other.title(), "{kind:?} vs {other:?}");
                assert!(!kind.confirm_label().is_empty());
            }
        }
    }

    #[test]
    fn the_detail_line_names_the_branch_and_says_history_is_rewritten() {
        let line = detail_line(
            WorkspacePromptKind::SetParent,
            "feature/api",
            None,
            "main",
        );
        assert!(line.contains("feature/api"), "{line}");
        assert!(line.contains("rebased"), "{line}");
    }

    #[test]
    fn an_empty_base_makes_the_branch_independent() {
        let line = detail_line(WorkspacePromptKind::SetParent, "feature/api", None, "");
        assert!(line.contains("independent"), "{line}");
    }

    #[test]
    fn removing_says_the_git_branch_is_kept() {
        let line = detail_line(WorkspacePromptKind::Remove, "feature/api", None, "");
        assert!(line.contains("kept"), "{line}");
    }
}