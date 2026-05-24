use super::*;

pub(super) fn model(host: &PopoverHost) -> ContextMenuModel {
    model_for_diff_actions(
        host.diff_whitespace_mode,
        host.diff_reveal_whitespace_chars,
        host.diff_word_wrap,
        host.diff_show_line_numbers,
    )
}

fn model_for_diff_actions(
    mode: DiffWhitespaceMode,
    reveal_whitespace_chars: bool,
    word_wrap: bool,
    show_line_numbers: bool,
) -> ContextMenuModel {
    let show_whitespace = mode == DiffWhitespaceMode::Show;
    let next_mode = mode.toggled();

    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Diff actions".into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Show whitespace changes".into(),
            icon: show_whitespace.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetDiffWhitespaceMode { mode: next_mode }),
        },
        ContextMenuItem::Entry {
            label: "Reveal whitespace characters".into(),
            icon: reveal_whitespace_chars.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetDiffRevealWhitespaceChars {
                enabled: !reveal_whitespace_chars,
            }),
        },
        ContextMenuItem::Entry {
            label: "Word wrap".into(),
            icon: word_wrap.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetDiffWordWrap {
                enabled: !word_wrap,
            }),
        },
        ContextMenuItem::Entry {
            label: "Show line numbers".into(),
            icon: show_line_numbers.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetDiffShowLineNumbers {
                enabled: !show_line_numbers,
            }),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_toggles_whitespace_mode() {
        let model = model_for_diff_actions(DiffWhitespaceMode::Show, false, false, true);

        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry {
                    label,
                    icon,
                    action,
                    ..
                } if label.as_ref() == "Show whitespace changes"
                    && icon
                        .as_ref()
                        .is_some_and(|icon| icon.as_ref() == "icons/check.svg")
                    && matches!(
                        action.as_ref(),
                        ContextMenuAction::SetDiffWhitespaceMode {
                            mode: DiffWhitespaceMode::Ignore
                        }
                    )
            )
        }));

        let model = model_for_diff_actions(DiffWhitespaceMode::Ignore, false, false, true);
        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry {
                    label,
                    icon,
                    action,
                    ..
                } if label.as_ref() == "Show whitespace changes"
                    && icon.is_none()
                    && matches!(
                        action.as_ref(),
                        ContextMenuAction::SetDiffWhitespaceMode {
                            mode: DiffWhitespaceMode::Show
                        }
                    )
            )
        }));
    }

    #[test]
    fn model_toggles_reveal_whitespace_chars_and_word_wrap() {
        let model = model_for_diff_actions(DiffWhitespaceMode::Show, true, false, true);

        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry {
                    label,
                    icon,
                    action,
                    ..
                } if label.as_ref() == "Reveal whitespace characters"
                    && icon
                        .as_ref()
                        .is_some_and(|icon| icon.as_ref() == "icons/check.svg")
                    && matches!(
                        action.as_ref(),
                        ContextMenuAction::SetDiffRevealWhitespaceChars { enabled: false }
                    )
            )
        }));
        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry {
                    label,
                    icon,
                    action,
                    ..
                } if label.as_ref() == "Word wrap"
                    && icon.is_none()
                    && matches!(
                        action.as_ref(),
                        ContextMenuAction::SetDiffWordWrap { enabled: true }
                    )
            )
        }));
        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry {
                    label,
                    icon,
                    action,
                    ..
                } if label.as_ref() == "Show line numbers"
                    && icon
                        .as_ref()
                        .is_some_and(|icon| icon.as_ref() == "icons/check.svg")
                    && matches!(
                        action.as_ref(),
                        ContextMenuAction::SetDiffShowLineNumbers { enabled: false }
                    )
            )
        }));
    }
}
