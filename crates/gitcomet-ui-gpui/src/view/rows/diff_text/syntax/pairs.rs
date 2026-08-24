//! Matching open/close pair lookup, shared by both syntax engines.
//!
//! Read straight off the tree-sitter tree rather than from `brackets.scm`
//! queries. Every such query is a list of `("(" @open ")" @close)` patterns --
//! the delimiters are sibling children of one node -- so the tree already
//! carries the fact, and taking it from there works for every wired grammar
//! without a query file each. It is also exact where a scanner is not: a brace
//! inside a string or comment is a leaf *of* that string or comment, never a
//! delimiter, so it correctly matches nothing.
//!
//! Three kinds of pair are recognised, and the innermost one wins. See
//! [`BRACKET_PAIRS`], [`TAG_PAIRS`] and [`QUOTE_MARKER_PAIRS`] for what each
//! covers and what it deliberately does not.

use std::ops::Range;

/// Which kind of construct a matched pair delimits.
///
/// All three are painted with the same theme colour; the kind exists to rank
/// candidates against each other and to let tests say what they mean.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum SyntaxPairKind {
    Bracket,
    Tag,
    Quote,
}

/// A matched open/close pair, in document byte coordinates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct SyntaxPair {
    pub(in crate::view) open: Range<usize>,
    pub(in crate::view) close: Range<usize>,
    pub(in crate::view) kind: SyntaxPairKind,
}

impl SyntaxPair {
    fn new(open: Range<usize>, close: Range<usize>, kind: SyntaxPairKind) -> Self {
        Self { open, close, kind }
    }

    /// How far the pair reaches, for picking the innermost of several.
    fn span(&self) -> usize {
        self.close.end.saturating_sub(self.open.start)
    }
}

/// Bracket delimiters, matched as anonymous token siblings.
///
/// Angle brackets are deliberately absent: `<` and `>` are comparison operators
/// in most grammars and only sometimes delimiters, so pairing them lights up
/// arithmetic. Tag angle brackets are covered by [`TAG_PAIRS`] instead, which
/// keys on the element node and so cannot make that mistake.
const BRACKET_PAIRS: [(&str, &str); 8] = [
    ("(", ")"),
    ("[", "]"),
    ("{", "}"),
    // PowerShell spells its subexpression and array openers as their own
    // anonymous tokens, both ending at a plain `)`. Several opens against one
    // close is why the close side is matched with [`closes_open`] rather than
    // by looking up a single counterpart.
    ("$(", ")"),
    ("@(", ")"),
    // Some grammars wrap their delimiters in named nodes instead of exposing the
    // punctuation as anonymous siblings, which puts the `{` a level too deep for
    // the sibling scan to see. HCL is the in-tree example: a `block`'s children
    // are `block_start`, `body`, `block_end`. Naming the wrappers here matches
    // them at the level they actually live at, and since each wrapper spans just
    // its delimiter the highlight still covers only the brace.
    ("block_start", "block_end"),
    ("object_start", "object_end"),
    ("tuple_start", "tuple_end"),
];

/// Element tag pairs, by node kind.
///
/// Unlike [`BRACKET_PAIRS`] these are *named* nodes spanning a whole tag,
/// attributes included, so a matched pair covers `<div class="x">` and
/// `</div>` in full.
///
/// Kind names are grammar-scoped, so one flat table needs no per-language
/// dispatch and a new grammar is one line. Verified against each grammar's
/// `node-types.json`:
///
/// - `tree-sitter-html`, the vendored `tree-sitter-vue` and
///   `tree-sitter-svelte-ng` all use `element` / `script_element` /
///   `style_element` / `template_element` with `start_tag` and `end_tag`.
/// - `tree-sitter-xml` uses `element` with `STag` and `ETag`.
/// - `tree-sitter-typescript` (tsx) uses `jsx_element` with
///   `jsx_opening_element` and `jsx_closing_element`.
///
/// Self-closing tags (`self_closing_tag`, `jsx_self_closing_element`) have no
/// partner and are absent by construction. So is `erroneous_end_tag`: an
/// unclosed or mismatched tag matches nothing rather than pairing with the
/// wrong element, and marking the mismatch is a separate feature.
const TAG_PAIRS: [(&str, &str); 3] = [
    ("start_tag", "end_tag"),                       // html, vue, svelte
    ("STag", "ETag"),                               // xml
    ("jsx_opening_element", "jsx_closing_element"), // jsx, tsx
];

