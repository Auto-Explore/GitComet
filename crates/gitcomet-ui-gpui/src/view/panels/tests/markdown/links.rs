//! Links and anchors: menus, Ctrl+click, hover, local and anchor resolution.

use super::*;

#[gpui::test]
fn clicking_a_badge_opens_its_menu_without_arming_a_selection(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The row under a picture also listens for a left press, so without the
    // picture stopping propagation the click opens the menu *and* starts a
    // drag-selection behind it.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    // Two badges, so they stay inline instead of one alone becoming a block.
    let source = "[![one](badge.svg)](https://example.com/badge)\n[![two](badge.svg)](https://example.com/other)\n";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(85),
        "markdown_badge_click",
        source,
    );
    std::fs::write(
        fixture.workdir.join("docs/badge.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"80\" height=\"20\"><rect width=\"80\" height=\"20\"/></svg>",
    )
    .expect("write the badge the link points at");
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let source_byte = *fixture
        .picture_offsets()
        .first()
        .expect("the fixture carries a picture");
    let badge = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_{source_byte}"
        )))
        .expect("the badge is drawn");

    simulate_counted_click(cx, badge.center(), 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let popover = this.popover_host.read(cx).popover_kind_for_tests();
            assert!(
                matches!(
                    popover,
                    Some(PopoverKind::WebLinkMenu { ref url, .. })
                        if url.as_ref() == "https://example.com/badge"
                ),
                "clicking a badge opens its link menu, got {popover:?}"
            );
            assert!(
                !this.main_pane.read(cx).diff_text_selecting,
                "and the row underneath must not have started selecting text"
            );
        });
    });

    fixture.cleanup();
}

#[gpui::test]
fn ctrl_clicking_a_linked_badge_opens_the_browser(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let source = "[![one](badge.svg)](https://example.com/badge)\n[![two](badge.svg)](https://example.com/other)\n";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8832),
        "markdown_ctrl_click_badge",
        source,
    );
    std::fs::write(
        fixture.workdir.join("docs/badge.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"80\" height=\"20\"><rect width=\"80\" height=\"20\"/></svg>",
    )
    .expect("write the badge the link points at");
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
    let source_byte = *fixture
        .picture_offsets()
        .first()
        .expect("the fixture carries a picture");
    let badge = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_{source_byte}"
        )))
        .expect("the badge is drawn");
    crate::view::panes::main::take_opened_web_links_for_tests();

    simulate_modified_click(cx, badge.center(), 1, Modifiers::secondary_key());
    cx.run_until_parked();

    assert_eq!(
        crate::view::panes::main::take_opened_web_links_for_tests(),
        vec!["https://example.com/badge".to_string()],
        "Ctrl/Cmd+click follows the link the badge wraps"
    );
    cx.update(|_window, app| {
        let this = view.read(app);
        let popover = this.popover_host.read(app).popover_kind_for_tests();
        assert!(popover.is_none(), "without a menu, got {popover:?}");
        assert!(
            !this.main_pane.read(app).diff_text_selecting,
            "and without arming a selection"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn linked_blocked_image_menu_loads_one_image_only_in_ask_mode(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.set_remote_markdown_image_policy(
                crate::view::RemoteMarkdownImagePolicy::AskBeforeLoading,
                cx,
            );
        });
    });

    // Keep two pictures in one paragraph so both remain inline and wrapped in
    // their respective links.
    let first_image_url = "https://images.example.invalid/one.svg";
    let second_image_url = "https://images.example.invalid/two.svg";
    let source = format!(
        "[![one]({first_image_url})](https://example.com/one) \
         [![two]({second_image_url})](https://example.com/two)\n"
    );
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(108),
        "markdown_linked_remote_image_approval",
        &source,
    );
    let source_byte = *fixture
        .picture_offsets()
        .first()
        .expect("the fixture carries a linked picture");
    let retry = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_load_{source_byte}"
        )))
        .expect("Ask mode draws the linked image's Retry control");

    // The linked image is one action: its completed click opens a menu with
    // navigation and image approval; the retry icon is part of that action.
    simulate_counted_click(cx, retry.center(), 1);
    cx.run_until_parked();
    cx.update(|_window, app| {
        let popover = view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests();
        assert!(matches!(
            popover,
            Some(PopoverKind::WebLinkMenu {
                ref url,
                load_remote_image_url: Some(ref image_url),
            }) if url.as_ref() == "https://example.com/one"
                && image_url.as_ref() == first_image_url
        ));
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let load_image = cx
        .debug_bounds("context_menu_load_image")
        .expect("a linked blocked image menu offers Load image")
        .center();
    cx.simulate_mouse_move(load_image, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        load_image,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseUpEvent {
        position: load_image,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.read(app);
        assert_eq!(main_pane.remote_markdown_images.approved_urls.len(), 1);
        assert!(
            main_pane
                .remote_markdown_images
                .approved_urls
                .contains(first_image_url)
        );
        assert!(
            !main_pane
                .remote_markdown_images
                .approved_urls
                .contains(second_image_url),
            "Load image must approve only the image represented by the menu"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.set_remote_markdown_image_policy(
                crate::view::RemoteMarkdownImagePolicy::NeverLoad,
                cx,
            );
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let blocked = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_load_{source_byte}_blocked_box"
        )))
        .expect("Never mode draws the linked image's blocked control");
    simulate_counted_click(cx, blocked.center(), 1);
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds("context_menu_load_image").is_none(),
        "Never mode must not offer any path to approve the remote image"
    );
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.read(app);
        assert!(main_pane.remote_markdown_images.approved_urls.is_empty());
        let popover = view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests();
        assert!(matches!(
            popover,
            Some(PopoverKind::WebLinkMenu {
                load_remote_image_url: None,
                ..
            })
        ));
    });

    fixture.cleanup();
}

