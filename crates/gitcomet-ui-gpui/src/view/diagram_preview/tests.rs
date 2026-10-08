use super::*;
use gitcomet_diagrams::scene::PreparedScene;

fn render(cache: &DiagramCache, source: &str) -> Arc<RasterDiagram> {
    cache
        .render(DiagramKind::Mermaid, Arc::from(source))
        .unwrap()
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
    let key = |s: &str| CacheKey::new(DiagramKind::Mermaid, Arc::from(s));
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
    assert_eq!(input.read().unwrap().1.as_ref(), "flowchart LR;A-->B");
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
    assert!(viewport.keys(1, 2.0).len() <= 6);
}
#[test]
fn viewport_zoom_preserves_center_and_clamps_pan_reset_and_steps() {
    let mut view = Viewport::default();
    view.resize(600.0, 320.0, (1000.0, 1000.0));
    let point = ((300.0 - view.pan.0), 160.0 - view.pan.1);
    view.zoom_by(1);
    assert_eq!((300.0 - view.pan.0) / 1.25, point.0);
    assert_eq!((160.0 - view.pan.1) / 1.25, point.1);
    view.dragging = Some((0.0, 0.0));
    view.drag_to((1e9, -1e9));
    assert_eq!(view.pan, (568.0, 32.0 - 1250.0));
    for _ in 0..20 {
        view.zoom_by(1);
    }
    assert_eq!(view.zoom, 7);
    for _ in 0..20 {
        view.zoom_by(-1);
    }
    assert_eq!(view.zoom, 0);
    view.reset();
    assert_eq!(view.zoom, 2);
    assert_eq!(view.pan, (16.0, 16.0));
    assert!(view.dragging.is_none());
}
#[test]
fn pan_zoom_and_dpr_changes_reuse_layout_but_request_new_tiles() {
    let cache = DiagramCache::default();
    let scene = render(&cache, "flowchart LR;A-->B");
    let mut view = Viewport::default();
    view.resize(600.0, 320.0, (scene.scene.width(), scene.scene.height()));
    let generation = AtomicU64::new(0);
    for (delta, dpr) in [(0, 1.0), (1, 1.0), (0, 2.0), (-1, 1.0)] {
        view.zoom_by(delta);
        for key in view.keys(scene.id, dpr) {
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
        viewport: Viewport::default(),
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
        preview_mode: RenderedPreviewMode::Source,
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
    input: DiagramInput,
    force_source: bool,
}

struct InteractionHarness {
    state: Entity<DiagramState>,
    scroll: gpui::ScrollHandle,
}
impl Render for InteractionHarness {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let preview = self.state.update(cx, |state, cx| {
            state.render(AppTheme::gitcomet_dark(), None, false, true, cx)
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

#[gpui::test]
fn wheel_scrolls_the_document_and_drag_capture_survives_outside_release(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
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
        }
    });
    let state = cx.update(|_, app| view.read(app).state.clone());
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        crate::view::test_support::redraw(cx);
        if cx.update(|_, app| state.read(app).viewport.initialized) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let bounds = cx.debug_bounds("diagram_viewport").unwrap();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-60.0))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    crate::view::test_support::redraw(cx);
    cx.update(|_, app| {
        assert!(view.read(app).scroll.offset().y < px(0.0));
        assert_eq!(state.read(app).viewport.zoom, 2);
    });
    let bounds = cx.debug_bounds("diagram_viewport").unwrap();
    let before = cx.update(|_, app| state.read(app).viewport.pan);
    let outside = bounds.bottom_right() + gpui::point(px(100.0), px(50.0));
    cx.simulate_mouse_down(bounds.center(), gpui::MouseButton::Left, Default::default());
    cx.simulate_mouse_move(outside, Some(gpui::MouseButton::Left), Default::default());
    cx.update(|_, app| {
        assert_ne!(state.read(app).viewport.pan, before);
        assert!(state.read(app).viewport.dragging.is_some());
    });
    cx.simulate_mouse_up(outside, gpui::MouseButton::Left, Default::default());
    let released = cx.update(|_, app| {
        assert!(state.read(app).viewport.dragging.is_none());
        state.read(app).viewport.pan
    });
    cx.simulate_mouse_move(bounds.center(), None, Default::default());
    cx.update(|_, app| {
        assert_eq!(state.read(app).viewport.pan, released);
        assert_eq!(
            state.read(app).cache.preparations.load(Ordering::Relaxed),
            1
        );
    });
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
    a.zoom_by(3);
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
            input: self.input.clone(),
            theme: AppTheme::gitcomet_dark(),
            source_element: Some(
                div()
                    .debug_selector(|| "diagram_test_source".into())
                    .child("Source fallback")
                    .into_any_element(),
            ),
            force_source: self.force_source,
            embedded: true,
        })
    }
}

#[gpui::test]
fn embedded_diagram_uses_preview_code_and_markdown_source_fallback(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let source: Arc<str> = "flowchart LR\nA-->B".into();
    let (view, cx) = cx.add_window_view(|_, _| Harness {
        input: DiagramInput::Text {
            kind: DiagramKind::Mermaid,
            text: source.clone(),
        },
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
                        .peek(&CacheKey::new(DiagramKind::Mermaid, source.clone()))
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
    assert_eq!(f32::from(bounds.size.height), 320.0);
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
            cache
                .inner
                .lock()
                .unwrap()
                .tiles
                .iter()
                .any(|(key, _)| key.zoom == 3)
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
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("diagram_test_source").is_some());
    assert!(cx.debug_bounds("diagram_viewport").is_none());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.force_source = false;
            cx.notify();
        });
    });
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("diagram_test_source").is_none());
    assert!(cx.debug_bounds("diagram_viewport").is_some());
    let invalid: Arc<str> = "invalid diagram".into();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.input = DiagramInput::Text {
                kind: DiagramKind::Mermaid,
                text: invalid.clone(),
            };
            cx.notify();
        })
    });
    wait(cx, invalid, false);
    assert!(cx.debug_bounds("diagram_test_source").is_some());
}
