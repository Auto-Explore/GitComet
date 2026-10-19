use super::*;
use crate::rendered_preview::RenderedPreviewMode;
use gitcomet_base::Loadable;
use gitcomet_diagrams::scene::{PreparedScene, TILE_SIZE};
use gitcomet_ui_kit::theme::AppTheme;
use gpui::Entity;
use gpui::IntoElement;
use gpui::Render;
use gpui::Window;
use gpui::div;
use gpui::px;
use std::sync::Arc;
use std::time::Duration;

fn render(cache: &DiagramCache, source: &str) -> Arc<RasterDiagram> {
    cache
        .render(DiagramKind::Mermaid, Arc::from(source))
        .unwrap()
}
fn scrolled_of(
    state: &Entity<DiagramState>,
    app: &App,
    axis: gitcomet_ui_kit::ScrollbarAxis,
) -> f32 {
    state.read(app).viewport.borrow().scrolled(axis)
}
fn overflow_of(
    state: &Entity<DiagramState>,
    app: &App,
    axis: gitcomet_ui_kit::ScrollbarAxis,
) -> f32 {
    state.read(app).viewport.borrow().overflow(axis)
}
fn whole(view: &Viewport) -> (std::ops::Range<f32>, std::ops::Range<f32>) {
    (0.0..view.size.0, 0.0..view.size.1)
}
fn tile_key(scene: u64, zoom: u8, dpr: f32, column: u32, row: u32) -> TileKey {
    TileKey {
        scene,
        zoom,
        dpr: dpr.to_bits(),
        column,
        row,
    }
}