#[gpui::test]
fn copying_a_link_address_says_that_it_was_copied(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Nothing on screen changes when a link's address goes to the clipboard —
    // the document shows the link's text, never its destination — so the copy
    // has to say so or the reader cannot tell it happened.
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    // Two badges, so they stay inline instead of one alone becoming a block.
    let source = "[![one](badge.svg)](https://example.com/badge)\n[![two](badge.svg)](https://example.com/other)\n";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(94),
        "markdown_copy_link_toast",
        source,
    );
    std::fs::write(
        fixture.workdir.join("docs/badge.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"80\" height=\"20\"><rect width=\"80\" height=\"20\"/></svg>",
    )
    .expect("write the badge the link points at");
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let source_byte = *fixture
        .picture_offsets()
        .first()
        .expect("the fixture carries a picture");
    let badge = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_{source_byte}"
        )))
        .expect("the badge is drawn");

    simulate_counted_click(cx, badge.center(), 1);
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let copy_entry = cx
        .debug_bounds("context_menu_copy_link_address")
        .expect("the link menu offers copying the address")
        .center();
    // Menu entries require their own completed click.
    cx.simulate_mouse_down(
        copy_entry,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_move(
        copy_entry,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseUpEvent {
        position: copy_entry,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
    });
    cx.run_until_parked();

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("https://example.com/badge".to_string())
    );
    cx.update(|_window, app| {
        let toasts = view.read(app).toast_host.read(app).toasts_for_tests(app);
        assert_eq!(
            toasts,
            vec![(
                crate::view::components::ToastKind::Success,
                "Link copied to clipboard".to_string()
            )],
            "copying a link address confirms itself"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn clicking_a_markdown_preview_link_opens_the_open_in_browser_menu(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(78),
        "markdown_link_menu",
        "[the docs](https://example.com/docs)\n",
    );

    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");
    // Left edge of the row's text is inside the link, which spans the row.
    let on_link = point(text_bounds.left() + px(4.0), text_bounds.center().y);

    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let link = this.main_pane.read(cx).markdown_preview_link_span_at(
                0,
                DiffTextRegion::Inline,
                on_link,
            );
            assert_eq!(
                link.as_ref().map(|(url, _)| url.as_ref()),
                Some("https://example.com/docs"),
                "the click position must resolve to the link destination"
            );

            let popover = this.popover_host.read(cx).popover_kind_for_tests();
            assert!(
                matches!(
                    popover,
                    Some(PopoverKind::WebLinkMenu { ref url, .. })
                        if url.as_ref() == "https://example.com/docs"
                ),
                "clicking a link should open its menu, got {popover:?}"
            );

            // The same menu is reachable from a commit message, where handing
            // focus back to the diff panel on close would be wrong. Closing
            // reads this flag, so a preview link has to set it.
            assert!(
                this.popover_host
                    .read(cx)
                    .popover_opened_from_diff_panel_for_tests(),
                "a preview link is a diff-panel invoker, so its focus returns there"
            );

            // The menu hangs off the link's own box rather than the row that
            // holds it, so it opens flush under the words it describes.
            let anchor = this
                .popover_host
                .read(cx)
                .popover_anchor_bounds_for_tests()
                .expect("a preview link menu anchors on the link's box");
            assert!(
                anchor.contains(&on_link),
                "the anchor must be the box the click landed in, got {anchor:?}"
            );
            assert!(
                anchor.top() >= text_bounds.top()
                    && anchor.bottom() <= text_bounds.bottom() + px(1.0),
                "the anchor must be a line of the row, not the row's own edges; \
                 anchor={anchor:?} row={text_bounds:?}"
            );
        });
    });

    fixture.cleanup();
}

/// Where row `row_ix`'s first visual line ends, in row bytes.
fn first_wrap_offset(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    row_ix: usize,
) -> usize {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hitbox = pane
            .diff_text_hitboxes
            .get(&(row_ix, DiffTextRegion::Inline))
            .expect("the row is drawn");
        let layout = &hitbox.wrapped.as_ref().expect("a wrapping row").layout;
        let line = layout.line_layout_for_index(0).expect("laid out");
        assert!(line.line_count() > 1, "the row wraps");
        line.visual_lines()[0].text_range.end
    })
}

/// A point on the link to `url` in each visual line of `row_ix` that holds it.
fn link_points_by_line(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    row_ix: usize,
    url: &str,
) -> Vec<gpui::Point<Pixels>> {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hitbox = pane
            .diff_text_hitboxes
            .get(&(row_ix, DiffTextRegion::Inline))
            .expect("the row is drawn");
        let line_height = hitbox
            .wrapped
            .as_ref()
            .expect("a wrapping row")
            .layout
            .line_height();
        let mut points = Vec::new();
        let mut y = hitbox.bounds.top() + line_height / 2.0;
        while y < hitbox.bounds.bottom() {
            let mut x = hitbox.bounds.left() + px(1.0);
            while x < hitbox.bounds.right() {
                let at = point(x, y);
                if pane
                    .markdown_preview_link_span_at(row_ix, DiffTextRegion::Inline, at)
                    .is_some_and(|(link, _)| link.as_ref() == url)
                {
                    points.push(at);
                    break;
                }
                x += px(2.0);
            }
            y += line_height;
        }
        points
    })
}

#[gpui::test]
fn a_link_menu_opens_under_the_words_on_the_line_clicked(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // gpui puts an offset at a wrap boundary at the end of the line above, so
    // a link that began a visual line hung its menu off the far end of the
    // previous one; a link over two lines hung it off its first part wherever
    // it was clicked.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));
    let words = |count: usize| "word ".repeat(count);

    let probe = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8843),
        "markdown_link_menu_wrap_probe",
        &format!("{}\n", words(200)),
    );
    let wrap = first_wrap_offset(cx, &view, 0);
    assert_eq!(wrap % "word ".len(), 0, "lines break between words");
    let per_line = wrap / "word ".len();
    probe.cleanup();

    // Preserve the probe's displayed words exactly. Proportional fonts do not
    // give "word" and "link" the same advance, so substituting different labels
    // would not guarantee either of the wrap boundaries this regression needs.
    let at_wrap = "https://example.com/at-wrap";
    let across = "https://example.com/across";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8844),
        "markdown_link_menu_wrap",
        &format!(
            "{}[word]({at_wrap}) {}\n\n{}[word word word]({across}) {}\n",
            words(per_line),
            words(30),
            words(per_line - 1),
            words(30),
        ),
    );
    let row_with = |text: String| {
        fixture
            .document
            .rows
            .iter()
            .position(|row| row.text.as_ref() == text.trim_end())
            .expect("the paragraph")
    };
    for (row_ix, url, lines) in [
        (
            row_with(format!("{}word {}", words(per_line), words(30))),
            at_wrap,
            1,
        ),
        (
            row_with(format!(
                "{}word word word {}",
                words(per_line - 1),
                words(30)
            )),
            across,
            2,
        ),
    ] {
        let points = link_points_by_line(cx, &view, row_ix, url);
        assert_eq!(points.len(), lines, "{url} is on {lines} visual line(s)");
        let row_top = cx.update(|_window, app| {
            view.read(app)
                .main_pane
                .read(app)
                .diff_text_hitbox_bounds_for_tests(row_ix, DiffTextRegion::Inline)
                .expect("the row is drawn")
                .top()
        });
        // The part on the second visual line.
        let on_link = *points.last().expect("a point on the link");
        assert!(
            on_link.y > row_top + px(20.0),
            "clicked below the first line"
        );

        simulate_counted_click(cx, on_link, 1);
        cx.run_until_parked();
        cx.update(|_window, app| {
            let host = view.read(app).popover_host.clone();
            let anchor = host
                .read(app)
                .popover_anchor_bounds_for_tests()
                .expect("the link menu anchors on the link");
            assert!(
                anchor.contains(&on_link),
                "{url}: the menu hangs off the words clicked at {on_link:?}, not {anchor:?}"
            );
            host.update(app, |host, cx| host.close_popover(cx));
        });
        cx.run_until_parked();
    }

    fixture.cleanup();
}

