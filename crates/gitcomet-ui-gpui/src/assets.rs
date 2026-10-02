use gitcomet_ui_kit::assets::KitAssets;
use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Product artwork under neutral paths: the product's own (from its
/// identity's branding) or GitComet's.
pub const BRAND_ASSETS: [&str; 4] = [
    "brand/app-icon.png",
    "brand/window-icon.png",
    "brand/logo.svg",
    "brand/mark.svg",
];

/// The artwork at a [`BRAND_ASSETS`] path.
fn brand_asset(path: &str) -> Option<&'static [u8]> {
    let branding = gitcomet_core::identity::current().branding();
    let (branded, default): (_, &'static [u8]) = match path {
        "brand/app-icon.png" => (
            branding.app_icon_png,
            include_bytes!("../../../assets/gitcomet-512.png"),
        ),
        "brand/window-icon.png" => (
            branding.window_icon_png,
            include_bytes!("../../../assets/gitcomet-window-icon.png"),
        ),
        "brand/logo.svg" => (
            branding.logo_svg,
            include_bytes!("../../../assets/gitcomet_logo.svg"),
        ),
        "brand/mark.svg" => (
            branding.mark_svg,
            include_bytes!("../assets/gitcomet_mark.svg"),
        ),
        _ => return None,
    };
    Some(branded.unwrap_or(default))
}

/// The window icon (X11 and Wayland), decoded once.
pub(crate) fn window_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    static ICON: std::sync::OnceLock<Option<std::sync::Arc<image::RgbaImage>>> =
        std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        let png = brand_asset("brand/window-icon.png")?;
        let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
        Some(std::sync::Arc::new(image.into_rgba8()))
    })
    .clone()
}

/// Product artwork over the UI kit's icon set, plus any extension assets
/// under `extensions/<id>/`.
#[derive(Default)]
pub struct GitCometAssets {
    extensions: Vec<(SharedString, &'static [u8])>,
}

impl GitCometAssets {
    pub(crate) fn with_extensions(assets: &[(String, &'static [u8])]) -> Self {
        Self {
            extensions: assets
                .iter()
                .map(|(path, bytes)| (SharedString::from(path.clone()), *bytes))
                .collect(),
        }
    }

    fn extension_asset(&self, path: &str) -> Option<Cow<'static, [u8]>> {
        self.extensions
            .iter()
            .find_map(|(asset, bytes)| (asset == path).then_some(Cow::Borrowed(*bytes)))
    }

    fn load_static(path: &str) -> Option<Cow<'static, [u8]>> {
        brand_asset(path)
            .map(Cow::Borrowed)
            .or_else(|| KitAssets::load_static(path))
    }

    fn list_static(dir: &str) -> Vec<SharedString> {
        match dir.trim_end_matches('/') {
            // GPUI's AssetRegistry registers only list(""); it does not recurse
            // into directory entries. Include every embedded file here.
            "" => BRAND_ASSETS
                .into_iter()
                .map(SharedString::from)
                .chain(KitAssets::list_static(""))
                .collect(),
            "brand" => BRAND_ASSETS.into_iter().map(SharedString::from).collect(),
            other => KitAssets::list_static(other),
        }
    }
}

impl AssetSource for GitCometAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.starts_with("extensions/") {
            return Ok(self.extension_asset(path));
        }
        Ok(Self::load_static(path))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut listed = Self::list_static(path);
        let dir = path.trim_end_matches('/');
        if dir.is_empty() || dir == "extensions" || dir.starts_with("extensions/") {
            listed.extend(
                self.extensions
                    .iter()
                    .filter(|(asset, _)| dir.is_empty() || asset.starts_with(&format!("{dir}/")))
                    .map(|(asset, _)| asset.clone()),
            );
        }
        Ok(listed)
    }
}

