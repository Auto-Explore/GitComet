//! The details pane's layout, one child module per view; shared scroll and
//! bounds helpers live here.

use super::*;
use crate::kit::interaction as controls;
use crate::view::components::{ControlInteractionExt, InteractionState, InteractionStyle};
use gpui::{AnyElement, Div, Stateful};

mod commit_details;
mod commit_form;
mod commit_metadata;
mod comparison;
mod file_lists;
mod status_sections;
mod status_view;
mod worktree_uncommitted;

// Free helpers the sibling modules and the tests share.
#[cfg(test)]
use commit_form::*;
use commit_metadata::*;
use comparison::*;
#[cfg(test)]
use file_lists::*;
use status_sections::*;

#[cfg(test)]
mod indexed_tests;

fn visible_bounds_probe() -> Div {
    // Use a fill probe to capture the clipped viewport bounds for a container.
    // Unioning child bounds can stay larger than the visible area after window resizes.
    div().absolute().top_0().left_0().size_full()
}

impl DetailsPaneView {
    /// The details pane's standard vertical-scroll frame: the list fills the
    /// container, a gutter reserves room for the scrollbar so rows never sit
    /// underneath it, and the scrollbar overlays the right edge. Every scrolling
    /// list in this pane is built from this, so they all scroll alike.
    fn vertical_scroll_frame(
        theme: AppTheme,
        container_id: impl Into<ElementId>,
        scrollbar_id: impl Into<ElementId>,
        scroll: &UniformListScrollHandle,
        list: gpui::UniformList,
    ) -> Stateful<Div> {
        let list = restrict_scroll_to_vertical_axis(
            list.w_full().h_full().min_h(px(0.0)).track_scroll(scroll),
        );
        Self::vertical_scroll_frame_content(theme, container_id, scrollbar_id, scroll, list)
    }

    fn vertical_scroll_frame_content(
        theme: AppTheme,
        container_id: impl Into<ElementId>,
        scrollbar_id: impl Into<ElementId>,
        scroll: &UniformListScrollHandle,
        list: impl IntoElement,
    ) -> Stateful<Div> {
        let scrollbar_gutter = components::Scrollbar::visible_gutter(
            scroll.clone(),
            components::ScrollbarAxis::Vertical,
        );
        div()
            .id(container_id)
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .w_full()
            .overflow_hidden()
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .pr(scrollbar_gutter)
                    .child(list),
            )
            .child(components::Scrollbar::new(scrollbar_id, scroll.clone()).render(theme))
    }
}

#[cfg(test)]
mod tests;
