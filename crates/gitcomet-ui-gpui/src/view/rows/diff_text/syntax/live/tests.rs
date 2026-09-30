use super::*;

fn line_starts_for(text: &str) -> Arc<[usize]> {
    let mut starts = vec![0usize];
    for (ix, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(ix + 1);
        }
    }
    if starts.last() == Some(&text.len()) && !text.is_empty() {
        starts.pop();
    }
    starts.into()
}

fn document(text: &str, mask: Vec<Range<usize>>) -> LiveSyntaxDocument {
    document_in(DiffSyntaxLanguage::Rust, text, mask)
}

pub(super) fn document_in(
    language: DiffSyntaxLanguage,
    text: &str,
    mask: Vec<Range<usize>>,
) -> LiveSyntaxDocument {
    LiveSyntaxDocument::new(language, Rope::from_text(text), mask.into(), None)
        .unwrap_or_else(|| panic!("{language:?} live document should build"))
}

pub(super) fn styles_at(
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
    offset: usize,
) -> Option<gpui::HighlightStyle> {
    highlights
        .iter()
        .find(|(range, _)| range.contains(&offset))
        .map(|(_, style)| *style)
}

#[test]
fn runs_are_sorted_disjoint_and_clipped() {
    let text = "fn main() {\n    let value = 1;\n}\n";
    let doc = document(text, Vec::new());
    let snapshot = doc.snapshot(AppTheme::gitcomet_dark());
    let highlights = snapshot.highlights_for_byte_range(0..text.len());

    assert!(!highlights.is_empty(), "rust source should highlight");
    let mut previous_end = 0usize;
    for (range, _) in &highlights {
        assert!(
            range.start >= previous_end,
            "runs must not overlap: {highlights:?}"
        );
        assert!(range.start < range.end, "runs must be non-empty");
        assert!(range.end <= text.len(), "runs must stay inside the text");
        previous_end = range.end;
    }
}

#[test]
fn window_query_matches_the_same_span_of_a_full_query() {
    let text = "fn main() {\n    let value = 1;\n    let other = 2;\n}\n";
    let doc = document(text, Vec::new());
    let snapshot = doc.snapshot(AppTheme::gitcomet_dark());

    let window = 12..47;
    let full = snapshot.highlights_for_byte_range(0..text.len());
    let windowed = snapshot.highlights_for_byte_range(window.clone());

    for offset in window.clone() {
        assert_eq!(
            styles_at(&windowed, offset),
            styles_at(&full, offset),
            "byte {offset} should style identically whether queried whole or windowed"
        );
    }
}

#[test]
fn occurrence_scan_crosses_rope_chunk_boundaries_without_flattening() {
    let padding = " ".repeat(crate::kit::rope::MAX_CHUNK_BYTES - 5);
    let text = format!("{padding}fn boundary_name() {{ boundary_name(); }}\n");
    let name_start = text.find("boundary_name").expect("fixture name");
    assert!(
        name_start < crate::kit::rope::MAX_CHUNK_BYTES
            && name_start + "boundary_name".len() > crate::kit::rope::MAX_CHUNK_BYTES,
        "the first name must straddle two rope chunks"
    );

    let doc = document(&text, Vec::new());
    let occurrences = doc
        .snapshot(AppTheme::gitcomet_dark())
        .occurrences_at(name_start + 1);
    assert_eq!(
        occurrences,
        vec![
            name_start..name_start + "boundary_name".len(),
            text.rfind("boundary_name").expect("fixture call")
                ..text.rfind("boundary_name").expect("fixture call") + "boundary_name".len(),
        ]
    );
}

#[test]
fn occurrences_share_the_full_document_syntax_ceiling() {
    assert_eq!(
        OCCURRENCE_MAX_TEXT_BYTES,
        PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES
    );
    let old_interactive_ceiling = 1024usize * 1024;
    let prefix = "fn shared_name() {}\n";
    let suffix = "fn caller() { shared_name(); }\n";
    let padding_line =
        "// syntax-sized live occurrence padding ..............................................\n";
    let padding = padding_line.repeat(
        old_interactive_ceiling
            .saturating_mul(2)
            .div_ceil(padding_line.len()),
    );
    let text = format!("{prefix}{padding}{suffix}");
    assert!(text.len() > old_interactive_ceiling);
    assert!(text.len() <= PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES);

    let first = text.find("shared_name").expect("fixture declaration");
    let second = text.rfind("shared_name").expect("fixture call");
    let doc = document(&text, Vec::new());
    assert_eq!(
        doc.snapshot(AppTheme::gitcomet_dark())
            .occurrences_at(first + 1),
        vec![
            first..first + "shared_name".len(),
            second..second + "shared_name".len(),
        ]
    );
}