/// String delimiters that a grammar exposes as explicit *named* marker nodes,
/// as Python does with `(string (string_start) (string_content) (string_end))`.
///
/// Grammars that instead flank the content with identical anonymous quote
/// tokens are handled by [`QUOTE_CHARS`].
const QUOTE_MARKER_PAIRS: [(&str, &str); 2] = [
    ("string_start", "string_end"),
    // HCL wraps its quotes the same way it wraps its braces.
    ("quoted_template_start", "quoted_template_end"),
];

/// Quote characters that appear as identical anonymous siblings flanking a
/// string's content, as in JSON's `(string "\"" (string_content) "\"")` and
/// Rust's `(string_literal "\"" ... "\"")`.
///
/// Open and close are the same token here, so these cannot be depth-counted
/// like a bracket; they are paired by position among their parent's children
/// of that kind. A lone `'` -- a Rust lifetime -- has no partner under its
/// parent and so matches nothing.
///
/// Two documented gaps follow from matching only the bare quote token:
///
/// - Rust raw strings (`r#"` / `"#`) never pair, and cannot: tree-sitter-rust
///   emits their delimiters as *hidden* external tokens, so they are absent from
///   the tree entirely.
/// - C/C++ prefixed literals (`L"`, `u"`, `U"`, `u8"`) never pair, because the
///   grammar leaves the prefix on the opening token instead of aliasing it back
///   to `"` the way tree-sitter-rust does for `b"`/`c"`. Plain `"..."` in the
///   same file does pair, so the behaviour is per-literal.
const QUOTE_CHARS: [&str; 3] = ["\"", "'", "`"];

/// What part a node plays in a pair, if any.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PairRole {
    Open(SyntaxPairKind),
    Close(SyntaxPairKind),
    /// Open and close are the same token, so which end this is depends on where
    /// it sits among its siblings.
    Ambiguous(SyntaxPairKind),
}

fn pair_role(node: &tree_sitter::Node<'_>) -> Option<PairRole> {
    let kind = node.kind();
    if node.is_named() {
        return table_role(kind, SyntaxPairKind::Tag)
            .or_else(|| table_role(kind, SyntaxPairKind::Quote))
            .or_else(|| table_role(kind, SyntaxPairKind::Bracket));
    }
    if QUOTE_CHARS.contains(&kind) {
        return Some(PairRole::Ambiguous(SyntaxPairKind::Quote));
    }
    table_role(kind, SyntaxPairKind::Bracket)
}

fn table_role(kind: &str, pair: SyntaxPairKind) -> Option<PairRole> {
    table_for(pair).iter().find_map(|(open, close)| {
        if *open == kind {
            Some(PairRole::Open(pair))
        } else if *close == kind {
            Some(PairRole::Close(pair))
        } else {
            None
        }
    })
}

/// The open/close table a pair kind is declared in. The single place the three
/// tables are related to their kinds, so a fourth kind is one arm here.
fn table_for(pair: SyntaxPairKind) -> &'static [(&'static str, &'static str)] {
    match pair {
        SyntaxPairKind::Bracket => &BRACKET_PAIRS,
        SyntaxPairKind::Tag => &TAG_PAIRS,
        SyntaxPairKind::Quote => &QUOTE_MARKER_PAIRS,
    }
}

/// The closing counterpart of an opening delimiter kind.
fn counterpart_of_open(kind: &str, pair: SyntaxPairKind) -> Option<&'static str> {
    table_for(pair)
        .iter()
        .find(|(open, _)| *open == kind)
        .map(|(_, close)| *close)
}

