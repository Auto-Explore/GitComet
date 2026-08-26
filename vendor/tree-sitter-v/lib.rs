//! V language grammar for [tree-sitter](https://tree-sitter.github.io/tree-sitter/).
///
/// Maintained fork of [nedpals/tree-sitter-v](https://github.com/nedpals/tree-sitter-v).
use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_v() -> *const ();
}

/// Tree-sitter language for V.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_v) };

/// Static node types for this grammar.
pub const NODE_TYPES: &str = include_str!("src/node-types.json");

/// Highlight queries for V syntax.
pub const HIGHLIGHTS_QUERY: &str = include_str!("highlights.scm");

#[cfg(test)]
mod tests {
    #[test]
    fn test_can_load_grammar() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("Error loading V language");
    }
}
