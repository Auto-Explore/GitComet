use super::*;

pub(super) fn model(host: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let repo = host.state.repos.iter().find(|repo| repo.id == repo_id);
    let current_scope = repo
        .map(|repo| repo.history_state.history_scope)
        .unwrap_or_default();
    let focused_refs = repo
        .filter(|repo| !repo.history_state.history_focus.is_empty())
        .map(|repo| crate::view::history_focus::history_focus_count(repo).focused);
    model_for_scope(repo_id, current_scope, focused_refs)
}

fn model_for_scope(
    repo_id: RepoId,
    current_scope: gitcomet_core::domain::LogScope,
    focused_refs: Option<usize>,
) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header("History mode".into()),
        ContextMenuItem::Separator,
    ];
    items.extend(
        crate::view::history_mode::history_mode_ui_specs()
            .iter()
            .map(|spec| ContextMenuItem::Entry {
                label: spec.label.into(),
                icon: (spec.mode == current_scope).then_some("icons/check.svg".into()),
                shortcut: Some(spec.shortcut.into()),
                disabled: false,
                action: Box::new(ContextMenuAction::SetHistoryScope {
                    repo_id,
                    scope: spec.mode,
                }),
            }),
    );
    // Focus is set from a ref's own menu in the sidebar, but it has to be
    // clearable from the header too: the ref it was set on can be scrolled out
    // of view, collapsed, or deleted outright. The section counts the refs
    // rather than naming them; a long list of names is noise in this menu.
    if let Some(focused_refs) = focused_refs {
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::Header(
            format!(
                "Focus: {}",
                crate::view::history_focus::refs_label(focused_refs)
            )
            .into(),
        ));
        items.push(ContextMenuItem::Entry {
            label: "Stop focus".into(),
            icon: Some("icons/eye_off.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetHistoryFocus {
                repo_id,
                focus: gitcomet_core::domain::HistoryFocusSet::default(),
            }),
        });
    }
    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_marks_current_history_mode() {
        let model = super::model_for_scope(
            RepoId(11),
            gitcomet_core::domain::LogScope::MergesOnly,
            None,
        );

        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, icon, .. }
                    if label.as_ref() == "Merges only"
                        && icon
                            .as_ref()
                            .is_some_and(|icon| icon.as_ref() == "icons/check.svg")
            )
        }));
    }

    /// The focus section counts the refs instead of listing them, and offers
    /// the one way out.
    #[test]
    fn model_counts_focused_refs_without_naming_them() {
        let model = super::model_for_scope(
            RepoId(11),
            gitcomet_core::domain::LogScope::FullReachable,
            Some(8),
        );

        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Header(text) if text.as_ref() == "Focus: 8 refs"
        )));
        assert!(
            !model
                .items
                .iter()
                .any(|item| matches!(item, ContextMenuItem::Label(_))),
            "the focused refs must not be listed by name"
        );
        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Stop focus"
                    && matches!(
                        **action,
                        ContextMenuAction::SetHistoryFocus { ref focus, .. } if focus.is_empty()
                    )
        )));
    }

    #[test]
    fn model_has_no_focus_section_while_nothing_is_focused() {
        let model = super::model_for_scope(
            RepoId(11),
            gitcomet_core::domain::LogScope::FullReachable,
            None,
        );

        assert!(!model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { label, .. } if label.as_ref() == "Stop focus"
        )));
    }
}
