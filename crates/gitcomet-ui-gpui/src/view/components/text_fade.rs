use crate::theme::with_alpha;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Background, Bounds, Display, Div, Element, ElementId, GlobalElementId,
    InspectorElementId, LayoutId, Length, Rgba, SharedString, Size, Style, Window, div,
    linear_color_stop, px,
};
use std::cell::Cell;
use std::rc::Rc;

/// Default width of the fade ramp.
const FADE_WIDTH_PX: f32 = 16.0;
/// How far past its box the text has to run before it counts as clipped.
/// Layout lands on device pixels, so a text that exactly fills its box can
/// measure a hair wider than it.
const OVERFLOW_SLACK_PX: f32 = 0.5;

/// A single line of text that dissolves into its background where it runs out
/// of room, instead of being cut mid-glyph or trailing an ellipsis. Use it for
/// names and messages whose tail carries no meaning; paths, where the last
/// segment is the part worth reading, want [`super::TruncatedText`] instead.
///
/// The ramp is only painted where the text really is clipped: a name that fits
/// its box — including one in a box that hugs it, like a repository tab sized
/// to its label — is left alone.
///
/// The gradient has to land on the exact color behind the text, so the caller
/// passes the resolved row background — including the hovered one, which is
/// picked up from the row's group.
pub struct FadingText {
    text: AnyElement,
    bg: Rgba,
    hover: Option<(SharedString, Rgba)>,
    overflowing: Rc<Cell<bool>>,
}

impl FadingText {
    pub fn new(text: impl IntoElement, bg: Rgba) -> Self {
        Self {
            text: text.into_any_element(),
            bg,
            hover: None,
            overflowing: Rc::new(Cell::new(false)),
        }
    }

    /// Background to fade into while the named group is hovered.
    pub fn hover_bg(mut self, group: impl Into<SharedString>, bg: Rgba) -> Self {
        self.hover = Some((group.into(), bg));
        self
    }

    pub fn render(self, ui_scale: impl Into<UiScale>) -> Div {
        let ui_scale = ui_scale.into();
        let mut fade = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(ui_scale.px(FADE_WIDTH_PX))
            .bg(fade_gradient(self.bg));
        if let Some((group, hover_bg)) = self.hover {
            fade = fade.group_hover(group, move |s| s.bg(fade_gradient(hover_bg)));
        }

        div()
            .relative()
            .min_w(px(0.0))
            .overflow_hidden()
            .whitespace_nowrap()
            .child(OverflowProbe {
                // The text keeps its natural width in here rather than being
                // squeezed to fit, which is what makes the two widths differ
                // once it no longer fits.
                child: Some(div().flex_shrink_0().child(self.text).into_any_element()),
                overflowing: Rc::clone(&self.overflowing),
            })
            .child(PaintedWhen {
                child: Some(fade.into_any_element()),
                visible: self.overflowing,
            })
    }

    #[cfg(test)]
    fn overflow_flag(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.overflowing)
    }
}

/// Left-to-right ramp from fully transparent to `bg`. Both stops share the
/// same color so the ramp can't drift through grey on its way to invisible.
fn fade_gradient(bg: Rgba) -> Background {
    gpui::linear_gradient(
        90.0,
        linear_color_stop(with_alpha(bg, 0.0), 0.0),
        linear_color_stop(bg, 1.0),
    )
}

/// Standalone version of the same trailing ramp, used when an overlay sits on
/// top of text rather than the text itself overflowing its layout box.
pub fn trailing_fade(bg: Rgba, width: gpui::Pixels) -> Div {
    div().flex_none().w(width).h_full().bg(fade_gradient(bg))
}

struct ProbeLayout {
    child: AnyElement,
    child_layout_id: LayoutId,
}

/// Stretches to the visible box while the text inside keeps its natural width,
/// so comparing the two at prepaint says whether the text is being clipped.
struct OverflowProbe {
    child: Option<AnyElement>,
    overflowing: Rc<Cell<bool>>,
}

