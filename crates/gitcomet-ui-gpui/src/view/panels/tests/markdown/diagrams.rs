//! Mermaid blocks embedded in the rendered markdown preview.

use super::*;

/// Mermaid source for `nodes` boxes stacked downwards.
fn flowchart_source(nodes: usize) -> String {
    let mut source = String::from("flowchart TD\n");
    for node in 1..nodes {
        source.push_str(&format!("N{}-->N{node}\n", node - 1));
    }
    source
}

/// A document around one Mermaid flowchart of `nodes` boxes stacked downwards.
fn stacked_flowchart(nodes: usize) -> String {
    format!(
        "# Title\n\n```mermaid\n{}```\n\nAfter\n",
        flowchart_source(nodes)
    )
}

fn open_diagram_preview<'a>(
    cx: &'a mut gpui::TestAppContext,
    repo_id: u64,
    name: &str,
    source: &str,
) -> (
    gpui::Entity<super::super::super::GitCometView>,
    &'a mut gpui::VisualTestContext,
    RenderedPreviewFixture,
) {
    open_diagram_preview_with(cx, |_| {}, repo_id, name, source)
}

/// As `open_diagram_preview`, after `setup` runs on the app before the window
/// opens (where appearance pins have to happen).
fn open_diagram_preview_with<'a>(
    cx: &'a mut gpui::TestAppContext,
    setup: impl FnOnce(&mut App),
    repo_id: u64,
    name: &str,
    source: &str,
) -> (
    gpui::Entity<super::super::super::GitCometView>,
    &'a mut gpui::VisualTestContext,
    RenderedPreviewFixture,
) {
    cx.update(setup);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(repo_id),
        name,
        source,
    );
    for _ in 0..20 {
        draw_frames(cx, 1);
        if cx.debug_bounds("diagram_viewport").is_some() {
            return (view, cx, fixture);
        }
    }
    panic!("the embedded diagram never finished rendering");
}

fn zoomed_in(app: &App) -> bool {
    crate::view::diagram_preview::cached_tiles_for_tests(app)
        .iter()
        .any(|(zoom, _)| *zoom == 3)
}

/// The window zoom listens in the capture phase and is painted last, so a
/// bubble-phase diagram listener never saw Ctrl/Cmd+scroll: the whole window
/// zoomed instead of the diagram.
#[gpui::test]
fn ctrl_scroll_over_an_embedded_diagram_zooms_the_diagram_not_the_window(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx, fixture) = open_diagram_preview(
        cx,
        9701,
        "markdown_diagram_ctrl_scroll",
        &stacked_flowchart(4),
    );
    let bounds = cx.debug_bounds("diagram_viewport").unwrap();
    let window_zoom = cx.update(|_, app| view.read(app).ui_scale_percent);
    cx.update(|_, app| {
        assert!(!zoomed_in(app), "no tile is zoomed in before the wheel");
    });

    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(60.0))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..Default::default()
    });
    draw_frames(cx, 2);

    cx.update(|_, app| {
        assert_eq!(
            view.read(app).ui_scale_percent,
            window_zoom,
            "Ctrl/Cmd+scroll over the diagram must not zoom the window"
        );
        assert!(
            zoomed_in(app),
            "Ctrl/Cmd+scroll over the diagram zooms it one step"
        );
    });

    // Outside the diagram the wheel still zooms the window.
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: point(px(5.0), px(5.0)),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(60.0))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..Default::default()
    });
    draw_frames(cx, 1);
    cx.update(|_, app| assert!(view.read(app).ui_scale_percent > window_zoom));

    fixture.cleanup();
}

fn viewport_height(cx: &mut gpui::VisualTestContext) -> f32 {
    f32::from(cx.debug_bounds("diagram_viewport").unwrap().size.height)
}

