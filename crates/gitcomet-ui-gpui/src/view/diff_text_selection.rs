use super::*;
use rustc_hash::FxHasher;

fn diff_text_empty_space_surface(
    view: Entity<MainPaneView>,
    region: DiffTextRegion,
) -> gpui::Stateful<gpui::Div> {
    let left_view = view.clone();
    let right_view = view;
    div()
        .id(("diff_text_empty_space", usize::from(region.order())))
        .debug_selector(move || format!("diff_text_empty_space_{region:?}"))
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            crate::press_gesture::claim_press(cx);
            cx.stop_propagation();
            let focus = left_view.read(cx).diff_panel_focus_handle.clone();
            window.focus(&focus, cx);
            left_view.update(cx, |this, cx| {
                this.handle_diff_text_empty_space_mouse_down(region, event.position, cx);
                cx.notify();
            });
        })
        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
            crate::press_gesture::claim_press(cx);
            cx.stop_propagation();
            let focus = right_view.read(cx).diff_panel_focus_handle.clone();
            window.focus(&focus, cx);
            right_view.update(cx, |this, cx| {
                this.open_diff_editor_context_menu_at_eof(region, event.position, window, cx);
                cx.notify();
            });
        })
}

fn diff_text_trailing_space_height(
    viewport_height: Pixels,
    scroll_y: Pixels,
    item_height: Pixels,
    item_count: usize,
) -> Pixels {
    (viewport_height - scroll_y - item_height * item_count).max(px(0.0))
}

/// Adds the interactive text area after the final item of a short virtualized
/// document. Decorations are laid out from the list's scrolled content origin,
/// so the surface counteracts horizontal scrolling and is clipped by the list's
/// own content mask and padding.
pub(super) struct DiffTextEmptySpaceDecoration {
    pub(super) view: Entity<MainPaneView>,
    pub(super) region: DiffTextRegion,
}

impl gpui::UniformListDecoration for DiffTextEmptySpaceDecoration {
    fn compute(
        &self,
        _visible_range: Range<usize>,
        bounds: Bounds<Pixels>,
        scroll_offset: Point<Pixels>,
        item_height: Pixels,
        item_count: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        let trailing_height = diff_text_trailing_space_height(
            bounds.size.height,
            scroll_offset.y,
            item_height,
            item_count,
        );
        if trailing_height <= px(0.0) {
            return div().into_any_element();
        }

        div()
            .relative()
            .size_full()
            .child(
                diff_text_empty_space_surface(self.view.clone(), self.region)
                    .absolute()
                    .left(-scroll_offset.x)
                    .top(item_height * item_count)
                    .w(bounds.size.width)
                    .h(trailing_height),
            )
            .into_any_element()
    }
}

/// Fills the remainder of a naturally laid-out document column. The caller
/// places this immediately after the complete body, so pictures and other
/// non-text blocks remain outside the hit area.
pub(super) fn flowing_diff_text_empty_space(
    view: Entity<MainPaneView>,
    region: DiffTextRegion,
) -> AnyElement {
    diff_text_empty_space_surface(view, region)
        .w_full()
        .min_h(px(0.0))
        .flex_1()
        .into_any_element()
}

/// A zero-length source has no row from which a uniform-list decoration could
/// be measured. Keep its empty-state presentation, but make the document
/// viewport itself behave as the sole EOF position.
pub(super) fn empty_diff_text_document(
    view: Entity<MainPaneView>,
    region: DiffTextRegion,
    child: AnyElement,
) -> AnyElement {
    div()
        .relative()
        // This wrapper is also used as one side of the split Markdown
        // preview. It must participate in that row's flex sizing instead of
        // advertising a 100%-wide flex basis and squeezing out the document
        // on the other side.
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .min_h(px(0.0))
        .child(child)
        .child(
            diff_text_empty_space_surface(view, region)
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
        .into_any_element()
}

pub(super) struct DiffTextSelectionTracker {
    pub(super) view: Entity<MainPaneView>,
}

impl IntoElement for DiffTextSelectionTracker {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for DiffTextSelectionTracker {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = px(0.0).into();
        style.size.height = px(0.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let view_for_move = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != gpui::DispatchPhase::Bubble {
                return;
            }
            view_for_move.update(cx, |this, cx| {
                if !this.diff_text_selecting {
                    return;
                }
                let before = this.diff_text_head;
                this.update_diff_text_selection_from_mouse(event.position);
                if this.diff_text_head != before {
                    cx.notify();
                }
            });
        });

        let view_for_up = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != gpui::DispatchPhase::Bubble {
                return;
            }
            if event.button != MouseButton::Left {
                return;
            }
            view_for_up.update(cx, |this, cx| {
                if this.diff_text_selecting {
                    this.end_diff_text_selection();
                    cx.notify();
                }
            });
        });
    }
}

