use super::*;

#[test]
fn prepared_document_cache_keeps_multiple_documents_available() {
    let first_doc = prepare_test_document(DiffSyntaxLanguage::Rust, "/* one */ let a = 1;");
    let second_doc = prepare_test_document(DiffSyntaxLanguage::Rust, "/* two */ let b = 2;");

    let first_tokens = syntax_tokens_for_prepared_document_line(first_doc, 0)
        .expect("first prepared document should remain in cache");
    let second_tokens = syntax_tokens_for_prepared_document_line(second_doc, 0)
        .expect("second prepared document should be in cache");

    assert!(
        first_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Comment),
        "first document should keep its tokens available"
    );
    assert!(
        second_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Comment),
        "second document should keep its tokens available"
    );
}

#[test]
fn prepared_document_tokens_are_chunked_and_materialized_lazily() {
    // The prepared-document cache is thread-local and persists across tests on the same worker
    // thread, so clear it before asserting exact miss/hit behavior.
    reset_prepared_syntax_cache();
    let lines = (0..(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 3))
        .map(|ix| format!("let value_{ix} = {ix};"))
        .collect::<Vec<_>>();
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

    assert_eq!(
        prepared_syntax_loaded_chunk_count(document),
        0,
        "prepared document should start with no chunk materialization"
    );

    let _ = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("first line tokens should resolve");
    assert_eq!(
        prepared_syntax_loaded_chunk_count(document),
        1,
        "first lookup should materialize one chunk"
    );
    let after_first_lookup = prepared_syntax_cache_metrics();
    assert_eq!(after_first_lookup.miss, 1);
    assert_eq!(after_first_lookup.hit, 0);

    let _ = syntax_tokens_for_prepared_document_line(document, 1)
        .expect("same-chunk lookup should resolve");
    assert_eq!(
        prepared_syntax_loaded_chunk_count(document),
        1,
        "same chunk lookup should reuse cached chunk"
    );
    let after_second_lookup = prepared_syntax_cache_metrics();
    assert_eq!(after_second_lookup.miss, 1);
    assert_eq!(after_second_lookup.hit, 1);

    let _ = syntax_tokens_for_prepared_document_line(document, TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
        .expect("next-chunk lookup should resolve");
    assert_eq!(
        prepared_syntax_loaded_chunk_count(document),
        2,
        "lookup on next chunk boundary should build one additional chunk"
    );
    let after_third_lookup = prepared_syntax_cache_metrics();
    assert_eq!(after_third_lookup.miss, 2);
    assert_eq!(after_third_lookup.hit, 1);
    assert!(
        after_third_lookup.chunk_build_ms >= after_first_lookup.chunk_build_ms,
        "chunk build metric should accumulate monotonically"
    );
}

#[test]
fn prepared_document_chunk_request_builds_in_background() {
    let lines = (0..(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2))
        .map(|ix| format!("let value_{ix} = {ix};"))
        .collect::<Vec<_>>();
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

    assert_eq!(
        prepared_syntax_loaded_chunk_count(document),
        0,
        "prepared document should start with no chunk materialization"
    );
    assert_eq!(
        request_syntax_tokens_for_prepared_document_line(document, 0),
        Some(PreparedSyntaxLineTokensRequest::Pending),
        "first request should enqueue a background chunk build"
    );
    assert_eq!(
        prepared_syntax_loaded_chunk_count(document),
        0,
        "pending request should not materialize the chunk synchronously"
    );
    assert!(
        has_pending_prepared_syntax_chunk_builds(),
        "background chunk request should remain pending until drained"
    );

    assert!(
        wait_for_all_background_chunk_builds_for_document(document, Duration::from_secs(2)) > 0,
        "background chunk builds should complete within timeout"
    );
    assert_eq!(
        prepared_syntax_loaded_chunk_count(document),
        2,
        "first visible miss should also prefetch the adjacent chunk"
    );

    let ready = request_syntax_tokens_for_prepared_document_line(document, 0);
    match ready {
        Some(PreparedSyntaxLineTokensRequest::Ready(tokens)) => {
            assert!(
                tokens
                    .iter()
                    .any(|token| token.kind == SyntaxTokenKind::Keyword),
                "ready chunk should expose syntax tokens"
            );
        }
        other => panic!("expected ready tokens after background chunk build, got {other:?}"),
    }
    let prefetched = request_syntax_tokens_for_prepared_document_line(
        document,
        TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS,
    );
    match prefetched {
        Some(PreparedSyntaxLineTokensRequest::Ready(tokens)) => {
            assert!(
                tokens
                    .iter()
                    .any(|token| token.kind == SyntaxTokenKind::Keyword),
                "adjacent prefetched chunk should already be ready"
            );
        }
        other => panic!("expected prefetched adjacent chunk to be ready, got {other:?}"),
    }
    assert!(
        !has_pending_prepared_syntax_chunk_builds(),
        "drained chunk request should clear pending state"
    );
}

#[test]
fn prepared_document_chunk_prefetch_shares_one_tree_state_clone() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();
    reset_prepared_syntax_cache();
    let lines = (0..(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2))
        .map(|ix| format!("let value_{ix} = {ix};"))
        .collect::<Vec<_>>();
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

    let clones_before_request = tree_state_clone_count();
    assert_eq!(
        request_syntax_tokens_for_prepared_document_line(document, 0),
        Some(PreparedSyntaxLineTokensRequest::Pending),
        "first request should enqueue the visible chunk and its prefetched neighbor"
    );
    assert_eq!(
        tree_state_clone_count(),
        clones_before_request.saturating_add(1),
        "the queued chunk burst should share one cloned tree state"
    );
}

