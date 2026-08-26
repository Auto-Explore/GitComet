//! This crate provides OCaml language support for the [tree-sitter][] parsing
//! library. There are separate languages for implementation, (`.ml`),
//! interfaces (`.mli`) and types.
//!
//! Typically, you will use [`LANGUAGE_OCAML`][LANGUAGE_OCAML] to add this language
//! to a tree-sitter [`Parser`][Parser], and then use the parser to parse some code:
//!
//! ```
//! let code = r#"
//!   module M = struct
//!     let x = 0
//!   end
//! "#;
//! let mut parser = tree_sitter::Parser::new();
//! let language = tree_sitter_ocaml::LANGUAGE_OCAML;
//! parser
//!     .set_language(&language.into())
//!     .expect("Error loading OCaml language");
//! let tree = parser.parse(code, None).unwrap();
//! assert!(!tree.root_node().has_error());
//! ```
//!
//! Use [`LANGUAGE_OCAML_INTERFACE`][LANGUAGE_OCAML_INTERFACE] to parse interface
//! files (with `.mli` extension) and [`LANGUAGE_OCAML_TYPE`][LANGUAGE_OCAML_TYPE]
//! to parse type signatures.
//!
//! [LANGUAGE_OCAML]: constant.LANGUAGE_OCAML.html
//! [LANGUAGE_OCAML_INTERFACE]: constant.LANGUAGE_OCAML_INTERFACE.html
//! [LANGUAGE_OCAML_TYPE]: constant.LANGUAGE_OCAML_TYPE.html
//! [Parser]: https://docs.rs/tree-sitter/*/tree_sitter/struct.Parser.html
//! [tree-sitter]: https://tree-sitter.github.io/

use tree_sitter_language::LanguageFn;

unsafe extern "C" {
    fn tree_sitter_ocaml() -> *const ();
    fn tree_sitter_ocaml_interface() -> *const ();
    fn tree_sitter_ocaml_type() -> *const ();
}

/// The tree-sitter [`LanguageFn`][LanguageFn] for OCaml.
///
/// [LanguageFn]: https://docs.rs/tree-sitter-language/*/tree_sitter_language/struct.LanguageFn.html
pub const LANGUAGE_OCAML: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_ocaml) };

/// The tree-sitter [`LanguageFn`] for OCaml interfaces.
///
/// [LanguageFn]: https://docs.rs/tree-sitter-language/*/tree_sitter_language/struct.LanguageFn.html
pub const LANGUAGE_OCAML_INTERFACE: LanguageFn =
    unsafe { LanguageFn::from_raw(tree_sitter_ocaml_interface) };

/// The tree-sitter [`LanguageFn`] for OCaml types.
///
/// [LanguageFn]: https://docs.rs/tree-sitter-language/*/tree_sitter_language/struct.LanguageFn.html
pub const LANGUAGE_OCAML_TYPE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_ocaml_type) };

/// The content of the [`node-types.json`][] file for OCaml.
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers/6-static-node-types.html
pub const OCAML_NODE_TYPES: &str = include_str!("../../grammars/ocaml/src/node-types.json");

/// The content of the [`node-types.json`][] file for OCaml interfaces.
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers/6-static-node-types.html
pub const INTERFACE_NODE_TYPES: &str = include_str!("../../grammars/interface/src/node-types.json");

/// The content of the [`node-types.json`][] file for OCaml types.
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers/6-static-node-types.html
pub const TYPE_NODE_TYPES: &str = include_str!("../../grammars/type/src/node-types.json");

#[cfg(with_highlights_query)]
/// The syntax highlighting query for OCaml.
pub const HIGHLIGHTS_QUERY: &str = include_str!("../../queries/highlights.scm");

#[cfg(with_locals_query)]
/// The local variable query for OCaml.
pub const LOCALS_QUERY: &str = include_str!("../../queries/locals.scm");

#[cfg(with_tags_query)]
/// The symbol tagging query for OCaml.
pub const TAGS_QUERY: &str = include_str!("../../queries/tags.scm");

#[cfg(test)]
mod tests {
    #[test]
    fn test_ocaml() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE_OCAML.into())
            .expect("Error loading OCaml parser");

        let code = r#"
            module M = struct
              let x = 0
            end
        "#;

        let tree = parser.parse(code, None).unwrap();
        let root = tree.root_node();
        assert!(!root.has_error());
    }

    #[test]
    fn test_ocaml_interface() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE_OCAML_INTERFACE.into())
            .expect("Error loading OCaml interface parser");

        let code = r#"
            module M : sig
              val x : int
            end
        "#;

        let tree = parser.parse(code, None).unwrap();
        let root = tree.root_node();
        assert!(!root.has_error());
    }

    #[test]
    fn test_ocaml_type() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE_OCAML_TYPE.into())
            .expect("Error loading OCaml type parser");

        let code = r#"int list"#;

        let tree = parser.parse(code, None).unwrap();
        let root = tree.root_node();
        assert!(!root.has_error());
    }
}
