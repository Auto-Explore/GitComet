use super::*;

#[test]
fn mask_text_preserves_length_and_newlines() {
    let input = "a\nb\r\nc";
    let masked = mask_text_for_display(input);
    assert_eq!(masked.len(), input.len());
    assert_eq!(masked, "*\n*\r\n*");
}

#[test]
fn mask_text_removes_original_characters() {
    let input = "secret-passphrase";
    let masked = mask_text_for_display(input);
    assert_ne!(masked, input);
    assert!(masked.chars().all(|ch| ch == '*'));
}

#[test]
fn truncate_line_for_shaping_respects_utf8_boundary_and_appends_suffix() {
    let input = "éééé";
    let (truncated, hash) = truncate_line_for_shaping(input, 5);
    assert_eq!(truncated.as_ref(), "é…");
    // hash_shaping_slice must be consistent with truncate_line_for_shaping
    let (hash2, _) = hash_shaping_slice(input, 5);
    assert_eq!(hash, hash2);
}

#[test]
fn visible_plain_line_range_applies_guard_rows() {
    let range = visible_plain_line_range(100, px(20.0), px(200.0), px(260.0), 2);
    assert_eq!(range, 8..16);
}

#[test]
fn wrapped_line_index_and_visible_range_use_row_counts() {
    let row_counts = vec![1, 3, 1, 2, 1];
    let y_offsets = vec![px(0.0), px(10.0), px(40.0), px(50.0), px(70.0)];
    let line_height = px(10.0);

    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(35.0)),
        1
    );
    let range =
        visible_wrapped_line_range(&y_offsets, &row_counts, line_height, px(42.0), px(58.0), 0);
    assert_eq!(range, 2..4);
}

#[test]
fn compute_line_starts_and_line_text_handle_trailing_newline() {
    let text = "alpha\nbeta\n";
    let starts = compute_line_starts(text);
    assert_eq!(starts, vec![0, 6, 11]);
    assert_eq!(line_text_for_index(text, starts.as_slice(), 0), "alpha");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 1), "beta");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 2), "");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 3), "");
}

#[test]
fn line_text_for_index_excludes_crlf_terminators() {
    let text = "alpha\r\nbeta\r\n";
    let starts = compute_line_starts(text);
    assert_eq!(starts, vec![0, 7, 13]);
    assert_eq!(line_text_for_index(text, starts.as_slice(), 0), "alpha");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 1), "beta");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 2), "");
}

#[gpui::test]
fn multiline_crlf_plain_layout_draws_without_painting_carriage_returns(
    cx: &mut gpui::TestAppContext,
) {
    let text = "[server]\r\nhost=local-api.internal\r\nport=8181\r\n";
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                soft_wrap: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| input.set_text(text, cx));
        let _ = window.draw(app);

        let input = input.read(app);
        assert_eq!(input.text(), text);
        let TextInputLayout::Plain(lines) = input
            .layout
            .last
            .as_ref()
            .expect("expected plain text input layout")
        else {
            panic!("expected plain text input layout");
        };
        assert_eq!(
            lines.get(0).expect("shaped line 0").text.as_ref(),
            "[server]"
        );
        assert_eq!(
            lines.get(1).expect("shaped line 1").text.as_ref(),
            "host=local-api.internal"
        );
    });
}

/// A plain multiline input inside a fixed-height scrolling viewport, the way
/// the merge tool hosts its resolved-output editor.
struct ScrolledPlainInputView {
    input: Entity<TextInput>,
    scroll_handle: ScrollHandle,
}

impl ScrolledPlainInputView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let scroll_handle = ScrollHandle::new();
        let input = cx.new({
            let scroll_handle = scroll_handle.clone();
            move |cx| {
                let mut input = TextInput::new(
                    TextInputOptions {
                        multiline: true,
                        soft_wrap: false,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                input.set_vertical_scroll_handle(Some(scroll_handle));
                input
            }
        });
        Self {
            input,
            scroll_handle,
        }
    }
}

