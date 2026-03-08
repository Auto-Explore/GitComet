#[cfg(target_os = "windows")]
fn main() {
    println!("cargo:rerun-if-changed=windows/gitcomet.rc");
    println!("cargo:rerun-if-changed=../../assets/gitcomet.ico");
    let _ = embed_resource::compile("windows/gitcomet.rc", embed_resource::NONE);
}

#[cfg(not(target_os = "windows"))]
fn main() {}
