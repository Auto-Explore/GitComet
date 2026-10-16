use super::*;

#[test]
fn resetting_one_scope_preserves_another_threads_document_seed() {
    reset_prepared_syntax_cache();
    let own = prepare_test_document(DiffSyntaxLanguage::Rust, "fn same() {}\n");
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (reset_tx, reset_rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let foreign = prepare_test_document(DiffSyntaxLanguage::Rust, "fn same() {}\n");
        ready_tx.send(foreign).unwrap();
        reset_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(
            shared_prepared_document_seed_store()
                .lock()
                .unwrap()
                .contains_key(&foreign.cache_key)
        );
        assert!(prepared_document_tree_state(foreign).is_some());
    });
    let foreign = ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_ne!(own.cache_key, foreign.cache_key);
    reset_prepared_syntax_cache();
    assert!(!syntax_test_scope_is_active(own.cache_key));
    assert!(syntax_test_scope_is_active(foreign.cache_key));
    reset_tx.send(()).unwrap();
    thread.join().unwrap();
    assert!(!syntax_test_scope_is_active(foreign.cache_key));
}

#[test]
fn a_late_drop_receipt_cannot_increment_reset_counters() {
    reset_deferred_drop_counters();
    let payload = SyntaxCacheDropPayload::new(Vec::new(), TS_DEFERRED_DROP_MIN_BYTES);
    reset_deferred_drop_counters();
    drop_line_tokens_with_mode(payload, SyntaxCacheDropMode::DeferredWhenLarge);
    assert!(flush_deferred_syntax_cache_drop_queue_with_timeout(
        Duration::from_secs(3)
    ));
    assert_eq!(deferred_drop_counters(), (0, 0, 0));
    let payload = SyntaxCacheDropPayload::new(Vec::new(), TS_DEFERRED_DROP_MIN_BYTES);
    drop_line_tokens_with_mode(payload, SyntaxCacheDropMode::DeferredWhenLarge);
    assert!(flush_deferred_syntax_cache_drop_queue_with_timeout(
        Duration::from_secs(3)
    ));
    assert_eq!(deferred_drop_counters(), (1, 1, 0));
}

#[test]
fn reset_rejects_late_chunk_results_and_document_seeds() {
    reset_prepared_syntax_cache();
    let old = prepare_test_document(DiffSyntaxLanguage::Rust, "fn old() {}\n");
    let seed = shared_prepared_document_seed_store()
        .lock()
        .unwrap()
        .get(&old.cache_key)
        .unwrap()
        .clone();
    let result = PreparedSyntaxChunkBuildResult {
        chunk_key: PreparedSyntaxChunkKey {
            cache_key: old.cache_key,
            chunk_ix: 0,
        },
        chunk_tokens: None,
        chunk_build_ms: 0,
        thread_id: std::thread::current().id(),
    };
    assert!(should_apply_chunk_build_result(
        &result,
        result.thread_id,
        None
    ));
    reset_prepared_syntax_cache();
    assert!(!should_apply_chunk_build_result(
        &result,
        result.thread_id,
        None
    ));
    store_shared_prepared_document_seed(&seed);
    assert!(
        !shared_prepared_document_seed_store()
            .lock()
            .unwrap()
            .contains_key(&old.cache_key)
    );
}
