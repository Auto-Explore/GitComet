use super::*;
use gitcomet_core::domain::BareConversionHead;
use gitcomet_state::model::BareModeConversionState;

pub(in crate::view) const BARE_MODE_TOOLTIP: &str = "This repository stores Git history without an active working tree. You can browse history, fetch, and push. Checkout, staging, and committing working files are unavailable.";

impl PopoverHost {
    pub(super) fn sync_bare_mode_confirmation(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::DisableBareModeConfirm,
        }) = self.popover.as_ref()
        else {
            return;
        };
        if self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == *repo_id)
            .is_none_or(|repo| repo.has_working_tree() && !repo.bare_mode_conversion.is_busy())
        {
            self.close_resolved_popover(cx);
        }
    }
}

pub(super) fn menu(this: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    ContextMenuModel::new(vec![ContextMenuItem::Entry {
        label: "Disable bare mode".into(),
        icon: None,
        shortcut: None,
        disabled: repo.is_none_or(|repo| !repo.is_bare() || repo.bare_mode_conversion.is_busy()),
        action: Box::new(ContextMenuAction::PreviewDisableBareMode { repo_id }),
    }])
}

fn head_description(head: &BareConversionHead) -> String {
    match head {
        BareConversionHead::Branch {
            name,
            commit: Some(_),
        } => format!("Branch: {name}"),
        BareConversionHead::Branch { name, commit: None } => {
            format!("Unborn branch: {name} (empty checkout)")
        }
        BareConversionHead::Detached { commit } => format!("Detached HEAD: {commit}"),
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let scale = crate::ui_scale::UiScale::current(cx);
    let Some(repo) = this.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return div();
    };
    let conversion = repo.bare_mode_conversion.clone();
    let preview = match &conversion {
        BareModeConversionState::AwaitingConfirmation(preview) => Some(Arc::clone(preview)),
        _ => None,
    };
    let mut dialog = ConfirmDialog::new("Disable bare mode", DIALOG_540_WIDTH);
    let busy = conversion.is_busy();
    match &conversion {
        BareModeConversionState::Idle | BareModeConversionState::Previewing => {
            dialog = dialog.text(theme, "Checking the repository before making changes…");
        }
        BareModeConversionState::AwaitingConfirmation(preview) => {
            let root: SharedString = preview.root.display().to_string().into();
            let head: SharedString = head_description(&preview.head).into();
            dialog = dialog
                .section(div().px_2().min_w(px(0.0)).child(
                    div().id("bare_mode_root").truncate().child(root.clone()).gitcomet_tooltip(theme, root),
                ))
                .section(div().px_2().min_w(px(0.0)).child(
                    div().id("bare_mode_head").truncate().child(head.clone()).gitcomet_tooltip(theme, head),
                ))
                .text(theme, "Convert this folder into a normal checkout. Git data moves into its .git directory, and working files are created from the HEAD shown above.")
                .text(theme, "Stop other Git tools, server jobs, and the git-annex assistant using this repository or its linked worktrees before continuing. Keep them stopped until conversion finishes.")
                .note(theme, "Configured checkout filters may run while working files are created.");
        }
        BareModeConversionState::Converting => {
            dialog = dialog.text(theme, "Disabling bare mode. Keep other Git tools stopped while the repository is converted…");
        }
        BareModeConversionState::Reopening => {
            dialog = dialog.text(
                theme,
                "Verifying and reopening the repository and its linked worktrees…",
            );
        }
        BareModeConversionState::Failed(error) => {
            dialog = dialog.text(theme, error.to_string());
        }
    }
    dialog
        .render(
            theme,
            components::Button::new("bare_mode_cancel", if busy { "Close" } else { "Cancel" })
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, move |this, _, _, cx| {
                    this.close_popover(cx);
                }),
            components::Button::new(
                "bare_mode_confirm",
                if busy {
                    "Working…"
                } else {
                    "Disable bare mode"
                },
            )
            .style(components::ButtonStyle::Filled)
            .busy(busy)
            .disabled(preview.is_none())
            .on_click(theme, cx, move |this, _, _, _| {
                if let Some(preview) = preview.clone() {
                    this.store
                        .dispatch(Msg::DisableBareMode { repo_id, preview });
                }
            }),
            cx,
        )
        .w(scale.px(540.0))
        .max_w_full()
        .debug_selector(|| "bare_mode_confirmation".to_owned())
}
