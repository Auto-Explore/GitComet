//! This crate provides Sql language support for the [tree-sitter][] parsing library.
//!
//! Typically, you will use the [LANGUAGE][] constant to add this language to a
//! tree-sitter [Parser][], and then use the parser to parse some code:
//!
//! ```
//! let code = r#"
//! "#;
//! let mut parser = tree_sitter::Parser::new();
//! let language = tree_sitter_sequel::LANGUAGE;
//! parser
//!     .set_language(&language.into())
//!     .expect("Error loading Sql parser");
//! let tree = parser.parse(code, None).unwrap();
//! assert!(!tree.root_node().has_error());
//! ```
//!
//! [Parser]: https://docs.rs/tree-sitter/*/tree_sitter/struct.Parser.html
//! [tree-sitter]: https://tree-sitter.github.io/

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_sql() -> *const ();
}

/// The tree-sitter [`LanguageFn`][LanguageFn] for this grammar.
///
/// [LanguageFn]: https://docs.rs/tree-sitter-language/*/tree_sitter_language/struct.LanguageFn.html
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_sql) };

/// The content of the [`node-types.json`][] file for this grammar.
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers#static-node-types
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");

// NOTE: uncomment these to include any queries that this grammar contains:

// GitComet: these consts are guarded so the crate still builds with `queries/`
// unvendored. `vendor/tree-sitter-build` declares and never sets the cfgs, so they
// compile out; GitComet keeps its own queries under
// crates/gitcomet-ui-gpui/src/view/rows/diff_text/queries/.
#[cfg(with_highlights_query)]
pub const HIGHLIGHTS_QUERY: &str = include_str!("../../queries/highlights.scm");
// pub const INJECTIONS_QUERY: &str = include_str!("../../queries/injections.scm");
// pub const LOCALS_QUERY: &str = include_str!("../../queries/locals.scm");
// pub const TAGS_QUERY: &str = include_str!("../../queries/tags.scm");

#[cfg(test)]
mod tests {
    #[test]
    fn test_can_load_grammar() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("Error loading Sql parser");
    }
}

#[cfg(test)]
mod scanner_tests {
    #[test]
    fn scanner_serialization_preserves_the_active_tag() {
        use std::ffi::c_void;
        unsafe extern "C" {
            fn tree_sitter_sql_external_scanner_create() -> *mut c_void;
            fn tree_sitter_sql_external_scanner_destroy(payload: *mut c_void);
            fn tree_sitter_sql_external_scanner_serialize(
                payload: *mut c_void,
                buffer: *mut u8,
            ) -> u32;
            fn tree_sitter_sql_external_scanner_deserialize(
                payload: *mut c_void,
                buffer: *const u8,
                length: u32,
            );
        }
        // Serializing must be read-only: another branch may serialize the same
        // scanner before restoring it. Upstream used to free the tag on this call.
        let tag = b"$body$\0";
        let mut first = [0; 1024];
        let mut second = first;
        let (first_len, second_len) = unsafe {
            let scanner = tree_sitter_sql_external_scanner_create();
            tree_sitter_sql_external_scanner_deserialize(scanner, tag.as_ptr(), tag.len() as u32);
            let first_len = tree_sitter_sql_external_scanner_serialize(scanner, first.as_mut_ptr());
            let second_len =
                tree_sitter_sql_external_scanner_serialize(scanner, second.as_mut_ptr());
            tree_sitter_sql_external_scanner_deserialize(scanner, std::ptr::null(), 0);
            tree_sitter_sql_external_scanner_destroy(scanner);
            (first_len as usize, second_len as usize)
        };
        assert_eq!(&first[..first_len], tag);
        assert_eq!(&second[..second_len], tag);
    }
}
