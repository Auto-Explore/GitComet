//! Pictures and the remote-image policy: loading, sizing, approval, copy.

use super::*;

#[gpui::test]
fn markdown_preview_draws_an_inline_picture_beside_its_heading(cx: &mut gpui::TestAppContext) {
    // The pictures are sized by `max_h` against a `flex_none` wrapper, which is
    // the kind of constraint that can collapse to zero without any parse-level
    // assertion noticing.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let logo = "docs/logo.svg";
    let source = "# <img alt=\"logo\" src=\"logo.svg\" width=\"26\" /> Title\n\nBody.\n";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(84),
        "markdown_inline_picture_bounds",
        source,
    );
    std::fs::write(
        fixture.workdir.join(logo),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"26\" height=\"26\"><rect width=\"26\" height=\"26\"/></svg>",
    )
    .expect("write the logo the heading points at");
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

    let picture = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_{source_byte}"
        )))
        .expect("the inline picture is drawn");
    assert!(
        picture.size.width > px(0.0) && picture.size.height > px(0.0),
        "the picture must occupy space: {picture:?}"
    );
    let heading = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("the heading text box");
    assert!(
        picture.right() <= heading.left() + px(1.0),
        "the logo sits before the heading text it belongs to; picture={picture:?} text={heading:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn markdown_diff_preview_draws_rows_that_carry_inline_pictures(cx: &mut gpui::TestAppContext) {
    // The diff preview paints a fixed row grid, so a picture written on a line
    // with text has to fit into the line rather than take a block of its own.
    // Its rows still have to draw.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(81);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_diff_inline_images",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/badges.md");
    let old_text = concat!(
        "# <img alt=\"logo\" src=\"logo.svg\" width=\"26\" /> Title\n",
        "\n",
        "[![One](one.svg)](https://a.example) [![Two](two.svg)](https://b.example)\n",
        "\n",
        "Body before.\n",
    );
    let new_text = old_text.replace("Body before.", "Body after.");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some(old_text.to_string()),
                    Some(new_text.clone()),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            });
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active());
    });

    // The heading keeps its text beside the logo, and the badges stay on the
    // line they were written on rather than becoming blocks.
    let document = crate::view::markdown_preview::parse_markdown(&new_text)
        .expect("badge markdown should parse");
    let with_pictures: Vec<&str> = document
        .rows
        .iter()
        .filter(|row| !row.inline_images.is_empty())
        .map(|row| row.text.as_ref())
        .collect();
    assert_eq!(
        with_pictures,
        vec!["Title", ""],
        "rows: {:?}",
        document
            .rows
            .iter()
            .map(|row| row.text.as_ref())
            .collect::<Vec<_>>()
    );

    // And the row grid actually draws them: the pictures are sized against a
    // `flex_none` wrapper, which can collapse without any parse-level
    // assertion noticing.
    let source_bytes = picture_offsets(&document);
    assert!(!source_bytes.is_empty(), "the fixture carries pictures");
    for source_byte in source_bytes {
        let picture = cx
            .debug_bounds(leaked_selector(format!(
                "markdown_preview_inline_image_{source_byte}"
            )))
            .unwrap_or_else(|| panic!("picture at {source_byte} should be drawn"));
        assert!(
            picture.size.width > px(0.0) && picture.size.height > px(0.0),
            "a picture in the diff preview must occupy space: {picture:?}"
        );
    }

    std::fs::remove_dir_all(&workdir).ok();
}

