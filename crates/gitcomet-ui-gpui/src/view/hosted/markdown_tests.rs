use super::*;
use crate::view::markdown_preview::MarkdownPreviewRowKind;

fn open_markdown<'a>(
    cx: &'a mut gpui::TestAppContext,
    source: &str,
) -> (Entity<MarkdownView>, &'a mut gpui::VisualTestContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    cx.update(|app| {
        app.bind_keys([
            gpui::KeyBinding::new(
                "secondary-a",
                crate::kit::SelectAll,
                Some("MarkdownPreview"),
            ),
            gpui::KeyBinding::new("secondary-c", crate::kit::Copy, Some("MarkdownPreview")),
        ]);
    });
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut view = MarkdownView::new(
            Rc::new(Cell::new(AppTheme::gitcomet_dark())),
            Default::default(),
            crate::view::hosted::environment::DiffPreferences::Fixed(Default::default()),
            cx,
        );
        view.set_source(source, cx);
        view
    });
    redraw(cx);
    (view, cx)
}

fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

fn draw_scheduled_frames(cx: &mut gpui::VisualTestContext) {
    for _ in 0..16 {
        if cx.update(|window, app| window.simulate_next_frame(app)) == 0 {
            return;
        }
        redraw(cx);
    }
    panic!("Markdown layout did not settle");
}

fn row_ix(view: &Entity<MarkdownView>, cx: &mut gpui::VisualTestContext, text: &str) -> usize {
    cx.update(|_, app| {
        view.read(app)
            .document
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .position(|row| row.text.as_ref() == text)
            .unwrap_or_else(|| panic!("row {text:?} was not parsed"))
    })
}

#[track_caller]
fn bounds(cx: &mut gpui::VisualTestContext, prefix: &str, row: usize) -> Bounds<Pixels> {
    cx.debug_bounds(Box::leak(format!("{prefix}_{row}").into_boxed_str()))
        .expect("row is drawn")
}

fn click_count(
    cx: &mut gpui::VisualTestContext,
    position: Point<Pixels>,
    count: usize,
    modifiers: gpui::Modifiers,
) {
    cx.simulate_mouse_move(position, None, modifiers);
    cx.simulate_event(gpui::MouseDownEvent {
        position,
        modifiers,
        button: MouseButton::Left,
        click_count: count,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position,
        modifiers,
        button: MouseButton::Left,
        click_count: count,
    });
    redraw(cx);
}

fn clipboard(view: &Entity<MarkdownView>, cx: &mut gpui::VisualTestContext) -> Option<String> {
    cx.update(|_, app| view.update(app, |_, cx| crate::clipboard::read_text(cx)))
}

fn activate_key(cx: &mut gpui::VisualTestContext, key: &str) {
    cx.simulate_keystrokes(key);
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse(key).unwrap(),
            }),
            app,
        );
    });
    redraw(cx);
}

fn text_point(
    view: &Entity<MarkdownView>,
    cx: &mut gpui::VisualTestContext,
    row: usize,
    offset: usize,
) -> Point<Pixels> {
    cx.update(|_, app| {
        let view = view.read(app);
        let hitbox = &view.hitboxes[&(row, DiffTextRegion::Inline)];
        let wrapped = hitbox.wrapped.as_ref().expect("text has shaped geometry");
        let mut position = wrapped
            .layout
            .position_for_index(offset)
            .expect("text offset is drawn");
        position.y += wrapped.layout.line_height() / 2.0;
        position.x += px(1.0);
        position
    })
}

#[gpui::test]
fn hosted_markdown_copy_excludes_closed_middle_and_final_details(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_markdown(
        cx,
        "Before\n\n<details><summary>Middle</summary>\n\nHidden middle\n\n</details>\n\nAfter\n\n<details><summary>Final</summary>\n\nHidden final\n\n</details>",
    );
    let first = row_ix(&view, cx, "Before");
    let first = text_point(&view, cx, first, 1);
    click_count(cx, first, 1, Default::default());
    cx.simulate_keystrokes("secondary-a secondary-c");
    let copied = clipboard(&view, cx).expect("selected visible text is copied");
    assert!(copied.contains("Before"));
    assert!(copied.contains("Middle"));
    assert!(copied.contains("After"));
    assert!(copied.ends_with("Final"), "{copied:?}");
    assert!(!copied.contains("Hidden"));
    cx.update(|_, app| {
        let view = view.read(app);
        assert_eq!(
            view.head.unwrap().source_visible_ix,
            view.document
                .as_ref()
                .unwrap()
                .rows
                .iter()
                .position(|row| row.text.as_ref() == "Final")
                .unwrap()
        );
    });
}

