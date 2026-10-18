use super::shortcut_keys::{compact_keycap_plate, compact_keycaps};
use crate::icons::svg_spinner;
use crate::theme::{AppTheme, composite_over};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, ClickEvent, Div, ElementId, FocusHandle, IntoElement, Pixels, Rgba,
    SharedString, Stateful, StyleRefinement, Svg, Window, div, px,
};
use palette::IntoColor;
use std::cell::RefCell;
use std::rc::Rc;

use super::{
    ControlActivation, ControlInteractionExt, InteractionState, InteractionStyle, control_height,
    control_pad_x, control_pad_y, icon_pad_x,
};

/// Gap between a button's icon and its label.
const CONTENT_GAP_PX: f32 = 4.0;
const CONTENT_GAP_COMFORTABLE_PX: f32 = 6.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonStyle {
    Filled,
    Outlined,
    Solid,
    Subtle,
    Transparent,
    Danger,
}

/// Slot content, dimmed with the button when it is disabled. `Custom` is built
/// at render time from that state and dims its own colours.
pub enum ButtonSlot {
    Icon(Box<Svg>),
    Spinner {
        id: ElementId,
        color: Rgba,
        size: Pixels,
    },
    Keys {
        label: SharedString,
        fg: Rgba,
        bg: Rgba,
        scale: UiScale,
    },
    Custom(Box<dyn FnOnce(bool) -> AnyElement>),
}

impl ButtonSlot {
    pub fn spinner(id: impl Into<ElementId>, color: Rgba, size: Pixels) -> Self {
        Self::Spinner {
            id: id.into(),
            color,
            size,
        }
    }

    pub fn keys(label: impl Into<SharedString>, fg: Rgba, scale: UiScale) -> Self {
        Self::Keys {
            label: label.into(),
            fg,
            bg: compact_keycap_plate(fg),
            scale,
        }
    }

    pub fn custom<E: IntoElement>(build: impl FnOnce(bool) -> E + 'static) -> Self {
        Self::Custom(Box::new(move |disabled| build(disabled).into_any_element()))
    }

    fn dimmed(self, interaction: &InteractionStyle, state: InteractionState) -> Self {
        if !interaction.dims(state) {
            return self;
        }
        let dim = |color| interaction.color(color, state);
        match self {
            Self::Icon(icon) => Self::Icon(Box::new(dimmed_icon(*icon, dim))),
            Self::Spinner { id, color, size } => Self::Spinner {
                id,
                color: dim(color),
                size,
            },
            Self::Keys {
                label,
                fg,
                bg,
                scale,
            } => Self::Keys {
                label,
                fg: dim(fg),
                bg: dim(bg),
                scale,
            },
            Self::Custom(build) => Self::Custom(build),
        }
    }

    fn into_element(self, interaction: &InteractionStyle, state: InteractionState) -> AnyElement {
        match self.dimmed(interaction, state) {
            Self::Icon(icon) => (*icon).into_any_element(),
            Self::Spinner { id, color, size } => svg_spinner(id, color, size).into_any_element(),
            Self::Keys {
                label,
                fg,
                bg,
                scale,
            } => compact_keycaps(&label, fg, bg, scale).into_any_element(),
            Self::Custom(build) => build(interaction.dims(state)),
        }
    }
}

impl From<Svg> for ButtonSlot {
    fn from(icon: Svg) -> Self {
        Self::Icon(Box::new(icon))
    }
}

