use super::*;

/// The `#[ctor]` really did beat the harness.
///
/// Everything else in this binary depends on it: if the install slipped to
/// after the first test body, tests that build a `Query` straight off a
/// `LANGUAGE` would allocate through libc and free through `mi_free`, which
/// with `MI_DEBUG` off corrupts the heap silently rather than aborting. That
/// failure would surface as unrelated flakiness somewhere else entirely, so
/// assert the precondition here where the message can say what broke.
#[test]
fn tree_sitter_allocator_is_installed_before_any_test_runs() {
    assert!(
        gitcomet_tree_sitter_alloc::is_installed(),
        "gitcomet-tree-sitter-alloc's `install_before_main` #[ctor] should have \
             run before libtest's main; without it this binary switches \
             allocators while tests are already holding tree-sitter allocations",
    );
}

/// Two documents whose injections agree on everything but the grammar must
/// not be able to answer for each other.
///
/// `TS_INJECTION_CACHE` is a thread-local shared by every document, so in a
/// diff both sides of a file land in it together. Changing a fence from
/// ```` ```html ```` to ```` ```bash ```` is a same-length edit: the fenced
/// bytes keep their offsets *and* their content, so range and `content_hash`
/// both match across the two revisions and only `language` moves. Without
/// `document_hash` in the key both entries pass every filter in
/// `injected_syntax_pair_at` and the tie-break on width cannot separate them
/// either -- whichever the hash map yielded first won, so a click in the
/// bash block could be answered by the html grammar.
#[test]
fn injection_cache_separates_same_bytes_under_different_grammars() {
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    let body = "<p>a</p>\n";
    let html_doc = format!("# t\n\n```html\n{body}```\n");
    let bash_doc = format!("# t\n\n```bash\n{body}```\n");
    // The premise: only the four info-string bytes differ, so every injected
    // byte keeps its offset and its value.
    assert_eq!(html_doc.len(), bash_doc.len());
    assert_eq!(
        html_doc.replace("html", "____"),
        bash_doc.replace("bash", "____"),
    );

    let tokenize_all = |text: &str| {
        let document = prepare_test_document(DiffSyntaxLanguage::Markdown, text);
        for line_ix in 0..text.lines().count() {
            let _ = syntax_tokens_for_prepared_document_line(document, line_ix);
        }
        document
    };

    tokenize_all(&html_doc);
    let bash_document = tokenize_all(&bash_doc);

    let keys: Vec<TreesitterInjectionMatch> =
        TS_INJECTION_CACHE.with(|cache| cache.borrow().keys().copied().collect());

    // Both grammars cached the identical span, which is the collision this
    // key exists to survive. If markdown ever stops injecting fences by
    // their info string this stops testing anything, so assert it directly.
    let mut collided = keys.iter().filter(|key| {
        keys.iter().any(|other| {
            other.language != key.language
                && other.byte_start == key.byte_start
                && other.byte_end == key.byte_end
                && other.content_hash == key.content_hash
        })
    });
    let one = collided
        .next()
        .expect("both fences should cache the same span under different grammars");
    let two = collided
        .next()
        .expect("the collision needs both halves to be present");
    assert_ne!(
        one.document_hash, two.document_hash,
        "identical injected bytes under different grammars must still be \
             distinguishable, or the pair lookup picks by hash-map order: {one:?} vs {two:?}",
    );

    // And the click that motivated all this resolves against its own
    // document rather than the sibling revision still sitting in the cache.
    let fence_line_ix = 3;
    let open_angle = body.find('<').expect("the tag opens the injected line");
    let pair =
        prepared_document_syntax_pair_at_display_offset(bash_document, fence_line_ix, open_angle);
    assert!(
        pair.is_none(),
        "bash owns these bytes and has no tag pair in them, but the html \
             revision's tree answered: {pair:?}",
    );

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}

