use super::*;
use std::path::{Path, PathBuf};

#[test]
fn crlf_indexed_long_lines_need_no_tail_reads_across_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.txt");
    // Put CR and LF on opposite sides of the scanner's buffer boundary.
    let prefix = "x".repeat(FILE_DIFF_INDEX_SCAN_BUFFER_BYTES - 1);
    for ending in ["\r", " "] {
        std::fs::write(&path, format!("{prefix}{ending}\nsecond\r\nthird\n")).unwrap();
        let source = IndexedFileDiffSource::from_file(&path).unwrap();
        gitcomet_core::file_diff::take_file_slice_bytes_read_for_tests();
        for _ in 0..10 {
            assert_eq!(
                source.line_text(0).unwrap().has_trailing_cr(),
                ending == "\r"
            );
            assert!(source.line_text(1).unwrap().has_trailing_cr());
            assert!(!source.line_text(2).unwrap().has_trailing_cr());
        }
        assert_eq!(
            gitcomet_core::file_diff::take_file_slice_bytes_read_for_tests(),
            0
        );
    }
}

#[test]
fn file_backed_lines_are_read_once_across_frames() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("source.txt");
    std::fs::write(&path, "zero\none\ntwo\n").expect("write source");
    let source = IndexedFileDiffSource::from_file(&path).expect("index source");
    let _ = gitcomet_core::file_diff::take_file_slice_opens_for_tests();
    // Each frame asks a clone of the source for the row's text again.
    for source in [source.clone(), source] {
        assert_eq!(source.line_text(1).expect("line").as_str(), "one");
    }
    assert_eq!(
        gitcomet_core::file_diff::take_file_slice_opens_for_tests(),
        1
    );
}

fn streamed_file_diff_source_for_test_with_mode(
    old: &str,
    new: &str,
    whitespace_mode: DiffWhitespaceMode,
) -> Arc<StreamedFileDiffSource> {
    let old_text_arc = Arc::<str>::from(old);
    let new_text_arc = Arc::<str>::from(new);
    let old_source = IndexedFileDiffSource::from_shared(Some(&old_text_arc));
    let new_source = IndexedFileDiffSource::from_shared(Some(&new_text_arc));
    let old_text = file_diff_source_text(&old_source);
    let new_text = file_diff_source_text(&new_source);
    let plan = Arc::new(build_file_diff_plan_from_document_sources(
        &old_text,
        old_source.line_starts.as_ref(),
        &new_text,
        new_source.line_starts.as_ref(),
    ));
    Arc::new(StreamedFileDiffSource::new(
        plan,
        old_source,
        new_source,
        whitespace_mode,
    ))
}

fn streamed_file_diff_source_for_test(old: &str, new: &str) -> Arc<StreamedFileDiffSource> {
    streamed_file_diff_source_for_test_with_mode(old, new, DiffWhitespaceMode::Show)
}

fn prepare_test_document(
    language: rows::DiffSyntaxLanguage,
    text: &str,
) -> rows::PreparedDiffSyntaxDocument {
    let text: SharedString = text.to_owned().into();
    let line_starts = Arc::from(build_line_starts(text.as_ref()));
    match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        language,
        rows::DiffSyntaxMode::Auto,
        text.clone(),
        Arc::clone(&line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_millis(50),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        rows::PrepareDiffSyntaxDocumentResult::TimedOut => {
            rows::inject_background_prepared_diff_syntax_document(
                rows::prepare_diff_syntax_document_in_background_text(
                    language,
                    rows::DiffSyntaxMode::Auto,
                    text,
                    line_starts,
                )
                .expect("background parse should be available for supported test documents"),
            )
        }
        rows::PrepareDiffSyntaxDocumentResult::Unsupported => {
            panic!("test document should support prepared syntax parsing")
        }
    }
}

