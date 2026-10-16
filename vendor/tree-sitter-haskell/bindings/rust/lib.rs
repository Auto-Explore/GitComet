//! This crate provides Haskell language support for the [tree-sitter][] parsing library.
//!
//! Typically, you will use the [language][language func] function to add this language to a
//! tree-sitter [Parser][], and then use the parser to parse some code:
//!
//! ```
//! use tree_sitter::Parser;
//!
//! let code = r#"
//! main = putStrLn "Hello World!"
//! "#;
//! let mut parser = Parser::new();
//! let language = tree_sitter_haskell::LANGUAGE;
//! parser
//!     .set_language(&language.into())
//!     .expect("Error loading Haskell parser");
//! let tree = parser.parse(code, None).unwrap();
//! assert!(!tree.root_node().has_error());
//! ```
//!
//! [Language]: https://docs.rs/tree-sitter/*/tree_sitter/struct.Language.html
//! [language func]: fn.language.html
//! [Parser]: https://docs.rs/tree-sitter/*/tree_sitter/struct.Parser.html
//! [tree-sitter]: https://tree-sitter.github.io/

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_haskell() -> *const ();
}

/// The tree-sitter [`LanguageFn`] for this grammar.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_haskell) };

/// The content of the [`node-types.json`][] file for this grammar.
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers#static-node-types
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");

// GitComet: `queries/` is vendored because GitComet reads these consts (see
// Cargo.toml); `vendor/tree-sitter-build` sets each `with_*_query` cfg for the `.scm`
// files present.
#[cfg(with_highlights_query)]
/// The syntax highlighting query for this language.
pub const HIGHLIGHTS_QUERY: &str = include_str!("../../queries/highlights.scm");

#[cfg(with_injections_query)]
/// The syntax highlighting query for languages injected into this one.
pub const INJECTIONS_QUERY: &str = include_str!("../../queries/injections.scm");

#[cfg(with_locals_query)]
/// The local-variable syntax highlighting query for this language.
pub const LOCALS_QUERY: &str = include_str!("../../queries/locals.scm");

#[cfg(test)]
mod tests {
    #[test]
    fn test_can_load_grammar() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("Error loading Haskell parser");
    }
}
