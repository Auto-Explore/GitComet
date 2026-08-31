use super::*;

#[test]
fn capture_name_mapping_preserves_rich_semantics() {
    // Full dot-qualified names should map to specific variants
    assert_eq!(
        syntax_kind_from_capture_name("comment.doc"),
        Some(SyntaxTokenKind::CommentDoc)
    );
    assert_eq!(
        syntax_kind_from_capture_name("string.escape"),
        Some(SyntaxTokenKind::StringEscape)
    );
    assert_eq!(
        syntax_kind_from_capture_name("keyword.control"),
        Some(SyntaxTokenKind::KeywordControl)
    );
    assert_eq!(
        syntax_kind_from_capture_name("function.method"),
        Some(SyntaxTokenKind::FunctionMethod)
    );
    assert_eq!(
        syntax_kind_from_capture_name("function.special"),
        Some(SyntaxTokenKind::FunctionSpecial)
    );
    assert_eq!(
        syntax_kind_from_capture_name("constructor"),
        Some(SyntaxTokenKind::Constructor)
    );
    assert_eq!(
        syntax_kind_from_capture_name("type.builtin"),
        Some(SyntaxTokenKind::TypeBuiltin)
    );
    assert_eq!(
        syntax_kind_from_capture_name("type.interface"),
        Some(SyntaxTokenKind::TypeInterface)
    );
    assert_eq!(
        syntax_kind_from_capture_name("namespace"),
        Some(SyntaxTokenKind::Namespace)
    );
    assert_eq!(
        syntax_kind_from_capture_name("variable"),
        Some(SyntaxTokenKind::Variable)
    );
    assert_eq!(
        syntax_kind_from_capture_name("variable.parameter"),
        Some(SyntaxTokenKind::VariableParameter)
    );
    assert_eq!(
        syntax_kind_from_capture_name("variable.special"),
        Some(SyntaxTokenKind::VariableSpecial)
    );
    assert_eq!(
        syntax_kind_from_capture_name("variable.builtin"),
        Some(SyntaxTokenKind::VariableBuiltin)
    );
    assert_eq!(
        syntax_kind_from_capture_name("label"),
        Some(SyntaxTokenKind::Label)
    );
    assert_eq!(
        syntax_kind_from_capture_name("operator"),
        Some(SyntaxTokenKind::Operator)
    );
    assert_eq!(
        syntax_kind_from_capture_name("punctuation.bracket"),
        Some(SyntaxTokenKind::PunctuationBracket)
    );
    assert_eq!(
        syntax_kind_from_capture_name("punctuation.delimiter"),
        Some(SyntaxTokenKind::PunctuationDelimiter)
    );
    assert_eq!(
        syntax_kind_from_capture_name("punctuation.special"),
        Some(SyntaxTokenKind::PunctuationSpecial)
    );
    assert_eq!(
        syntax_kind_from_capture_name("punctuation.list_marker.markup"),
        Some(SyntaxTokenKind::PunctuationListMarker)
    );
    assert_eq!(
        syntax_kind_from_capture_name("punctuation.list_marker"),
        Some(SyntaxTokenKind::PunctuationListMarker)
    );
    assert_eq!(
        syntax_kind_from_capture_name("tag"),
        Some(SyntaxTokenKind::Tag)
    );
    assert_eq!(
        syntax_kind_from_capture_name("attribute"),
        Some(SyntaxTokenKind::Attribute)
    );
    assert_eq!(
        syntax_kind_from_capture_name("lifetime"),
        Some(SyntaxTokenKind::Lifetime)
    );
    assert_eq!(
        syntax_kind_from_capture_name("boolean"),
        Some(SyntaxTokenKind::Boolean)
    );
    assert_eq!(
        syntax_kind_from_capture_name("preproc"),
        Some(SyntaxTokenKind::Preproc)
    );
    assert_eq!(
        syntax_kind_from_capture_name("string.regex"),
        Some(SyntaxTokenKind::StringRegex)
    );
    assert_eq!(
        syntax_kind_from_capture_name("string.regexp"),
        Some(SyntaxTokenKind::StringRegex)
    );
    assert_eq!(
        syntax_kind_from_capture_name("string.special.regex"),
        Some(SyntaxTokenKind::StringRegex)
    );
    assert_eq!(
        syntax_kind_from_capture_name("string.special.symbol"),
        Some(SyntaxTokenKind::StringSpecial)
    );
    assert_eq!(
        syntax_kind_from_capture_name("constant.builtin"),
        Some(SyntaxTokenKind::ConstantBuiltin)
    );
    assert_eq!(
        syntax_kind_from_capture_name("markup.heading"),
        Some(SyntaxTokenKind::MarkupHeading)
    );
    assert_eq!(
        syntax_kind_from_capture_name("title.markup"),
        Some(SyntaxTokenKind::MarkupHeading)
    );
    assert_eq!(
        syntax_kind_from_capture_name("markup.link.url"),
        Some(SyntaxTokenKind::MarkupLink)
    );
    assert_eq!(
        syntax_kind_from_capture_name("link_uri.markup"),
        Some(SyntaxTokenKind::MarkupLink)
    );
    assert_eq!(
        syntax_kind_from_capture_name("text.uri"),
        Some(SyntaxTokenKind::MarkupLink)
    );
    assert_eq!(
        syntax_kind_from_capture_name("text.literal.markup"),
        Some(SyntaxTokenKind::TextLiteral)
    );
    assert_eq!(
        syntax_kind_from_capture_name("text.literal"),
        Some(SyntaxTokenKind::TextLiteral)
    );
    assert_eq!(
        syntax_kind_from_capture_name("text.title"),
        Some(SyntaxTokenKind::MarkupHeading)
    );
    assert_eq!(
        syntax_kind_from_capture_name("diff.plus"),
        Some(SyntaxTokenKind::DiffPlus)
    );
    assert_eq!(
        syntax_kind_from_capture_name("diff.minus"),
        Some(SyntaxTokenKind::DiffMinus)
    );
    assert_eq!(
        syntax_kind_from_capture_name("diff.delta"),
        Some(SyntaxTokenKind::DiffDelta)
    );
    assert_eq!(
        syntax_kind_from_capture_name("tag.jsx"),
        Some(SyntaxTokenKind::Tag)
    );
    assert_eq!(
        syntax_kind_from_capture_name("property.name"),
        Some(SyntaxTokenKind::Property)
    );
    assert_eq!(
        syntax_kind_from_capture_name("type.name"),
        Some(SyntaxTokenKind::Type)
    );
    assert_eq!(
        syntax_kind_from_capture_name("punctuation.bracket.html"),
        Some(SyntaxTokenKind::PunctuationBracket)
    );
    assert_eq!(
        syntax_kind_from_capture_name("punctuation.delimiter.jsx"),
        Some(SyntaxTokenKind::PunctuationDelimiter)
    );

    // Base names should still work
    assert_eq!(
        syntax_kind_from_capture_name("comment"),
        Some(SyntaxTokenKind::Comment)
    );
    assert_eq!(
        syntax_kind_from_capture_name("string"),
        Some(SyntaxTokenKind::String)
    );
    assert_eq!(
        syntax_kind_from_capture_name("keyword"),
        Some(SyntaxTokenKind::Keyword)
    );

    // Unknown dot-qualified names fall back through shorter dotted prefixes
    assert_eq!(
        syntax_kind_from_capture_name("keyword.operator.regex"),
        Some(SyntaxTokenKind::Keyword)
    );
    assert_eq!(
        syntax_kind_from_capture_name("comment.block"),
        Some(SyntaxTokenKind::Comment)
    );

    // Truly unknown names return None
    assert_eq!(syntax_kind_from_capture_name("none"), None);
    assert_eq!(syntax_kind_from_capture_name("embedded"), None);
    assert_eq!(syntax_kind_from_capture_name("text.jsx"), None);
}