#[test]
fn build_inline_text_joins_lines_with_trailing_newline() {
    let rows = vec![
        AnnotatedDiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Header,
            text: "diff --git a/file b/file".into(),
            old_line: None,
            new_line: None,
        },
        AnnotatedDiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Remove,
            text: "-old".into(),
            old_line: Some(1),
            new_line: None,
        },
        AnnotatedDiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Add,
            text: "+new".into(),
            old_line: None,
            new_line: Some(1),
        },
    ];

    let text = build_inline_text(rows.as_slice());
    assert_eq!(text.as_ref(), "diff --git a/file b/file\n-old\n+new\n");
}

#[test]
fn build_inline_text_returns_empty_for_empty_rows() {
    let text = build_inline_text(&[]);
    assert!(text.as_ref().is_empty());
}

#[test]
fn file_diff_visual_kinds_ignore_whitespace_only_changes() {
    let source = streamed_file_diff_source_for_test_with_mode(
        "let\tvalue = 1;\n",
        "let value=1;\n",
        DiffWhitespaceMode::Ignore,
    );
    let split = PagedFileDiffRows::new(Arc::clone(&source), 1);
    let inline = PagedFileDiffInlineRows::new(Arc::clone(&source), 1);

    let split_raw = split
        .slice(0, split.len_hint())
        .map(|row| row.kind)
        .collect::<Vec<_>>();
    let inline_raw = inline
        .slice(0, inline.len_hint())
        .map(|row| row.kind)
        .collect::<Vec<_>>();
    let split_visual = (0..split.len_hint())
        .map(|ix| split.visual_kind(ix).expect("split visual kind"))
        .collect::<Vec<_>>();
    let inline_visual = (0..inline.len_hint())
        .map(|ix| inline.visual_kind(ix).expect("inline visual kind"))
        .collect::<Vec<_>>();

    assert_eq!(
        split_raw,
        vec![gitcomet_core::file_diff::FileDiffRowKind::Modify]
    );
    assert_eq!(
        inline_raw,
        vec![
            gitcomet_core::domain::DiffLineKind::Remove,
            gitcomet_core::domain::DiffLineKind::Add
        ]
    );
    assert_eq!(
        split_visual,
        vec![gitcomet_core::file_diff::FileDiffRowKind::Context]
    );
    assert_eq!(
        inline_visual,
        vec![
            gitcomet_core::domain::DiffLineKind::Context,
            gitcomet_core::domain::DiffLineKind::Context
        ]
    );
    assert!(split.change_blocks().is_empty());
    assert!(inline.change_blocks().is_empty());
    assert!(split.scrollbar_markers().is_empty());
    assert!(inline.scrollbar_markers().is_empty());
}

#[test]
fn file_diff_visual_kinds_ignore_line_break_only_changes() {
    let source = streamed_file_diff_source_for_test_with_mode(
        "foo\nbar\n",
        "foobar\n",
        DiffWhitespaceMode::Ignore,
    );
    let split = PagedFileDiffRows::new(Arc::clone(&source), 1);
    let inline = PagedFileDiffInlineRows::new(Arc::clone(&source), 1);

    let split_visual = (0..split.len_hint())
        .map(|ix| split.visual_kind(ix).expect("split visual kind"))
        .collect::<Vec<_>>();
    let inline_visual = (0..inline.len_hint())
        .map(|ix| inline.visual_kind(ix).expect("inline visual kind"))
        .collect::<Vec<_>>();

    assert!(!split_visual.is_empty());
    assert!(!inline_visual.is_empty());
    assert!(
        split_visual
            .iter()
            .all(|kind| *kind == gitcomet_core::file_diff::FileDiffRowKind::Context)
    );
    assert!(
        inline_visual
            .iter()
            .all(|kind| *kind == gitcomet_core::domain::DiffLineKind::Context)
    );
    assert!(split.change_blocks().is_empty());
    assert!(inline.change_blocks().is_empty());
}

#[test]
fn file_diff_change_blocks_span_each_contiguous_run() {
    let source = streamed_file_diff_source_for_test(
        "alpha\nold one\nold two\nmiddle\nold three\nomega\n",
        "alpha\nnew one\nnew two\nmiddle\nnew three\nomega\n",
    );
    let split = PagedFileDiffRows::new(Arc::clone(&source), 1);
    let inline = PagedFileDiffInlineRows::new(Arc::clone(&source), 1);

    assert_eq!(split.change_blocks(), vec![1..3, 4..5]);
    // Inline interleaves each modified pair as `-`/`+`.
    assert_eq!(inline.change_blocks(), vec![1..5, 6..8]);
}

