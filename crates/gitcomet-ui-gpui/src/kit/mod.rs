mod scrollbar;
mod text_input;
pub(crate) mod text_model;

pub use scrollbar::{Scrollbar, ScrollbarMarker, ScrollbarMarkerKind};
pub use text_input::{
    Backspace, Copy, Cut, Delete, DeleteWordLeft, DeleteWordRight, Down, End, Enter,
    HighlightProvider, HighlightProviderResult, Home, Left, PageDown, PageUp, Paste, Right,
    SelectAll, SelectDown, SelectEnd, SelectHome, SelectLeft, SelectPageDown, SelectPageUp,
    SelectRight, SelectUp, SelectWordLeft, SelectWordRight, TextInput, TextInputOptions, Undo, Up,
    WordLeft, WordRight,
};
pub(crate) use text_input::{
    benchmark_text_input_runs_legacy_visible_window,
    benchmark_text_input_runs_streamed_visible_window,
};

#[cfg(target_os = "macos")]
pub use text_input::ShowCharacterPalette;
