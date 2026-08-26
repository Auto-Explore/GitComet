//! This crate provides Swift language support for the [tree-sitter][] parsing library.
//!
//! Typically, you will use the [language][language func] function to add this language to a
//! tree-sitter [Parser][], and then use the parser to parse some code:
//!
//! ```
//! let code = r#"
//! "#;
//! let mut parser = tree_sitter::Parser::new();
//! let language = tree_sitter_swift::LANGUAGE;
//! parser
//!     .set_language(&language.into())
//!     .expect("Error loading Swift parser");
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
    fn tree_sitter_swift() -> *const ();
}

/// The tree-sitter [`LanguageFn`] for this grammar.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_swift) };

/// The content of the [`node-types.json`][] file for this grammar.
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers#static-node-types
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");

// GitComet: these consts are guarded so the crate still builds with `queries/`
// unvendored. `vendor/tree-sitter-build` declares and never sets the cfgs, so they
// compile out; GitComet keeps its own queries under
// crates/gitcomet-ui-gpui/src/view/rows/diff_text/queries/.
#[cfg(with_highlights_query)]
pub const HIGHLIGHTS_QUERY: &str = include_str!("../../queries/highlights.scm");
#[cfg(with_injections_query)]
pub const INJECTIONS_QUERY: &str = include_str!("../../queries/injections.scm");
#[cfg(with_locals_query)]
pub const LOCALS_QUERY: &str = include_str!("../../queries/locals.scm");
#[cfg(with_tags_query)]
pub const TAGS_QUERY: &str = include_str!("../../queries/tags.scm");

#[cfg(test)]
mod tests {
    #[test]
    fn test_can_load_grammar() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("Error loading Swift parser");
    }

    #[test]
    fn test_can_parse_basic_file() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("Error loading Swift parser");

        let tree = parser
            .parse("_ = \"Hello!\"\n", None)
            .expect("Unable to parse!");

        assert_eq!(
            "(source_file (assignment target: (directly_assignable_expression (simple_identifier)) result: (line_string_literal text: (line_str_text))))",
            tree.root_node().to_sexp(),
        );
    }
}