impl Render for ScrolledPlainInputView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div()
                .id("plain_input_viewport")
                .w(px(600.0))
                .h(px(400.0))
                .overflow_y_scroll()
                .track_scroll(&self.scroll_handle)
                .child(self.input.clone()),
        )
    }
}

#[gpui::test]
fn mouse_drag_resolves_its_press_after_an_invalidated_layout(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha beta gamma", cx);
        });
        let _ = window.draw(app);
    });

    let (anchor_position, target_position) = cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input.layout.bounds.expect("text input bounds");
        let y = bounds.center().y;
        let position_for_offset = |wanted| {
            (0..f32::from(bounds.size.width).ceil() as usize)
                .map(|x| point(bounds.left() + px(x as f32), y))
                .find(|position| input.index_for_mouse_position(*position) == wanted)
                .unwrap_or_else(|| panic!("a hit position for offset {wanted}"))
        };
        (position_for_offset(6), position_for_offset(10))
    });

    // Replace the text and press before the next paint. The old implementation
    // resolved this temporarily layout-less press as byte zero.
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha beta gamma delta", cx);
            assert!(input.layout.last.is_none());
            input.on_mouse_down(
                &MouseDownEvent {
                    position: anchor_position,
                    modifiers: gpui::Modifiers::default(),
                    button: MouseButton::Left,
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
            assert_eq!(
                input.selection.range,
                input.text().len()..input.text().len(),
                "the unavailable layout must not invent a selection from byte zero"
            );
        });
        let _ = window.draw(app);
    });

    let expected = cx.update(|_window, app| {
        let input = input.read(app);
        let anchor = input.index_for_mouse_position(anchor_position);
        let target = input.index_for_mouse_position(target_position);
        anchor.min(target)..anchor.max(target)
    });
    assert!(expected.start > 0);
    cx.simulate_mouse_move(
        target_position,
        Some(MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.update(|_window, app| {
        assert_eq!(input.read(app).selection.range, expected);
    });
    cx.simulate_mouse_up(
        target_position,
        MouseButton::Left,
        gpui::Modifiers::default(),
    );
}

#[gpui::test]
fn plain_multiline_layout_shapes_only_the_viewport_of_a_large_document(
    cx: &mut gpui::TestAppContext,
) {
    // `ShapedLine` is ~3 KB, so a layout kept per document line costs megabytes
    // of zeroing on every frame of a large buffer — which is what made typing in
    // the merge tool's resolved output scale with the file instead of the
    // viewport.
    let line_count = 5_000;
    let text = (0..line_count)
        .map(|ix| format!("fn line_{ix:05}(value: usize) -> usize {{ value + {ix} }}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (view, cx) = cx.add_window_view(ScrolledPlainInputView::new);
    let input = cx.update(|_window, app| view.read(app).input.clone());

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text(text.clone(), cx);
            input.set_caret(0, cx);
        });
        let _ = window.draw(app);

        let input = input.read(app);
        let TextInputLayout::Plain(lines) = input
            .layout
            .last
            .as_ref()
            .expect("expected plain text input layout")
        else {
            panic!("expected plain text input layout");
        };
        assert_eq!(lines.line_count(), line_count);
        assert!(
            lines.shaped_line_count() < line_count / 4,
            "shaped {} of {line_count} lines — the layout should cover the viewport, not the document",
            lines.shaped_line_count(),
        );
        // The rows that are on screen still resolve, and off-screen rows report
        // no geometry rather than a bogus zero-width line.
        assert_eq!(
            lines.get(0).expect("shaped line 0").text.as_ref(),
            "fn line_00000(value: usize) -> usize { value + 0 }"
        );
        assert!(lines.get(line_count - 1).is_none());
    });
}

