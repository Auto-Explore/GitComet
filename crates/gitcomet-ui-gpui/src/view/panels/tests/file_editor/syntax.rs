use super::*;

#[test]
fn pair_overlay_paints_both_delimiters_over_the_syntax_runs() {
    let text = "fn f() {}";
    let keyword = gpui::HighlightStyle {
        color: Some(gpui::rgb(0x112233).into_color()),
        ..Default::default()
    };
    let bracket = gpui::HighlightStyle {
        background_color: Some(gpui::rgba(0xffffff26).into_color()),
        ..Default::default()
    };
    let open = text.find('{').expect("open brace");
    let close = text.find('}').expect("close brace");

    let highlights = apply_file_editor_pair_highlights(
        vec![(0..2, keyword), (open..close + 1, keyword)],
        Some(&rows::SyntaxPair {
            open: open..open + 1,
            close: close..close + 1,
            kind: rows::SyntaxPairKind::Bracket,
        }),
        0..text.len(),
        bracket,
    );

    // Sorted, disjoint and inside the window, as the input requires.
    let mut previous_end = 0usize;
    for (range, _) in &highlights {
        assert!(range.start >= previous_end, "runs overlap: {highlights:?}");
        assert!(range.start < range.end);
        assert!(range.end <= text.len());
        previous_end = range.end;
    }

    let background_at = |offset: usize| {
        highlights
            .iter()
            .find(|(range, _)| range.contains(&offset))
            .and_then(|(_, style)| style.background_color)
    };
    assert_eq!(background_at(open), bracket.background_color);
    assert_eq!(background_at(close), bracket.background_color);
    assert_eq!(background_at(0), None, "unrelated runs keep their styling");
}

#[test]
fn whole_tag_pair_overlay_preserves_disjoint_syntax_runs() {
    let punctuation = gpui::HighlightStyle {
        color: Some(gpui::rgb(0x112233).into_color()),
        ..Default::default()
    };
    let tag_name = gpui::HighlightStyle {
        color: Some(gpui::rgb(0x445566).into_color()),
        ..Default::default()
    };
    let attribute = gpui::HighlightStyle {
        color: Some(gpui::rgb(0x778899).into_color()),
        ..Default::default()
    };
    let pair_style = gpui::HighlightStyle {
        background_color: Some(gpui::rgba(0xffffff26).into_color()),
        ..Default::default()
    };
    let runs = vec![
        (0..1, punctuation),
        (1..4, tag_name),
        (5..10, attribute),
        (10..11, punctuation),
        (11..12, punctuation),
        (12..15, tag_name),
        (15..16, punctuation),
    ];

    let highlights = apply_file_editor_pair_highlights(
        runs,
        Some(&rows::SyntaxPair {
            open: 0..11,
            close: 11..16,
            kind: rows::SyntaxPairKind::Tag,
        }),
        0..16,
        pair_style,
    );

    assert_eq!(
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color,))
            .collect::<Vec<_>>(),
        vec![
            (0..1, punctuation.color, pair_style.background_color),
            (1..4, tag_name.color, pair_style.background_color),
            // The grammar left the whitespace unstyled; the overlay fills only
            // that gap instead of adding a run across the whole start tag.
            (4..5, pair_style.color, pair_style.background_color),
            (5..10, attribute.color, pair_style.background_color),
            (10..11, punctuation.color, pair_style.background_color),
            (11..12, punctuation.color, pair_style.background_color),
            (12..15, tag_name.color, pair_style.background_color),
            (15..16, punctuation.color, pair_style.background_color),
        ]
    );

    for adjacent in highlights.windows(2) {
        assert_eq!(
            adjacent[0].0.end, adjacent[1].0.start,
            "whole-tag overlay runs must form a sorted, disjoint tiling"
        );
    }
}

#[test]
fn pair_overlay_is_a_no_op_without_a_pair() {
    let style = gpui::HighlightStyle::default();
    let runs = vec![(0..4, style), (6..9, style)];
    assert_eq!(
        apply_file_editor_pair_highlights(runs.clone(), None, 0..9, style),
        runs
    );
}

