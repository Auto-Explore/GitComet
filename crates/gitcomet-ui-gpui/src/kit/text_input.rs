use super::text_model::{TextModel, TextModelSnapshot};
use crate::kit::text_truncation::{
    TextTruncationProfile, TruncatedLineLayout, shape_truncated_line_cached,
};
use crate::theme::AppTheme;
use crate::view::components::CONTROL_HEIGHT_PX;
use gpui::prelude::*;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Div, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, IsZero, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Rgba,
    ScrollHandle, ShapedLine, SharedString, Style, TextAlign, TextRun, UTF16Selection, Window,
    WrappedLine, actions, anchored, deferred, div, fill, point, px, relative, size,
};
use rustc_hash::FxHashMap as HashMap;
#[cfg(any(test, feature = "benchmarks"))]
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::borrow::Cow;
use std::hash::Hash;
#[cfg(any(test, feature = "benchmarks"))]
use std::hash::Hasher;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation as _;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        Enter,
        ShiftEnter,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        Home,
        SelectHome,
        End,
        SelectEnd,
        PageUp,
        SelectPageUp,
        PageDown,
        SelectPageDown,
        Paste,
        Cut,
        Copy,
        Undo,
        Redo,
        ShowCharacterPalette,
    ]
);

const MAX_UNDO_STEPS: usize = 100;
const TEXT_INPUT_GUARD_ROWS: usize = 2;
const TEXT_INPUT_PROVIDER_PREFETCH_GUARD_ROWS: usize = 24;
const TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT: usize = 4;
const TEXT_INPUT_MAX_LINE_SHAPE_BYTES: usize = 4 * 1024;
const TEXT_INPUT_SHAPE_CACHE_LIMIT: usize = 8 * 1024;
const TEXT_INPUT_TRUNCATION_SUFFIX: &str = "…";
const TEXT_INPUT_WRAP_SYNC_LINE_THRESHOLD: usize = 256;
const TEXT_INPUT_WRAP_FOREGROUND_BUDGET_MS: u64 = 4;
const TEXT_INPUT_WRAP_BACKGROUND_YIELD_EVERY_ROWS: usize = 100;
const TEXT_INPUT_WRAP_DIRTY_SYNC_LINE_LIMIT: usize = 128;
const TEXT_INPUT_WRAP_TAB_STOP_COLUMNS: usize = 4;
const TEXT_INPUT_WRAP_CHAR_ADVANCE_FACTOR: f32 = 0.6;
const TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES: usize = 4_096;
const TEXT_INPUT_STREAMED_HIGHLIGHT_LEGACY_LINE_THRESHOLD: usize = 64;
const TEXT_INPUT_STREAMED_HIGHLIGHT_ESTIMATED_RUNS_PER_VISIBLE_LINE: usize = 2;
const TEXT_INPUT_INLINE_ACTIVE_HIGHLIGHT_CAPACITY: usize = 8;
const TEXT_INPUT_INLINE_TEXT_RUN_CAPACITY: usize = 32;

mod editing;
mod element;
mod highlight;
mod render;
mod shaping;
mod state;
mod wrap;

pub use state::{HighlightProvider, HighlightProviderResult, TextInput, TextInputOptions};

#[cfg(feature = "benchmarks")]
pub(crate) use highlight::{
    benchmark_text_input_runs_legacy_visible_window,
    benchmark_text_input_runs_streamed_visible_window,
};
#[cfg(feature = "benchmarks")]
pub(crate) use shaping::benchmark_text_input_shaping_slice;
#[cfg(feature = "benchmarks")]
pub(crate) use wrap::benchmark_text_input_wrap_rows_for_line;

#[cfg(test)]
mod tests;