#[test]
fn document_scoped_chunk_drain_preserves_other_documents() {
    let lines_a = (0..TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
        .map(|ix| format!("let alpha_{ix} = {ix};"))
        .collect::<Vec<_>>();
    let lines_b = (0..TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
        .map(|ix| format!("let beta_{ix} = {ix};"))
        .collect::<Vec<_>>();
    let document_a = prepare_test_document(DiffSyntaxLanguage::Rust, &lines_a.join("\n"));
    let document_b = prepare_test_document(DiffSyntaxLanguage::Rust, &lines_b.join("\n"));

    assert_eq!(
        request_syntax_tokens_for_prepared_document_line(document_a, 0),
        Some(PreparedSyntaxLineTokensRequest::Pending)
    );
    assert_eq!(
        request_syntax_tokens_for_prepared_document_line(document_b, 0),
        Some(PreparedSyntaxLineTokensRequest::Pending)
    );
    assert!(has_pending_prepared_syntax_chunk_builds_for_document(
        document_a
    ));
    assert!(has_pending_prepared_syntax_chunk_builds_for_document(
        document_b
    ));

    assert!(
        wait_for_background_chunk_build_for_document(document_a, Duration::from_secs(2)) > 0,
        "document-scoped drain should eventually apply the requested chunk"
    );
    assert_eq!(prepared_syntax_loaded_chunk_count(document_a), 1);
    assert_eq!(
        prepared_syntax_loaded_chunk_count(document_b),
        0,
        "draining document_a should not materialize document_b"
    );
    assert!(!has_pending_prepared_syntax_chunk_builds_for_document(
        document_a
    ));
    assert!(
        has_pending_prepared_syntax_chunk_builds_for_document(document_b),
        "other document work should remain pending"
    );

    assert!(
        wait_for_background_chunk_build_for_document(document_b, Duration::from_secs(2)) > 0,
        "remaining document chunk should still be drainable afterward"
    );
    assert_eq!(prepared_syntax_loaded_chunk_count(document_b), 1);
    assert!(!has_pending_prepared_syntax_chunk_builds_for_document(
        document_b
    ));
}

#[test]
fn prepared_document_chunk_hit_does_not_clone_tree_state() {
    let _lock = lock_global_counter_tests();
    reset_deferred_drop_counters();
    reset_prepared_syntax_cache();
    let lines = (0..(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2))
        .map(|ix| format!("let chunk_clone_probe_{ix} = {ix};"))
        .collect::<Vec<_>>();
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

    let _ = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("first miss should resolve and build first chunk");
    let clones_after_miss = tree_state_clone_count();
    assert!(
        clones_after_miss >= 1,
        "chunk miss should clone tree state for chunk build"
    );

    let _ = syntax_tokens_for_prepared_document_line(document, 1)
        .expect("same-chunk hit should resolve");
    assert_eq!(
        tree_state_clone_count(),
        clones_after_miss,
        "chunk-hit lookup should not clone tree state"
    );
}

#[test]
fn prepared_tree_state_clones_share_source_buffers() {
    let lines = (0..128usize)
        .map(|ix| format!("let value_{ix} = {ix};"))
        .collect::<Vec<_>>();
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

    let (first, second) = TS_DOCUMENT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let first = cache
            .tree_state(document.cache_key)
            .expect("first tree state clone should exist");
        let second = cache
            .tree_state(document.cache_key)
            .expect("second tree state clone should exist");
        (first, second)
    });

    assert!(
        first.text.as_ptr() == second.text.as_ptr() && first.text.len() == second.text.len(),
        "tree state clones should share source text storage"
    );
    assert!(
        Arc::ptr_eq(&first.line_starts, &second.line_starts),
        "tree state clones should share line start storage"
    );
}