#[test]
fn wrapped_line_index_for_y_handles_row_boundaries() {
    let row_counts = vec![2, 1, 3];
    let y_offsets = vec![px(0.0), px(20.0), px(30.0)];
    let line_height = px(10.0);

    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(0.0)),
        0
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(19.0)),
        0
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(20.0)),
        1
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(30.0)),
        2
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(250.0)),
        2
    );
}

#[test]
fn line_display_columns_expands_tabs_to_tab_stops() {
    // No tabs: display columns equal the char/byte count.
    assert_eq!(
        line_display_columns("", TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
        0
    );
    assert_eq!(
        line_display_columns("abcd", TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
        4
    );

    // A leading tab advances to the first tab stop, not one column.
    assert_eq!(
        line_display_columns("\t", TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
        TEXT_INPUT_WRAP_TAB_STOP_COLUMNS
    );
    // "ab\t" -> 2 columns, then advance to the next multiple of the tab stop.
    assert_eq!(
        line_display_columns("ab\t", TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
        TEXT_INPUT_WRAP_TAB_STOP_COLUMNS
    );
    // A tab landing exactly on a stop still advances a full tab width.
    assert_eq!(
        line_display_columns("abcd\t", TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
        TEXT_INPUT_WRAP_TAB_STOP_COLUMNS * 2
    );
    // Several leading tabs (common source indentation).
    assert_eq!(
        line_display_columns("\t\tx", TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
        TEXT_INPUT_WRAP_TAB_STOP_COLUMNS * 2 + 1
    );

    // ASCII fast path and the non-ASCII char scan agree on tab handling.
    for sample in ["", "\t", "a\tb", "ab\tcd\tef", "\t\t\t", "trailing-tab\t"] {
        let mut reference = 0usize;
        for ch in sample.chars() {
            if ch == '\t' {
                reference += TEXT_INPUT_WRAP_TAB_STOP_COLUMNS
                    - (reference % TEXT_INPUT_WRAP_TAB_STOP_COLUMNS);
            } else {
                reference += 1;
            }
        }
        assert_eq!(
            line_display_columns(sample, TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
            reference,
            "sample={sample:?}"
        );
    }
}

#[gpui::test]
fn content_width_cache_updates_only_edited_lines(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("short\nlongest-line\nmid", cx);
            input.set_content_width_layout(true);
            assert_eq!(input.content_width_max_units(), "longest-line\n".len());

            input.replace_utf8_range(6..18, "x", cx);
            assert_eq!(input.content_width_max_units(), "short\n".len());

            input.replace_utf8_range(0..5, "\t\twide", cx);
            assert_eq!(
                input.content_width_max_units(),
                TEXT_INPUT_WRAP_TAB_STOP_COLUMNS * 2 + "wide\n".len()
            );
        });
    });
}

/// Maintaining the content-width cache must not flatten the document.
///
/// The cache needs the width of the rows an edit touched, which the rope answers
/// in O(log n) per row. Reading it from a materialized `&str` plus a whole
/// line-start array instead made every keystroke cost the document — the
/// regression this guards, worth ~7ms per keystroke on a 100k-line buffer.
#[gpui::test]
fn maintaining_the_content_width_cache_does_not_flatten_the_document(
    cx: &mut gpui::TestAppContext,
) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let text = (0..500)
        .map(|ix| format!("line {ix} with some content"))
        .collect::<Vec<_>>()
        .join("\n");

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text(&text, cx);
            input.set_content_width_layout(true);

            // The edit itself is what must stay windowed. `set_text` above is a
            // wholesale replacement and is allowed to build whatever it likes,
            // so measure only from here.
            input.replace_utf8_range(3..3, "z", cx);

            let snapshot = input.text_snapshot();
            assert!(
                !snapshot.is_materialized(),
                "an edit must not flatten the rope into a contiguous string"
            );
            assert!(
                !snapshot.is_line_index_built(),
                "an edit must not build a document-sized line-start index"
            );
        });
    });
}