#[test]
fn masked_placeholder_does_not_poison_following_lines() {
    // Same document twice: once with the placeholder row masked, once with
    // the row already replaced by spaces. Masking should make these agree.
    let with_placeholder = "fn a() {}\n<Merge Conflict>\nfn b() -> u32 { 7 }\n";
    let with_spaces = "fn a() {}\n                \nfn b() -> u32 { 7 }\n";
    assert_eq!(with_placeholder.len(), with_spaces.len());

    let placeholder_span = 10..26;
    assert_eq!(
        &with_placeholder[placeholder_span.clone()],
        "<Merge Conflict>"
    );

    let masked = document(with_placeholder, vec![placeholder_span]);
    let masked = masked.snapshot(AppTheme::gitcomet_dark());
    let spaced = document(with_spaces, Vec::new());
    let spaced = spaced.snapshot(AppTheme::gitcomet_dark());

    let tail = 27..with_placeholder.len();
    for offset in tail {
        assert_eq!(
            styles_at(
                &masked.highlights_for_byte_range(0..with_placeholder.len()),
                offset
            ),
            styles_at(
                &spaced.highlights_for_byte_range(0..with_spaces.len()),
                offset
            ),
            "byte {offset} after a masked placeholder should match the spaces-only parse"
        );
    }
}

#[test]
fn unmasked_placeholder_is_what_masking_protects_against() {
    // Guards the premise: without the mask the tail really does change.
    let text = "fn a() {}\n<Merge Conflict>\nfn b() -> u32 { 7 }\n";
    let masked = document(text, Vec::from([10..26usize; 1])).snapshot(AppTheme::gitcomet_dark());
    let unmasked = document(text, Vec::new()).snapshot(AppTheme::gitcomet_dark());

    let full = 0..text.len();
    assert_ne!(
        masked.highlights_for_byte_range(full.clone()),
        unmasked.highlights_for_byte_range(full),
        "masking should change the parse; if not, the fixture stopped exercising it"
    );
}

#[test]
fn edit_keeps_the_document_exact() {
    let before = "fn main() {\n    let value = 1;\n}\n";
    let mut doc = document(before, Vec::new());
    let first_version = doc.version();

    // Insert "let extra = 2;\n    " at the start of the body line.
    let inserted_text = "let extra = 2;\n    ";
    let after = "fn main() {\n    let extra = 2;\n    let value = 1;\n}\n";
    let at = 16usize;
    assert_eq!(&after[at..at + inserted_text.len()], inserted_text);

    let outcome = doc.sync(
        Rope::from_text(after),
        Vec::new().into(),
        Some((at..at, at..at + inserted_text.len())),
        None,
    );
    assert_eq!(outcome, LiveSyntaxSyncOutcome::Reparsed);
    assert!(
        doc.background_reparse_request().is_none(),
        "a document that finished its reparse has nothing to defer"
    );
    assert_ne!(
        doc.version(),
        first_version,
        "version must advance per edit"
    );

    let incremental = doc.snapshot(AppTheme::gitcomet_dark());
    let scratch = document(after, Vec::new()).snapshot(AppTheme::gitcomet_dark());
    assert_eq!(
        incremental.highlights_for_byte_range(0..after.len()),
        scratch.highlights_for_byte_range(0..after.len()),
        "an incrementally reparsed tree must match a cold parse of the same text"
    );
}

#[test]
fn an_edit_past_the_size_ceiling_abandons_the_document() {
    // `new` refuses to build over the ceiling, so an edit that crosses it
    // has to refuse too — otherwise one paste buys a full parse now and an
    // unbounded background reparse for the rest of the session.
    let before = "fn main() {}\n";
    let mut doc = document(before, Vec::new());
    let version_before = doc.version();

    let at = before.len();
    let padding =
        "// ".to_string() + &"x".repeat(PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES) + "\n";
    let after = format!("{before}{padding}");
    assert!(after.len() > PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES);
    assert!(
        LiveSyntaxDocument::new(
            DiffSyntaxLanguage::Rust,
            Rope::from_text(after.as_str()),
            Vec::new().into(),
            None,
        )
        .is_none(),
        "the fixture must be past the ceiling for this test to mean anything"
    );

    let outcome = doc.sync(
        Rope::from_text(after.as_str()),
        Vec::new().into(),
        Some((at..at, at..at + padding.len())),
        None,
    );

    assert_eq!(outcome, LiveSyntaxSyncOutcome::Abandoned);
    assert_eq!(
        doc.version(),
        version_before,
        "an abandoned sync must leave the document untouched"
    );
    assert!(
        doc.background_reparse_request().is_none(),
        "an abandoned document must not owe an unbounded background parse"
    );
}

#[test]
fn an_edit_that_outruns_the_budget_defers_and_the_background_pass_catches_up() {
    let before = "fn main() {\n    let value = 1;\n}\n".repeat(400);
    let mut doc = document(&before, Vec::new());

    let at = 11usize; // just inside the first body
    let inserted = "\n    let extra = 2;";
    let mut after = before.clone();
    after.insert_str(at, inserted);

    let outcome = doc.sync(
        Rope::from_text(after.as_str()),
        Vec::new().into(),
        Some((at..at, at..at + inserted.len())),
        Some(Duration::ZERO),
    );
    assert_eq!(
        outcome,
        LiveSyntaxSyncOutcome::Deferred,
        "a zero budget cannot finish a reparse"
    );

    // Deferred still renders: the edited tree moved with the edit, so the
    // provider has something positionally correct to paint right now.
    let deferred = doc.snapshot(AppTheme::gitcomet_dark());
    assert!(
        !deferred.highlights_for_byte_range(0..200).is_empty(),
        "a deferred document must keep painting rather than blank the viewport"
    );

    let request = doc
        .background_reparse_request()
        .expect("a deferred document owes a background reparse");
    let (version, tree, injections) =
        live_syntax_reparse(request).expect("unbudgeted reparse succeeds");
    assert!(
        doc.adopt_background_tree(version, tree, injections),
        "the version has not moved, so the tree should be adopted"
    );
    assert!(doc.background_reparse_request().is_none());

    let caught_up = doc.snapshot(AppTheme::gitcomet_dark());
    let scratch = document(&after, Vec::new()).snapshot(AppTheme::gitcomet_dark());
    assert_eq!(
        caught_up.highlights_for_byte_range(0..400),
        scratch.highlights_for_byte_range(0..400),
        "after the background pass the tree must match a cold parse"
    );
}

