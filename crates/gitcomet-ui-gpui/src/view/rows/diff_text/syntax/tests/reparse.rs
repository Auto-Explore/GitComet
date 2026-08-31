use super::*;

#[test]
fn incremental_edit_ranges_cover_the_changed_window() {
    let old = b"alpha\nbeta\ngamma\n";
    let new = b"alpha\nbeta changed\ngamma\n";
    let ranges = compute_incremental_edit_ranges(old, new);
    assert_eq!(
        ranges.len(),
        1,
        "single local edit should produce one edit range"
    );

    let edit = ranges[0];
    let mut rebuilt = Vec::new();
    rebuilt.extend_from_slice(&old[..edit.start_byte]);
    rebuilt.extend_from_slice(&new[edit.start_byte..edit.new_end_byte]);
    rebuilt.extend_from_slice(&old[edit.old_end_byte..]);
    assert_eq!(
        rebuilt.as_slice(),
        new,
        "edit range should reconstruct the new buffer when applied to old bytes"
    );
}

#[test]
fn incremental_reparse_fallback_thresholds_cover_percent_and_absolute_limits() {
    let small_edit = [TreesitterByteEditRange {
        start_byte: 100,
        old_end_byte: 120,
        new_end_byte: 128,
    }];
    assert!(
        !incremental_reparse_should_fallback(&small_edit, 4_000, 4_008),
        "small deltas should stay on incremental path"
    );

    let percent_threshold_edit = [TreesitterByteEditRange {
        start_byte: 0,
        old_end_byte: 2_000,
        new_end_byte: 2_000,
    }];
    assert!(
        incremental_reparse_should_fallback(&percent_threshold_edit, 4_000, 4_000),
        "large percent deltas should force full parse fallback"
    );

    let absolute_threshold_edit = [TreesitterByteEditRange {
        start_byte: 0,
        old_end_byte: TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES.saturating_add(8),
        new_end_byte: TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES.saturating_add(8),
    }];
    assert!(
        incremental_reparse_should_fallback(
            &absolute_threshold_edit,
            TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES.saturating_add(16),
            TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES.saturating_add(16),
        ),
        "absolute changed-byte cap should force full parse fallback"
    );
}

#[test]
fn treesitter_point_for_byte_maps_newline_terminated_eof_to_next_row() {
    let input = b"alpha\nbeta\n";
    let line_starts: Vec<usize> = vec![0, 6];
    assert_eq!(
        treesitter_point_for_byte(&line_starts, input, input.len()),
        tree_sitter::Point::new(2, 0),
        "EOF for newline-terminated input should point to the next row start"
    );
}

#[test]
fn small_reparse_reuses_old_tree_with_input_edit() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();
    let base_lines = vec!["let value = 1;".to_string(); 256];
    let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));
    let base_version =
        prepared_document_source_version(base_document).expect("base source version");
    assert_eq!(
        prepared_document_parse_mode(base_document),
        Some(TreesitterParseReuseMode::Full)
    );

    let mut edited = base_lines.clone();
    edited[42].push_str(" // tiny edit");
    let attempt = prepare_test_document_with_budget_reuse(
        DiffSyntaxLanguage::Rust,
        &edited.join("\n"),
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(50),
        },
        Some(base_document),
    );
    let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
        panic!("small reparse should complete within default budget");
    };

    assert_eq!(
        prepared_document_parse_mode(reparsed_document),
        Some(TreesitterParseReuseMode::Incremental)
    );
    let reparsed_version =
        prepared_document_source_version(reparsed_document).expect("reparsed source version");
    assert!(
        reparsed_version > base_version,
        "incremental reparse should advance source version"
    );

    let (incremental, fallback) = incremental_reparse_counters();
    assert!(
        incremental > 0,
        "small edit should use incremental reparse path"
    );
    assert_eq!(fallback, 0, "small edit should not trigger fallback");
}