#[gpui::test]
fn content_width_cache_tracks_line_splits_joins_and_undo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_content_width_layout(true);
            assert_eq!(
                input.content_width_cache.as_ref().unwrap().line_units.len(),
                2
            );

            input.replace_utf8_range(2..2, "\n", cx);
            assert_eq!(
                input.content_width_cache.as_ref().unwrap().line_units.len(),
                3
            );

            input.replace_utf8_range(2..3, "", cx);
            assert_eq!(
                input.content_width_cache.as_ref().unwrap().line_units.len(),
                2
            );

            input.undo(&Undo, window, cx);
            assert_eq!(
                input.content_width_cache.as_ref().unwrap().line_units.len(),
                3
            );
        });
    });
}

#[test]
fn estimate_wrap_rows_for_line_handles_tabs_and_overflow() {
    assert_eq!(
        estimate_wrap_rows_for_line("abcd", 4, TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
        1
    );
    assert_eq!(
        estimate_wrap_rows_for_line("abcde", 4, TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
        2
    );
    assert_eq!(
        estimate_wrap_rows_for_line("a\tb", 4, TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
        2
    );
}

#[test]
fn estimate_wrap_rows_for_line_matches_reference_for_ascii_tabs() {
    fn reference_wrap_rows_for_line(line_text: &str, wrap_columns: usize) -> usize {
        if line_text.is_empty() {
            return 1;
        }
        let wrap_columns = wrap_columns.max(1);
        let mut rows = 1usize;
        let mut column = 0usize;
        for ch in line_text.chars() {
            let width = if ch == '\t' {
                let rem = column % TEXT_INPUT_WRAP_TAB_STOP_COLUMNS;
                if rem == 0 {
                    TEXT_INPUT_WRAP_TAB_STOP_COLUMNS
                } else {
                    TEXT_INPUT_WRAP_TAB_STOP_COLUMNS - rem
                }
            } else {
                1
            };

            if width >= wrap_columns {
                if column > 0 {
                    rows += 1;
                }
                rows += width / wrap_columns;
                column = width % wrap_columns;
                if column == 0 {
                    column = wrap_columns;
                }
                continue;
            }

            if column + width > wrap_columns {
                rows += 1;
                column = width;
            } else {
                column += width;
            }
        }
        rows.max(1)
    }

    let samples = [
        "",
        "\t",
        "a\tb",
        "ab\tcd\tef",
        "\tsection_00000\tvalue = token\ttoken\ttoken\ttoken\t",
        "token\ttoken\ttoken\ttoken\ttoken\t",
        "abcd",
        "abcde",
        "\t\t\t",
        "trailing-tab\t",
    ];

    for wrap_columns in (TEXT_INPUT_WRAP_TAB_STOP_COLUMNS + 1)..=12 {
        for sample in samples {
            assert_eq!(
                estimate_wrap_rows_for_line(sample, wrap_columns, TEXT_INPUT_WRAP_TAB_STOP_COLUMNS),
                reference_wrap_rows_for_line(sample, wrap_columns),
                "sample={sample:?}, wrap_columns={wrap_columns}"
            );
        }
    }
}

fn text_input_test_position(input: &TextInput) -> Point<Pixels> {
    let bounds = input.layout.bounds.expect("expected text input bounds");
    let line_height = if input.layout.line_height.is_zero() {
        px(16.0)
    } else {
        input.layout.line_height
    };
    point(bounds.left() + px(2.0), bounds.top() + line_height / 2.0)
}

#[allow(clippy::single_range_in_vec_init)]
fn text_input_hotspot_ranges() -> Vec<Range<usize>> {
    vec![0..7]
}

#[gpui::test]
fn hotspot_hit_test_finds_range_at_pointer_position(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("deadbee target", cx);
        });
        let _ = window.draw(app);
    });

    let position = cx.update(|_window, app| text_input_test_position(input.read(app)));
    cx.update(|_window, app| {
        let hotspot = input
            .read(app)
            .hotspot_range_index_at_position(position, &text_input_hotspot_ranges());
        assert_eq!(hotspot, Some(0));
    });
}

