mod details;
mod history;
pub(in crate::view) mod main;
mod reflog;
mod sidebar;

pub(super) use details::{DetailsPaneInit, DetailsPaneView};
pub(super) use history::HistoryView;
#[allow(unused_imports)]
pub(in crate::view) use history::{
    HistoryColumnDragLayout, HistoryPrimarySelection, history_column_resize_drag_params,
    history_column_resize_max_width, history_column_resize_state, history_primary_selection,
    history_resize_state_visible_columns, history_visible_columns_for_layout,
    history_visible_columns_for_layout_with_resize_state,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(in crate::view) use history::{
    history_resize_state_preserves_visible_columns,
    history_resize_state_visible_columns_for_current_width,
};
pub(in crate::view) use main::MainPaneInit;
pub(crate) use main::MainPaneView;
pub(super) use reflog::{ReflogPaneInit, ReflogPaneView};
pub(in crate::view) use sidebar::file_browser_search_is_active;
pub(super) use sidebar::{CollapsedSidebarSection, SidebarPaneView};
