//! This crate provides HTML language support for the [tree-sitter] parsing library.
//!
//! [tree-sitter]: https://tree-sitter.github.io/

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_html() -> *const ();
}

/// The tree-sitter [`LanguageFn`] for this grammar.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_html) };

/// The content of the [`node-types.json`][] file for this grammar.
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers/6-static-node-types
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");

#[cfg(with_highlights_query)]
/// The syntax highlighting query for this grammar.
pub const HIGHLIGHTS_QUERY: &str = include_str!("../../queries/highlights.scm");

#[cfg(test)]
mod tests {
    #[test]
    fn qualified_component_names_are_single_tag_names() {
        let source = "<RazorCorpus.Components.SamplePanel Title=\"literal\"></RazorCorpus.Components.SamplePanel>";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(!tree.root_node().has_error());
        let element = tree.root_node().named_child(0).unwrap();
        for tag_index in [0, 1] {
            let tag = element.named_child(tag_index).unwrap();
            let name = tag.named_child(0).unwrap();
            assert_eq!(name.kind(), "tag_name");
            assert_eq!(
                &source[name.byte_range()],
                "RazorCorpus.Components.SamplePanel"
            );
        }
    }

    #[test]
    fn test_can_load_grammar() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("Error loading HTML parser");
    }
}
