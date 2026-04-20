use super::*;
use gpui::AppContext;

pub(super) fn model(cx: &gpui::Context<PopoverHost>) -> ContextMenuModel {
    let current_percent =
        cx.read_global::<crate::ui_scale::AppUiScale, _>(|scale, _| scale.percent);
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
                icon: (percent == current_percent).then_some("icons/check.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::SetUiScale { percent }),
            }),
    );

    ContextMenuModel::new(items)
}
