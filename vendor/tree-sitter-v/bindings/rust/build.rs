//! The published 0.0.4 build script hard-codes GNU compiler and archiver
//! conventions. The shared helper delegates both jobs to `cc`, including MSVC.
fn main() {
    tree_sitter_build::compile();
}