fn dimmed_icon(mut icon: Svg, dim: impl Fn(Rgba) -> Rgba) -> Svg {
    let text = icon.text_style();
    text.color = text.color.map(|color| dim(color.into_color()).into_color());
    icon
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ButtonRounding {
    #[default]
    All,
    Left,
    Right,
}

pub struct Button {
    id: SharedString,
    label: SharedString,
    style: ButtonStyle,
    disabled: bool,
    selected: bool,
    open: bool,
    busy: bool,
    selected_bg: Option<gpui::Rgba>,
    bg: Option<gpui::Rgba>,
    hover_bg: Option<gpui::Rgba>,
    text_color: Option<gpui::Rgba>,
    rounding: ButtonRounding,
    borderless: bool,
    suppress_hover_border: bool,
    no_focus: bool,
    focus_handle: Option<FocusHandle>,
    start_slot: Option<ButtonSlot>,
    end_slot: Option<ButtonSlot>,
    separate_end_slot: bool,
    truncate_label: bool,
    /// Set when the caller pins the button's geometry instead of taking the
    /// app's. See [`Button::unscaled`].
    scale: Option<UiScale>,
}

impl Button {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            style: ButtonStyle::Subtle,
            disabled: false,
            selected: false,
            open: false,
            busy: false,
            selected_bg: None,
            bg: None,
            hover_bg: None,
            text_color: None,
            rounding: ButtonRounding::All,
            borderless: false,
            suppress_hover_border: false,
            no_focus: false,
            focus_handle: None,
            start_slot: None,
            end_slot: None,
            separate_end_slot: false,
            truncate_label: false,
            scale: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Persistent state belongs to the surface opened by this control.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Shares the operation's lifetime with its spinner; does not change eligibility.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn selected_bg(mut self, bg: gpui::Rgba) -> Self {
        self.selected_bg = Some(bg);
        self
    }

    /// Resting background, for a button that carries its own tint instead of
    /// the style's neutral fill.
    pub fn bg(mut self, bg: gpui::Rgba) -> Self {
        self.bg = Some(bg);
        self
    }

    /// Hover *and* pressed background. A tinted resting background needs this
    /// too: the style's neutral overlay would otherwise read as a step *down*
    /// from the tint under the cursor.
    pub fn hover_bg(mut self, bg: gpui::Rgba) -> Self {
        self.hover_bg = Some(bg);
        self
    }

    pub fn text_color(mut self, color: gpui::Rgba) -> Self {
        self.text_color = Some(color);
        self
    }

    /// Allow a parent with limited width to shrink the label with an ellipsis.
    /// Icons and action targets retain their size.
    pub fn truncate_label(mut self) -> Self {
        self.truncate_label = true;
        self
    }

    /// Round only the outside edge of a button in a segmented control.
    pub fn rounded_left(mut self) -> Self {
        self.rounding = ButtonRounding::Left;
        self
    }

    /// Round only the outside edge of a button in a segmented control.
    pub fn rounded_right(mut self) -> Self {
        self.rounding = ButtonRounding::Right;
        self
    }

    pub fn borderless(mut self) -> Self {
        self.borderless = true;
        self
    }

    pub fn no_hover_border(mut self) -> Self {
        self.suppress_hover_border = true;
        self
    }

    pub fn no_focus(mut self) -> Self {
        self.no_focus = true;
        self
    }

    pub fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }

    pub fn start_slot(mut self, slot: impl Into<ButtonSlot>) -> Self {
        self.start_slot = Some(slot.into());
        self
    }

    pub fn end_slot(mut self, slot: impl Into<ButtonSlot>) -> Self {
        self.end_slot = Some(slot.into());
        self.separate_end_slot = false;
        self
    }

    pub fn separated_end_slot(mut self, slot: impl Into<ButtonSlot>) -> Self {
        self.end_slot = Some(slot.into());
        self.separate_end_slot = true;
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Pins the button to the window chrome's fixed scale, ignoring the app's
    /// UI scale, density and font size. For a button in a title bar, which
    /// shares its row with OS controls that never resize.
    pub fn unscaled(mut self) -> Self {
        self.scale = Some(crate::ui_scale::chrome_scale());
        self
    }

    pub fn on_click<V: 'static>(
        self,
        theme: AppTheme,
        cx: &mut gpui::Context<V>,
        f: impl Fn(&mut V, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Stateful<Div> {
        let ui_scale = self.scale.unwrap_or_else(|| UiScale::current(cx));
        self.on_click_handler(theme, ui_scale, cx.listener(f))
    }

    /// Attach an already constructed listener through the same eligibility gate.
    pub fn on_click_handler(
        self,
        theme: AppTheme,
        ui_scale: impl Into<UiScale>,
        f: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Stateful<Div> {
        let disabled = self.disabled;
        self.render(theme, ui_scale)
            // Render already resolves explicit/default/no-focus behavior.
            .on_activate(disabled, ControlActivation::ManagedFocus, f)
    }

    pub fn on_click_with_bounds<V: 'static>(
        self,
        theme: AppTheme,
        cx: &mut gpui::Context<V>,
        f: impl Fn(&mut V, &ClickEvent, Bounds<Pixels>, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Stateful<Div> {
        let ui_scale = self.scale.unwrap_or_else(|| UiScale::current(cx));

        let truncate_label = self.truncate_label;
        let last_bounds: Rc<RefCell<Option<Bounds<Pixels>>>> = Rc::new(RefCell::new(None));
        let last_bounds_for_prepaint = Rc::clone(&last_bounds);
        let last_bounds_for_click = Rc::clone(&last_bounds);
        let wrapper_id: SharedString = format!("{}_bounds_wrapper", self.id).into();

        let button = self.on_click_handler(
            theme,
            ui_scale,
            cx.listener(move |this, e: &ClickEvent, window, cx| {
                let bounds = (*last_bounds_for_click.borrow())
                    .unwrap_or_else(|| Bounds::new(e.position(), gpui::size(px(0.0), px(0.0))));
                f(this, e, bounds, window, cx);
            }),
        );

        div()
            .when(truncate_label, |d| d.min_w(px(0.0)))
            .on_children_prepainted(move |children_bounds, _window, _cx| {
                if let Some(bounds) = children_bounds.first() {
                    *last_bounds_for_prepaint.borrow_mut() = Some(*bounds);
                }
            })
            .child(button)
            .id(wrapper_id)
    }

    pub fn render(self, theme: AppTheme, ui_scale: impl Into<UiScale>) -> Stateful<Div> {
        let Self {
            id,
            label,
            style,
            disabled,
            selected,
            open,
            busy,
            selected_bg,
            bg: bg_override,
            hover_bg: hover_bg_override,
            text_color: text_color_override,
            rounding,
            borderless,
            suppress_hover_border,
            no_focus,
            focus_handle,
            start_slot,
            end_slot,
            separate_end_slot,
            truncate_label,
            scale,
        } = self;
        let ui_scale = scale.unwrap_or_else(|| ui_scale.into().with_appearance(theme.metrics));

        let transparent = gpui::rgba(0x00000000);
        let outlined_border = if theme.is_dark {
            with_alpha(theme.colors.foreground.secondary, 0.38)
        } else {
            theme.colors.stroke.control
        };
        let hover_overlay = theme.hover_overlay();
        let active_overlay = theme.active_overlay();
        let (bg, hover_bg, active_bg, border, hover_border, active_border, text) = match style {
            ButtonStyle::Filled => (
                transparent,
                hover_overlay,
                active_overlay,
                with_alpha(theme.colors.accent.foreground, 0.90),
                with_alpha(theme.colors.accent.foreground, 1.00),
                with_alpha(theme.colors.accent.foreground, 1.00),
                theme.colors.accent.foreground,
            ),
            ButtonStyle::Outlined => (
                transparent,
                hover_overlay,
                active_overlay,
                outlined_border,
                if theme.is_dark {
                    with_alpha(theme.colors.foreground.secondary, 0.55)
                } else {
                    theme.colors.interaction.selected_indicator
                },
                if theme.is_dark {
                    with_alpha(theme.colors.foreground.secondary, 0.62)
                } else {
                    theme.colors.interaction.selected_indicator
                },
                theme.colors.foreground.primary,
            ),
            ButtonStyle::Solid => {
                let bg = theme.colors.surface.raised;
                let hover_bg = if theme.is_dark {
                    mix(bg, theme.colors.foreground.primary, 0.06)
                } else {
                    composite_over(bg, hover_overlay)
                };
                let active_bg = if theme.is_dark {
                    mix(bg, theme.colors.foreground.primary, 0.10)
                } else {
                    composite_over(bg, active_overlay)
                };
                (
                    bg,
                    hover_bg,
                    active_bg,
                    if theme.is_dark {
                        with_alpha(theme.colors.foreground.secondary, 0.34)
                    } else {
                        theme.colors.stroke.control
                    },
                    if theme.is_dark {
                        with_alpha(theme.colors.foreground.secondary, 0.55)
                    } else {
                        theme.colors.interaction.selected_indicator
                    },
                    if theme.is_dark {
                        with_alpha(theme.colors.foreground.secondary, 0.62)
                    } else {
                        theme.colors.interaction.selected_indicator
                    },
                    theme.colors.foreground.primary,
                )
            }
            ButtonStyle::Subtle => (
                transparent,
                hover_overlay,
                active_overlay,
                transparent,
                with_alpha(
                    theme.colors.foreground.secondary,
                    if theme.is_dark { 0.45 } else { 0.32 },
                ),
                with_alpha(
                    theme.colors.foreground.secondary,
                    if theme.is_dark { 0.52 } else { 0.38 },
                ),
                theme.colors.foreground.primary,
            ),
            ButtonStyle::Transparent => (
                transparent,
                hover_overlay,
                active_overlay,
                transparent,
                with_alpha(
                    theme.colors.foreground.secondary,
                    if theme.is_dark { 0.40 } else { 0.30 },
                ),
                with_alpha(
                    theme.colors.foreground.secondary,
                    if theme.is_dark { 0.46 } else { 0.34 },
                ),
                theme.colors.foreground.secondary,
            ),
            ButtonStyle::Danger => {
                let danger = theme.colors.status.danger;
                if theme.is_dark {
                    (
                        with_alpha(danger.foreground, 0.18),
                        with_alpha(danger.foreground, 0.26),
                        with_alpha(danger.foreground, 0.32),
                        with_alpha(danger.foreground, 0.42),
                        with_alpha(danger.foreground, 0.46),
                        with_alpha(danger.foreground, 0.52),
                        theme.colors.foreground.primary,
                    )
                } else {
                    (
                        danger.background,
                        composite_over(danger.background, hover_overlay),
                        composite_over(danger.background, active_overlay),
                        danger.border,
                        danger.foreground,
                        danger.foreground,
                        danger.foreground,
                    )
                }
            }
        };

        let hover_bg = hover_bg_override.unwrap_or(hover_bg);
        let active_bg = hover_bg_override.unwrap_or(active_bg);
        let mut hover = StyleRefinement::default().bg(hover_bg);
        let mut pressed = StyleRefinement::default().bg(active_bg);
        if !(suppress_hover_border || borderless) {
            hover = hover.border_color(hover_border);
            pressed = pressed.border_color(active_border);
        }
        let interaction = InteractionStyle::accent(theme)
            .persistent_background(selected_bg.unwrap_or(super::control_open_background(theme)))
            .hover(hover)
            .pressed(pressed)
            .dim_when_disabled(true);
        let state = InteractionState::default()
            .selected(
                selected,
                selected_bg.unwrap_or(theme.colors.interaction.pressed_background),
            )
            .open(open)
            .busy(busy)
            .disabled(disabled);
        let state_color = |color: Rgba| interaction.color(color, state);
        let bg = state_color(bg_override.unwrap_or(bg));
        let text = state_color(text_color_override.unwrap_or(text));
        let border = state_color(border);
        let separator_color = state_color(with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.34 } else { 0.26 },
        ));
        let start_slot = start_slot.map(|slot| slot.into_element(&interaction, state));
        let end_slot = end_slot.map(|slot| slot.into_element(&interaction, state));
        let icon_only = looks_like_icon_button(label.as_ref());
        let control_height = control_height(ui_scale);
        let control_pad_x = control_pad_x(ui_scale);
        let control_pad_y = control_pad_y(ui_scale);
        let icon_pad_x = icon_pad_x(ui_scale);
        let content_gap = ui_scale.px(ui_scale
            .appearance
            .ramp(CONTENT_GAP_PX, CONTENT_GAP_COMFORTABLE_PX));
        let separated_slot_pad = ui_scale.px(6.0);

        let mut leading = div()
            .flex()
            .items_center()
            .gap(content_gap)
            .when(truncate_label, |d| d.min_w(px(0.0)).overflow_hidden());
        if let Some(start_slot) = start_slot {
            leading = leading.child(start_slot);
        }
        if !label.is_empty() {
            leading = if truncate_label {
                leading.child(div().min_w(px(0.0)).truncate().child(label))
            } else {
                leading.child(label)
            };
        }
        let inner = match (separate_end_slot, end_slot) {
            (true, Some(end_slot)) => div()
                .flex()
                .items_center()
                .h_full()
                .child(leading.pr(separated_slot_pad))
                .child(
                    div()
                        .debug_selector({
                            let id = id.clone();
                            move || format!("{}_end_slot_separator", id.as_ref())
                        })
                        .flex()
                        .items_center()
                        .h_full()
                        .pl(separated_slot_pad)
                        .border_l_1()
                        .border_color(separator_color)
                        .child(end_slot),
                ),
            (_, Some(end_slot)) => leading.child(end_slot),
            (_, None) => leading,
        };

        let control_radius = px(theme.radii.control);
        let mut base = div()
            .id(id.clone())
            .debug_selector(move || id.to_string())
            .h(control_height)
            .when(truncate_label, |d| d.min_w(px(0.0)))
            .when(icon_only, |d| d.min_w(control_height))
            .px(if icon_only { icon_pad_x } else { control_pad_x })
            .py(control_pad_y)
            .flex()
            .items_center()
            .justify_center()
            .when(rounding == ButtonRounding::All, |d| {
                d.rounded(control_radius)
            })
            .when(rounding == ButtonRounding::Left, |d| {
                d.rounded_tl(control_radius).rounded_bl(control_radius)
            })
            .when(rounding == ButtonRounding::Right, |d| {
                d.rounded_tr(control_radius).rounded_br(control_radius)
            })
            .bg(bg)
            .text_size(ui_scale.ui_text(14.0))
            .text_color(text)
            .child(inner);

        if let Some(focus_handle) = focus_handle {
            let focus_handle = focus_handle.tab_stop(!disabled);
            base = base.track_focus(&focus_handle);
        } else if !no_focus {
            base = base.tab_index(0).tab_stop(!disabled);
        }

        if !borderless {
            base = base.border_1().border_color(border);
        }
        interaction.apply(base, state)
    }
}

fn looks_like_icon_button(label: &str) -> bool {
    let trimmed = label.trim();
    trimmed.is_empty()
        || (trimmed.chars().count() <= 2 && !trimmed.chars().any(|c| c.is_alphanumeric()))
}

/// Small actions embedded in editor rows share geometry and eligibility;
/// callers supply only their icon, size and action.
pub fn inline_icon_button(
    id: impl Into<gpui::ElementId>,
    theme: AppTheme,
    size: Pixels,
    icon: &'static str,
    icon_size: Pixels,
    icon_color: gpui::Rgba,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(size)
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.radii.row))
        .control_interaction(
            InteractionStyle::header(theme),
            InteractionState::default().disabled(!enabled),
        )
        .child(crate::icons::svg_icon(icon, icon_color, icon_size))
}

