use super::state::*;
use super::wrap::*;
use super::*;
use crate::test_support::refresh_and_draw as draw_frame;

struct WrappedInputView {
    input: Entity<TextInput>,
    scroll: ScrollHandle,
    width: Pixels,
}

impl WrappedInputView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let scroll = ScrollHandle::new();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(
                TextInputOptions {
                    multiline: true,
                    soft_wrap: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(scroll.clone()));
            input
        });
        Self {
            input,
            scroll,
            width: px(280.0),
        }
    }
}

impl Render for WrappedInputView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div().w(self.width).child(
                crate::view::components::ScrollContainer::vertical(
                    "wrap_test_surface",
                    "wrap_test_scrollbar",
                    self.scroll.clone(),
                    px(140.0),
                )
                .render(AppTheme::gitcomet_dark(), self.input.clone()),
            ),
        )
    }
}

fn seed_wrapped_input(
    view: &Entity<WrappedInputView>,
    cx: &mut gpui::VisualTestContext,
    text: &str,
) -> Entity<TextInput> {
    let input = cx.update(|window, app| {
        let input = view.read(app).input.clone();
        input.update(app, |input, cx| {
            input.set_text(text.to_owned(), cx);
            input.set_caret(0, cx);
            window.focus(&input.focus_handle(), cx);
        });
        input
    });
    for _ in 0..4 {
        draw_frame(cx);
    }
    input
}

#[gpui::test]
fn reversed_ime_ranges_keep_multiline_text_and_row_caches_in_step(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let original = "first é😀\nsecond\nthird\nfourth";
    for content_width in [false, true] {
        for composing in [false, true] {
            for end in [original.find("third").unwrap(), original.len()] {
                let input = seed_wrapped_input(&view, cx, original);
                cx.update(|window, app| {
                    input.update(app, |input, cx| {
                        input.set_content_width_layout(content_width);
                        let start = original.find('é').unwrap();
                        let reversed = input.offset_to_utf16(end)..input.offset_to_utf16(start);
                        input.drain_recent_utf8_edit_deltas();
                        if composing {
                            input.replace_and_mark_text_in_range(
                                Some(reversed),
                                "new 😀\ntext",
                                None,
                                window,
                                cx,
                            );
                        } else {
                            input.replace_text_in_range(Some(reversed), "new 😀\ntext", window, cx);
                        }
                        let mut expected = original.to_owned();
                        expected.replace_range(start..end, "new 😀\ntext");
                        assert_eq!(input.text(), expected);
                        assert_eq!(
                            input.wrap.row_counts.len(),
                            input.text_snapshot().line_count()
                        );
                        assert_eq!(
                            input.drain_recent_utf8_edit_deltas(),
                            vec![(start..end, start..start + "new 😀\ntext".len())]
                        );
                        if content_width {
                            assert_eq!(
                                input.content_width_cache.as_ref().unwrap().line_units.len(),
                                input.text_snapshot().line_count()
                            );
                        }
                        input.unmark_text(window, cx);
                    });
                });
                draw_frame(cx);
            }
        }
    }
}

#[gpui::test]
fn shrinking_wrap_estimates_fill_the_viewport_in_the_same_frame(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let input = seed_wrapped_input(&view, cx, &"short line\n".repeat(300));
    cx.run_until_parked();
    cx.update(|_window, app| {
        input.update(app, |input, _| {
            // Model a pessimistic estimate before these lines have been shaped.
            // Each pass exposes another line as the previous one shrinks.
            input.wrap.row_counts.fill(40);
            input.wrap.row_counts_current.fill(false);
            input.wrap.recompute_requested = false;
            input.wrap.pending_job = None;
            let rows = total_wrap_rows(&input.wrap.row_counts);
            input.wrap.cache.as_mut().unwrap().rows = rows;
            input.wrap.last_rows = Some(rows);
            input.interaction.pending_cursor_autoscroll = false;
        });
    });
    draw_frame(cx);
    cx.update(|_window, app| {
        let input = input.read(app);
        let viewport = view.read(app).scroll.bounds();
        let origin = input.layout.bounds.unwrap().top();
        let TextInputLayout::Wrapped {
            lines,
            y_offsets,
            row_counts,
        } = input.layout.last.as_ref().unwrap()
        else {
            panic!("wrapped layout")
        };
        let visible = visible_wrapped_line_range(
            y_offsets,
            row_counts,
            input.layout.line_height,
            viewport.top() - origin,
            viewport.bottom() - origin,
            0,
        );
        assert!(
            visible.len() > 2,
            "fixture must expose more lines than the old two passes"
        );
        for line in visible {
            assert!(
                lines[line].len() > 0,
                "visible line {line} was left blank for this frame"
            );
        }
    });
}