#[gpui::test]
fn hosted_markdown_selection_survives_same_source_and_yields_to_another_owner(
    cx: &mut gpui::TestAppContext,
) {
    let source = "Selectable words";
    let (view, cx) = open_markdown(cx, source);
    let word = text_point(&view, cx, 0, 2);
    click_count(cx, word, 2, Default::default());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            let selected = (view.anchor, view.head);
            assert!(view.has_selection());
            view.set_source(source, cx);
            assert_eq!((view.anchor, view.head), selected);
        });
    });
    cx.update(|window, app| {
        let mut other_owner = crate::kit::text_selection_owner::SelectionOwnerToken::default();
        other_owner.adopt(window, app);
    });
    redraw(cx);
    cx.update(|_, app| assert!(!view.read(app).has_selection()));
}

#[gpui::test]
fn hosted_markdown_unrelated_disclosure_keeps_visible_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_markdown(
        cx,
        "Before\n\n<details><summary>Middle</summary>\n\nHidden middle\n\n</details>\n\nAfter\n\n<details><summary>Final</summary>\n\nFinal body\n\n</details>",
    );
    let after = row_ix(&view, cx, "After");
    let final_summary = row_ix(&view, cx, "Final");
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.select_all(window, cx);
            view.head = Some(DiffTextPos {
                source_visible_ix: after,
                region: DiffTextRegion::Inline,
                offset: "After".len(),
            });
            let selected = (view.anchor, view.head);
            let id = view.document.as_ref().unwrap().rows[final_summary]
                .details
                .as_ref()
                .unwrap()
                .id;
            view.toggle_disclosure(id, cx);
            assert_eq!((view.anchor, view.head), selected);
            view.toggle_disclosure(id, cx);
            assert_eq!((view.anchor, view.head), selected);
            view.copy(cx);
        })
    });
    let copied = clipboard(&view, cx).unwrap();
    assert!(copied.starts_with("Before"));
    assert!(copied.ends_with("After"));
    assert!(!copied.contains("Hidden"));
}

#[gpui::test]
fn hosted_markdown_disclosure_whole_heading_toggles(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_markdown(
        cx,
        "<details><summary>**Selectable** `title`</summary>\n\nBody\n\n</details>",
    );
    let summary = row_ix(&view, cx, "Selectable title");
    let body = row_ix(&view, cx, "Body");
    let title = text_point(&view, cx, summary, 3);
    click_count(cx, title, 1, Default::default());
    bounds(cx, "markdown_preview_text_box", body);
    let header = bounds(cx, "markdown_preview_row_box", summary);
    let title_bounds = bounds(cx, "markdown_preview_text_box", summary);
    let title_offset = AppTheme::gitcomet_dark().markdown_px(20.0, 100);
    assert!((title_bounds.left() - header.left() - title_offset).abs() < px(1.0));
    let background = point(header.right() - px(4.0), header.center().y);
    click_count(cx, background, 1, Default::default());
    cx.update(|_, app| {
        let view = view.read(app);
        assert!(
            !view
                .disclosures
                .is_visible(&view.document.as_ref().unwrap().rows[body])
        );
    });
    let icon = point(header.left() + px(6.0), header.center().y);
    click_count(cx, icon, 1, Default::default());
    bounds(cx, "markdown_preview_text_box", body);
    let control = bounds(cx, "markdown_disclosure", summary);
    assert_eq!(control, bounds(cx, "markdown_preview_row_box", summary));
    activate_key(cx, "space");
    cx.update(|_, app| {
        let view = view.read(app);
        assert!(
            !view
                .disclosures
                .is_visible(&view.document.as_ref().unwrap().rows[body])
        );
    });
    activate_key(cx, "enter");
    bounds(cx, "markdown_preview_text_box", body);
}

