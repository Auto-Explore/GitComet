use super::*;

fn seed(scope: u64, hash: u64) -> PreparedSyntaxDocumentData {
    PreparedSyntaxDocumentData {
        cache_key: PreparedSyntaxCacheKey {
            language: DiffSyntaxLanguage::Rust,
            doc_hash: hash,
            test_scope: scope,
        },
        line_count: 1,
        line_token_chunks: FxHashMap::default(),
        tree_state: None,
    }
}

fn seed_presence(scope: u64, inserted: usize) -> Vec<bool> {
    let store = shared_prepared_document_seed_store().lock().unwrap();
    (0..inserted)
        .map(|ix| store.contains_key(&seed(scope, ix as u64).cache_key))
        .collect()
}

fn assert_latest_seed_presence(present: &[bool]) {
    let inserted = present.len();
    let first_retained = inserted.saturating_sub(TS_SHARED_DOCUMENT_SEED_MAX_ENTRIES);
    for (ix, &present) in present.iter().enumerate() {
        assert_eq!(
            present,
            ix >= first_retained,
            "seed {ix} after {inserted} insertions must follow insertion order"
        );
    }
}

fn assert_latest_seeds(scope: u64, inserted: usize) {
    assert_latest_seed_presence(&seed_presence(scope, inserted));
}

#[test]
fn shared_seed_capacity_evicts_the_oldest_insertions() {
    reset_prepared_syntax_cache();
    let scope = syntax_test_scope();
    let count = TS_SHARED_DOCUMENT_SEED_MAX_ENTRIES + 7;
    for ix in 0..count {
        store_shared_prepared_document_seed(&seed(scope, ix as u64));
    }
    assert_latest_seeds(scope, count);
}

#[test]
fn shared_seed_churn_keeps_the_latest_64_documents() {
    reset_prepared_syntax_cache();
    let scope = syntax_test_scope();
    for ix in 0..1000 {
        store_shared_prepared_document_seed(&seed(scope, ix));
    }
    assert_latest_seeds(scope, 1000);
}

#[test]
fn replacing_shared_seeds_preserves_other_documents_and_fifo_age() {
    reset_prepared_syntax_cache();
    let scope = syntax_test_scope();
    let capacity = TS_SHARED_DOCUMENT_SEED_MAX_ENTRIES;
    for ix in 0..capacity {
        store_shared_prepared_document_seed(&seed(scope, ix as u64));
    }
    for ix in 0..capacity {
        let mut replacement = seed(scope, ix as u64);
        replacement.line_token_chunks.insert(0, vec![Arc::from([])]);
        store_shared_prepared_document_seed(&replacement);
        assert_latest_seeds(scope, capacity);
        let has_completed_chunk = shared_prepared_document_seed_store()
            .lock()
            .unwrap()
            .get(&replacement.cache_key)
            .map(|document| document.line_token_chunks.contains_key(&0));
        assert_eq!(has_completed_chunk, Some(true));
    }
    // Replacing an existing seed must not leave duplicate order entries that
    // eventually evict a newer incarnation or allow the store to grow.
    for ix in capacity..capacity * 3 {
        store_shared_prepared_document_seed(&seed(scope, ix as u64));
        assert_latest_seeds(scope, ix + 1);
    }
}

#[test]
fn shared_seed_churn_and_scope_retirement_leave_other_scopes_intact() {
    reset_prepared_syntax_cache();
    let own_scope = syntax_test_scope();
    let capacity = TS_SHARED_DOCUMENT_SEED_MAX_ENTRIES;
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (retire_tx, retire_rx) = std::sync::mpsc::channel();
    let (foreign_scope, own_before_retirement, foreign_before_retirement, foreign_at_exit) =
        std::thread::scope(|threads| {
            let thread = threads.spawn(move || {
                reset_prepared_syntax_cache();
                let foreign_scope = syntax_test_scope();
                for ix in 0..capacity {
                    store_shared_prepared_document_seed(&seed(foreign_scope, ix as u64));
                }
                ready_tx.send(foreign_scope).ok()?;
                retire_rx.recv_timeout(Duration::from_secs(3)).ok()?;
                Some(seed_presence(foreign_scope, capacity))
            });
            let foreign_scope = ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            for ix in 0..1000 {
                store_shared_prepared_document_seed(&seed(own_scope, ix));
            }
            let own_before_retirement = seed_presence(own_scope, 1000);
            let foreign_before_retirement = seed_presence(foreign_scope, capacity);
            retire_tx.send(()).unwrap();
            let foreign_at_exit = thread
                .join()
                .unwrap()
                .expect("foreign scope must stay live");
            (
                foreign_scope,
                own_before_retirement,
                foreign_before_retirement,
                foreign_at_exit,
            )
        });

    // Assert only after releasing the shared mutex and retiring the other
    // thread, so a regression cannot poison the store or leak a waiting scope.
    assert_latest_seed_presence(&own_before_retirement);
    assert_latest_seed_presence(&foreign_before_retirement);
    assert_latest_seed_presence(&foreign_at_exit);
    for present in seed_presence(foreign_scope, capacity) {
        assert!(!present);
    }
    assert_latest_seeds(own_scope, 1000);
}