#[test]
fn provider_binding_key_changes_only_when_something_changed() {
    let pair = rows::SyntaxPair {
        open: 3..4,
        close: 9..10,
        kind: rows::SyntaxPairKind::Bracket,
    };
    let no_matches: &[std::ops::Range<usize>] = &[];
    let no_occurrences: &[std::ops::Range<usize>] = &[];
    // Two ranges, not one: clippy reads a single-range array literal as a typo.
    let one_match: &[std::ops::Range<usize>] = &[20..25, 40..45];
    let other_match: &[std::ops::Range<usize>] = &[30..35, 40..45];
    let base = file_editor_provider_binding_key(7, 1, Some(&pair), no_matches, no_occurrences);

    assert_eq!(
        base,
        file_editor_provider_binding_key(7, 1, Some(&pair), no_matches, no_occurrences),
        "an unchanged binding must not rebind — that is what stops the observe cycle"
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(8, 1, Some(&pair), no_matches, no_occurrences)
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(7, 2, Some(&pair), no_matches, no_occurrences)
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(
            7,
            1,
            Some(&rows::SyntaxPair {
                open: 3..4,
                close: 12..13,
                kind: rows::SyntaxPairKind::Bracket,
            }),
            no_matches,
            no_occurrences
        ),
        "moving the caret to another pair must rebind"
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(
            7,
            1,
            Some(&rows::SyntaxPair {
                open: 3..4,
                close: 9..10,
                kind: rows::SyntaxPairKind::Quote,
            }),
            no_matches,
            no_occurrences
        ),
        "same span, different kind: the key must still separate them"
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(7, 1, None, no_matches, no_occurrences)
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(7, 1, Some(&pair), one_match, no_occurrences),
        "a search match moves no text and touches no tree, so this key is the \
         only thing that can tell the input its highlights changed"
    );
    assert_ne!(
        file_editor_provider_binding_key(7, 1, Some(&pair), one_match, no_occurrences),
        file_editor_provider_binding_key(7, 1, Some(&pair), other_match, no_occurrences),
        "stepping to the next match changes which hits are washed"
    );
    assert_ne!(
        base,
        file_editor_provider_binding_key(7, 1, Some(&pair), no_matches, &[20..25, 40..45]),
        "moving the caret onto a name moves no text and touches no tree, so this \
         key is the only thing that can tell the input its highlights changed"
    );
}