#[test]
fn normalize_non_overlapping_tokens_keeps_later_same_range_token() {
    let tokens = normalize_non_overlapping_tokens(vec![
        SyntaxToken {
            range: 0..5,
            kind: SyntaxTokenKind::Function,
        },
        SyntaxToken {
            range: 0..5,
            kind: SyntaxTokenKind::Type,
        },
    ]);
    assert_eq!(
        tokens,
        vec![SyntaxToken {
            range: 0..5,
            kind: SyntaxTokenKind::Type,
        }]
    );
}

#[test]
fn normalize_non_overlapping_tokens_splits_outer_token_for_inner_semantics() {
    let tokens = normalize_non_overlapping_tokens(vec![
        SyntaxToken {
            range: 0..22,
            kind: SyntaxTokenKind::Comment,
        },
        SyntaxToken {
            range: 2..10,
            kind: SyntaxTokenKind::DiffPlus,
        },
        SyntaxToken {
            range: 12..22,
            kind: SyntaxTokenKind::StringSpecial,
        },
    ]);
    assert_eq!(
        tokens,
        vec![
            SyntaxToken {
                range: 0..2,
                kind: SyntaxTokenKind::Comment,
            },
            SyntaxToken {
                range: 2..10,
                kind: SyntaxTokenKind::DiffPlus,
            },
            SyntaxToken {
                range: 10..12,
                kind: SyntaxTokenKind::Comment,
            },
            SyntaxToken {
                range: 12..22,
                kind: SyntaxTokenKind::StringSpecial,
            },
        ]
    );
}