#[gpui::test]
fn ctrl_clicking_a_web_link_opens_the_browser_without_a_menu(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8827),
        "markdown_ctrl_click_web_link",
        "[the docs](https://example.com/docs)\n",
    );
    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");
    let on_link = point(text_bounds.left() + px(4.0), text_bounds.center().y);
    crate::view::panes::main::take_opened_web_links_for_tests();

    simulate_modified_click(cx, on_link, 1, Modifiers::secondary_key());
    cx.run_until_parked();

    assert_eq!(
        crate::view::panes::main::take_opened_web_links_for_tests(),
        vec!["https://example.com/docs".to_string()],
        "Ctrl/Cmd+click opens the link in the browser"
    );
    cx.update(|_window, app| {
        let this = view.read(app);
        let popover = this.popover_host.read(app).popover_kind_for_tests();
        assert!(popover.is_none(), "and skips the menu, got {popover:?}");
        assert!(
            !this.main_pane.read(app).diff_text_has_selection(),
            "following the link is not a text selection"
        );
    });

    // A plain click still asks first.
    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();
    assert!(
        crate::view::panes::main::take_opened_web_links_for_tests().is_empty(),
        "a plain click opens nothing by itself"
    );
    let popover = popover_kind(cx, &view);
    assert!(
        matches!(popover, Some(PopoverKind::WebLinkMenu { .. })),
        "a plain click opens the menu, got {popover:?}"
    );

    fixture.cleanup();
}