#[test]
fn cache_reuses_identical_scenes_and_invalidates_edits() {
    let cache = DiagramCache::default();
    let first = render(&cache, "flowchart LR;A-->B");
    let same = render(&cache, "flowchart LR;A-->B");
    let edit = render(&cache, "flowchart LR;A-->C");
    assert!(Arc::ptr_eq(&first, &same));
    assert_ne!(first.id, edit.id);
    assert_eq!(cache.preparations.load(Ordering::Relaxed), 2);
}
#[test]
fn concurrent_identical_requests_prepare_once_and_share_errors() {
    let cache = Arc::new(DiagramCache::default());
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let threads = (0..8)
        .map(|_| {
            let cache = cache.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                render(&cache, "flowchart LR;A-->B")
            })
        })
        .collect::<Vec<_>>();
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert!(results.iter().all(|scene| Arc::ptr_eq(scene, &results[0])));
    assert_eq!(cache.preparations.load(Ordering::Relaxed), 1);
    for _ in 0..2 {
        assert!(cache.render(DiagramKind::Mermaid, "broken".into()).is_err());
    }
    assert_eq!(cache.preparations.load(Ordering::Relaxed), 2);
}
#[test]
fn tile_cache_is_bounded_reuses_pixels_and_cancels_obsolete_jobs() {
    let cache = DiagramCache::default();
    let scene = render(&cache, "flowchart LR;A[Red]-->B");
    let generation = AtomicU64::new(7);
    let key = tile_key(scene.id, 2, 1.0, 0, 0);
    let first = cache.tile(&scene, key, &generation, 7).unwrap().unwrap();
    let same = cache.tile(&scene, key, &generation, 7).unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &same));
    assert_eq!(cache.rasterizations.load(Ordering::Relaxed), 1);
    assert!(cache.tile(&scene, key, &generation, 6).unwrap().is_none());
    for column in 1..70 {
        cache
            .tile(
                &scene,
                tile_key(scene.id, 2, 1.0, column, 0),
                &generation,
                7,
            )
            .unwrap();
    }
    let inner = cache.inner.lock().unwrap();
    assert_eq!(inner.tiles.len(), 64);
    assert_eq!(inner.dropped.len(), 6);
    assert!(!inner.tiles.contains(&key));
}
#[test]
fn large_scene_tiles_keep_actual_dimensions_and_use_bgra() {
    let scene=PreparedScene::new(Arc::from(&br#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100"><rect width="100000" height="100" fill="red"/></svg>"#[..])).unwrap();
    let diagram = RasterDiagram {
        scene,
        diagnostics: vec![],
        id: 1,
        bytes: 0,
    };
    assert_eq!(diagram.scene.width(), 100000.0);
    let image = DiagramCache::default()
        .tile(&diagram, tile_key(1, 7, 2.0, 30, 0), &AtomicU64::new(0), 0)
        .unwrap()
        .unwrap();
    assert_eq!(&image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
}
#[test]
fn scene_cache_evicts_by_retained_svg_bytes_and_entry_count() {
    let cache = DiagramCache::default();
    let diagram = render(&cache, "flowchart LR;A-->B");
    let weighted = |bytes| {
        Ok(Arc::new(RasterDiagram {
            scene: PreparedScene::new(diagram.scene.svg.clone()).unwrap(),
            id: 1,
            diagnostics: vec![],
            bytes,
        }))
    };
    let mut inner = cache.inner.lock().unwrap();
    inner.entries.clear();
    inner.bytes = 0;
    let key = |s: &str| DiagramKey::new(DiagramKind::Mermaid, Arc::from(s));
    inner.insert(key("old"), weighted(10 * 1024 * 1024));
    inner.insert(key("new"), weighted(10 * 1024 * 1024));
    assert!(!inner.entries.contains(&key("old")));
    assert_eq!(inner.bytes, 10 * 1024 * 1024);
    inner.insert(key("too large"), weighted(MAX_SCENE_BYTES + 1));
    assert!(!inner.entries.contains(&key("too large")));
    for index in 0..40 {
        inner.insert(key(&index.to_string()), Err("invalid".into()));
    }
    assert_eq!(inner.entries.len(), 32);
    assert_eq!(inner.bytes, 0);
}
#[test]
fn file_inputs_enforce_budget_utf8_and_revision_identity() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let input = DiagramInput::File {
        kind: DiagramKind::Mermaid,
        path: file.path().into(),
        revision: "first".into(),
    };
    std::fs::write(file.path(), "flowchart LR;A-->B").unwrap();
    assert_eq!(
        input.read().unwrap().source().as_ref(),
        "flowchart LR;A-->B"
    );
    let other = DiagramInput::File {
        kind: DiagramKind::Mermaid,
        path: file.path().into(),
        revision: "second".into(),
    };
    assert_ne!(input, other);
    std::fs::write(file.path(), [255, 255]).unwrap();
    assert!(input.read().is_err());
    std::fs::write(
        file.path(),
        vec![b'a'; gitcomet_diagrams::MAX_SOURCE_BYTES + 1],
    )
    .unwrap();
    assert!(input.read().unwrap_err().contains("limit"));
    assert!(
        DiagramInput::TooLarge {
            revision: "buffer".into()
        }
        .read()
        .unwrap_err()
        .contains("limit")
    );
}
#[test]
fn viewport_defaults_centers_fitting_graphs_and_pads_overflow() {
    let mut viewport = Viewport::default();
    viewport.resize(600.0, 320.0, (200.0, 100.0));
    assert_eq!(viewport.zoom, 2);
    assert_eq!(viewport.pan, (200.0, 110.0));
    viewport.natural = (100000.0, 10000.0);
    viewport.reset();
    assert_eq!(viewport.pan, (16.0, 16.0));
    assert!(viewport.keys(1, 2.0, whole(&viewport)).len() <= 6);
}
#[test]
fn viewport_zoom_keeps_its_anchor_and_clamps_pan_reset_and_steps() {
    let mut view = Viewport::default();
    view.resize(600.0, 320.0, (1000.0, 1000.0));
    let point = ((300.0 - view.pan.0), 160.0 - view.pan.1);
    view.zoom_by(1, (300.0, 160.0));
    assert_eq!((300.0 - view.pan.0) / 1.25, point.0);
    assert_eq!((160.0 - view.pan.1) / 1.25, point.1);
    view.dragging = Some((0.0, 0.0));
    view.drag_to((1e9, -1e9));
    assert_eq!(view.pan, (16.0, 320.0 - 1282.0 + 16.0));
    for _ in 0..20 {
        view.zoom_by(1, (300.0, 160.0));
    }
    assert_eq!(view.zoom, 7);
    for _ in 0..20 {
        view.zoom_by(-1, (300.0, 160.0));
    }
    assert_eq!(view.zoom, 0);
    view.reset();
    assert_eq!(view.zoom, 2);
    assert_eq!(view.pan, (16.0, 16.0));
    assert!(view.dragging.is_none());
}
/// A full-height block can be far taller than the window; asking for its
/// off-screen tiles would overrun the 64-tile cache and re-raster forever.
#[test]
fn viewport_keys_cover_only_the_visible_part() {
    let mut view = Viewport::default();
    view.resize(600.0, full_height(4000.0), (500.0, 4000.0));
    assert_eq!(view.pan, (50.0, 16.0));
    let rows = |keys: Vec<TileKey>| keys.iter().map(|key| key.row).collect::<Vec<_>>();
    assert_eq!(
        rows(view.keys(1, 1.0, whole(&view))),
        (0..8).collect::<Vec<_>>()
    );
    assert_eq!(
        rows(view.keys(1, 1.0, (0.0..600.0, 1000.0..1300.0))),
        [1, 2]
    );
    assert!(view.keys(1, 1.0, (0.0..600.0, 700.0..700.0)).is_empty());
    assert!(view.keys(1, 1.0, (0.0..0.0, 0.0..300.0)).is_empty());
}
/// A block not drawn yet holds its diagram's real height once the scene is
/// prepared, as the reader collapsed it, with its diagnostics; the collapsed
/// box before that.
#[test]
fn block_estimate_follows_the_prepared_scene() {
    let cache = Arc::new(DiagramCache::default());
    let heights = DiagramHeights {
        cache: cache.clone(),
        choices: BlockChoices::default(),
    };
    let key = |source: &str| DiagramKey::new(DiagramKind::Mermaid, source.into());
    let short = key("flowchart LR;A-->B");
    let tall = key("flowchart TD;A-->B;B-->C;C-->D;D-->E;E-->F");
    let diagnosed = key("%%{init: {\"notASetting\": 1}}%%\nflowchart LR;A-->B");
    let scale = gitcomet_ui_kit::ui_scale::UiScale::from_percent(100);
    let estimate = |key: &DiagramKey| heights.estimate_block(key, scale, 0.0);
    let row = block_control_row(scale);
    assert_eq!(estimate(&short), row + COLLAPSED_HEIGHT);
    assert_eq!(
        DiagramHeights::default().estimate_block(&short, scale, 0.0),
        row + COLLAPSED_HEIGHT
    );

    let height = |key: &DiagramKey| {
        cache
            .render(key.kind(), key.source().clone())
            .unwrap()
            .scene
            .height()
    };
    let (short_height, tall_height) = (height(&short), height(&tall));
    assert_eq!(estimate(&short), row + full_height(short_height));
    assert!(full_height(tall_height) > COLLAPSED_HEIGHT);
    assert_eq!(
        estimate(&tall),
        2.0 * row + full_height(tall_height),
        "a tall block also has its bar"
    );
    heights
        .choices
        .update(&tall, |choice| choice.height = BlockHeight::Collapsed);
    assert_eq!(estimate(&tall), 2.0 * row + COLLAPSED_HEIGHT);
    heights
        .choices
        .update(&tall, |choice| choice.mode = RenderedPreviewMode::Source);
    assert_eq!(
        heights.estimate_block(&tall, scale, 123.0),
        row + 123.0,
        "a block showing its source is its header and the listing"
    );

    let status = f32::from(scale.ui_text(STATUS_LINE) + scale.px(BLOCK_GAP));
    let diagnosed_height = height(&diagnosed);
    assert_eq!(
        estimate(&diagnosed),
        row + full_height(diagnosed_height) + status,
        "one diagnostic line"
    );
}
/// Choices are remembered by diagram source; one back at the default lets its
/// source go.
#[test]
fn a_choice_back_at_the_default_is_forgotten() {
    let choices = BlockChoices::default();
    let key = DiagramKey::new(DiagramKind::Mermaid, "flowchart LR;A-->B".into());
    choices.update(&key, |choice| choice.height = BlockHeight::Collapsed);
    assert_eq!(choices.0.borrow().len(), 1);
    assert!(choices.get(&key).height == BlockHeight::Collapsed);
    choices.update(&key, |choice| choice.height = BlockHeight::Full);
    assert!(choices.0.borrow().is_empty());
}
/// The tile cache holds twice the tiles every live viewport shows, and never
/// shrinks.
#[test]
fn the_tile_cache_grows_to_twice_what_is_shown() {
    let cache = DiagramCache::default();
    let capacity = || cache.inner.lock().unwrap().tiles.cap().get();
    let (a, b) = (cache.viewport_id(), cache.viewport_id());
    cache.show_tiles(a, 30);
    assert_eq!(capacity(), 64, "the floor holds twice 30");
    cache.show_tiles(b, 40);
    assert_eq!(capacity(), 2 * (30 + 40));
    cache.show_tiles(a, 50);
    assert_eq!(capacity(), 2 * (50 + 40));
    cache.show_tiles(a, 0);
    cache.show_tiles(b, 0);
    assert_eq!(capacity(), 2 * (50 + 40));
    assert!(cache.inner.lock().unwrap().shown.is_empty());
}
/// Scrollbars read the pan as a position in the diagram plus the padding a
/// reset leaves around it.
#[test]
fn viewport_scroll_geometry_follows_the_pan() {
    use gitcomet_ui_kit::ScrollbarAxis::{Horizontal, Vertical};
    let mut view = Viewport::default();
    view.resize(600.0, 320.0, (1000.0, 200.0));
    assert_eq!(view.pan, (16.0, 60.0));
    assert_eq!(view.overflow(Horizontal), 1000.0 + 32.0 - 600.0);
    assert_eq!(view.overflow(Vertical), 0.0, "the diagram fits its height");
    assert_eq!(view.scrolled(Horizontal), 0.0);
    view.scroll_to(Horizontal, 100.0);
    assert_eq!(view.pan.0, 16.0 - 100.0);
    assert_eq!(view.scrolled(Horizontal), 100.0);
    view.scroll_to(Horizontal, 1e9);
    assert_eq!(view.scrolled(Horizontal), view.overflow(Horizontal));
    view.zoom_by(3, (0.0, 0.0));
    assert_eq!(view.overflow(Vertical), 200.0 * 2.0 + 32.0 - 320.0);

    // A full-height block that layout rounded half a pixel short still fits.
    let mut view = Viewport::default();
    view.resize(600.0, full_height(400.0) - 0.5, (500.0, 400.0));
    assert_eq!(view.overflow(Vertical), 0.0);
}
/// Dragging and the scrollbars share one pan range: where the diagram
/// overflows it stops where the bar ends, and where it fits it stays inside.
#[test]
fn dragging_stays_within_the_scrollbar_range() {
    use gitcomet_ui_kit::ScrollbarAxis::{Horizontal, Vertical};
    let mut view = Viewport::default();
    view.resize(600.0, 320.0, (1000.0, 200.0));
    view.dragging = Some((0.0, 0.0));
    view.drag_to((1e9, 1e9));
    assert_eq!(view.scrolled(Horizontal), 0.0);
    assert_eq!(
        view.pan,
        (16.0, 320.0 - 200.0 - 16.0),
        "the left edge; the bottom"
    );
    view.drag_to((-1e9, -1e9));
    assert_eq!(view.scrolled(Horizontal), view.overflow(Horizontal));
    assert_eq!(
        view.pan,
        (600.0 - 1000.0 - 16.0, 16.0),
        "the right edge; the top"
    );
    assert_eq!(view.scrolled(Vertical), 0.0);
}
/// A plain wheel pans a diagram sideways where it overflows; a block leaves a
/// vertical wheel to its document, and a wheel at an edge is still taken.
#[test]
fn a_plain_wheel_pans_where_the_diagram_overflows() {
    use gitcomet_ui_kit::ScrollbarAxis::{Horizontal, Vertical};
    let wheel = |dx: f32, dy: f32, shift: bool| gpui::ScrollWheelEvent {
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(dx), px(dy))),
        modifiers: gpui::Modifiers {
            shift,
            ..Default::default()
        },
        ..Default::default()
    };
    let line = px(20.0);
    let mut view = Viewport::default();
    view.resize(600.0, 320.0, (1000.0, 200.0));
    assert!(view.wheel_pan(&wheel(-100.0, 0.0, false), false, line));
    assert_eq!(view.scrolled(Horizontal), 100.0);
    assert!(
        !view.wheel_pan(&wheel(0.0, -100.0, false), false, line),
        "a block leaves a vertical wheel to the document"
    );
    assert!(view.wheel_pan(&wheel(0.0, -50.0, true), false, line));
    assert_eq!(
        view.scrolled(Horizontal),
        150.0,
        "Shift+wheel goes sideways"
    );
    view.scroll_to(Horizontal, 1e9);
    let edge = view.pan;
    assert!(view.wheel_pan(&wheel(-100.0, 0.0, false), false, line));
    assert_eq!(view.pan, edge, "taken at the edge, without moving");
    assert!(
        !view.wheel_pan(&wheel(0.0, -100.0, false), true, line),
        "a pane's vertical wheel pans only where the diagram overflows"
    );
    assert_eq!(view.scrolled(Vertical), 0.0);
    assert!(!view.wheel_pan(
        &gpui::ScrollWheelEvent {
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            ..wheel(-100.0, 0.0, false)
        },
        false,
        line
    ));
}
#[test]
fn pan_zoom_and_dpr_changes_reuse_layout_but_request_new_tiles() {
    let cache = DiagramCache::default();
    let scene = render(&cache, "flowchart LR;A-->B");
    let mut view = Viewport::default();
    view.resize(600.0, 320.0, (scene.scene.width(), scene.scene.height()));
    let generation = AtomicU64::new(0);
    for (delta, dpr) in [(0, 1.0), (1, 1.0), (0, 2.0), (-1, 1.0)] {
        view.zoom_by(delta, (300.0, 160.0));
        for key in view.keys(scene.id, dpr, whole(&view)) {
            cache.tile(&scene, key, &generation, 0).unwrap();
        }
    }
    assert_eq!(cache.preparations.load(Ordering::Relaxed), 1);
    assert!(cache.rasterizations.load(Ordering::Relaxed) > 1);
}
#[test]
fn stale_completion_cannot_replace_current_source_or_scene() {
    let cache = Arc::new(DiagramCache::default());
    let mut state = DiagramState {
        cache,
        viewport_id: 0,
        viewport: Rc::default(),
        hidden_above: 0.0,
        generation: Arc::new(AtomicU64::new(0)),
        source_generation: Arc::new(AtomicU64::new(0)),
        tile_task: None,
        requested: vec![],
        tile_error: None,
        input: DiagramInput::Text {
            kind: DiagramKind::Mermaid,
            text: "current".into(),
        },
        result: Loadable::Loading,
        seq: 2,
        task: None,
        choices: BlockChoices::default(),
    };
    assert!(!state.complete(1, Err("old error".into())));
    assert!(matches!(state.result, Loadable::Loading));
    assert!(state.complete(2, Err("invalid".into())));
    assert!(matches!(&state.result, Loadable::Error(error) if error == "invalid"));
    assert_eq!(
        state.input,
        DiagramInput::Text {
            kind: DiagramKind::Mermaid,
            text: "current".into(),
        }
    );
}
struct Harness {
    key: DiagramKey,
    force_source: bool,
}