#[test]
fn shared_text_input_reuses_snapshot_line_start_storage() {
    let snapshot = crate::kit::text_model::TextModel::from("alpha\nbeta\ngamma").snapshot();
    let shared_line_starts = snapshot.shared_line_starts();
    let input = treesitter_document_input_from_shared_text(
        snapshot.as_shared_string(),
        shared_line_starts.clone(),
    );

    assert!(
        Arc::ptr_eq(&input.line_starts, &shared_line_starts),
        "full-text tree-sitter input should reuse snapshot line-start storage"
    );
    assert_eq!(input.line_starts.as_ref(), snapshot.line_starts());
}

#[test]
fn collected_input_last_line_content_excludes_trailing_newline() {
    let input = treesitter_document_input_from_text("alpha\nbeta");

    assert_eq!(
        line_content_end_byte(input.line_starts.as_ref(), input.text.as_bytes(), 0),
        5
    );
    assert_eq!(
        line_content_end_byte(input.line_starts.as_ref(), input.text.as_bytes(), 1),
        input.text.len(),
        "text-built input should not include trailing content beyond the last line"
    );
}

#[test]
fn shared_text_input_last_line_content_excludes_trailing_newline() {
    let snapshot = crate::kit::text_model::TextModel::from("alpha\nbeta\n").snapshot();
    let text_input = treesitter_document_input_from_text("alpha\nbeta\n");
    let input = treesitter_document_input_from_shared_text(
        snapshot.as_shared_string(),
        snapshot.shared_line_starts(),
    );

    assert_eq!(
        input.line_starts.as_ref(),
        text_input.line_starts.as_ref(),
        "shared full-text input should normalize trailing-newline line starts to the same shape as collected text input"
    );
    assert_eq!(
        line_content_end_byte(input.line_starts.as_ref(), input.text.as_bytes(), 1),
        input.text.len() - 1,
        "shared full-text input should trim the real trailing newline from the last line"
    );
}