/// Click the single entry a local-file link menu offers, once the menu has
/// been drawn.
fn click_open_in_gitcomet(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let entry = cx
        .debug_bounds("context_menu_open_in_gitcomet")
        .expect("a local file link menu offers Open in GitComet")
        .center();
    cx.simulate_mouse_move(entry, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(entry, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseUpEvent {
        position: entry,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
    });
    cx.run_until_parked();
}

#[gpui::test]
fn clicking_a_local_markdown_link_offers_open_in_gitcomet(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(79),
        "markdown_local_link_menu",
        "[other](./other.md)\n",
    );
    // Written after `open`, which starts from an empty workdir.
    std::fs::write(fixture.workdir.join("docs/other.md"), "# Other\n").expect("write link target");

    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");
    let on_link = point(text_bounds.left() + px(4.0), text_bounds.center().y);

    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let popover = this.popover_host.read(cx).popover_kind_for_tests();
            assert!(
                matches!(
                    popover,
                    Some(PopoverKind::LocalFileLinkMenu {
                        source: crate::view::LocalFileLinkSource::Version(
                            gitcomet_core::domain::FileSource::WorkingDirectory
                        ),
                        ref path,
                        missing: false,
                        load_remote_image_url: None,
                        ..
                    }) if path == std::path::Path::new("docs/other.md")
                ),
                "a local link resolves against the document's directory, got {popover:?}"
            );
            assert!(
                this.popover_host
                    .read(cx)
                    .popover_opened_from_diff_panel_for_tests(),
                "a preview link is a diff-panel invoker, so its focus returns there"
            );
        });
    });

    click_open_in_gitcomet(cx);

    // The entry dispatches to the store, whose worker reduces it off the gpui
    // executor: poll the store rather than the pane, which the poller feeds in
    // the running app.
    let expected_target = gitcomet_core::domain::DiffTarget::working_tree(
        std::path::PathBuf::from("docs/other.md"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let navigated = cx.update(|_window, app| {
            let snapshot = view.read(app).store.snapshot();
            let repo = snapshot
                .repos
                .iter()
                .find(|repo| repo.id == gitcomet_state::model::RepoId(79));
            repo.map(|repo| {
                (
                    repo.diff_state.diff_target.clone(),
                    repo.diff_state.content_preview,
                )
            })
        });
        if navigated == Some((Some(expected_target.clone()), true)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Open in GitComet must open the linked file as a content preview, got {navigated:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cx.run_until_parked();
    cx.update(|_window, app| {
        let popover = view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests();
        assert!(
            popover.is_none(),
            "the menu closes once its entry runs, got {popover:?}"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn a_local_link_to_a_missing_file_shows_a_disabled_entry(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    // `..` climbs from `docs/` to the root, where nothing is written.
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(80),
        "markdown_missing_local_link_menu",
        "[gone](../missing.txt)\n",
    );
    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");
    let on_link = point(text_bounds.left() + px(4.0), text_bounds.center().y);

    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        let popover = view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests();
        assert!(
            matches!(
                popover,
                Some(PopoverKind::LocalFileLinkMenu {
                    ref path,
                    missing: true,
                    ..
                }) if path == std::path::Path::new("missing.txt")
            ),
            "a dangling link still says where it points, got {popover:?}"
        );
    });

    // A disabled entry has no activation: the menu stays and nothing moves.
    click_open_in_gitcomet(cx);
    cx.update(|_window, app| {
        let this = view.read(app);
        let popover = this.popover_host.read(app).popover_kind_for_tests();
        assert!(
            matches!(popover, Some(PopoverKind::LocalFileLinkMenu { .. })),
            "a greyed-out entry does not close the menu, got {popover:?}"
        );
        let target = this
            .main_pane
            .read(app)
            .active_repo()
            .and_then(|repo| repo.diff_state.diff_target.clone());
        assert_eq!(
            target,
            Some(gitcomet_core::domain::DiffTarget::working_tree(
                std::path::PathBuf::from("docs/preview.md"),
                gitcomet_core::domain::DiffArea::Unstaged
            )),
            "nothing to open, so the pane stays on the document"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn ctrl_clicking_a_local_link_opens_the_file_without_a_menu(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8828),
        "markdown_ctrl_click_local_link",
        "[other](./other.md)\n",
    );
    std::fs::write(fixture.workdir.join("docs/other.md"), "# Other\n").expect("write link target");
    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");
    let on_link = point(text_bounds.left() + px(4.0), text_bounds.center().y);

    simulate_modified_click(cx, on_link, 1, Modifiers::secondary_key());
    cx.run_until_parked();
    let popover = popover_kind(cx, &view);
    assert!(
        popover.is_none(),
        "Ctrl/Cmd+click skips the menu, got {popover:?}"
    );

    // As with the menu entry, the store's worker does the navigating.
    let expected_target = gitcomet_core::domain::DiffTarget::working_tree(
        std::path::PathBuf::from("docs/other.md"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let navigated = cx.update(|_window, app| {
            let snapshot = view.read(app).store.snapshot();
            let repo = snapshot
                .repos
                .iter()
                .find(|repo| repo.id == gitcomet_state::model::RepoId(8828));
            repo.map(|repo| {
                (
                    repo.diff_state.diff_target.clone(),
                    repo.diff_state.content_preview,
                )
            })
        });
        if navigated == Some((Some(expected_target.clone()), true)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Ctrl/Cmd+click must open the linked file as a content preview, got {navigated:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    fixture.cleanup();
}

#[gpui::test]
fn ctrl_clicking_a_link_to_a_missing_file_still_opens_the_menu(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8829),
        "markdown_ctrl_click_missing_local_link",
        "[gone](../missing.txt)\n",
    );
    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");
    let on_link = point(text_bounds.left() + px(4.0), text_bounds.center().y);

    simulate_modified_click(cx, on_link, 1, Modifiers::secondary_key());
    cx.run_until_parked();

    // Nothing to open, so the greyed-out entry is left to say why.
    let popover = popover_kind(cx, &view);
    assert!(
        matches!(
            popover,
            Some(PopoverKind::LocalFileLinkMenu { missing: true, .. })
        ),
        "a dangling link falls back to its menu, got {popover:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_root_relative_link_resolves_from_the_repo_root(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(81),
        "markdown_root_relative_link_menu",
        "[root](/docs/other.md)\n",
    );
    std::fs::write(fixture.workdir.join("docs/other.md"), "# Other\n").expect("write link target");
    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");
    let on_link = point(text_bounds.left() + px(4.0), text_bounds.center().y);

    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        let popover = view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests();
        assert!(
            matches!(
                popover,
                Some(PopoverKind::LocalFileLinkMenu {
                    ref path,
                    missing: false,
                    ..
                }) if path == std::path::Path::new("docs/other.md")
            ),
            "a leading slash is repository-root-relative, got {popover:?}"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn a_link_in_a_commit_preview_is_offered_even_when_the_worktree_lost_the_file(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The worktree is not the tree the document came from: a file deleted
    // since that commit is still there to read.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(82);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_commit_link",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join("docs")).expect("create workdir");
    let commit_id = gitcomet_core::domain::CommitId("deadbeef".into());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::commit(
                commit_id.clone(),
                std::path::PathBuf::from("docs/preview.md"),
            ));
            repo.diff_state.diff_state_rev = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let kind = pane.markdown_preview_link_popover_kind(
            DiffTextRegion::Inline,
            0,
            &"./deleted.md".into(),
            None,
        );
        assert!(
            matches!(
                kind,
                Some(PopoverKind::LocalFileLinkMenu {
                    source: crate::view::LocalFileLinkSource::Version(
                        gitcomet_core::domain::FileSource::Commit(ref id)
                    ),
                    ref path,
                    missing: false,
                    ..
                }) if *id == commit_id && path == std::path::Path::new("docs/deleted.md")
            ),
            "a commit's link opens from that commit, got {kind:?}"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[cfg(unix)]
#[gpui::test]
fn a_local_link_through_a_symlink_out_of_the_repo_is_inert(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(83),
        "markdown_symlink_escape_link",
        "[file](alias/secret.txt) [meta](meta/config)\n",
    );
    // Outside the fixture's workdir, which is the repository.
    let outside = fixture.workdir.with_extension("outside");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).expect("create outside dir");
    std::fs::write(outside.join("secret.txt"), "secret").expect("write outside file");
    std::fs::create_dir_all(fixture.workdir.join(".git")).expect("create .git");
    std::fs::write(fixture.workdir.join(".git/config"), "[core]").expect("write config");
    std::os::unix::fs::symlink(&outside, fixture.workdir.join("docs/alias")).expect("alias");
    std::os::unix::fs::symlink(
        fixture.workdir.join(".git"),
        fixture.workdir.join("docs/meta"),
    )
    .expect("meta");

    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");
    let on_link = point(text_bounds.left() + px(4.0), text_bounds.center().y);
    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        let popover = view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests();
        assert!(
            popover.is_none(),
            "a symlink must not carry a link out of the repository, got {popover:?}"
        );
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.markdown_preview_link_popover_kind(
                DiffTextRegion::Inline,
                0,
                &"meta/config".into(),
                None
            ),
            None,
            "a symlink must not carry a link into .git"
        );
    });

    std::fs::remove_dir_all(&outside).expect("cleanup outside");
    fixture.cleanup();
}

#[gpui::test]
fn a_linked_blocked_image_whose_link_cannot_open_still_loads_on_click(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // `./` names no file, so there is no menu to carry Load image: the click
    // must approve the picture directly, as it did before local links opened.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.set_remote_markdown_image_policy(
                crate::view::RemoteMarkdownImagePolicy::AskBeforeLoading,
                cx,
            );
        });
    });

    let first_image_url = "https://images.example.invalid/one.svg";
    let second_image_url = "https://images.example.invalid/two.svg";
    let source = format!("[![one]({first_image_url})](./) [![two]({second_image_url})](../)\n");
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(84),
        "markdown_inert_linked_remote_image",
        &source,
    );
    let source_byte = *fixture
        .picture_offsets()
        .first()
        .expect("the fixture carries a linked picture");
    let retry = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_load_{source_byte}"
        )))
        .expect("Ask mode draws the linked image's Retry control");

    simulate_counted_click(cx, retry.center(), 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        let this = view.read(app);
        let popover = this.popover_host.read(app).popover_kind_for_tests();
        assert!(
            popover.is_none(),
            "an inert link opens no menu, got {popover:?}"
        );
        let main_pane = this.main_pane.read(app);
        assert!(
            main_pane
                .remote_markdown_images
                .approved_urls
                .contains(first_image_url),
            "the click approves the picture it landed on"
        );
        assert!(
            !main_pane
                .remote_markdown_images
                .approved_urls
                .contains(second_image_url),
            "and only that picture"
        );
    });

    fixture.cleanup();
}