#[gpui::test]
fn copying_across_a_picture_writes_its_description_once(cx: &mut gpui::TestAppContext) {
    // A picture's row carries its alt text, which is what copying a selection
    // across it writes: once. (It was cut into eight bands for a fixed-row
    // renderer, and copy repeated the description per band.)
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(92),
        "markdown_copy_over_picture",
        "Above.\n\n![demo](demo.png)\n\nBelow.\n",
    );
    assert!(
        fixture
            .document
            .rows
            .iter()
            .filter(|row| row.text.as_ref() == "demo")
            .count()
            == 1,
        "the picture is one row"
    );

    let above = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Above.")
        )))
        .expect("expected the paragraph above the picture");
    let below = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Below.")
        )))
        .expect("expected the paragraph below the picture");

    drag_preview_selection(
        cx,
        point(above.left(), above.center().y),
        point(below.right(), below.center().y),
    );

    let copied = copied_preview_selection(cx, &view).expect("the drag should have selected text");
    assert_eq!(
        copied, "Above.\ndemo\nBelow.",
        "a picture is one line of the document"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_picture_draws_at_the_size_its_skeleton_reserved(cx: &mut gpui::TestAppContext) {
    // The other half of `a_skeleton_holds_the_box_the_picture_will_fill`: that
    // one pins the box the skeleton claims from the picture's header, this one
    // pins the box the picture actually lands in. They have to be the same
    // numbers, or reserving the room would just move the jump rather than
    // remove it. The decode itself is too fast to catch mid-flight in a test,
    // so the skeleton is measured through its own unit test instead.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(97),
        "markdown_picture_skeleton",
        "![demo](demo.png)\n\nAfter.\n",
    );
    // Narrower than the pane, so the picture keeps its own size rather than
    // being clamped and the reserved box has to match it exactly.
    std::fs::write(
        fixture.workdir.join("docs/demo.png"),
        test_png_bytes(40, 20).as_slice(),
    )
    .expect("write the picture the document points at");
    let row_ix = fixture.row_ix("demo");
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let picture = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_{row_ix}"
        )))
        .expect("the picture is drawn once it has decoded");
    assert!(
        (picture.size.width - px(40.0)).abs() <= px(0.5)
            && (picture.size.height - px(20.0)).abs() <= px(0.5),
        "a picture narrower than the pane draws at its own size, which is the \
         box its skeleton reserved; got {picture:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn ask_mode_blocks_remote_markdown_images_and_offers_approval_controls(
    cx: &mut gpui::TestAppContext,
) {
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

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(107),
        "markdown_remote_image_approval",
        concat!(
            "<img alt=\"remote\" src=\"https://example.invalid/tracking.png\" ",
            "width=\"120\" height=\"60\" />\n\n",
            "<img alt=\"other\" src=\"https://example.invalid/other.png\" ",
            "width=\"80\" height=\"40\" />\n",
        ),
    );
    let row_ix = fixture.row_ix("remote");
    let retry_selector = leaked_selector(format!("markdown_preview_block_image_load_{row_ix}"));
    let retry_icon_selector = leaked_selector(format!(
        "markdown_preview_block_image_load_{row_ix}_retry_icon"
    ));
    let blocked_icon_selector = leaked_selector(format!(
        "markdown_preview_block_image_load_{row_ix}_blocked_icon"
    ));

    let retry = cx
        .debug_bounds(retry_selector)
        .expect("approval mode should draw a per-image Retry control");
    assert!(
        (retry.size.width - px(120.0)).abs() <= px(0.5)
            && (retry.size.height - px(60.0)).abs() <= px(0.5),
        "the blocked box should preserve the declared image size; got {retry:?}"
    );
    assert!(cx.debug_bounds(retry_icon_selector).is_some());
    assert!(cx.debug_bounds(blocked_icon_selector).is_none());
    assert!(
        cx.debug_bounds("markdown_preview_load_all_remote_images")
            .is_some(),
        "the preview header should offer Load all images"
    );
    assert!(
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_{row_ix}"
        )))
        .is_none(),
        "the remote image element must not be created before approval"
    );

    cx.simulate_click(retry.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            assert_eq!(pane.remote_markdown_images.approved_urls.len(), 1);
            assert!(
                pane.remote_markdown_images
                    .approved_urls
                    .contains("https://example.invalid/tracking.png"),
                "clicking Retry should approve only that image's exact URL"
            );
            assert!(
                !pane
                    .remote_markdown_images
                    .approved_urls
                    .contains("https://example.invalid/other.png")
            );
            pane.set_remote_markdown_image_policy(
                crate::view::RemoteMarkdownImagePolicy::NeverLoad,
                cx,
            );
            assert!(
                pane.remote_markdown_images.approved_urls.is_empty(),
                "changing policy should clear preview-scoped approvals"
            );
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let blocked = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_load_{row_ix}_blocked_box"
        )))
        .expect("never-load mode should draw a blocked-image box");
    assert!(
        (blocked.size.width - px(120.0)).abs() <= px(0.5)
            && (blocked.size.height - px(60.0)).abs() <= px(0.5),
        "the never-load box should preserve the declared image size; got {blocked:?}"
    );
    assert!(cx.debug_bounds(blocked_icon_selector).is_some());
    assert!(cx.debug_bounds(retry_icon_selector).is_none());

    fixture.cleanup();
}