#[gpui::test]
fn offscreen_wrapped_edits_keep_measured_height_until_current_text_is_shaped(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let paragraph = format!("{}\n", "aaaaaaa bbbbbbb ccccccc ".repeat(12));
    let input = seed_wrapped_input(&view, cx, &paragraph.repeat(60));
    let measured = cx.update(|_window, app| input.read(app).wrap.row_counts[0]);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            let (font, size) = input.wrap.row_counts_font.as_ref().unwrap();
            let _ = font;
            let estimate = estimate_wrap_rows_for_line(
                &paragraph,
                wrap_columns_for_width(input.wrap.row_counts_width.unwrap(), *size),
            );
            assert_ne!(
                estimate, measured,
                "fixture must distinguish shaping from estimation"
            );
            let at = input.text_snapshot().line_range(30).start;
            input.set_selected_range(at..at, true, window, cx);
        });
    });
    for _ in 0..8 {
        draw_frame(cx);
    }
    let baseline = cx.update(|_window, app| view.read(app).scroll.offset());
    for step in 0..4 {
        cx.update(|_window, app| {
            input.update(app, |input, cx| {
                input.replace_utf8_range_preserving_view(
                    0..1,
                    if step % 2 == 0 { "b" } else { "a" },
                    cx,
                );
            });
        });
        for frame in 0..3 {
            draw_frame(cx);
            cx.update(|_window, app| {
                assert_eq!(
                    input.read(app).wrap.row_counts[0],
                    measured,
                    "offscreen edit {step}, frame {frame}"
                );
                assert_eq!(view.read(app).scroll.offset(), baseline);
            });
        }
    }
}

#[gpui::test]
fn wrapped_reveals_survive_multiple_destination_layout_corrections(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let paragraph = format!("{}\n", "aaaaaaa bbbbbbb ccccccc ".repeat(12));
    for eof in [true, false] {
        let input = seed_wrapped_input(&view, cx, &paragraph.repeat(300));
        cx.run_until_parked();
        cx.update(|_window, app| {
            view.read(app).scroll.set_offset(point(px(0.0), px(-400.0)));
            input.update(app, |input, _| {
                // A foreground estimate may stop before reaching the target.
                input.wrap.row_counts.fill(1);
                input.wrap.row_counts_current.fill(false);
                input.wrap.recompute_requested = false;
                input.wrap.pending_job = None;
                let rows = input.wrap.row_counts.len();
                input.wrap.cache.as_mut().unwrap().rows = rows;
                input.wrap.last_rows = Some(rows);
            });
        });
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                if eof {
                    input.document_end(&DocumentEnd, window, cx);
                } else {
                    let at = input.text_snapshot().line_range(260).start + 30;
                    input.set_selected_range(at..at + 5, true, window, cx);
                }
            });
        });
        for _ in 0..16 {
            draw_frame(cx);
        }
        cx.update(|_window, app| {
            let input = input.read(app);
            let viewport = view.read(app).scroll.bounds();
            let origin = input.layout.bounds.unwrap().top();
            let (top, bottom) = input.cursor_vertical_span(input.cursor_offset()).unwrap();
            assert!(
                origin + top >= viewport.top() - px(1.0)
                    && origin + bottom <= viewport.bottom() + px(1.0),
                "eof={eof}: caret {:?}..{:?}, viewport {viewport:?}",
                origin + top,
                origin + bottom
            );
            assert!(
                !input.interaction.pending_cursor_autoscroll,
                "reveal must settle"
            );
        });
    }
}