struct InteractionHarness {
    state: Entity<DiagramState>,
    scroll: gpui::ScrollHandle,
    key: DiagramKey,
}
impl Render for InteractionHarness {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let preview = self.state.update(cx, |state, cx| {
            state.render(
                AppTheme::gitcomet_dark(),
                None,
                false,
                &DiagramPlacement::Block {
                    key: self.key.clone(),
                    scroll: None,
                },
                cx,
            )
        });
        div()
            .id("diagram_outer_scroll")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(preview)
            .child(div().h(px(1200.0)))
    }
}

/// A drawn diagram inside a scrolling document, ready for pointer input.
fn open_interaction_harness(
    cx: &mut gpui::TestAppContext,
) -> (
    Entity<InteractionHarness>,
    Entity<DiagramState>,
    &mut gpui::VisualTestContext,
) {
    let (view, cx) = cx.add_window_view(|_, cx| {
        let cache = cache(cx);
        InteractionHarness {
            state: cx.new(|cx| {
                DiagramState::new(
                    DiagramInput::Text {
                        kind: DiagramKind::Mermaid,
                        text: "flowchart LR;A-->B".into(),
                    },
                    cache,
                    cx,
                )
            }),
            scroll: gpui::ScrollHandle::new(),
            key: DiagramKey::new(DiagramKind::Mermaid, "flowchart LR;A-->B".into()),
        }
    });
    let state = cx.update(|_, app| view.read(app).state.clone());
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        gitcomet_ui_kit::test_support::redraw(cx);
        if cx.update(|_, app| state.read(app).viewport.borrow().initialized) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    (view, state, cx)
}