/// A document whose table links to a heading far below the fold, with room
/// below it for the heading to reach the top.
fn anchor_link_fixture_source(heading: &str) -> String {
    let filler: String = (0..200)
        .map(|ix| format!("paragraph {ix:03}\n\n"))
        .collect();
    format!(
        "| Badge | Meaning |\n| --- | --- |\n\
         | **Untrusted key** | See [Trust a GPG key](#trust-a-gpg-key). |\n\n\
         {filler}## {heading}\n\nThe heading's section.\n\n{filler}"
    )
}

/// A point on the first link in rendered row `row_ix` of the worktree preview.
fn point_on_link_in_row(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    row_ix: usize,
) -> gpui::Point<Pixels> {
    // A table row paints one text box per cell, each of which may wrap.
    let boxes: Vec<Bounds<Pixels>> = std::iter::once(format!("markdown_preview_text_box_{row_ix}"))
        .chain((0..32).map(|column| format!("markdown_preview_cell_text_box_{row_ix}_{column}")))
        .filter_map(|selector| cx.debug_bounds(leaked_selector(selector)))
        .collect();
    assert!(!boxes.is_empty(), "row {row_ix} draws no text");
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for text_box in &boxes {
            let mut y = text_box.top() + px(2.0);
            while y < text_box.bottom() {
                let mut x = text_box.left();
                while x < text_box.right() {
                    let position = point(x, y);
                    if pane
                        .markdown_preview_link_span_at(row_ix, DiffTextRegion::Inline, position)
                        .is_some()
                    {
                        return position;
                    }
                    x += px(2.0);
                }
                y += px(4.0);
            }
        }
        panic!("no link in row {row_ix}");
    })
}

#[gpui::test]
fn clicking_an_anchor_link_in_a_table_scrolls_to_the_heading(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(86),
        "markdown_anchor_link_scroll",
        &anchor_link_fixture_source("Trust a GPG key"),
    );
    let link_row = fixture
        .document
        .rows
        .iter()
        .position(|row| row.text.contains("Trust a GPG key"))
        .expect("the table row with the link");
    let heading_row = fixture.row_ix("Trust a GPG key");
    let on_link = point_on_link_in_row(cx, &view, link_row);

    // The click's frame is where the reveal runs. It moves the offset during
    // prepaint, too late for that frame, so it has to ask for the next one or
    // the scrollbar moves while the text stays put until the next input. Only
    // frames the app itself asked for are drawn from here on.
    // The click's frame is where the reveal runs. It moves the offset during
    // prepaint, too late for that frame, so it has to ask for the next one or
    // the scrollbar moves while the text stays put until the next input.
    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();
    let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let _subscription = cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let notified = std::rc::Rc::clone(&notified);
        app.observe(&main_pane, move |_, _| notified.set(notified.get() + 1))
    });
    cx.update(|window, app| {
        window.simulate_next_frame(app);
    });
    assert!(
        notified.get() > 0,
        "the reveal must ask for a repaint after moving the scroll offset"
    );
    cx.run_until_parked();

    let viewport = cx.update(|_window, app| {
        let this = view.read(app);
        let popover = this.popover_host.read(app).popover_kind_for_tests();
        assert!(
            popover.is_none(),
            "an anchor opens no menu, got {popover:?}"
        );
        let pane = this.main_pane.read(app);
        assert!(
            !pane.diff_text_has_selection(),
            "following the link is not a text selection"
        );
        assert_eq!(
            pane.markdown_interaction.reveal.pending(),
            None,
            "the reveal is claimed once"
        );
        let scroll = pane.worktree_preview_scroll.0.borrow().base_handle.clone();
        assert!(
            scroll.offset().y < px(0.0),
            "the preview scrolls down to the heading"
        );
        scroll.bounds()
    });
    // The heading lands at the top of the viewport, as in a browser.
    let heading = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{heading_row}"
        )))
        .expect("the heading is drawn once scrolled to");
    assert!(
        heading.top() >= viewport.top() - px(1.0) && heading.top() < viewport.top() + px(40.0),
        "heading at {:?}, viewport from {:?}",
        heading.top(),
        viewport.top()
    );

    fixture.cleanup();
}

