//! The kit's shared test helpers; its locks serialize host and kit tests alike.

pub(crate) use gitcomet_ui_kit::test_support::{
    lock_clipboard_test, lock_visual_test, painted_control_quads, refresh_and_draw,
};