use crate::theme::with_alpha;

fn mix(a: gpui::Rgba, b: gpui::Rgba, t: f32) -> gpui::Rgba {
    let t = t.clamp(0.0, 1.0);
    gpui::Rgba::new(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
        a.alpha + (b.alpha - a.alpha) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        assert_color_close, color_close, painted_control_quads, redraw, solid_background,
        solid_color,
    };
    use gpui::{Context, Render, TestAppContext, Window};
    use std::cell::RefCell;
    use std::rc::Rc;

    const STYLES: [ButtonStyle; 6] = [
        ButtonStyle::Filled,
        ButtonStyle::Outlined,
        ButtonStyle::Solid,
        ButtonStyle::Subtle,
        ButtonStyle::Transparent,
        ButtonStyle::Danger,
    ];

    fn themes() -> [AppTheme; 2] {
        [
            AppTheme::from_key(crate::theme::DEFAULT_DARK_THEME_KEY).unwrap(),
            AppTheme::gitcomet_light(),
        ]
    }

    fn scale() -> UiScale {
        UiScale::from_percent(100)
    }

    fn root_colors(button: &mut Stateful<Div>) -> [Option<Rgba>; 3] {
        let border = button.style().border_color.as_ref().and_then(solid_color);
        let text = button.style().text.color.map(|color| color.into_color());
        [solid_background(button), border, text]
    }

    #[test]
    fn disabled_buttons_dim_their_colours_without_element_opacity() {
        for theme in themes() {
            for style in STYLES {
                let button = || Button::new("subject", "Action").style(style);
                let mut enabled = button().render(theme, scale());
                let mut disabled = button().disabled(true).render(theme, scale());

                assert_eq!(disabled.style().opacity, None, "{style:?}");
                for (enabled, disabled) in root_colors(&mut enabled)
                    .into_iter()
                    .zip(root_colors(&mut disabled))
                {
                    assert_color_close(disabled, enabled.map(|color| theme.disabled_color(color)));
                }
            }
        }
    }

    #[test]
    fn disabled_buttons_dim_their_selected_and_open_fills() {
        for theme in themes() {
            let button = || Button::new("subject", "Action").disabled(true);
            let mut selected = button().selected(true).render(theme, scale());
            let mut open = button().open(true).render(theme, scale());

            assert_color_close(
                solid_background(&mut selected),
                Some(theme.disabled_color(theme.colors.interaction.pressed_background)),
            );
            assert_color_close(
                solid_background(&mut open),
                Some(theme.disabled_color(super::super::control_open_background(theme))),
            );
        }
    }

    #[test]
    fn dimming_a_slot_dims_each_of_its_colours() {
        let theme = themes()[0];
        let color = gpui::rgb(0x3388cc);
        let plate = with_alpha(color, 0.14);
        let interaction = InteractionStyle::new(theme).dim_when_disabled(true);
        let colors = |slot: ButtonSlot, disabled: bool| -> Vec<Rgba> {
            match slot.dimmed(&interaction, InteractionState::default().disabled(disabled)) {
                ButtonSlot::Icon(mut icon) => icon
                    .text_style()
                    .color
                    .map(|c| c.into_color())
                    .into_iter()
                    .collect(),
                ButtonSlot::Spinner { color, .. } => vec![color],
                ButtonSlot::Keys { fg, bg, .. } => vec![fg, bg],
                ButtonSlot::Custom(_) => Vec::new(),
            }
        };
        let slots = || {
            [
                (
                    ButtonSlot::from(crate::icons::svg_icon("icons/check.svg", color, px(14.0))),
                    vec![color],
                ),
                (ButtonSlot::spinner("spinner", color, px(14.0)), vec![color]),
                (
                    ButtonSlot::keys("Ctrl+S", color, scale()),
                    vec![color, plate],
                ),
            ]
        };
        for disabled in [false, true] {
            for (slot, expected) in slots() {
                let actual = colors(slot, disabled);
                assert_eq!(actual.len(), expected.len());
                for (actual, expected) in actual.into_iter().zip(expected) {
                    let expected = theme.disabled_color_if(disabled, expected);
                    assert_color_close(Some(actual), Some(expected));
                }
            }
        }
    }

    struct EnabledAndDisabled {
        theme: AppTheme,
        color: Rgba,
        custom_states: Rc<RefCell<Vec<bool>>>,
    }

    impl Render for EnabledAndDisabled {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let button = |id: &'static str, disabled: bool| {
                let custom_states = Rc::clone(&self.custom_states);
                Button::new(id, "Go")
                    .start_slot(ButtonSlot::custom(move |disabled| {
                        custom_states.borrow_mut().push(disabled);
                        div()
                    }))
                    .separated_end_slot(ButtonSlot::keys("Enter", self.color, scale()))
                    .disabled(disabled)
                    .render(self.theme, scale())
            };
            div()
                .flex()
                .child(button("on", false))
                .child(button("off", true))
        }
    }

    fn fills_inside(cx: &mut gpui::VisualTestContext, selector: &'static str) -> Vec<Rgba> {
        let bounds = cx.debug_bounds(selector).expect("button must be drawn");
        cx.update(|window, _| {
            let scale = window.scale_factor();
            let inside = |value: f32, start: gpui::Pixels, length: gpui::Pixels| {
                value >= f32::from(start) * scale - 1.0
                    && value <= f32::from(start + length) * scale + 1.0
            };
            window
                .painted_quads()
                .into_iter()
                .filter(|quad| {
                    inside(quad.bounds.origin.x.0, bounds.origin.x, bounds.size.width)
                        && inside(quad.bounds.origin.y.0, bounds.origin.y, bounds.size.height)
                })
                .filter_map(|quad| solid_color(&quad.background))
                .collect()
        })
    }

    const SLOT_COLOR: u32 = 0x3388cc;

    fn draw_enabled_and_disabled(
        cx: &mut TestAppContext,
    ) -> (&mut gpui::VisualTestContext, Rc<RefCell<Vec<bool>>>) {
        let custom_states = Rc::new(RefCell::new(Vec::new()));
        let states = Rc::clone(&custom_states);
        let (_view, cx) = cx.add_window_view(move |_, _| EnabledAndDisabled {
            theme: themes()[0],
            color: gpui::rgb(SLOT_COLOR),
            custom_states: states,
        });
        redraw(cx);
        (cx, custom_states)
    }

    #[gpui::test]
    fn custom_slots_are_built_with_the_buttons_disabled_state(cx: &mut TestAppContext) {
        let (_cx, custom_states) = draw_enabled_and_disabled(cx);
        let states = custom_states.borrow().clone();
        assert!(!states.is_empty());
        assert!(
            states.chunks(2).all(|pair| pair == [false, true]),
            "{states:?}"
        );
    }

    #[gpui::test]
    fn disabled_buttons_paint_dimmed_keycap_plates(cx: &mut TestAppContext) {
        let (cx, _) = draw_enabled_and_disabled(cx);
        let plate = with_alpha(gpui::rgb(SLOT_COLOR), 0.14);
        let has =
            |fills: &[Rgba], expected: Rgba| fills.iter().any(|fill| color_close(*fill, expected));
        let enabled = fills_inside(cx, "on");
        let disabled = fills_inside(cx, "off");
        assert!(has(&enabled, plate), "{enabled:?}");
        assert!(
            has(&disabled, themes()[0].disabled_color(plate)),
            "{disabled:?}"
        );
        assert!(!has(&disabled, plate), "{disabled:?}");
    }

    #[gpui::test]
    fn disabled_buttons_paint_a_dimmed_slot_separator(cx: &mut TestAppContext) {
        let (cx, _) = draw_enabled_and_disabled(cx);
        let separator = |cx: &mut gpui::VisualTestContext, selector| {
            painted_control_quads(cx, selector)
                .first()
                .and_then(|(_, border)| solid_color(border))
        };
        let enabled = separator(cx, "on_end_slot_separator");
        assert!(enabled.is_some());
        assert_color_close(
            separator(cx, "off_end_slot_separator"),
            enabled.map(|color| themes()[0].disabled_color(color)),
        );
    }
}