/// The height the document reserves for the block before drawing it matches
/// the block once drawn.
fn assert_estimate_matches_drawn_block(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    source: &str,
) {
    let block = f32::from(cx.debug_bounds("diagram_preview").unwrap().size.height);
    let estimate = cx.update(|_, app| {
        crate::view::diagram_preview::block::DiagramHeights::new(app).estimate_block(
            &gitcomet_diagrams::source::DiagramKey::new(
                gitcomet_diagrams::DiagramKind::Mermaid,
                source.into(),
            ),
            crate::ui_scale::UiScale::from_percent(view.read(app).ui_scale_percent)
                .with_appearance(view.read(app).theme.metrics),
            0.0,
        )
    });
    assert!(
        (block - estimate).abs() <= 1.0,
        "estimated {estimate} for a block drawn {block} tall"
    );
}

/// A tall diagram shows whole at 100%, and the bar under it folds it into the
/// fixed viewport and back.
#[gpui::test]
fn a_tall_embedded_diagram_shows_in_full_and_its_bar_collapses_it(cx: &mut gpui::TestAppContext) {
    let (view, cx, fixture) = open_diagram_preview(
        cx,
        9702,
        "markdown_diagram_full_height",
        &stacked_flowchart(7),
    );
    let full = viewport_height(cx);
    assert_estimate_matches_drawn_block(cx, &view, &flowchart_source(7));
    let viewport = cx.debug_bounds("diagram_viewport").unwrap();
    let bar = cx
        .debug_bounds("diagram_block_height_toggle")
        .expect("a diagram taller than the collapsed viewport offers the bar");
    assert!(full > 400.0, "seven stacked boxes outgrow 320 px: {full}");
    assert!(
        bar.top() >= viewport.bottom(),
        "the bar sits under the diagram"
    );

    // Zooming pans inside the block; its height stays.
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: viewport.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(60.0))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..Default::default()
    });
    draw_frames(cx, 2);
    cx.update(|_, app| assert!(zoomed_in(app)));
    assert_eq!(viewport_height(cx), full);

    cx.simulate_click(bar.center(), Modifiers::default());
    draw_frames(cx, 2);
    assert_eq!(viewport_height(cx), 320.0);
    let bar = cx
        .debug_bounds("diagram_block_height_toggle")
        .expect("a collapsed diagram keeps the bar");
    cx.simulate_click(bar.center(), Modifiers::default());
    draw_frames(cx, 2);
    assert_eq!(viewport_height(cx), full);

    fixture.cleanup();
}

/// A diagram that fits the collapsed viewport takes only its own height, and
/// there is nothing to collapse.
#[gpui::test]
fn a_short_embedded_diagram_takes_its_own_height_without_a_bar(cx: &mut gpui::TestAppContext) {
    let (view, cx, fixture) = open_diagram_preview(
        cx,
        9703,
        "markdown_diagram_short",
        "```mermaid\nflowchart LR\nA-->B\n```\n",
    );
    let height = viewport_height(cx);
    assert!(height < 200.0, "one row of boxes plus padding: {height}");
    assert_estimate_matches_drawn_block(cx, &view, "flowchart LR\nA-->B\n");
    assert!(cx.debug_bounds("diagram_block_height_toggle").is_none());

    // The header grows with the window zoom; the diagram keeps its size.
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: point(px(5.0), px(5.0)),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(60.0))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..Default::default()
    });
    draw_frames(cx, 3);
    assert!(cx.update(|_, app| view.read(app).ui_scale_percent) > 100);
    assert_estimate_matches_drawn_block(cx, &view, "flowchart LR\nA-->B\n");
    fixture.cleanup();
}

