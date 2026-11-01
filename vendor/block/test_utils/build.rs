extern crate cc;

fn main() {
    cc::Build::new()
        .compiler("clang")
        .file("block_utils.c")
        .flag("-fblocks")
        .compile("block_utils");
}
