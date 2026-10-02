//! The example's artwork replaces GitComet's wherever it is served, under
//! paths that never name GitComet, and the house themes carry its name. Its
//! own process: the identity is installed once per process.

use gitcomet_ui_gpui::{BRAND_ASSETS, GitCometAssets};
use gitcomet_ui_kit::gpui::AssetSource as _;
use gitcomet_ui_kit::theme::{DEFAULT_DARK_THEME_KEY, DEFAULT_LIGHT_THEME_KEY, theme_label};

#[test]
fn the_examples_artwork_and_names_replace_gitcomets() {
    gitcomet_core::identity::install(gitcomet_extension_example::identity())
        .expect("nothing resolved the identity yet");
    let assets = GitCometAssets::default();
    for path in assets.list("").expect("list assets") {
        assert!(
            !path.to_lowercase().contains("gitcomet"),
            "{path} names GitComet"
        );
    }

    let branding = gitcomet_extension_example::BRANDING;
    let expected = [
        ("brand/app-icon.png", branding.app_icon_png),
        ("brand/window-icon.png", branding.window_icon_png),
        ("brand/logo.svg", branding.logo_svg),
        ("brand/mark.svg", branding.mark_svg),
    ];
    assert_eq!(
        expected.len(),
        BRAND_ASSETS.len(),
        "every brand path is checked"
    );
    for (path, bytes) in expected {
        let served = assets.load(path).expect("load").expect("served");
        assert_eq!(Some(served.as_ref()), bytes, "{path}");
    }
    assert_eq!(
        gitcomet_core::identity::current().branding().tagline,
        Some("Code review on any Git repository")
    );
    assert!(branding.splash_backdrop_dark_png.is_some());
    assert!(branding.splash_backdrop_light_png.is_some());

    assert_eq!(
        theme_label(DEFAULT_DARK_THEME_KEY).as_deref(),
        Some("Comet Example Dark")
    );
    assert_eq!(
        theme_label(DEFAULT_LIGHT_THEME_KEY).as_deref(),
        Some("Comet Example Light")
    );
}