#[gpui::test]
fn wheel_scrolls_the_document_and_drag_capture_survives_outside_release(
    cx: &mut gpui::TestAppContext,
) {
    let (view, state, cx) = open_interaction_harness(cx);
    let bounds = cx.debug_bounds("diagram_viewport").unwrap();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-60.0))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    gitcomet_ui_kit::test_support::redraw(cx);
    cx.update(|_, app| {
        assert!(view.read(app).scroll.offset().y < px(0.0));
        assert_eq!(state.read(app).viewport.borrow().zoom, 2);
    });
    let bounds = cx.debug_bounds("diagram_viewport").unwrap();
    let before = cx.update(|_, app| state.read(app).viewport.borrow().pan);
    let outside = bounds.bottom_right() + gpui::point(px(100.0), px(50.0));
    cx.simulate_mouse_down(bounds.center(), gpui::MouseButton::Left, Default::default());
    cx.simulate_mouse_move(outside, Some(gpui::MouseButton::Left), Default::default());
    cx.update(|_, app| {
        assert_ne!(state.read(app).viewport.borrow().pan, before);
        assert!(state.read(app).viewport.borrow().dragging.is_some());
    });
    cx.simulate_mouse_up(outside, gpui::MouseButton::Left, Default::default());
    let released = cx.update(|_, app| {
        assert!(state.read(app).viewport.borrow().dragging.is_none());
        state.read(app).viewport.borrow().pan
    });
    cx.simulate_mouse_move(bounds.center(), None, Default::default());
    cx.update(|_, app| {
        assert_eq!(state.read(app).viewport.borrow().pan, released);
        assert_eq!(
            state.read(app).cache.preparations.load(Ordering::Relaxed),
            1
        );
    });
}