/// A full-height diagram can run far past the window; only the tiles on
/// screen, and a row either side, are drawn, and scrolling the document brings
/// in the next ones.
#[gpui::test]
fn a_full_height_diagram_rasterizes_only_the_tiles_on_screen(cx: &mut gpui::TestAppContext) {
    let (_view, cx, fixture) = open_diagram_preview(
        cx,
        9704,
        "markdown_diagram_visible_tiles",
        &stacked_flowchart(40),
    );
    let viewport = cx.debug_bounds("diagram_viewport").unwrap();
    let dpr = cx.update(|window, _| window.scale_factor());
    let tile = 512.0 / dpr;
    let rows = |cx: &mut gpui::VisualTestContext| {
        let mut rows: Vec<u32> = cx.update(|_, app| {
            crate::view::diagram_preview::cached_tiles_for_tests(app)
                .into_iter()
                .map(|(_, row)| row)
                .collect()
        });
        rows.sort_unstable();
        rows.dedup();
        rows
    };
    // The document clips at its scroll container's bottom edge.
    let shown = f32::from(
        cx.debug_bounds("worktree_markdown_preview_document")
            .unwrap()
            .bottom()
            - viewport.top(),
    );
    let total = ((viewport_height(cx) - 32.0) / tile).ceil() as u32;
    let first_hidden = ((shown - 16.0) / tile).ceil() as u32;
    let before = rows(cx);
    assert_eq!(
        before,
        (0..=first_hidden).collect::<Vec<_>>(),
        "the rows on screen and one more below"
    );
    assert!(total > first_hidden + 1, "the diagram runs past the window");

    // The viewport runs past the window, so its center is off screen.
    let on_diagram = viewport.origin + point(px(100.0), px(100.0));
    cx.simulate_mouse_move(on_diagram, None, Modifiers::default());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: on_diagram,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-1500.0))),
        ..Default::default()
    });
    draw_frames(cx, 3);
    assert!(
        cx.debug_bounds("diagram_viewport").unwrap().top() < viewport.top() - px(1000.0),
        "a plain wheel over the diagram scrolls the document"
    );
    let after = rows(cx);
    assert!(
        after.last() > before.last(),
        "scrolling brings later rows in: {before:?} -> {after:?}"
    );

    fixture.cleanup();
}

/// Collapsing from the bar under a diagram whose top has scrolled away would
/// fold it out of sight and leave the reader in the text after it.
#[gpui::test]
fn collapsing_a_diagram_scrolled_past_its_top_keeps_it_in_view(cx: &mut gpui::TestAppContext) {
    let (_view, cx, fixture) = open_diagram_preview(
        cx,
        9705,
        "markdown_diagram_collapse_in_view",
        &(stacked_flowchart(7) + &"\nMore text.\n".repeat(60)),
    );
    let document = cx
        .debug_bounds("worktree_markdown_preview_document")
        .unwrap();
    let on_diagram = cx.debug_bounds("diagram_viewport").unwrap().center();
    cx.simulate_mouse_move(on_diagram, None, Modifiers::default());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: on_diagram,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-250.0))),
        ..Default::default()
    });
    draw_frames(cx, 2);
    let viewport = cx.debug_bounds("diagram_viewport").unwrap();
    assert!(
        viewport.top() < document.top(),
        "the diagram's top is out of view"
    );

    let bar = cx.debug_bounds("diagram_block_height_toggle").unwrap();
    cx.simulate_click(bar.center(), Modifiers::default());
    draw_frames(cx, 3);
    let viewport = cx.debug_bounds("diagram_viewport").unwrap();
    assert_eq!(f32::from(viewport.size.height), 320.0);
    assert!(
        viewport.top() >= document.top() - px(1.0),
        "the collapsed diagram starts in view: {viewport:?} in {document:?}"
    );

    fixture.cleanup();
}

/// Wheel the document (not a diagram) by `dy` pixels, then draw.
fn scroll_document(cx: &mut gpui::VisualTestContext, dy: f32) {
    let heading = cx
        .debug_bounds("markdown_preview_text_box_0")
        .unwrap()
        .center();
    cx.simulate_mouse_move(heading, None, Modifiers::default());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: heading,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(dy))),
        ..Default::default()
    });
    draw_frames(cx, 2);
}

