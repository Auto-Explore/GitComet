use super::*;
use gitcomet_diagrams::scene::TILE_SIZE;
use gitcomet_ui_kit::theme::AppTheme;
use gitcomet_ui_kit::tooltip::GitCometTooltipExt as _;
use gpui::AnyElement;
use gpui::div;
use gpui::px;
use std::sync::Arc;
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
        self.cache.show_tiles(self.viewport_id, keys.len());
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
            let error = gitcomet_ui_kit::ui_runtime::background_compute(move || {
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

    /// A `height` of `None` fills the pane.
    pub(super) fn render_viewport(
        &mut self,
        diagram: Arc<RasterDiagram>,
        theme: AppTheme,
        height: Option<f32>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let state = cx.entity();
        let state_for_paint = state.clone();
        let canvas = gpui::canvas(
            move |bounds, window, cx| {
                let hitbox = window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal);
                let tiles = state.update(cx, |this, cx| {
                    let natural = (diagram.scene.width(), diagram.scene.height());
                    let resized = this.viewport.borrow_mut().resize(
                        bounds.size.width.into(),
                        bounds.size.height.into(),
                        natural,
                    );
                    if resized {
                        this.invalidate_tiles();
                    }
                    let dpr = window.scale_factor();
                    // The document's clip: the viewport sets none of its own.
                    let mask = window.content_mask().bounds;
                    this.hidden_above = f32::from((mask.top() - bounds.top()).max(px(0.0)));
                    // The box on screen, plus a tile row past the screen's top
                    // and bottom where the box goes on, so the rows a scroll
                    // brings in are already drawn.
                    let margin = px(TILE_SIZE as f32 / dpr);
                    let top = (mask.top() - margin).max(bounds.top());
                    let bottom = (mask.bottom() + margin).min(bounds.bottom());
                    let left = mask.left().max(bounds.left());
                    let right = mask.right().min(bounds.right());
                    let visible = (
                        f32::from(left - bounds.left())..f32::from(right - bounds.left()),
                        f32::from(top - bounds.top())..f32::from(bottom - bounds.top()),
                    );
                    let keys = this.viewport.borrow().keys(diagram.id, dpr, visible);
                    this.request_tiles(diagram.clone(), keys.clone(), cx);
                    let pan = this.viewport.borrow().pan;
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
                });
                (hitbox, tiles)
            },
            move |bounds, (hitbox, tiles), window, _cx| {
                // Clip whole tiles to the box rather than mapping a clipped
                // part of each, which rounds texture coordinates.
                let mask = gpui::ContentMask {
                    bounds,
                    ..Default::default()
                };
                window.with_content_mask(Some(mask), |window| {
                    for (image_bounds, image) in tiles {
                        let _ = window.paint_image(
                            image_bounds,
                            image_bounds,
                            gpui::Corners::default(),
                            0.0,
                            image,
                            0,
                            false,
                        );
                    }
                });
                let state = state_for_paint.clone();
                let origin = hitbox.origin;
                gitcomet_ui_kit::zoom_wheel::on_zoom_wheel(window, hitbox, move |event, _, cx| {
                    let pointer = event.position - origin;
                    let pointer = (pointer.x.into(), pointer.y.into());
                    state.update(cx, |this, cx| {
                        let zoomed = this.viewport.borrow_mut().wheel_zoom(event, pointer);
                        if zoomed {
                            this.invalidate_tiles();
                            cx.notify();
                        }
                    });
                });
                // Window listeners retain a drag outside the viewport and clear it
                // when the primary button comes up, including outside releases.
                let state = state_for_paint.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                    if phase != gpui::DispatchPhase::Bubble {
                        return;
                    }
                    state.update(cx, |this, cx| {
                        let mut viewport = this.viewport.borrow_mut();
                        if viewport.dragging.is_none() {
                            return;
                        }
                        if !event.dragging() {
                            viewport.dragging = None;
                        } else {
                            // The next prepaint asks for the tiles the new pan
                            // shows, as for the wheel and the scrollbars.
                            viewport.drag_to((event.position.x.into(), event.position.y.into()));
                        }
                        drop(viewport);
                        cx.notify();
                    });
                });
                let state = state_for_paint.clone();
                window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble
                        && event.button == gpui::MouseButton::Left
                    {
                        state.update(cx, |this, cx| {
                            if this.viewport.borrow_mut().dragging.take().is_some() {
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
            .bg(gpui::white())
            .map(|d| match height {
                Some(height) => d.h(px(height)).flex_shrink_0(),
                None => d.flex_1().min_h(px(160.0)),
            })
            .cursor(if self.viewport.borrow().dragging.is_some() {
                gpui::CursorStyle::ClosedHand
            } else {
                gpui::CursorStyle::OpenHand
            })
            .gitcomet_tooltip(
                theme,
                "Drag to pan · Ctrl/Cmd+scroll to zoom · Double-click to reset".into(),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        this.viewport.borrow_mut().reset();
                        this.invalidate_tiles();
                    } else {
                        this.viewport.borrow_mut().dragging =
                            Some((event.position.x.into(), event.position.y.into()));
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(
                move |this, event: &gpui::ScrollWheelEvent, window, cx| {
                    // A pane has nothing else to scroll; a markdown block leaves
                    // vertical wheels to its document.
                    let mut viewport = this.viewport.borrow_mut();
                    let before = viewport.pan;
                    let taken = viewport.wheel_pan(event, height.is_none(), window.line_height());
                    let moved = viewport.pan != before;
                    drop(viewport);
                    // The next prepaint asks for the tiles the new pan shows,
                    // and lets a raster still drawing them finish.
                    if moved {
                        cx.notify();
                    }
                    if taken {
                        cx.stop_propagation();
                    }
                },
            ))
            .relative()
            .child(canvas)
            .children(self.scrollbars(cx))
            .into_any_element()
    }

    /// Scrollbars on both axes, which read the shared viewport as this frame
    /// lays it out: where the diagram runs past it, the cue that there is more
    /// and a way to get there; where it fits, nothing.
    fn scrollbars(&self, cx: &mut gpui::Context<Self>) -> Vec<AnyElement> {
        use gitcomet_ui_kit::{Scrollbar, ScrollbarAxis};
        let driver = super::pan_scrollbar::PanScrollDriver {
            viewport: self.viewport.clone(),
            state: cx.weak_entity(),
        };
        // The canvas is white in every theme; the light theme's bars read on it.
        let theme = AppTheme::default_for_window_appearance(gpui::WindowAppearance::Light);
        [ScrollbarAxis::Horizontal, ScrollbarAxis::Vertical]
            .into_iter()
            .map(|axis| {
                let id = match axis {
                    ScrollbarAxis::Horizontal => "diagram_scrollbar_horizontal",
                    ScrollbarAxis::Vertical => "diagram_scrollbar_vertical",
                };
                let bar = match axis {
                    ScrollbarAxis::Horizontal => Scrollbar::horizontal(id, driver.clone()),
                    ScrollbarAxis::Vertical => Scrollbar::new(id, driver.clone()),
                };
                #[cfg(test)]
                let bar = bar.debug_selector(id);
                bar.render(theme).into_any_element()
            })
            .collect()
    }
}