#[test]
fn unchanged_reparse_reuses_old_document_without_rehashing() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();
    reset_prepared_syntax_cache();

    let source = "let value = 1;\n".repeat(256);
    let base_input = treesitter_document_input_from_text(&source);
    let PrepareTreesitterDocumentResult::Ready(base_document) =
        prepare_treesitter_document_with_budget_reuse_text(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            source.clone().into(),
            base_input.line_starts.clone(),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(50),
            },
            None,
            None,
        )
    else {
        panic!("base text document should parse");
    };

    reset_deferred_drop_counters();
    let repeated_input = treesitter_document_input_from_text(&source);
    let attempt = prepare_treesitter_document_with_budget_reuse_text(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        source.into(),
        repeated_input.line_starts,
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(50),
        },
        Some(base_document),
        None,
    );
    let PrepareTreesitterDocumentResult::Ready(reused_document) = attempt else {
        panic!("unchanged reparse should reuse the existing prepared document");
    };

    assert_eq!(reused_document, base_document);
    assert_eq!(
        document_hash_count(),
        0,
        "unchanged reparses with an old document should not rehash the full source"
    );
}

#[test]
fn small_reparse_without_edit_hint_does_not_rehash_full_source() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();
    reset_prepared_syntax_cache();

    let base_text = "let value = 1;\n".repeat(256);
    let base_input = treesitter_document_input_from_text(&base_text);
    let PrepareTreesitterDocumentResult::Ready(base_document) =
        prepare_treesitter_document_with_budget_reuse_text(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            base_text.clone().into(),
            base_input.line_starts.clone(),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(50),
            },
            None,
            None,
        )
    else {
        panic!("base text document should parse");
    };

    let insert_offset = base_input.line_starts[42].saturating_add("let value = 1;".len());
    let mut edited_text = base_text;
    edited_text.insert_str(insert_offset, " // tiny edit");
    let edited_input = treesitter_document_input_from_text(&edited_text);

    reset_deferred_drop_counters();
    let attempt = prepare_treesitter_document_with_budget_reuse_text(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        edited_text.into(),
        edited_input.line_starts,
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(50),
        },
        Some(base_document),
        None,
    );
    let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
        panic!("small reparse should complete within budget");
    };

    assert_eq!(
        prepared_document_parse_mode(reparsed_document),
        Some(TreesitterParseReuseMode::Incremental)
    );
    assert_eq!(
        document_hash_count(),
        0,
        "small no-hint reparses should reuse the old source fingerprint without hashing the full text"
    );
}

#[test]
fn small_reparse_reuses_cached_prefix_chunks_before_the_edit() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();
    reset_prepared_syntax_cache();

    let line_count = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 3;
    let base_lines = (0..line_count)
        .map(|ix| format!("let value_{ix} = {ix};"))
        .collect::<Vec<_>>();
    let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));

    let _ = syntax_tokens_for_prepared_document_line(base_document, 0)
        .expect("base document should materialize its first chunk");
    assert_eq!(
        prepared_syntax_loaded_chunk_count(base_document),
        1,
        "base document should only have its first chunk materialized"
    );

    let mut edited = base_lines.clone();
    let edited_line = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2;
    edited[edited_line].push_str(" // tiny edit");
    let attempt = prepare_test_document_with_budget_reuse(
        DiffSyntaxLanguage::Rust,
        &edited.join("\n"),
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(50),
        },
        Some(base_document),
    );
    let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
        panic!("small reparse should complete within budget");
    };

    assert_eq!(
        prepared_document_parse_mode(reparsed_document),
        Some(TreesitterParseReuseMode::Incremental),
        "small later-line edit should stay on the incremental path"
    );
    assert_eq!(
        prepared_syntax_loaded_chunk_count(reparsed_document),
        1,
        "cached prefix chunks before the edit should carry forward to the reparsed document"
    );

    benchmark_reset_prepared_syntax_cache_metrics();
    let _ = syntax_tokens_for_prepared_document_line(reparsed_document, 0)
        .expect("reparsed document should reuse the carried prefix chunk");
    let after_prefix_hit = prepared_syntax_cache_metrics();
    assert_eq!(after_prefix_hit.hit, 1);
    assert_eq!(after_prefix_hit.miss, 0);

    let _ = syntax_tokens_for_prepared_document_line(reparsed_document, edited_line)
        .expect("changed chunk should still be buildable on demand");
    let after_changed_lookup = prepared_syntax_cache_metrics();
    assert_eq!(after_changed_lookup.hit, 1);
    assert_eq!(after_changed_lookup.miss, 1);
}

