use crate::perf_alloc::measure_allocations;

use super::*;

#[test]
fn crlf_preview_indexes_keep_tail_metadata_across_scan_chunks() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let text = format!(
        "{}\r\nlf\ninterior\rcr\nfinal\r",
        "x".repeat(WORKTREE_PREVIEW_INDEX_SCAN_BUFFER_BYTES - 1)
    );
    std::fs::write(file.path(), &text).unwrap();
    let request = TextDecodeRequest {
        kind: gitcomet_core::text_format::SideKind::Worktree,
        attributes: Arc::default(),
        encoding: None,
    };
    let indexed = index_worktree_preview_file(file.path(), &request).unwrap();
    let (_, memory_flags) = index_text_lines(&text);
    for flags in [indexed.line_flags.as_ref(), memory_flags.as_slice()] {
        assert_eq!(
            flags
                .iter()
                .map(|&f| preview_line_has_trailing_cr(f))
                .collect::<Vec<_>>(),
            [true, false, false, true]
        );
    }
}

#[test]
fn review_binary_preview_stops_before_indexing_the_file() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), b"\0\xff\n".repeat(1024 * 1024)).unwrap();
    let request = TextDecodeRequest {
        kind: gitcomet_core::text_format::SideKind::Worktree,
        attributes: Arc::default(),
        encoding: None,
    };
    struct CountReads<R> {
        reader: R,
        bytes: usize,
    }
    impl<R: std::io::Read> std::io::Read for CountReads<R> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = self.reader.read(out)?;
            self.bytes += n;
            Ok(n)
        }
    }
    let mut reader = CountReads {
        reader: std::fs::File::open(file.path()).unwrap(),
        bytes: 0,
    };
    let result = index_worktree_preview_reader(
        file.path(),
        &request,
        &file.as_file().metadata().unwrap(),
        &mut reader,
    );
    assert_eq!(result.err().as_deref(), Some(BINARY_PREVIEW_MESSAGE));
    assert!(
        reader.bytes <= WORKTREE_PREVIEW_INDEX_SCAN_BUFFER_BYTES,
        "read {} bytes of a known binary file",
        reader.bytes
    );
}

#[test]
fn conflict_preview_mapping_decodes_identical_payloads_once() {
    let decodes = std::cell::Cell::new(0_usize);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let images = map_deduplicated_conflict_preview_sides(
        [Some(vec![1, 2, 3]), Some(vec![4, 5]), Some(vec![1, 2, 3])],
        &cancel,
        |bytes| {
            decodes.set(decodes.get() + 1);
            Some(bytes.len())
        },
    );

    assert_eq!(images, [Some(3), Some(2), Some(3)]);
    assert_eq!(decodes.get(), 2);
}

#[test]
fn build_conflict_markdown_preview_documents_parses_each_side() {
    let documents = build_conflict_markdown_preview_documents(&ThreeWaySides {
        base: "# Base\n".into(),
        ours: "- item\n".into(),
        theirs: "plain text".into(),
    });

    assert!(matches!(documents.base, Loadable::Ready(_)));
    assert!(matches!(documents.ours, Loadable::Ready(_)));
    assert!(matches!(documents.theirs, Loadable::Ready(_)));
}

#[test]
fn worktree_preview_index_line_capacity_hint_is_bounded_for_massive_files() {
    assert_eq!(worktree_preview_index_line_capacity_hint(0), 1);
    assert_eq!(worktree_preview_index_line_capacity_hint(128), 3);
    assert_eq!(
        worktree_preview_index_line_capacity_hint(usize::MAX),
        WORKTREE_PREVIEW_INDEX_LINE_CAPACITY_MAX
    );
}