#[gpui::test]
fn hosted_markdown_disclosure_title_selection_and_cancelled_click_keep_state(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_markdown(
        cx,
        "<details open><summary>Select these words</summary>\n\nBody\n\n</details>",
    );
    let summary = row_ix(&view, cx, "Select these words");
    let body = row_ix(&view, cx, "Body");
    let from = text_point(&view, cx, summary, 0);
    let to = text_point(&view, cx, summary, "Select these".len());
    cx.simulate_mouse_down(from, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Default::default());
    cx.simulate_mouse_up(to, MouseButton::Left, Default::default());
    redraw(cx);
    bounds(cx, "markdown_preview_text_box", body);
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(clipboard(&view, cx), Some("Select these".into()));
    let word = text_point(&view, cx, summary, 8);
    click_count(cx, word, 2, Default::default());
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(clipboard(&view, cx), Some("these".into()));
    click_count(cx, word, 3, Default::default());
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(clipboard(&view, cx), Some("Select these words".into()));
    bounds(cx, "markdown_preview_text_box", body);
    let outside = text_point(&view, cx, body, 1);
    cx.simulate_mouse_down(word, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(outside, Some(MouseButton::Left), Default::default());
    cx.simulate_mouse_up(outside, MouseButton::Left, Default::default());
    redraw(cx);
    bounds(cx, "markdown_preview_text_box", body);
    cx.simulate_mouse_down(from, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Default::default());
    cx.simulate_mouse_move(from, Some(MouseButton::Left), Default::default());
    cx.simulate_mouse_up(from, MouseButton::Left, Default::default());
    redraw(cx);
    bounds(cx, "markdown_preview_text_box", body);
    let header = bounds(cx, "markdown_disclosure", summary);
    let blank = point(header.right() - px(80.0), header.center().y);
    let moved = point(blank.x - px(24.0), blank.y);
    cx.simulate_mouse_down(blank, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(moved, Some(MouseButton::Left), Default::default());
    cx.simulate_mouse_move(blank, Some(MouseButton::Left), Default::default());
    cx.simulate_mouse_up(blank, MouseButton::Left, Default::default());
    redraw(cx);
    bounds(cx, "markdown_preview_text_box", body);
}

#[gpui::test]
fn hosted_markdown_disclosure_summary_links_take_priority(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_markdown(
        cx,
        "<details open><summary>[Project](https://example.test/docs) [Local](./private.md) title</summary>\n\nBody\n\n</details>",
    );
    let menus = Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|_, app| {
        view.update(app, |view, _| {
            let menus = menus.clone();
            view.commands.dispatch = Some(Rc::new(move |command, _| {
                if let DiffCommand::OpenPopover { request, .. } = command {
                    menus.borrow_mut().push(request.kind);
                }
            }));
        });
    });
    let summary = row_ix(&view, cx, "Project Local title");
    let body = row_ix(&view, cx, "Body");
    let project = text_point(&view, cx, summary, 2);
    click_count(cx, project, 1, Default::default());
    assert!(
        matches!(menus.borrow().as_slice(), [PopoverKind::WebLinkMenu { url, .. }] if url.as_ref() == "https://example.test/docs")
    );
    bounds(cx, "markdown_preview_text_box", body);
    crate::view::panes::diff::take_opened_web_links_for_tests();
    click_count(cx, project, 1, gpui::Modifiers::secondary_key());
    assert_eq!(
        crate::view::panes::diff::take_opened_web_links_for_tests(),
        ["https://example.test/docs"]
    );
    bounds(cx, "markdown_preview_text_box", body);
    let local = text_point(&view, cx, summary, 10);
    click_count(cx, local, 1, Default::default());
    assert_eq!(menus.borrow().len(), 1);
    bounds(cx, "markdown_preview_text_box", body);
}

#[gpui::test]
fn hosted_markdown_compact_list_geometry_scales_with_the_preview(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_markdown(
        cx,
        "Paragraph\n\n- Outer\n  - Inner\n    - Deep\n\n9. Nine\n10. Ten\n\n- [ ] Task\n\n1. Install:\n\n   ```sh\n   command\n   ```\n",
    );
    for width in [400.0, 900.0] {
        cx.simulate_resize(gpui::size(px(width), px(1200.0)));
        for percent in [100, 150] {
            cx.update(|_, app| {
                crate::ui_scale::set_default(app, percent);
                view.update(app, |_, cx| cx.notify());
            });
            redraw(cx);
            draw_scheduled_frames(cx);
            let paragraph = row_ix(&view, cx, "Paragraph");
            let outer = row_ix(&view, cx, "Outer");
            let inner = row_ix(&view, cx, "Inner");
            let deep = row_ix(&view, cx, "Deep");
            let left = |cx: &mut gpui::VisualTestContext, row| {
                bounds(cx, "markdown_preview_text_box", row).left()
            };
            let step = AppTheme::gitcomet_dark().markdown_px(16.0, percent);
            assert!((left(cx, inner) - left(cx, outer) - step).abs() < px(1.0));
            assert!((left(cx, deep) - left(cx, inner) - step).abs() < px(1.0));
            let first_indent = AppTheme::gitcomet_dark().markdown_px(44.0, percent);
            assert!((left(cx, outer) - left(cx, paragraph) - first_indent).abs() < px(1.0));
            let install = row_ix(&view, cx, "Install:");
            let code = row_ix(&view, cx, "command");
            let code = bounds(cx, "markdown_preview_code_shell", code);
            assert!((code.left() - left(cx, install)).abs() < px(1.0));
            let nine = row_ix(&view, cx, "Nine");
            let ten = row_ix(&view, cx, "Ten");
            let task = row_ix(&view, cx, "Task");
            assert!((left(cx, nine) - left(cx, ten)).abs() < px(1.0));
            assert!((left(cx, task) - left(cx, outer)).abs() < px(1.0));
        }
    }
}

#[gpui::test]
fn hosted_markdown_nested_disclosures_toggle_and_reset_only_for_changed_source(
    cx: &mut gpui::TestAppContext,
) {
    let source = "<details><summary>Outer</summary>\n\n<details open><summary>Inner</summary>\n\nNested body\n\n</details>\n\n</details>";
    let (view, cx) = open_markdown(cx, source);
    let outer = row_ix(&view, cx, "Outer");
    let inner = row_ix(&view, cx, "Inner");
    let body = row_ix(&view, cx, "Nested body");
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown_preview_text_box_{body}").into_boxed_str()
        ))
        .is_none()
    );
    let control = bounds(cx, "markdown_disclosure", outer);
    click_count(cx, control.center(), 1, Default::default());
    bounds(cx, "markdown_preview_text_box", body);
    let inner_control = bounds(cx, "markdown_disclosure", inner);
    click_count(cx, inner_control.center(), 1, Default::default());
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown_preview_text_box_{body}").into_boxed_str()
        ))
        .is_none()
    );
    cx.update(|_, app| view.update(app, |view, cx| view.set_source(source, cx)));
    redraw(cx);
    bounds(cx, "markdown_disclosure", inner);
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown_preview_text_box_{body}").into_boxed_str()
        ))
        .is_none()
    );
    cx.update(|window, app| {
        window.focus(&view.read(app).focus.clone(), app);
        window.focus_next(app);
        window.focus_next(app);
    });
    redraw(cx);
    activate_key(cx, "space");
    bounds(cx, "markdown_preview_text_box", body);
    activate_key(cx, "enter");
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown_preview_text_box_{body}").into_boxed_str()
        ))
        .is_none()
    );
    activate_key(cx, "enter");
    bounds(cx, "markdown_preview_text_box", body);
    let body_point = text_point(&view, cx, body, 1);
    click_count(cx, body_point, 3, Default::default());
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(clipboard(&view, cx), Some("Nested body".into()));
    let outer_control = bounds(cx, "markdown_disclosure", outer);
    click_count(cx, outer_control.center(), 1, Default::default());
    cx.update(|_, app| assert!(!view.read(app).has_selection()));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_source(&format!("{source}\n\nNew"), cx)
        })
    });
    redraw(cx);
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown_disclosure_{inner}").into_boxed_str()
        ))
        .is_none()
    );
}