#[gpui::test]
fn markdown_image_access_snapshots_share_approved_url_storage(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let first = pane.markdown_remote_image_access(None);
        let second = pane.markdown_remote_image_access(None);
        assert!(
            Arc::ptr_eq(&first.approved_urls, &second.approved_urls),
            "render snapshots should share approval storage instead of cloning every URL"
        );
    });
}

#[gpui::test]
fn blocked_remote_image_summary_does_not_rescan_unchanged_document(cx: &mut gpui::TestAppContext) {
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

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(109),
        "markdown_remote_image_summary_cache",
        "![remote](https://example.invalid/tracking.png)\n",
    );
    crate::view::panes::main::reset_remote_markdown_image_row_visits_for_tests();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.has_blocked_remote_markdown_images());
        let after_first = crate::view::panes::main::remote_markdown_image_row_visits_for_tests();
        assert!(pane.has_blocked_remote_markdown_images());
        let after_second = crate::view::panes::main::remote_markdown_image_row_visits_for_tests();
        assert_eq!(
            after_second, after_first,
            "an unchanged render must reuse the blocked-image summary"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn markdown_below_eof_drag_selects_an_image_only_document(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(103),
        "markdown_below_eof_image_only",
        "![demo](demo.png)\n",
    );
    std::fs::write(
        fixture.workdir.join("docs/demo.png"),
        test_png_bytes(40, 20).as_slice(),
    )
    .expect("write image-only preview picture");
    let row_ix = fixture.row_ix("demo");
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }

    let picture = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_{row_ix}"
        )))
        .expect("image-only document picture bounds");
    let empty_space = cx
        .debug_bounds("diff_text_empty_space_Inline")
        .expect("image-only document below-EOF surface");
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_text_hitboxes.is_empty(),
            "a block image must not need a synthetic painted-text hitbox"
        );
        assert!(
            !pane.diff_text_motion_targets.is_empty(),
            "the image still needs a logical selection-motion target"
        );
    });

    drag_preview_selection(cx, empty_space.center(), picture.center());
    assert_eq!(
        copied_preview_selection(cx, &view).as_deref(),
        Some("demo"),
        "dragging upward from EOF should copy an image-only document's alt text"
    );

    fixture.cleanup();
}

