//! Every vendored grammar is compiled identically; the body lives in
//! `vendor/tree-sitter-build` so there is one copy rather than one per grammar.
fn main() {
    tree_sitter_build::compile();
}