fn paragraphs(count: usize) -> String {
    (0..count)
        .map(|ix| format!("Paragraph {ix}.\n\n"))
        .collect()
}

/// A diagram the document builds ahead of the scroll position, but further
/// below the screen than a tile row, has nothing to draw yet. The viewport
/// clipped its own box, so the window seemed to end at its top edge and its
/// first tile row was always asked for.
#[gpui::test]
fn a_diagram_well_below_the_screen_draws_no_tiles(cx: &mut gpui::TestAppContext) {
    let source = format!(
        "# Title\n\n{}{}",
        paragraphs(45),
        &stacked_flowchart(7)[9..]
    );
    let (_view, cx, fixture) =
        open_diagram_preview(cx, 9706, "markdown_diagram_below_screen", &source);
    let document = cx
        .debug_bounds("worktree_markdown_preview_document")
        .unwrap();
    let viewport = cx.debug_bounds("diagram_viewport").unwrap();
    let tile = 512.0 / cx.update(|window, _| window.scale_factor());
    assert!(
        viewport.top() > document.bottom() + px(tile),
        "the diagram starts more than a tile row below the screen: {viewport:?} {document:?}"
    );
    cx.update(|_, app| {
        assert_eq!(
            crate::view::diagram_preview::cached_tiles_for_tests(app),
            Vec::new()
        )
    });
    fixture.cleanup();
}

/// Collapsing is the reader's choice for that diagram: it survives the block
/// being rebuilt, as it is when an edit above it moves its rows.
#[gpui::test]
fn a_collapsed_diagram_stays_collapsed_when_its_block_is_rebuilt(cx: &mut gpui::TestAppContext) {
    let (view, cx, fixture) = open_diagram_preview(
        cx,
        9707,
        "markdown_diagram_collapse_sticks",
        &stacked_flowchart(7),
    );
    let bar = cx.debug_bounds("diagram_block_height_toggle").unwrap();
    cx.simulate_click(bar.center(), Modifiers::default());
    draw_frames(cx, 2);
    assert_eq!(viewport_height(cx), 320.0);
    assert_estimate_matches_drawn_block(cx, &view, &flowchart_source(7));

    let edited = format!("Inserted above.\n\n{}", stacked_flowchart(7));
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.worktree_markdown.document = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(&edited).unwrap(),
                ));
                cx.notify();
            });
    });
    draw_frames(cx, 3);
    assert_eq!(viewport_height(cx), 320.0, "still collapsed after the edit");
    fixture.cleanup();
}

/// The guess for an undrawn block matches the drawn block at every density and
/// UI font, and counts the diagram's diagnostic lines.
#[gpui::test]
fn the_block_estimate_matches_every_density_and_diagnostics(cx: &mut gpui::TestAppContext) {
    let diagnosed = "%%{init: {\"notASetting\": 1}}%%\nflowchart LR\nA-->B\n";
    let (view, cx, fixture) = open_diagram_preview(
        cx,
        9708,
        "markdown_diagram_diagnostics",
        &format!("```mermaid\n{diagnosed}```\n"),
    );
    assert!(
        cx.update(|_, app| crate::view::diagram_preview::cached_diagnostics_for_tests(app)) > 0,
        "the fixture produces a diagnostic"
    );
    assert_estimate_matches_drawn_block(cx, &view, diagnosed);
    fixture.cleanup();
}

#[gpui::test]
fn the_block_estimate_matches_compact_density(cx: &mut gpui::TestAppContext) {
    estimate_matches_at_density(cx, crate::appearance::UiDensity::Compact, None, 9709);
}

#[gpui::test]
fn the_block_estimate_matches_spacious_density_and_a_large_ui_font(cx: &mut gpui::TestAppContext) {
    estimate_matches_at_density(cx, crate::appearance::UiDensity::Spacious, Some(18), 9710);
}