/// A trackpad sends Ctrl/Cmd+scroll as many small pixel deltas; stepping on
/// each one ran a single gesture through every zoom level.
#[gpui::test]
fn smooth_ctrl_scroll_steps_the_diagram_zoom_once_per_40_px(cx: &mut gpui::TestAppContext) {
    let (_view, state, cx) = open_interaction_harness(cx);
    let center = cx.debug_bounds("diagram_viewport").unwrap().center();
    let wheel = |cx: &mut gpui::VisualTestContext, dy: f32| {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: center,
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(dy))),
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            touch_phase: gpui::TouchPhase::Moved,
        });
    };
    let zoom = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| state.read(app).viewport.borrow().zoom)
    };
    for _ in 0..8 {
        wheel(cx, 15.0);
    }
    assert_eq!(zoom(cx), 4, "120 px of smooth scrolling is two steps");
    wheel(cx, -15.0);
    wheel(cx, -15.0);
    assert_eq!(zoom(cx), 4, "reversing drops the unspent distance");
    wheel(cx, -15.0);
    assert_eq!(zoom(cx), 3);
}

struct PaneHarness {
    state: Entity<DiagramState>,
    input: DiagramInput,
}
impl Render for PaneHarness {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let preview = self.state.update(cx, |state, cx| {
            state.render(
                AppTheme::gitcomet_dark(),
                None,
                false,
                &DiagramPlacement::Pane(self.input.clone()),
                cx,
            )
        });
        div().size_full().flex().flex_col().child(preview)
    }
}

/// Rows above and below a viewport are never seen: its own box clips them. A
/// zoomed diagram that overflows its pane asked for one more row on each side,
/// enough for two side-by-side panes to outgrow the tile cache.
#[gpui::test]
fn a_zoomed_pane_requests_only_the_rows_inside_it(cx: &mut gpui::TestAppContext) {
    let input = DiagramInput::Text {
        kind: DiagramKind::Mermaid,
        text: "flowchart TD;A-->B;B-->C;C-->D;D-->E;E-->F;F-->G;G-->H;H-->I;I-->J".into(),
    };
    let (view, cx) = cx.add_window_view(|_, cx| {
        let cache = cache(cx);
        PaneHarness {
            state: cx.new(|cx| DiagramState::new(input.clone(), cache, cx)),
            input: input.clone(),
        }
    });
    let state = cx.update(|_, app| view.read(app).state.clone());
    gitcomet_ui_kit::test_support::redraw(cx);
    let center = cx.debug_bounds("diagram_viewport").unwrap().center();
    for _ in 0..4 {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: center,
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(40.0))),
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            touch_phase: gpui::TouchPhase::Moved,
        });
    }
    gitcomet_ui_kit::test_support::redraw(cx);
    let dpr = cx.update(|window, _| window.scale_factor());
    cx.update(|_, app| {
        let state = state.read(app);
        let viewport = state.viewport.borrow();
        assert_eq!(viewport.zoom, 6);
        let tile = TILE_SIZE as f32 / dpr;
        let scale = ZOOM_STEPS[viewport.zoom as usize];
        assert!(
            viewport.natural.1 * scale > viewport.size.1 + 2.0 * tile,
            "the zoomed diagram overflows the pane by more than a row each way"
        );
        assert!(!state.requested.is_empty());
        for key in &state.requested {
            let top = viewport.pan.1 + key.row as f32 * tile;
            assert!(
                top < viewport.size.1 && top + tile > 0.0,
                "row {} at {top} lies outside the {} px pane",
                key.row,
                viewport.size.1
            );
        }
    });
}

/// Ctrl/Cmd+scroll zooms around the pointer. Zooming around the viewport's
/// centre pushed the point under the pointer away, out of sight on a block
/// taller than the window.
#[gpui::test]
fn ctrl_scroll_keeps_the_point_under_the_pointer(cx: &mut gpui::TestAppContext) {
    let (_view, state, cx) = open_interaction_harness(cx);
    let bounds = cx.debug_bounds("diagram_viewport").unwrap();
    let pointer = bounds.origin + gpui::point(px(30.0), px(20.0));
    let local = (
        f32::from(pointer.x - bounds.origin.x),
        f32::from(pointer.y - bounds.origin.y),
    );
    let under_pointer = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            let viewport = state.read(app).viewport.borrow();
            let scale = ZOOM_STEPS[viewport.zoom as usize];
            (
                (local.0 - viewport.pan.0) / scale,
                (local.1 - viewport.pan.1) / scale,
            )
        })
    };
    let before = under_pointer(cx);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: pointer,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(40.0))),
        modifiers: gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        touch_phase: gpui::TouchPhase::Moved,
    });
    assert_eq!(
        cx.update(|_, app| state.read(app).viewport.borrow().zoom),
        3
    );
    let after = under_pointer(cx);
    assert!(
        (after.0 - before.0).abs() < 0.01 && (after.1 - before.1).abs() < 0.01,
        "the scene point under the pointer moved: {before:?} -> {after:?}"
    );
}

