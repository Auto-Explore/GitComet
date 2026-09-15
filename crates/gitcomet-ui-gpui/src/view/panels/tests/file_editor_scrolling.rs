use super::*;
use gpui::EntityInputHandler as _;
use std::path::PathBuf;

fn draw_editor_frame(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
}

#[gpui::test]
fn markdown_inline_colors_stay_stable_through_held_key_frames(test_cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = test_cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });
    let workdir = unique_workdir("markdown_inline_typing");
    let file_rel = PathBuf::from("notes.md");
    let text = "Words around `inline_code` and **bold** text.\n\n".repeat(300);
    std::fs::write(workdir.join(&file_rel), &text).unwrap();
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            push_test_state(
                view,
                editor_state(gitcomet_state::model::RepoId(1106), &workdir, &file_rel),
                cx,
            );
            view.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    let input = cx.update(|_window, app| main_pane.read(app).file_editor_input.clone());
    let code_color = cx.update(|_window, app| {
        input.update(app, |input, _| {
            input
                .debug_effective_highlights_for_range(0..100)
                .iter()
                .find(|(range, _)| range.contains(&text.find("inline_code").unwrap()))
                .and_then(|(_, style)| style.color)
                .expect("inline code starts with a syntax color")
        })
    });
    for wrap in [false, true] {
        cx.update(|window, app| {
            main_pane.update(app, |pane, cx| pane.set_diff_word_wrap(wrap, cx));
            input.update(app, |input, cx| {
                input.set_cursor_offset(2, cx);
                window.focus(&input.focus_handle(), cx);
            });
        });
        for step in 0..20 {
            cx.update(|window, app| {
                input.update(app, |input, cx| {
                    input.replace_text_in_range(None, "a", window, cx);
                });
            });
            for frame in 0..3 {
                draw_editor_frame(cx);
                cx.update(|_window, app| {
                    input.update(app, |input, _| {
                        let text = input.text();
                        let code = text.find("inline_code").unwrap();
                        let highlights = input.debug_effective_highlights_for_range(0..code + 11);
                        let color = highlights
                            .iter()
                            .find(|(range, _)| range.contains(&code))
                            .and_then(|(_, style)| style.color);
                        assert_eq!(
                            color,
                            Some(code_color),
                            "wrap {wrap}, edit {step}, frame {frame}"
                        );
                    });
                });
            }
        }
    }
    std::fs::remove_dir_all(workdir).unwrap();
}

#[gpui::test]
fn file_editor_horizontal_scrollbar_works_for_all_text_file_types(
    test_cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    for (case, filename) in ["notes.md", "main.rs", "notes.txt"].into_iter().enumerate() {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) = test_cx.add_window_view(|window, cx| {
            crate::view::GitCometView::new(store, events, None, window, cx)
        });
        let repo_id = gitcomet_state::model::RepoId(1100 + case as u64);
        let workdir = unique_workdir("file_editor_horizontal_scrollbar");
        let file_rel = PathBuf::from(filename);
        std::fs::write(
            workdir.join(&file_rel),
            format!("{}\n", "long line ".repeat(200)).repeat(100),
        )
        .unwrap();
        cx.update(|_window, app| {
            view.update(app, |view, cx| {
                push_test_state(view, editor_state(repo_id, &workdir, &file_rel), cx);
                view.main_pane.update(cx, |pane, cx| {
                    pane.diff_word_wrap = false;
                    pane.ensure_file_editor_loaded(cx);
                });
            });
        });
        cx.run_until_parked();
        for _ in 0..3 {
            draw_editor_frame(cx);
        }
        let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
        let horizontal = cx
            .debug_bounds("file_editor_hscrollbar")
            .expect("horizontal track");
        let vertical = cx
            .debug_bounds("file_editor_scrollbar")
            .expect("vertical track");
        let viewport = cx
            .debug_bounds("file_editor_scroll")
            .expect("editor viewport");
        assert!(
            horizontal.top() >= viewport.bottom() - px(1.0),
            "track must not cover the last row"
        );
        assert!(
            horizontal.right() <= vertical.left() + px(1.0),
            "tracks must not overlap"
        );
        assert!(vertical.bottom() <= horizontal.top() + px(1.0));
        cx.update(|_window, app| {
            let scroll = &main_pane.read(app).file_editor_scroll;
            assert!(
                scroll.max_offset().x > px(0.0),
                "{filename} must overflow horizontally"
            );
            assert!(scroll.max_offset().y > px(0.0));
        });
        let start = point(horizontal.left() + px(8.0), horizontal.center().y);
        let end = point(horizontal.center().x, horizontal.center().y);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        draw_editor_frame(cx);
        cx.update(|_window, app| {
            assert!(
                main_pane.read(app).file_editor_scroll.offset().x < px(0.0),
                "thumb drag must scroll {filename}"
            )
        });
        assert_eq!(
            cx.debug_bounds("file_editor_hscrollbar").unwrap(),
            horizontal,
            "track stays fixed while content moves"
        );

        cx.update(|_window, app| {
            let pane = main_pane.read(app);
            pane.file_editor_scroll
                .set_offset(point(px(0.0), px(-100.0)));
        });
        draw_editor_frame(cx);
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(-80.0), px(0.0))),
            ..Default::default()
        });
        draw_editor_frame(cx);
        cx.update(|_window, app| {
            assert!(main_pane.read(app).file_editor_scroll.offset().x < px(0.0))
        });

        cx.update(|_window, app| {
            main_pane.update(app, |pane, cx| pane.set_diff_word_wrap(true, cx))
        });
        for _ in 0..4 {
            draw_editor_frame(cx);
        }
        assert!(cx.debug_bounds("file_editor_hscrollbar").is_none());
        cx.update(|_window, app| {
            let pane = main_pane.read(app);
            assert_eq!(pane.file_editor_scroll.offset().x, px(0.0));
            assert_eq!(pane.file_editor_scroll.max_offset().x, px(0.0));
            assert!(
                pane.file_editor_input
                    .read(app)
                    .wrap_row_counts()
                    .iter()
                    .any(|rows| *rows > 1)
            );
        });
        // A horizontal-only gesture must not turn into vertical motion in wrap mode.
        let offset = cx.update(|_window, app| main_pane.read(app).file_editor_scroll.offset());
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(-80.0), px(0.0))),
            ..Default::default()
        });
        draw_editor_frame(cx);
        cx.update(|_window, app| {
            assert_eq!(main_pane.read(app).file_editor_scroll.offset(), offset)
        });

        // Returning to unwrapped text restores horizontal scrolling.
        cx.update(|_window, app| {
            main_pane.update(app, |pane, cx| pane.set_diff_word_wrap(false, cx))
        });
        for _ in 0..3 {
            draw_editor_frame(cx);
        }
        assert!(cx.debug_bounds("file_editor_hscrollbar").is_some());
        cx.update(|_window, app| {
            main_pane
                .read(app)
                .file_editor_input
                .clone()
                .update(app, |input, cx| input.set_text("short", cx));
        });
        for _ in 0..3 {
            draw_editor_frame(cx);
        }
        cx.update(|_window, app| {
            assert_eq!(
                main_pane.read(app).file_editor_scroll.max_offset().x,
                px(0.0),
                "fitting text must hide the thumb"
            );
            assert_eq!(main_pane.read(app).file_editor_scroll.offset().x, px(0.0));
        });
        std::fs::remove_dir_all(workdir).unwrap();
    }
}

