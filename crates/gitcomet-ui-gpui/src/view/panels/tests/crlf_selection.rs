//! Selection state alone does not catch a CR moving the end caret to x=0.
//! These tests use the real shaper and inspect the quads actually painted.
use super::*;

pub(super) fn copied_selection(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> String {
    cx.update(|_window, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.copy_selected_diff_text_to_clipboard(cx);
            });
    });
    cx.read_from_clipboard()
        .and_then(|item| item.text())
        .expect("selected text on clipboard")
}

pub(super) fn assert_selection_is_painted(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    region: DiffTextRegion,
) {
    crate::test_support::refresh_and_draw(cx);
    cx.update(|window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        let color = pane.diff_text_selection_color();
        let scale = window.scale_factor();
        let quads = window.painted_quads();
        let mut selected_rows = 0;
        for ((visible_ix, row_region), hitbox) in &pane.diff_text_hitboxes {
            if *row_region != region || hitbox.painted_text.is_empty() {
                continue;
            }
            let Some(range) = pane.diff_text_local_selection_range(*visible_ix, region) else {
                continue;
            };
            if range.is_empty() {
                continue;
            }
            selected_rows += 1;
            let bounds = hitbox.bounds;
            assert!(
                quads.iter().any(|quad| {
                    quad.background == color.into()
                        && quad.bounds.size.width.0 > 0.0
                        && (quad.bounds.origin.y.0 - f32::from(bounds.top()) * scale).abs() < 1.0
                        && quad.bounds.origin.x.0 >= f32::from(bounds.left()) * scale - 1.0
                        && quad.bounds.origin.x.0 < f32::from(bounds.right()) * scale
                }),
                "row {visible_ix} {region:?} must paint its selection: {:?}, {range:?}",
                hitbox.painted_text
            );
        }
        assert!(
            selected_rows >= 3,
            "must exercise multiple selected nonempty rows"
        );
    });
}

pub(super) fn drag_between_alpha_and_gamma(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    region: DiffTextRegion,
    reverse: bool,
) -> String {
    crate::test_support::refresh_and_draw(cx);
    wait_for_main_pane_condition(
        cx,
        view,
        "CRLF selection row hitboxes",
        |pane| {
            ["alpha", "gamma"].into_iter().all(|prefix| {
                pane.diff_text_hitboxes
                    .iter()
                    .any(|((_, r), hitbox)| *r == region && hitbox.painted_text.starts_with(prefix))
            })
        },
        |pane| {
            format!(
                "mode={:?} wrap={} collapsed={:?} source={:?} rows={:?}",
                pane.diff_view,
                pane.diff_word_wrap,
                pane.collapsed_diff_visible_rows,
                (0..pane.file_diff_inline_row_len())
                    .map(|ix| pane
                        .file_diff_inline_render_data(ix)
                        .map(|row| row.text.as_ref().to_owned()))
                    .collect::<Vec<_>>(),
                pane.diff_text_hitboxes
                    .iter()
                    .map(|(key, hit)| (key, &hit.painted_text))
                    .collect::<Vec<_>>()
            )
        },
    );
    let mut row_for = |prefix: &str| {
        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            pane.diff_text_hitboxes
                .iter()
                .find_map(|((ix, r), hitbox)| {
                    (*r == region && hitbox.painted_text.starts_with(prefix)).then_some(*ix)
                })
                .unwrap_or_else(|| panic!("missing {prefix} row in {region:?}"))
        })
    };
    let first = row_for("alpha");
    let last = row_for("gamma");
    let from = wait_for_diff_text_click_position_for_offset_range(
        cx,
        view,
        first,
        region,
        1..2,
        "alpha caret",
    );
    let to = wait_for_diff_text_click_position_for_offset_range(
        cx,
        view,
        last,
        region,
        3..4,
        "gamma caret",
    );
    let (from, to) = if reverse { (to, from) } else { (from, to) };
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    assert_selection_is_painted(cx, view, region);
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        let (start, end) = crate::view::text_geometry::normalized_selection(
            pane.diff_text_anchor,
            pane.diff_text_head,
        )
        .expect("drag selection");
        assert_eq!(start.offset, 1);
        assert_eq!(end.offset, 3);
        assert!(start.source_visible_ix < end.source_visible_ix);
    });
    copied_selection(cx, view)
}

#[gpui::test]
fn crlf_preview_drag_paints_every_row_and_preserves_copy(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let dir = tempfile::tempdir().unwrap();
    let path = std::path::PathBuf::from("selection.razor");
    let repo_id = gitcomet_state::model::RepoId(9901);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, dir.path());
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        })
    });
    let long = format!("alpha\r\nbeta {}\r\n\r\ngamma\r\n", "word ".repeat(30));
    for document in [
        "alpha\nbeta\té🙂\n\ngamma",
        "alpha\r\nbeta\té🙂\r\n\r\ngamma\r\n",
        "alpha\r\nbeta\té🙂\n\r\ngamma",
        "alpha\r\nbeta a\rb\r\n\r\ngamma\r\n",
        "alpha\nbeta\rold\rMac\ngamma",
        long.as_str(),
    ] {
        std::fs::write(dir.path().join(&path), document).unwrap();
        let lines = Arc::new(document.split_terminator('\n').map(str::to_owned).collect());
        cx.update(|_window, app| {
            view.read(app)
                .diff_renderer
                .clone()
                .update(app, |pane, cx| {
                    set_ready_worktree_preview(
                        pane,
                        dir.path().join(&path),
                        lines,
                        document.len(),
                        cx,
                    );
                })
        });
        // The tab after "beta" advances to column eight, and copy keeps the
        // existing expanded-tab contract while retaining each source CR.
        let expected = document[1..document.find("gamma").unwrap() + 3].replace('\t', "    ");
        for wrap in [false, true] {
            cx.update(|_window, app| {
                view.read(app)
                    .diff_renderer
                    .clone()
                    .update(app, |pane, cx| {
                        pane.set_diff_word_wrap(wrap, cx);
                        pane.set_diff_reveal_whitespace_chars(false, cx);
                    })
            });
            for reverse in [false, true] {
                assert_eq!(
                    drag_between_alpha_and_gamma(cx, &view, DiffTextRegion::Inline, reverse),
                    expected
                );
                let selection = cx.update(|_window, app| {
                    let pane = view.read(app).diff_renderer.read(app);
                    (pane.diff_text_anchor, pane.diff_text_head)
                });
                for reveal in [true, false] {
                    cx.update(|_window, app| {
                        view.read(app)
                            .diff_renderer
                            .clone()
                            .update(app, |pane, cx| {
                                pane.set_diff_reveal_whitespace_chars(reveal, cx)
                            })
                    });
                    assert_selection_is_painted(cx, &view, DiffTextRegion::Inline);
                    cx.update(|_window, app| {
                        let pane = view.read(app).diff_renderer.read(app);
                        assert_eq!((pane.diff_text_anchor, pane.diff_text_head), selection);
                        assert_eq!(pane.worktree_preview_text.as_ref(), document);
                    });
                    assert_eq!(copied_selection(cx, &view), expected);
                }
            }
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join(&path)).unwrap(),
            document
        );
    }
}