#[test]
fn normalize_non_overlapping_tokens_splits_contained_later_token() {
    let tokens = normalize_non_overlapping_tokens(vec![
        SyntaxToken {
            range: 0..10,
            kind: SyntaxTokenKind::Function,
        },
        SyntaxToken {
            range: 4..6,
            kind: SyntaxTokenKind::Operator,
        },
    ]);
    assert_eq!(
        tokens,
        vec![
            SyntaxToken {
                range: 0..4,
                kind: SyntaxTokenKind::Function,
            },
            SyntaxToken {
                range: 4..6,
                kind: SyntaxTokenKind::Operator,
            },
            SyntaxToken {
                range: 6..10,
                kind: SyntaxTokenKind::Function,
            },
        ]
    );
}

#[test]
fn normalize_non_overlapping_tokens_assigns_partial_overlap_to_later_token() {
    let tokens = normalize_non_overlapping_tokens(vec![
        SyntaxToken {
            range: 0..8,
            kind: SyntaxTokenKind::Comment,
        },
        SyntaxToken {
            range: 5..12,
            kind: SyntaxTokenKind::DiffMinus,
        },
    ]);
    assert_eq!(
        tokens,
        vec![
            SyntaxToken {
                range: 0..5,
                kind: SyntaxTokenKind::Comment,
            },
            SyntaxToken {
                range: 5..12,
                kind: SyntaxTokenKind::DiffMinus,
            },
        ]
    );
}

/// Subtracting a batch of ranges in one sweep must land exactly where
/// subtracting them one at a time did.
///
/// The batched form exists because the one-at-a-time loop rebuilt the whole
/// line's token vector per intersecting cut, which is quadratic in the
/// injected tokens on a line -- but it is only worth having if it is the
/// same function. Cross-checked here against the single-range form on
/// shapes that stress the differences: cuts given out of order, cuts that
/// overlap each other, empty cuts, cuts that fall entirely outside the
/// line, and cuts that swallow a token whole.
#[test]
fn batched_line_token_subtraction_matches_one_cut_at_a_time() {
    use super::prepared::{
        subtract_relative_range_from_line_tokens, subtract_relative_ranges_from_line_tokens,
    };

    let kinds = [
        SyntaxTokenKind::Comment,
        SyntaxTokenKind::Keyword,
        SyntaxTokenKind::String,
    ];
    // A deterministic spread of shapes rather than a handful of literals:
    // the failure mode here is an off-by-one at a boundary, and boundaries
    // are what vary.
    let mut seed = 0x9E3779B9u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as usize
    };

    for case in 0..400 {
        let token_count = 1 + next() % 6;
        let mut line_tokens: Vec<SyntaxToken> = Vec::new();
        let mut at = 0usize;
        for _ in 0..token_count {
            let gap = next() % 3;
            let width = 1 + next() % 9;
            let start = at + gap;
            line_tokens.push(SyntaxToken {
                range: start..start + width,
                kind: kinds[next() % kinds.len()],
            });
            at = start + width;
        }
        let cut_count = next() % 8;
        let cuts: Vec<std::ops::Range<usize>> = (0..cut_count)
            .map(|_| {
                let start = next() % (at + 4);
                // Deliberately includes zero-width cuts, which both forms
                // must ignore rather than split on.
                let width = next() % 6;
                start..start + width
            })
            .collect();

        let mut one_at_a_time = line_tokens.clone();
        for cut in &cuts {
            subtract_relative_range_from_line_tokens(&mut one_at_a_time, cut.clone());
        }

        let mut batched = line_tokens.clone();
        let mut batch = cuts.clone();
        subtract_relative_ranges_from_line_tokens(&mut batched, &mut batch);

        assert_eq!(
            batched, one_at_a_time,
            "case {case}: tokens {line_tokens:?} minus {cuts:?}"
        );
    }
}