#[gpui::test]
fn ctrl_clicking_an_anchor_link_scrolls_to_the_heading(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8831),
        "markdown_ctrl_click_anchor_link",
        &anchor_link_fixture_source("Trust a GPG key"),
    );
    let link_row = fixture
        .document
        .rows
        .iter()
        .position(|row| row.text.contains("Trust a GPG key"))
        .expect("the table row with the link");
    let on_link = point_on_link_in_row(cx, &view, link_row);

    simulate_modified_click(cx, on_link, 1, Modifiers::secondary_key());
    cx.run_until_parked();
    cx.update(|window, app| {
        window.simulate_next_frame(app);
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let this = view.read(app);
        let popover = this.popover_host.read(app).popover_kind_for_tests();
        assert!(
            popover.is_none(),
            "an anchor opens no menu, got {popover:?}"
        );
        let pane = this.main_pane.read(app);
        let scroll = pane.worktree_preview_scroll.0.borrow().base_handle.clone();
        assert!(
            scroll.offset().y < px(0.0),
            "Ctrl/Cmd+click scrolls to the heading like a plain click"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn an_anchor_link_without_a_heading_is_plain_text(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(87),
        "markdown_anchor_link_missing",
        &anchor_link_fixture_source("Something else"),
    );
    let link_row = fixture
        .document
        .rows
        .iter()
        .position(|row| row.text.contains("Trust a GPG key"))
        .expect("the table row with the link");
    let on_link = point_on_link_in_row(cx, &view, link_row);

    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let this = view.read(app);
        let popover = this.popover_host.read(app).popover_kind_for_tests();
        assert!(popover.is_none(), "nothing to open, got {popover:?}");
        let pane = this.main_pane.read(app);
        assert_eq!(pane.markdown_interaction.reveal.pending(), None);
        assert_eq!(
            pane.worktree_preview_scroll
                .0
                .borrow()
                .base_handle
                .offset()
                .y,
            px(0.0),
            "nothing to scroll to"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn an_anchor_link_scrolls_the_rendered_markdown_diff(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let old_text = anchor_link_fixture_source("Trust a GPG key");
    let new_text = format!("{old_text}\nAn added line.\n");
    let workdir = open_rendered_markdown_diff(
        cx,
        &view,
        gitcomet_state::model::RepoId(88),
        "markdown_diff_anchor_scroll",
        &old_text,
        &new_text,
    );

    let followed = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                reset_uniform_list_offsets(&[&pane.diff_scroll]);
                assert!(
                    !pane.scroll_markdown_preview_to_anchor(
                        DiffTextRegion::Inline,
                        "#no-such-heading",
                        cx
                    ),
                    "an unknown anchor is not followed"
                );
                pane.scroll_markdown_preview_to_anchor(
                    DiffTextRegion::Inline,
                    "#trust-a-gpg-key",
                    cx,
                )
            })
        })
    });
    assert!(followed, "the heading is in the diff preview");
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            uniform_list_offset(&pane.diff_scroll).y < px(0.0),
            "the rendered diff scrolls to the heading, offset stayed at {:?}",
            uniform_list_offset(&pane.diff_scroll),
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

fn hovered_link(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) -> Option<crate::view::rows::MarkdownPreviewHoveredLink> {
    cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .markdown_interaction
            .hovered_link
            .clone()
    })
}

fn move_mouse(cx: &mut gpui::VisualTestContext, position: gpui::Point<Pixels>, held: bool) {
    cx.simulate_mouse_move(
        position,
        held.then_some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
}

#[gpui::test]
fn hovering_a_link_underlines_it_and_shows_the_pointer(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    use crate::view::rows::MarkdownPreviewHoveredLink;

    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(89),
        "markdown_link_hover",
        "Plain words then [a **bold** link](https://example.com)\n",
    );
    let text = fixture.document.rows[0].text.clone();
    let link_start = text.find("a bold link").expect("link text");
    let link_end = link_start + "a bold link".len();
    let text_box = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("the row's text box");
    let on_plain = point(text_box.left() + px(4.0), text_box.center().y);
    let on_link = point_on_link_in_row(cx, &view, 0);
    // The text box spans the pane; its far end is past the painted text.
    let past_text = point(text_box.right() - px(4.0), text_box.center().y);

    move_mouse(cx, on_link, false);
    let hovered = hovered_link(cx, &view).expect("the pointer is on the link");
    assert_eq!(
        (hovered.row_ix, hovered.byte_range.clone()),
        (0, link_start..link_end),
        "the whole link is hovered, across its bold run"
    );
    assert_eq!(
        MarkdownPreviewHoveredLink::cursor(Some(&hovered), DiffTextRegion::Inline, 0),
        gpui::CursorStyle::PointingHand
    );
    assert_eq!(
        MarkdownPreviewHoveredLink::cursor(Some(&hovered), DiffTextRegion::Inline, 1),
        gpui::CursorStyle::IBeam,
        "only the row under the pointer shows the hand"
    );

    move_mouse(cx, on_plain, false);
    assert_eq!(hovered_link(cx, &view), None, "plain words are not a link");

    move_mouse(cx, past_text, false);
    assert_eq!(
        hovered_link(cx, &view),
        None,
        "beside a line that ends in a link is not on it"
    );
    simulate_counted_click(cx, past_text, 1);
    cx.run_until_parked();
    cx.update(|_window, app| {
        let popover = view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests();
        assert!(
            popover.is_none(),
            "a click beside the link does not open it, got {popover:?}"
        );
    });

    // A drag across a link is a selection, not a hover.
    move_mouse(cx, on_link, true);
    assert_eq!(hovered_link(cx, &view), None);

    // Leaving the row drops the hover.
    move_mouse(cx, on_link, false);
    assert!(hovered_link(cx, &view).is_some());
    move_mouse(cx, point(px(1.0), px(1.0)), false);
    assert_eq!(
        hovered_link(cx, &view),
        None,
        "the pointer left the preview"
    );

    fixture.cleanup();
}

#[gpui::test]
fn hovering_a_link_in_the_rendered_diff_tracks_it(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let old_text = "# Title\n\nSee [the guide](https://example.com/guide) here.\n";
    let new_text = format!("{old_text}\nAdded.\n");
    let workdir = open_rendered_markdown_diff(
        cx,
        &view,
        gitcomet_state::model::RepoId(90),
        "markdown_diff_link_hover",
        old_text,
        &new_text,
    );
    let link_row = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        preview
            .inline
            .rows
            .iter()
            .position(|row| row.text.contains("the guide"))
            .expect("the row with the link")
    });
    let on_link = point_on_link_in_row(cx, &view, link_row);

    move_mouse(cx, on_link, false);
    let hovered = hovered_link(cx, &view).expect("the pointer is on the link");
    assert_eq!(hovered.region, DiffTextRegion::Inline);
    assert_eq!(hovered.row_ix, link_row);
    move_mouse(cx, point(px(1.0), px(1.0)), false);
    assert_eq!(hovered_link(cx, &view), None);

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

/// A point on the link in one table cell of the worktree preview.
fn point_on_link_in_cell(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    row_ix: usize,
    column: usize,
) -> gpui::Point<Pixels> {
    let text_box = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_cell_text_box_{row_ix}_{column}"
        )))
        .unwrap_or_else(|| panic!("cell {column} of row {row_ix} draws text"));
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let y = text_box.center().y;
        let mut x = text_box.left();
        while x < text_box.right() {
            let position = point(x, y);
            if pane
                .markdown_preview_link_span_at(row_ix, DiffTextRegion::Inline, position)
                .is_some()
            {
                return position;
            }
            x += px(1.0);
        }
        panic!("no link in cell {column} of row {row_ix}")
    })
}

