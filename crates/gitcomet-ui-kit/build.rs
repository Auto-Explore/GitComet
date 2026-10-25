mod build_themes;

use std::env;
use std::fs;
use std::path::PathBuf;

/// Generate a `<fn_name>` lookup and `<list_name>` list embedding every
/// `*.svg` in `dir` via `include_bytes!` under the `<asset_prefix>` asset
/// path, so the icons are served by the asset registry in `assets.rs`
/// without hand-maintaining the entries.
fn generate_svg_dir_assets(
    dir: &std::path::Path,
    asset_prefix: &str,
    fn_name: &str,
    list_name: &str,
) -> String {
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read svg asset dir")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            name.ends_with(".svg").then_some(name)
        })
        .collect();
    names.sort();

    let mut generated =
        format!("fn {fn_name}(path: &str) -> Option<&'static [u8]> {{\n    match path {{\n");
    for name in &names {
        let abs = dir.join(name);
        generated.push_str(&format!(
            "        \"{asset_prefix}/{name}\" => Some(include_bytes!({abs:?}).as_slice()),\n"
        ));
    }
    generated.push_str(&format!(
        "        _ => None,\n    }}\n}}\n\nconst {list_name}: &[&str] = &[\n"
    ));
    for name in &names {
        generated.push_str(&format!("    \"{asset_prefix}/{name}\",\n"));
    }
    generated.push_str("];\n");
    generated
}

fn main() {
    build_themes::generate_embedded_theme_registry();
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR missing"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR missing"));
    let icons_dir = manifest_dir.join("assets/icons");
    let mut generated = generate_svg_dir_assets(&icons_dir, "icons", "icon_bytes", "ICON_ASSETS");
    generated.push('\n');
    generated.push_str(&generate_svg_dir_assets(
        &icons_dir.join("file_icons"),
        "icons/file_icons",
        "file_icon_bytes",
        "FILE_ICON_ASSETS",
    ));
    fs::write(out_dir.join("icons_assets.rs"), generated).expect("write icons_assets.rs");
}