#[test]
fn small_reparse_reuses_old_tree_with_explicit_edit_hint_text_input() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();

    let base_text = "let value = 1;\n".repeat(256);
    let base_input = treesitter_document_input_from_text(&base_text);
    let base_document =
        prepare_test_document_from_shared_text(DiffSyntaxLanguage::Rust, &base_text);

    let insert_offset = base_input.line_starts[42].saturating_add("let value = 1;".len());
    let mut edited_text = base_text.clone();
    edited_text.insert_str(insert_offset, " // tiny edit");
    let edited_input = treesitter_document_input_from_text(&edited_text);
    let attempt = prepare_treesitter_document_with_budget_reuse_text(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        edited_text.into(),
        edited_input.line_starts.clone(),
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(50),
        },
        Some(base_document),
        Some(DiffSyntaxEdit {
            old_range: insert_offset..insert_offset,
            new_range: insert_offset..insert_offset.saturating_add(" // tiny edit".len()),
        }),
    );
    let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
        panic!("explicit-edit text reparse should complete within budget");
    };

    assert_eq!(
        prepared_document_parse_mode(reparsed_document),
        Some(TreesitterParseReuseMode::Incremental),
        "explicit edit hints should keep full-text reparses on the incremental path"
    );

    let (incremental, fallback) = incremental_reparse_counters();
    assert!(
        incremental > 0,
        "explicit edit hint path should use incremental reparse"
    );
    assert_eq!(
        fallback, 0,
        "explicit edit hint should not trigger fallback"
    );
}

#[test]
fn large_reparse_falls_back_to_full_parse() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();
    let base_lines = vec!["let value = 1;".to_string(); 256];
    let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));

    let mut edited = base_lines.clone();
    for line in edited.iter_mut().take(180) {
        *line = "pub fn massive_fallback_path() { let x = vec![1,2,3,4]; }".to_string();
    }
    let attempt = prepare_test_document_with_budget_reuse(
        DiffSyntaxLanguage::Rust,
        &edited.join("\n"),
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(200),
        },
        Some(base_document),
    );
    let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
        panic!("large reparse should complete within the test full-parse budget");
    };

    assert_eq!(
        prepared_document_parse_mode(reparsed_document),
        Some(TreesitterParseReuseMode::Full)
    );
    let (_incremental, fallback) = incremental_reparse_counters();
    assert!(
        fallback > 0,
        "large edit should trigger full-parse fallback path"
    );
}

#[test]
fn large_late_edit_with_preserved_prefix_can_stay_incremental() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();

    let base_lines = (0..256)
        .map(|ix| format!("let value_{ix} = {ix}; {}", "x".repeat(96)))
        .collect::<Vec<_>>();
    let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));

    let mut edited = base_lines.clone();
    for (offset, line) in edited.iter_mut().skip(96).enumerate() {
        *line = format!(
            "pub fn large_late_edit_{offset}() {{ let values = [{offset}, {offset}, {offset}, {offset}]; }} {}",
            "y".repeat(64)
        );
    }
    let attempt = prepare_test_document_with_budget_reuse(
        DiffSyntaxLanguage::Rust,
        &edited.join("\n"),
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(200),
        },
        Some(base_document),
    );
    let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
        panic!("large later-line reparse should complete within the test budget");
    };

    assert_eq!(
        prepared_document_parse_mode(reparsed_document),
        Some(TreesitterParseReuseMode::Incremental)
    );
    let (incremental, fallback) = incremental_reparse_counters();
    assert!(
        incremental > 0,
        "later large edit should use incremental reparse"
    );
    assert_eq!(
        fallback, 0,
        "later large edit should avoid full-parse fallback"
    );
}