#[gpui::test]
fn hotspot_hit_test_includes_right_side_of_final_glyph(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("deadbee target", cx);
        });
        let _ = window.draw(app);
    });

    let position = cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input.layout.bounds.expect("expected text input bounds");
        let line_height = if input.layout.line_height.is_zero() {
            px(16.0)
        } else {
            input.layout.line_height
        };
        let TextInputLayout::Plain(lines) = input
            .layout
            .last
            .as_ref()
            .expect("expected text input layout")
        else {
            panic!("expected plain text input layout");
        };
        let line = lines.get(0).expect("expected first shaped line");
        let final_glyph_left = line.x_for_index(6);
        let final_glyph_right = line.x_for_index(7);
        let final_glyph_width = final_glyph_right - final_glyph_left;

        point(
            bounds.left() + final_glyph_left + (final_glyph_width * 3.0) / 4.0,
            bounds.top() + line_height / 2.0,
        )
    });
    cx.update(|_window, app| {
        let hotspot = input
            .read(app)
            .hotspot_range_index_at_position(position, &text_input_hotspot_ranges());
        assert_eq!(hotspot, Some(0));
    });
}

#[gpui::test]
fn hotspot_hit_test_returns_none_outside_bounds(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("deadbee target", cx);
        });
        let _ = window.draw(app);
    });

    let outside = cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input.layout.bounds.expect("expected text input bounds");
        point(bounds.left() + px(2.0), bounds.top() - px(2.0))
    });
    cx.update(|_window, app| {
        let hotspot = input
            .read(app)
            .hotspot_range_index_at_position(outside, &text_input_hotspot_ranges());
        assert_eq!(hotspot, None);
    });
}

#[gpui::test]
fn hotspot_hit_test_ignores_trailing_blank_space_after_link(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("deadbee", cx);
        });
        let _ = window.draw(app);
    });

    let position = cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input.layout.bounds.expect("expected text input bounds");
        let line_height = if input.layout.line_height.is_zero() {
            px(16.0)
        } else {
            input.layout.line_height
        };
        let position = point(bounds.right() - px(2.0), bounds.top() + line_height / 2.0);
        assert!(bounds.contains(&position));
        assert_eq!(input.offset_for_position(position), 7);
        position
    });
    cx.update(|_window, app| {
        let hotspot = input
            .read(app)
            .hotspot_range_index_at_position(position, &text_input_hotspot_ranges());
        assert_eq!(hotspot, None);
    });
}

#[gpui::test]
fn hotspot_bounds_match_range_extent(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("deadbee target", cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input
            .hotspot_bounds(&(0..7))
            .expect("expected hotspot bounds");
        assert!(bounds.size.width > px(0.0));
        assert!(bounds.size.height > px(0.0));
        assert!(bounds.contains(&text_input_test_position(input)));
    });
}

#[gpui::test]
fn hotspot_bounds_report_only_the_first_line_of_a_range_that_wraps(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let text = "deadbee\nfeedface";
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text(text.to_string(), cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let input = input.read(app);
        let first_line = input
            .hotspot_bounds(&(0..7))
            .expect("expected hotspot bounds for a range on one line");
        // The second line's x carries no information about how far the first
        // one runs, and can even sit left of where the range started.
        let across_lines = input
            .hotspot_bounds(&(0..text.len()))
            .expect("expected hotspot bounds for a range that spans two lines");

        assert_eq!(
            across_lines.origin, first_line.origin,
            "a range that wraps still begins where its first line begins"
        );
        assert_eq!(
            across_lines.bottom(),
            first_line.bottom(),
            "and it still ends with that line rather than a later one"
        );
        assert!(across_lines.size.width > px(0.0));
        assert!(across_lines.size.height > px(0.0));
    });
}