struct SplitHarness {
    sides: [Entity<DiagramState>; 3],
    inputs: [DiagramInput; 3],
}
impl Render for SplitHarness {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let sides = [0, 1, 2].map(|pane| {
            let preview = self.sides[pane].update(cx, |state, cx| {
                state.render(
                    AppTheme::gitcomet_dark(),
                    None,
                    false,
                    &DiagramPlacement::Pane(self.inputs[pane].clone()),
                    cx,
                )
            });
            div()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .flex()
                .flex_col()
                .child(preview)
        });
        div().size_full().flex().children(sides)
    }
}

/// Three large diagrams side by side on a 2560×1440 HiDPI window show more
/// tiles than the 64 the cache held, so each evicted the others' and all
/// re-rastered on every frame, forever.
#[gpui::test]
fn side_by_side_diagrams_settle_on_a_large_screen(cx: &mut gpui::TestAppContext) {
    // A grid of boxes large enough to overflow both panes at 100%.
    let mut source = String::from("flowchart TD\n");
    for row in 0..12 {
        for column in 0..12 {
            if column < 11 {
                source.push_str(&format!("R{row}C{column}-->R{row}C{}\n", column + 1));
            }
            if row < 11 {
                source.push_str(&format!("R{row}C{column}-->R{}C{column}\n", row + 1));
            }
        }
    }
    // A different diagram per pane: equal ones would share their tiles.
    let inputs = [0, 1, 2].map(|pane| DiagramInput::Text {
        kind: DiagramKind::Mermaid,
        text: format!("{source}Pane{pane}\n").into(),
    });
    let (view, cx) = cx.add_window_view(|_, cx| {
        let cache = cache(cx);
        SplitHarness {
            sides: inputs
                .clone()
                .map(|input| cx.new(|cx| DiagramState::new(input, cache.clone(), cx))),
            inputs: inputs.clone(),
        }
    });
    cx.simulate_resize(gpui::size(px(2560.0), px(1440.0)));
    let rasterized = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            app.global::<AppDiagramCache>()
                .0
                .rasterizations
                .load(Ordering::Relaxed)
        })
    };
    for _ in 0..2 {
        gitcomet_ui_kit::test_support::redraw(cx);
    }
    let settled = rasterized(cx);
    let requested = cx.update(|_, app| {
        view.read(app)
            .sides
            .each_ref()
            .map(|side| side.read(app).requested.len())
    });
    assert!(
        requested.iter().sum::<usize>() > 64,
        "the panes show more tiles than the old cache held: {requested:?}"
    );
    for _ in 0..2 {
        gitcomet_ui_kit::test_support::redraw(cx);
    }
    assert_eq!(
        rasterized(cx),
        settled,
        "no tile is drawn twice once settled"
    );
}

/// A diagram wider than its block shows a scrollbar along the bottom, and
/// clicking its far end pans to the diagram's right edge.
#[gpui::test]
fn a_wide_diagram_scrolls_sideways_with_its_scrollbar(cx: &mut gpui::TestAppContext) {
    use gitcomet_ui_kit::ScrollbarAxis::{Horizontal, Vertical};
    let source: String = std::iter::once("flowchart LR".to_string())
        .chain((1..30).map(|node| format!("N{}-->N{node}", node - 1)))
        .collect::<Vec<_>>()
        .join("\n");
    let key = DiagramKey::new(DiagramKind::Mermaid, source.into());
    let (view, cx) = cx.add_window_view(|_, cx| {
        let cache = cache(cx);
        InteractionHarness {
            state: cx.new(|cx| DiagramState::new(DiagramInput::Key(key.clone()), cache, cx)),
            scroll: gpui::ScrollHandle::new(),
            key: key.clone(),
        }
    });
    let state = cx.update(|_, app| view.read(app).state.clone());
    for _ in 0..3 {
        gitcomet_ui_kit::test_support::redraw(cx);
    }
    cx.update(|_, app| {
        let viewport = state.read(app).viewport.borrow();
        assert!(viewport.overflow(Horizontal) > 0.0);
        assert_eq!(viewport.overflow(Vertical), 0.0);
    });
    let bar = cx
        .debug_bounds("diagram_scrollbar_horizontal")
        .expect("a horizontal scrollbar");
    let end = gpui::point(bar.right() - px(6.0), bar.center().y);
    cx.simulate_click(end, gpui::Modifiers::default());
    gitcomet_ui_kit::test_support::redraw(cx);
    cx.update(|_, app| {
        let viewport = state.read(app).viewport.borrow();
        assert_eq!(
            viewport.scrolled(Horizontal),
            viewport.overflow(Horizontal),
            "panned to the right edge"
        );
    });
}

