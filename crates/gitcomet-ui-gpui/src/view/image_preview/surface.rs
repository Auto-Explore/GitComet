use super::*;

fn visible_keys(
    source: &Source,
    frame: usize,
    camera: Camera,
    view: (f64, f64),
    prefs: Preferences,
) -> Vec<TileKey> {
    let output = (
        (f64::from(source.dimensions.0) * camera.scale)
            .round()
            .max(1.0) as u32,
        (f64::from(source.dimensions.1) * camera.scale)
            .round()
            .max(1.0) as u32,
    );
    let origin = camera.origin(view);
    let left = (-origin.0).max(0.0).min(f64::from(output.0)) as u32;
    let top = (-origin.1).max(0.0).min(f64::from(output.1)) as u32;
    let right = (view.0 - origin.0).ceil().max(0.0).min(f64::from(output.0)) as u32;
    let bottom = (view.1 - origin.1).ceil().max(0.0).min(f64::from(output.1)) as u32;
    if right <= left || bottom <= top {
        return Vec::new();
    }
    // Integer output dimensions define the actual scale seen on screen.
    let scale = (f64::from(output.0) / f64::from(source.dimensions.0))
        .min(f64::from(output.1) / f64::from(source.dimensions.1));
    let sampling = if source.vector {
        filter::Sampling::Nearest
    } else {
        filter::sampling(prefs.mode, prefs.filter, scale)
    };
    let mut keys = Vec::new();
    for row in top / resample::TILE..bottom.div_ceil(resample::TILE) {
        for column in left / resample::TILE..right.div_ceil(resample::TILE) {
            keys.push(TileKey {
                source: source.id,
                frame,
                output,
                sampling,
                column,
                row,
            });
        }
    }
    keys
}

impl Group {
    fn request(
        &mut self,
        index: usize,
        source: Arc<Source>,
        keys: Vec<TileKey>,
        cx: &mut gpui::Context<Self>,
    ) {
        let side = &mut self.sides[index];
        if side.requested == keys
            && (side.task.is_some()
                || side.error.is_some()
                || keys.iter().all(|k| self.cache.peek(k).is_some()))
        {
            return;
        }
        side.cancel();
        side.requested = keys.clone();
        if keys.iter().all(|key| self.cache.peek(key).is_some()) {
            return;
        }
        let generation = side.generation.clone();
        let expected = generation.load(Ordering::Relaxed);
        let cache = self.cache.clone();
        side.task = Some(cx.spawn(async move |group, cx| {
            let result = crate::ui_runtime::background_compute(move || {
                cache.render(&source, &keys, &generation, expected)
            })
            .await;
            let _ = group.update(cx, |this, cx| {
                let side = &mut this.sides[index];
                if side.generation.load(Ordering::Relaxed) != expected {
                    return;
                }
                side.task = None;
                side.error = result.err();
                this.cache.drain(cx);
                cx.notify();
            });
        }));
    }

