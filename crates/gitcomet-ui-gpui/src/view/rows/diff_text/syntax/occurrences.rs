//! Every place in a document that names the same thing as the token under a
//! click.
//!
//! There is no language server here, so this cannot tell a definition from a
//! use the way an LSP `documentHighlight` does. What it can do is exact: the
//! tree says which byte range is one token and what kind it is, so a match is
//! accepted only where the grammar also tokenised it -- never inside a string,
//! a comment, or the middle of a longer word.

use std::ops::Range;

/// Ceiling for the occurrence scan.
///
/// The scan is O(document), and the editor asks on every caret move, so a large
/// buffer must not pay for it per keystroke. The diff side is bounded by the
/// same number rather than by the prepared document's own 8 MB ceiling: a click
/// is cheaper than a keystroke but not free, and it runs on the UI thread too.
pub(in crate::view) const OCCURRENCE_MAX_TEXT_BYTES: usize = 256 * 1024;

/// The most matches worth reporting for one click.
///
/// A very common identifier in a large file can occur thousands of times, and
/// past a point the highlight stops meaning "here is where this is used" and
/// starts meaning "this file is full of colour". The cap also bounds the work
/// the paint path does per row.
const MAX_OCCURRENCES: usize = 512;

/// The most candidates worth examining for one click.
///
/// [`MAX_OCCURRENCES`] alone bounds only what is *accepted*. A name that occurs
/// forty thousand times inside string literals passes the cheap byte test every
/// time, pays a root-to-leaf tree descent, and is then rejected -- so the cheap
/// cap never trips and the scan runs the whole document for no results. This
/// bounds the descents themselves.
const MAX_OCCURRENCE_CANDIDATES: usize = 4_096;

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
///
/// `.` and `-` are allowed because this only ever sees a *whole leaf token's*
/// text, never a slice of a larger construct -- so a dot here is one the grammar
/// itself put inside a single token, not the field access in `foo.bar` (which is
/// three nodes, and whose leaves have no dot). Assembly is what forces it: GAS
/// directives are `.section`, `.align`, `.p2align`, and arm64 condition suffixes
/// make `b.eq`, so every one of them was rejected before reaching the tree. The
/// same widening is what lets an Ansible key like `ansible.builtin.copy` or
/// `on-failure` resolve. A leading `.` is allowed, a leading `-` is not: `-5` is
/// a number literal in most grammars, where `.text` is a name in all of them.
fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_' || first == '.') {
        return false;
    }
    if !text
        .chars()
        .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '.' | '-'))
    {
        return false;
    }
    // At least one letter, so `...` and `.-.` are not names.
    text.chars().any(char::is_alphabetic)
}

/// Whether `node` is a single grammar token that names something.
///
/// `child_count() == 0` is what makes this a token rather than a construct: a
/// `string` or a `call_expression` has children, an `identifier` does not. The
/// kind check then drops the leaf nodes that are content rather than names --
/// comment bodies and string contents, which are single tokens too.
fn is_name_token(node: &tree_sitter::Node<'_>, text: &str) -> bool {
    is_name_token_kind(node) && text.get(node.byte_range()).is_some_and(is_name)
}

/// Leaf kinds whose name contains `string` but which are not string content.
///
/// The substring test in [`is_name_token_kind`] is a heuristic over how ~60
/// grammars name their nodes, and it is right nearly everywhere. Where it
/// misfires is a grammar that says "string" to mean "a bare word with no quotes
/// round it" -- which is exactly the thing a name is:
///
/// * `string_scalar` (`tree-sitter-yaml`) is the *unquoted* plain scalar, so
///   every mapping key in a playbook -- `hosts:`, `become_user:`,
///   `ansible.builtin.copy:` -- read as string content.
/// * `unquoted_string` (`tree-sitter-containerfile`) is the name and the value of
///   every `ARG` and `ENV` pair, so no `ARG BASE_TAG` in any Dockerfile resolved.
///
/// Both kinds are unique to their own grammar, so listing them here is
/// unambiguous. The genuinely quoted kinds beside them (`double_quote_scalar`,
/// `single_quote_scalar`) stay excluded, because those really are strings.
///
/// This list has grown twice now. If it reaches a third or fourth entry, the
/// substring test has stopped paying for itself and the kind should come from
/// the highlight capture instead -- a `@property` or `@variable` capture already
/// says "name" without guessing from a node's spelling.
const NAME_TOKEN_KINDS_DESPITE_SUBSTRING: &[&str] = &["string_scalar", "unquoted_string"];

