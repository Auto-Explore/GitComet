use super::*;

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
        cx: &mut App,
    ) {
        if !self.view.read(cx).diff_text_selecting {
            return;
        }

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

        let selection = self.view.read(cx).diff_text_local_selection_range(
            self.visible_ix,
            self.region,
            self.text.len(),
        );

        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());

        let mut hasher = rustc_hash::FxHasher::default();
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

        let hitbox = DiffTextHitbox {
            bounds,
            layout_key,
            text_len: self.text.len(),
            streamed_ascii_monospace_cell_width: None,
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
