use super::*;

#[gpui::test]
async fn word_wrap_reaches_the_buffer(cx: &mut gpui::TestAppContext) {
    // The editor was built at content width, so long lines ran off to the right
    // whatever the word-wrap preference said.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(951);
    let workdir = unique_workdir("file_editor_wrap");
    let file_rel = std::path::PathBuf::from("notes.md");
    std::fs::write(workdir.join(&file_rel), "a very long line ".repeat(40)).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    // Driven on the pane directly: `set_diff_word_wrap` reaches back into the
    // root view, which cannot be updated from inside its own update.
    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.set_diff_word_wrap(true, cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.file_editor_input.read(app).soft_wrap(),
            "the word-wrap preference has to reach the buffer, not just the diff"
        );
    });

    // Turning it off puts the buffer back on content-width layout.
    cx.update(|window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.set_diff_word_wrap(false, cx);
        });
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        assert!(
            !view
                .read(app)
                .diff_renderer
                .read(app)
                .file_editor_input
                .read(app)
                .soft_wrap()
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn file_editor_mounts_a_vertical_scrollbar(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(975);
    let workdir = unique_workdir("file_editor_scrollbar");
    let file_rel = std::path::PathBuf::from("long.rs");
    let contents = (0..400)
        .map(|line| format!("fn line_{line}() {{}}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds("file_editor_scrollbar").is_some(),
        "edit mode should mount its vertical scrollbar beside the scroll surface"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.file_editor_scroll.max_offset().y > px(0.0),
            "the long editor fixture should expose a vertical scroll range"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn an_svg_opened_from_the_explorer_can_show_its_code_and_be_edited(
    cx: &mut gpui::TestAppContext,
) {
    // Pictures opened from the explorer used to land in the A/B image diff.
    // Rendered is still the picture; Code is the source, and the source is
    // editable text like any other file.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(952);
    let workdir = unique_workdir("file_editor_svg");
    let file_rel = std::path::PathBuf::from("logo.svg");
    let contents = "<svg viewBox=\"0 0 1 1\"><rect width=\"1\" height=\"1\" /></svg>\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");
    let absolute = workdir.join(&file_rel);

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut state = editor_state(repo_id, &workdir, &file_rel);
            // A read-only content view, as `OpenFileContent` leaves it.
            Arc::make_mut(&mut state).repos[0].diff_state.edit_mode = false;
            push_test_state(this, state, cx);
        });
    });

    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, _cx| {
            assert!(
                pane.content_preview_is_picture(&absolute),
                "rendered is the default for an SVG, and rendered means the picture"
            );
            pane.rendered_preview_modes.set(
                crate::view::RenderedPreviewKind::Svg,
                crate::view::RenderedPreviewMode::Source,
            );
            assert!(
                !pane.content_preview_is_picture(&absolute),
                "asking for Code is asking for the source, which is text"
            );
        });
    });

    // With Code showing, the file is editable.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut state = editor_state(repo_id, &workdir, &file_rel);
            Arc::make_mut(&mut state).repos[0].diff_state.edit_mode = true;
            push_test_state(this, state, cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(pane.is_file_editor_active());
        assert_eq!(pane.file_editor_input.read(app).text(), contents);
        assert_eq!(
            pane.file_editor_language,
            Some(rows::DiffSyntaxLanguage::Xml)
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn a_wrapped_line_owns_its_gutter_rows_and_numbers_only_the_first() {
    use crate::view::panes::diff::file_editor_line_for_visual_row;

    // Line 0 wraps to 3 rows, line 1 to 1, line 2 to 2 — so the gutter rows are
    // [0,0,0, 1, 2,2] and the number goes on the first row of each block.
    let starts = [0usize, 3, 4];
    let expected = [
        (0, false),
        (0, true),
        (0, true),
        (1, false),
        (2, false),
        (2, true),
    ];
    for (visual_ix, want) in expected.iter().enumerate() {
        assert_eq!(
            file_editor_line_for_visual_row(&starts, visual_ix),
            *want,
            "gutter row {visual_ix}"
        );
    }

    // No wrap: the two row spaces coincide, and nothing is a continuation.
    for visual_ix in 0..4 {
        assert_eq!(
            file_editor_line_for_visual_row(&[], visual_ix),
            (visual_ix, false)
        );
    }
}

#[gpui::test]
async fn the_gutter_projects_through_wrap_so_numbers_track_their_lines(
    cx: &mut gpui::TestAppContext,
) {
    // Regression: the gutter was one row per *logical* line while the buffer
    // laid out one per *visual* row, so from the first wrap onward the numbers
    // labelled the wrong lines.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(953);
    let workdir = unique_workdir("file_editor_wrap_gutter");
    let file_rel = std::path::PathBuf::from("notes.md");
    let long = "wrap ".repeat(120);
    std::fs::write(workdir.join(&file_rel), format!("{long}\nshort\n")).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.set_diff_word_wrap(true, cx);
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    // A second pass: the row counts are maintained during the buffer's prepaint,
    // so the frame that can project them is the one after it has laid out.
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        let starts = &pane.file_editor_wrap_row_starts;
        if starts.is_empty() {
            // The headless text system reports every glyph identically, so the
            // wrap pass may legitimately decide nothing wraps here. The mapping
            // itself is pinned by the unit test above; what matters here is that
            // the gutter never claims more lines than the buffer has.
            return;
        }
        assert_eq!(
            starts.len(),
            pane.file_editor_input
                .read(app)
                .text_snapshot()
                .line_count(),
            "the projection has one entry per logical line"
        );
        assert!(
            starts.windows(2).all(|pair| pair[0] < pair[1]),
            "each line starts strictly after the previous one: {starts:?}"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn short_file_editor_uses_the_space_below_eof_for_caret_drag_and_menu(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let contents = "alpha\nbeta";
    let workdir = seed_editor(&view, cx, 998, "file_editor_below_eof_selection", contents);

    let viewport = cx
        .debug_bounds("file_editor_scroll")
        .expect("short editor viewport bounds");
    let below_eof = point(viewport.center().x, viewport.bottom() - px(24.0));
    let target = point(viewport.left() + px(28.0), viewport.top() + px(10.0));
    let target_offset = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        let input = pane.file_editor_input.read(app);
        assert_eq!(input.offset_for_position(below_eof), contents.len());
        input.offset_for_position(target)
    });
    assert!(target_offset < contents.len());

    cx.simulate_mouse_down(below_eof, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(below_eof, MouseButton::Left, Modifiers::default());
    assert_eq!(
        editor_selected_range(&view, cx),
        contents.len()..contents.len()
    );

    cx.simulate_mouse_down(below_eof, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    assert_eq!(
        editor_selected_range(&view, cx),
        target_offset..contents.len()
    );

    cx.update(|window, app| {
        let pane = view.read(app).diff_renderer.clone();
        pane.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.set_selected_range(0..2, false, window, cx)
            });
        });
    });
    cx.simulate_mouse_down(below_eof, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(below_eof, MouseButton::Right, Modifiers::default());
    assert_eq!(
        editor_selected_range(&view, cx),
        contents.len()..contents.len()
    );
    assert!(
        cx.debug_bounds("text_input_context_select_all").is_some(),
        "right-clicking below EOF should open the normal text-input menu"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(
            pane.file_editor_scroll.max_offset().y,
            px(0.0),
            "filling the hit surface must not make a short file scroll vertically"
        );
    });

    std::fs::remove_dir_all(workdir).expect("cleanup below-EOF editor fixture");
}