/// A wholesale replacement that outruns its budget must not keep the tree.
///
/// `Deferred` is sound only for a *seeded* sync: there `tree.edit()` has
/// moved the old tree into the new coordinates, so it still paints. With
/// `edit: None` nothing moved it, so keeping it pairs the new rope with a
/// tree describing text that is gone — and every query over it answers for
/// the wrong document. The file editor reached exactly that on a file
/// switch: the buffer is blanked, a document is built over the empty text,
/// then the file lands as a wholesale replacement whose budgeted parse
/// fails, leaving a full rope with a 0-byte tree and no highlighting at all.
#[test]
fn a_wholesale_replacement_that_outruns_the_budget_is_abandoned() {
    // The empty buffer the editor blanks to before a file lands.
    let mut doc = document("", Vec::new());
    assert!(
        doc.snapshot(AppTheme::gitcomet_dark())
            .0
            .tree
            .root_node()
            .end_byte()
            == 0,
        "the fixture must start with a tree that spans nothing"
    );

    let landed = "fn main() {\n    let value = 1;\n}\n".repeat(400);
    let outcome = doc.sync(
        Rope::from_text(landed.as_str()),
        Vec::new().into(),
        // `None` -- the text was replaced, not edited.
        None,
        Some(Duration::ZERO),
    );

    assert_eq!(
        outcome,
        LiveSyntaxSyncOutcome::Abandoned,
        "an unseeded sync that cannot parse must hand the document back, so \
             the caller falls back to heuristics and rebuilds off-thread"
    );
}

/// The same shape as the test above, but *seeded*: this one must stay
/// `Deferred`, because the tree really did move with the edit.
#[test]
fn a_seeded_edit_that_outruns_the_budget_still_defers() {
    let before = "fn main() {\n    let value = 1;\n}\n".repeat(400);
    let mut doc = document(&before, Vec::new());
    let inserted = "\n    let extra = 2;";
    let at = 11usize;
    let mut after = before.clone();
    after.insert_str(at, inserted);

    assert_eq!(
        doc.sync(
            Rope::from_text(after.as_str()),
            Vec::new().into(),
            Some((at..at, at..at + inserted.len())),
            Some(Duration::ZERO),
        ),
        LiveSyntaxSyncOutcome::Deferred
    );
}

#[test]
fn a_background_tree_for_a_superseded_version_is_rejected() {
    let before = "fn main() {}\n";
    let mut doc = document(before, Vec::new());
    let stale_version = doc.version();
    let tree = doc.snapshot(AppTheme::gitcomet_dark()).0.tree.clone();

    let after = "fn main() { let x = 1; }\n";
    doc.sync(Rope::from_text(after), Vec::new().into(), None, None);

    assert!(
        !doc.adopt_background_tree(stale_version, tree, Vec::new()),
        "a tree parsed for text that has since changed must be discarded"
    );
}

#[test]
fn wholesale_replacement_reparses_from_scratch() {
    let mut doc = document("fn main() {}\n", Vec::new());
    let after = "struct Point { x: u32, y: u32 }\n";

    let outcome = doc.sync(Rope::from_text(after), Vec::new().into(), None, None);
    assert_eq!(outcome, LiveSyntaxSyncOutcome::Reparsed);

    let replaced = doc.snapshot(AppTheme::gitcomet_dark());
    let scratch = document(after, Vec::new()).snapshot(AppTheme::gitcomet_dark());
    assert_eq!(
        replaced.highlights_for_byte_range(0..after.len()),
        scratch.highlights_for_byte_range(0..after.len()),
    );
}

#[test]
fn masked_read_serves_blanks_then_real_text() {
    let rope = Rope::from_text("abcdefghij");
    let mask = [2..5usize; 1];
    let mut read = masked_read(&rope, &mask);

    assert_eq!(read(0, tree_sitter::Point::new(0, 0)), b"ab");
    assert_eq!(read(2, tree_sitter::Point::new(0, 2)), b"   ");
    assert_eq!(read(5, tree_sitter::Point::new(0, 5)), b"fghij");
    assert_eq!(read(10, tree_sitter::Point::new(0, 10)), b"");
}