#[gpui::test]
fn file_editor_wrapped_typing_keeps_the_viewport_stable(test_cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    for filename in ["notes.md", "main.rs", "notes.txt"] {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) = test_cx.add_window_view(|window, cx| {
            crate::view::GitCometView::new(store, events, None, window, cx)
        });
        let workdir = unique_workdir("file_editor_wrapped_typing");
        let file_rel = PathBuf::from(filename);
        let paragraph = format!("{}end\n", "words with room for an edit ".repeat(70));
        std::fs::write(workdir.join(&file_rel), paragraph.repeat(60)).unwrap();
        cx.update(|_window, app| {
            view.update(app, |view, cx| {
                push_test_state(
                    view,
                    editor_state(gitcomet_state::model::RepoId(1105), &workdir, &file_rel),
                    cx,
                );
                view.main_pane.update(cx, |pane, cx| {
                    pane.diff_word_wrap = true;
                    pane.ensure_file_editor_loaded(cx);
                });
            });
        });
        cx.run_until_parked();
        for _ in 0..4 {
            draw_editor_frame(cx);
        }
        let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
        cx.update(|_window, app| {
            main_pane
                .read(app)
                .file_editor_scroll
                .set_offset(point(px(0.0), px(-700.0)))
        });
        for _ in 0..3 {
            draw_editor_frame(cx);
        }
        let viewport = cx.debug_bounds("file_editor_scroll").unwrap();
        let press = point(viewport.left() + px(60.0), viewport.top() + px(100.0));
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(press, MouseButton::Left, Modifiers::default());
        for _ in 0..3 {
            draw_editor_frame(cx);
        }
        let (input, baseline) = cx.update(|_window, app| {
            let pane = main_pane.read(app);
            assert!(
                pane.file_editor_input
                    .read(app)
                    .wrap_row_counts()
                    .iter()
                    .any(|rows| *rows > 1)
            );
            (
                pane.file_editor_input.clone(),
                pane.file_editor_scroll.offset(),
            )
        });
        for step in 0..6 {
            cx.update(|window, app| {
                input.update(app, |input, cx| {
                    if step % 2 == 0 {
                        input.replace_text_in_range(None, "x", window, cx);
                    } else {
                        let cursor = input.cursor_offset();
                        input.replace_utf8_range(cursor - 1..cursor, "", cx);
                    }
                });
            });
            for frame in 0..4 {
                draw_editor_frame(cx);
                cx.update(|_window, app| {
                    let pane = main_pane.read(app);
                    let offset = pane.file_editor_scroll.offset();
                    assert!(
                        (offset.y - baseline.y).abs() <= px(1.0),
                        "{filename}, edit {step}, frame {frame}: {baseline:?} -> {offset:?}"
                    );
                    assert_eq!(offset.x, px(0.0));
                    let gutter_y = pane
                        .file_editor_gutter_scroll
                        .0
                        .borrow()
                        .base_handle
                        .offset()
                        .y;
                    assert!(
                        (gutter_y - offset.y).abs() <= px(1.0),
                        "line numbers must remain aligned"
                    );
                });
            }
        }
        std::fs::remove_dir_all(workdir).unwrap();
    }
}