#[gpui::test]
fn truncated_line_hit_testing_snaps_ellipsis_to_hidden_range_boundaries(
    cx: &mut gpui::TestAppContext,
) {
    let text: SharedString = "0123456789abcdef0123456789abcdef".into();
    let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

    cx.update(|window, app| {
        let line = shape_truncated_line_cached(
            window,
            app,
            &window.text_style(),
            &text,
            Some(px(80.0)),
            TextTruncationProfile::Middle,
            &[],
            None,
        );

        assert!(line.truncated, "expected the line to truncate");
        let (hidden_range, display_range) = line
            .projection
            .ellipsis_segment_for_source_offset(text.len() / 2)
            .expect("expected a middle ellipsis segment");

        let start_x = line.shaped_line.x_for_index(display_range.start);
        let end_x = line.shaped_line.x_for_index(display_range.end);
        let span = end_x - start_x;
        let left_x = start_x + span / 4.0;
        let right_x = start_x + (span * 3.0) / 4.0;

        assert_eq!(
            truncated_line_source_offset_for_x(&line, left_x),
            hidden_range.start
        );
        assert_eq!(
            truncated_line_source_offset_for_x(&line, right_x),
            hidden_range.end
        );
    });
}

#[gpui::test]
fn focused_truncated_line_hit_testing_snaps_both_ellipsis_segments_to_hidden_boundaries(
    cx: &mut gpui::TestAppContext,
) {
    let text: SharedString = "prefix-aaaaaaaaaa-suffix".into();
    let focus = 7..17;
    let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

    cx.update(|window, app| {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let runs_four = vec![style.clone().to_run("…aaaa…".len())];
        let runs_five = vec![style.clone().to_run("…aaaaa…".len())];
        let width_four = window
            .text_system()
            .shape_line("…aaaa…".into(), font_size, &runs_four)
            .width;
        let width_five = window
            .text_system()
            .shape_line("…aaaaa…".into(), font_size, &runs_five)
            .width;
        let max_width = width_four + (width_five - width_four) / 2.0;

        let line = shape_truncated_line_cached(
            window,
            app,
            &style,
            &text,
            Some(max_width),
            TextTruncationProfile::Middle,
            &[],
            Some(focus),
        );

        assert!(line.truncated, "expected the line to truncate");

        let (left_hidden_range, left_display_range) = line
            .projection
            .ellipsis_segment_for_source_offset(0)
            .expect("expected a left ellipsis segment");
        let (right_hidden_range, right_display_range) = line
            .projection
            .ellipsis_segment_for_source_offset(text.len())
            .expect("expected a right ellipsis segment");

        assert_ne!(left_display_range, right_display_range);

        let left_x0 = line.shaped_line.x_for_index(left_display_range.start);
        let left_x1 = line.shaped_line.x_for_index(left_display_range.end);
        let left_span = left_x1 - left_x0;
        let left_inside = left_x0 + left_span / 4.0;
        let left_outside = left_x0 + (left_span * 3.0) / 4.0;

        assert_eq!(
            truncated_line_source_offset_for_x(&line, left_inside),
            left_hidden_range.start
        );
        assert_eq!(
            truncated_line_source_offset_for_x(&line, left_outside),
            left_hidden_range.end
        );

        let right_x0 = line.shaped_line.x_for_index(right_display_range.start);
        let right_x1 = line.shaped_line.x_for_index(right_display_range.end);
        let right_span = right_x1 - right_x0;
        let right_inside = right_x0 + right_span / 4.0;
        let right_outside = right_x0 + (right_span * 3.0) / 4.0;

        assert_eq!(
            truncated_line_source_offset_for_x(&line, right_inside),
            right_hidden_range.start
        );
        assert_eq!(
            truncated_line_source_offset_for_x(&line, right_outside),
            right_hidden_range.end
        );
    });
}

