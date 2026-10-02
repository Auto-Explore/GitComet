use super::*;

/// The bottom bar's zoom menu: presets for this window, then a way back to
/// the default UI scale once the window has its own zoom.
pub(super) fn model(cx: &gpui::Context<PopoverHost>) -> ContextMenuModel {
    let zoomed = cx
        .current_window_id()
        .and_then(|id| crate::ui_scale::window_override(cx, id));
    model_for(zoomed, crate::ui_scale::default_percent(cx))
}

fn model_for(zoomed: Option<u32>, default_percent: u32) -> ContextMenuModel {
    let current = zoomed.unwrap_or(default_percent);
    let mut items = vec![
        ContextMenuItem::Header("Zoom".into()),
        ContextMenuItem::Separator,
    ];
    items.extend(
        crate::ui_scale::UI_SCALE_PRESETS
            .iter()
            .copied()
            .map(|percent| ContextMenuItem::Entry {
                label: crate::ui_scale::label(percent).into(),
                icon: (percent == current).then_some("icons/check.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::SetUiScale {
                    percent: Some(percent),
                }),
            }),
    );
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: format!("Use default ({})", crate::ui_scale::label(default_percent)).into(),
        icon: None,
        shortcut: None,
        disabled: zoomed.is_none(),
        action: Box::new(ContextMenuAction::SetUiScale { percent: None }),
    });
    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(model: &ContextMenuModel) -> Vec<String> {
        model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry {
                    label,
                    icon: Some(_),
                    ..
                } => Some(label.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn checks_the_window_zoom_and_offers_the_default_once_zoomed() {
        let following = model_for(None, 125);
        assert_eq!(checked(&following), ["125%"]);
        assert!(following.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { label, disabled: true, .. } if label.as_ref() == "Use default (125%)"
        )));

        let zoomed = model_for(Some(150), 125);
        assert_eq!(checked(&zoomed), ["150%"]);
        assert!(zoomed.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { label, disabled: false, .. } if label.as_ref() == "Use default (125%)"
        )));
    }
}
