use super::*;
use crate::view::markdown_preview::MarkdownPreviewRefusal;
use gitcomet_core::domain::{DiffArea, DiffLine, DiffLineKind, DiffTarget};
use std::path::Path;
use std::path::PathBuf;

fn patch_diff_for_visual_tests(lines: Vec<(DiffLineKind, &str)>) -> gitcomet_core::domain::Diff {
    gitcomet_core::domain::Diff {
        target: DiffTarget::WorkingTree {
            path: PathBuf::from("demo.txt"),
            area: DiffArea::Unstaged,
        },
        lines: lines
            .into_iter()
            .map(|(kind, text)| DiffLine {
                kind,
                text: text.into(),
            })
            .collect(),
    }
}

#[test]
fn patch_visual_line_kinds_ignore_whitespace_only_groups() {
    let diff = patch_diff_for_visual_tests(vec![
        (DiffLineKind::Hunk, "@@ -1 +1 @@"),
        (DiffLineKind::Remove, "-let\tvalue = 1;"),
        (DiffLineKind::Add, "+let value=1;"),
    ]);

    let visual = visual_line_kinds_for_patch_diff(&diff, DiffWhitespaceMode::Ignore);

    assert_eq!(
        visual,
        vec![
            DiffLineKind::Hunk,
            DiffLineKind::Context,
            DiffLineKind::Context
        ]
    );
}

#[test]
fn patch_visual_line_kinds_ignore_line_break_only_groups() {
    let diff = patch_diff_for_visual_tests(vec![
        (DiffLineKind::Hunk, "@@ -1,2 +1 @@"),
        (DiffLineKind::Remove, "-foo"),
        (DiffLineKind::Remove, "-bar"),
        (DiffLineKind::Add, "+foobar"),
    ]);

    let visual = visual_line_kinds_for_patch_diff(&diff, DiffWhitespaceMode::Ignore);

    assert_eq!(
        visual,
        vec![
            DiffLineKind::Hunk,
            DiffLineKind::Context,
            DiffLineKind::Context,
            DiffLineKind::Context
        ]
    );
}

#[test]
fn patch_visual_line_kinds_ignore_eof_marker_between_equal_lines() {
    let diff = patch_diff_for_visual_tests(vec![
        (DiffLineKind::Hunk, "@@ -1 +1 @@"),
        (DiffLineKind::Remove, "-foo"),
        (DiffLineKind::Context, "\\ No newline at end of file"),
        (DiffLineKind::Add, "+foo"),
    ]);

    let visual = visual_line_kinds_for_patch_diff(&diff, DiffWhitespaceMode::Ignore);

    assert_eq!(
        visual,
        vec![
            DiffLineKind::Hunk,
            DiffLineKind::Context,
            DiffLineKind::Context,
            DiffLineKind::Context
        ]
    );
}

#[test]
fn patch_visual_line_kinds_ignore_eof_marker_after_equal_lines() {
    let diff = patch_diff_for_visual_tests(vec![
        (DiffLineKind::Hunk, "@@ -1 +1 @@"),
        (DiffLineKind::Remove, "-foo"),
        (DiffLineKind::Add, "+foo"),
        (DiffLineKind::Context, "\\ No newline at end of file"),
    ]);

    let visual = visual_line_kinds_for_patch_diff(&diff, DiffWhitespaceMode::Ignore);

    assert_eq!(
        visual,
        vec![
            DiffLineKind::Hunk,
            DiffLineKind::Context,
            DiffLineKind::Context,
            DiffLineKind::Context
        ]
    );
}

#[test]
fn patch_visual_line_kinds_keep_real_content_changes() {
    let diff = patch_diff_for_visual_tests(vec![
        (DiffLineKind::Hunk, "@@ -1 +1 @@"),
        (DiffLineKind::Remove, "-let value = 1;"),
        (DiffLineKind::Add, "+let result = 1;"),
    ]);

    let visual = visual_line_kinds_for_patch_diff(&diff, DiffWhitespaceMode::Ignore);

    assert_eq!(
        visual,
        vec![DiffLineKind::Hunk, DiffLineKind::Remove, DiffLineKind::Add]
    );
}