#[test]
fn masked_read_spans_longer_than_the_blank_buffer_are_served_in_pieces() {
    let rope = Rope::from_text(&"x".repeat(200));
    let mask = [0..200usize; 1];
    let mut read = masked_read(&rope, &mask);

    let mut served = 0usize;
    while served < 200 {
        let chunk = read(served, tree_sitter::Point::new(0, served));
        assert!(!chunk.is_empty(), "reader must always make progress");
        assert!(chunk.iter().all(|byte| *byte == b' '));
        served += chunk.len();
    }
    assert_eq!(served, 200);
}

#[test]
fn oversized_text_has_no_live_document() {
    let huge: Arc<str> =
        Arc::from("fn a() {}\n".repeat(PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES / 5));
    assert!(
        LiveSyntaxDocument::new(
            DiffSyntaxLanguage::Rust,
            Rope::from_text(&huge),
            Vec::new().into(),
            None,
        )
        .is_none(),
        "text past the ceiling should not get a live document"
    );
}

#[test]
fn language_without_a_grammar_has_no_live_document() {
    let text: Arc<str> = Arc::from("x = 1\n");
    assert!(
        LiveSyntaxDocument::new(
            DiffSyntaxLanguage::VisualBasic,
            Rope::from_text(&text),
            Vec::new().into(),
            None,
        )
        .is_none(),
        "a language with no wired grammar should fall back rather than build"
    );
}

/// The editable buffers — the merge tool's resolved output and the file
/// editor — and the read-only diff panes must colour the same code the same
/// way, and they do not share an engine: this one sweeps a `QueryCursor`
/// over the viewport ([`sweep_runs`]), the diff panes materialize per-line
/// tokens and resolve overlaps with `normalize_non_overlapping_tokens`. Hold
/// the tree constant and check the two derivations agree byte for byte.
///
/// `probes` are `(needle, expected kind)` pairs asserted against the live
/// side first, so a fixture that stopped being tree-sitter-highlighted
/// cannot make the comparison pass by leaving both sides empty. They are
/// also where each language's *precedence* is pinned: the divergence this
/// guards against is a query that colours a node by capturing its parent
/// afterwards, which only shows up as one kind rather than another.
///
/// Fixtures must avoid constructs that trigger `*_injections.scm`:
/// [`super::prepared`] is driven here with the root tree alone, while the
/// live snapshot merges its injected layers, so an injected region is a
/// known divergence rather than a regression.
pub(super) fn assert_engines_agree(
    language: DiffSyntaxLanguage,
    text: &str,
    probes: &[(&str, SyntaxTokenKind)],
) {
    // A fixture inside one rope chunk never exercises the chunked feed this
    // engine parses through (`masked_read` hands the parser one chunk at a
    // time, `prepared` hands it a contiguous slice), so it compares the two
    // engines on the one input where they cannot differ. Every fixture here
    // used to be under 512 bytes, which is why several rounds of "the
    // engines agree" said nothing about files the app actually opens.
    assert!(
        text.len() > crate::kit::rope::MAX_CHUNK_BYTES,
        "an equivalence fixture must span more than one rope chunk \
             ({} bytes); this one is {} bytes",
        crate::kit::rope::MAX_CHUNK_BYTES,
        text.len(),
    );
    let theme = AppTheme::gitcomet_dark();
    let palette = syntax_highlight_palette(theme);
    let snapshot = document_in(language, text, Vec::new()).snapshot(theme);

    let mut live_by_byte = vec![None; text.len()];
    for (range, style) in snapshot.highlights_for_byte_range(0..text.len()) {
        for byte in range {
            live_by_byte[byte] = Some(style);
        }
    }

    // Same tree, so any disagreement below is in how the captures are turned
    // into styles -- which is exactly what differs between the two engines.
    let line_starts = line_starts_for(text);
    let spec = tree_sitter_highlight_spec(language)
        .unwrap_or_else(|| panic!("{language:?} should have a wired grammar"));
    let per_line = collect_treesitter_document_line_tokens_for_line_window(
        &snapshot.0.tree,
        spec,
        text.as_bytes(),
        line_starts.as_ref(),
        0,
        line_starts.len(),
        treesitter_text_hash(text),
    );
    let mut prepared_by_byte = vec![None; text.len()];
    for (line_ix, tokens) in per_line.iter().enumerate() {
        let line_start = line_starts[line_ix];
        for token in tokens {
            let Some(style) = palette.style(token.kind) else {
                continue;
            };
            let span = (line_start + token.range.start)..(line_start + token.range.end);
            prepared_by_byte[span].fill(Some(style));
        }
    }

    for (needle, kind) in probes {
        let at = text.find(needle).expect("fixture should contain the probe");
        assert_eq!(
            live_by_byte[at],
            palette.style(*kind),
            "{language:?}: {needle:?} at {at} should carry {kind:?}; without these \
                 classes the comparison below cannot tell tree-sitter from the \
                 heuristic tokenizer"
        );
    }

    // Newlines are excluded: `prepared` clips every token to
    // `line_content_end_byte`, so a capture spanning a line break stops at
    // the `\n`, while a swept run carries straight through it. Invisible in
    // rendering -- a newline has no glyph -- and not worth reshaping either
    // engine over.
    let mismatched = (0..text.len())
        .filter(|byte| text.as_bytes()[*byte] != b'\n')
        .filter(|byte| live_by_byte[*byte] != prepared_by_byte[*byte])
        .map(|byte| {
            let line_ix = line_starts.partition_point(|start| *start <= byte) - 1;
            format!(
                "byte {byte} (line {line_ix}, {:?}): live={:?} prepared={:?}",
                text.as_bytes()[byte] as char,
                live_by_byte[byte].and_then(|style| style.color),
                prepared_by_byte[byte].and_then(|style| style.color),
            )
        })
        .collect::<Vec<_>>();
    assert!(
        mismatched.is_empty(),
        "{language:?}: the editable buffers and the diff panes must colour \
             identical text identically; diverging bytes:\n  {}",
        mismatched.join("\n  ")
    );
}

