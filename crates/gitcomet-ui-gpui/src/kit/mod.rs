mod scrollbar;
mod text_input;
pub(crate) mod text_model;

pub use scrollbar::{Scrollbar, ScrollbarAxis, ScrollbarMarker, ScrollbarMarkerKind};
#[cfg(feature = "benchmarks")]
pub(crate) use scrollbar::{compute_vertical_click_offset, vertical_thumb_metrics};
pub use text_input::{
    Backspace, Copy, Cut, Delete, DeleteWordLeft, DeleteWordRight, Down, End, Enter,
    HighlightProvider, HighlightProviderResult, Home, Left, PageDown, PageUp, Paste, Redo, Right,
    SelectAll, SelectDown, SelectEnd, SelectHome, SelectLeft, SelectPageDown, SelectPageUp,
    SelectRight, SelectUp, SelectWordLeft, SelectWordRight, TextInput, TextInputOptions, Undo, Up,
    WordLeft, WordRight,
};
#[cfg(feature = "benchmarks")]
pub(crate) use text_input::{
    benchmark_text_input_runs_legacy_visible_window,
    benchmark_text_input_runs_streamed_visible_window, benchmark_text_input_shaping_slice,
    benchmark_text_input_wrap_rows_for_line,
};

#[cfg(feature = "benchmarks")]
pub(crate) use scrollbar::{compute_vertical_click_offset, vertical_thumb_metrics};

#[cfg(target_os = "macos")]
pub use text_input::ShowCharacterPalette;
