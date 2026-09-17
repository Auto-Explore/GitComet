use super::*;

pub(super) fn model(host: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let repo = host.state.repos.iter().find(|repo| repo.id == repo_id);
    let current_scope = repo
        .map(|repo| repo.history_state.history_scope)
        .unwrap_or_default();
    let solo = repo
        .map(|repo| repo.history_state.history_solo.clone())
        .unwrap_or_default();
    model_for_scope(repo_id, current_scope, solo)
}

fn model_for_scope(
    repo_id: RepoId,
    current_scope: gitcomet_core::domain::LogScope,
    solo: gitcomet_core::domain::HistorySoloSet,
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
    // Solo is set from a ref's own menu in the sidebar, but it has to be
    // clearable from the header too: the ref it was set on can be scrolled out
    // of view, collapsed, or deleted outright.
    if !solo.is_empty() {
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::Header("Solo".into()));
        for target in solo.iter() {
            items.push(ContextMenuItem::Label(target.label().into()));
        }
        items.push(ContextMenuItem::Entry {
            label: "Stop soloing".into(),
            icon: Some("icons/generic_close.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetHistorySolo {
                repo_id,
                solo: gitcomet_core::domain::HistorySoloSet::default(),
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
            Default::default(),
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
}
