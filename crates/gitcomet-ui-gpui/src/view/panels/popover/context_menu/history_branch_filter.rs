use super::*;

pub(super) fn model(host: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let current_scope = host
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| repo.history_state.history_scope)
        .unwrap_or_default();
    model_for_scope(repo_id, current_scope)
}

fn model_for_scope(
    repo_id: RepoId,
    current_scope: gitcomet_core::domain::LogScope,
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
    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_marks_current_history_mode() {
        let model = super::model_for_scope(RepoId(11), gitcomet_core::domain::LogScope::MergesOnly);

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
