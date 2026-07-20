use super::*;

use crate::view::shortcut_labels::secondary_shortcut;

pub(super) fn model(
    _cursor_line: usize,
    selected_text: &Option<String>,
    _has_source_a: bool,
    _has_source_b: bool,
    _has_source_c: bool,
    _is_three_way: bool,
) -> ContextMenuModel {
    let has_selection = selected_text.is_some();
    let copy_text = selected_text.clone().unwrap_or_default();
    let cut_text = copy_text.clone();

    let items = vec![
        ContextMenuItem::Entry {
            label: "Copy".into(),
            icon: None,
            shortcut: Some(secondary_shortcut("C").into()),
            disabled: !has_selection,
            action: Box::new(ContextMenuAction::CopyText { text: copy_text }),
        },
        ContextMenuItem::Entry {
            label: "Cut".into(),
            icon: None,
            shortcut: Some(secondary_shortcut("X").into()),
            disabled: !has_selection,
            action: Box::new(ContextMenuAction::ConflictResolverOutputCut { text: cut_text }),
        },
        ContextMenuItem::Entry {
            label: "Paste".into(),
            icon: None,
            shortcut: Some(secondary_shortcut("V").into()),
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverOutputPaste),
        },
    ];

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_builds_all_entries_three_way() {
        let selected = Some("hello".to_string());
        let m = model(2, &selected, true, true, true, true);

        // Copy, Cut, Paste
        assert_eq!(m.items.len(), 3);

        // Copy should be enabled with selection
        match &m.items[0] {
            ContextMenuItem::Entry {
                label, disabled, ..
            } => {
                assert_eq!(label.as_ref(), "Copy");
                assert!(!*disabled);
            }
            _ => panic!("expected Copy entry"),
        }

        match &m.items[2] {
            ContextMenuItem::Entry { label, .. } => assert_eq!(label.as_ref(), "Paste"),
            _ => panic!("expected Paste entry"),
        }
    }

    #[test]
    fn model_two_way_has_only_clipboard_actions() {
        let m = model(0, &None, true, true, false, false);

        // Copy, Cut, Paste
        assert_eq!(m.items.len(), 3);

        // Copy/Cut disabled without selection
        match &m.items[0] {
            ContextMenuItem::Entry { disabled, .. } => assert!(*disabled),
            _ => panic!("expected entry"),
        }
        match &m.items[1] {
            ContextMenuItem::Entry { disabled, .. } => assert!(*disabled),
            _ => panic!("expected entry"),
        }

        match &m.items[2] {
            ContextMenuItem::Entry { label, .. } => assert_eq!(label.as_ref(), "Paste"),
            _ => panic!("expected entry"),
        }
    }

    #[test]
    fn paste_is_always_enabled() {
        let m = model(5, &None, false, true, false, true);

        match &m.items[2] {
            ContextMenuItem::Entry { disabled, .. } => assert!(!*disabled),
            _ => panic!("expected entry"),
        }
    }
}
