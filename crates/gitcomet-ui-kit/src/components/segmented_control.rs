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
use std::rc::Rc;

mod layout;

const TRACK_INSET_PX: f32 = 2.0;
const SEGMENT_GAP_PX: f32 = 2.0;
const LABEL_GAP_PX: f32 = 5.0;
const SEGMENT_PADDING_PX: f32 = 8.0;
const ICON_SIZE_PX: f32 = 14.0;
const FONT_SIZE_PX: f32 = 12.0;
const SEGMENT_BORDER_PX: f32 = 1.0;
const MINIMUM_LABEL_EM: f32 = 2.0;

enum SegmentSizing {
    Natural,
    SelectedPriority,
}

type Activation = Rc<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App)>;

/// One option of a [`SegmentedControl`].
pub struct Segment {
    id: SharedString,
    label: SharedString,
    icon: Option<SharedString>,
    selected: bool,
    disabled: bool,
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
            disabled: false,
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

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
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
    sizing: SegmentSizing,
    font: Option<gpui::Font>,
}

impl SegmentedControl {
    /// `id` is the track's element id and debug selector.
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            segments: Vec::new(),
            sizing: SegmentSizing::Natural,
            font: None,
        }
    }

    pub fn segment(mut self, segment: Segment) -> Self {
        self.segments.push(segment);
        self
    }

    pub fn adaptive(mut self) -> Self {
        self.sizing = SegmentSizing::SelectedPriority;
        self
    }

    pub fn font(mut self, font: gpui::Font) -> Self {
        self.font = Some(font);
        self
    }

    fn track_spacing(&self, scale: UiScale) -> gpui::Pixels {
        scale.px(TRACK_INSET_PX) * 2.0
            + scale.px(SEGMENT_GAP_PX) * self.segments.len().saturating_sub(1) as f32
    }

    fn measurements(
        &self,
        theme: AppTheme,
        scale: UiScale,
        window: &mut Window,
    ) -> Vec<layout::SegmentMetrics> {
        let mut style = window.text_style();
        style.font_size = theme.ui_text(FONT_SIZE_PX).into();
        if let Some(font) = &self.font {
            style.font_family = font.family.clone();
            style.font_features = font.features.clone();
            style.font_fallbacks = font.fallbacks.clone();
            style.font_weight = font.weight;
            style.font_width = font.width;
            style.font_style = font.style;
        }
        let font_size = style.font_size.to_pixels(window.rem_size());
        self.segments
            .iter()
            .map(|segment| {
                let run = style.to_run(segment.label.len());
                let label = window
                    .text_system()
                    .shape_line(segment.label.clone(), font_size, &[run])
                    .width
                    .ceil();
                let collapsible = !segment.selected && segment.icon.is_some();
                let base = scale.px(SEGMENT_PADDING_PX) * 2.0
                    + px(SEGMENT_BORDER_PX) * 2.0
                    + if segment.icon.is_some() {
                        scale.px(ICON_SIZE_PX)
                    } else {
                        px(0.0)
                    };
                layout::SegmentMetrics {
                    minimum: base
                        + if collapsible {
                            px(0.0)
                        } else {
                            label
                                + if segment.icon.is_some() {
                                    scale.px(LABEL_GAP_PX)
                                } else {
                                    px(0.0)
                                }
                        },
                    label,
                    collapsible,
                }
            })
            .collect()
    }

    pub fn minimum_width(
        &self,
        theme: AppTheme,
        scale: UiScale,
        window: &mut Window,
    ) -> gpui::Pixels {
        self.track_spacing(scale)
            + self
                .measurements(theme, scale, window)
                .iter()
                .map(|m| m.minimum)
                .sum::<gpui::Pixels>()
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
        let on_select = Rc::new(on_select);
        let callbacks: Vec<Activation> = (0..self.segments.len())
            .map(|index| {
                let on_select = on_select.clone();
                Rc::new(cx.listener(move |view, _: &ClickEvent, window, cx| {
                    on_select(view, index, window, cx);
                })) as Activation
            })
            .collect();
        match self.sizing {
            SegmentSizing::Natural => self.render_track(theme, scale, callbacks, None),
            SegmentSizing::SelectedPriority => div()
                .id(SharedString::from(format!("{}_container", self.id)))
                .flex_1()
                .min_w(px(0.0))
                .h(super::control_height(scale))
                .child(gpui::container_query(move |size, window, _| {
                    let metrics = self.measurements(theme, scale, window);
                    let allocation = layout::allocate(
                        &metrics,
                        size.width - self.track_spacing(scale),
                        scale.px(LABEL_GAP_PX),
                        theme.ui_text(FONT_SIZE_PX).to_pixels(window.rem_size()) * MINIMUM_LABEL_EM,
                    );
                    self.render_track(theme, scale, callbacks, Some(allocation))
                })),
        }
    }

    fn render_track(
        self,
        theme: AppTheme,
        scale: UiScale,
        callbacks: Vec<Activation>,
        allocation: Option<Vec<layout::SegmentLayout>>,
    ) -> Stateful<Div> {
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
            .min_w(px(0.0))
            .h(super::control_height(scale))
            .p(scale.px(TRACK_INSET_PX))
            .gap(scale.px(SEGMENT_GAP_PX))
            .rounded(px(theme.radii.row))
            .bg(theme.hover_overlay())
            .when_some(self.font, |track, font| track.font(font));
        if let Some(layout) = &allocation {
            track = track.w(layout.iter().map(|s| s.width).sum::<gpui::Pixels>()
                + scale.px(TRACK_INSET_PX) * 2.0
                + scale.px(SEGMENT_GAP_PX) * layout.len().saturating_sub(1) as f32);
        }
        for (index, (segment, callback)) in self.segments.into_iter().zip(callbacks).enumerate() {
            let Segment {
                id,
                label,
                icon,
                selected,
                disabled,
                tooltip,
            } = segment;
            let text = if selected {
                theme.colors.foreground.primary
            } else {
                theme.colors.foreground.secondary
            };
            let selector = id.clone();
            let icon_selector = format!("{id}_icon");
            let label_selector = format!("{id}_label");
            let style = InteractionStyle::header(theme).selection_outline(false);
            let mut button = div()
                .id(id)
                .debug_selector(move || selector.to_string())
                .h_full()
                .flex()
                .items_center()
                .min_w(px(0.0))
                .gap(scale.px(LABEL_GAP_PX))
                .px(scale.px(SEGMENT_PADDING_PX))
                .rounded(px(theme.radii.control))
                .border_1()
                .border_color(if selected {
                    theme.colors.stroke.subtle
                } else {
                    gpui::rgba(0x00000000)
                })
                .text_size(theme.ui_text(FONT_SIZE_PX))
                .text_color(text)
                .control_interaction(
                    style,
                    InteractionState::default()
                        .selected(selected, raised)
                        .disabled(disabled),
                )
                .on_activate(
                    disabled,
                    ControlActivation::Action,
                    move |event, window, cx| callback(event, window, cx),
                )
                .children(icon.map(|icon| {
                    svg_icon(icon, text, scale.px(ICON_SIZE_PX))
                        .flex_none()
                        .debug_selector(move || icon_selector)
                }));
            let show_label = allocation.as_ref().is_none_or(|layout| layout[index].label);
            if let Some(layout) = &allocation {
                button = button.w(layout[index].width).flex_none();
            }
            if show_label {
                button = button.child(
                    div()
                        .min_w(px(0.0))
                        .truncate()
                        .debug_selector(move || label_selector)
                        .child(label),
                );
            }
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

    struct AdaptivePicker {
        selected: usize,
        width: gpui::Pixels,
    }

    impl Render for AdaptivePicker {
        fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
            let mut control = SegmentedControl::new("adaptive").adaptive();
            for (index, label) in ["Branches", "Files", "History", "Long extension panel"]
                .into_iter()
                .enumerate()
            {
                control = control.segment(
                    Segment::new(format!("adaptive_{index}"), label)
                        .icon("icons/history.svg")
                        .selected(index == self.selected)
                        .tooltip(label, Vec::new()),
                );
            }
            div().w(self.width).flex().child(control.render(
                AppTheme::gitcomet_dark(),
                UiScale::current(cx),
                cx,
                |this, index, _, cx| {
                    this.selected = index;
                    cx.notify();
                },
            ))
        }
    }

    #[gpui::test]
    fn adaptive_tabs_keep_selection_full_and_collapse_trailing_labels(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|_, _| AdaptivePicker {
            selected: 0,
            width: px(500.0),
        });
        crate::test_support::redraw(cx);
        let active = cx.debug_bounds("adaptive_0").unwrap().size.width;
        assert!(cx.debug_bounds("adaptive_3_label").is_some());
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.width = px(200.0);
                cx.notify();
            })
        });
        crate::test_support::redraw(cx);
        assert_eq!(cx.debug_bounds("adaptive_0").unwrap().size.width, active);
        assert!(cx.debug_bounds("adaptive_0_label").is_some());
        assert!(cx.debug_bounds("adaptive_3_label").is_none());
        assert!(cx.debug_bounds("adaptive_3_icon").is_some());
        let last = cx.debug_bounds("adaptive_3").unwrap();
        assert!(last.right() <= px(200.0));
        cx.simulate_click(last.center(), gpui::Modifiers::default());
        crate::test_support::redraw(cx);
        assert_eq!(cx.update(|_, app| view.read(app).selected), 3);
        assert!(cx.debug_bounds("adaptive_3_label").is_some());
        assert!(cx.debug_bounds("adaptive_0_label").is_none());
    }

    struct Picker {
        theme: AppTheme,
        selected: usize,
        split_disabled: bool,
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
                    .segment(
                        Segment::new("picker_split", "Split")
                            .selected(selected == 1)
                            .disabled(self.split_disabled),
                    )
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
        let theme = AppTheme::gitcomet_dark();
        let (view, cx) = cx.add_window_view(|_, _| Picker {
            theme,
            selected: 0,
            split_disabled: false,
        });
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

    #[gpui::test]
    fn disabled_segments_ignore_clicks_until_enabled(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| Picker {
            theme: AppTheme::gitcomet_dark(),
            selected: 0,
            split_disabled: true,
        });
        crate::test_support::redraw(cx);
        let split = cx.debug_bounds("picker_split").unwrap();
        cx.simulate_click(split.center(), gpui::Modifiers::default());
        crate::test_support::redraw(cx);
        assert_eq!(cx.update(|_, app| view.read(app).selected), 0);

        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.split_disabled = false;
                cx.notify();
            });
        });
        crate::test_support::redraw(cx);
        let split = cx.debug_bounds("picker_split").unwrap();
        cx.simulate_click(split.center(), gpui::Modifiers::default());
        crate::test_support::redraw(cx);
        assert_eq!(cx.update(|_, app| view.read(app).selected), 1);
    }
}