#[test]
fn injection_cache_identity_includes_the_host_language() {
    reset_prepared_syntax_cache();
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    let text = "<script lang=\"tsx\">\n<Widget></Widget>\n</script>\n";
    let html = prepare_test_document(DiffSyntaxLanguage::Html, text);
    let vue = prepare_test_document(DiffSyntaxLanguage::Vue, text);
    for document in [html, vue] {
        for line_ix in 0..text.lines().count() {
            let _ = syntax_tokens_for_prepared_document_line(document, line_ix);
        }
    }

    let keys = TS_INJECTION_CACHE.with(|cache| {
        cache
            .borrow()
            .keys()
            .filter(|key| {
                matches!(
                    key.language,
                    DiffSyntaxLanguage::JavaScript | DiffSyntaxLanguage::Tsx
                )
            })
            .copied()
            .collect::<Vec<_>>()
    });
    let javascript = keys
        .iter()
        .find(|key| key.language == DiffSyntaxLanguage::JavaScript)
        .expect("HTML should inject the script body as JavaScript");
    let tsx = keys
        .iter()
        .find(|key| key.language == DiffSyntaxLanguage::Tsx)
        .expect("Vue should honor lang=tsx");
    assert_eq!(javascript.byte_start, tsx.byte_start);
    assert_eq!(javascript.byte_end, tsx.byte_end);
    assert_eq!(javascript.content_hash, tsx.content_hash);
    assert_ne!(
        javascript.document_hash, tsx.document_hash,
        "identical source bytes parsed under different host grammars need distinct identities"
    );

    let html_pair = prepared_document_syntax_pair_at_display_offset(html, 1, 0)
        .expect("HTML's JavaScript injection should retain its JSX tag tree");
    assert_eq!(html_pair.kind, SyntaxPairKind::Tag);
    let vue_pair = prepared_document_syntax_pair_at_display_offset(vue, 1, 0)
        .expect("Vue's TSX injection should pair the component tags");
    assert_eq!(vue_pair.kind, SyntaxPairKind::Tag);

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}