#[test]
fn file_diff_change_blocks_drop_whitespace_only_blocks_when_ignored() {
    let old_text = "alpha\nold one\nmiddle\n  keep\nomega\n";
    let new_text = "alpha\nnew one\nmiddle\nkeep\nomega\n";

    let shown = streamed_file_diff_source_for_test(old_text, new_text);
    let shown_starts = PagedFileDiffRows::new(Arc::clone(&shown), 1)
        .change_blocks()
        .into_iter()
        .map(|block| block.start)
        .collect::<Vec<_>>();
    assert_eq!(shown_starts, vec![1, 3]);

    let ignored = streamed_file_diff_source_for_test_with_mode(
        old_text,
        new_text,
        DiffWhitespaceMode::Ignore,
    );
    assert_eq!(
        PagedFileDiffRows::new(Arc::clone(&ignored), 1).change_blocks(),
        vec![1..2]
    );
    assert_eq!(
        PagedFileDiffInlineRows::new(Arc::clone(&ignored), 1).change_blocks(),
        vec![1..3]
    );
}

#[test]
fn file_diff_lines_from_starts_matches_std_lines_semantics() {
    for text in [
        "",
        "alpha",
        "\n",
        "alpha\n",
        "alpha\n\n",
        "alpha\nbeta",
        "alpha\nbeta\n",
        "alpha\n\nbeta\n",
    ] {
        let line_starts = build_line_starts(text);
        assert_eq!(
            file_diff_lines_from_starts(text, line_starts.as_slice()),
            text.lines().collect::<Vec<_>>(),
            "line slicing should keep std::str::lines semantics for {text:?}",
        );
    }
}

