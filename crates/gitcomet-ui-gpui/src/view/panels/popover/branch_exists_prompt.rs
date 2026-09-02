use super::*;

fn display_target(target: &str) -> SharedString {
    let is_full_object_id =
        matches!(target.len(), 40 | 64) && target.bytes().all(|byte| byte.is_ascii_hexdigit());
    if is_full_object_id {
        format!("{}…", &target[..7]).into()
    } else {
        target.to_owned().into()
    }
}

/// Previews the worktree redirect when the listed worktrees show `name`
/// checked out elsewhere; the backend decides for real.
fn worktree_note(
    this: &PopoverHost,
    repo_id: RepoId,
    name: &str,
    operation: &BranchExistsPromptOperation,
) -> Option<String> {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id)?;
    let path = crate::view::rows::listed_workspace_paths_by_branch(repo).remove(name)?;
    let mut note = format!(
        "'{name}' is checked out in the worktree at {}. Checkout existing opens that worktree; overwriting applies there and opens it.",
        path.display()
    );
    if let BranchExistsPromptOperation::RenameBranch { old_name } = operation
        && matches!(&repo.head_branch, Loadable::Ready(head) if head == old_name)
    {
        note.push_str(" This tab is left on a detached HEAD at the same commit.");
    }
    Some(note)
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    name: String,
    target: String,
    operation: BranchExistsPromptOperation,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let dialog_width = DIALOG_540_WIDTH.preferred_px(popover_ui_scale(cx));

    let (source_label, show_full_text_tooltip, overwrite_label, overwrite_note): (
        SharedString,
        bool,
        &'static str,
        String,
    ) = match &operation {
        BranchExistsPromptOperation::CreateBranch => {
            let display = display_target(&target);
            let abbreviated = display.as_ref() != target.as_str();
            (
                format!("Target: {display}").into(),
                !abbreviated,
                "Overwrite and checkout",
                "Overwriting moves the branch to the target commit, clears its upstream tracking, and checks it out.".to_string(),
            )
        }
        BranchExistsPromptOperation::CheckoutRemoteBranch { .. } => {
            let display = display_target(&target);
            let abbreviated = display.as_ref() != target.as_str();
            (
                format!("Target: {display}").into(),
                !abbreviated,
                "Overwrite and checkout",
                "Overwriting moves the local branch to the remote commit, configures it to track that remote branch, and checks it out.".to_string(),
            )
        }
        BranchExistsPromptOperation::RenameBranch { old_name } => (
            format!("Renaming: {old_name} → {name}").into(),
            true,
            "Overwrite",
            format!(
                "Overwriting deletes the existing '{name}' and renames '{old_name}' to '{name}'."
            ),
        ),
    };
    let worktree_note = worktree_note(this, repo_id, &name, &operation);

    let mut source_text = components::TruncatedText::new(source_label)
        .id("branch_exists_target_text")
        .text_sm()
        .text_color(theme.colors.foreground.secondary);
    if show_full_text_tooltip {
        source_text = source_text.full_text_tooltip(this.tooltip_host.clone());
    }

    let mut dialog = ConfirmDialog::new("Branch already exists", DIALOG_540_WIDTH)
        .text(
            theme,
            format!("A local branch named '{name}' already exists."),
        )
        .section(
            div()
                .debug_selector(|| "branch_exists_target".to_string())
                .px_2()
                .py_1()
                .min_w(px(0.0))
                .max_w(dialog_width)
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .child(source_text.render(cx)),
        )
        .note(theme, overwrite_note);
    if let Some(note) = worktree_note {
        dialog = dialog.section(
            div()
                .debug_selector(|| "branch_exists_worktree_note".to_string())
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
                .child(SharedString::from(note)),
        );
    }

    dialog.render(
        theme,
        cancel_button("branch_exists_cancel", "branch_exists_cancel_hint", theme).on_click(
            theme,
            cx,
            |this, _e, _w, cx| {
                this.resolve_open_branch_exists_prompt(BranchExistsChoice::Cancel);
                this.close_popover(cx);
            },
        ),
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                components::Button::new("branch_exists_checkout_existing", "Checkout existing")
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.resolve_open_branch_exists_prompt(
                            BranchExistsChoice::CheckoutExisting,
                        );
                        this.close_popover(cx);
                    })
                    .debug_selector(|| "branch_exists_checkout_existing".to_string()),
            )
            .child(
                components::Button::new("branch_exists_overwrite", overwrite_label)
                    .style(components::ButtonStyle::Danger)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.resolve_open_branch_exists_prompt(
                            BranchExistsChoice::OverwriteAndCheckout,
                        );
                        this.close_popover(cx);
                    })
                    .debug_selector(|| "branch_exists_overwrite".to_string()),
            ),
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::display_target;

    #[test]
    fn abbreviates_only_full_object_ids() {
        assert_eq!(
            display_target("0123456789abcdef0123456789abcdef01234567").as_ref(),
            "0123456…"
        );
        assert_eq!(
            display_target("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .as_ref(),
            "0123456…"
        );
    }

    #[test]
    fn preserves_revision_names_and_short_hex_values() {
        assert_eq!(
            display_target("origin/feature-one").as_ref(),
            "origin/feature-one"
        );
        assert_eq!(
            display_target("origin/feature-two").as_ref(),
            "origin/feature-two"
        );
        assert_eq!(display_target("deadbeef").as_ref(), "deadbeef");
    }
}
