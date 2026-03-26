mod details;
mod history;
pub(in crate::view) mod main;
mod sidebar;

pub(super) use details::{DetailsPaneInit, DetailsPaneView};
pub(super) use history::HistoryView;
pub(super) use main::MainPaneView;
pub(super) use sidebar::SidebarPaneView;