#[test]
fn the_live_engine_agrees_with_the_prepared_engine_the_diff_panes_use() {
    let text = concat!(
        "use std::fmt;\n",
        "\n",
        "/// A stage in the pipeline.\n",
        "pub struct Stage<'a> {\n",
        "    pub name: &'a str,\n",
        "    pub retries: usize,\n",
        "}\n",
        "\n",
        "impl Stage<'_> {\n",
        "    pub fn bump(&mut self) -> usize {\n",
        "        self.retries = self.retries.wrapping_add(1);\n",
        "        self.retries\n",
        "    }\n",
        "}\n",
    );

    // Repeated so the fixture spans several rope chunks: the chunked parser
    // feed is only exercised past 512 bytes.
    let text = text.repeat(6);
    let text = text.as_str();
    assert_engines_agree(
        DiffSyntaxLanguage::Rust,
        text,
        &[
            ("Stage<'a>", SyntaxTokenKind::Type),
            ("usize", SyntaxTokenKind::TypeBuiltin),
            ("retries: usize", SyntaxTokenKind::Property),
            ("wrapping_add", SyntaxTokenKind::FunctionMethod),
        ],
    );
}

/// The regression that made an entire `Cargo.toml` render in one colour.
///
/// `tree-sitter-toml-ng` colours keys by capturing the enclosing node —
/// `(bare_key) @type` first, then `(pair (bare_key)) @property` — so a
/// resolver that prefers the innermost capture gives every key `@type`,
/// which is the same green as `@string` in the shipped themes. The `@property`
/// probe below is what pins the precedence.
#[test]
fn the_two_engines_agree_on_toml_keys() {
    let text = concat!(
        "[package]\n",
        "name = \"gitcomet\"\n",
        "version = \"0.1.16\"\n",
        "edition = \"2024\"\n",
        "\n",
        "# A comment.\n",
        "[dependencies]\n",
        "serde = { version = \"1\", features = [\"derive\"] }\n",
        "retries = 3\n",
        "strict = true\n",
    );

    let text = text.repeat(6);
    let text = text.as_str();
    assert_engines_agree(
        DiffSyntaxLanguage::Toml,
        text,
        &[
            ("name", SyntaxTokenKind::Property),
            ("\"gitcomet\"", SyntaxTokenKind::String),
            ("# A comment.", SyntaxTokenKind::Comment),
            ("3", SyntaxTokenKind::Number),
            ("true", SyntaxTokenKind::Boolean),
        ],
    );
}

/// Python's `highlights.scm` uses the same capture-the-parent idiom for
/// f-string interpolations and docstrings.
#[test]
fn the_two_engines_agree_on_python() {
    let text = concat!(
        "import os\n",
        "\n",
        "\n",
        "class Stage:\n",
        "    \"\"\"A stage in the pipeline.\"\"\"\n",
        "\n",
        "    def __init__(self, name):\n",
        "        self.name = name\n",
        "        self.retries = 0\n",
        "\n",
        "    def bump(self):\n",
        "        self.retries += 1\n",
        "        return f\"{self.name}: {self.retries}\"\n",
    );

    let text = text.repeat(6);
    let text = text.as_str();
    assert_engines_agree(
        DiffSyntaxLanguage::Python,
        text,
        &[
            ("import", SyntaxTokenKind::Keyword),
            ("Stage", SyntaxTokenKind::Type),
            ("__init__", SyntaxTokenKind::FunctionSpecial),
            ("0", SyntaxTokenKind::Number),
        ],
    );
}