#[gpui::test]
fn hosted_markdown_disclosure_in_quote_keeps_nested_list_geometry(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_markdown(
        cx,
        "<blockquote>\n<details><summary>Quoted details</summary>\n<ul><li>Nested list text</li></ul>\n</details>\n<p>Visible quote</p>\n</blockquote>",
    );
    let summary = row_ix(&view, cx, "Quoted details");
    let body = row_ix(&view, cx, "Nested list text");
    let visible = row_ix(&view, cx, "Visible quote");
    bounds(cx, "markdown_preview_text_box", visible);
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown_preview_marker_{body}").into_boxed_str()
        ))
        .is_none()
    );
    let toggle = bounds(cx, "markdown_disclosure", summary);
    click_count(cx, toggle.center(), 1, Default::default());
    bounds(cx, "markdown_preview_marker", body);
    let body_point = text_point(&view, cx, body, 1);
    click_count(cx, body_point, 3, Default::default());
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(clipboard(&view, cx), Some("Nested list text".into()));
}

#[gpui::test]
fn hosted_markdown_anchor_expands_hidden_heading(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_markdown(
        cx,
        "[Jump](#hidden-heading)\n\n<details><summary>More</summary>\n\n# Hidden heading\n\n</details>",
    );
    let link = row_ix(&view, cx, "Jump");
    let heading = row_ix(&view, cx, "Hidden heading");
    let link_point = text_point(&view, cx, link, 1);
    click_count(cx, link_point, 1, Default::default());
    bounds(cx, "markdown_preview_text_box", heading);
    cx.update(|_, app| {
        let view = view.read(app);
        let row = &view.document.as_ref().unwrap().rows[heading];
        assert!(view.disclosures.is_visible(row));
    });
}

