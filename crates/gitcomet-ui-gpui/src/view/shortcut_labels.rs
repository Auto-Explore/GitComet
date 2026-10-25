//! The key labels every diff surface shows for its file and change
//! navigation buttons, matching the bindings the app registers.

/// The keys for every diff surface's previous-file button.
pub(crate) fn previous_file_shortcuts() -> Vec<gpui::SharedString> {
    vec![
        "F1".into(),
        crate::kit::components::alt_shortcut("Left").into(),
    ]
}

/// The keys for every diff surface's next-file button.
pub(crate) fn next_file_shortcuts() -> Vec<gpui::SharedString> {
    vec![
        "F4".into(),
        crate::kit::components::alt_shortcut("Right").into(),
    ]
}

/// The keys for every diff surface's previous-change button.
pub(crate) fn previous_change_shortcuts() -> Vec<gpui::SharedString> {
    vec![
        "F2".into(),
        "Shift+F7".into(),
        crate::kit::components::alt_shortcut("Up").into(),
    ]
}

/// The keys for every diff surface's next-change button.
pub(crate) fn next_change_shortcuts() -> Vec<gpui::SharedString> {
    vec![
        "F3".into(),
        "F7".into(),
        crate::kit::components::alt_shortcut("Down").into(),
    ]
}
