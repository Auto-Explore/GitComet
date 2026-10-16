use super::*;
use gitcomet_ui_kit::interaction::ControlActivation;
use gitcomet_ui_kit::interaction::{ControlInteractionExt, InteractionState, InteractionStyle};
use gitcomet_ui_kit::theme::AppTheme;
use gitcomet_ui_kit::tooltip::GitCometTooltipExt as _;
use gpui::AnyElement;
use gpui::App;
use gpui::ClickEvent;
use gpui::Entity;
use gpui::Pixels;
use gpui::Point;
use gpui::SharedString;
use gpui::WeakEntity;
use gpui::Window;
use gpui::div;
use gpui::px;

#[derive(Clone, Copy)]
enum Control {
    Menu(ImageMenu),
    Zoom(Zoom),
    Link,
    Pause,
}

#[derive(Clone)]
struct ZoomMenuTarget {
    group: WeakEntity<Group>,
    side: Option<usize>,
    target: String,
}

#[derive(Default)]
struct MenuTarget(std::collections::HashMap<gpui::WindowId, ZoomMenuTarget>);
impl gpui::Global for MenuTarget {}

pub fn apply_menu_zoom(zoom: Zoom, window: &Window, cx: &mut App) {
    if let Some(target) = cx
        .try_global::<MenuTarget>()
        .and_then(|target| target.0.get(&window.window_handle().window_id()))
        .cloned()
        && let Some(group) = target.group.upgrade()
    {
        group.update(cx, |group, cx| {
            if group.target == target.target {
                group.zoom_side(target.side.unwrap_or(group.active), zoom, cx);
            }
        });
    }
}

/// Opens `menu` for the chip `invoker` at `position`.
pub type OpenMenu<V> =
    fn(&mut V, ImageMenu, SharedString, Point<Pixels>, &mut Window, &mut gpui::Context<V>);

struct Controls<V: 'static> {
    group: Entity<Group>,
    side: Option<usize>,
    theme: AppTheme,
    open_menu: OpenMenu<V>,
}

impl<V: 'static> Controls<V> {
    fn chip(
        &self,
        name: &'static str,
        label: String,
        tooltip: &'static str,
        action: Control,
        cx: &mut gpui::Context<V>,
    ) -> AnyElement {
        let group = self.group.clone();
        let side = self.side;
        let theme = self.theme;
        let open_menu = self.open_menu;
        let id: SharedString = side
            .map_or_else(|| name.to_string(), |side| format!("{name}_{side}"))
            .into();
        let selector = id.clone();
        let scaled_px = gitcomet_ui_kit::ui_scale::scaler(
            gitcomet_ui_kit::ui_scale::UiScale::current(cx).percent(),
        );
        div()
            .id(id.clone())
            .debug_selector(move || selector.to_string())
            .flex()
            .flex_none()
            .items_center()
            .gap(scaled_px(2.0))
            .px_1()
            .h(scaled_px(20.0))
            .rounded(px(theme.radii.row))
            .tab_index(0)
            .control_interaction(InteractionStyle::header(theme), InteractionState::default())
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .whitespace_nowrap()
            .child(label)
            .when(matches!(action, Control::Menu(_)), |d| {
                d.child(gitcomet_ui_kit::icons::svg_icon(
                    "icons/chevron_down.svg",
                    theme.colors.foreground.secondary,
                    scaled_px(10.0),
                ))
            })
            .gitcomet_tooltip(theme, tooltip.into())
            .on_activate(
                false,
                ControlActivation::Action,
                cx.listener(move |this, event: &ClickEvent, window, cx| match action {
                    Control::Menu(section) => {
                        if !cx.has_global::<MenuTarget>() {
                            cx.set_global(MenuTarget::default());
                        }
                        let zoom_target = ZoomMenuTarget {
                            group: group.downgrade(),
                            side,
                            target: group.read(cx).target.clone(),
                        };
                        cx.update_global::<MenuTarget, _>(|target, _| {
                            target.0.retain(|_, target| target.group.is_upgradable());
                            target
                                .0
                                .insert(window.window_handle().window_id(), zoom_target);
                        });
                        open_menu(this, section, id.clone(), event.position(), window, cx);
                    }
                    Control::Zoom(zoom) => group.update(cx, |group, cx| {
                        group.zoom_side(side.unwrap_or(group.active), zoom, cx);
                    }),
                    Control::Link | Control::Pause => group.update(cx, |group, cx| match action {
                        Control::Link => group.toggle_linked(cx),
                        _ => group.toggle_paused(cx),
                    }),
                }),
            )
            .into_any_element()
    }

    fn zoom_controls(&self, cx: &mut gpui::Context<V>) -> AnyElement {
        let state = self.group.read(cx);
        let zoom = self
            .side
            .map_or_else(|| state.zoom_label(), |side| state.zoom_label_for(side));
        let scaled_px = gitcomet_ui_kit::ui_scale::scaler(
            gitcomet_ui_kit::ui_scale::UiScale::current(cx).percent(),
        );
        let zoom_controls = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(scaled_px(2.0))
            .child(self.chip(
                "image_zoom_out",
                "−".into(),
                "Zoom out (−)",
                Control::Zoom(Zoom::Step(ZoomStep::Out)),
                cx,
            ))
            .child(self.chip(
                "image_zoom",
                zoom,
                "Choose zoom level · 100% is one image pixel per screen pixel",
                Control::Menu(ImageMenu::Zoom),
                cx,
            ))
            .child(self.chip(
                "image_zoom_in",
                "+".into(),
                "Zoom in (+)",
                Control::Zoom(Zoom::Step(ZoomStep::In)),
                cx,
            ));
        let size_controls = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(scaled_px(2.0))
            .child(self.chip(
                "image_fit",
                "Fit".into(),
                "Fit image to the available space (0)",
                Control::Zoom(Zoom::Fit),
                cx,
            ))
            .child(self.chip(
                "image_native",
                "Actual size".into(),
                "Actual size (100%) · One image pixel per screen pixel (1)",
                Control::Zoom(Zoom::Scale(1.0)),
                cx,
            ));
        div()
            .flex()
            .flex_wrap()
            .min_w(px(0.0))
            .max_w_full()
            .items_center()
            .justify_center()
            .gap(scaled_px(8.0))
            .child(zoom_controls)
            .child(size_controls)
            .into_any_element()
    }
}