fn estimate_matches_at_density(
    cx: &mut gpui::TestAppContext,
    density: crate::appearance::UiDensity,
    ui_font_size_px: Option<u32>,
    repo_id: u64,
) {
    let (view, cx, fixture) = open_diagram_preview_with(
        cx,
        |app| {
            crate::appearance::pin_density_for_test(app, density);
            if let Some(size) = ui_font_size_px {
                let mut appearance = crate::appearance::current(app);
                appearance.set_size(crate::appearance::FontRole::Ui, size);
                app.set_global(appearance);
            }
        },
        repo_id,
        "markdown_diagram_density",
        &stacked_flowchart(7),
    );
    assert_estimate_matches_drawn_block(cx, &view, &flowchart_source(7));
    fixture.cleanup();
}

/// Collapsing from the bar while the whole diagram is above the screen used to
/// fold it to just above the top edge, leaving only the bar in sight.
#[gpui::test]
fn collapsing_a_diagram_wholly_above_the_screen_brings_it_into_view(cx: &mut gpui::TestAppContext) {
    let (_view, cx, fixture) = open_diagram_preview(
        cx,
        9711,
        "markdown_diagram_collapse_from_above",
        &(stacked_flowchart(7) + &paragraphs(60)),
    );
    let document = cx
        .debug_bounds("worktree_markdown_preview_document")
        .unwrap();
    let bar = cx.debug_bounds("diagram_block_height_toggle").unwrap();
    // The bar's top 4 px past the document's top edge; the viewport above it.
    scroll_document(cx, -f32::from(bar.top() - document.top() - px(4.0)));
    let viewport = cx.debug_bounds("diagram_viewport").unwrap();
    assert!(
        viewport.bottom() < document.top(),
        "the diagram is above the screen"
    );

    let bar = cx.debug_bounds("diagram_block_height_toggle").unwrap();
    cx.simulate_click(bar.center(), Modifiers::default());
    draw_frames(cx, 3);
    let viewport = cx.debug_bounds("diagram_viewport").unwrap();
    assert_eq!(f32::from(viewport.size.height), 320.0);
    assert!(
        viewport.top() >= document.top() - px(1.0),
        "the collapsed diagram starts in view: {viewport:?} in {document:?}"
    );
    fixture.cleanup();
}

/// Sizing undrawn diagram blocks must not hash their sources on every frame.
#[gpui::test]
fn undrawn_diagram_blocks_are_sized_without_hashing_each_frame(cx: &mut gpui::TestAppContext) {
    let source = format!(
        "# Title\n\n{}{}",
        paragraphs(400),
        &stacked_flowchart(7)[9..]
    );
    let (_view, cx, fixture) = open_diagram_preview_without_drawing_diagram(
        cx,
        9712,
        "markdown_diagram_estimate_hashing",
        &source,
    );
    gitcomet_diagrams::source::take_sources_hashed_for_tests();
    for _ in 0..3 {
        scroll_document(cx, -20.0);
    }
    assert_eq!(
        gitcomet_diagrams::source::take_sources_hashed_for_tests(),
        0
    );
    fixture.cleanup();
}

/// Opens a document whose diagram lies beyond what the first frames build.
fn open_diagram_preview_without_drawing_diagram<'a>(
    cx: &'a mut gpui::TestAppContext,
    repo_id: u64,
    name: &str,
    source: &str,
) -> (
    gpui::Entity<super::super::super::GitCometView>,
    &'a mut gpui::VisualTestContext,
    RenderedPreviewFixture,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(repo_id),
        name,
        source,
    );
    draw_frames(cx, 2);
    assert!(cx.debug_bounds("diagram_preview").is_none());
    (view, cx, fixture)
}