impl Element for OverflowProbe {
    type RequestLayoutState = ProbeLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child = self.child.take().expect("fading text probe child");
        let child_layout_id = child.request_layout(window, cx);
        let style = Style {
            display: Display::Flex,
            // Without this the probe inherits the text's width as its own
            // minimum and never reports an overflow.
            min_size: Size {
                width: px(0.0).into(),
                height: Length::Auto,
            },
            ..Default::default()
        };
        let layout_id = window.request_layout(style, [child_layout_id], cx);
        (
            layout_id,
            ProbeLayout {
                child,
                child_layout_id,
            },
        )
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<gpui::Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let text_width = f32::from(window.layout_bounds(layout.child_layout_id).size.width);
        self.overflowing
            .set(text_width > f32::from(bounds.size.width) + OVERFLOW_SLACK_PX);
        layout.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<gpui::Pixels>,
        layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        layout.child.paint(window, cx);
    }
}

impl IntoElement for OverflowProbe {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Draws its child only while the flag is set. The probe beside it sets that
/// flag during prepaint, and siblings prepaint in order, so the fade acts on
/// the measurement taken for the text next to it in the same frame.
struct PaintedWhen {
    child: Option<AnyElement>,
    visible: Rc<Cell<bool>>,
}

impl Element for PaintedWhen {
    type RequestLayoutState = AnyElement;
    type PrepaintState = bool;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child = self.child.take().expect("conditional child");
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<gpui::Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let visible = self.visible.get();
        if visible {
            child.prepaint(window, cx);
        }
        visible
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<gpui::Pixels>,
        child: &mut Self::RequestLayoutState,
        visible: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if *visible {
            child.paint(window, cx);
        }
    }
}

impl IntoElement for PaintedWhen {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgba;

    const LONG_LABEL: &str = "a-very-long-repository-name-that-cannot-possibly-fit";

    /// One case per box the fade has to tell apart: a box that is too narrow
    /// for the text, a box that hugs it (a repository tab at its natural
    /// width), and a box with room to spare.
    struct FadeProbeTestView {
        clipped: Rc<Cell<bool>>,
        hugging: Rc<Cell<bool>>,
        roomy: Rc<Cell<bool>>,
    }

    impl FadeProbeTestView {
        fn new() -> Self {
            Self {
                clipped: Rc::new(Cell::new(false)),
                hugging: Rc::new(Cell::new(false)),
                roomy: Rc::new(Cell::new(false)),
            }
        }
    }

    impl Render for FadeProbeTestView {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let bg = rgba(0x202020ff);
            let label = |text: &'static str, flag: &mut Rc<Cell<bool>>| {
                let fading = FadingText::new(div().text_sm().child(text), bg);
                *flag = fading.overflow_flag();
                fading.render(100u32).flex_1()
            };

            div()
                .flex()
                .flex_col()
                .w(px(600.0))
                // Too narrow for the label: the tail is cut and has to fade.
                .child(
                    div()
                        .flex()
                        .w(px(80.0))
                        .child(label(LONG_LABEL, &mut self.clipped)),
                )
                // Sized to its own content, like a tab at its natural width.
                .child(div().flex().child(label("gitcomet", &mut self.hugging)))
                // Plenty of slack around a short label.
                .child(
                    div()
                        .flex()
                        .w(px(400.0))
                        .child(label("gitcomet", &mut self.roomy)),
                )
        }
    }

    #[gpui::test]
    fn fade_only_marks_text_that_runs_out_of_room(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_window, _cx| FadeProbeTestView::new());

        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });

        cx.update(|_window, app| {
            let view = view.read(app);
            assert!(view.clipped.get(), "clipped label should fade");
            assert!(
                !view.hugging.get(),
                "label in a box sized to it should not fade"
            );
            assert!(!view.roomy.get(), "label with slack should not fade");
        });
    }
}