#[test]
fn incremental_reparse_append_line_matches_full_parse_tokens() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();

    let base_lines = vec!["let value = 41;".to_string(); 256];
    let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));

    let mut edited = base_lines.clone();
    edited.push("let appended = 42;".to_string());
    let attempt = prepare_test_document_with_budget_reuse(
        DiffSyntaxLanguage::Rust,
        &edited.join("\n"),
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(50),
        },
        Some(base_document),
    );
    let PrepareTreesitterDocumentResult::Ready(incremental_document) = attempt else {
        panic!("incremental append reparse should complete within budget");
    };
    assert_eq!(
        prepared_document_parse_mode(incremental_document),
        Some(TreesitterParseReuseMode::Incremental),
        "small EOF append should stay on incremental reparse path"
    );

    let edited_text = edited.join("\n");
    let edited_input = treesitter_document_input_from_text(&edited_text);
    let request = treesitter_document_parse_request_from_input(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        edited_input,
    )
    .expect("edited rust lines should produce parse request");
    let full_tree = with_ts_parser(&request.ts_language, |parser| {
        parse_treesitter_tree(parser, request.input.text.as_bytes(), None, None)
    })
    .flatten()
    .expect("full parse should succeed");
    let highlight =
        tree_sitter_highlight_spec(request.language).expect("rust highlight spec should exist");

    let full_tokens = collect_treesitter_document_line_tokens_for_line_window(
        &full_tree,
        highlight,
        request.input.text.as_bytes(),
        &request.input.line_starts,
        0,
        request.input.line_starts.len(),
        treesitter_text_hash(&request.input.text),
    );
    let incremental_tokens = (0..edited.len())
        .map(|line_ix| {
            syntax_tokens_for_prepared_document_line(incremental_document, line_ix)
                .expect("incremental document should have line tokens")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        incremental_tokens, full_tokens,
        "incremental append reparse should match full-parse tokenization"
    );
}

#[test]
fn large_cache_replacement_uses_deferred_drop_queue() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();

    let mut cache = TreesitterDocumentCache::new();
    cache.insert_document(
        TreesitterDocumentCache::make_test_cache_key(1),
        benchmark_line_tokens_payload(2_048, 8, 0),
    );
    let (queued_before, dropped_before, _) = deferred_drop_counters();

    cache.insert_document(
        TreesitterDocumentCache::make_test_cache_key(1),
        benchmark_line_tokens_payload(2_048, 8, 0),
    );
    let (queued_after, _, _) = deferred_drop_counters();
    assert!(
        queued_after > queued_before,
        "large replacement should enqueue deferred drop work"
    );

    assert!(
        benchmark_flush_deferred_drop_queue(),
        "deferred drop queue should flush"
    );
    let (_, dropped_after, _) = deferred_drop_counters();
    assert!(
        dropped_after > dropped_before,
        "deferred drop worker should process queued payloads"
    );
}

#[test]
fn small_cache_replacement_keeps_inline_drop_path() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();

    let mut cache = TreesitterDocumentCache::new();
    cache.insert_document(
        TreesitterDocumentCache::make_test_cache_key(1),
        benchmark_line_tokens_payload(8, 1, 0),
    );
    let (_, _, inline_before) = deferred_drop_counters();

    cache.insert_document(
        TreesitterDocumentCache::make_test_cache_key(1),
        benchmark_line_tokens_payload(8, 1, 0),
    );
    let (_, _, inline_after) = deferred_drop_counters();
    assert!(
        inline_after > inline_before,
        "small replacement should drop old payload inline"
    );
}