/// A side backed by a file on disk -- an unstaged diff's new side is the
/// worktree file -- keeps its text off the heap, which is what lets a huge
/// diff render from per-line slices. The rebuild must therefore hand back
/// the *path*, or nothing can ever parse a whole document for that side.
///
/// Without it `file_diff_old_text`/`new_text` are empty, every prepared
/// syntax document lookup returns `None`, and anything needing the whole
/// tree (matching delimiters) silently gets nothing while row colouring
/// carries on from per-line tokens.
#[test]
fn file_backed_diff_sides_report_their_source_path() {
    let dir = std::env::temp_dir().join(format!(
        "gitcomet_file_diff_syntax_text_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let new_path = dir.join("lib.rs");
    let new_body = "fn main() {\n    let v = vec![1, 2];\n}\n";
    std::fs::write(&new_path, new_body).expect("write new side");

    let file = gitcomet_core::domain::FileDiffText::new_sources(
        std::path::PathBuf::from("lib.rs"),
        None,
        Some(gitcomet_core::domain::FileDiffTextSource::new(
            new_path.clone(),
        )),
    );
    let rebuild = build_file_diff_cache_rebuild(&file, &dir)
        .expect("rebuild should succeed for a file-backed side");

    assert!(
        rebuild.new_text.is_empty(),
        "a file-backed side deliberately keeps its text off the heap"
    );
    assert_eq!(
        rebuild.new_source_path.as_deref(),
        Some(&new_path),
        "but it must report where that text can be read back from"
    );
    assert!(
        !rebuild.new_line_starts.is_empty(),
        "line starts are indexed even when the text is not held"
    );
    // And what the path points at is what the line starts describe.
    let body = std::fs::read_to_string(&new_path).expect("read back");
    assert_eq!(body, new_body);
    // Three lines plus the start following the trailing newline.
    assert_eq!(rebuild.new_line_starts.len(), 4);
    assert_eq!(rebuild.new_line_starts.first(), Some(&0));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_diff_index_line_capacity_hint_is_bounded_for_massive_files() {
    assert_eq!(file_diff_index_line_capacity_hint(0), 1);
    assert_eq!(file_diff_index_line_capacity_hint(128), 3);
    assert_eq!(
        file_diff_index_line_capacity_hint(usize::MAX),
        FILE_DIFF_INDEX_LINE_CAPACITY_MAX
    );
}

/// The staleness guard the click path leans on, against the indexer that
/// produced the index it is checking.
///
/// Pinned against `IndexedFileDiffSource` itself rather than against a
/// hand-written expectation: if the two ever disagree about the convention
/// -- a start after a trailing newline, no start for an empty file -- every
/// source-backed side would silently stop pairing instead of failing here.
#[test]
fn line_starts_describe_accepts_its_own_index_and_rejects_a_changed_file() {
    for text in [
        "alpha\nbeta\n",
        "alpha\nbeta",
        "\n",
        "one line no newline",
        "trailing\n\n\n",
    ] {
        let indexed = IndexedFileDiffSource::from_shared(Some(&Arc::from(text)));
        assert!(
            line_starts_describe(text, indexed.line_starts.as_ref()),
            "the indexer's own answer for {text:?} must be accepted",
        );
    }

    let indexed = IndexedFileDiffSource::from_shared(Some(&Arc::from("alpha\nbeta\ngamma\n")));
    let starts = indexed.line_starts.as_ref();
    // The file shrank: the index's later starts sit past the end of the
    // text, which is exactly the pairing that used to panic downstream.
    assert!(!line_starts_describe("alpha\n", starts));
    // The file grew, or a line moved: same length class, different lines.
    assert!(!line_starts_describe("alpha\nbeta\ngamma\ndelta\n", starts));
    assert!(!line_starts_describe("alphaX\nbeta\ngamma\n", starts));
    // An empty read against a real index is not a match either.
    assert!(!line_starts_describe("", starts));
}

#[test]
fn build_file_diff_cache_rebuild_preserves_real_document_sources() {
    let file = gitcomet_core::domain::FileDiffText::new(
        PathBuf::from("src/demo.rs"),
        Some("alpha\nbeta\n".to_string()),
        Some("gamma\ndelta".to_string()),
    );

    let rebuild = build_file_diff_cache_rebuild(&file, Path::new("/tmp/repo"))
        .expect("shared file diff should index");

    assert_eq!(
        rebuild.file_path,
        Some(PathBuf::from("/tmp/repo/src/demo.rs"))
    );
    assert_eq!(rebuild.language, Some(rows::DiffSyntaxLanguage::Rust));
    assert_eq!(rebuild.old_text.as_ref(), "alpha\nbeta\n");
    assert_eq!(rebuild.old_line_starts.as_ref(), &[0, 6, 11]);
    assert_eq!(rebuild.new_text.as_ref(), "gamma\ndelta");
    assert_eq!(rebuild.new_line_starts.as_ref(), &[0, 6]);
    assert_eq!(rebuild.row_provider.source_byte_lengths(), (11, 11));
}

#[test]
fn a_patch_without_hunks_diffs_the_decoded_sides_itself() {
    // What git prints for UTF-16 text without working-tree-encoding.
    let binary_patch = gitcomet_core::domain::Diff::from_unified(
        gitcomet_core::domain::DiffTarget::working_tree(
            PathBuf::from("notes.txt"),
            gitcomet_core::domain::DiffArea::Unstaged,
        ),
        "diff --git a/notes.txt b/notes.txt\nBinary files a/notes.txt and b/notes.txt differ\n",
    );
    let file = gitcomet_core::domain::FileDiffText::new(
        PathBuf::from("notes.txt"),
        Some("hello\nsame\n".to_string()),
        Some("hello\nchanged\n".to_string()),
    );
    let rebuild = build_file_diff_cache_rebuild_with_patch(
        &file,
        Path::new("/tmp/repo"),
        Some(&binary_patch),
        DiffWhitespaceMode::Show,
    )
    .expect("rebuild");
    assert!(
        rebuild
            .rows
            .iter()
            .any(|row| row.kind != gitcomet_core::file_diff::FileDiffRowKind::Context),
        "the changed line must not be shown as unchanged"
    );
}

#[test]
fn build_file_diff_cache_rebuild_reports_invalid_source_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let source_path = tmp.path().join("binary.dat");
    std::fs::write(&source_path, [0xff, 0xfe, b'\n']).expect("write invalid utf8 source");
    let file = gitcomet_core::domain::FileDiffText::new_sources(
        PathBuf::from("binary.dat"),
        None,
        Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
            source_path.clone(),
            "invalid-source",
        )),
    );

    let error = build_file_diff_cache_rebuild(&file, tmp.path())
        .expect_err("invalid UTF-8 source should be reported");

    assert_eq!(
        error,
        FileDiffCacheError::NotText {
            old_bytes: None,
            new_bytes: Some(3),
        },
        "binary content is a placeholder outcome, not an error dump"
    );
}