#[gpui::test]
async fn editing_markdown_highlights_and_leaves_the_rendered_preview(
    cx: &mut gpui::TestAppContext,
) {
    // Markdown opens rendered by default, so entering the editor has to flip the
    // toggle to Source — otherwise `is_markdown_preview_active` reports a
    // preview over a buffer that is plainly showing text and greys out Edit and
    // Blame. The highlight assertions pin the other half: the editor really is
    // running the markdown grammar, injections included.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(949);
    let workdir = unique_workdir("file_editor_markdown");
    let file_rel = std::path::PathBuf::from("README.md");
    let contents = "# Heading\n\nSome `code` here.\n\n```rust\nfn main() {}\n```\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            !pane.is_markdown_preview_active(),
            "the editor is not a rendered preview, whatever the toggle said before"
        );

        let document = pane
            .file_editor_live_syntax
            .as_ref()
            .expect("markdown must get a tree-sitter document, not the heuristic fallback");
        let highlights = document
            .snapshot(crate::kit::syntax_style::SyntaxHighlightPalette::new(
                pane.theme,
            ))
            .highlights_for_byte_range(0..contents.len());

        let styled = |needle: &str| {
            let at = contents.find(needle).expect("fixture contains the needle");
            highlights
                .iter()
                .any(|(range, style)| range.contains(&at) && style.color.is_some())
        };
        assert!(styled("Heading"), "headings must be highlighted");
        assert!(styled("`code`"), "code spans must be highlighted");
        assert!(
            styled("fn main"),
            "a fenced rust block must pick up the injected grammar"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The editor used to apply its 256 KiB caret-scan ceiling before asking what
/// the caret was on, so this crate's own `syntax.rs` parsed and colored normally
/// but no identifier click produced occurrence highlights.
#[gpui::test]
async fn file_editor_click_highlights_names_in_the_actual_syntax_rs(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(965);
    let workdir = unique_workdir("file_editor_actual_syntax_rs_click");
    let file_rel = std::path::PathBuf::from("syntax.rs");
    let source_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../gitcomet-syntax/src/lib.rs");
    let mut contents = std::fs::read_to_string(&source_path).expect("read actual syntax.rs");
    for module in [
        "prepared/drop_queue.rs",
        "prepared/document_cache.rs",
        "prepared/document_prepare.rs",
        "prepared/incremental_reparse.rs",
        "prepared/highlight_spec.rs",
        "prepared/query_tokens.rs",
        "prepared/injections.rs",
        "live.rs",
        "heuristic.rs",
    ] {
        let module_path = source_path.parent().expect("syntax dir").join(module);
        contents.push_str(
            &std::fs::read_to_string(module_path)
                .expect("read syntax module as part of the large-file fixture"),
        );
    }
    assert!(
        contents.len() > 256 * 1024,
        "the regression needs a large file"
    );
    assert!(
        contents.len() <= rows::OCCURRENCE_MAX_TEXT_BYTES,
        "the actual source must stay inside the interactive scan ceiling"
    );
    std::fs::write(workdir.join(&file_rel), &contents).expect("write syntax.rs editor fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let target = "ensure_tree_sitter_allocator";
    let target_start = contents
        .find(target)
        .expect("actual syntax.rs should retain the allocator funnel");
    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            assert!(
                pane.file_editor_live_syntax.is_some(),
                "the large Rust file should have a live syntax tree"
            );
            pane.file_editor_input.update(cx, |input, cx| {
                input.set_cursor_offset(target_start + 3, cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert!(
            pane.file_editor_occurrences.iter().any(
                |range| range.start == target_start && range.end == target_start + target.len()
            ),
            "clicking the allocator name should highlight it in the actual half-megabyte file; \
             occurrences={:?}",
            pane.file_editor_occurrences,
        );
        assert!(
            pane.file_editor_occurrences.len() >= 3,
            "the definition and calls should all be highlighted"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn a_supported_language_is_never_latched_as_unparseable() {
    use crate::view::rows;

    // Regression: a failed build used to latch the file as unhighlightable for
    // the rest of the session, so one unlucky parse left markdown plain until
    // the file was reopened. The permanent reasons are now asked directly.
    assert!(
        rows::live_syntax_document_supported(rows::DiffSyntaxLanguage::Markdown, 4_096),
        "markdown has a wired grammar and a small file is well under the ceiling"
    );
    assert!(
        !rows::live_syntax_document_supported(
            rows::DiffSyntaxLanguage::Markdown,
            rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES + 1,
        ),
        "past the parse ceiling is a permanent no, and is checked without parsing"
    );
}

#[gpui::test]
fn the_editor_renders_rust_highlights_as_the_engine_produced_them(cx: &mut gpui::TestAppContext) {
    assert_editor_renders_the_engines_highlights(
        cx,
        gitcomet_state::model::RepoId(962),
        "file_editor_rs_highlights",
        "main.rs",
        concat!(
            "use std::collections::HashMap;\n",
            "\n",
            "/// Doc comment.\n",
            "pub struct Stage<'a> {\n",
            "    pub name: &'a str,\n",
            "    map: HashMap<String, usize>,\n",
            "}\n",
            "\n",
            "impl<'a> Stage<'a> {\n",
            "    pub fn run(&mut self, n: usize) -> Result<usize, String> {\n",
            "        let mut total = 0usize;\n",
            "        for (key, _value) in self.map.iter() {\n",
            "            println!(\"{key}: {}\", n);\n",
            "            total += key.len();\n",
            "        }\n",
            "        Ok(total)\n",
            "    }\n",
            "}\n",
        ),
    );
}

#[gpui::test]
fn the_editor_renders_shell_highlights_as_the_engine_produced_them(cx: &mut gpui::TestAppContext) {
    assert_editor_renders_the_engines_highlights(
        cx,
        gitcomet_state::model::RepoId(963),
        "file_editor_sh_highlights",
        "build.sh",
        concat!(
            "#!/usr/bin/env bash\n",
            "set -euo pipefail\n",
            "\n",
            "NAME=\"world\"\n",
            "count=0\n",
            "\n",
            "greet() {\n",
            "  local who=\"$1\"\n",
            "  echo \"hello ${who}\"\n",
            "  if [[ -n \"$who\" ]]; then\n",
            "    count=$((count + 1))\n",
            "  fi\n",
            "}\n",
            "\n",
            "for f in *.txt; do\n",
            "  greet \"$f\"\n",
            "done\n",
        ),
    );
}

/// Opening a *second* file in the same pane must highlight it too.
///
/// This is the sequence the whole class of "edit mode has no highlighting" bugs
/// lived in, and no test used to walk it: every other test opens exactly one
/// file, where the buffer is already empty so the blanking `set_text("")` is a
/// no-op. From the second file on, the blanking is a real edit that used to
/// build a document over the empty text, which a budget-blown wholesale
/// replacement then kept — a full rope paired with a 0-byte tree.
///
/// Driven at a zero foreground budget so the test takes the arm the app takes:
/// the shipped budget is 1 ms against unoptimised tree-sitter in a dev build,
/// while tests get 2 ms at `opt-level = 1`, which is why this never reproduced
/// under `cargo test`.
#[gpui::test]
async fn a_second_file_opened_in_the_same_pane_is_still_highlighted(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(965);
    let workdir = unique_workdir("file_editor_second_file");
    let first = std::path::PathBuf::from("first.rs");
    let second = std::path::PathBuf::from("build.sh");
    std::fs::write(workdir.join(&first), "fn first() -> usize { 1 }\n").expect("write first");
    // Big enough that a zero-budget parse cannot finish, and shaped like the
    // reported repro: a quoted heredoc, a case block, `${var:-}` expansions.
    let second_contents = concat!(
        "#!/usr/bin/env bash\n",
        "set -euo pipefail\n",
        "\n",
        "usage() {\n",
        "  cat <<'USAGE'\n",
        "Usage: scripts/build.sh --out PATH [--verify]\n",
        "USAGE\n",
        "}\n",
        "\n",
        "out=\"\"\n",
        "verify=\"false\"\n",
        "while [[ $# -gt 0 ]]; do\n",
        "  case \"$1\" in\n",
        "    --out) out=\"${2:-}\"; shift 2 ;;\n",
        "    --verify) verify=\"true\"; shift ;;\n",
        "    *) echo \"unknown: $1\" >&2; usage; exit 1 ;;\n",
        "  esac\n",
        "done\n",
    )
    .repeat(8);
    std::fs::write(workdir.join(&second), &second_contents).expect("write second");

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, _cx| {
            pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                foreground_parse: std::time::Duration::ZERO,
            });
        });
    });

    // First file, then the second — the order that matters.
    for path in [&first, &second] {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                push_test_state(this, editor_state(repo_id, &workdir, path), cx);
                this.diff_renderer
                    .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
            });
        });
        cx.run_until_parked();
    }

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            second_contents,
            "the second file must be the one in the buffer"
        );
        let rendered = diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, _| {
                input.debug_effective_highlights_for_range(0..second_contents.len())
            })
        });
        assert!(
            !rendered.is_empty(),
            "the second file opened in a pane must still be highlighted; a zero \
             budget must fall back to heuristics or an off-thread parse, never to \
             a tree describing the file that was here before"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn syntax_builds_coalesce_edits_and_file_switch_discards_obsolete_completion(
    cx: &mut gpui::TestAppContext,
) {
    cx.skip_drawing();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let workdir = directory.path().canonicalize().unwrap();
    let repo_id = gitcomet_state::model::RepoId(998);
    let first = Path::new("first.rs");
    let second = Path::new("second.rs");
    std::fs::write(workdir.join(first), "fn first() {}\n").unwrap();
    std::fs::write(workdir.join(second), "fn second() {}\n").unwrap();
    let (release, gate) = smol::channel::unbounded();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, editor_state(repo_id, &workdir, first), cx);
            view.diff_renderer.update(cx, |pane, cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
                pane.live_syntax_background_gate_for_tests = Some(gate);
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
            });
        });
    });
    cx.run_until_parked();
    let pane = cx.update(|_, app| view.read(app).diff_renderer.clone());
    cx.update(|_, app| pane.update(app, |pane, cx| pane.ensure_file_editor_loaded(cx)));
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "// edit\n", cx);
                });
            })
        });
        cx.run_until_parked();
    }
    cx.update(|_, app| {
        assert_eq!(
            pane.read(app).live_syntax_background_jobs_started,
            1,
            "loading={} language={:?} text={:?} error={:?}",
            pane.read(app).file_editor_loading(),
            pane.read(app).file_editor_language,
            pane.read(app).file_editor_input.read(app).text(),
            pane.read(app).file_editor_error()
        )
    });
    release.try_send(()).unwrap();
    cx.run_until_parked();
    cx.update(|_, app| {
        assert_eq!(pane.read(app).live_syntax_background_jobs_started, 2);
        assert!(pane.read(app).file_editor_live_syntax_building.is_some());
    });
    release.try_send(()).unwrap();
    cx.run_until_parked();
    cx.update(|_, app| {
        let pane = pane.read(app);
        assert!(pane.file_editor_live_syntax_building.is_none());
        assert!(pane.file_editor_live_syntax.is_some());
        let snapshot = pane.file_editor_input.read(app).text_snapshot();
        assert_eq!(
            pane.file_editor_live_syntax_source,
            Some((snapshot.model_id(), snapshot.revision()))
        );
    });
    // A wholesale replacement changes recovery from a reparse to a cold build.
    // It must wait for the same document's active computation to finish.
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// before replacement\n", cx);
            });
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.set_text("fn replaced() {}\n", cx);
            });
            let snapshot = pane.file_editor_input.read(cx).text_snapshot();
            pane.refresh_file_editor_syntax(&snapshot, None, cx);
            assert!(pane.file_editor_live_syntax.is_none());
            assert!(pane.file_editor_live_syntax_reparsing.is_some());
            assert!(pane.file_editor_live_syntax_building.is_none());
            assert_eq!(pane.live_syntax_background_jobs_started, 3);
        });
    });
    cx.run_until_parked();
    release.try_send(()).unwrap();
    cx.run_until_parked();
    cx.update(|_, app| {
        assert_eq!(pane.read(app).live_syntax_background_jobs_started, 4);
        assert!(pane.read(app).file_editor_live_syntax_building.is_some());
        assert!(pane.read(app).file_editor_live_syntax_reparsing.is_none());
    });
    release.try_send(()).unwrap();
    cx.run_until_parked();
    cx.update(|_, app| {
        let pane = pane.read(app);
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            "fn replaced() {}\n"
        );
        assert!(pane.file_editor_live_syntax.is_some());
        assert!(pane.file_editor_live_syntax_building.is_none());
    });
    // Hold a reparse for A, then switch documents before it is allowed to finish.
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "/* pending */\n", cx)
            });
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, editor_state(repo_id, &workdir, second), cx)
        });
        pane.update(app, |pane, cx| pane.ensure_file_editor_loaded(cx));
    });
    cx.run_until_parked();
    cx.update(|_, app| pane.update(app, |pane, cx| pane.ensure_file_editor_loaded(cx)));
    cx.run_until_parked();
    release.try_send(()).unwrap();
    cx.run_until_parked();
    cx.update(|_, app| {
        let pane = pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "fn second() {}\n");
        assert!(pane.file_editor_live_syntax.is_some());
        assert!(pane.file_editor_live_syntax_building.is_none());
        assert!(pane.file_editor_live_syntax_reparsing.is_none());
    });
    // Destruction cancels the completion owner while work is pending.
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// pending destruction\n", cx)
            });
        })
    });
    cx.run_until_parked();
    let weak = pane.downgrade();
    struct EmptyPane;
    impl gpui::Render for EmptyPane {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }
    cx.update(|window, app| {
        window.replace_root(app, |_, _| EmptyPane);
    });
    drop(pane);
    drop(view);
    cx.cx.update(|_| {});
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    let _ = release.try_send(());
    cx.run_until_parked();
}