#[test]
fn recent_duplicate_line_tokens_reuse_existing_arcs() {
    let document =
        TreesitterCachedDocument::from_line_tokens(benchmark_line_tokens_payload(4, 8, 0), None);
    let first_chunk = document
        .line_token_chunks
        .get(&0)
        .expect("single chunk should be present");
    assert_eq!(first_chunk.len(), 4);
    assert!(
        Arc::ptr_eq(&first_chunk[0], &first_chunk[2]),
        "alternating duplicate line tokens should reuse the two-back Arc"
    );
    assert!(
        Arc::ptr_eq(&first_chunk[1], &first_chunk[3]),
        "alternating duplicate line tokens should reuse the matching recent Arc"
    );
}

#[test]
fn cached_document_drop_payload_bytes_match_flattened_chunks() {
    let mut document =
        TreesitterCachedDocument::from_chunked_line_tokens(128, FxHashMap::default(), None);
    let first_chunk = benchmark_line_tokens_payload(64, 4, 0)
        .into_iter()
        .map(Arc::from)
        .collect::<Vec<_>>();
    let second_chunk = benchmark_line_tokens_payload(64, 4, 1)
        .into_iter()
        .map(Arc::from)
        .collect::<Vec<_>>();

    insert_line_token_chunk(&mut document, 0, Some(first_chunk));
    let bytes_after_first_insert = document.line_token_bytes;
    insert_line_token_chunk(&mut document, 0, Some(second_chunk.clone()));
    assert_eq!(
        document.line_token_bytes, bytes_after_first_insert,
        "reinserting an existing chunk should not double-count drop bytes"
    );

    insert_line_token_chunk(&mut document, 1, Some(second_chunk));
    let payload = document.into_drop_payload();
    assert_eq!(
        payload.estimated_bytes,
        estimated_line_tokens_allocation_bytes(&payload.line_tokens),
        "cached drop bytes should match the flattened payload"
    );
    assert_eq!(payload.line_tokens.len(), 128);
}

#[test]
fn large_cache_eviction_uses_deferred_drop_queue() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();

    let mut cache = TreesitterDocumentCache::new();
    for key in 0..TS_DOCUMENT_CACHE_MAX_ENTRIES {
        cache.insert_document(
            TreesitterDocumentCache::make_test_cache_key(key as u64),
            benchmark_line_tokens_payload(2_048, 8, 0),
        );
    }
    let (queued_before, dropped_before, _) = deferred_drop_counters();

    cache.insert_document(
        TreesitterDocumentCache::make_test_cache_key(TS_DOCUMENT_CACHE_MAX_ENTRIES as u64 + 1),
        benchmark_line_tokens_payload(2_048, 8, 0),
    );
    let (queued_after, _, _) = deferred_drop_counters();
    assert!(
        queued_after > queued_before,
        "large eviction should enqueue deferred drop work"
    );

    assert!(
        benchmark_flush_deferred_drop_queue(),
        "deferred drop queue should flush"
    );
    let (_, dropped_after, _) = deferred_drop_counters();
    assert!(
        dropped_after > dropped_before,
        "deferred drop worker should process evicted payloads"
    );
}

#[test]
fn parse_budget_timeout_falls_back_to_background_prepare() {
    let text = vec!["/* budget */ let value = Some(42);"; 2_048].join("\n");
    let attempt = prepare_test_document_with_budget_reuse(
        DiffSyntaxLanguage::Rust,
        &text,
        DiffSyntaxBudget {
            foreground_parse: Duration::ZERO,
        },
        None,
    );
    assert_eq!(attempt, PrepareTreesitterDocumentResult::TimedOut);

    let prepared = prepare_test_document_in_background(DiffSyntaxLanguage::Rust, &text)
        .expect("background parse should produce a prepared document");
    let document = inject_prepared_document_data(prepared);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("background-prepared document should have tokens");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
        "background parse should still yield syntax tokens"
    );
}