/// Whether `open_kind` is an opening delimiter that `close_kind` closes.
///
/// A predicate rather than a lookup because the relation is many-to-one: a
/// grammar can spell several opens against one close (PowerShell's `(`, `$(`
/// and `@(` all end at `)`), and taking only the table's first match would
/// leave every other one of them unpaired.
fn closes_open(open_kind: &str, close_kind: &str, pair: SyntaxPairKind) -> bool {
    table_for(pair)
        .iter()
        .any(|(open, close)| *open == open_kind && *close == close_kind)
}

/// Whether `node` is a leaf delimiter a caret can sit directly on, as opposed
/// to a named node that merely spans one.
///
/// Tags are excluded on purpose: a caret inside `<div class="x">` is not
/// sitting *on* a delimiter, it is inside one, which the outward walk in
/// [`syntax_pair_in_tree`] handles instead.
fn is_delimiter_token(node: &tree_sitter::Node<'_>) -> bool {
    !node.is_named() && pair_role(node).is_some()
}

/// The delimiter node a click on `node` actually means.
///
/// Some grammars wrap a delimiter in a named node of its own -- HCL's
/// `tuple_start` is a `[` and nothing else -- and it is the *wrapper* that
/// [`BRACKET_PAIRS`] names, because that is the level its partner lives at. A
/// click lands on the anonymous token one level below, where the sibling scan
/// finds no partner and gives up. Rising through wrappers that span exactly the
/// same bytes puts the search back where the table expects it.
///
/// The equal-span test is what keeps this from over-reaching: a `(` whose
/// parent is an `arguments` node spanning the whole call stops immediately, so
/// no click is ever promoted to a pair wider than the delimiter it landed on.
fn delimiter_node_for_click(node: tree_sitter::Node<'_>) -> tree_sitter::Node<'_> {
    let mut node = node;
    while let Some(parent) = node.parent() {
        if parent.byte_range() != node.byte_range() || pair_role(&parent).is_none() {
            break;
        }
        node = parent;
    }
    node
}

/// The pair `offset` belongs to: the delimiter it sits on (or immediately
/// after), otherwise the innermost pair enclosing it.
///
/// O(tree depth) -- cheap enough to run on the caret-move path.
pub(in crate::view) fn syntax_pair_in_tree(
    tree: &tree_sitter::Tree,
    offset: usize,
) -> Option<SyntaxPair> {
    let root = tree.root_node();
    let limit = root.end_byte();

    // A caret sits *between* two characters, so it touches the delimiter on
    // either side. The one to the right wins, matching where the caret is drawn.
    for probe in [Some(offset), offset.checked_sub(1)].into_iter().flatten() {
        if probe >= limit {
            continue;
        }
        if let Some(node) = root.descendant_for_byte_range(probe, probe + 1)
            && is_delimiter_token(&node)
            && let Some(pair) = partner_of_delimiter(delimiter_node_for_click(node))
        {
            return Some(pair);
        }
    }

    // Otherwise the caret is inside something: walk outward and take the first
    // node that brackets it, which is the innermost enclosing pair. A node that
    // is itself a delimiter answers on the way out -- that is how a caret
    // anywhere in `<div class="x">` reaches the element's tag pair.
    let mut node = root.descendant_for_byte_range(offset.min(limit), offset.min(limit))?;
    loop {
        if let Some(pair) = enclosing_pair_among_children(&node, offset) {
            return Some(pair);
        }
        if pair_role(&node).is_some()
            && let Some(pair) = partner_of_delimiter(node)
        {
            return Some(pair);
        }
        node = node.parent()?;
    }
}

/// The delimiter matching `node`, found among its parent's direct children.
///
/// Nesting is counted rather than assuming one pair per parent: a node can hold
/// several same-kind pairs side by side (`(a)(b)` as arguments), and a deeper
/// pair lives under a deeper node, so counting direct children is exact.
fn partner_of_delimiter(node: tree_sitter::Node<'_>) -> Option<SyntaxPair> {
    let parent = node.parent()?;
    let role = pair_role(&node)?;
    let kind = node.kind();
    let mut cursor = parent.walk();
    let children: Vec<tree_sitter::Node<'_>> = parent.children(&mut cursor).collect();
    let index = children.iter().position(|child| child.id() == node.id())?;

    match role {
        PairRole::Ambiguous(pair) => {
            // Open and close are the same token, so position decides: among the
            // parent's children of this kind, they pair up two by two.
            let same: Vec<&tree_sitter::Node<'_>> = children
                .iter()
                .filter(|child| !child.is_named() && child.kind() == kind)
                .collect();
            let position = same.iter().position(|child| child.id() == node.id())?;
            let (open, close) = if position % 2 == 0 {
                (same.get(position)?, same.get(position + 1)?)
            } else {
                (same.get(position - 1)?, same.get(position)?)
            };
            let (open, close) = (open.byte_range(), close.byte_range());
            delimits_whole_node(&parent, &open, &close).then(|| SyntaxPair::new(open, close, pair))
        }
        PairRole::Open(pair) => {
            let close = counterpart_of_open(kind, pair)?;
            let mut depth = 1usize;
            for child in &children[index + 1..] {
                if child.kind() == kind {
                    depth += 1;
                } else if child.kind() == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(SyntaxPair::new(node.byte_range(), child.byte_range(), pair));
                    }
                }
            }
            None
        }
        PairRole::Close(pair) => {
            let mut depth = 1usize;
            for child in children[..index].iter().rev() {
                if child.kind() == kind {
                    depth += 1;
                } else if closes_open(child.kind(), kind, pair) {
                    depth -= 1;
                    if depth == 0 {
                        return Some(SyntaxPair::new(child.byte_range(), node.byte_range(), pair));
                    }
                }
            }
            None
        }
    }
}

