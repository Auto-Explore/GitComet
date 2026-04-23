use super::*;

pub(super) fn model(host: &PopoverHost) -> ContextMenuModel {
    model_for_mode(host.diff_content_mode)
}

fn model_for_mode(mode: DiffContentMode) -> ContextMenuModel {
    let check = |enabled: bool| enabled.then_some("icons/check.svg".into());

    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Diff mode".into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: DiffContentMode::Collapsed.label().into(),
            icon: check(mode == DiffContentMode::Collapsed),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetDiffContentMode {
                mode: DiffContentMode::Collapsed,
            }),
        },
        ContextMenuItem::Entry {
            label: DiffContentMode::Full.label().into(),
            icon: check(mode == DiffContentMode::Full),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetDiffContentMode {
                mode: DiffContentMode::Full,
            }),
        },
        ContextMenuItem::Separator,
        ContextMenuItem::Label(
            "Collapsed hides unchanged sections. Full shows the entire file.".into(),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_marks_current_mode() {
        let model = super::model_for_mode(DiffContentMode::Collapsed);

        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, icon, .. }
                    if label.as_ref() == DiffContentMode::Collapsed.label()
                        && icon
                            .as_ref()
                            .is_some_and(|icon| icon.as_ref() == "icons/check.svg")
            )
        }));
    }
}