#[test]
fn large_full_documents_skip_default_foreground_probe_without_reuse() {
    let text = vec!["fn parse_budget_probe() { let value = Some(42); }"; 2_048].join("\n");
    let request = treesitter_document_parse_request_from_input(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        treesitter_document_input_from_text(&text),
    )
    .expect("rust request should build");

    assert!(should_skip_budgeted_foreground_parse(
        &request,
        DiffSyntaxBudget {
            foreground_parse: DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST,
        },
        false,
        false,
    ));
    assert!(!should_skip_budgeted_foreground_parse(
        &request,
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(50),
        },
        false,
        false,
    ));
    assert!(!should_skip_budgeted_foreground_parse(
        &request,
        DiffSyntaxBudget {
            foreground_parse: DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST,
        },
        true,
        false,
    ));
}

#[test]
fn small_full_documents_keep_default_foreground_probe() {
    let text = vec!["fn small_probe() { value += 1; }"; 256].join("\n");
    let request = treesitter_document_parse_request_from_input(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        treesitter_document_input_from_text(&text),
    )
    .expect("rust request should build");

    assert!(!should_skip_budgeted_foreground_parse(
        &request,
        DiffSyntaxBudget {
            foreground_parse: DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST,
        },
        false,
        false,
    ));
}

#[test]
fn background_text_reparse_reuses_old_tree_without_explicit_edit_hint() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();

    let base_text = "let value = 1;\n".repeat(256);
    let base_input = treesitter_document_input_from_text(&base_text);
    let base_document =
        prepare_test_document_from_shared_text(DiffSyntaxLanguage::Rust, &base_text);
    let base_version =
        prepared_document_source_version(base_document).expect("base source version");

    let insert_offset = base_input.line_starts[42].saturating_add("let value = 1;".len());
    let mut edited_text = base_text.clone();
    edited_text.insert_str(insert_offset, " // background tiny edit");
    let edited_input = treesitter_document_input_from_text(&edited_text);

    let prepared = prepare_treesitter_document_in_background_text_with_reuse(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        edited_text.into(),
        edited_input.line_starts.clone(),
        Some(base_document),
        None,
    )
    .expect("background text reparse should produce prepared data");
    let reparsed_document = inject_prepared_document_data(prepared);

    assert_eq!(
        prepared_document_parse_mode(reparsed_document),
        Some(TreesitterParseReuseMode::Incremental),
        "background text reparses should keep small edits on the incremental path even without explicit edit hints"
    );
    let reparsed_version =
        prepared_document_source_version(reparsed_document).expect("reparsed source version");
    assert!(
        reparsed_version > base_version,
        "background incremental reparse should advance source version"
    );

    let (incremental, fallback) = incremental_reparse_counters();
    assert!(
        incremental > 0,
        "background no-edit-hint path should use incremental reparse"
    );
    assert_eq!(
        fallback, 0,
        "background no-edit-hint path should not trigger fallback"
    );
}

#[test]
fn background_text_reparse_reuses_old_tree_with_explicit_edit_hint() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();

    let base_text = "let value = 1;\n".repeat(256);
    let base_input = treesitter_document_input_from_text(&base_text);
    let base_document =
        prepare_test_document_from_shared_text(DiffSyntaxLanguage::Rust, &base_text);
    let base_version =
        prepared_document_source_version(base_document).expect("base source version");

    let insert_offset = base_input.line_starts[42].saturating_add("let value = 1;".len());
    let mut edited_text = base_text.clone();
    edited_text.insert_str(insert_offset, " // background tiny edit");
    let edited_input = treesitter_document_input_from_text(&edited_text);

    let prepared = prepare_treesitter_document_in_background_text_with_reuse(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        edited_text.into(),
        edited_input.line_starts.clone(),
        Some(base_document),
        Some(DiffSyntaxEdit {
            old_range: insert_offset..insert_offset,
            new_range: insert_offset
                ..insert_offset.saturating_add(" // background tiny edit".len()),
        }),
    )
    .expect("background text reparse should produce prepared data");
    let reparsed_document = inject_prepared_document_data(prepared);

    assert_eq!(
        prepared_document_parse_mode(reparsed_document),
        Some(TreesitterParseReuseMode::Incremental),
        "background text reparses should keep small edits on the incremental path"
    );
    let reparsed_version =
        prepared_document_source_version(reparsed_document).expect("reparsed source version");
    assert!(
        reparsed_version > base_version,
        "background incremental reparse should advance source version"
    );

    let (incremental, fallback) = incremental_reparse_counters();
    assert!(
        incremental > 0,
        "background explicit edit hint path should use incremental reparse"
    );
    assert_eq!(
        fallback, 0,
        "background explicit edit hint should not trigger fallback"
    );
}