#[gpui::test]
fn waiting_for_wrap_layout_does_not_exhaust_destination_reveals(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let paragraph = format!("{}\n", "aaaaaaa bbbbbbb ccccccc ".repeat(12));
    let input = seed_wrapped_input(&view, cx, &paragraph.repeat(300));
    cx.run_until_parked();
    cx.update(|_window, app| view.read(app).scroll.set_offset(point(px(0.0), px(-800.0))));
    for _ in 0..3 {
        draw_frame(cx);
    }
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.document_end(&DocumentEnd, window, cx);
            // Exercise the state after shaping updates the height, while the
            // parent still reports its earlier scroll extent. Waiting here
            // must not spend the attempts needed after scrolling to EOF.
            input.layout.bounds.as_mut().unwrap().size.height -= input.layout.line_height;
            for _ in 0..TEXT_INPUT_CURSOR_AUTOSCROLL_RETRIES {
                input.ensure_cursor_visible_in_vertical_scroll(cx);
            }
        });
    });
    for _ in 0..12 {
        draw_frame(cx);
    }
    cx.update(|_window, app| {
        let input = input.read(app);
        let viewport = view.read(app).scroll.bounds();
        let origin = input.layout.bounds.unwrap().top();
        let (top, bottom) = input.cursor_vertical_span(input.cursor_offset()).unwrap();
        assert!(
            origin + top >= viewport.top() - px(1.0)
                && origin + bottom <= viewport.bottom() + px(1.0),
            "caret {:?}..{:?}, viewport {viewport:?}",
            origin + top,
            origin + bottom
        );
        assert!(!input.interaction.pending_cursor_autoscroll);
    });
}

#[gpui::test]
fn wrapped_highlight_updates_preserve_measured_height(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let text = "a paragraph with several words ".repeat(80);
    let input = seed_wrapped_input(&view, cx, &text);
    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let rows = input.wrap.row_counts.clone();
            assert!(rows[0] > 1, "fixture must actually wrap: {rows:?}");
            let height = input.wrap.last_rows;
            let highlights = vec![(
                0..1,
                gpui::HighlightStyle {
                    background_color: Some(gpui::hsla(0.1, 0.5, 0.5, 1.0)),
                    ..Default::default()
                },
            )];
            input.set_highlights(highlights, cx);
            assert_eq!(input.wrap.row_counts, rows);
            assert_eq!(input.wrap.last_rows, height);
            input.set_highlight_provider_with_key(
                1,
                HighlightProvider::from_fn(|_| Vec::new()),
                input.content.len(),
                cx,
            );
            assert_eq!(input.wrap.row_counts, rows);
            assert_eq!(input.wrap.last_rows, height);
            input.note_provider_highlights_changed();
            assert_eq!(input.wrap.row_counts, rows);
            assert_eq!(input.wrap.last_rows, height);
        });
    });
}

