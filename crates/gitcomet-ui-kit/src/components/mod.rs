//! Generic components built on the kit's interaction primitives.

mod avatar;
mod button;
mod containers;
mod context_menu;
mod diff_stat;
mod interactive_row;
mod interstitial;
mod list_layout;
mod modal;
mod navigation;
mod painted_when;
mod panel_tab;
mod picker_prompt;
mod quick_search_bar;
mod repository_badge;
mod resize_grip;
mod settings_rows;
mod shortcut_keys;
mod skeleton;
mod split_button;
mod tab;
mod tab_bar;
mod text_fade;
mod toast;
mod tokens;
mod truncated_text;

pub use crate::interaction::{
    ControlActivation, ControlInteractionExt, InteractionState, InteractionStyle,
    control_open_background,
};
pub use avatar::{
    AVATAR_DIAMETER_PX, AVATAR_FONT_PX, author_avatar, author_color, author_initials,
    initials_paint_origin_y,
};
pub use button::{Button, ButtonStyle, inline_icon_button};
pub use containers::{
    ScrollContainer, content_header_bar, empty_state, empty_state_message, progress_bar,
    progress_bar_border_color, progress_bar_track_color, split_columns_header,
};
#[cfg(any(test, feature = "test-support"))]
pub use containers::{panel, pill};
pub use context_menu::{
    ContextMenuEntry, ContextMenuIconSlot, ContextMenuText, MENU_SCROLL_ARROW_HEIGHT_PX,
    MenuScrollDirection, context_menu, context_menu_description, context_menu_group,
    context_menu_header, context_menu_icon_path, context_menu_label, context_menu_scroll_arrow,
    context_menu_separator,
};
pub use diff_stat::{diff_stat, diff_stat_optional};
pub use interactive_row::{InteractiveRowExt, InteractiveRowState, InteractiveRowStyle};
pub use interstitial::{INTERSTITIAL_CARD_MAX_WIDTH_PX, interstitial, interstitial_cta_button};
pub use list_layout::{ListLayout, list_layout_button};
pub use modal::{modal_scrim, modal_surface, popover_surface};
pub use navigation::{
    NavTab, navigation_tab, navigation_tab_metrics, navigation_tab_strip, selectable_field,
};
pub use painted_when::{PaintedWhen, painted_when};
pub use panel_tab::{on_nested_control_click, panel_tab, panel_tab_close, panel_tab_text_color};
/// Public field type of [`PickerPromptLayout::headers`], carried out of the
/// private module with it so a caller can name what that field hands them
/// instead of only ever binding it through an inferred closure argument.
#[allow(unused_imports)]
pub use picker_prompt::PickerPromptHeader;
pub use picker_prompt::picker_prompt_layout;
pub use picker_prompt::{
    OnRemoveFn, PICKER_LIST_MAX_HEIGHT_PX, PickerPrompt, PickerPromptContextMenuEvent,
    PickerPromptGeometry, PickerPromptItem, PickerPromptItemPart, PickerPromptLayout,
    PickerPromptOrder, PickerRowKey, PickerRowSpec, PickerSwatch, PickerSwatchColors,
    picker_prompt_layout_ordered, picker_row, remove_row_button, row_height as picker_row_height,
    selected_hint_pill,
};
pub use quick_search_bar::{QuickSearchBar, QuickSearchStatus};
pub use repository_badge::{
    REPOSITORY_BADGE_SIZE_PX, repository_initials, repository_initials_box,
};
pub use resize_grip::{ResizeGripAxis, resize_grip, resize_grip_hover_tint};
pub use settings_rows::{
    SETTINGS_NAV_COLUMN_WIDTH_PX, settings_card, settings_card_with_action,
    settings_detail_container, settings_dropdown_background, settings_dropdown_border_color,
    settings_info_row, settings_link_row, settings_nav_item, settings_option_row,
    settings_row_separator_color, settings_subsection_heading, settings_summary_row,
    settings_summary_row_with_value_prefix, settings_toggle_row,
};
pub use shortcut_keys::shortcut_keys;
pub use skeleton::skeleton;
pub use split_button::{SplitButton, SplitButtonStyle};
pub use tab::Tab;
pub use tab_bar::{TabBar, TabBarScroll};
pub use text_fade::{FadingText, trailing_fade};
pub use toast::{TOAST_BADGE_PX, TOAST_WIDTH_PX, ToastKind, toast};
pub use tokens::*;
pub use truncated_text::{
    PathTruncationAlignmentGroup, TruncatedText, TruncatedTextFlex, TruncatedTextTooltipMode,
};

pub use crate::text_truncation::TextTruncationProfile;
pub use crate::{
    MINIMAP_COLUMN_WIDTH_PX, MinimapColumn, Scrollbar, ScrollbarAxis, ScrollbarMarker,
    ScrollbarMarkerKind, TextInput, TextInputOptions,
};