#[gpui::test]
fn hosted_markdown_web_links_use_menu_modifier_and_selection_gestures(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_markdown(
        cx,
        "[Select **these** words](https://example.test/docs) then plain text\n\n[Local](./private.md)\n",
    );
    let menus = Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|_, app| {
        view.update(app, |view, _| {
            let menus = menus.clone();
            view.commands.dispatch = Some(Rc::new(move |command, _| {
                if let DiffCommand::OpenPopover { request, .. } = command {
                    menus.borrow_mut().push(request.kind);
                }
            }));
        })
    });
    let link = row_ix(&view, cx, "Select these words then plain text");
    let link_point = text_point(&view, cx, link, 8);
    cx.simulate_mouse_move(link_point, None, Default::default());
    cx.update(|_, app| {
        assert_eq!(
            view.read(app).hovered_link.as_ref().unwrap().byte_range,
            0.."Select these words".len()
        );
    });
    click_count(cx, link_point, 1, Default::default());
    assert!(
        matches!(menus.borrow().as_slice(), [PopoverKind::WebLinkMenu { url, .. }] if url.as_ref() == "https://example.test/docs")
    );
    crate::view::panes::diff::take_opened_web_links_for_tests();
    click_count(cx, link_point, 1, gpui::Modifiers::secondary_key());
    assert_eq!(
        crate::view::panes::diff::take_opened_web_links_for_tests(),
        ["https://example.test/docs"]
    );
    click_count(cx, link_point, 2, Default::default());
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(clipboard(&view, cx), Some("these".into()));
    assert_eq!(menus.borrow().len(), 1);
    let local = row_ix(&view, cx, "Local");
    let local_point = text_point(&view, cx, local, 1);
    click_count(cx, local_point, 1, gpui::Modifiers::secondary_key());
    assert!(crate::view::panes::diff::take_opened_web_links_for_tests().is_empty());
    assert_eq!(menus.borrow().len(), 1);
    let from = text_point(&view, cx, link, 0);
    let to = text_point(&view, cx, link, "Select these words".len());
    cx.simulate_mouse_down(from, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Default::default());
    cx.simulate_mouse_up(to, MouseButton::Left, Default::default());
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(clipboard(&view, cx), Some("Select these words".into()));
    assert_eq!(menus.borrow().len(), 1);
}