#[gpui::test]
fn a_blocked_image_linked_to_an_anchor_still_loads_on_click(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.set_remote_markdown_image_policy(
                crate::view::RemoteMarkdownImagePolicy::AskBeforeLoading,
                cx,
            );
        });
    });

    let image_url = "https://images.example.invalid/badge.svg";
    // Text beside the picture keeps it inline; alone on a line it is a block.
    let source =
        format!("[![badge]({image_url})](#install) Install guide\n\n## Install\n\nSteps.\n");
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(962),
        "markdown_anchor_linked_blocked_image",
        &source,
    );
    let source_byte = *fixture
        .picture_offsets()
        .first()
        .expect("the fixture carries a linked picture");
    let load = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_load_{source_byte}"
        )))
        .expect("Ask mode draws the linked image's load control");

    simulate_counted_click(cx, load.center(), 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        assert!(
            view.read(app)
                .main_pane
                .read(app)
                .remote_markdown_images
                .approved_urls
                .contains(image_url),
            "an anchor has no menu to carry Load image, so the click loads the picture"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn a_linked_image_on_the_new_side_follows_the_new_documents_anchor(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1400.0), px(600.0)));

    let filler: String = (0..80).map(|ix| format!("paragraph {ix:03}\n\n")).collect();
    // Filler below the heading too, so it has room to reach the top.
    let old_text = format!("Intro.\n\n{filler}End.\n\n{filler}");
    let new_text = format!(
        "Intro.\n\n[![setup](missing.png)](#setup) How to begin\n\n{filler}## Setup\n\nSteps.\n\nEnd.\n\n{filler}"
    );
    let workdir = open_rendered_markdown_diff_in(
        cx,
        &view,
        gitcomet_state::model::RepoId(963),
        "markdown_split_new_side_image_anchor",
        &old_text,
        &new_text,
        DiffViewMode::Split,
    );
    let (source_byte, heading_row) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        (
            *picture_offsets(&preview.new)
                .first()
                .expect("the new side carries the picture"),
            row_ix_with_text(&preview.new, "Setup"),
        )
    });
    // The first-change autoscroll has run; start from the top so only the
    // click can move the view.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_scroll
                    .0
                    .borrow()
                    .base_handle
                    .set_offset(point(px(0.0), px(0.0)));
                pane.markdown_interaction.reveal.clear();
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    let image = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_{source_byte}"
        )))
        .expect("the linked picture is drawn on the new side");

    // The picture can be wider than its column; its left edge is on it.
    simulate_counted_click(cx, point(image.left() + px(6.0), image.center().y), 1);
    cx.run_until_parked();
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }

    let (viewport, heading) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.diff_scroll.0.borrow().base_handle.bounds(),
            pane.diff_text_hitbox_bounds_for_tests(heading_row, DiffTextRegion::SplitRight),
        )
    });
    let heading = heading.expect("the #setup heading is drawn once scrolled to");
    assert!(
        heading.top() >= viewport.top() - px(1.0) && heading.top() < viewport.top() + px(60.0),
        "the new side's heading lands at the top: heading={heading:?} viewport={viewport:?}"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn a_link_on_the_old_side_of_a_commit_diff_opens_the_parent_version(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let repo_id = gitcomet_state::model::RepoId(966);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_commit_old_side_link",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join("docs")).expect("create workdir");
    let commit_id = gitcomet_core::domain::CommitId("c0ffee".into());
    let path = std::path::PathBuf::from("docs/a.md");
    let target = gitcomet_core::domain::DiffTarget::commit(commit_id.clone(), path.clone());
    let old_text = "See [old](old.md) here.\n\nBefore.\n";
    let new_text = "See [old](old.md) here.\n\nAfter.\n";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = 1;
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some(old_text.to_string()),
                    Some(new_text.to_string()),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_markdown.cache_repo_id = Some(repo_id);
                pane.diff_markdown.cache_rev = 1;
                pane.diff_markdown.cache_target = Some(target.clone());
                pane.diff_markdown.preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::build_markdown_diff_preview(old_text, new_text)
                        .expect("markdown diff preview should parse"),
                ));
                pane.diff_markdown.inflight = None;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            });
        });
    });
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }
    let row = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active(), "the commit diff renders");
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        row_ix_with_text(&preview.old, "See old here.")
    });

    let on_link = point_on_link_in_region(cx, &view, row, DiffTextRegion::SplitLeft);
    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();
    let popover = popover_kind(cx, &view);
    let Some(PopoverKind::LocalFileLinkMenu { source, path, .. }) = popover else {
        panic!("a local link opens its menu, got {popover:?}");
    };
    assert_eq!(path, std::path::PathBuf::from("docs/old.md"));
    assert_eq!(
        source,
        crate::view::LocalFileLinkSource::ParentOf(commit_id.clone()),
        "the old side shows the file before the commit, so its links open there too"
    );
    close_popover(cx, &view);

    // The new side is the commit itself.
    let on_link = point_on_link_in_region(cx, &view, row, DiffTextRegion::SplitRight);
    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();
    let popover = popover_kind(cx, &view);
    assert!(
        matches!(
            popover,
            Some(PopoverKind::LocalFileLinkMenu {
                source: crate::view::LocalFileLinkSource::Version(
                    gitcomet_core::domain::FileSource::Commit(ref id)
                ),
                ..
            }) if *id == commit_id
        ),
        "the new side's links open at the commit, got {popover:?}"
    );
    close_popover(cx, &view);

    // Ctrl/Cmd+click opens the parent version the menu would have offered.
    let on_link = point_on_link_in_region(cx, &view, row, DiffTextRegion::SplitLeft);
    simulate_modified_click(cx, on_link, 1, Modifiers::secondary_key());
    cx.run_until_parked();
    let popover = popover_kind(cx, &view);
    assert!(
        popover.is_none(),
        "Ctrl/Cmd+click on the old side skips the menu, got {popover:?}"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn hovering_a_link_that_cannot_open_shows_no_pointer(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    for (ix, source) in [
        "[the next page](?page=2)\n",
        "[a section](#no-such-heading)\n",
        "[up and out](../../../outside.md)\n",
    ]
    .into_iter()
    .enumerate()
    {
        let fixture = RenderedPreviewFixture::open(
            cx,
            &view,
            gitcomet_state::model::RepoId(968),
            &format!("markdown_inert_link_hover_{ix}"),
            source,
        );
        let on_link = point_on_link_in_row(cx, &view, 0);
        move_mouse(cx, on_link, false);
        assert_eq!(
            hovered_link(cx, &view),
            None,
            "{source:?}: a click does nothing here, so the hover must not promise one"
        );
        move_mouse(cx, point(px(1.0), px(1.0)), false);
        fixture.cleanup();
    }
}

#[gpui::test]
fn moving_between_links_in_one_table_row_keeps_the_new_hover(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(969),
        "markdown_table_link_hover_hop",
        "| First | Second |\n| --- | --- |\n| [one](https://example.com/one) | [two](https://example.com/two) |\n",
    );
    let row = fixture
        .document
        .rows
        .iter()
        .position(|row| row.text.contains("one"))
        .expect("the body row");
    let two_start = fixture.document.rows[row]
        .text
        .find("two")
        .expect("second link text");
    let on_one = point_on_link_in_cell(cx, &view, row, 0);
    let on_two = point_on_link_in_cell(cx, &view, row, 1);

    move_mouse(cx, on_one, false);
    assert!(hovered_link(cx, &view).is_some(), "on the first link");
    move_mouse(cx, on_two, false);
    let hovered = hovered_link(cx, &view).expect("the pointer is on the second link");
    assert_eq!(
        hovered.byte_range,
        two_start..two_start + "two".len(),
        "leaving the first cell must not clear the link the second one claimed"
    );

    fixture.cleanup();
}