pub fn viewport<V: 'static>(
    group: Entity<Group>,
    side: usize,
    theme: AppTheme,
    cx: &mut gpui::Context<V>,
    open_menu: OpenMenu<V>,
) -> AnyElement {
    let independent = !group.read(cx).linked && group.read(cx).comparison();
    let image = ImageViewport {
        group: group.clone(),
        side,
        theme,
    };
    if !independent {
        return image.into_any_element();
    }
    let controls = Controls {
        group,
        side: Some(side),
        theme,
        open_menu,
    };
    div()
        .size_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .child(div().flex_1().min_h(px(0.0)).child(image))
        .child(
            div()
                .id(("image_pane_controls", side))
                .debug_selector(move || format!("image_pane_controls_{side}"))
                .flex_none()
                .w_full()
                .p_1()
                .border_t_1()
                .border_color(theme.colors.stroke.default)
                .bg(theme.colors.surface.raised)
                .child(controls.zoom_controls(cx)),
        )
        .into_any_element()
}

pub fn toolbar<V: 'static>(
    group: Entity<Group>,
    theme: AppTheme,
    cx: &mut gpui::Context<V>,
    open_menu: OpenMenu<V>,
) -> AnyElement {
    let state = group.read(cx);
    let (raster, comparison, linked, animated, paused) = (
        state.raster(),
        state.comparison(),
        state.linked,
        state.animated(),
        state.paused,
    );
    let controls = Controls {
        group,
        side: None,
        theme,
        open_menu,
    };
    let scaled_px = gitcomet_ui_kit::ui_scale::scaler(
        gitcomet_ui_kit::ui_scale::UiScale::current(cx).percent(),
    );
    let mut strip = div()
        .id("image_format_strip")
        .debug_selector(|| "image_format_strip".into())
        .flex()
        .flex_wrap()
        .flex_shrink_0()
        .min_w(px(0.0))
        .max_w_full()
        .items_center()
        .justify_center()
        .gap_x(scaled_px(8.0))
        .gap_y(scaled_px(4.0));
    if raster {
        strip = strip.child(controls.chip(
            "image_sampling",
            Preferences::current(cx).label(),
            "Image sampling · Saved for all files and windows",
            Control::Menu(ImageMenu::Sampling),
            cx,
        ));
    }
    if linked || !comparison {
        strip = strip.child(controls.zoom_controls(cx));
    }
    if comparison {
        strip = strip.child(
            controls.chip(
                "image_link",
                if linked {
                    "Linked zoom & pan"
                } else {
                    "Independent zoom & pan"
                }
                .into(),
                if linked {
                    "Unlink zoom and pan to control each image separately"
                } else {
                    "Link zoom and pan using the last image you adjusted"
                },
                Control::Link,
                cx,
            ),
        );
    }
    // Reduced motion never advances frames, so there is nothing to pause.
    if animated && !cx.reduce_motion() {
        strip = strip.child(controls.chip(
            "image_pause",
            if paused { "Resume" } else { "Pause to inspect" }.into(),
            "Pause on the current frame to inspect original pixels",
            Control::Pause,
            cx,
        ));
    }
    strip.into_any_element()
}
