//! The UI kit's components plus the host's domain components.

pub use gitcomet_ui_kit::components::*;

mod commit_link_menu;
#[cfg(test)]
mod resize_grip_tests;

pub use commit_link_menu::{CommitLinkMenu, LinkTarget, MessageLink};