/// The shape of the file this was reported on: a shebang, a quoted heredoc,
/// a `case` block and `${var:-}` expansions, over several rope chunks.
///
/// Heredocs are the construct most likely to tell the two engines apart —
/// tree-sitter-bash matches the delimiter in an external scanner, which is
/// exactly the sort of thing that can read differently through a chunked
/// feed than through one contiguous slice.
#[test]
fn the_two_engines_agree_on_shell_with_heredocs() {
    let text = concat!(
        "#!/usr/bin/env bash\n",
        "set -euo pipefail\n",
        "\n",
        "usage() {\n",
        "  cat <<'USAGE'\n",
        "Usage: scripts/update.sh --dir PATH --version VERSION [--verify]\n",
        "USAGE\n",
        "}\n",
        "\n",
        "dir=\"\"\n",
        "version=\"\"\n",
        "verify=\"false\"\n",
        "\n",
        "while [[ $# -gt 0 ]]; do\n",
        "  case \"$1\" in\n",
        "    --dir) dir=\"${2:-}\"; shift 2 ;;\n",
        "    --version) version=\"${2:-}\"; shift 2 ;;\n",
        "    --verify) verify=\"true\"; shift ;;\n",
        "    *) echo \"unknown option: $1\" >&2; usage; exit 1 ;;\n",
        "  esac\n",
        "done\n",
        "\n",
        "if [[ -z \"$dir\" ]]; then\n",
        "  echo \"--dir is required\" >&2\n",
        "  exit 1\n",
        "fi\n",
    )
    .repeat(3);

    assert_engines_agree(
        DiffSyntaxLanguage::Bash,
        text.as_str(),
        &[
            ("while", SyntaxTokenKind::KeywordControl),
            ("esac", SyntaxTokenKind::KeywordControl),
            ("USAGE\n", SyntaxTokenKind::String),
        ],
    );
}

/// `(open, close)` as `&str` slices, so failures read as source text rather
/// than as byte offsets.
fn syntax_pair_text<'a>(
    document: &LiveSyntaxDocument,
    text: &'a str,
    offset: usize,
) -> Option<(&'a str, &'a str)> {
    document
        .snapshot(AppTheme::gitcomet_dark())
        .syntax_pair_at(offset)
        .map(|pair| (&text[pair.open], &text[pair.close]))
}

#[test]
fn syntax_pair_matches_a_delimiter_the_caret_sits_on() {
    let text = "fn main() {\n    let value = compute(1, 2);\n}\n";
    let document = document(text, Vec::new());

    let open_paren = text.find("(1").expect("call paren");
    assert_eq!(
        syntax_pair_text(&document, text, open_paren),
        Some(("(", ")")),
        "the caret on an opening paren must find its own closer"
    );

    // Caret immediately *after* the closer: an editor caret touches the
    // character to its left too.
    let close_paren = text.find(");").expect("call close");
    assert_eq!(
        syntax_pair_text(&document, text, close_paren + 1),
        Some(("(", ")"))
    );
}

#[test]
fn syntax_pair_matches_the_innermost_block_around_the_caret() {
    let text = "fn main() {\n    let value = compute(1, 2);\n}\n";
    let document = document(text, Vec::new());

    let inside_call = text.find("1, 2").expect("call args") + 2;
    assert_eq!(
        syntax_pair_text(&document, text, inside_call),
        Some(("(", ")")),
        "inside the argument list the call parens win over the block braces"
    );

    let inside_block = text.find("let").expect("statement");
    let pair = document
        .snapshot(AppTheme::gitcomet_dark())
        .syntax_pair_at(inside_block)
        .expect("the body braces enclose the statement");
    assert_eq!(
        (&text[pair.open.clone()], &text[pair.close.clone()]),
        ("{", "}")
    );
    assert_eq!(pair.kind, SyntaxPairKind::Bracket);
    assert_eq!(pair.open.start, text.find('{').expect("body open"));
}

#[test]
fn syntax_pair_ignores_braces_inside_strings_and_comments() {
    let text = "fn main() {\n    let s = \"a } b\";\n    // ) not a paren\n}\n";
    let document = document(text, Vec::new());

    // Sitting on the brace inside the string literal: it is a byte of the
    // string node, not a delimiter, so it pairs with nothing and the answer
    // comes from the quotes enclosing it instead.
    let in_string = text.find("} b").expect("brace in string");
    let pair = document
        .snapshot(AppTheme::gitcomet_dark())
        .syntax_pair_at(in_string)
        .expect("the string's own quotes enclose the brace");
    assert_eq!(pair.kind, SyntaxPairKind::Quote);
    assert_eq!(
        (&text[pair.open.clone()], &text[pair.close.clone()]),
        ("\"", "\"")
    );
    assert_eq!(pair.open.start, text.find('"').expect("string open"));

    let in_comment = text.find(") not").expect("paren in comment");
    let pair = document
        .snapshot(AppTheme::gitcomet_dark())
        .syntax_pair_at(in_comment)
        .expect("the function body still encloses the comment");
    assert_eq!(pair.kind, SyntaxPairKind::Bracket);
    assert_eq!(pair.open.start, text.find('{').expect("body open"));
}

#[test]
fn syntax_pair_distinguishes_sibling_pairs_of_the_same_kind() {
    let text = "fn main() {\n    f((1), (2));\n}\n";
    let document = document(text, Vec::new());

    let first_open = text.find("(1").expect("first inner");
    let first = document
        .snapshot(AppTheme::gitcomet_dark())
        .syntax_pair_at(first_open)
        .expect("first inner pair");
    let second_open = text.find("(2").expect("second inner");
    let second = document
        .snapshot(AppTheme::gitcomet_dark())
        .syntax_pair_at(second_open)
        .expect("second inner pair");

    assert_eq!(first.open.start, first_open);
    assert_eq!(second.open.start, second_open);
    assert_ne!(
        first.close, second.close,
        "two sibling pairs must not share a closer"
    );
}

