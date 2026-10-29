use super::*;

#[test]
fn injection_content_exclusion_requires_explicit_combined_opt_in() {
    let language = tree_sitter_javascript::LANGUAGE.into();
    let query = tree_sitter::Query::new(
        &language,
        r#"
        ((array) @injection.content)
        ((array) @injection.content (#set! injection.combined))
        ((array) @injection.content
          (#set! injection.combined)
          (#set! injection.exclude-named-children))
        "#,
    )
    .unwrap();
    assert_eq!(
        injection_patterns_for_query(&query).unwrap(),
        vec![
            InjectionPattern::Single,
            InjectionPattern::Combined(InjectionContentMode::WholeNode),
            InjectionPattern::Combined(InjectionContentMode::ExcludeNamedChildren),
        ]
    );

    let invalid = tree_sitter::Query::new(
        &language,
        "((array) @injection.content (#set! injection.exclude-named-children))",
    )
    .unwrap();
    assert_eq!(
        injection_patterns_for_query(&invalid),
        Err("injection.exclude-named-children requires injection.combined")
    );
}

#[test]
fn injection_content_exclusion_preserves_only_direct_child_gaps() {
    let text = "[ α, /* nested */ [beta],\r\n gamma ]";
    let language = tree_sitter_javascript::LANGUAGE.into();
    let tree = with_ts_parser_parse_result(&language, |parser| parser.parse(text, None)).unwrap();
    let array = tree
        .root_node()
        .named_child(0)
        .unwrap()
        .named_child(0)
        .unwrap();
    assert_eq!(array.kind(), "array");
    let mut ranges = Vec::new();
    append_injection_content_byte_ranges(
        array,
        text.len(),
        InjectionContentMode::ExcludeNamedChildren,
        &mut ranges,
    );
    assert_eq!(
        ranges
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<Vec<_>>(),
        vec!["[ ", ", ", " ", ",\r\n ", " ]"]
    );

    let mut clipped = Vec::new();
    let end = text.find("beta").unwrap() + "beta".len();
    append_injection_content_byte_ranges(
        array,
        end,
        InjectionContentMode::ExcludeNamedChildren,
        &mut clipped,
    );
    assert_eq!(clipped, ranges[..3]);
    append_injection_content_byte_ranges(
        array,
        0,
        InjectionContentMode::ExcludeNamedChildren,
        &mut clipped,
    );
    assert_eq!(clipped, ranges[..3]);

    let mut whole = Vec::new();
    append_injection_content_byte_ranges(
        array,
        text.len(),
        InjectionContentMode::WholeNode,
        &mut whole,
    );
    assert_eq!(whole.len(), 1);
    assert_eq!(&text[whole[0].clone()], text);
}

#[test]
fn injection_content_modes_keep_string_normalization_and_leaf_bounds() {
    let text = "const x = 'héllo';";
    let language = tree_sitter_javascript::LANGUAGE.into();
    let tree = with_ts_parser_parse_result(&language, |parser| parser.parse(text, None)).unwrap();
    let string = tree
        .root_node()
        .named_child(0)
        .unwrap()
        .named_child(0)
        .unwrap()
        .child_by_field_name("value")
        .unwrap();
    assert_eq!(string.kind(), "string");
    let mut whole = Vec::new();
    append_injection_content_byte_ranges(
        string,
        text.len(),
        InjectionContentMode::WholeNode,
        &mut whole,
    );
    assert_eq!(
        whole
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<Vec<_>>(),
        vec!["héllo"]
    );
    let mut excluded = Vec::new();
    append_injection_content_byte_ranges(
        string,
        text.len(),
        InjectionContentMode::ExcludeNamedChildren,
        &mut excluded,
    );
    assert!(excluded.is_empty());
    append_injection_content_byte_ranges(
        string.named_child(0).unwrap(),
        text.len(),
        InjectionContentMode::ExcludeNamedChildren,
        &mut excluded,
    );
    assert_eq!(excluded, whole);
}
