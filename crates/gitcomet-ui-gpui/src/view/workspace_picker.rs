//! Workspace rows for the generic picker: the adapter from a saved workspace to
//! a picker item, and its colour swatch.

use super::chrome;
use super::components::{
    PickerPromptItem, PickerPromptItemPart, PickerSwatch, PickerSwatchColors, TextTruncationProfile,
};
use gitcomet_state::session::{self, WorkspaceColor};

/// A workspace as every workspace list shows it: its name and state, its
/// repositories underneath, and its colour.
pub(crate) fn workspace_picker_item(workspace: &session::Workspace) -> PickerPromptItem {
    let state = if workspace.restore_on_launch {
        "Open"
    } else {
        "Saved"
    };
    let detail = format!(
        "{state} · {}",
        crate::workspaces::repository_count_label(workspace.repositories.len())
    );
    let repositories = workspace
        .repositories
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(" · ");
    PickerPromptItem::from_parts([
        PickerPromptItemPart::new(workspace.display_name())
            .profile(TextTruncationProfile::End)
            .flexible(false),
        PickerPromptItemPart::separator(" - "),
        PickerPromptItemPart::path(detail),
    ])
    .secondary_parts([PickerPromptItemPart::path(repositories)])
    .swatch(workspace_swatch(workspace.color))
}

/// A workspace's dot and, once a colour is chosen, its title-bar tint behind
/// the row.
pub(crate) fn workspace_swatch(color: Option<WorkspaceColor>) -> PickerSwatch {
    let key = color.map_or(0, |color| color as u64 + 1);
    PickerSwatch::new(key, move |theme| PickerSwatchColors {
        dot: chrome::workspace_color(color),
        row_tint: chrome::workspace_row_tint(color, theme),
    })
}