#[gpui::test]
fn hosted_markdown_image_menu_approves_only_its_invoking_preview(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    cx.update(|app| {
        let registry = gitcomet_extension_api::Registry::build(vec![Box::new(
            gitcomet_extension_example::review::ReviewExtension,
        )])
        .unwrap();
        crate::view::extension_host::install(registry, app);
    });
    let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let image_url = "https://images.example.invalid/hosted.svg";
    let source = format!("[![badge]({image_url})](https://example.test/project) trailing text");
    let (host, first, second) = cx.update(|_, app| {
        root.update(app, |root, cx| {
            root.set_remote_markdown_image_policy(RemoteMarkdownImagePolicy::AskBeforeLoading, cx);
        });
        let host = root.read(app).extension_window.as_ref().unwrap().host();
        let first = host.create_markdown_view(app).unwrap();
        let second = host.create_markdown_view(app).unwrap();
        first.set_source(source.clone(), app);
        second.set_source(source, app);
        let content = first.view();
        host.open_dialog("First preview", move |_, _| content, app)
            .unwrap();
        (host, first, second)
    });
    let first = first.view().downcast::<MarkdownView>().unwrap();
    let second_view = second.view().downcast::<MarkdownView>().unwrap();
    redraw(cx);
    let source_byte = cx.update(|_, app| {
        first
            .read(app)
            .document
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .flat_map(|row| row.inline_images.iter())
            .next()
            .unwrap()
            .source_byte
    });
    let load_selector =
        Box::leak(format!("markdown_preview_inline_image_load_{source_byte}").into_boxed_str());
    let load_image = cx
        .debug_bounds(load_selector)
        .expect("Ask mode offers image loading in the hosted preview");
    click_count(cx, load_image.center(), 1, Default::default());
    let menu_load = cx
        .debug_bounds("context_menu_load_image")
        .expect("the actual hosted link menu offers Load image");
    click_count(cx, menu_load.center(), 1, Default::default());
    cx.update(|_, app| {
        assert_eq!(first.read(app).approved_images.len(), 1);
        assert!(first.read(app).approved_images.contains(image_url));
        assert!(second_view.read(app).approved_images.is_empty());
        assert!(
            root.read(app)
                .diff_renderer
                .read(app)
                .remote_markdown_images
                .approved_urls
                .is_empty(),
            "the menu must not approve the default diff renderer"
        );
    });

    cx.update(|_, app| {
        let content = second.view();
        host.open_dialog("Second preview", move |_, _| content, app)
            .unwrap();
    });
    redraw(cx);
    let second_load = cx
        .debug_bounds(load_selector)
        .expect("the other hosted preview still requires approval");
    click_count(cx, second_load.center(), 1, Default::default());
    assert!(cx.debug_bounds("context_menu_load_image").is_some());
    cx.update(|window, app| {
        root.update(app, |root, cx| {
            root.set_remote_markdown_image_policy(RemoteMarkdownImagePolicy::NeverLoad, cx);
        });
        let popover = root.read(app).popover_host.clone();
        popover.update(app, |popover, cx| {
            popover.context_menu_activate_action(
                crate::view::panels::ContextMenuAction::LoadRemoteMarkdownImage {
                    url: image_url.into(),
                },
                window,
                cx,
            );
        });
        assert!(
            second_view.read(app).approved_images.is_empty(),
            "a stale Ask action must not grant approval after switching to Never"
        );
    });
    cx.update(|_, app| {
        let content = second.view();
        host.open_dialog("Second preview", move |_, _| content, app)
            .unwrap();
    });
    redraw(cx);
    assert!(cx.debug_bounds(load_selector).is_none());
    let blocked_selector = Box::leak(
        format!("markdown_preview_inline_image_load_{source_byte}_blocked_box").into_boxed_str(),
    );
    let blocked = cx
        .debug_bounds(blocked_selector)
        .expect("Never mode keeps the hosted image blocked");
    click_count(cx, blocked.center(), 1, Default::default());
    assert!(cx.debug_bounds("context_menu_load_image").is_none());
    cx.update(|_, app| {
        assert!(matches!(
            root.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests(),
            Some(PopoverKind::WebLinkMenu {
                load_remote_image_url: None,
                ..
            })
        ));
        assert!(second_view.read(app).approved_images.is_empty());
        assert!(
            root.read(app)
                .diff_renderer
                .read(app)
                .remote_markdown_images
                .approved_urls
                .is_empty()
        );
    });
}

#[gpui::test]
fn hosted_markdown_table_and_code_share_selection_geometry(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_markdown(
        cx,
        "| First | Second |\n| --- | --- |\n| Left | Right |\n\n```text\ncode\ttext\n```\n",
    );
    let table_row = cx.update(|_, app| {
        view.read(app)
            .document
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .position(|row| {
                matches!(
                    row.kind,
                    MarkdownPreviewRowKind::TableRow { is_header: false }
                )
            })
            .unwrap()
    });
    let cell = cx
        .debug_bounds(Box::leak(
            format!("markdown_preview_cell_text_box_{table_row}_0").into_boxed_str(),
        ))
        .unwrap();
    click_count(cx, cell.center(), 1, Default::default());
    cx.update(|window, app| view.update(app, |view, cx| view.select_all(window, cx)));
    cx.simulate_keystrokes("secondary-c");
    let copied = clipboard(&view, cx).unwrap();
    assert!(copied.contains("First"));
    assert!(copied.contains("Second"));
    assert!(copied.contains("Left"));
    assert!(copied.contains("Right"));
    assert!(copied.contains("code\ttext"));
}

struct ScrollHost {
    markdown: gitcomet_extension_api::MarkdownView,
    scroll: ScrollHandle,
}

impl Render for ScrollHost {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .id("markdown_scroll_host")
            .w(px(420.0))
            .h(px(240.0))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(self.markdown.view())
    }
}