/// The half of [`is_name_token`] that needs no text, so a caller can ask it
/// before deciding whether the text is worth materializing.
fn is_name_token_kind(node: &tree_sitter::Node<'_>) -> bool {
    if !node.is_named() || node.child_count() != 0 {
        return false;
    }
    let kind = node.kind();
    if NAME_TOKEN_KINDS_DESPITE_SUBSTRING.contains(&kind) {
        return true;
    }
    !(kind.contains("comment") || kind.contains("string") || kind.contains("char"))
}

/// The name token at `offset`, read through `token_text` so a caller whose text
/// is expensive to materialize pays only for that one token's bytes.
///
/// Split out because the editor asks on every caret move, and most of those land
/// on punctuation, whitespace or a literal. Answering "not a name" here costs a
/// tree descent; answering it after flattening the document costs the document.
pub(in crate::view) fn name_token_at(
    tree: &tree_sitter::Tree,
    offset: usize,
    token_text: impl Fn(Range<usize>) -> Option<String>,
) -> Option<Range<usize>> {
    let root = tree.root_node();
    let limit = root.end_byte();
    // A caret sits between two characters, so it touches the token on either
    // side; the one to the right wins, matching where a caret is drawn.
    [Some(offset), offset.checked_sub(1)]
        .into_iter()
        .flatten()
        .filter(|probe| *probe < limit)
        .find_map(|probe| {
            let node = root.named_descendant_for_byte_range(probe, probe)?;
            if !is_name_token_kind(&node) {
                return None;
            }
            let range = node.byte_range();
            token_text(range.clone())
                .filter(|text| is_name(text))
                .map(|_| range)
        })
}

/// The occurrences of the name at `offset`, or `None` if the click did not land
/// on one.
pub(in crate::view) fn syntax_occurrences_in_tree(
    tree: &tree_sitter::Tree,
    text: &str,
    offset: usize,
) -> Option<SyntaxOccurrences> {
    let root = tree.root_node();
    let token = name_token_at(tree, offset, |range| {
        text.get(range).map(std::borrow::ToOwned::to_owned)
    })?;

    let name = text.get(token.clone())?;
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut candidates = 0usize;
    let mut search_from = 0usize;
    // `name.len()`, not one byte: a word-bounded name cannot overlap itself, and
    // stepping a single byte lands *inside* the leading character of a name like
    // `café`, where the slice below fails and the `?` throws away every match
    // found so far -- including the clicked one.
    while let Some(found) = text.get(search_from..).and_then(|rest| rest.find(name)) {
        let start = search_from + found;
        let end = start + name.len();
        search_from = end;

        // Word boundaries first: they are a byte comparison, where the tree
        // lookup below walks the depth of the document.
        //
        // Only where the name's *own* edge is a word byte, though. A boundary
        // test asks "is this hit the whole word, or the tail of a longer one",
        // and that question is meaningless when the name begins with punctuation:
        // `.eq` in `b.eq` is preceded by `b`, so the plain test rejected the only
        // occurrence there is. The exact-leaf check below is what actually
        // guarantees correctness; this is a prefilter, and a prefilter that
        // cannot apply must let the candidate through rather than drop it.
        let name_bytes = name.as_bytes();
        let before_ok = !name_bytes.first().copied().is_some_and(is_word_byte)
            || start == 0
            || !is_word_byte(bytes[start - 1]);
        let after_ok = !name_bytes.last().copied().is_some_and(is_word_byte)
            || end >= bytes.len()
            || !is_word_byte(bytes[end]);
        if !before_ok || !after_ok {
            continue;
        }
        // Counted *after* the boundary test, because the budget exists to bound
        // the descents and only a word-bounded hit pays for one. Counting raw
        // substring hits instead spends the whole budget on `uuid` and `valid`
        // and then stops before the real uses of `id`.
        candidates += 1;
        if candidates > MAX_OCCURRENCE_CANDIDATES {
            break;
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