/// A sideways swipe over a diagram wider than its block pans it, and leaves
/// the document where it is.
#[gpui::test]
fn a_sideways_swipe_pans_a_wide_diagram(cx: &mut gpui::TestAppContext) {
    use gitcomet_ui_kit::ScrollbarAxis::Horizontal;
    let source: String = std::iter::once("flowchart LR".to_string())
        .chain((1..30).map(|node| format!("N{}-->N{node}", node - 1)))
        .collect::<Vec<_>>()
        .join("\n");
    let key = DiagramKey::new(DiagramKind::Mermaid, source.into());
    let (view, cx) = cx.add_window_view(|_, cx| {
        let cache = cache(cx);
        InteractionHarness {
            state: cx.new(|cx| DiagramState::new(DiagramInput::Key(key.clone()), cache, cx)),
            scroll: gpui::ScrollHandle::new(),
            key: key.clone(),
        }
    });
    let state = cx.update(|_, app| view.read(app).state.clone());
    for _ in 0..3 {
        gitcomet_ui_kit::test_support::redraw(cx);
    }
    let center = cx.debug_bounds("diagram_viewport").unwrap().center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: center,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(-120.0), px(0.0))),
        modifiers: gpui::Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    gitcomet_ui_kit::test_support::redraw(cx);
    cx.update(|_, app| {
        assert_eq!(scrolled_of(&state, app, Horizontal), 120.0);
        assert_eq!(
            view.read(app).scroll.offset().y,
            px(0.0),
            "the document stays"
        );
    });
}

/// After a reset (Collapse, Show full diagram) a diagram that fits gives its
/// scrollbars nothing to scroll.
#[gpui::test]
fn a_reset_diagram_that_fits_has_nothing_to_scroll(cx: &mut gpui::TestAppContext) {
    use gitcomet_ui_kit::ScrollbarAxis::{Horizontal, Vertical};
    let (view, state, cx) = open_interaction_harness(cx);
    cx.update(|_, app| {
        state.update(app, |state, _| {
            *state.viewport.borrow_mut() = Viewport::default()
        });
        view.update(app, |_, cx| cx.notify());
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_, app| {
        assert_eq!(overflow_of(&state, app, Horizontal), 0.0);
        assert_eq!(overflow_of(&state, app, Vertical), 0.0);
    });
}

/// Collapsing a tall diagram gives it a vertical scrollbar that scrolls it,
/// and no horizontal one.
#[gpui::test]
fn a_collapsed_tall_diagram_scrolls_with_its_vertical_scrollbar(cx: &mut gpui::TestAppContext) {
    use gitcomet_ui_kit::ScrollbarAxis::{Horizontal, Vertical};
    let key = DiagramKey::new(
        DiagramKind::Mermaid,
        "flowchart TD;A-->B;B-->C;C-->D;D-->E;E-->F;F-->G".into(),
    );
    let (view, cx) = cx.add_window_view(|_, cx| {
        let cache = cache(cx);
        InteractionHarness {
            state: cx.new(|cx| DiagramState::new(DiagramInput::Key(key.clone()), cache, cx)),
            scroll: gpui::ScrollHandle::new(),
            key: key.clone(),
        }
    });
    let state = cx.update(|_, app| view.read(app).state.clone());
    for _ in 0..3 {
        gitcomet_ui_kit::test_support::redraw(cx);
    }
    cx.update(|_, app| assert_eq!(overflow_of(&state, app, Vertical), 0.0));
    let toggle = cx.debug_bounds("diagram_block_height_toggle").unwrap();
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    gitcomet_ui_kit::test_support::redraw(cx);
    cx.update(|_, app| {
        assert!(overflow_of(&state, app, Vertical) > 0.0);
        assert_eq!(overflow_of(&state, app, Horizontal), 0.0);
    });
    let bar = cx.debug_bounds("diagram_scrollbar_vertical").unwrap();
    cx.simulate_click(
        gpui::point(bar.center().x, bar.bottom() - px(6.0)),
        gpui::Modifiers::default(),
    );
    cx.update(|_, app| {
        assert_eq!(
            scrolled_of(&state, app, Vertical),
            overflow_of(&state, app, Vertical)
        );
    });
}

/// A diagram that fits until its window narrows then has a scrollbar, and a
/// click at the bar's end pans to the diagram's right edge.
#[gpui::test]
fn narrowing_the_window_gives_a_fitting_diagram_its_scrollbar(cx: &mut gpui::TestAppContext) {
    use gitcomet_ui_kit::ScrollbarAxis::Horizontal;
    let source: String = std::iter::once("flowchart LR".to_string())
        .chain((1..6).map(|node| format!("N{}-->N{node}", node - 1)))
        .collect::<Vec<_>>()
        .join("\n");
    let key = DiagramKey::new(DiagramKind::Mermaid, source.into());
    let (view, cx) = cx.add_window_view(|_, cx| {
        let cache = cache(cx);
        InteractionHarness {
            state: cx.new(|cx| DiagramState::new(DiagramInput::Key(key.clone()), cache, cx)),
            scroll: gpui::ScrollHandle::new(),
            key: key.clone(),
        }
    });
    let state = cx.update(|_, app| view.read(app).state.clone());
    let overflow = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| state.read(app).viewport.borrow().overflow(Horizontal))
    };
    for _ in 0..3 {
        gitcomet_ui_kit::test_support::redraw(cx);
    }
    assert_eq!(overflow(cx), 0.0, "fits the wide window");

    cx.simulate_resize(gpui::size(px(500.0), px(600.0)));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(overflow(cx) > 0.0, "overflows the narrow one");
    let bar = cx
        .debug_bounds("diagram_scrollbar_horizontal")
        .expect("a horizontal scrollbar");
    cx.simulate_click(
        gpui::point(bar.right() - px(6.0), bar.center().y),
        gpui::Modifiers::default(),
    );
    cx.update(|_, app| {
        assert_eq!(
            scrolled_of(&state, app, Horizontal),
            overflow_of(&state, app, Horizontal)
        );
    });
}