#[gpui::test]
fn replace_utf8_range_clears_shaped_row_caches(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                soft_wrap: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);

            input.layout.plain_line_cache.insert(
                ShapedRowCacheKey {
                    line_ix: 0,
                    font_size_key: 13,
                },
                EditorLine::default(),
            );

            assert_eq!(input.layout.plain_line_cache.len(), 1);

            input.replace_utf8_range(0..5, "gamma", cx);

            assert!(
                input.layout.plain_line_cache.is_empty(),
                "text edits must invalidate cached plain shaped rows"
            );
        });
    });
}

/// Rendering a frame must not flatten the document into one string.
///
/// The plain arm reads row text through `LineTextSource::window`, which copies
/// only the rows it is about to shape. If any path in prepaint reaches for
/// `as_str()` instead, the model's materialization cache warms up and the frame
/// silently starts costing the whole buffer — the exact regression that makes
/// typing in a large file scale with the file. Nothing observable fails when
/// that happens, so observing the cache is the only way to hold the line.
#[gpui::test]
fn drawing_a_plain_multiline_frame_does_not_materialize_the_document(
    cx: &mut gpui::TestAppContext,
) {
    let line_count = 5_000;
    let text = (0..line_count)
        .map(|ix| format!("fn line_{ix:05}(value: usize) -> usize {{ value + {ix} }}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (view, cx) = cx.add_window_view(ScrolledPlainInputView::new);
    let input = cx.update(|_window, app| view.read(app).input.clone());

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text(text.clone(), cx);
            input.set_caret(0, cx);
        });
        let _ = window.draw(app);
    });

    // An edit clears the cache, so this asserts the *frame* keeps it cold
    // rather than merely observing a buffer that was never flattened.
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.replace_utf8_range(0..0, "// edited\n", cx);
        });
        assert!(
            !input.read(app).content.snapshot().is_materialized(),
            "an edit should have dropped the materialization cache"
        );
        let _ = window.draw(app);

        assert!(
            !input.read(app).content.snapshot().is_materialized(),
            "drawing a frame must cost the viewport, not the document"
        );

        // KNOWN GAP: the companion assertion
        //
        //     assert!(!input.read(app).content.snapshot().is_line_index_built());
        //
        // does not hold yet, so it is deliberately absent rather than silently
        // forgotten. Prepaint still calls `shared_line_starts()`, and the edit
        // above cleared that `OnceLock`, so each post-edit frame rebuilds a
        // row-per-entry index: measured at 1.6ms for 100k rows, roughly a sixth
        // of the frame. Closing it means threading windowed row access through
        // the ~10 places prepaint and paint read `line_starts`, at which point
        // this assertion belongs here — see
        // `maintaining_the_content_width_cache_does_not_flatten_the_document`,
        // which does make it for the edit path.
    });

    // And the frame still renders the right rows.
    cx.update(|_window, app| {
        let input = input.read(app);
        let TextInputLayout::Plain(lines) = input
            .layout
            .last
            .as_ref()
            .expect("expected plain text input layout")
        else {
            panic!("expected plain text input layout");
        };
        assert_eq!(lines.line_count(), line_count + 1);
        assert!(lines.shaped_line_count() < line_count / 4);
    });
}

/// The windowed and whole-document row sources must return identical text.
///
/// The plain arm shapes through `LineTextSource::Window` and the soft-wrap and
/// masked arms through `Whole`; any disagreement makes the same row measure
/// differently depending on which arm drew it. The awkward case is a final row
/// with no trailing newline: `Window` used to invent a terminator for it, so a
/// row ending in a bare `\r` lost that character in one arm only.
#[test]
fn windowed_and_whole_row_text_agree_including_an_unterminated_final_row() {
    use crate::text_model::TextModel;

    for document in [
        "alpha\nbeta\ngamma",
        "alpha\r\nbeta\r\ngamma\r",
        "alpha\r\nbeta\r\ngamma\r\n",
        "only-one-row",
        "trailing\n",
        "",
    ] {
        let model = TextModel::from_large_text(document);
        let snapshot = model.snapshot();
        let starts = compute_line_starts(document);
        let whole = LineTextSource::Whole {
            text: document,
            starts: starts.as_slice(),
        };
        let row_count = snapshot.line_count();
        let window = LineTextSource::window(&snapshot, 0..row_count, None);

        for row in 0..row_count {
            assert_eq!(
                window.line_text(row),
                whole.line_text(row),
                "row {row} of {document:?}"
            );
        }

        // The caret's row is stored separately when it falls outside the
        // window, and has to agree with the in-window rendering of the same
        // row — otherwise scrolling the caret in and out changes its width.
        for row in 0..row_count {
            let elsewhere = if row == 0 {
                row_count.saturating_sub(1)
            } else {
                0
            };
            let stray_window =
                LineTextSource::window(&snapshot, elsewhere..elsewhere + 1, Some(row));
            assert_eq!(
                stray_window.line_text(row),
                whole.line_text(row),
                "stray row {row} of {document:?}"
            );
        }
    }
}