#[gpui::test]
fn wrapped_typing_and_highlight_rebinding_keep_every_frame_stable(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let paragraph = "words with room for editing ".repeat(12);
    let text = (0..60)
        .map(|_| paragraph.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let input = seed_wrapped_input(&view, cx, &text);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            let cursor = input.text_snapshot().line_range(20).start + 2;
            input.set_selected_range(cursor..cursor, true, window, cx);
        });
    });
    for _ in 0..4 {
        draw_frame(cx);
    }
    cx.update(|_window, app| {
        let scroll = view.read(app).scroll.clone();
        let input = input.read(app);
        let (top, _) = input.cursor_vertical_span(input.cursor_offset()).unwrap();
        scroll.set_offset(point(px(0.0), -(top - px(40.0))));
    });
    for _ in 0..3 {
        draw_frame(cx);
    }
    let (offset, width, rows) = cx.update(|_window, app| {
        let input = input.read(app);
        assert!(input.wrap.row_counts[20] > 1);
        (
            view.read(app).scroll.offset(),
            input.layout.bounds.unwrap().size.width,
            input.wrap.row_counts.clone(),
        )
    });
    assert!(offset.y < px(0.0));

    for step in 0..12 {
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                match step % 6 {
                    0 => input.replace_text_in_range(None, "x", window, cx),
                    1 => input.backspace(&Backspace, window, cx),
                    2 => input.undo(&Undo, window, cx),
                    3 => input.redo(&Redo, window, cx),
                    4 => input.replace_and_mark_text_in_range(None, "x", None, window, cx),
                    _ => {
                        input.unmark_text(window, cx);
                        input.backspace(&Backspace, window, cx);
                    }
                }
                input.set_highlight_provider_with_key(
                    step + 1,
                    HighlightProvider::from_fn(|_| Vec::new()),
                    input.content.len(),
                    cx,
                );
            });
        });
        for frame in 0..4 {
            draw_frame(cx);
            cx.update(|_window, app| {
                let actual = view.read(app).scroll.offset();
                assert!(
                    (actual.y - offset.y).abs() <= px(1.0),
                    "step {step}, frame {frame}: {offset:?} -> {actual:?}"
                );
                assert_eq!(actual.x, px(0.0));
                let input = input.read(app);
                assert_eq!(input.layout.bounds.unwrap().size.width, width);
                assert_eq!(
                    input.wrap.row_counts, rows,
                    "fixture edits must not add a wrapped row"
                );
            });
        }
    }
}

#[gpui::test]
fn wrapped_document_end_reveals_the_caret_after_destination_rows_are_measured(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let paragraph = format!("{}\n", "aaaaaaa bbbbbbb ccccccc ".repeat(12));
    let input = seed_wrapped_input(&view, cx, &paragraph.repeat(300));
    cx.run_until_parked();
    cx.update(|_window, app| {
        view.read(app).scroll.set_offset(point(px(0.0), px(-800.0)));
    });
    for _ in 0..3 {
        draw_frame(cx);
    }
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.document_end(&DocumentEnd, window, cx);
        });
    });
    for _ in 0..8 {
        draw_frame(cx);
    }
    let offset = cx.update(|_window, app| {
        let input = input.read(app);
        let viewport = view.read(app).scroll.bounds();
        let bounds = input.layout.bounds.unwrap();
        let (top, bottom) = input.cursor_vertical_span(input.cursor_offset()).unwrap();
        assert!(
            bounds.top() + top >= viewport.top() - px(1.0)
                && bounds.top() + bottom <= viewport.bottom() + px(1.0),
            "EOF caret must remain visible after wrapping settles: caret {:?}..{:?}, viewport {viewport:?}",
            bounds.top() + top,
            bounds.top() + bottom,
        );
        assert!(!input.interaction.pending_cursor_autoscroll);
        view.read(app).scroll.offset()
    });
    for _ in 0..3 {
        draw_frame(cx);
        cx.update(|_window, app| assert_eq!(view.read(app).scroll.offset(), offset));
    }
}

#[gpui::test]
fn wrapped_message_width_is_stable_when_vertical_overflow_starts(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let input = seed_wrapped_input(&view, cx, "short");
    let width = cx.update(|_window, app| {
        assert_eq!(view.read(app).scroll.max_offset().y, px(0.0));
        input.read(app).layout.bounds.unwrap().size.width
    });
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.replace_text_in_range(None, &"\nanother line".repeat(30), window, cx);
        });
    });
    for _ in 0..4 {
        draw_frame(cx);
        cx.update(|_window, app| {
            assert_eq!(input.read(app).layout.bounds.unwrap().size.width, width)
        });
    }
    cx.update(|_window, app| assert!(view.read(app).scroll.max_offset().y > px(0.0)));
}

