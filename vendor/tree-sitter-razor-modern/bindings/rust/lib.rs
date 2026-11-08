use tree_sitter_language::LanguageFn;

unsafe extern "C" {
    fn tree_sitter_razor() -> *const ();
}

pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_razor) };
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");

#[cfg(test)]
mod tests;