#[test]
fn reused_prefix_chunks_carry_injection_trees_to_the_new_revision() {
    reset_prepared_syntax_cache();
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    let line_count = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 3;
    let mut lines = vec!["plain markdown".to_owned(); line_count];
    lines[0] = "```html".to_owned();
    lines[1] = "<div>unchanged</div>".to_owned();
    lines[2] = "```".to_owned();
    let base_text = lines.join("\n");
    let base_document = prepare_test_document(DiffSyntaxLanguage::Markdown, &base_text);
    let _ = syntax_tokens_for_prepared_document_line(base_document, 1)
        .expect("the injected prefix should materialize");

    let base_hash = base_document.cache_key.doc_hash;
    assert!(TS_INJECTION_CACHE.with(|cache| {
        cache
            .borrow()
            .keys()
            .any(|key| key.document_hash == base_hash && key.language == DiffSyntaxLanguage::Html)
    }));

    let edited_line = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2;
    lines[edited_line].push_str(" edited");
    let edited_text = lines.join("\n");
    let PrepareTreesitterDocumentResult::Ready(reparsed_document) =
        prepare_test_document_with_budget_reuse(
            DiffSyntaxLanguage::Markdown,
            &edited_text,
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(200),
            },
            Some(base_document),
        )
    else {
        panic!("later edit should reparse successfully");
    };
    assert_ne!(base_document.cache_key, reparsed_document.cache_key);
    assert_eq!(
        prepared_syntax_loaded_chunk_count(reparsed_document),
        1,
        "the already-materialized prefix chunk should be reused"
    );

    let reparsed_hash = reparsed_document.cache_key.doc_hash;
    assert!(
        TS_INJECTION_CACHE.with(|cache| cache.borrow().keys().any(|key| {
            key.document_hash == reparsed_hash && key.language == DiffSyntaxLanguage::Html
        })),
        "the reused prefix's injection tree should be re-keyed to the new revision"
    );
    let pair = prepared_document_syntax_pair_at_display_offset(reparsed_document, 1, 0)
        .expect("pair lookup should retain the unchanged injected prefix tree");
    assert_eq!(pair.kind, SyntaxPairKind::Tag);
    assert_eq!(pair.open[0].display_range, 0..5);
    assert_eq!(pair.close[0].display_range, 14..20);

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// Prepared token chunks outlive the small injection-tree LRU. Returning to
/// an early fence must rebuild its tree for pair lookup even though asking
/// for that line's already-cached tokens does no work.
#[test]
fn prepared_pair_lookup_rebuilds_an_evicted_injection_tree() {
    reset_prepared_syntax_cache();
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    let mut lines = Vec::new();
    let mut body_line_indices = Vec::new();
    let mut bodies = Vec::new();
    for ix in 0..=TS_INJECTION_CACHE_MAX_ENTRIES {
        lines.push("```html".to_owned());
        body_line_indices.push(lines.len());
        let body = format!(r#"<div data-index="{ix}">value</div>"#);
        lines.push(body.clone());
        bodies.push(body);
        lines.push("```".to_owned());
        lines.push(String::new());
    }
    let text = lines.join("\n");
    let document = prepare_test_document(DiffSyntaxLanguage::Markdown, &text);

    // Materialize every chunk so more distinct fence injections are parsed
    // than the LRU can retain. The first region is then necessarily among
    // the least-recently-used half evicted on overflow.
    for &line_ix in &body_line_indices {
        let _ = syntax_tokens_for_prepared_document_line(document, line_ix)
            .expect("every fenced body line should have prepared tokens");
    }
    let first_body_offset = text.find(&bodies[0]).expect("first fenced body");
    assert!(
        TS_INJECTION_CACHE.with(|cache| !cache.borrow().keys().any(|key| {
            key.document_hash == document.cache_key.doc_hash
                && first_body_offset >= key.byte_start
                && first_body_offset < key.byte_end
        })),
        "the test must evict the first fence's injection tree before lookup"
    );

    let pair = prepared_document_syntax_pair_at_display_offset(document, body_line_indices[0], 0)
        .expect("pair lookup should rebuild the evicted HTML injection tree");
    let first_body = &bodies[0];
    assert_eq!(pair.kind, SyntaxPairKind::Tag);
    assert_eq!(
        pair.open[0].display_range,
        0..first_body.find('>').expect("opening tag end") + 1
    );
    let close_start = first_body.rfind("</div>").expect("closing tag");
    assert_eq!(
        pair.close[0].display_range,
        close_start..close_start + "</div>".len()
    );
    assert!(
        TS_INJECTION_CACHE.with(|cache| cache.borrow().keys().any(|key| {
            key.document_hash == document.cache_key.doc_hash
                && first_body_offset >= key.byte_start
                && first_body_offset < key.byte_end
        })),
        "the rebuilt injection should be retained for the next lookup"
    );

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// The surviving cache entry at a click can be only the outer layer. Pair
/// lookup must still inspect it and recreate an evicted nested layer rather
/// than accepting the outer grammar's answer.
#[test]
fn prepared_pair_lookup_rebuilds_an_evicted_nested_injection_tree() {
    reset_prepared_syntax_cache();
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    let mut lines = vec!["```html".to_owned()];
    let mut script_line_indices = Vec::new();
    let mut script_bodies = Vec::new();
    for ix in 0..=TS_INJECTION_CACHE_MAX_ENTRIES {
        lines.push("<script>".to_owned());
        script_line_indices.push(lines.len());
        let body = format!("const value{ix} = ({ix});");
        lines.push(body.clone());
        script_bodies.push(body);
        lines.push("</script>".to_owned());
    }
    lines.push("```".to_owned());
    let text = lines.join("\n");
    let document = prepare_test_document(DiffSyntaxLanguage::Markdown, &text);
    for &line_ix in &script_line_indices {
        let _ = syntax_tokens_for_prepared_document_line(document, line_ix)
            .expect("every nested script line should have prepared tokens");
    }

    let first_body_offset = text
        .find(&script_bodies[0])
        .expect("first nested script body");
    TS_INJECTION_CACHE.with(|cache| {
        let cache = cache.borrow();
        assert!(
            cache.keys().any(|key| {
                key.document_hash == document.cache_key.doc_hash
                    && key.language == DiffSyntaxLanguage::Html
                    && first_body_offset >= key.byte_start
                    && first_body_offset < key.byte_end
            }),
            "the outer HTML injection must survive for this nested-cache regression"
        );
        assert!(
            !cache.keys().any(|key| {
                key.document_hash == document.cache_key.doc_hash
                    && key.language == DiffSyntaxLanguage::JavaScript
                    && first_body_offset >= key.byte_start
                    && first_body_offset < key.byte_end
            }),
            "the first nested JavaScript tree must be evicted before lookup"
        );
    });

    let first_body = &script_bodies[0];
    let open = first_body.find('(').expect("opening parenthesis");
    let close = first_body.rfind(')').expect("closing parenthesis");
    let pair =
        prepared_document_syntax_pair_at_display_offset(document, script_line_indices[0], open)
            .expect("pair lookup should rebuild the evicted nested JavaScript tree");
    assert_eq!(pair.kind, SyntaxPairKind::Bracket);
    assert_eq!(pair.open[0].display_range, open..open + 1);
    assert_eq!(pair.close[0].display_range, close..close + 1);

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}

#[test]
fn injection_cache_lru_eviction_preserves_recent_entries() {
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    // Fill the cache to max capacity with distinct entries, using the
    // global counter so access values are monotonically ordered.
    for i in 0..TS_INJECTION_CACHE_MAX_ENTRIES {
        let key = TreesitterInjectionMatch {
            document_hash: 0,
            language: DiffSyntaxLanguage::JavaScript,
            byte_start: i * 100,
            byte_end: i * 100 + 50,
            content_hash: i as u64,
        };
        let access = next_injection_access();
        TS_INJECTION_CACHE.with(|cache| {
            cache.borrow_mut().insert(
                key,
                CachedInjection {
                    all_line_tokens: vec![],
                    injection_line_starts: vec![],
                    injection_start_line_ix: 0,
                    tree: empty_injection_tree(),
                    last_access: access,
                },
            );
        });
    }

    // Access the first entry to make it "recent" (higher counter than all others).
    let first_key = TreesitterInjectionMatch {
        document_hash: 0,
        language: DiffSyntaxLanguage::JavaScript,
        byte_start: 0,
        byte_end: 50,
        content_hash: 0,
    };
    TS_INJECTION_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow_mut().get_mut(&first_key) {
            entry.last_access = next_injection_access();
        }
    });

    // Now insert one more to trigger eviction.
    let overflow_key = TreesitterInjectionMatch {
        document_hash: 0,
        language: DiffSyntaxLanguage::JavaScript,
        byte_start: 99900,
        byte_end: 99950,
        content_hash: 99999,
    };
    let access = next_injection_access();
    TS_INJECTION_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= TS_INJECTION_CACHE_MAX_ENTRIES {
            let mut entries: Vec<_> = cache.iter().map(|(k, v)| (*k, v.last_access)).collect();
            entries.sort_unstable_by_key(|(_, a)| *a);
            let evict_count = entries.len() / 2;
            for (key, _) in entries.into_iter().take(evict_count) {
                cache.remove(&key);
            }
        }
        cache.insert(
            overflow_key,
            CachedInjection {
                all_line_tokens: vec![],
                injection_line_starts: vec![],
                injection_start_line_ix: 0,
                tree: empty_injection_tree(),
                last_access: access,
            },
        );
    });

    TS_INJECTION_CACHE.with(|cache| {
        let cache = cache.borrow();
        // The recently-accessed first entry should survive eviction.
        assert!(
            cache.contains_key(&first_key),
            "recently accessed entry should survive LRU eviction"
        );
        // The new entry should be present.
        assert!(
            cache.contains_key(&overflow_key),
            "newly inserted entry should be present"
        );
        // Cache should be below max.
        assert!(
            cache.len() <= TS_INJECTION_CACHE_MAX_ENTRIES,
            "cache should not exceed max entries"
        );
    });

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}