/// A block showing its source draws no tiles, so it no longer counts toward
/// the tiles the cache must hold.
#[gpui::test]
fn a_block_showing_its_source_shows_no_tiles(cx: &mut gpui::TestAppContext) {
    let (view, state, cx) = open_interaction_harness(cx);
    gitcomet_ui_kit::test_support::redraw(cx);
    let shown = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            let state = state.read(app);
            let inner = state.cache.inner.lock().unwrap();
            inner.shown.get(&state.viewport_id).copied()
        })
    };
    assert!(shown(cx).is_some_and(|tiles| tiles > 0));
    cx.update(|_, app| {
        let key = view.read(app).key.clone();
        state.update(app, |state, _| {
            state
                .choices
                .update(&key, |choice| choice.mode = RenderedPreviewMode::Source)
        });
        view.update(app, |_, cx| cx.notify());
    });
    gitcomet_ui_kit::test_support::redraw(cx);
    assert_eq!(shown(cx), None);
}

#[test]
fn shared_scenes_keep_independent_preview_viewports() {
    let cache = DiagramCache::default();
    let first = render(&cache, "flowchart LR;A-->B");
    let second = render(&cache, "flowchart LR;A-->B");
    assert!(Arc::ptr_eq(&first, &second));
    let mut a = Viewport::default();
    a.resize(600.0, 320.0, (first.scene.width(), first.scene.height()));
    let b = a.clone();
    a.zoom_by(3, (300.0, 160.0));
    a.dragging = Some((0.0, 0.0));
    a.drag_to((100.0, 50.0));
    assert_eq!(b.zoom, 2);
    assert_ne!(a.pan, b.pan);
    assert!(b.dragging.is_none());
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(DiagramPreview {
            id: "test_diagram".into(),
            theme: AppTheme::gitcomet_dark(),
            source_element: Some(
                div()
                    .debug_selector(|| "diagram_test_source".into())
                    .child("Source fallback")
                    .into_any_element(),
            ),
            force_source: self.force_source,
            placement: DiagramPlacement::Block {
                key: self.key.clone(),
                scroll: None,
            },
        })
    }
}

#[gpui::test]
fn embedded_diagram_uses_preview_code_and_markdown_source_fallback(cx: &mut gpui::TestAppContext) {
    let source: Arc<str> = "flowchart LR\nA-->B".into();
    let (view, cx) = cx.add_window_view(|_, _| Harness {
        key: DiagramKey::new(DiagramKind::Mermaid, source.clone()),
        force_source: false,
    });
    let wait = |cx: &mut gpui::VisualTestContext, source: Arc<str>, success: bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
            let ready = cx.update(|_, app| {
                app.try_global::<AppDiagramCache>().is_some_and(|cache| {
                    cache
                        .0
                        .inner
                        .lock()
                        .unwrap()
                        .entries
                        .peek(&DiagramKey::new(DiagramKind::Mermaid, source.clone()))
                        .is_some_and(|result| result.is_ok() == success)
                })
            });
            if ready && cx.debug_bounds("diagram_loading").is_none() {
                cx.update(|window, app| {
                    let _ = window.draw(app);
                });
                cx.run_until_parked();
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "diagram task did not complete"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    wait(cx, source.clone(), true);
    let bounds = cx.debug_bounds("diagram_viewport").unwrap();
    let facts = cx.update(|_, app| {
        app.global::<AppDiagramCache>()
            .0
            .scene_facts(&DiagramKey::new(DiagramKind::Mermaid, source.clone()))
    });
    let Some(SceneFacts::Drawn {
        height: scene_height,
        ..
    }) = facts
    else {
        panic!("a prepared scene, not {facts:?}");
    };
    // A short diagram takes its own height, with nothing to collapse.
    assert_eq!(f32::from(bounds.size.height), full_height(scene_height));
    assert!(f32::from(bounds.size.height) < COLLAPSED_HEIGHT);
    assert!(cx.debug_bounds("diagram_block_height_toggle").is_none());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(60.0))),
        modifiers: gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        touch_phase: gpui::TouchPhase::Moved,
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        let ready = cx.update(|_, app| {
            let cache = &app.global::<AppDiagramCache>().0;
            assert_eq!(cache.preparations.load(Ordering::Relaxed), 1);
            cached_tiles_for_tests(app)
                .iter()
                .any(|(zoom, _)| *zoom == 3)
        });
        if ready {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "zoom tile did not complete"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.simulate_event(gpui::MouseDownEvent {
        position: bounds.center(),
        button: gpui::MouseButton::Left,
        modifiers: Default::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    assert!(cx.debug_bounds("diagram_preview").is_some());
    assert!(cx.debug_bounds("diagram_test_source").is_none());
    let button = cx.debug_bounds("diagram_diff_view_code").unwrap();
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("diagram_test_source").is_some());
    assert!(cx.debug_bounds("diagram_viewport").is_none());
    let button = cx.debug_bounds("diagram_diff_view_preview").unwrap();
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("diagram_test_source").is_none());
    assert!(cx.debug_bounds("diagram_viewport").is_some());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.force_source = true;
            cx.notify();
        });
    });
    gitcomet_ui_kit::test_support::redraw(cx);
    assert!(cx.debug_bounds("diagram_test_source").is_some());
    assert!(cx.debug_bounds("diagram_viewport").is_none());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.force_source = false;
            cx.notify();
        });
    });
    gitcomet_ui_kit::test_support::redraw(cx);
    assert!(cx.debug_bounds("diagram_test_source").is_none());
    assert!(cx.debug_bounds("diagram_viewport").is_some());
    let invalid: Arc<str> = "invalid diagram".into();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.key = DiagramKey::new(DiagramKind::Mermaid, invalid.clone());
            cx.notify();
        })
    });
    wait(cx, invalid, false);
    assert!(cx.debug_bounds("diagram_test_source").is_some());
}