#[gpui::test]
fn wrapped_font_and_viewport_changes_reflow_then_settle(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let input = seed_wrapped_input(&view, cx, &"several words ".repeat(80));
    let mut previous_rows = cx.update(|_window, app| input.read(app).wrap.row_counts[0]);
    assert!(previous_rows > 1);
    for font_change in [false, true] {
        cx.update(|_window, app| {
            view.update(app, |view, cx| {
                if font_change {
                    let mut appearance = crate::appearance::current(cx);
                    appearance.ui_font_size_px = 24;
                    cx.set_global(appearance);
                } else {
                    view.width = px(180.0);
                }
                cx.notify();
            });
        });
        for _ in 0..4 {
            draw_frame(cx);
        }
        let rows = cx.update(|_window, app| input.read(app).wrap.row_counts[0]);
        assert!(
            rows > previous_rows,
            "font change {font_change}: expected more rows than {previous_rows}, got {rows}"
        );
        for _ in 0..3 {
            draw_frame(cx);
            cx.update(|_window, app| assert_eq!(input.read(app).wrap.row_counts[0], rows));
        }
        previous_rows = rows;
    }
}

#[gpui::test]
fn wrap_estimates_do_not_replace_current_rows_on_small_or_large_documents(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let input = seed_wrapped_input(&view, cx, "seed");
    for count in [20, TEXT_INPUT_WRAP_SYNC_LINE_THRESHOLD + 20] {
        cx.update(|_window, app| {
            input.update(app, |input, cx| {
                let text = "word ".repeat(100) + "\n";
                input.set_text(text.repeat(count), cx);
                let snapshot = input.text_snapshot();
                let starts = snapshot.shared_line_starts();
                let count = starts.len();
                input.wrap.row_counts = vec![1; count];
                input.wrap.row_counts_current = vec![false; count];
                input.set_measured_wrap_rows(0, 7);
                input.request_wrap_recompute();
                input.maybe_recompute_wrap_rows(
                    snapshot.as_ref(),
                    &starts,
                    px(200.0),
                    px(16.0),
                    count,
                    cx,
                );
                assert_eq!(input.wrap.row_counts[0], 7);
                input.set_measured_wrap_rows(1, 9);
                input.maybe_recompute_wrap_rows(
                    snapshot.as_ref(),
                    &starts,
                    px(200.0),
                    px(16.0),
                    count,
                    cx,
                );
                assert_eq!(
                    input.wrap.row_counts[1], 9,
                    "an idle frame must retain measured rows"
                );
                if let Some(job) = input.wrap.pending_job {
                    input.complete_wrap_recompute_job(
                        job.sequence,
                        job.width_key,
                        count,
                        vec![3; count],
                        cx,
                    );
                    assert_eq!(&input.wrap.row_counts[..3], &[7, 9, 3]);
                }
            });
        });
    }
}

#[gpui::test]
fn wrapped_edits_rebase_rows_and_pending_dirty_ranges(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let input = seed_wrapped_input(&view, cx, "alpha\nbeta\ngamma\ndelta");
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.wrap.row_counts = vec![2, 3, 4, 5];
            input.wrap.row_counts_current = vec![true; 4];
            // Two changes before the next frame: the later dirty line must move
            // along with its text when a preceding line gains a newline.
            input.replace_utf8_range(12..12, "x", cx);
            input.replace_utf8_range(2..2, "\n", cx);
            assert_eq!(&input.wrap.row_counts[2..], &[3, 4, 5]);
            assert_eq!(input.take_normalized_wrap_dirty_ranges(5), vec![0..2, 3..4]);
            input.undo(&Undo, window, cx);
            assert_eq!(&input.wrap.row_counts[1..], &[3, 4, 5]);
            input.redo(&Redo, window, cx);
            assert_eq!(&input.wrap.row_counts[2..], &[3, 4, 5]);
            // An IME update uses the same edit bookkeeping.
            input.replace_and_mark_text_in_range(Some(0..1), "é\n", None, window, cx);
            assert_eq!(&input.wrap.row_counts[3..], &[3, 4, 5]);
            input.unmark_text(window, cx);
        });
    });
}