#[gpui::test]
fn markdown_below_eof_surface_starts_after_a_trailing_picture(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(98),
        "markdown_below_eof_trailing_picture",
        "Before.\n\n![demo](demo.png)\n",
    );
    std::fs::write(
        fixture.workdir.join("docs/demo.png"),
        test_png_bytes(40, 20).as_slice(),
    )
    .expect("write trailing preview picture");
    let row_ix = fixture.row_ix("demo");
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }

    let picture = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_{row_ix}"
        )))
        .expect("trailing picture bounds");
    let empty_space = cx
        .debug_bounds("diff_text_empty_space_Inline")
        .expect("flowing preview below-EOF surface");
    let before = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Before.")
        )))
        .expect("paragraph before the trailing picture");
    assert!(
        empty_space.top() >= picture.bottom(),
        "the EOF surface must begin after the complete picture block; picture={picture:?} surface={empty_space:?}"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.worktree_preview_scroll
                .0
                .borrow()
                .base_handle
                .max_offset()
                .y,
            px(0.0),
            "a short flowing document should not gain vertical scroll range"
        );
    });

    cx.simulate_mouse_down(
        empty_space.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        empty_space.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let last_row_ix = fixture
            .document
            .rows
            .len()
            .checked_sub(1)
            .expect("the preview document should contain picture rows");
        let last_row_len = fixture.document.rows[last_row_ix].text.len();
        assert_eq!(pane.diff_text_anchor, pane.diff_text_head);
        assert_eq!(
            pane.diff_text_head,
            Some(DiffTextPos {
                source_visible_ix: last_row_ix,
                region: DiffTextRegion::Inline,
                offset: last_row_len,
            }),
            "below-EOF selection must end after every row of the trailing picture"
        );
    });

    drag_preview_selection(
        cx,
        empty_space.center(),
        point(before.left(), before.center().y),
    );
    let copied = copied_preview_selection(cx, &view)
        .expect("dragging upward from below EOF should select the document");
    assert!(
        copied.contains("demo"),
        "dragging upward from below EOF must include a trailing picture's alt text: {copied:?}"
    );
    assert_eq!(
        copied.matches("demo").count(),
        1,
        "a multi-row trailing picture should contribute its alt text once: {copied:?}"
    );

    fixture.cleanup();
}

/// A minimal PNG of the given size — only its header is ever read.
fn test_png_bytes(width: u32, height: u32) -> Vec<u8> {
    use image::ImageEncoder as _;
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            &vec![0u8; (width * height * 4) as usize],
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .expect("encode a test png");
    out.into_inner()
}

#[gpui::test]
fn a_picture_that_is_still_decoding_is_waited_on(cx: &mut gpui::TestAppContext) {
    // `gpui` wakes only the first view that asked for an image, so a pane that
    // starts showing one another pane is already decoding is never told the
    // decode finished and holds an empty slot. The pane waits on its own
    // pictures instead of relying on that.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(93),
        "markdown_image_wait",
        "![demo](demo.svg)\n\nAfter.\n",
    );
    // Written after the preview opened, so the first draw resolved nothing and
    // the next one is the one that finds a picture to load.
    std::fs::write(
        fixture.workdir.join("docs/demo.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\"><rect width=\"40\" height=\"20\"/></svg>",
    )
    .expect("write the picture the document points at");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.worktree_markdown.image_waits.is_empty(),
            "a picture that has not decoded yet needs something waiting to repaint the pane"
        );
    });

    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.worktree_markdown.image_waits.is_empty(),
            "and the wait is released once the picture has been decided one way or the other"
        );
    });

    fixture.cleanup();
}

fn diff_scroll_offset_y(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) -> Pixels {
    cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .y
    })
}

#[gpui::test]
fn change_navigation_reaches_an_added_picture_or_rule(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(500.0)));

    let filler: String = (0..80).map(|ix| format!("paragraph {ix:03}\n\n")).collect();
    for (ix, (added, mode)) in [
        ("![pic](missing.png)", DiffViewMode::Inline),
        ("---", DiffViewMode::Inline),
        ("![pic](missing.png)", DiffViewMode::Split),
        ("---", DiffViewMode::Split),
    ]
    .into_iter()
    .enumerate()
    {
        let old_text = format!("Intro.\n\n{filler}End.\n");
        let new_text = format!("Intro.\n\n{filler}{added}\n\nEnd.\n");
        let workdir = open_rendered_markdown_diff_in(
            cx,
            &view,
            // A repo id names one workdir, so each fixture gets its own.
            gitcomet_state::model::RepoId(970 + ix as u64),
            &format!("markdown_nav_to_non_text_block_{ix}"),
            &old_text,
            &new_text,
            mode,
        );
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_scroll
                        .0
                        .borrow()
                        .base_handle
                        .set_offset(point(px(0.0), px(0.0)));
                    pane.markdown_interaction.reveal.clear();
                    let entries = pane.diff_nav_entries();
                    let first = *entries.first().expect("the addition is a change");
                    pane.scroll_diff_to_item_strict(first, gpui::ScrollStrategy::Center);
                    cx.notify();
                });
            });
        });
        for _ in 0..3 {
            draw_and_drain_test_window(cx);
        }
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(
                pane.markdown_interaction.reveal.pending(),
                None,
                "{added:?} in {mode:?}: the reveal is claimed"
            );
        });
        assert!(
            diff_scroll_offset_y(cx, &view) < px(0.0),
            "{added:?} in {mode:?}: the change is below the fold, so revealing it scrolls"
        );
        std::fs::remove_dir_all(&workdir).expect("cleanup");
    }
}