#[gpui::test]
fn inputs_restyle_when_their_window_theme_changes(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                placeholder: "theme".into(),
                ..Default::default()
            },
            window,
            cx,
        )
    });
    let window_id = cx.update(|window, _| window.window_handle().window_id());
    for theme in [AppTheme::gitcomet_light(), AppTheme::gitcomet_dark()] {
        cx.update(|_, app| crate::theme::set_window_theme(app, window_id, theme));
        crate::test_support::refresh_and_draw(cx);
        let color = cx.update(|_, app| input.read(app).debug_text_color());
        assert_eq!(color, TextInputStyle::from_theme(theme).text);
    }
}

/// Selection-quad capacity painted by a caret-only frame, then by a frame
/// with everything selected.
fn selection_capacity_painted(
    cx: &mut gpui::TestAppContext,
    options: TextInputOptions,
    text: &str,
    truncate: bool,
) -> (usize, usize) {
    use crate::text_input::element::take_selection_quad_capacity_for_tests;

    let (input, cx) = cx.add_window_view(|window, cx| TextInput::new(options, window, cx));
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text(text, cx);
            if truncate {
                input.set_display_truncation(Some(TextTruncationProfile::End), cx);
            }
            input.set_caret(0, cx);
        });
    });
    take_selection_quad_capacity_for_tests();
    crate::test_support::refresh_and_draw(cx);
    let caret = take_selection_quad_capacity_for_tests();

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_selected_range(0..text.len(), false, window, cx);
        });
    });
    crate::test_support::refresh_and_draw(cx);
    let selection = take_selection_quad_capacity_for_tests();
    (caret, selection)
}

/// A caret-only frame used to reserve a 208-byte `PaintQuad` per visible line
/// for a selection it never painted, on every prepaint of the file editor.
#[gpui::test]
fn caret_frame_reserves_no_selection_quads_in_a_plain_input(cx: &mut gpui::TestAppContext) {
    let options = TextInputOptions {
        multiline: true,
        ..Default::default()
    };
    let (caret, selection) = selection_capacity_painted(cx, options, "one\ntwo\nthree", false);
    assert_eq!(caret, 0);
    assert!(selection > 0, "the probe must see a selection's quads");
}

#[gpui::test]
fn caret_frame_reserves_no_selection_quads_in_a_soft_wrapped_input(cx: &mut gpui::TestAppContext) {
    let options = TextInputOptions {
        multiline: true,
        soft_wrap: true,
        ..Default::default()
    };
    let (caret, selection) = selection_capacity_painted(cx, options, "one\ntwo\nthree", false);
    assert_eq!(caret, 0);
    assert!(selection > 0, "the probe must see a selection's quads");
}

#[gpui::test]
fn caret_frame_reserves_no_selection_quads_in_a_truncated_line(cx: &mut gpui::TestAppContext) {
    let (caret, selection) =
        selection_capacity_painted(cx, TextInputOptions::selectable(), "truncated", true);
    assert_eq!(caret, 0);
    assert!(selection > 0, "the probe must see a selection's quads");
}