#[test]
fn syntax_pair_is_correct_after_an_incremental_edit() {
    // The case the feature exists for: typing into the middle of the file
    // must not leave the pair pointing at pre-edit offsets.
    let before = "fn main() {\n    let value = 1;\n}\n";
    let mut document = document(before, Vec::new());

    let insert_at = before.find("let").expect("statement");
    let inserted = "if x { }\n    ";
    let after = format!(
        "{}{}{}",
        &before[..insert_at],
        inserted,
        &before[insert_at..]
    );
    let outcome = document.sync(
        Rope::from_text(&after),
        Arc::default(),
        Some((insert_at..insert_at, insert_at..insert_at + inserted.len())),
        None,
    );
    assert_eq!(outcome, LiveSyntaxSyncOutcome::Reparsed);

    let new_block_open = after.find("{ }").expect("new block");
    let pair = document
        .snapshot(AppTheme::gitcomet_dark())
        .syntax_pair_at(new_block_open)
        .expect("the freshly typed block must pair");
    assert_eq!(pair.open.start, new_block_open);
    assert_eq!(&after[pair.close.clone()], "}");

    // And the statement that moved down still resolves to the outer body.
    let moved_statement = after.rfind("let").expect("moved statement");
    let pair = document
        .snapshot(AppTheme::gitcomet_dark())
        .syntax_pair_at(moved_statement)
        .expect("the body still encloses the moved statement");
    assert_eq!(pair.open.start, after.find('{').expect("body open"));
}

/// `(open, close)` in `language`, as `&str` slices.
fn pair_text_in(language: DiffSyntaxLanguage, text: &str, offset: usize) -> Option<(&str, &str)> {
    let document = document_in(language, text, Vec::new());
    syntax_pair_text(&document, text, offset)
}

fn pair_kind_in(language: DiffSyntaxLanguage, text: &str, offset: usize) -> Option<SyntaxPairKind> {
    document_in(language, text, Vec::new())
        .snapshot(AppTheme::gitcomet_dark())
        .syntax_pair_at(offset)
        .map(|pair| pair.kind)
}

/// A tag pair covers both tags in full, attributes included -- not just the
/// angle brackets and not just the element name.
#[test]
fn syntax_pair_matches_whole_html_tags() {
    let text = "<div class=\"card\">\n  <span>hi</span>\n</div>\n";

    // Caret on the element name of the outer start tag.
    let on_outer_name = text.find("div").expect("outer name");
    assert_eq!(
        pair_text_in(DiffSyntaxLanguage::Html, text, on_outer_name),
        Some(("<div class=\"card\">", "</div>")),
        "the whole start tag pairs with the whole end tag"
    );
    assert_eq!(
        pair_kind_in(DiffSyntaxLanguage::Html, text, on_outer_name),
        Some(SyntaxPairKind::Tag)
    );

    // Caret in the inner element's text content takes the inner pair.
    let in_inner_text = text.find("hi").expect("inner text");
    assert_eq!(
        pair_text_in(DiffSyntaxLanguage::Html, text, in_inner_text),
        Some(("<span>", "</span>")),
        "the innermost element wins over the one enclosing it"
    );

    // Caret in the outer element's content, outside the inner element.
    let between = text.find("\n  <span").expect("gap before inner");
    assert_eq!(
        pair_text_in(DiffSyntaxLanguage::Html, text, between),
        Some(("<div class=\"card\">", "</div>"))
    );
}

/// The three tag naming schemes in the wired grammars all resolve, with no
/// per-language dispatch: `start_tag`/`end_tag`, `STag`/`ETag`, and JSX's
/// `jsx_opening_element`/`jsx_closing_element`.
#[test]
fn syntax_pair_matches_every_tag_naming_scheme() {
    let xml = "<root><item>x</item></root>\n";
    assert_eq!(
        pair_text_in(DiffSyntaxLanguage::Xml, xml, xml.find("item").expect("tag")),
        Some(("<item>", "</item>")),
        "xml names them STag and ETag"
    );

    let tsx = "const a = <Foo bar={1}>text</Foo>;\n";
    assert_eq!(
        pair_text_in(
            DiffSyntaxLanguage::Tsx,
            tsx,
            tsx.find("text").expect("child")
        ),
        Some(("<Foo bar={1}>", "</Foo>")),
        "jsx names them jsx_opening_element and jsx_closing_element"
    );

    let tsx_fragment = "const a = <>text</>;\n";
    assert_eq!(
        pair_text_in(
            DiffSyntaxLanguage::Tsx,
            tsx_fragment,
            tsx_fragment.find("text").expect("fragment child")
        ),
        Some(("<>", "</>")),
        "nameless JSX fragments should remain pairable"
    );

    let vue = "<template><p>hi</p></template>\n";
    assert_eq!(
        pair_text_in(DiffSyntaxLanguage::Vue, vue, vue.find("hi").expect("text")),
        Some(("<p>", "</p>"))
    );

    let svelte = "<main><b>hi</b></main>\n";
    assert_eq!(
        pair_text_in(
            DiffSyntaxLanguage::Svelte,
            svelte,
            svelte.find("hi").expect("text")
        ),
        Some(("<b>", "</b>"))
    );
}

