//! The kit's source guards over this crate's rendering code.

use gitcomet_ui_kit::test_support::source_guards::{
    DiscreteControlAllowlist, direct_clipboard_access, discrete_control_violations,
    unscaled_icon_sizes,
};

fn src_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// The viewport handles raw presses and releases: selection drags, mouse
/// reporting to the PTY and its context menu all start on press.
#[test]
fn discrete_controls_use_the_interaction_kit() {
    let violations = discrete_control_violations(
        &src_dir(),
        &DiscreteControlAllowlist {
            custom_styling: &[],
            release_gestures: &["viewport.rs"],
            primary_press: &["viewport.rs"],
            context_press: &["viewport.rs"],
        },
    );
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn clipboard_access_goes_through_the_kit() {
    let offenders = direct_clipboard_access(&src_dir(), &[]);
    assert!(offenders.is_empty(), "{offenders:?}");
}

#[test]
fn icon_sizes_follow_the_ui_scale() {
    let unscaled = unscaled_icon_sizes(&src_dir(), &[]);
    assert!(unscaled.is_empty(), "{unscaled:#?}");
}
