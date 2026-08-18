mod avatar;
mod button;
mod commit_link_menu;
mod containers;
mod context_menu;
mod diff_stat;
mod interactive_row;
mod modal;
mod picker_prompt;
mod repository_badge;
mod resize_grip;
mod shortcut_keys;
mod skeleton;
mod split_button;
mod tab;
mod tab_bar;
mod text_fade;
mod toast;
mod tokens;
mod truncated_text;

pub use avatar::{
    AVATAR_DIAMETER_PX, AVATAR_FONT_PX, author_avatar, author_color, author_initials,
    initials_paint_origin_y,
};
pub use button::{Button, ButtonStyle};
pub use commit_link_menu::{CommitLinkMenu, LinkTarget, MessageLink};
pub use containers::{ScrollContainer, empty_state, empty_state_message, split_columns_header};
#[cfg(test)]
pub use containers::{panel, pill};
pub use context_menu::{
    ContextMenuEntry, ContextMenuIconSlot, ContextMenuText, context_menu, context_menu_description,
    context_menu_header, context_menu_label, context_menu_separator,
};
pub use diff_stat::diff_stat;
pub use interactive_row::{
    InteractiveRowExt, InteractiveRowState, InteractiveRowStyle, light_theme_selection_outline,
};
pub use modal::{modal_scrim, modal_surface, popover_surface};
/// Public field type of [`PickerPromptLayout::headers`], carried out of the
/// private module with it so a caller can name what that field hands them
/// instead of only ever binding it through an inferred closure argument.
#[allow(unused_imports)]
pub use picker_prompt::PickerPromptHeader;
pub use picker_prompt::picker_prompt_layout;
pub use picker_prompt::{
    PICKER_LIST_MAX_HEIGHT_PX, PickerPrompt, PickerPromptContextMenuEvent, PickerPromptGeometry,
    PickerPromptItem, PickerPromptItemPart, PickerPromptLayout,
    picker_prompt_layout_with_collapsed,
};
pub use repository_badge::{
    REPOSITORY_BADGE_SIZE_PX, repository_initials, repository_initials_box,
};
pub use resize_grip::{ResizeGripAxis, resize_grip};
pub use shortcut_keys::shortcut_keys;
pub use skeleton::skeleton;
pub use split_button::{SplitButton, SplitButtonStyle};
pub use tab::Tab;
pub use tab_bar::{TabBar, TabBarScroll};
pub use text_fade::{FadingText, trailing_fade};
pub use toast::{ToastKind, toast};
pub use tokens::*;
pub(crate) use truncated_text::{
    PathTruncationAlignmentGroup, TruncatedText, TruncatedTextFlex, TruncatedTextTooltipMode,
};

pub(crate) use crate::kit::text_truncation::TextTruncationProfile;
pub use crate::kit::{
    MINIMAP_COLUMN_WIDTH_PX, MinimapColumn, Scrollbar, ScrollbarAxis, ScrollbarMarker,
    ScrollbarMarkerKind, TextInput, TextInputOptions,
};