#[test]
fn preview_source_text_from_lines_preserves_missing_trailing_newline() {
    let lines = vec![
        "fn main() {".to_string(),
        "    42".to_string(),
        "}".to_string(),
    ];
    let source_len = "fn main() {\n    42\n}".len();

    let source = preview_source_text_from_lines(&lines, source_len);
    let (_, line_starts) = preview_source_text_and_line_starts_from_lines(&lines, source_len);

    assert_eq!(source.as_ref(), "fn main() {\n    42\n}");
    assert_eq!(line_starts.as_ref(), &[0, 12, 19]);
}

#[test]
fn preview_source_text_from_lines_restores_trailing_newline() {
    let lines = vec!["alpha".to_string(), "beta".to_string()];
    let source_len = "alpha\nbeta\n".len();

    let source = preview_source_text_from_lines(&lines, source_len);
    let (_, line_starts) = preview_source_text_and_line_starts_from_lines(&lines, source_len);

    assert_eq!(source.as_ref(), "alpha\nbeta\n");
    assert_eq!(line_starts.as_ref(), &[0, 6, 11]);
}

#[test]
fn full_document_syntax_mode_is_always_auto() {
    assert_eq!(FULL_DOCUMENT_SYNTAX_MODE, rows::DiffSyntaxMode::Auto);
}

#[test]
fn file_diff_style_cache_epochs_map_rows_to_matching_side() {
    let epochs = FileDiffStyleCacheEpochs {
        split_left: 11,
        split_right: 23,
    };

    assert_eq!(
        epochs.split_epoch(crate::view::DiffTextRegion::SplitLeft),
        11
    );
    assert_eq!(
        epochs.split_epoch(crate::view::DiffTextRegion::SplitRight),
        23
    );
    assert_eq!(
        epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Remove),
        11
    );
    assert_eq!(
        epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Add),
        23
    );
    assert_eq!(
        epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Context),
        23
    );
    assert_eq!(
        epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Header),
        0
    );
    assert_eq!(
        epochs.inline_epoch(gitcomet_core::domain::DiffLineKind::Hunk),
        0
    );
}

#[test]
fn build_single_markdown_preview_document_reports_row_limit() {
    let preview_lines = vec!["---\n".repeat(crate::view::markdown_preview::MAX_PREVIEW_ROWS + 1)];
    let source =
        preview_source_text_from_lines(&preview_lines, preview_lines_source_len(&preview_lines));
    assert!(source.len() < crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES);

    let error = build_single_markdown_preview_document(source.as_ref())
        .expect_err("row-limit markdown preview should return an error");
    // The parser cap is unrecoverable: there is no parsed document to show.
    let MarkdownPreviewRefusal::Unavailable(reason) = &error else {
        panic!("parser row cap should be unavailable, got {error:?}");
    };
    assert!(
        reason.contains("row limit"),
        "row-limit markdown preview should mention the rendered row limit: {reason}"
    );
    assert!(!error.prefers_source());
}

#[test]
fn file_diff_style_cache_epochs_bump_only_changed_side() {
    let mut epochs = FileDiffStyleCacheEpochs {
        split_left: 5,
        split_right: 9,
    };

    epochs.bump_left();
    assert_eq!(
        epochs,
        FileDiffStyleCacheEpochs {
            split_left: 6,
            split_right: 9,
        }
    );

    epochs.bump_right();
    assert_eq!(
        epochs,
        FileDiffStyleCacheEpochs {
            split_left: 6,
            split_right: 10,
        }
    );

    epochs.bump_both();
    assert_eq!(
        epochs,
        FileDiffStyleCacheEpochs {
            split_left: 7,
            split_right: 11,
        }
    );
}