    pub(super) fn render_viewport(
        &mut self,
        index: usize,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let state = cx.entity();
        let painted_state = state.clone();
        let canvas = gpui::canvas(
            move |bounds, window, cx| {
                state.update(cx, |this, cx| {
                    this.cache.drain(cx);
                    let dpr = window.scale_factor();
                    let view = (
                        f64::from(f32::from(bounds.size.width) * dpr),
                        f64::from(f32::from(bounds.size.height) * dpr),
                    );
                    let resized = this.sides[index].device_size != view;
                    let before = this.sides[index].camera;
                    this.sides[index].device_size = view;
                    this.layout(index);
                    let after = this.sides[index].camera;
                    if resized || (before.fit, before.scale) != (after.fit, after.scale) {
                        let state = cx.entity().downgrade();
                        window.on_next_frame(move |_, cx| {
                            let _ = state.update(cx, |_, cx| cx.notify());
                        });
                    }
                    let input = this.sides[index].input.clone();
                    let camera = this.sides[index].camera;
                    let Some(source) = input.source else {
                        return (None, Vec::new());
                    };
                    let hitbox = window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal);
                    let prefs = Preferences::current(cx);
                    let keys = visible_keys(&source, input.frame, camera, view, prefs);
                    // Reduced motion or an inactive window holds the frame still.
                    let playing = !this.paused
                        && input.preview.as_ref().is_some_and(|i| i.frame_count() > 1)
                        && animates(window, cx);
                    if !playing {
                        // Very large displays can exceed the entire tile budget.
                        // Keep detail nearest the viewport center and use the bounded
                        // preview elsewhere, instead of continuously evicting and
                        // rebuilding the other comparison pane's visible tiles.
                        let count = this
                            .sides
                            .iter()
                            .filter(|s| s.input.source.is_some())
                            .count()
                            .max(1);
                        let mut detailed = keys.clone();
                        if detailed.len() > cache::TILE_COUNT / count {
                            let origin = camera.origin(view);
                            detailed.sort_by(|a, b| {
                                let distance = |key: &TileKey| {
                                    let x =
                                        f64::from(key.column * resample::TILE) + 256.0 + origin.0
                                            - view.0 / 2.0;
                                    let y = f64::from(key.row * resample::TILE) + 256.0 + origin.1
                                        - view.1 / 2.0;
                                    x * x + y * y
                                };
                                distance(a).total_cmp(&distance(b))
                            });
                            detailed.truncate(cache::TILE_COUNT / count);
                        }
                        this.request(index, source.clone(), detailed, cx);
                    }
                    let origin = camera.origin(view);
                    let origin = gpui::point(
                        px((f32::from(bounds.origin.x) * dpr).round() / dpr
                            + origin.0 as f32 / dpr),
                        px((f32::from(bounds.origin.y) * dpr).round() / dpr
                            + origin.1 as f32 / dpr),
                    );
                    let mut draws = Vec::new();
                    for key in keys {
                        let tile_origin = origin
                            + gpui::point(
                                px((key.column * resample::TILE) as f32 / dpr),
                                px((key.row * resample::TILE) as f32 / dpr),
                            );
                        let tile_size = gpui::size(
                            px(
                                resample::TILE.min(key.output.0 - key.column * resample::TILE)
                                    as f32
                                    / dpr,
                            ),
                            px(
                                resample::TILE.min(key.output.1 - key.row * resample::TILE) as f32
                                    / dpr,
                            ),
                        );
                        let tile_bounds = Bounds::new(tile_origin, tile_size);
                        if !playing && let Some(image) = this.cache.peek(&key) {
                            draws.push((bounds.intersect(&tile_bounds), tile_bounds, image, 0));
                        } else if let Some(image) = input.preview.as_ref() {
                            // Clip the temporary preview to missing tiles so translucent
                            // pixels never composite on top of a second copy of themselves.
                            let image_bounds = Bounds::new(
                                origin,
                                gpui::size(
                                    px(key.output.0 as f32 / dpr),
                                    px(key.output.1 as f32 / dpr),
                                ),
                            );
                            draws.push((
                                bounds.intersect(&tile_bounds),
                                image_bounds,
                                image.clone(),
                                input.frame,
                            ));
                        }
                    }
                    (Some(hitbox), draws)
                })
            },
            move |_, (hitbox, draws), window, _cx| {
                for (clip, bounds, image, frame) in draws {
                    // paint_image rounds clipped texture coordinates to integer
                    // texels. Keep the full mapping and clip only the content.
                    window.with_content_mask(
                        Some(gpui::ContentMask {
                            bounds: clip,
                            ..Default::default()
                        }),
                        |window| {
                            let _ = window.paint_image(
                                bounds,
                                bounds,
                                gpui::Corners::default(),
                                image,
                                frame,
                                false,
                            );
                        },
                    );
                }
                if let Some(hitbox) = hitbox {
                    let state = painted_state.clone();
                    // Capture phase: ahead of the window zoom, which is painted last.
                    // Anything drawn over the image takes the hit test, and the
                    // window zoom then applies there as it does elsewhere.
                    window.on_mouse_event(
                        move |event: &gpui::ScrollWheelEvent, phase, window, cx| {
                            if phase != gpui::DispatchPhase::Capture
                                || !(event.modifiers.secondary() || event.modifiers.control)
                                || !hitbox.should_handle_scroll(window)
                            {
                                return;
                            }
                            let pointer = event.position - hitbox.origin;
                            let dpr = f64::from(window.scale_factor());
                            state.update(cx, |this, cx| {
                                this.wheel_zoom(index, event, pointer, dpr, cx)
                            });
                            cx.stop_propagation();
                        },
                    );
                }
                let state = painted_state.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Bubble {
                        return;
                    }
                    state.update(cx, |this, cx| {
                        let Some((side, previous)) = this.dragging else {
                            return;
                        };
                        if side != index {
                            return;
                        }
                        if !event.dragging() {
                            this.dragging = None;
                            return;
                        }
                        let dpr = f64::from(window.scale_factor());
                        let delta = event.position - previous;
                        this.dragging = Some((index, event.position));
                        this.pan(
                            index,
                            (
                                f64::from(f32::from(delta.x)) * dpr,
                                f64::from(f32::from(delta.y)) * dpr,
                            ),
                            cx,
                        );
                    });
                });
                let state = painted_state.clone();
                window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble
                        && event.button == gpui::MouseButton::Left
                    {
                        state.update(cx, |this, cx| {
                            if this.dragging.take().is_some() {
                                cx.notify();
                            }
                        });
                    }
                });
            },
        )
        .size_full();
        div()
            .id(("image_viewport", index))
            .debug_selector(|| "image_viewport".into())
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .track_focus(&self.sides[index].focus)
            .tab_index(0)
            .cursor(if self.dragging.is_some() {
                gpui::CursorStyle::ClosedHand
            } else {
                gpui::CursorStyle::OpenHand
            })
            .gitcomet_tooltip(
                theme,
                "Drag or scroll to pan · Ctrl/Cmd+scroll to zoom · Double-click to fit".into(),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.active = index;
                    window.focus(&this.sides[index].focus, cx);
                    if event.click_count == 2 {
                        this.zoom(Zoom::Fit, None, cx);
                    } else {
                        this.dragging = Some((index, event.position));
                        cx.notify();
                    }
                    cx.stop_propagation();
                }),
            )
            .on_scroll_wheel(cx.listener(
                move |this, event: &gpui::ScrollWheelEvent, window, cx| {
                    // Zooming is the capture-phase listener's.
                    if event.modifiers.secondary() || event.modifiers.control {
                        return;
                    }
                    let delta = event.delta.pixel_delta(window.line_height());
                    let dpr = f64::from(window.scale_factor());
                    this.pan(
                        index,
                        (
                            f64::from(f32::from(delta.x)) * dpr,
                            f64::from(f32::from(delta.y)) * dpr,
                        ),
                        cx,
                    );
                    cx.stop_propagation();
                },
            ))
            .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                let modifiers = &event.keystroke.modifiers;
                if modifiers.control || modifiers.platform || modifiers.alt {
                    return;
                }
                let zoom = match event.keystroke.key.as_str() {
                    "+" | "=" => Zoom::In,
                    "-" => Zoom::Out,
                    "0" => Zoom::Fit,
                    "1" => Zoom::Scale(1.0),
                    _ => return,
                };
                this.active = index;
                this.zoom(zoom, None, cx);
                cx.stop_propagation();
            }))
            .child(canvas)
            .when_some(self.sides[index].error.clone(), |d, error| {
                d.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .p_1()
                        .bg(theme.colors.surface.raised)
                        .text_color(theme.colors.status.warning.foreground)
                        .child(format!("Detailed preview unavailable: {error}")),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_scale_is_independent_of_dpr_and_only_visible_tiles_are_requested() {
        let source = super::super::tests::source(1200, 800);
        let camera = Camera {
            fit: false,
            scale: 1.0,
            center: (600.0, 400.0),
        };
        for dpr in [1.0, 1.5, 2.0] {
            let keys = visible_keys(
                &source,
                0,
                camera,
                (400.0 * dpr, 300.0 * dpr),
                Preferences::default(),
            );
            assert!(keys.len() <= 6);
            assert!(keys.iter().all(|k| k.output == (1200, 800)));
        }
        let camera = Camera {
            scale: 64.0,
            ..camera
        };
        let keys = visible_keys(&source, 0, camera, (400.0, 300.0), Preferences::default());
        assert!(keys.len() <= 4);
        assert!(keys.iter().all(|k| k.sampling == filter::Sampling::Nearest));
    }
}