/// Code view is the reader's choice for that diagram too: an edit above the
/// block used to flip it back to Preview while a collapse survived.
#[gpui::test]
fn a_diagrams_code_view_stays_when_its_block_is_rebuilt(cx: &mut gpui::TestAppContext) {
    let (view, cx, fixture) = open_diagram_preview(
        cx,
        9713,
        "markdown_diagram_code_sticks",
        &stacked_flowchart(4),
    );
    let code = cx.debug_bounds("diagram_diff_view_code").unwrap();
    cx.simulate_click(code.center(), Modifiers::default());
    draw_frames(cx, 2);
    assert!(cx.debug_bounds("diagram_viewport").is_none());

    let edited = format!("Inserted above.\n\n{}", stacked_flowchart(4));
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.worktree_markdown.document = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(&edited).unwrap(),
                ));
                cx.notify();
            });
    });
    draw_frames(cx, 3);
    assert!(cx.debug_bounds("diagram_preview").is_some());
    assert!(
        cx.debug_bounds("diagram_viewport").is_none(),
        "still showing the code after the edit"
    );
    fixture.cleanup();
}

/// A rebuilt block whose diagram is already prepared draws it in its first
/// frame. It used to show "Rendering diagram…" for a frame, so the document
/// measured a short block and jumped when the diagram arrived.
#[gpui::test]
fn a_prepared_diagram_draws_in_the_first_frame_of_its_block(cx: &mut gpui::TestAppContext) {
    let (view, cx, fixture) = open_diagram_preview(
        cx,
        9714,
        "markdown_diagram_first_frame",
        &stacked_flowchart(4),
    );
    let edited = format!("Inserted above.\n\n{}", stacked_flowchart(4));
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.worktree_markdown.document = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(&edited).unwrap(),
                ));
                cx.notify();
            });
    });
    // One frame, with no background work allowed to finish first.
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("diagram_loading").is_none());
    assert!(cx.debug_bounds("diagram_viewport").is_some());
    fixture.cleanup();
}

/// A diagram that fails to render draws as its error and its source listing,
/// and is sized so before drawing: the same as that listing in a plain code
/// block, plus the header and the error line. It used to be the 320 px box.
#[gpui::test]
fn a_failed_diagram_is_sized_as_its_error_and_source(cx: &mut gpui::TestAppContext) {
    let broken: String = std::iter::once("flowchart TD".to_string())
        .chain((0..30).map(|row| format!("A{row} -->")))
        .collect::<Vec<_>>()
        .join("\n");
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(9718),
        "markdown_diagram_failed",
        &format!("```mermaid\n{broken}\n```\n\n```text\n{broken}\n```\n"),
    );
    for _ in 0..20 {
        draw_frames(cx, 1);
        if cx.debug_bounds("diagram_error").is_some() {
            break;
        }
    }
    assert!(
        cx.debug_bounds("diagram_error").is_some(),
        "the fixture fails"
    );
    let (document, theme, percent) = cx.update(|_, app| {
        let view = view.read(app);
        let document = match &view.diff_renderer.read(app).worktree_markdown.document {
            gitcomet_state::model::Loadable::Ready(document) => document.clone(),
            other => panic!("a parsed document, not {other:?}"),
        };
        (document, view.theme, view.ui_scale_percent)
    });
    let blocks = crate::view::markdown_preview::markdown_document_blocks(&document);
    let estimate = |cx: &mut gpui::VisualTestContext, block: usize| {
        cx.update(|_, app| {
            crate::view::rows::markdown_block_estimate_for_tests(
                &document, block, theme, percent, app,
            )
        })
    };
    let (diagram_estimate, listing_estimate) = (estimate(cx, 0), estimate(cx, 1));
    let diagram = f32::from(cx.debug_bounds("diagram_preview").unwrap().size.height);
    let listing = f32::from(
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_code_shell_{}",
            blocks[1].row_range().start
        )))
        .unwrap()
        .size
        .height,
    );
    assert!(
        ((diagram_estimate - listing_estimate) - (diagram - listing)).abs() <= 1.0,
        "the diagram adds {} over its listing, estimated {}",
        diagram - listing,
        diagram_estimate - listing_estimate
    );
    fixture.cleanup();
}
