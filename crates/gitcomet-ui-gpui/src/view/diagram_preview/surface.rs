use super::*;
use gitcomet_diagrams::scene::TILE_SIZE;
impl DiagramState {
    pub(super) fn invalidate_tiles(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.tile_task = None;
        self.requested.clear();
        self.tile_error = None;
    }

    fn request_tiles(
        &mut self,
        diagram: Arc<RasterDiagram>,
        keys: Vec<TileKey>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.requested == keys
            && (self.tile_task.is_some()
                || self.tile_error.is_some()
                || keys.iter().all(|key| self.cache.peek_tile(*key).is_some()))
        {
            return;
        }
        self.invalidate_tiles();
        self.requested = keys.clone();
        let generation = self.generation.clone();
        let expected = generation.load(Ordering::Relaxed);
        let cache = self.cache.clone();
        if keys.iter().all(|key| cache.peek_tile(*key).is_some()) {
            return;
        }
        self.tile_task = Some(cx.spawn(async move |view, cx| {
            let error = crate::ui_runtime::background_compute(move || {
                for key in keys {
                    if generation.load(Ordering::Relaxed) != expected {
                        break;
                    }
                    match cache.tile(&diagram, key, &generation, expected) {
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                        Err(error) => return Some(error),
                    }
                }
                None
            })
            .await;
            let _ = view.update(cx, |this, cx| {
                if this.generation.load(Ordering::Relaxed) != expected {
                    return;
                }
                this.tile_error = error;
                this.tile_task = None;
                cx.notify();
            });
        }));
    }

    pub(super) fn render_viewport(
        &mut self,
        diagram: Arc<RasterDiagram>,
        theme: AppTheme,
        embedded: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let state = cx.entity();
        let state_for_paint = state.clone();
        let canvas = gpui::canvas(
            move |bounds, window, cx| {
                state.update(cx, |this, cx| {
                    let natural = (diagram.scene.width(), diagram.scene.height());
                    if this.viewport.resize(
                        bounds.size.width.into(),
                        bounds.size.height.into(),
                        natural,
                    ) {
                        this.invalidate_tiles();
                    }
                    let dpr = window.scale_factor();
                    let keys = this.viewport.keys(diagram.id, dpr);
                    this.request_tiles(diagram.clone(), keys.clone(), cx);
                    let pan = this.viewport.pan;
                    keys.into_iter()
                        .filter_map(|key| {
                            let image = this.cache.peek_tile(key)?;
                            let size = TILE_SIZE as f32 / dpr;
                            let origin = bounds.origin
                                + gpui::point(
                                    px(pan.0 + key.column as f32 * size),
                                    px(pan.1 + key.row as f32 * size),
                                );
                            Some((
                                gpui::Bounds::new(origin, gpui::size(px(size), px(size))),
                                image,
                            ))
                        })
                        .collect::<Vec<_>>()
                })
            },
            move |bounds, tiles, window, _cx| {
                for (image_bounds, image) in tiles {
                    let _ = window.paint_image(
                        bounds,
                        image_bounds,
                        gpui::Corners::default(),
                        image,
                        0,
                        false,
                    );
                }
                // Window listeners retain a drag outside the viewport and clear it
                // when the primary button comes up, including outside releases.
                let state = state_for_paint.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                    if phase != gpui::DispatchPhase::Bubble {
                        return;
                    }
                    state.update(cx, |this, cx| {
                        if this.viewport.dragging.is_none() {
                            return;
                        }
                        if !event.dragging() {
                            this.viewport.dragging = None;
                        } else {
                            this.viewport
                                .drag_to((event.position.x.into(), event.position.y.into()));
                            // Invalidate immediately, even if the canvas was virtualized
                            // out between this event and the next frame.
                            this.invalidate_tiles();
                        }
                        cx.notify();
                    });
                });
                let state = state_for_paint.clone();
                window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble
                        && event.button == gpui::MouseButton::Left
                    {
                        state.update(cx, |this, cx| {
                            if this.viewport.dragging.take().is_some() {
                                cx.notify();
                            }
                        });
                    }
                });
            },
        )
        .size_full();
        div()
            .id("diagram_viewport")
            .debug_selector(|| "diagram_viewport".into())
            .w_full()
            .min_w(px(0.0))
            .overflow_hidden()
            .bg(gpui::white())
            .when(embedded, |d| d.h(px(320.0)).flex_shrink_0())
            .when(!embedded, |d| d.flex_1().min_h(px(160.0)))
            .cursor(if self.viewport.dragging.is_some() {
                gpui::CursorStyle::ClosedHand
            } else {
                gpui::CursorStyle::OpenHand
            })
            .gitcomet_tooltip(
                theme,
                "Drag to pan · Ctrl/Cmd+scroll to zoom · Double-click to reset".into(),
            )
            .on_scroll_wheel(
                cx.listener(|this, event: &gpui::ScrollWheelEvent, window, cx| {
                    if !event.modifiers.control && !event.modifiers.platform {
                        return;
                    }
                    let delta = event.delta.pixel_delta(window.line_height()).y;
                    if delta.is_zero() {
                        return;
                    }
                    this.viewport.zoom_by(if delta > px(0.0) { 1 } else { -1 });
                    this.invalidate_tiles();
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        this.viewport.reset();
                        this.invalidate_tiles();
                    } else {
                        this.viewport.dragging =
                            Some((event.position.x.into(), event.position.y.into()));
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(canvas)
            .into_any_element()
    }
}
