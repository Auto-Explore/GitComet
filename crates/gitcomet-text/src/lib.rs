//! Text foundations shared by syntax highlighting, previews and the UI kit,
//! with no dependency on GPUI itself: a rope and text model, tab expansion,
//! instrumented caches and the token kinds a highlighter emits.

pub mod cache;
pub mod date_time;
pub mod path_display;
pub mod rope;
pub mod tab_width;
pub mod text_model;
mod token;

pub use token::{SyntaxPalette, SyntaxTokenKind};
