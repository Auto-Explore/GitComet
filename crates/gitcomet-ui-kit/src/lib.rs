//! GitComet's GPUI foundations, reusable without the GitComet application:
//! the runtime policy, appearance and UI scale, themes, fonts, interaction
//! primitives, text inputs, tooltips, icons, and components.
//!
//! The kit knows nothing about repositories, sessions, or the application
//! window. Hosts pass stored preferences in, install a [`ui_runtime`] policy
//! at launch, and supply any product artwork.

// Every type a public signature exposes must be nameable by a consumer.
#![warn(unnameable_types)]

/// The GPUI this kit is built against; use it instead of a separate `gpui`
/// dependency so versions cannot drift.
pub use gpui;

/// The exact GPUI revision used by this kit and its host.
pub const GPUI_REVISION: &str = "46e1ede350cc95071b389e83831e18e7d114bbaf";

pub mod appearance;
pub mod assets;
pub mod bundled_fonts;
pub mod click;
pub mod clipboard;
pub mod components;
pub mod drag_autoscroll;
pub mod file_icons;
pub mod font_preferences;
pub mod icons;
pub mod interaction;
pub mod interaction_paint;
pub mod linux_gui_env;
pub mod menu;
pub mod menu_placement;
mod minimap;
pub mod press_gesture;
pub mod rope;
mod scrollbar;
mod text_input;
pub mod text_model;
pub mod text_runs;
pub mod text_selection;
pub mod text_selection_owner;
pub mod text_truncation;
pub mod theme;
mod thread_cpu;
pub mod tooltip;
pub mod tooltip_host;
pub mod ui_probe;
pub mod ui_runtime;
pub mod ui_scale;
pub mod window_focus;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use minimap::{MINIMAP_COLUMN_WIDTH_PX, MinimapColumn};
pub use scrollbar::{
    SCROLLBAR_GUTTER_PX, Scrollbar, ScrollbarAxis, ScrollbarDriver, ScrollbarMarker,
    ScrollbarMarkerKind,
};
#[cfg(feature = "benchmarks")]
pub use scrollbar::{ThumbMetrics, compute_vertical_click_offset, vertical_thumb_metrics};
pub use text_input::utf8_edit_delta_between_texts;
pub use text_input::{
    Backspace, Copy, Cut, Delete, DeleteToLineEnd, DeleteToLineStart, DeleteWordLeft,
    DeleteWordRight, DocumentEnd, DocumentHome, Down, End, Enter, HighlightProvider,
    HighlightProviderResult, Home, Left, PageDown, PageUp, Paste, Redo, Right, SelectAll,
    SelectDown, SelectEnd, SelectHome, SelectLeft, SelectPageDown, SelectPageUp, SelectRight,
    SelectUp, SelectWordLeft, SelectWordRight, ShiftEnter, TextInput, TextInputChanged,
    TextInputOptions, Undo, Up, WordLeft, WordRight,
};
#[cfg(feature = "benchmarks")]
pub use text_input::{
    benchmark_text_input_runs_legacy_visible_window,
    benchmark_text_input_runs_streamed_visible_window, benchmark_text_input_shaping_slice,
    benchmark_text_input_wrap_rows_for_line,
};

#[cfg(target_os = "macos")]
pub use text_input::ShowCharacterPalette;

/// Keeps a scroll container from scrolling sideways.
pub fn restrict_scroll_to_vertical_axis<E: gpui::Styled>(mut element: E) -> E {
    element.style().restrict_scroll_to_axis = Some(true);
    element
}

#[cfg(test)]
mod version_contract {
    #[test]
    fn exported_gpui_revision_matches_both_workspace_dependencies() {
        let manifest = include_str!("../../../Cargo.toml");
        let pins: Vec<_> = manifest
            .lines()
            .filter(|line| line.starts_with("gpui = ") || line.starts_with("gpui_platform = "))
            .collect();
        assert_eq!(pins.len(), 2);
        for pin in pins {
            assert!(
                pin.contains(super::GPUI_REVISION),
                "GPUI_REVISION must track {pin}"
            );
        }
    }
}