#[gpui::test]
fn wrapped_background_results_cannot_overwrite_edits_or_changed_line_indices(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = cx.add_window_view(WrappedInputView::new);
    let input = seed_wrapped_input(&view, cx, "alpha\nbeta\ngamma");
    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.wrap.row_counts = vec![2, 3, 4];
            input.wrap.row_counts_current = vec![false; 3];
            let job = PendingWrapJob {
                sequence: 40,
                width_key: 200,
                line_count: 3,
                wrap_columns: 20,
            };
            input.wrap.pending_job = Some(job);
            input.replace_utf8_range(7..7, "typed", cx);
            input.complete_wrap_recompute_job(40, 200, 3, vec![1; 3], cx);
            assert_eq!(
                input.wrap.row_counts,
                vec![1, 3, 1],
                "edited row stays provisional until current text is shaped"
            );
            input.wrap.pending_job = Some(job);
            input.replace_utf8_range(0..0, "\n", cx);
            let rows = input.wrap.row_counts.clone();
            input.complete_wrap_recompute_job(40, 200, 3, vec![99; 3], cx);
            assert_eq!(input.wrap.row_counts, rows);
            assert_eq!(rows.len(), 4);
            assert!(input.wrap.recompute_requested);
        });
    });
}

/// Exercise the platform's actual font shaper without opening a desktop window.
#[cfg(target_os = "linux")]
#[test]
fn wrapped_real_font_typing_is_stable_at_normal_and_enlarged_scale() {
    let _visual_guard = crate::test_support::lock_visual_test();
    let platform = gpui_platform::current_platform(true);
    for percent in [100, 150] {
        let mut cx = gpui::HeadlessAppContext::new(platform.text_system());
        let window = cx
            .open_window(size(px(640.0), px(480.0)), |window, app| {
                crate::ui_scale::set_current(app, percent);
                crate::ui_scale::apply_to_window(window, percent);
                app.new(|cx| {
                    let mut view = WrappedInputView::new(window, cx);
                    view.width = px(280.0 * percent as f32 / 100.0);
                    view
                })
            })
            .unwrap();
        let (input, scroll) = cx
            .update_window(window.into(), |root, window, app| {
                let root = root.downcast::<WrappedInputView>().unwrap();
                let input = root.read(app).input.clone();
                let scroll = root.read(app).scroll.clone();
                input.update(app, |input, cx| {
                    input.set_text(
                        format!("{}end\n", "alpha beta gamma delta ".repeat(11)).repeat(30),
                        cx,
                    );
                    input.set_caret(0, cx);
                    window.focus(&input.focus_handle(), cx);
                });
                (input, scroll)
            })
            .unwrap();
        let draw = |cx: &mut gpui::HeadlessAppContext| {
            cx.update_window(window.into(), |_, window, app| {
                window.refresh();
                let _ = window.draw(app);
            })
            .unwrap();
            cx.run_until_parked();
        };
        for _ in 0..4 {
            draw(&mut cx);
        }
        cx.update_window(window.into(), |_, window, app| {
            input.update(app, |input, cx| {
                assert!(input.wrap.row_counts[0] > 1);
                let cursor = input.text_snapshot().line_range(10).start + 2;
                input.set_selected_range(cursor..cursor, true, window, cx);
            });
        })
        .unwrap();
        for _ in 0..4 {
            draw(&mut cx);
        }
        cx.update(|app| {
            let input = input.read(app);
            let (top, _) = input.cursor_vertical_span(input.cursor_offset()).unwrap();
            scroll.set_offset(point(px(0.0), -(top - px(40.0))));
        });
        for _ in 0..3 {
            draw(&mut cx);
        }
        let baseline = scroll.offset();
        assert!(baseline.y < px(0.0));
        for step in 0..6 {
            cx.update_window(window.into(), |_, window, app| {
                input.update(app, |input, cx| {
                    if step % 2 == 0 {
                        input.replace_text_in_range(None, "x", window, cx);
                    } else {
                        input.backspace(&Backspace, window, cx);
                    }
                    input.set_highlight_provider_with_key(
                        step + 1,
                        HighlightProvider::from_fn(|_| Vec::new()),
                        input.content.len(),
                        cx,
                    );
                });
            })
            .unwrap();
            for frame in 0..4 {
                draw(&mut cx);
                let actual = scroll.offset();
                assert_eq!(actual.x, px(0.0));
                assert!(
                    (actual.y - baseline.y).abs() <= px(1.0),
                    "scale {percent}%, step {step}, frame {frame}: {baseline:?} -> {actual:?}"
                );
            }
        }
    }
}
