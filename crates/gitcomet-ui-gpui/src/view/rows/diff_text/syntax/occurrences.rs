//! Every place in a document that names the same thing as the token under a
//! click.
//!
//! There is no language server here, so this cannot tell a definition from a
//! use the way an LSP `documentHighlight` does. What it can do is exact: the
//! tree says which byte range is one token and what kind it is, so a match is
//! accepted only where the grammar also tokenised it -- never inside a string,
//! a comment, or the middle of a longer word.

use std::ops::Range;

/// The most matches worth reporting for one click.
///
/// A very common identifier in a large file can occur thousands of times, and
/// past a point the highlight stops meaning "here is where this is used" and
/// starts meaning "this file is full of colour". The cap also bounds the work
/// the paint path does per row.
const MAX_OCCURRENCES: usize = 512;

/// The token a click landed on, and everywhere else the document names it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct SyntaxOccurrences {
    /// The token under the click. Always also present in `ranges`.
    pub(in crate::view) token: Range<usize>,
    /// Every occurrence, in document order, including `token`.
    pub(in crate::view) ranges: Vec<Range<usize>>,
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// Whether `text` reads as a name rather than a literal or punctuation.
///
/// Leading digits are excluded on purpose: a grammar tokenises `1` as its own
/// node, and lighting every `1` in a file is noise, not information.
fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    text.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
}

/// Whether `node` is a single grammar token that names something.
///
/// `child_count() == 0` is what makes this a token rather than a construct: a
/// `string` or a `call_expression` has children, an `identifier` does not. The
/// kind check then drops the leaf nodes that are content rather than names --
/// comment bodies and string contents, which are single tokens too.
fn is_name_token(node: &tree_sitter::Node<'_>, text: &str) -> bool {
    if !node.is_named() || node.child_count() != 0 {
        return false;
    }
    let kind = node.kind();
    if kind.contains("comment") || kind.contains("string") || kind.contains("char") {
        return false;
    }
    text.get(node.byte_range()).is_some_and(is_name)
}

/// The occurrences of the name at `offset`, or `None` if the click did not land
/// on one.
pub(in crate::view) fn syntax_occurrences_in_tree(
    tree: &tree_sitter::Tree,
    text: &str,
    offset: usize,
) -> Option<SyntaxOccurrences> {
    let root = tree.root_node();
    let limit = root.end_byte().min(text.len());

    // A caret sits between two characters, so it touches the token on either
    // side; the one to the right wins, matching where a caret is drawn.
    let token = [Some(offset), offset.checked_sub(1)]
        .into_iter()
        .flatten()
        .filter(|probe| *probe < limit)
        .find_map(|probe| {
            let node = root.named_descendant_for_byte_range(probe, probe)?;
            is_name_token(&node, text).then(|| node.byte_range())
        })?;

    let name = text.get(token.clone())?;
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut search_from = 0usize;
    while let Some(found) = text.get(search_from..)?.find(name) {
        let start = search_from + found;
        let end = start + name.len();
        search_from = start + 1;

        // Word boundaries first: they are a byte comparison, where the tree
        // lookup below walks the depth of the document.
        let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_word_byte(bytes[end]);
        if !before_ok || !after_ok {
            continue;
        }
        // And the grammar has to agree this span is one whole name token -- the
        // same test the clicked token had to pass. Exact range alone is not
        // enough: a string's content is a leaf that starts and ends exactly at
        // the quoted text, so `"total"` would otherwise match `total`.
        let Some(node) = root.descendant_for_byte_range(start, end) else {
            continue;
        };
        if node.start_byte() != start || node.end_byte() != end || !is_name_token(&node, text) {
            continue;
        }
        ranges.push(start..end);
        if ranges.len() >= MAX_OCCURRENCES {
            break;
        }
    }

    // The clicked token itself always counts, even if the scan somehow missed
    // it -- an answer that omits what was clicked would read as a bug.
    if !ranges.contains(&token) {
        ranges.push(token.clone());
        ranges.sort_by_key(|range| range.start);
    }
    Some(SyntaxOccurrences { token, ranges })
}