#[test]
fn shared_text_input_preserves_real_empty_last_line_while_trimming_phantom_entry() {
    let source = "alpha\n\n";
    let snapshot = crate::kit::text_model::TextModel::from(source).snapshot();
    let input = treesitter_document_input_from_shared_text(
        snapshot.as_shared_string(),
        snapshot.shared_line_starts(),
    );

    assert_eq!(
        snapshot.line_starts(),
        &[0, 6, source.len()],
        "snapshot line starts should still include the text-model phantom trailing entry"
    );
    assert_eq!(
        input.line_starts.as_ref(),
        &[0, 6],
        "tree-sitter input should keep the real empty last line but drop the phantom trailing entry"
    );
    assert_eq!(
        line_content_end_byte(input.line_starts.as_ref(), input.text.as_bytes(), 1),
        source.len() - 1,
        "the empty last line should end before the terminal newline byte"
    );
}

#[test]
fn treesitter_document_cache_lru_touch_keeps_recent_entry_alive() {
    for trial in 0..128usize {
        let mut cache = TreesitterDocumentCache::new();
        for key in 0..TS_DOCUMENT_CACHE_MAX_ENTRIES {
            cache.insert_document(
                TreesitterDocumentCache::make_test_cache_key(key as u64),
                vec![Vec::new()],
            );
        }

        let touched_key = TreesitterDocumentCache::make_test_cache_key(0);
        assert!(cache.contains_document(touched_key, 1));
        cache.insert_document(
            TreesitterDocumentCache::make_test_cache_key(10_000 + trial as u64),
            vec![Vec::new()],
        );

        assert!(
            cache.contains_key(touched_key),
            "touched key should survive eviction on trial {trial}"
        );
    }
}

#[test]
fn prepared_handle_rehydrates_after_thread_local_tree_eviction() {
    let _lock = lock_global_counter_tests();
    reset_prepared_syntax_cache();

    let text = "fn target() { let value = [1, 2]; }\n";
    let target = prepare_test_document(DiffSyntaxLanguage::Rust, text);
    for nonce in 0..TS_DOCUMENT_CACHE_MAX_ENTRIES {
        let source = format!("fn evict_{nonce}() {{ let value = [{nonce}]; }}\n");
        prepare_test_document(DiffSyntaxLanguage::Rust, &source);
    }
    assert!(
        TS_DOCUMENT_CACHE.with(|cache| !cache.borrow().contains_key(target.cache_key)),
        "the fixture must evict the target from this thread's small tree cache"
    );

    assert!(
        prepared_syntax_document_is_available(target),
        "a retained shared seed should rehydrate an otherwise stale handle"
    );
    let open = text.find('[').expect("opening bracket");
    let pair = prepared_document_syntax_pair_at_display_offset(target, 0, open)
        .expect("the rehydrated handle should support pair lookup");
    assert_eq!(pair.open[0].display_range, open..open + 1);
    let close = text.find(']').expect("closing bracket");
    assert_eq!(pair.close[0].display_range, close..close + 1);
}

#[test]
fn warm_shared_text_prepare_reuses_source_identity_without_rehashing() {
    let _lock = lock_global_counter_tests();
    reset_prepared_syntax_cache();
    reset_deferred_drop_counters();

    let source = vec!["fn warm_identity() { let value = Some(42); }"; 512].join("\n");
    let text: SharedString = source.clone().into();
    let line_starts = treesitter_document_input_from_text(&source).line_starts;
    let budget = DiffSyntaxBudget {
        foreground_parse: Duration::from_secs(1),
    };

    let first = match prepare_treesitter_document_with_budget_reuse_text(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        text.clone(),
        Arc::clone(&line_starts),
        budget,
        None,
        None,
    ) {
        PrepareTreesitterDocumentResult::Ready(document) => document,
        other => panic!("expected prepared document, got {other:?}"),
    };
    let first_hash_count = document_hash_count();
    assert!(
        first_hash_count > 0,
        "initial prepare should still hash the source at least once"
    );

    let second = match prepare_treesitter_document_with_budget_reuse_text(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        text,
        line_starts,
        budget,
        None,
        None,
    ) {
        PrepareTreesitterDocumentResult::Ready(document) => document,
        other => panic!("expected warm prepared document, got {other:?}"),
    };

    assert_eq!(second, first);
    assert_eq!(
        document_hash_count(),
        first_hash_count,
        "warm prepare should reuse the source-identity cache hit without rehashing the full text"
    );
}