#[gpui::test]
fn image_paths_resolve_within_the_repository_like_links(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    // docs/preview.md referring to pictures beside it, one level up, and by a
    // repository-root path, as GitHub resolves all three.
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8805),
        "markdown_image_paths",
        "![here](assets/here.png)\n\n![up](../assets/up.png)\n\n![root](/assets/root.png)\n",
    );
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    for rel in ["docs/assets/here.png", "assets/up.png", "assets/root.png"] {
        let path = fixture.workdir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("asset dir");
        std::fs::write(&path, PNG_1X1).expect("write asset");
    }
    draw_frames(cx, 3);

    let pictures: Vec<usize> = fixture
        .document
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            matches!(
                row.kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::Image
            )
        })
        .map(|(ix, _)| ix)
        .collect();
    assert_eq!(pictures.len(), 3, "three block pictures");
    let resolved: Vec<bool> = pictures
        .iter()
        .map(|ix| {
            cx.debug_bounds(leaked_selector(format!(
                "markdown_preview_block_image_{ix}"
            )))
            .is_some()
        })
        .collect();
    assert!(
        resolved[0],
        "control: a picture beside the document resolves"
    );
    assert_eq!(
        resolved,
        vec![true, true, true],
        "`../` and `/`-rooted pictures inside the repository show as \"Image unavailable\""
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_link_on_the_old_copy_of_a_modified_paragraph_opens_the_parent_version(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The inline diff shows a paragraph that changed in part twice, old copy
    // first; both copies are marked modified, not removed and added.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let repo_id = gitcomet_state::model::RepoId(8814);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_inline_modified_old_link",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join("docs")).expect("create workdir");
    let commit_id = gitcomet_core::domain::CommitId("c0ffee".into());
    let path = std::path::PathBuf::from("docs/a.md");
    let target = gitcomet_core::domain::DiffTarget::commit(commit_id.clone(), path.clone());
    let old_text = "See [spec](spec.md) and\nthe old ending.\n";
    let new_text = "See [spec](spec.md) and\nthe new ending.\n";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = 1;
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some(old_text.to_string()),
                    Some(new_text.to_string()),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_markdown.cache_repo_id = Some(repo_id);
                pane.diff_markdown.cache_rev = 1;
                pane.diff_markdown.cache_target = Some(target.clone());
                pane.diff_markdown.preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::build_markdown_diff_preview(old_text, new_text)
                        .expect("markdown diff preview should parse"),
                ));
                pane.diff_markdown.inflight = None;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.diff_view = DiffViewMode::Inline;
                cx.notify();
            });
        });
    });
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }
    let old_row = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        row_ix_with_text(&preview.inline, "See spec and the old ending.")
    });

    let on_link = point_on_link_in_region(cx, &view, old_row, DiffTextRegion::Inline);
    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();
    let popover = popover_kind(cx, &view);
    let Some(PopoverKind::LocalFileLinkMenu { source, .. }) = popover else {
        panic!("a local link opens its menu, got {popover:?}");
    };
    assert_eq!(
        source,
        crate::view::LocalFileLinkSource::ParentOf(commit_id.clone()),
        "the old copy of a modified paragraph shows the file before the commit"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn moving_over_a_link_that_goes_nowhere_checks_it_once(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // A link to no heading stays plain words; working that out slugs every
    // heading (or, for a file link, stats the disk), so it is done once per
    // link the pointer enters, not on every move across it.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8816),
        "markdown_link_hover_dead_anchor",
        "See [a heading that is not there](#no-such-heading) here\n",
    );
    let on_link = point_on_link_in_row(cx, &view, 0);
    let text_box = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("the row's text box");

    crate::view::panes::main::take_link_followability_checks_for_tests();
    for step in 0..6 {
        move_mouse(cx, point(on_link.x + px(step as f32), on_link.y), false);
    }
    assert_eq!(hovered_link(cx, &view), None, "the dead link stays plain");
    assert_eq!(
        crate::view::panes::main::take_link_followability_checks_for_tests(),
        1,
        "the link is checked when the pointer enters it"
    );

    // Leaving the link and coming back checks it again: the file or heading
    // may have appeared meanwhile.
    move_mouse(cx, point(text_box.left() + px(2.0), on_link.y), false);
    move_mouse(cx, on_link, false);
    assert_eq!(
        crate::view::panes::main::take_link_followability_checks_for_tests(),
        1,
        "re-entering the link checks it once more"
    );

    fixture.cleanup();
}
