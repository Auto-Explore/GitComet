//! A segmented control: a row of mutually exclusive options on one track,
//! the selected one raised. Each segment may carry an icon beside its label,
//! and a tooltip naming its keyboard shortcuts.

use super::{ControlActivation, ControlInteractionExt, InteractionState, InteractionStyle};
use crate::icons::svg_icon;
use crate::theme::AppTheme;
use crate::tooltip::GitCometTooltipExt as _;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{ClickEvent, Div, SharedString, Stateful, Window, div, px};

/// One option of a [`SegmentedControl`].
pub struct Segment {
    id: SharedString,
    label: SharedString,
    icon: Option<SharedString>,
    selected: bool,
    tooltip: Option<(SharedString, Vec<SharedString>)>,
}

impl Segment {
    /// `id` is the element id and debug selector.
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            selected: false,
            tooltip: None,
        }
    }

    /// An icon asset path, drawn before the label.
    pub fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// A tooltip naming the option and its keyboard shortcuts.
    pub fn tooltip(mut self, label: impl Into<SharedString>, shortcuts: Vec<SharedString>) -> Self {
        self.tooltip = Some((label.into(), shortcuts));
        self
    }
}

/// Mutually exclusive options on one track. Built per render, like
/// [`Button`](crate::components::Button).
pub struct SegmentedControl {
    id: SharedString,
    segments: Vec<Segment>,
}

impl SegmentedControl {
    /// `id` is the track's element id and debug selector.
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            segments: Vec::new(),
        }
    }

    pub fn segment(mut self, segment: Segment) -> Self {
        self.segments.push(segment);
        self
    }

    /// The control, calling `on_select` with a segment's index when it is
    /// clicked, the selected one included.
    pub fn render<V: 'static>(
        self,
        theme: AppTheme,
        scale: UiScale,
        cx: &mut gpui::Context<V>,
        on_select: impl Fn(&mut V, usize, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Stateful<Div> {
        let on_select = std::rc::Rc::new(on_select);
        let raised = if theme.is_dark {
            theme.colors.surface.raised
        } else {
            theme.colors.surface.canvas
        };
        let track_id = self.id.clone();
        let mut track = div()
            .id(self.id)
            .debug_selector(move || track_id.to_string())
            .flex()
            .items_center()
            .h(super::control_height(scale))
            .p(scale.px(2.0))
            .gap(scale.px(2.0))
            .rounded(px(theme.radii.row))
            .bg(theme.hover_overlay());
        for (index, segment) in self.segments.into_iter().enumerate() {
            let Segment {
                id,
                label,
                icon,
                selected,
                tooltip,
            } = segment;
            let text = if selected {
                theme.colors.foreground.primary
            } else {
                theme.colors.foreground.secondary
            };
            let selector = id.clone();
            let on_select = on_select.clone();
            // The raised fill and its border mark the choice, so the light
            // theme's selection ring would only repeat it.
            let style = InteractionStyle::header(theme).selection_outline(false);
            let mut button = div()
                .id(id)
                .debug_selector(move || selector.to_string())
                .h_full()
                .flex()
                .items_center()
                .gap(scale.px(5.0))
                .px(scale.px(8.0))
                .rounded(px((theme.radii.row - 2.0).max(2.0)))
                .border_1()
                .border_color(if selected {
                    theme.colors.stroke.subtle
                } else {
                    gpui::rgba(0x00000000)
                })
                .text_size(theme.ui_text(12.0))
                .text_color(text)
                .control_interaction(
                    style,
                    InteractionState::default().selected(selected, raised),
                )
                .on_activate(
                    false,
                    ControlActivation::Action,
                    cx.listener(move |view, _: &ClickEvent, window, cx| {
                        on_select(view, index, window, cx);
                    }),
                )
                .children(icon.map(|icon| svg_icon(icon, text, scale.px(14.0))))
                .child(label);
            if let Some((label, shortcuts)) = tooltip {
                button = button.gitcomet_tooltip_keyed(theme, label, shortcuts);
            }
            track = track.child(button);
        }
        track
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Render;

    struct Picker {
        theme: AppTheme,
        selected: usize,
    }

    impl Render for Picker {
        fn render(
            &mut self,
            _window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let selected = self.selected;
            div().size_full().child(
                SegmentedControl::new("picker")
                    .segment(
                        Segment::new("picker_inline", "Inline")
                            .icon("icons/diff_inline.svg")
                            .selected(selected == 0),
                    )
                    .segment(Segment::new("picker_split", "Split").selected(selected == 1))
                    .render(
                        self.theme,
                        UiScale::current(cx),
                        cx,
                        |this, index, _, cx| {
                            this.selected = index;
                            cx.notify();
                        },
                    ),
            )
        }
    }

    /// A click reports the segment's index, and only the selected segment
    /// is raised.
    #[gpui::test]
    fn a_click_selects_its_segment_and_only_that_one_is_raised(cx: &mut gpui::TestAppContext) {
        let _guard = crate::test_support::lock_visual_test();
        let theme = AppTheme::gitcomet_dark();
        let (view, cx) = cx.add_window_view(|_, _| Picker { theme, selected: 0 });
        crate::test_support::redraw(cx);
        let raised: gpui::Background = theme.colors.surface.raised.into();
        let is_raised = |cx: &mut gpui::VisualTestContext, selector| {
            crate::test_support::painted_control_quads(cx, selector)
                .iter()
                .any(|(fill, _)| *fill == raised)
        };
        assert!(is_raised(cx, "picker_inline"));
        assert!(!is_raised(cx, "picker_split"));
        let split = cx.debug_bounds("picker_split").unwrap();
        cx.simulate_click(split.center(), gpui::Modifiers::default());
        crate::test_support::redraw(cx);
        assert_eq!(cx.update(|_, app| view.read(app).selected), 1);
        assert!(is_raised(cx, "picker_split"));
        assert!(!is_raised(cx, "picker_inline"));
    }
}
