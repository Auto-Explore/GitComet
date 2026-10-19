//! The kit's source guards over this crate's rendering code.

use gitcomet_ui_kit::test_support::source_guards::{
    DiscreteControlAllowlist, ELEMENT_OPACITY_HINT, direct_clipboard_access,
    discrete_control_violations, opacity_uses, unscaled_icon_sizes,
};

fn src_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Image and diagram panning start continuous drags on press.
#[test]
fn discrete_controls_use_the_interaction_kit() {
    let violations = discrete_control_violations(
        &src_dir(),
        &DiscreteControlAllowlist {
            custom_styling: &[],
            release_gestures: &[],
            primary_press: &["diagram_preview/surface.rs", "image_preview/surface.rs"],
            context_press: &[],
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

#[test]
fn colour_alpha_instead_of_element_opacity() {
    assert_eq!(
        opacity_uses(&src_dir()),
        Vec::<(String, usize)>::new(),
        "{ELEMENT_OPACITY_HINT}"
    );
}