#[gpui::test]
fn searching_a_picture_description_finds_it_once(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8806),
        "markdown_search_picture_once",
        "![unique project logo](missing.png)\n\nBody text.\n",
    );
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("secondary-f");
    draw_frames(cx, 1);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = "unique project logo".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("unique project logo", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });
    let matches = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_search_matches
            .clone()
    });
    assert_eq!(
        matches.len(),
        1,
        "one picture reports one match per band row it was sliced into: {matches:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_frame_of_a_picture_heavy_preview_checks_only_the_pictures_it_draws(
    cx: &mut gpui::TestAppContext,
) {
    // Each local picture is a stat. A frame draws the ones near the viewport;
    // the rest of a long document's pictures are not its business.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let pictures = 300;
    let source: String = (0..pictures)
        .map(|ix| format!("Paragraph {ix} with ![badge {ix}](pics/badge{ix}.png) inline.\n\n"))
        .collect();
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8820),
        "markdown_picture_stats_per_frame",
        &source,
    );
    draw_frames(cx, 3);

    crate::view::rows::take_markdown_image_stats_for_tests();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |_pane, cx| cx.notify());
        });
        let _ = window.draw(app);
    });
    let stats = crate::view::rows::take_markdown_image_stats_for_tests();
    assert!(
        stats < pictures / 3,
        "a redraw checks {stats} of {pictures} pictures on disk"
    );

    fixture.cleanup();
}

#[gpui::test]
fn the_merge_tool_preview_draws_tables_and_local_pictures_like_the_file_preview(
    cx: &mut gpui::TestAppContext,
) {
    // The merge tool drew markdown as fixed-height text rows: tables as padded
    // text and, with nowhere to resolve them from, every local picture as
    // "Image unavailable".
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1400.0), px(800.0)));
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_preview_flowing",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create conflict workdir");
    std::fs::write(
        workdir.join("logo.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\"><rect width=\"40\" height=\"20\"/></svg>",
    )
    .expect("write the picture");

    let ours = "# Local\n\n| Name | Value |\n|------|-------|\n| one | 1 |\n\n![logo](logo.svg)\n";
    open_conflict_markdown_preview(
        cx,
        &view,
        gitcomet_state::model::RepoId(8824),
        &workdir,
        ["# Base\n", ours, "# Remote\n"],
    );
    let (table_row, picture_row) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(document) =
            &pane.conflict_resolver.markdown_preview.documents.ours
        else {
            panic!("the local side is parsed");
        };
        let row = |matches: &dyn Fn(&crate::view::markdown_preview::MarkdownPreviewRow) -> bool| {
            document
                .rows
                .iter()
                .position(matches)
                .expect("the row is in the document")
        };
        (
            row(&|row| row.text.contains("one")),
            row(&|row| row.image.is_some()),
        )
    });
    assert!(
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_cell_box_{table_row}_1"
        )))
        .is_some(),
        "the table is a grid of cells"
    );
    assert!(
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_{picture_row}"
        )))
        .is_some(),
        "the local picture resolves"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}