#[test]
fn cold_prepare_hashes_the_source_only_once_on_cache_miss() {
    let _lock = lock_global_counter_tests();
    reset_prepared_syntax_cache();
    reset_deferred_drop_counters();

    let source = vec!["fn cold_hash_miss() { let value = Some(42); }"; 512].join("\n");
    let text: SharedString = source.clone().into();
    let line_starts = treesitter_document_input_from_text(&source).line_starts;
    let budget = DiffSyntaxBudget {
        foreground_parse: Duration::from_secs(1),
    };

    let document = match prepare_treesitter_document_with_budget_reuse_text(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        text,
        line_starts,
        budget,
        None,
        None,
    ) {
        PrepareTreesitterDocumentResult::Ready(document) => document,
        other => panic!("expected prepared document, got {other:?}"),
    };

    assert_eq!(document_hash_count(), 1);
    assert_eq!(prepared_syntax_loaded_chunk_count(document), 0);
}

#[test]
fn timed_out_prepare_reuses_pending_parse_request_in_background_without_rehashing() {
    let _lock = lock_global_counter_tests();
    reset_prepared_syntax_cache();
    reset_deferred_drop_counters();

    let source = vec!["fn background_reuse() { let value = Some(42); }"; 4_096].join("\n");
    let text: SharedString = source.clone().into();
    let line_starts = treesitter_document_input_from_text(&source).line_starts;

    let timed_out = prepare_treesitter_document_with_budget_reuse_text(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        text.clone(),
        Arc::clone(&line_starts),
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(1),
        },
        None,
        None,
    );
    assert_eq!(timed_out, PrepareTreesitterDocumentResult::TimedOut);
    assert_eq!(
        document_hash_count(),
        1,
        "timed-out foreground prepare should hash once while storing the pending request"
    );

    let background = prepare_treesitter_document_in_background_text_with_reuse(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        text,
        line_starts,
        None,
        None,
    )
    .expect("background parse should still succeed after foreground timeout");

    assert_eq!(
        document_hash_count(),
        1,
        "background parse should reuse the pending request instead of hashing again"
    );
    assert_eq!(background.line_count, 4_096);
}

#[test]
fn oversized_shared_text_prepare_falls_back_without_prepared_tree_sitter() {
    let _lock = lock_global_counter_tests();
    reset_prepared_syntax_cache();
    reset_deferred_drop_counters();

    let line = "let oversized_value: usize = 1;";
    let repeat = (TS_PREPARED_DOCUMENT_MAX_TEXT_BYTES / (line.len() + 1)).saturating_add(1);
    let source = std::iter::repeat_n(line, repeat)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        source.len() > TS_PREPARED_DOCUMENT_MAX_TEXT_BYTES,
        "fixture should exceed the prepared full-document syntax byte gate"
    );
    let input = treesitter_document_input_from_text(&source);
    let text: SharedString = source.clone().into();

    let attempt = prepare_treesitter_document_with_budget_reuse_text(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        text.clone(),
        Arc::clone(&input.line_starts),
        DiffSyntaxBudget {
            foreground_parse: Duration::from_secs(1),
        },
        None,
        None,
    );
    assert_eq!(
        attempt,
        PrepareTreesitterDocumentResult::Unsupported,
        "oversized full-document syntax should fall back before parsing"
    );
    assert_eq!(
        document_hash_count(),
        0,
        "oversized full-document syntax should skip whole-document hash work"
    );

    let background = prepare_treesitter_document_in_background_text_with_reuse(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        text,
        input.line_starts,
        None,
        None,
    );
    assert!(
        background.is_none(),
        "background prepared syntax should also skip oversized full-document inputs"
    );
}
