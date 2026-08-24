//! This crate provides Objective-C language support for the [tree-sitter][] parsing library.
//!
//! Typically, you will use the [LANGUAGE][] constant to add this language to a
//! tree-sitter [Parser][], and then use the parser to parse some code:
//!
//! ```
//! let code = r#"
//! @interface MyClass : NSObject
//! @property (nonatomic, strong) NSString *name;
//! - (void)printName;
//! @end
//! "#;
//! let mut parser = tree_sitter::Parser::new();
//! let language = tree_sitter_objc::LANGUAGE;
//! parser
//!     .set_language(&language.into())
//!     .expect("Error loading Objective-C parser");
//! let tree = parser.parse(code, None).unwrap();
//! assert!(!tree.root_node().has_error());
//! ```
//!
//! [Parser]: https://docs.rs/tree-sitter/*/tree_sitter/struct.Parser.html
//! [tree-sitter]: https://tree-sitter.github.io/

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_objc() -> *const ();
}

/// The tree-sitter [`LanguageFn`][LanguageFn] for this grammar.
///
/// [LanguageFn]: https://docs.rs/tree-sitter-language/*/tree_sitter_language/struct.LanguageFn.html
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_objc) };

/// The content of the [`node-types.json`][] file for this grammar.
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers#static-node-types
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");

// GitComet: these consts are guarded so the crate still builds with `queries/`
// unvendored. `vendor/tree-sitter-build` declares and never sets the cfgs, so they
// compile out; GitComet keeps its own queries under
// crates/gitcomet-ui-gpui/src/view/rows/diff_text/queries/.
#[cfg(with_folds_query)]
/// The folds query for this language.
pub const FOLDS_QUERY: &str = include_str!("../../queries/folds.scm");

#[cfg(with_highlights_query)]
/// The syntax highlighting query for this language.
pub const HIGHLIGHTS_QUERY: &str = include_str!("../../queries/highlights.scm");

#[cfg(with_indents_query)]
/// The indents query for this language.
pub const INDENTS_QUERY: &str = include_str!("../../queries/indents.scm");

#[cfg(with_injections_query)]
/// The injection query for this language.
pub const INJECTIONS_QUERY: &str = include_str!("../../queries/injections.scm");

#[cfg(with_locals_query)]
/// The symbol tagging query for this language.
pub const LOCALS_QUERY: &str = include_str!("../../queries/locals.scm");

#[cfg(test)]
mod tests {
    #[test]
    fn test_can_load_grammar() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("Error loading Objective-C parser");
    }
}