#[gpui::test]
fn the_editor_renders_nunjucks_with_the_jinja_grammar(cx: &mut gpui::TestAppContext) {
    assert_editor_renders_the_engines_highlights(
        cx,
        gitcomet_state::model::RepoId(964),
        "file_editor_nunjucks_highlights",
        "index.njk",
        // Front matter (yaml), markup (html) and a script body (javascript,
        // depth 2) all on one fixture, since each is its own layer.
        concat!(
            "---\n",
            "title: Home\n",
            "---\n",
            "{# navigation #}\n",
            "<nav class=\"menu\">\n",
            "  {% for item in items %}\n",
            "    <a href=\"{{ item.url }}\">{{ item.label | upper }}</a>\n",
            "  {% endfor %}\n",
            "</nav>\n",
            "<script>\nconst open = false;\n</script>\n",
        ),
    );
}

/// End-to-end check that what the buffer *renders* is what the syntax engine
/// produced, across the highlight-provider and `TextInput` windowing layers.
///
/// The engine-level equivalence with the read-only panes is pinned in
/// `syntax/live.rs`; this pins the rest of the path, which is where an offset or
/// a dropped window would show up as "the colours are wrong in edit mode" while
/// every engine test still passed.
fn assert_editor_renders_the_engines_highlights(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    label: &str,
    file_rel: &str,
    contents: &str,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let workdir = unique_workdir(label);
    let file_rel = std::path::PathBuf::from(file_rel);
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert_eq!(
            pane.file_editor_input.read(app).text(),
            contents,
            "{label}: the buffer must hold the fixture"
        );
        let document = pane
            .file_editor_live_syntax
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: the editor must build a tree-sitter document"));
        assert_eq!(
            document.language(),
            rows::diff_syntax_language_for_path(&file_rel)
                .unwrap_or_else(|| panic!("{label}: the fixture path must resolve a language")),
            "{label}: the editor must build the grammar selected from the file extension"
        );
        let expected = document
            .snapshot(crate::kit::syntax_style::SyntaxHighlightPalette::new(
                pane.theme,
            ))
            .highlights_for_byte_range(0..contents.len());
        assert!(
            expected.len() > 4,
            "{label}: the fixture must produce a real spread of tokens, got {}",
            expected.len()
        );

        let rendered = diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, _| {
                input.debug_effective_highlights_for_range(0..contents.len())
            })
        });
        assert_eq!(
            rendered, expected,
            "{label}: the buffer must render exactly the runs the engine produced"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}
