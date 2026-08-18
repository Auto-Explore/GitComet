//! Drag & drop payloads shared across views (source rows and drop targets).
//!
//! The worktree file rows in the Changes panel are draggable; virtual branch
//! rows in the sidebar act as drop targets that assign the dragged path to the
//! branch (the same operation as the "Assign to virtual branch…" context
//! menu item).

use gitcomet_state::model::RepoId;
use gpui::prelude::*;
use gpui::{App, Entity, Render, Window, div, px};
use std::path::PathBuf;

/// Payload dragged from a worktree file row: the repo owning the file and the
/// file's path relative to the repo root.
#[derive(Clone, Debug)]
pub(super) struct StatusFileDrag {
    pub repo_id: RepoId,
    pub path: PathBuf,
}

/// Invisible drag preview required by gpui's drag machinery. The source row
/// stays in place during the drag (like the repo tab reorder), so the carrier
/// is empty.
pub(super) struct StatusFileDragCarrier;

impl Render for StatusFileDragCarrier {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size(px(1.0)).opacity(0.0)
    }
}

/// Builds the invisible carrier entity for a `StatusFileDrag` payload.
pub(super) fn status_file_drag_carrier(
    _drag: &StatusFileDrag,
    _offset: gpui::Point<gpui::Pixels>,
    _window: &mut Window,
    cx: &mut App,
) -> Entity<StatusFileDragCarrier> {
    cx.new(|_cx| StatusFileDragCarrier)
}
