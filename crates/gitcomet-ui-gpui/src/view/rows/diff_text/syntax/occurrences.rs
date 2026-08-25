//! Every place in a document that names the same thing as the token under a
//! click.
//!
//! There is no language server here, so this cannot tell a definition from a
//! use the way an LSP `documentHighlight` does. What it can do is exact: the
//! tree says which byte range is one token and what kind it is, so a match is
//! accepted only where the grammar also tokenised it -- never inside a string,
//! a comment, or the middle of a longer word.

use std::ops::Range;

use crate::kit::rope::Rope;

/// Interactive names and pairs are available whenever full-document syntax is.
///
/// Keeping this as an alias, rather than another literal, makes the capability
/// boundary impossible to drift: if a document has a prepared/live tree, a
/// click can use it. The editor scans its rope without flattening it, and cold
/// diff documents are completed off-thread, so matching the larger ceiling does
/// not put an 8 MiB allocation or parse on the input path.
pub(in crate::view) const OCCURRENCE_MAX_TEXT_BYTES: usize =
    super::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES;

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
    // A leading variable sigil counts, but only in front of a real name: `$name`
    // and `@list` are what Perl calls a variable, and its grammar puts the sigil
    // *inside* the token, so without this no Perl variable was ever clickable.
    // Requiring a letter or `_` after it keeps `$`, `%` and a bare `@` out.
    //
    // Note this can only ever connect uses that share a sigil. Perl writes the
    // same hash `%hash`, `$hash{k}` and `@hash{...}` depending on what is being
    // taken from it, and those are different text; matching them would need to
    // know the language, which this does not.
    let sigil = matches!(first, '$' | '@' | '%')
        && chars
            .clone()
            .next()
            .is_some_and(|next| next.is_alphabetic() || next == '_');
    if !(first.is_alphabetic() || first == '_' || first == '.' || sigil) {
        return false;
    }
    if !text
        .chars()
        .skip(usize::from(sigil))
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
/// ...and leaf kinds the grammar models as a node with an anonymous child.
///
/// [`is_name_token_kind`] uses `child_count() == 0` to mean "this is a token
/// rather than a construct", which is right for the ~60 grammars whose names are
/// bare tokens. Perl's are not: `scalar_variable` is `seq('$', name)`, so the
/// sigil is an anonymous child and the node fails that test even though it is
/// exactly one name. No Perl variable was clickable because of it.
///
/// Checked for `named_child_count() == 0` instead, so a real construct -- which
/// always has named children -- still cannot slip through.
const NAME_TOKEN_KINDS_WITH_ANONYMOUS_PARTS: &[&str] =
    &["array_variable", "hash_variable", "scalar_variable"];

const NAME_TOKEN_KINDS_DESPITE_SUBSTRING: &[&str] = &["string_scalar", "unquoted_string"];

/// The half of [`is_name_token`] that needs no text, so a caller can ask it
/// before deciding whether the text is worth materializing.
fn is_name_token_kind(node: &tree_sitter::Node<'_>) -> bool {
    if !node.is_named() {
        return false;
    }
    let kind = node.kind();
    if NAME_TOKEN_KINDS_WITH_ANONYMOUS_PARTS.contains(&kind) {
        return node.named_child_count() == 0;
    }
    if node.child_count() != 0 {
        return false;
    }
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
    let token = name_token_at(tree, offset, |range| {
        text.get(range).map(std::borrow::ToOwned::to_owned)
    })?;

    let name = text.get(token.clone())?;
    let bytes = text.as_bytes();
    let mut candidates = Vec::new();
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
        candidates.push(start..end);
        if candidates.len() >= MAX_OCCURRENCE_CANDIDATES {
            break;
        }
    }

    Some(syntax_occurrences_from_candidates(tree, token, candidates))
}

/// The occurrences lookup used by the live editor, without materializing the
/// persistent rope into one document-sized `String`.
pub(in crate::view) fn syntax_occurrences_in_rope(
    tree: &tree_sitter::Tree,
    rope: &Rope,
    offset: usize,
) -> Option<SyntaxOccurrences> {
    let token = name_token_at(tree, offset, |range| Some(rope.text_for_range(range)))?;
    let name = rope.text_for_range(token.clone());
    let candidates = rope_name_candidates(rope, &name);
    Some(syntax_occurrences_from_candidates(tree, token, candidates))
}

/// Search a chunked rope as one byte stream.
///
/// KMP carries a partial match across chunk seams, while the ring holds the one
/// preceding byte needed by the cheap word-boundary filter. A completed match
/// is held for one byte so its trailing boundary can be decided. The result is
/// therefore identical to the contiguous scan, including names split between
/// two 512-byte rope leaves, without copying the document.
fn rope_name_candidates(rope: &Rope, name: &str) -> Vec<Range<usize>> {
    let needle = name.as_bytes();
    if needle.is_empty() {
        return Vec::new();
    }

    let mut failure = vec![0usize; needle.len()];
    for ix in 1..needle.len() {
        let mut prefix = failure[ix - 1];
        while prefix > 0 && needle[ix] != needle[prefix] {
            prefix = failure[prefix - 1];
        }
        if needle[ix] == needle[prefix] {
            prefix += 1;
        }
        failure[ix] = prefix;
    }

    let needs_before_boundary = needle.first().copied().is_some_and(is_word_byte);
    let needs_after_boundary = needle.last().copied().is_some_and(is_word_byte);
    let mut recent = vec![0u8; needle.len().saturating_add(1)];
    let mut matched = 0usize;
    let mut offset = 0usize;
    let mut pending: Option<Range<usize>> = None;
    let mut out = Vec::new();

    for byte in rope.chunks().flat_map(|chunk| chunk.bytes()) {
        if let Some(candidate) = pending.take()
            && (!needs_after_boundary || !is_word_byte(byte))
        {
            out.push(candidate);
            if out.len() >= MAX_OCCURRENCE_CANDIDATES {
                return out;
            }
        }

        while matched > 0 && byte != needle[matched] {
            matched = failure[matched - 1];
        }
        if byte == needle[matched] {
            matched += 1;
        }
        if matched == needle.len() {
            let start = offset + 1 - needle.len();
            let before_ok = !needs_before_boundary
                || start == 0
                || !is_word_byte(recent[(start - 1) % recent.len()]);
            if before_ok {
                pending = Some(start..offset + 1);
            }
            // The contiguous implementation advances by the whole name. A
            // word-bounded name cannot have a valid overlapping occurrence,
            // and discarding overlaps prevents hostile repeated bytes from
            // spending the candidate budget before a real name is reached.
            matched = 0;
        }

        let recent_ix = offset % recent.len();
        recent[recent_ix] = byte;
        offset += 1;
    }

    if let Some(candidate) = pending {
        out.push(candidate);
    }
    out
}

fn syntax_occurrences_from_candidates(
    tree: &tree_sitter::Tree,
    token: Range<usize>,
    candidates: impl IntoIterator<Item = Range<usize>>,
) -> SyntaxOccurrences {
    let root = tree.root_node();
    let mut ranges = Vec::new();
    for candidate in candidates.into_iter().take(MAX_OCCURRENCE_CANDIDATES) {
        let Some(node) = root.descendant_for_byte_range(candidate.start, candidate.end) else {
            continue;
        };
        // The grammar has to agree this span is one whole name token. Exact
        // range alone is not enough: string content can also be one leaf with
        // exactly the quoted body's range.
        if node.start_byte() != candidate.start
            || node.end_byte() != candidate.end
            || !is_name_token_kind(&node)
        {
            continue;
        }
        ranges.push(candidate);
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
    SyntaxOccurrences { token, ranges }
}
