use std::path::Path;

fn main() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    windows_bindgen::builder()
        .output(manifest_dir.join("../../src/bindings.rs"))
        .filter_file(manifest_dir.join("bindings.txt"))
        .flat()
        .sys()
        .write();
}