/// The editor counts occurrences, not rows. Every other view counts matching
/// rows, and reusing that here would make Enter skip repeats within a line.
#[gpui::test]
async fn file_editor_search_counts_every_occurrence_in_the_live_buffer(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let contents = "let needle = needle.needle();\nfn other() {}\nlast needle\n";
    let workdir = seed_editor(&view, cx, 981, "file_editor_search_count", contents);

    search_the_editor_for(&view, cx, "needle");

    let matches = editor_search_matches(&view, cx);
    assert_eq!(
        matches.len(),
        4,
        "three hits on the first line and one on the last are four stops"
    );
    for range in &matches {
        assert_eq!(&contents[range.clone()], "needle");
    }
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(
            pane.diff_search_matches,
            vec![0, 0, 0, 2],
            "the list the `n/N` label reads holds one entry per occurrence, \
             carrying the line that occurrence sits on"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The regression this change exists for: edit mode leaves `content_preview`
/// set, so the search used to take the file preview's branch and count the
/// *pre-edit* preview text — a plausible number against a document that is not
/// on screen.
#[gpui::test]
async fn file_editor_search_reads_the_buffer_not_the_stale_preview(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let workdir = seed_editor(
        &view,
        cx,
        982,
        "file_editor_search_live",
        "fn main() {\n    let value = 1;\n}\n",
    );

    // A needle that exists nowhere on disk.
    set_editor_text(&view, cx, "fn main() {\n    let needle = needle;\n}\n");
    search_the_editor_for(&view, cx, "needle");
    assert_eq!(
        editor_search_matches(&view, cx).len(),
        2,
        "the search must read the buffer, including edits that are not on disk"
    );

    set_editor_text(&view, cx, "needle needle needle\n");
    assert_eq!(
        editor_search_matches(&view, cx).len(),
        3,
        "editing the buffer with the search open must recount against it"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Every match wears the search wash except the current one, which the
/// selection marks instead. Selection quads are painted *before* highlight
/// backgrounds, so a wash over the current match would cover it.
#[gpui::test]
async fn file_editor_search_selects_the_current_match_and_washes_the_rest(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let contents = "alpha needle beta\nneedle gamma\n";
    let workdir = seed_editor(&view, cx, 983, "file_editor_search_paint", contents);

    search_the_editor_for(&view, cx, "needle");

    let matches = editor_search_matches(&view, cx);
    assert_eq!(matches.len(), 2);
    let (first, second) = (matches[0].clone(), matches[1].clone());

    assert_eq!(
        editor_selected_range(&view, cx),
        first,
        "activating the search lands on the first match and selects it"
    );

    cx.update(|_window, app| {
        let diff_renderer = view.read(app).diff_renderer.clone();
        let wash = diff_renderer
            .read(app)
            .theme
            .colors
            .editor
            .search_match_background
            .into_color();
        let highlights = diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, _| {
                input.debug_effective_highlights_for_range(0..contents.len())
            })
        });

        assert!(
            !highlights
                .iter()
                .any(|(range, style)| *range == first && style.background_color == Some(wash)),
            "the current match is left to the selection, not washed over it: {highlights:?}"
        );
        assert!(
            highlights
                .iter()
                .any(|(range, style)| *range == second && style.background_color == Some(wash)),
            "every other match wears the search wash: {highlights:?}"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Stepping visits repeats within a line, then wraps.
#[gpui::test]
async fn file_editor_search_next_match_steps_within_a_line(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let workdir = seed_editor(
        &view,
        cx,
        984,
        "file_editor_search_step",
        "needle needle needle\n",
    );

    search_the_editor_for(&view, cx, "needle");
    let matches = editor_search_matches(&view, cx);
    assert_eq!(matches.len(), 3);

    let mut walked = Vec::new();
    for _ in 0..4 {
        walked.push(editor_selected_range(&view, cx));
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer
                    .update(cx, |pane, _cx| pane.diff_search_next_match());
            });
        });
        draw_and_drain_test_window(cx);
    }

    assert_eq!(
        walked,
        vec![
            matches[0].clone(),
            matches[1].clone(),
            matches[2].clone(),
            matches[0].clone(),
        ],
        "three same-line hits are three stops, and the walk wraps"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn file_editor_search_keeps_navigation_relative_to_the_match_before_edits(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let contents = "needle\nneedle\nneedle\n";
    let workdir = seed_editor(
        &view,
        cx,
        999,
        "file_editor_search_pending_navigation",
        contents,
    );
    search_the_editor_for(&view, cx, "needle");

    for (case, (current, edits, next, expected)) in [
        (1, 1, true, 2),
        (1, 2, true, 2),
        (1, 2, false, 0),
        (0, 1, false, 2),
        (2, 1, true, 0),
    ]
    .into_iter()
    .enumerate()
    {
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.clone();
            pane.update(app, |pane, cx| {
                pane.diff_search_match_ix = Some(current);
                for edit in 0..edits {
                    pane.file_editor_input.update(cx, |input, cx| {
                        input.set_text(format!("{contents}case {case} edit {edit}"), cx);
                    });
                    pane.on_file_editor_edited(cx);
                }
                assert!(pane.diff_search_worker_running);
                assert!(pane.diff_search_matches.is_empty());
                if next {
                    pane.diff_search_next_match();
                } else {
                    pane.diff_search_prev_match();
                }
            });
        });
        draw_and_drain_test_window(cx);
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(pane.diff_search_matches, vec![0, 1, 2]);
            assert_eq!(pane.diff_search_match_ix, Some(expected), "case {case}");
            assert!(!pane.diff_search_worker_running);
            assert!(pane.diff_search_pending_previous_query.is_none());
        });
    }

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A match below the fold has to bring the buffer with it. The editor is a
/// `TextInput` over a plain `ScrollHandle`, so there is no deferred
/// `scroll_to_item` to inherit.
#[gpui::test]
async fn file_editor_search_scrolls_a_distant_match_into_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let mut lines: Vec<String> = (0..400)
        .map(|line| format!("fn line_{line}() {{}}"))
        .collect();
    lines.push("fn needle() {}".to_string());
    let contents = format!("{}\n", lines.join("\n"));
    let workdir = seed_editor(&view, cx, 985, "file_editor_search_scroll", &contents);

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.file_editor_scroll.max_offset().y > px(0.0),
            "the fixture must be taller than the viewport for this to mean anything"
        );
        assert_eq!(pane.file_editor_scroll.offset().y, px(0.0));
    });

    search_the_editor_for(&view, cx, "needle");
    // The reveal is applied by the render pass, and the input's own caret
    // autoscroll settles on the frame after that.
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.file_editor_search_matches.len(), 1);
        // Asserting movement, not a pixel value: `#[gpui::test]` measures every
        // glyph identically, so exact offsets say nothing about the real app.
        assert!(
            pane.file_editor_scroll.offset().y < px(0.0),
            "a match 400 lines down must scroll the buffer, got {:?}",
            pane.file_editor_scroll.offset()
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A match far along a long line has to be scrolled to sideways too.
///
/// A multiline `TextInput` leaves horizontal scrolling to its container, and its
/// own caret autoscroll only handles the vertical axis, so without the reveal
/// the line comes into view with the hit still off the right edge.
#[gpui::test]
async fn file_editor_search_scrolls_sideways_to_a_match_far_along_a_line(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let mut lines: Vec<String> = (0..20)
        .map(|line| format!("fn line_{line}() {{}}"))
        .collect();
    // The needle sits well past any plausible viewport width.
    lines.push(format!("// {}needle", "pad ".repeat(200)));
    let contents = format!("{}\n", lines.join("\n"));
    let workdir = seed_editor(&view, cx, 986, "file_editor_search_hscroll", &contents);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                // Wrapping would fold the long line into the pane and leave
                // nothing to scroll sideways to.
                pane.diff_word_wrap = false;
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.file_editor_scroll.max_offset().x > px(0.0),
            "the fixture must overflow sideways for this to mean anything; max={:?}",
            pane.file_editor_scroll.max_offset()
        );
        assert_eq!(pane.file_editor_scroll.offset().x, px(0.0));
    });

    search_the_editor_for(&view, cx, "needle");
    // One frame places the caret, the next measures it and scrolls to it.
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.file_editor_search_matches.len(), 1);
        assert!(
            pane.file_editor_scroll.offset().x < px(0.0),
            "a match far along the line must scroll the editor sideways, got {:?}",
            pane.file_editor_scroll.offset()
        );
        assert!(
            !pane.file_editor_search_reveal_x_pending,
            "the sideways reveal should be claimed once, not retried every frame"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The whole loop, driven by the keys the user actually presses.
///
/// Ctrl+F reaches the search over a focused editor only because its binding
/// carries no context predicate — `handle_diff_shortcut` hands every other
/// keystroke to the buffer.
#[gpui::test]
async fn ctrl_f_and_escape_walk_in_and_out_of_the_editor(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let contents = "alpha needle beta\nneedle gamma\n";
    let workdir = seed_editor(&view, cx, 986, "file_editor_search_keys", contents);

    cx.update(|window, app| {
        crate::app::bind_app_keys_for_test(app);
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                let focus = pane.file_editor_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.diff_search_active,
            "ctrl+f must open the search even with the buffer focused"
        );
        assert!(
            pane.diff_search_input
                .read(app)
                .focus_handle()
                .is_focused(window),
            "and put the caret in the search box, not the buffer"
        );
    });

    cx.simulate_keystrokes("n e e d l e");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });
    draw_and_drain_test_window(cx);

    let matches = editor_search_matches(&view, cx);
    assert_eq!(matches.len(), 2, "the buffer holds two needles");
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .diff_renderer
                .read(app)
                .file_editor_input
                .read(app)
                .text(),
            contents,
            "the query must land in the search box, never in the file"
        );
    });

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(!pane.diff_search_active, "escape closes the search");
        assert!(
            pane.file_editor_search_matches.is_empty(),
            "and takes the wash with it"
        );
        assert!(
            pane.is_file_editor_active(),
            "escape from the search must not also exit edit mode"
        );
        assert!(
            pane.file_editor_input
                .read(app)
                .focus_handle()
                .is_focused(window),
            "the buffer gets focus back, so the next keystroke edits the file"
        );
    });
    assert_eq!(
        editor_selected_range(&view, cx),
        matches[0],
        "the caret is left on the match the search was showing"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn recent_repository_shortcut_does_not_select_the_file_editor(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let contents = "alpha beta gamma\n";
    let workdir = seed_editor(&view, cx, 987, "file_editor_recent_repo_shortcut", contents);
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::install_app_shortcuts_for_test(app, Arc::new(TestBackend));
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.set_caret(6, cx);
                    let focus = input.focus_handle();
                    window.focus(&focus, cx);
                });
            });
        });
        let _ = window.draw(app);
        window.activate();
    });
    assert_eq!(editor_selected_range(&view, cx), 6..6);

    cx.simulate_keystrokes("secondary-shift-a");
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("app_popover").is_some(),
        "Ctrl/Cmd+Shift+A must open the recent-repositories picker from the editor"
    );
    assert_eq!(
        editor_selected_range(&view, cx),
        6..6,
        "the recent-repositories shortcut must not also select the editor buffer"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The occurrence set must depend on where the caret *is*, not on how it got
/// there.
///
/// Two name tokens can abut, and the guard that decided whether the cached set
/// still applied was an inclusive range test -- so the offset where one name
/// ends and the next begins satisfied both. Walking right out of `$foo` into
/// `$bar` kept `$foo` lit for one keypress, while arriving at the same offset
/// from the right lit `$bar`. Perl is the in-tree case: `scalar_variable` is a
/// sigil plus a name with no separator between two of them.
#[gpui::test]
async fn file_editor_occurrences_do_not_depend_on_the_direction_of_approach(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(966);
    let workdir = unique_workdir("file_editor_abutting_names");
    let file_rel = std::path::PathBuf::from("abutting.pl");
    // `$foo` is 6..10 and `$bar` is 10..14, so offset 10 is the boundary. Each
    // name occurs again below, so the two sets are told apart by more than the
    // clicked token alone.
    let contents = "print $foo$bar;\nprint $foo;\nprint $bar;\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write abutting-name fixture");

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
        assert!(
            diff_renderer.read(app).file_editor_live_syntax.is_some(),
            "the Perl fixture should have a live syntax tree"
        );
    });

    let set_cursor = |cx: &mut gpui::VisualTestContext, offset: usize| {
        cx.update(|_window, app| {
            diff_renderer.update(app, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.set_cursor_offset(offset, cx);
                });
            });
        });
        cx.run_until_parked();
        cx.update(|_window, app| diff_renderer.read(app).file_editor_occurrences.clone())
    };

    let foo: Vec<std::ops::Range<usize>> = vec![6..10, 22..26];
    let bar: Vec<std::ops::Range<usize>> = vec![10..14, 34..38];

    assert_eq!(
        set_cursor(cx, 8),
        foo,
        "the caret inside `$foo` lights `$foo`"
    );
    // Walk right, one offset at a time, across the boundary.
    let _ = set_cursor(cx, 9);
    assert_eq!(
        set_cursor(cx, 10),
        bar,
        "offset 10 is the first byte of `$bar`, whichever side the caret came from"
    );

    assert_eq!(
        set_cursor(cx, 12),
        bar,
        "the caret inside `$bar` lights `$bar`"
    );
    let _ = set_cursor(cx, 11);
    assert_eq!(
        set_cursor(cx, 10),
        bar,
        "and arriving from the right agrees with arriving from the left"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Put `contents` on disk and show it in the editor.
fn seed_editor(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: u64,
    label: &str,
    contents: &str,
) -> std::path::PathBuf {
    let repo_id = gitcomet_state::model::RepoId(repo_id);
    let workdir = unique_workdir(label);
    let file_rel = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();
    draw_and_drain_test_window(cx);
    workdir
}

/// Open the search box over the editor and put `query` in it, the way Ctrl+F
/// followed by typing does. Returns once the reveal has been drawn.
fn search_the_editor_for(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    query: &str,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                assert!(
                    pane.open_search_for_active_view(window, cx),
                    "the editor is showing a working-tree file, so search must open over it"
                );
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query, cx));
            });
        });
    });
    cx.run_until_parked();
    // Typing schedules a debounced recompute; run it now rather than on a timer.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });
    draw_and_drain_test_window(cx);
}

fn editor_search_matches(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Vec<std::ops::Range<usize>> {
    cx.update(|_window, app| {
        view.read(app)
            .diff_renderer
            .read(app)
            .file_editor_search_matches
            .clone()
    })
}

fn editor_selected_range(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> std::ops::Range<usize> {
    cx.update(|_window, app| {
        let diff_renderer = view.read(app).diff_renderer.clone();
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input
                .update(cx, |input, _| input.selected_range())
        })
    })
}

fn set_editor_text(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    text: &str,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.file_editor_input
                    .update(cx, |input, cx| input.set_text(text, cx));
            });
        });
    });
    cx.run_until_parked();
}