#[test]
fn materialized_preview_line_raw_text_avoids_full_source_copy() {
    let source: SharedString = "x".repeat(1024 * 1024).into();
    let iterations = 4u64;

    let ((copied_len, copied_slice_len), copied_metrics) = measure_allocations(|| {
        let mut len = 0usize;
        let mut slice_len = 0usize;
        for _ in 0..iterations as usize {
            let copied_source: Arc<str> = Arc::from(source.as_ref());
            let line = gitcomet_core::file_diff::FileDiffLineText::shared_slice(
                copied_source,
                0..source.len(),
            );
            len = len.wrapping_add(line.len());
            slice_len =
                slice_len.wrapping_add(line.slice_bytes(0..16).map_or(0, |slice| slice.len()));
        }
        (len, slice_len)
    });

    // `measure_allocations` watches a process-global allocator, so a
    // parallel test can only ever inflate a sample: keep the smallest.
    let ((shared_len, shared_slice_len), shared_metrics) = (0..3)
        .map(|_| {
            measure_allocations(|| {
                let mut len = 0usize;
                let mut slice_len = 0usize;
                for _ in 0..iterations as usize {
                    let line =
                        worktree_preview_materialized_line_raw_text(&source, 0..source.len());
                    len = len.wrapping_add(line.len());
                    slice_len = slice_len
                        .wrapping_add(line.slice_bytes(0..16).map_or(0, |slice| slice.len()));
                }
                (len, slice_len)
            })
        })
        .min_by_key(|(_, metrics)| metrics.alloc_bytes)
        .expect("at least one sample");

    assert_eq!(copied_len, source.len() * iterations as usize);
    assert_eq!(copied_slice_len, 16 * iterations as usize);
    assert_eq!(shared_len, source.len() * iterations as usize);
    assert_eq!(shared_slice_len, 16 * iterations as usize);
    assert!(
        copied_metrics.alloc_bytes >= source.len() as u64 * iterations,
        "copying baseline should allocate the source each time: {copied_metrics:?}"
    );
    assert!(
        shared_metrics.alloc_bytes.saturating_mul(8) < copied_metrics.alloc_bytes,
        "materialized preview row lookup should stay far below full-source copy cost: shared={shared_metrics:?} copied={copied_metrics:?}"
    );
}

#[test]
fn build_conflict_markdown_preview_documents_reports_per_side_size_limits() {
    let documents = build_conflict_markdown_preview_documents(&ThreeWaySides {
        base: "x"
            .repeat(crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES + 1)
            .into(),
        ours: "".into(),
        theirs: "".into(),
    });

    let Loadable::Error(message) = documents.base else {
        panic!("expected oversize base preview to error: {documents:?}");
    };
    assert!(
        message.contains("1 MiB"),
        "should mention size limit: {message}"
    );
    assert!(matches!(documents.ours, Loadable::Ready(_)));
    assert!(matches!(documents.theirs, Loadable::Ready(_)));
}

#[test]
fn build_conflict_markdown_preview_documents_handles_empty_sources() {
    let documents = build_conflict_markdown_preview_documents(&ThreeWaySides {
        base: "".into(),
        ours: "".into(),
        theirs: "".into(),
    });

    // Empty sources should still produce Ready documents, not errors.
    assert!(matches!(documents.base, Loadable::Ready(_)));
    assert!(matches!(documents.ours, Loadable::Ready(_)));
    assert!(matches!(documents.theirs, Loadable::Ready(_)));
}

#[test]
fn conflict_markdown_preview_state_document_returns_correct_side() {
    let state = ConflictResolverMarkdownPreviewState {
        source_hash: Some(42),
        columns: ThreeWaySides::default(),
        documents: build_conflict_markdown_preview_documents(&ThreeWaySides {
            base: "# Base".into(),
            ours: "# Ours".into(),
            theirs: "# Theirs".into(),
        }),
    };

    // Each side should have its own document with the expected content.
    let base = state.document(ThreeWayColumn::Base);
    let ours = state.document(ThreeWayColumn::Ours);
    let theirs = state.document(ThreeWayColumn::Theirs);

    let base_doc = match base {
        Loadable::Ready(d) => d,
        _ => panic!("expected Ready for base"),
    };
    let ours_doc = match ours {
        Loadable::Ready(d) => d,
        _ => panic!("expected Ready for ours"),
    };
    let theirs_doc = match theirs {
        Loadable::Ready(d) => d,
        _ => panic!("expected Ready for theirs"),
    };

    assert!(base_doc.rows[0].text.contains("Base"));
    assert!(ours_doc.rows[0].text.contains("Ours"));
    assert!(theirs_doc.rows[0].text.contains("Theirs"));
}