#[test]
fn pictures_are_measured_from_their_headers_without_decoding_them() {
    // Reading a picture's header is what lets the preview reserve its box
    // before the decode finishes. Only a local file can be measured that
    // cheaply — a remote one would have to be fetched, which is the
    // expensive half — and anything unreadable simply goes unmeasured.
    let dir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_measure_pictures",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the fixture directory");

    let mut png = std::io::Cursor::new(Vec::new());
    {
        use image::ImageEncoder as _;
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &vec![0u8; 12 * 5 * 4],
                12,
                5,
                image::ExtendedColorType::Rgba8,
            )
            .expect("encode a test png");
    }
    std::fs::write(dir.join("shot.png"), png.into_inner()).expect("write the picture");
    std::fs::write(dir.join("broken.png"), b"not a png").expect("write the broken picture");

    let document = markdown_preview::parse_markdown(
            "![a](shot.png)\n\n![b](broken.png)\n\n![c](missing.png)\n\n![d](https://example.com/x.png)\n",
        )
        .expect("the fixture parses");
    let sizes = measure_markdown_preview_pictures(&document, Some(dir.as_path()));

    assert_eq!(sizes.get("shot.png").copied(), Some((12, 5)));
    assert_eq!(sizes.get("broken.png"), None);
    assert_eq!(sizes.get("missing.png"), None);
    assert_eq!(sizes.get("https://example.com/x.png"), None);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_single_markdown_preview_document_reports_the_flowing_render_budget() {
    // The single-document preview lays every row out on every frame, so it
    // refuses a document the parser would happily produce.
    let rows = crate::view::markdown_preview::MAX_FLOWING_PREVIEW_ROWS + 1;
    let source: SharedString = "---\n".repeat(rows).into();
    assert!(rows < crate::view::markdown_preview::MAX_PREVIEW_ROWS);
    assert!(source.len() < crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES);
    assert!(
        crate::view::markdown_preview::parse_markdown(source.as_ref()).is_some(),
        "the parser itself accepts this document"
    );

    let error = build_single_markdown_preview_document(source.as_ref())
        .expect_err("a document past the flowing budget should return an error");
    // Distinct from the parser cap, and recoverable: the source still reads.
    assert_eq!(error, MarkdownPreviewRefusal::TooManyRowsToRender);
    assert!(error.prefers_source());
}

#[test]
fn build_single_markdown_preview_document_accepts_the_flowing_render_budget() {
    let source: SharedString = "---\n"
        .repeat(crate::view::markdown_preview::MAX_FLOWING_PREVIEW_ROWS)
        .into();
    let document = build_single_markdown_preview_document(source.as_ref())
        .expect("a document exactly at the budget still renders");
    assert_eq!(
        document.rows.len(),
        crate::view::markdown_preview::MAX_FLOWING_PREVIEW_ROWS
    );
}

#[test]
fn build_single_markdown_preview_document_respects_exact_source_length() {
    let mut source = "x".repeat(crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES);
    source.push('\n');
    assert_eq!(
        source.len(),
        crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES + 1
    );

    let error = build_single_markdown_preview_document(&source)
        .expect_err("exact source length over the cap should return an error");
    let MarkdownPreviewRefusal::Unavailable(reason) = &error else {
        panic!("the size cap should be unavailable, got {error:?}");
    };
    assert!(
        reason.contains("1 MiB"),
        "exact-size markdown preview should mention the size limit: {reason}"
    );
}