#[cfg(test)]
mod empty_space_tests {
    use super::*;

    #[test]
    fn trailing_space_is_only_the_unused_viewport_height() {
        assert_eq!(
            diff_text_trailing_space_height(px(400.0), px(0.0), px(20.0), 2),
            px(360.0)
        );
        assert_eq!(
            diff_text_trailing_space_height(px(400.0), px(0.0), px(20.0), 20),
            px(0.0)
        );
        assert_eq!(
            diff_text_trailing_space_height(px(400.0), px(-600.0), px(20.0), 50),
            px(0.0)
        );
    }
}

/// section 30 split: zero-size element that ends a conflict row-drag on mouse-up
/// anywhere in the window (per-row handlers cover extend inside the columns).
pub(super) struct ConflictRowSelectionTracker {
    pub(super) view: Entity<MainPaneView>,
}

impl IntoElement for ConflictRowSelectionTracker {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ConflictRowSelectionTracker {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = px(0.0).into();
        style.size.height = px(0.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let selecting = self
            .view
            .read(cx)
            .conflict_resolver
            .row_selection
            .is_some_and(|selection| selection.selecting);
        if !selecting {
            return;
        }

        let view_for_up = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != gpui::DispatchPhase::Bubble {
                return;
            }
            if event.button != MouseButton::Left {
                return;
            }
            view_for_up.update(cx, |this, cx| {
                this.conflict_resolver_end_row_selection(cx);
            });
        });
    }
}

pub(super) struct DiffTextSelectionOverlay {
    pub(super) view: Entity<MainPaneView>,
    pub(super) visible_ix: usize,
    pub(super) region: DiffTextRegion,
    pub(super) text: SharedString,
}

impl IntoElement for DiffTextSelectionOverlay {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for DiffTextSelectionOverlay {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        use std::hash::{Hash, Hasher};

        let selection = self
            .view
            .read(cx)
            .diff_text_local_selection_range(self.visible_ix, self.region);

        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());

        let mut hasher = FxHasher::default();
        self.text.as_ref().hash(&mut hasher);
        font_size.hash(&mut hasher);
        let layout_key = hasher.finish();

        let (x0, x1, shaped) = match self.view.read(cx).diff_text_layout_cache.get(&layout_key) {
            Some(entry) => {
                let layout = &entry.layout;
                let x0 = selection
                    .as_ref()
                    .map(|r| layout.x_for_index(r.start.min(self.text.len())));
                let x1 = selection
                    .as_ref()
                    .map(|r| layout.x_for_index(r.end.min(self.text.len())));
                (x0, x1, None)
            }
            None => {
                let run = TextRun {
                    len: self.text.len(),
                    font: style.font(),
                    color: style.color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                    letter_spacing: style.letter_spacing,
                };
                let layout =
                    window
                        .text_system()
                        .shape_line(self.text.clone(), font_size, &[run], None);
                let x0 = selection
                    .as_ref()
                    .map(|r| layout.x_for_index(r.start.min(self.text.len())));
                let x1 = selection
                    .as_ref()
                    .map(|r| layout.x_for_index(r.end.min(self.text.len())));
                (x0, x1, Some(layout))
            }
        };

        if let (Some(x0), Some(x1)) = (x0, x1)
            && x1 > x0
        {
            let color = self.view.read(cx).diff_text_selection_color();
            window.paint_quad(fill(
                Bounds::from_corners(
                    point(bounds.left() + x0, bounds.top()),
                    point(bounds.left() + x1, bounds.bottom()),
                ),
                color,
            ));
        }

        let (source_visible_ix, visual_range) = self
            .view
            .read(cx)
            .diff_text_visual_source_range_for_region(self.visible_ix, self.region);
        let hitbox = DiffTextHitbox {
            bounds,
            layout_key,
            source_visible_ix,
            text_start_offset: visual_range.start,
            text_len: self.text.len(),
            offset_map: None,
            painted_text: self.text.clone(),
            streamed_ascii_monospace_cell_width: None,
            wrapped: None,
        };

        let visible_ix = self.visible_ix;
        let region = self.region;
        let view = self.view.clone();
        view.update(cx, |this, _cx| {
            this.set_diff_text_hitbox(visible_ix, region, hitbox);
            this.touch_diff_text_layout_cache(layout_key, shaped);
        });
    }
}
