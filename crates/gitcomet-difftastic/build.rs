fn main() {
    let pin: serde_json::Value = serde_json::from_slice(include_bytes!("upstream.json"))
        .expect("the checked upstream pin must be valid JSON");
    let version = pin["version"]
        .as_str()
        .expect("upstream pin needs a version");
    println!("cargo:rustc-env=DIFFTASTIC_VERSION={version}");
    println!("cargo:rerun-if-changed=upstream.json");
}