/// The tightest delimiter pair among `node`'s direct children that contains
/// `offset` strictly between them.
fn enclosing_pair_among_children(
    node: &tree_sitter::Node<'_>,
    offset: usize,
) -> Option<SyntaxPair> {
    let mut best: Option<SyntaxPair> = None;
    let mut consider = |candidate: SyntaxPair| {
        if candidate.open.end > offset || candidate.close.start < offset {
            return;
        }
        if best
            .as_ref()
            .is_none_or(|current| candidate.span() < current.span())
        {
            best = Some(candidate);
        }
    };

    // Distinct open/close delimiters: a stack, so nesting resolves innermost
    // first regardless of how many pairs share this parent.
    let mut open_stack: Vec<(tree_sitter::Node<'_>, SyntaxPairKind)> = Vec::new();
    // Same-token delimiters cannot be stacked -- both ends are the same token --
    // so they are paired positionally instead. Only the *first* of a kind can
    // open a pair `delimits_whole_node` will accept, since that guard demands
    // the open start at the node's first byte, so one slot per kind is enough
    // and the children need only one pass.
    let mut quote_open: [Option<tree_sitter::Node<'_>>; QUOTE_CHARS.len()] =
        [None; QUOTE_CHARS.len()];
    let mut quote_settled = [false; QUOTE_CHARS.len()];

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named()
            && let Some(slot) = QUOTE_CHARS.iter().position(|quote| *quote == child.kind())
        {
            if quote_settled[slot] {
                continue;
            }
            match quote_open[slot] {
                None => quote_open[slot] = Some(child),
                Some(open) => {
                    quote_settled[slot] = true;
                    let (open, close) = (open.byte_range(), child.byte_range());
                    if delimits_whole_node(node, &open, &close) {
                        consider(SyntaxPair::new(open, close, SyntaxPairKind::Quote));
                    }
                }
            }
            continue;
        }
        match pair_role(&child) {
            Some(PairRole::Open(pair)) => open_stack.push((child, pair)),
            Some(PairRole::Close(pair)) => {
                let Some(position) = open_stack
                    .iter()
                    .rposition(|(open, _)| closes_open(open.kind(), child.kind(), pair))
                else {
                    continue;
                };
                let (open, _) = open_stack[position];
                // Truncate rather than remove: every opener still above the
                // match is one this close skipped past, so it can no longer
                // pair with anything to the right without crossing this
                // delimiter. Leaving them on the stack is how `( [ ) ]` -- an
                // ERROR node, or a grammar that flattens a malformed construct
                // -- produced a `[ ... ]` pair spanning the `)`.
                open_stack.truncate(position);
                consider(SyntaxPair::new(open.byte_range(), child.byte_range(), pair));
            }
            _ => {}
        }
    }

    best
}

/// Whether `open` and `close` are the outermost bytes of `node`.
///
/// The test that a same-token pair is real: two quotes that begin and end their
/// parent *are* that parent, which is what a string literal is. It needs no
/// allowlist of string node kinds, and it rejects the pairing a parent that
/// merely happens to contain two quote tokens would otherwise produce.
fn delimits_whole_node(
    node: &tree_sitter::Node<'_>,
    open: &Range<usize>,
    close: &Range<usize>,
) -> bool {
    node.start_byte() == open.start && node.end_byte() == close.end
}

/// The tab width the row canvases expand to.
use crate::view::diff_utils::DIFF_TEXT_TAB_WIDTH as DISPLAY_TAB_WIDTH;

/// Convert an offset in a row's *display* text back to an offset in the raw
/// line.
///
/// The row canvases expand every tab to [`DISPLAY_TAB_WIDTH`] spaces before the
/// text is shaped (`diff_text_full_line_for_region`), so a click offset counts
/// expanded columns while the tree-sitter tree indexes real bytes. Without this
/// every pair in a tab-indented file lands three columns per indent level off.
///
/// An offset landing inside an expanded tab resolves to that tab.
pub(in crate::view) fn raw_offset_for_display_offset(line: &str, display_offset: usize) -> usize {
    if !line.contains('\t') {
        return display_offset.min(line.len());
    }
    let mut display = 0usize;
    for (raw, ch) in line.char_indices() {
        let width = if ch == '\t' {
            DISPLAY_TAB_WIDTH
        } else {
            ch.len_utf8()
        };
        // The character whose display span covers the offset -- `>=` on the span
        // *start* would hand back the following character for any offset landing
        // inside a four-column tab.
        if display + width > display_offset {
            return raw;
        }
        display += width;
    }
    line.len()
}

/// The raw offset a *click* at `display_offset` landed on, or `None` when the
/// click fell past the line's last character.
///
/// The row hitbox spans the full width of the pane, not the width of the text,
/// and it clamps a point past the end of the line to the line's last column. A
/// caret belongs there, but a highlight does not: without this, clicking the
/// blank area to the right of `let sum = total;` resolves to the byte after the
/// `;` and the caret-adjacency probe one byte to its left then washes the whole
/// file's uses of the last name on the line.
pub(in crate::view) fn clicked_raw_offset_for_display_offset(
    line: &str,
    display_offset: usize,
) -> Option<usize> {
    (display_offset < crate::view::diff_utils::diff_text_display_len(line))
        .then(|| raw_offset_for_display_offset(line, display_offset))
}

/// Convert an offset in a raw line to the display column the canvas painted it
/// at -- the inverse of [`raw_offset_for_display_offset`].
pub(in crate::view) fn display_offset_for_raw_offset(line: &str, raw_offset: usize) -> usize {
    if !line.contains('\t') {
        return raw_offset.min(line.len());
    }
    let mut display = 0usize;
    for (raw, ch) in line.char_indices() {
        if raw >= raw_offset {
            return display;
        }
        display += if ch == '\t' {
            DISPLAY_TAB_WIDTH
        } else {
            ch.len_utf8()
        };
    }
    // Past the last character: the line's whole display width, which is what
    // the canvas measured when it painted the row.
    crate::view::diff_utils::diff_text_display_len(line)
}