#[test]
fn build_single_markdown_preview_document_from_deleted_markdown_table_preview_parses() {
    let diff = vec![
        AnnotatedDiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Header,
            text: "diff --git a/docs/table.md b/docs/table.md".into(),
            old_line: None,
            new_line: None,
        },
        AnnotatedDiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Header,
            text: "deleted file mode 100644".into(),
            old_line: None,
            new_line: None,
        },
        AnnotatedDiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Remove,
            text: "-| **Header Bold** | B |".into(),
            old_line: Some(1),
            new_line: None,
        },
        AnnotatedDiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Remove,
            text: "-| --- | --- |".into(),
            old_line: Some(2),
            new_line: None,
        },
        AnnotatedDiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Remove,
            text: "-| [link](https://example.com) | plain |".into(),
            old_line: Some(3),
            new_line: None,
        },
    ];
    let workdir = PathBuf::from("repo");
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("docs/table.md"),
        area: DiffArea::Unstaged,
    };

    let preview = crate::view::diff_preview::build_deleted_file_preview_from_diff(
        &diff,
        &workdir,
        Some(&target),
    )
    .expect("deleted markdown preview should reconstruct from diff");
    let source = preview_source_text_from_lines(&preview.lines, preview.source_len);
    let document = build_single_markdown_preview_document(source.as_ref())
        .expect("deleted markdown table preview should parse");
    let table_rows = document
        .rows
        .iter()
        .filter(|row| {
            matches!(
                row.kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::TableRow { .. }
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(table_rows.len(), 2);
    assert_eq!(table_rows[0].text.as_ref(), "Header Bold | B");
    assert_eq!(table_rows[1].text.as_ref(), "link        | plain");
}

#[test]
fn prepared_syntax_document_key_includes_repo_rev_path_and_view_mode() {
    let path = Path::new("src/lib.rs");
    let base = prepared_syntax_document_key(
        RepoId(7),
        42,
        path,
        PreparedSyntaxViewMode::FileDiffSplitRight,
    );
    let different_rev = prepared_syntax_document_key(
        RepoId(7),
        43,
        path,
        PreparedSyntaxViewMode::FileDiffSplitRight,
    );
    let different_view_mode = prepared_syntax_document_key(
        RepoId(7),
        42,
        path,
        PreparedSyntaxViewMode::FileDiffSplitLeft,
    );
    let different_repo = prepared_syntax_document_key(
        RepoId(8),
        42,
        path,
        PreparedSyntaxViewMode::FileDiffSplitRight,
    );
    let different_path = prepared_syntax_document_key(
        RepoId(7),
        42,
        Path::new("src/main.rs"),
        PreparedSyntaxViewMode::FileDiffSplitRight,
    );

    assert_ne!(base, different_rev);
    assert_ne!(base, different_view_mode);
    assert_ne!(base, different_repo);
    assert_ne!(base, different_path);
}

#[test]
fn diff_syntax_edit_identical_texts_returns_none() {
    assert!(diff_syntax_edit_from_text_change("hello world", "hello world").is_none());
    assert!(diff_syntax_edit_from_text_change("", "").is_none());
}

#[test]
fn diff_syntax_edit_completely_different_texts() {
    let edit = diff_syntax_edit_from_text_change("abc", "xyz").unwrap();
    assert_eq!(edit.old_range, 0..3);
    assert_eq!(edit.new_range, 0..3);
}

#[test]
fn diff_syntax_edit_shared_prefix() {
    let edit = diff_syntax_edit_from_text_change("hello world", "hello rust").unwrap();
    assert_eq!(edit.old_range, 6..11);
    assert_eq!(edit.new_range, 6..10);
}

#[test]
fn diff_syntax_edit_shared_suffix() {
    let edit = diff_syntax_edit_from_text_change("old suffix", "new suffix").unwrap();
    assert_eq!(edit.old_range, 0..3);
    assert_eq!(edit.new_range, 0..3);
}

#[test]
fn diff_syntax_edit_shared_prefix_and_suffix() {
    let edit = diff_syntax_edit_from_text_change("fn foo() {}", "fn bar() {}").unwrap();
    // "fn " is shared prefix (3 bytes), "() {}" is shared suffix (5 bytes)
    assert_eq!(edit.old_range, 3..6);
    assert_eq!(edit.new_range, 3..6);
}

#[test]
fn diff_syntax_edit_insertion_at_beginning() {
    let edit =
        diff_syntax_edit_from_text_change("fn main() {}", "/* comment */\nfn main() {}").unwrap();
    assert_eq!(edit.old_range, 0..0);
    assert_eq!(edit.new_range, 0..14);
}

#[test]
fn diff_syntax_edit_insertion_at_end() {
    let edit = diff_syntax_edit_from_text_change("fn main() {}", "fn main() {}\n// end").unwrap();
    // "fn main() {}" is 12 bytes; insertion starts after byte 12
    assert_eq!(edit.old_range, 12..12);
    assert_eq!(edit.new_range, 12..19);
}

#[test]
fn diff_syntax_edit_deletion() {
    let edit = diff_syntax_edit_from_text_change("fn foo() { body }", "fn foo() {}").unwrap();
    // shared prefix: "fn foo() {" (10 bytes), shared suffix: "}" (1 byte)
    assert_eq!(edit.old_range, 10..16);
    assert_eq!(edit.new_range, 10..10);
}

#[test]
fn diff_syntax_edit_one_empty_string() {
    let edit = diff_syntax_edit_from_text_change("", "hello").unwrap();
    assert_eq!(edit.old_range, 0..0);
    assert_eq!(edit.new_range, 0..5);

    let edit = diff_syntax_edit_from_text_change("hello", "").unwrap();
    assert_eq!(edit.old_range, 0..5);
    assert_eq!(edit.new_range, 0..0);
}

#[test]
fn diff_syntax_edit_multibyte_utf8() {
    // "café" is 5 bytes (é is 2 bytes), "caff" is 4 bytes
    let edit = diff_syntax_edit_from_text_change("café", "caff").unwrap();
    // shared prefix: "caf" (3 bytes), diverges at é vs f
    assert_eq!(edit.old_range, 3..5);
    assert_eq!(edit.new_range, 3..4);
}