/// A tag with no partner matches nothing rather than pairing with the wrong
/// element. Both cases are decisions, not gaps.
#[test]
fn syntax_pair_leaves_invalid_or_self_closing_tags_unpaired() {
    let self_closing = "<br/>\n";
    assert_eq!(
        pair_text_in(
            DiffSyntaxLanguage::Html,
            self_closing,
            self_closing.find("br").expect("name")
        ),
        None,
        "a self-closing tag has no partner"
    );

    let unclosed = "<div>\n";
    assert_eq!(
        pair_text_in(
            DiffSyntaxLanguage::Html,
            unclosed,
            unclosed.find("div").expect("name")
        ),
        None,
        "an unclosed tag must not reach for some other element's end tag"
    );

    let jsx_self_closing = "const a = <Foo />;\n";
    assert_eq!(
        pair_text_in(
            DiffSyntaxLanguage::Tsx,
            jsx_self_closing,
            jsx_self_closing.find("Foo").expect("name")
        ),
        None
    );

    let jsx_mismatched = "const a = <Foo>text</Bar>;\n";
    for needle in ["Foo", "text", "Bar"] {
        assert_eq!(
            pair_text_in(
                DiffSyntaxLanguage::Tsx,
                jsx_mismatched,
                jsx_mismatched.find(needle).expect("mismatched JSX part")
            ),
            None,
            "mismatched JSX tags must not pair when clicking {needle:?}"
        );
    }
}

/// Quotes flanking a string's content pair with each other, in the two
/// shapes grammars use for them.
#[test]
fn syntax_pair_matches_quote_delimiters() {
    // Anonymous same-kind tokens: JSON, Rust, JS.
    let json = "{\"alpha\": \"beta\"}\n";
    let in_value = json.find("beta").expect("value");
    assert_eq!(
        pair_kind_in(DiffSyntaxLanguage::Json, json, in_value),
        Some(SyntaxPairKind::Quote)
    );
    let document = document_in(DiffSyntaxLanguage::Json, json, Vec::new());
    let pair = document
        .snapshot(AppTheme::gitcomet_dark())
        .syntax_pair_at(in_value)
        .expect("the value's own quotes");
    assert_eq!(
        pair.open.start,
        json.find("\"beta").expect("value open quote"),
        "the value pairs with its own quotes, not the key's"
    );

    let rust = "let s = \"abc\";\n";
    assert_eq!(
        pair_kind_in(
            DiffSyntaxLanguage::Rust,
            rust,
            rust.find("abc").expect("body")
        ),
        Some(SyntaxPairKind::Quote)
    );

    let js = "const s = 'abc';\n";
    assert_eq!(
        pair_kind_in(
            DiffSyntaxLanguage::JavaScript,
            js,
            js.find("abc").expect("body")
        ),
        Some(SyntaxPairKind::Quote)
    );

    // Explicit named marker nodes: Python's string_start / string_end.
    let python = "s = \"abc\"\n";
    assert_eq!(
        pair_kind_in(
            DiffSyntaxLanguage::Python,
            python,
            python.find("abc").expect("body")
        ),
        Some(SyntaxPairKind::Quote)
    );
}

/// Two documented gaps, asserted so they read as decisions.
#[test]
fn syntax_pair_leaves_raw_strings_and_lifetimes_unpaired() {
    // A lone `'` has no partner among its parent's children.
    let lifetime = "struct S<'a> { r: &'a str }\n";
    let on_tick = lifetime.find("'a>").expect("lifetime");
    assert_ne!(
        pair_kind_in(DiffSyntaxLanguage::Rust, lifetime, on_tick),
        Some(SyntaxPairKind::Quote),
        "a lifetime tick is not half of a quote pair"
    );

    // Raw string delimiters are hidden external tokens: they are not in the
    // tree at all, so there is nothing to pair.
    let raw = "let s = r#\"abc\"#;\n";
    assert_ne!(
        pair_kind_in(
            DiffSyntaxLanguage::Rust,
            raw,
            raw.find("abc").expect("body")
        ),
        Some(SyntaxPairKind::Quote)
    );
}

/// When several kinds enclose the caret, the innermost one answers.
#[test]
fn syntax_pair_takes_the_innermost_enclosing_kind() {
    let text = "<div class=\"card\">x</div>\n";
    let in_attribute = text.find("card").expect("attribute value");
    assert_eq!(
        pair_kind_in(DiffSyntaxLanguage::Html, text, in_attribute),
        Some(SyntaxPairKind::Quote),
        "inside the attribute value the quotes are nearer than the tags"
    );

    let rust = "fn f() { let s = \"a\"; }\n";
    assert_eq!(
        pair_kind_in(
            DiffSyntaxLanguage::Rust,
            rust,
            rust.find('a').expect("body")
        ),
        Some(SyntaxPairKind::Quote),
        "the string's quotes are nearer than the enclosing block braces"
    );
}

#[test]
fn syntax_pair_is_none_outside_any_pair() {
    let text = "fn main() {\n    let value = 1;\n}\n";
    let document = document(text, Vec::new());
    assert_eq!(
        document
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(0),
        None,
        "the caret before `fn` is inside nothing"
    );
}