#[test]
fn build_file_diff_cache_rebuild_reports_unreadable_source_as_message() {
    let file = gitcomet_core::domain::FileDiffText::new_sources(
        PathBuf::from("missing.txt"),
        None,
        Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
            PathBuf::from("/nonexistent/gitcomet-missing-source"),
            "missing-source",
        )),
    );
    match build_file_diff_cache_rebuild(&file, Path::new("/tmp/repo")) {
        Err(FileDiffCacheError::Message(message)) => {
            assert!(message.contains("gitcomet-missing-source"), "{message}");
        }
        other => panic!("expected a message error, got {other:?}"),
    }
}

#[test]
fn resolved_lfs_diff_uses_content_changes_instead_of_pointer_hunks() {
    use gitcomet_core::domain::{Diff, DiffArea, DiffTarget, FileDiffText};
    use gitcomet_core::large_files::{LargeFileContent, LargeFilePointer, LargeFileSide};
    let old = (1..=10)
        .map(|i| format!("line {i:02}\n"))
        .collect::<String>();
    let new = old.replace("line 08", "edit 08");
    let side = |byte| LargeFileSide {
        pointer: LargeFilePointer::Lfs(gitcomet_core::lfs::LfsPointer {
            oid: gitcomet_core::lfs::LfsOid([byte; 32]),
            size: 80,
        }),
        content: LargeFileContent::Available,
    };
    let target = DiffTarget::working_tree("data.bin".into(), DiffArea::Unstaged);
    let patch = Diff::from_unified(
        target,
        &format!(
            "diff --git a/data.bin b/data.bin\n--- a/data.bin\n+++ b/data.bin\n@@ -1,3 +1,3 @@\n version https://git-lfs.github.com/spec/v1\n-oid sha256:{}\n+oid sha256:{}\n size 80\n",
            "01".repeat(32),
            "02".repeat(32)
        ),
    );
    let file = FileDiffText::new("data.bin".into(), Some(old), Some(new))
        .with_large_sides(Some(side(1)), Some(side(2)));
    let rebuilt = build_file_diff_cache_rebuild_with_patch(
        &file,
        Path::new("/tmp"),
        Some(&patch),
        DiffWhitespaceMode::Show,
    )
    .unwrap();
    assert_eq!(
        rebuilt.row_provider.change_blocks(),
        vec![7..8],
        "only payload line 8 changed"
    );
}

#[test]
fn indexed_file_diff_source_accepts_utf8_split_at_scan_boundary() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let source_path = tmp.path().join("split-utf8.txt");
    let mut bytes = vec![b'a'; FILE_DIFF_INDEX_SCAN_BUFFER_BYTES.saturating_sub(1)];
    bytes.extend_from_slice("é".as_bytes());
    bytes.push(b'\n');
    std::fs::write(&source_path, bytes).expect("write split utf8 source");

    let source = IndexedFileDiffSource::from_file(&source_path)
        .expect("valid UTF-8 split across scan buffers should index");

    assert_eq!(source.line_count(), 1);
    assert_eq!(
        source.source_len,
        FILE_DIFF_INDEX_SCAN_BUFFER_BYTES.saturating_add(2)
    );
}