fn open_scrolled_markdown<'a>(
    cx: &'a mut gpui::TestAppContext,
    source: &str,
) -> (Entity<ScrollHost>, &'a mut gpui::VisualTestContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (host, cx) = cx.add_window_view(|_, cx| {
        let entity = cx.new(|cx| {
            let mut view = MarkdownView::new(
                Rc::new(Cell::new(AppTheme::gitcomet_dark())),
                Default::default(),
                crate::view::hosted::environment::DiffPreferences::Fixed(Default::default()),
                cx,
            );
            view.set_source(source, cx);
            view
        });
        let markdown =
            gitcomet_extension_api::MarkdownView::new(Rc::new(HostedMarkdownView { entity }));
        let scroll = ScrollHandle::new();
        markdown.set_scroll_handle(Some(scroll.clone()), cx);
        ScrollHost { markdown, scroll }
    });
    redraw(cx);
    (host, cx)
}

#[gpui::test]
fn hosted_markdown_outer_scroll_draws_the_expanded_document_end(cx: &mut gpui::TestAppContext) {
    let source =
        include_str!("../../../../gitcomet-markdown/src/fixtures/dependabot_release_notes.md")
            .replace("<details>", "<details open>");
    let (host, cx) = open_scrolled_markdown(cx, &source);
    let (view, scroll) = cx.update(|_, app| {
        let host = host.read(app);
        (
            host.markdown.view().downcast::<MarkdownView>().unwrap(),
            host.scroll.clone(),
        )
    });
    draw_scheduled_frames(cx);
    scroll.scroll_to_bottom();
    cx.update(|_, app| host.update(app, |_, cx| cx.notify()));
    redraw(cx);
    draw_scheduled_frames(cx);
    cx.update(|_, app| {
        let view = view.read(app);
        let last = view
            .document
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .rposition(|row| !row.text.is_empty())
            .unwrap();
        let last = view
            .hitboxes
            .get(&(last, DiffTextRegion::Inline))
            .expect("document end is drawn after the outer container scrolls");
        assert!(last.bounds.intersects(&scroll.bounds()));
    });
    scroll.set_offset(point(px(0.0), -scroll.max_offset().y / 2.0));
    cx.update(|_, app| host.update(app, |_, cx| cx.notify()));
    redraw(cx);
    draw_scheduled_frames(cx);
    cx.update(|_, app| {
        assert!(
            view.read(app)
                .hitboxes
                .values()
                .any(|hitbox| hitbox.bounds.intersects(&scroll.bounds())),
            "programmatic outer scrolling draws text at the new viewport"
        );
    });
}

#[gpui::test]
fn hosted_markdown_drag_scrolls_the_associated_container(cx: &mut gpui::TestAppContext) {
    let source = (0..80)
        .map(|ix| format!("Paragraph {ix}: selectable text across a long hosted document."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (host, cx) = open_scrolled_markdown(cx, &source);
    let (view, scroll, markdown) = cx.update(|_, app| {
        let host = host.read(app);
        (
            host.markdown.view().downcast::<MarkdownView>().unwrap(),
            host.scroll.clone(),
            host.markdown.clone(),
        )
    });
    let from = text_point(&view, cx, 0, 0);
    let below = point(from.x, scroll.bounds().bottom() + px(40.0));
    cx.simulate_mouse_down(from, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(below, Some(MouseButton::Left), Default::default());
    cx.update(|_, app| view.update(app, |view, _| assert!(view.tick_autoscroll())));
    assert!(scroll.offset().y < px(0.0));
    redraw(cx);
    cx.update(|_, app| markdown.set_scroll_handle(None, app));
    let detached_offset = scroll.offset();
    cx.update(|_, app| {
        view.update(app, |view, _| {
            view.tick_autoscroll();
        })
    });
    assert_eq!(scroll.offset(), detached_offset);
    cx.simulate_mouse_up(below, MouseButton::Left, Default::default());
    cx.update(|_, app| assert!(!view.read(app).selecting));
}

#[gpui::test]
fn hosted_markdown_select_all_copies_rendered_text(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_markdown(cx, "# Title\n\nSome **selected** text.\n");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("heading text is rendered");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.simulate_keystrokes("secondary-a secondary-c");
    assert_eq!(
        clipboard(&view, cx),
        Some("Title\n\nSome selected text.".to_string())
    );
}