#[test]
fn background_seed_reuses_cached_prefix_chunks_before_large_edit_fallback() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();
    reset_prepared_syntax_cache();

    let line_count = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 4;
    let base_lines = (0..line_count)
        .map(|ix| format!("let value_{ix} = {ix};"))
        .collect::<Vec<_>>();
    let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));

    let _ = syntax_tokens_for_prepared_document_line(base_document, 0)
        .expect("base document should materialize its first chunk");
    assert_eq!(
        prepared_syntax_loaded_chunk_count(base_document),
        1,
        "base document should only have its first chunk materialized"
    );

    let reparse_seed =
        prepared_document_reparse_seed(base_document).expect("base document should expose a seed");
    let mut edited = base_lines.clone();
    let first_changed_line = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2;
    for (offset, line) in edited.iter_mut().skip(first_changed_line).enumerate() {
        *line = format!(
            "pub fn fallback_edit_{offset}() {{ let values = [{offset}, {offset}, {offset}, {offset}]; }}"
        );
    }
    let edited_text = edited.join("\n");
    let edited_input = treesitter_document_input_from_text(&edited_text);

    let prepared = prepare_treesitter_document_in_background_text_with_reparse_seed(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        edited_text.into(),
        edited_input.line_starts,
        Some(reparse_seed),
        None,
    )
    .expect("background large-edit reparse should produce prepared data");
    let reparsed_document = inject_prepared_document_data(prepared);

    assert_eq!(
        prepared_document_parse_mode(reparsed_document),
        Some(TreesitterParseReuseMode::Full),
        "large edit should still take the full-parse fallback path"
    );
    assert_eq!(
        prepared_syntax_loaded_chunk_count(reparsed_document),
        1,
        "background reparse seed should preserve cached prefix chunks before the edit"
    );

    benchmark_reset_prepared_syntax_cache_metrics();
    let _ = syntax_tokens_for_prepared_document_line(reparsed_document, 0)
        .expect("reparsed document should reuse the preserved prefix chunk");
    let after_prefix_hit = prepared_syntax_cache_metrics();
    assert_eq!(after_prefix_hit.hit, 1);
    assert_eq!(after_prefix_hit.miss, 0);
}

#[test]
fn background_prepared_document_not_in_tls_until_injected() {
    let text = "/* background comment */\nlet value = 42;".to_string();
    let prepared = std::thread::spawn({
        let text = text.clone();
        move || {
            let input = treesitter_document_input_from_text(&text);
            prepare_treesitter_document_in_background_text_with_reuse(
                DiffSyntaxLanguage::Rust,
                DiffSyntaxMode::Auto,
                SharedString::from(text),
                input.line_starts,
                None,
                None,
            )
            .expect("background parse should produce prepared data")
        }
    })
    .join()
    .expect("background parse thread should not panic");

    let unresolved_handle = PreparedSyntaxDocument {
        cache_key: prepared.cache_key,
    };
    assert!(
        syntax_tokens_for_prepared_document_line(unresolved_handle, 0).is_none(),
        "background parse must not populate main-thread TLS cache until injected"
    );

    let document = inject_prepared_document_data(prepared);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("injected background document should have tokens");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
        "injected document should include parsed comment tokens"
    );
}