#[test]
fn build_file_diff_cache_rebuild_source_backed_without_patch_uses_source_content() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let old_source_path = tmp.path().join("old.txt");
    let new_source_path = tmp.path().join("new.txt");
    std::fs::write(&old_source_path, "alpha\nbefore\ngamma\n").expect("write old source");
    std::fs::write(&new_source_path, "alpha\nafter\ngamma\n").expect("write new source");
    let file = gitcomet_core::domain::FileDiffText::new_sources(
        PathBuf::from("demo.txt"),
        Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
            old_source_path,
            "old",
        )),
        Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
            new_source_path,
            "new",
        )),
    );

    let rebuild = build_file_diff_cache_rebuild(&file, tmp.path())
        .expect("source-backed file diff should build without patch");

    assert_eq!(rebuild.row_provider.source_byte_lengths(), (19, 18));
    assert!(rebuild.rows.iter().any(|row| {
        row.kind != gitcomet_core::file_diff::FileDiffRowKind::Context
            && (row.old_line == Some(2) || row.new_line == Some(2))
    }));
    assert!(!rebuild.rows.iter().any(|row| {
        row.kind == gitcomet_core::file_diff::FileDiffRowKind::Context
            && row.old_line == Some(2)
            && row.new_line == Some(2)
    }));
}

#[test]
fn build_file_diff_cache_rebuild_inline_rows_keep_file_line_numbers() {
    use gitcomet_core::domain::DiffLineKind;

    let file = gitcomet_core::domain::FileDiffText::new(
        PathBuf::from("src/demo.rs"),
        Some("struct Old;\nfn keep() {}\n".to_string()),
        Some("fn keep() {}\nlet added = 42;\n".to_string()),
    );

    let rebuild = build_file_diff_cache_rebuild(&file, Path::new("/tmp/repo"))
        .expect("shared file diff should index");
    let language = rebuild
        .language
        .expect("rust path should resolve a syntax language");
    let old_document = prepare_test_document(language, rebuild.old_text.as_ref());
    let new_document = prepare_test_document(language, rebuild.new_text.as_ref());

    let remove_row = rebuild
        .inline_rows
        .iter()
        .find(|row| row.kind == DiffLineKind::Remove)
        .expect("diff should contain a remove row");
    assert_eq!(remove_row.old_line, Some(1));
    assert_eq!(
        rows::prepared_diff_syntax_line_for_inline_diff_row(
            Some(old_document),
            Some(new_document),
            remove_row,
        ),
        rows::PreparedDiffSyntaxLine {
            document: Some(old_document),
            line_ix: 0,
        }
    );

    let context_row = rebuild
        .inline_rows
        .iter()
        .find(|row| row.kind == DiffLineKind::Context)
        .expect("diff should contain a context row");
    assert_eq!(context_row.old_line, Some(2));
    assert_eq!(context_row.new_line, Some(1));
    assert_eq!(
        rows::prepared_diff_syntax_line_for_inline_diff_row(
            Some(old_document),
            Some(new_document),
            context_row,
        ),
        rows::PreparedDiffSyntaxLine {
            document: Some(new_document),
            line_ix: 0,
        }
    );

    let add_row = rebuild
        .inline_rows
        .iter()
        .find(|row| row.kind == DiffLineKind::Add)
        .expect("diff should contain an add row");
    assert_eq!(add_row.new_line, Some(2));
    assert_eq!(
        rows::prepared_diff_syntax_line_for_inline_diff_row(
            Some(old_document),
            Some(new_document),
            add_row,
        ),
        rows::PreparedDiffSyntaxLine {
            document: Some(new_document),
            line_ix: 1,
        }
    );
}

#[test]
fn streamed_file_diff_inline_full_text_matches_materialized_rows_without_paging() {
    let source =
        streamed_file_diff_source_for_test("alpha\nbeta\ngamma\n", "alpha\nbeta changed\ngamma\n");
    let provider = PagedFileDiffInlineRows::new(Arc::clone(&source), 1);

    let eager_rows = provider
        .slice(0, provider.len_hint())
        .collect::<Vec<AnnotatedDiffLine>>();
    let direct = provider.build_full_text();

    assert_eq!(direct, build_inline_text(eager_rows.as_slice()));
}