#[cfg(test)]
mod tests {
    use super::{BRAND_ASSETS, GitCometAssets};
    use gpui::{AssetRegistry, AssetSource, DevicePixels, SvgRenderer, SvgSize, size};
    use std::{collections::BTreeSet, sync::Arc};

    fn expected_asset_paths() -> impl Iterator<Item = &'static str> {
        BRAND_ASSETS
            .into_iter()
            .chain(gitcomet_ui_kit::assets::icon_paths())
    }

    #[test]
    fn root_listing_contains_every_asset_once() {
        let listed = GitCometAssets::default()
            .list("")
            .expect("list root assets");
        let paths: BTreeSet<&str> = listed.iter().map(|path| path.as_ref()).collect();

        assert_eq!(paths.len(), listed.len(), "duplicate root asset paths");
        assert_eq!(
            paths,
            expected_asset_paths().collect(),
            "root listing must contain every file without directory placeholders"
        );
    }

    #[test]
    fn gpui_registry_preserves_every_asset() {
        // Exercise the same conversion as Application::with_assets. Checking
        // GitCometAssets::load alone misses paths omitted from the root listing.
        let registry = AssetRegistry::from(GitCometAssets::default());
        for path in expected_asset_paths() {
            let expected = GitCometAssets::default()
                .load(path)
                .expect("load embedded asset")
                .unwrap_or_else(|| panic!("missing embedded asset: {path}"));
            let actual = registry
                .load(path)
                .unwrap_or_else(|| panic!("GPUI registry cannot load {path}"));
            assert!(!actual.is_empty(), "empty asset: {path}");
            assert_eq!(actual, expected, "GPUI changed asset bytes: {path}");
        }
        for path in ["icons", "icons/file_icons", "icons/does-not-exist.svg"] {
            assert!(registry.load(path).is_none(), "unexpected asset: {path}");
        }
    }

    #[test]
    fn registered_svgs_render_visible_pixels() {
        let registry = Arc::new(AssetRegistry::from(GitCometAssets::default()));
        let renderer = SvgRenderer::new(Arc::clone(&registry));
        for path in expected_asset_paths().filter(|path| path.ends_with(".svg")) {
            let bytes = registry
                .load(path)
                .unwrap_or_else(|| panic!("GPUI registry cannot load {path}"));
            let parsed = renderer
                .parse_svg(&bytes)
                .unwrap_or_else(|err| panic!("cannot parse {path}: {err}"));
            for edge in [16, 32] {
                let dimensions = size(DevicePixels(edge), DevicePixels(edge));
                let image = renderer
                    .render_parsed(&parsed, SvgSize::ExactSize(dimensions))
                    .unwrap_or_else(|err| panic!("cannot render {path} at {edge}px: {err}"));
                assert_eq!(image.size(0), dimensions, "incorrect size: {path}");
                let pixels = image.as_bytes(0).expect("rendered SVG frame");
                assert!(
                    pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0),
                    "{path} renders blank at {edge}px"
                );
            }
        }
    }

    #[test]
    fn icons_are_the_kit_icon_set_and_artwork_has_neutral_paths() {
        let listed: BTreeSet<String> = GitCometAssets::list_static("icons")
            .into_iter()
            .map(|path| path.to_string())
            .collect();
        let kit: BTreeSet<String> = gitcomet_ui_kit::assets::KitAssets::list_static("icons")
            .into_iter()
            .map(|path| path.to_string())
            .collect();
        assert_eq!(listed, kit);
        let root = GitCometAssets::default()
            .list("")
            .expect("list root assets");
        for path in &root {
            assert!(!path.contains("gitcomet"), "{path} names the product");
        }
        let brand = GitCometAssets::default().list("brand").expect("list brand");
        assert_eq!(brand.len(), BRAND_ASSETS.len());
    }

    #[test]
    fn the_window_icon_decodes() {
        let icon = super::window_icon().expect("the window icon decodes");
        assert!(icon.width() > 0 && icon.height() > 0);
    }
}
