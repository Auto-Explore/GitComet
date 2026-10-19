//! Window chrome: the title bars, their controls, and the frame around them.
//!
//! A title bar sits outside the appearance system. It shares its row with the OS
//! window controls, which do not resize, so the bar, its buttons and the
//! repository tabs hold one size at every UI scale, density and font size. Bar
//! geometry is written in literal `px`; the shared components a bar borrows are
//! handed [`chrome_scale`] so they size themselves the same way.
//!
//! That covers the bar and what it draws. The window *frame* around it --
//! [`client_side_decoration_inset`] and the resize band derived from it -- is a
//! pointer target on the window edge rather than something in the bar, and
//! deliberately still follows the UI scale.
use super::*;
use crate::kit::click::PointerClickExt as _;
use crate::kit::window_chrome::{
    MACOS_TRAFFIC_LIGHTS_SAFE_INSET, TITLE_BAR_BUTTON_HEIGHT, TITLE_BAR_BUTTON_WIDTH_PX,
    TITLE_BAR_HEIGHT, TITLE_BAR_ICON_SIZE_PX, TitleBarDragState, client_frame_corner_rounding,
    handle_titlebar_double_click, should_handle_titlebar_double_click,
    show_titlebar_secondary_menu, titlebar_control_button, window_controls_cluster,
    window_top_left_corner,
};
use crate::view::components::{ControlInteractionExt, InteractionState, InteractionStyle};

use std::cell::Cell;
use std::rc::Rc;

/// Empty title-bar width kept beside repository tabs so a full tab strip still
/// leaves somewhere to grab the window. Deliberately near the smallest usable
/// grab target: every pixel here is width the tab strip can never use, so it
/// narrows tabs before the bar is even full.
const REPO_TABS_TRAILING_DRAG_WIDTH_PX: f32 = 24.0;
/// The scale a shared component is handed when it is drawn into a bar. One
/// definition, so a component cannot end up pinned to a different baseline than
/// the bar around it.
pub(in crate::view) use crate::ui_scale::chrome_scale;

/// Fallback size for text in a bar that does not set its own, so it cannot
/// inherit the app root's rem-based size and grow with the UI scale.
const TITLE_BAR_TEXT_SIZE_PX: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::view) enum TitleBarMode {
    Standard,
    Gated,
}

pub(super) struct TitleBarView {
    mode: TitleBarMode,
    brand: Option<gpui::AnyView>,
    theme: AppTheme,
    root_view: WeakEntity<GitCometView>,
    title_drag_state: TitleBarDragState,
    app_menu_open: bool,
    app_menu_focus_handle: FocusHandle,
    repo_picker_open: bool,
    repo_tab_actions_enabled: bool,
    /// Painted bounds of the repository switcher chevron, so opening the picker
    /// from the keyboard can anchor to the same control the mouse uses.
    repo_picker_toggle_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

fn main_window_control(
    button: gpui::WindowButton,
    is_maximized: bool,
    theme: AppTheme,
    cx: &mut gpui::Context<TitleBarView>,
) -> AnyElement {
    match button {
        gpui::WindowButton::Minimize => titlebar_control_button(
            "win_min_btn",
            "icons/generic_minimize.svg",
            theme.colors.foreground.secondary,
            theme.colors.foreground.primary,
        )
        .id("win_min")
        .debug_selector(|| "titlebar_win_min".to_string())
        .window_control_area(WindowControlArea::Min)
        .gitcomet_tooltip(theme, SharedString::from("Minimize window"))
        .on_activate(
            false,
            components::ControlActivation::Action,
            cx.listener(|_this, _e: &ClickEvent, window, cx| {
                cx.stop_propagation();
                window.minimize_window();
            }),
        )
        .into_any_element(),
        gpui::WindowButton::Maximize => {
            let (icon, tooltip) = if is_maximized {
                ("icons/generic_restore.svg", "Restore window")
            } else {
                ("icons/generic_maximize.svg", "Maximize window")
            };
            titlebar_control_button(
                "win_max_btn",
                icon,
                theme.colors.foreground.secondary,
                theme.colors.foreground.primary,
            )
            .id("win_max")
            .debug_selector(|| "titlebar_win_max".to_string())
            .window_control_area(WindowControlArea::Max)
            .gitcomet_tooltip(theme, SharedString::from(tooltip))
            .on_activate(
                false,
                components::ControlActivation::Action,
                cx.listener(|_this, _e: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    crate::kit::window_chrome::toggle_window_zoom(window);
                    cx.notify();
                }),
            )
            .into_any_element()
        }
        gpui::WindowButton::Close => titlebar_control_button(
            "win_close_btn",
            "icons/generic_close.svg",
            theme.colors.foreground.secondary,
            theme.colors.status.danger.foreground,
        )
        .id("win_close")
        .debug_selector(|| "titlebar_win_close".to_string())
        .window_control_area(WindowControlArea::Close)
        .gitcomet_tooltip(theme, SharedString::from("Close window"))
        .on_activate(
            false,
            components::ControlActivation::Action,
            cx.listener(|_this, _e: &ClickEvent, window, cx| {
                cx.stop_propagation();
                crate::app::close_window_or_warn(window, cx);
            }),
        )
        .into_any_element(),
    }
}

fn mix(mut a: gpui::Rgba, b: gpui::Rgba, t: f32) -> gpui::Rgba {
    let t = t.clamp(0.0, 1.0);
    a.red = a.red + (b.red - a.red) * t;
    a.green = a.green + (b.green - a.green) * t;
    a.blue = a.blue + (b.blue - a.blue) * t;
    a.alpha = a.alpha + (b.alpha - a.alpha) * t;
    a
}

fn lighten(color: gpui::Rgba, amount: f32) -> gpui::Rgba {
    mix(color, gpui::rgba(0xFFFFFFFF), amount)
}

/// A workspace's dot and tint colour. Fixed per option, never theme-derived,
/// so a dot always shows the option that was picked; Default dots are Blue.
pub(in crate::view) fn workspace_color(
    color: Option<gitcomet_state::session::WorkspaceColor>,
) -> gpui::Rgba {
    use gitcomet_state::session::WorkspaceColor;
    match color {
        Some(WorkspaceColor::Gray) => gpui::rgba(0x8E949CFF),
        Some(WorkspaceColor::Brown) => gpui::rgba(0xA47551FF),
        Some(WorkspaceColor::Red) => gpui::rgba(0xE05252FF),
        Some(WorkspaceColor::Orange) => gpui::rgba(0xE08A3EFF),
        Some(WorkspaceColor::Yellow) => gpui::rgba(0xD2A83AFF),
        Some(WorkspaceColor::Lime) => gpui::rgba(0x90B43CFF),
        Some(WorkspaceColor::Green) => gpui::rgba(0x48A868FF),
        Some(WorkspaceColor::Teal) => gpui::rgba(0x2E9E8FFF),
        Some(WorkspaceColor::Cyan) => gpui::rgba(0x39B0D0FF),
        None | Some(WorkspaceColor::Blue) => gpui::rgba(0x4B8DDBFF),
        Some(WorkspaceColor::Indigo) => gpui::rgba(0x4E66D0FF),
        Some(WorkspaceColor::Purple) => gpui::rgba(0x956EDBFF),
        Some(WorkspaceColor::Magenta) => gpui::rgba(0xC757C7FF),
        Some(WorkspaceColor::Pink) => gpui::rgba(0xD866A4FF),
    }
}

/// The title bar's fill. The active window lifts it off the workspace surface;
/// an inactive one drops back to it. Repo tabs sit on this color, so anything
/// that has to blend into the bar (the label fade, for one) asks here.
pub(in crate::view) fn title_bar_background(
    theme: AppTheme,
    window_is_active: bool,
    group_color: Option<gitcomet_state::session::WorkspaceColor>,
) -> gpui::Rgba {
    let base = if window_is_active {
        lighten(
            theme.colors.surface.panel,
            if theme.is_dark { 0.06 } else { 0.03 },
        )
    } else {
        theme.colors.surface.panel
    };
    let Some(group_color) = group_color else {
        return base;
    };

    // Keep text, tabs and window controls in the theme's contrast system while
    // making groups visibly distinct. Active windows carry a little more tint;
    // inactive ones remain identifiable without competing for attention.
    mix(
        base,
        workspace_color(Some(group_color)),
        if window_is_active { 0.18 } else { 0.12 },
    )
}

/// A workspace's row in a popover list wears its active title bar's tint,
/// flattened onto the popover surface so hover and selection can composite over
/// it. `None` when no colour is chosen: the row stays plain.
pub(in crate::view) fn workspace_row_tint(
    color: Option<gitcomet_state::session::WorkspaceColor>,
    theme: AppTheme,
) -> Option<gpui::Rgba> {
    let color = color?;
    Some(crate::theme::composite_over(
        theme.colors.surface.raised,
        title_bar_background(theme, true, Some(color)),
    ))
}

impl TitleBarView {
    pub(super) fn new(
        theme: AppTheme,
        root_view: WeakEntity<GitCometView>,
        repo_tab_actions_enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        Self {
            theme,
            root_view,
            mode: TitleBarMode::Standard,
            brand: None,
            title_drag_state: TitleBarDragState::default(),
            app_menu_open: false,
            app_menu_focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
            repo_picker_open: false,
            repo_tab_actions_enabled,
            repo_picker_toggle_bounds: Rc::new(Cell::new(None)),
        }
    }

    pub(in crate::view) fn set_brand(
        &mut self,
        brand: Option<gpui::AnyView>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.brand = brand;
        cx.notify();
    }

    pub(in crate::view) fn set_mode(&mut self, mode: TitleBarMode, cx: &mut gpui::Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            cx.notify();
        }
    }

    pub(super) fn repo_picker_toggle_bounds(&self) -> Option<Bounds<Pixels>> {
        self.repo_picker_toggle_bounds.get()
    }

    #[cfg(test)]
    pub(in crate::view) fn app_menu_focus_handle_for_test(&self) -> FocusHandle {
        self.app_menu_focus_handle.clone()
    }

    #[cfg(test)]
    pub(in crate::view) fn title_drag_armed_for_test(&self) -> bool {
        self.title_drag_state.is_move_armed()
    }

    pub(super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(super) fn set_app_menu_open(&mut self, open: bool, cx: &mut gpui::Context<Self>) {
        if self.app_menu_open == open {
            return;
        }
        self.app_menu_open = open;
        cx.notify();
    }

    pub(super) fn set_repo_picker_open(&mut self, open: bool, cx: &mut gpui::Context<Self>) {
        if self.repo_picker_open == open {
            return;
        }
        self.repo_picker_open = open;
        cx.notify();
    }

    pub(super) fn set_repo_tab_actions_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.repo_tab_actions_enabled == enabled {
            return;
        }
        self.repo_tab_actions_enabled = enabled;
        if !enabled {
            // The app menu stays reachable from Home; only the picker closes.
            self.repo_picker_open = false;
        }
        cx.notify();
    }

    fn open_popover_at(
        &mut self,
        kind: impl Into<PopoverRequest>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_at(kind, anchor, window, cx);
        });
    }

    fn open_popover_for_bounds(
        &mut self,
        kind: impl Into<PopoverRequest>,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_for_bounds(kind, anchor_bounds, window, cx);
        });
    }
}

impl Render for TitleBarView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let is_macos = cfg!(target_os = "macos");
        let repo_tab_actions_enabled =
            self.mode == TitleBarMode::Standard && self.repo_tab_actions_enabled;
        let root_view_mode = self.root_view.upgrade().map(|root| root.read(cx).view_mode);
        let repo_tabs_enabled =
            repo_tab_actions_enabled && root_view_mode.is_some_and(show_titlebar_repo_tabs);
        // Normal windows always offer the app menu, Home included.
        let app_menu_enabled = root_view_mode.is_some_and(renders_full_chrome);
        let app_menu_open = self.app_menu_open;
        let app_menu_open_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.30 } else { 0.24 },
        );
        let app_menu_hover_bg = theme.titlebar_hover_overlay();
        let app_menu_active_bg = theme.titlebar_active_overlay();
        // Matches `ButtonStyle::Transparent`'s hover border exactly, so the
        // hand-rolled repo-picker div and the `Button`s either side of it grow
        // the same outline under the cursor.
        let titlebar_hover_border = with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.40 } else { 0.30 },
        );
        let workspace = crate::workspaces::with_workspace_for_window(
            cx,
            window.window_handle().window_id(),
            |workspace| (workspace.display_name(), workspace.color),
        );
        let workspace_chip_visible = self.mode == TitleBarMode::Standard && workspace.is_some();
        let bar_bg = title_bar_background(
            theme,
            window.is_window_active(),
            workspace.as_ref().and_then(|(_, color)| *color),
        );
        let app_menu_focus_handle = self.app_menu_focus_handle.clone();

        let menu_toggle = div().h_full().pl(px(2.0)).flex().items_center().child(
            components::Button::new("app_menu_btn", "")
                .start_slot(svg_icon(
                    "icons/menu.svg",
                    theme.colors.foreground.primary,
                    px(TITLE_BAR_ICON_SIZE_PX),
                ))
                .style(components::ButtonStyle::Transparent)
                .open(app_menu_open)
                .selected_bg(app_menu_open_bg)
                .focus_handle(app_menu_focus_handle)
                .unscaled()
                .on_click(theme, cx, |this, _e, window, cx| {
                    let anchor = window_top_left_corner(window);
                    this.open_popover_at(PopoverKind::AppMenu, anchor, window, cx);
                })
                // The button's intrinsic `icon_pad_x` is narrower than the
                // shared hitbox, so state the size rather than inherit it.
                .h(TITLE_BAR_BUTTON_HEIGHT)
                .w(px(TITLE_BAR_BUTTON_WIDTH_PX))
                .rounded(px(theme.radii.control))
                .block_mouse_except_scroll()
                .debug_selector(|| "app_menu".to_string())
                .gitcomet_tooltip(theme, "Application menu".into()),
        );

        // The group name and color identify this window beside the repository
        // tabs and open the repository picker.
        let repo_picker_open = self.repo_picker_open;
        let workspace_dot = workspace_color(workspace.as_ref().and_then(|(_, color)| *color));
        let workspace_label: SharedString = workspace
            .map(|(name, _)| name)
            .unwrap_or_else(|| "Workspace".to_string())
            .into();
        let workspace_tooltip: SharedString = format!("Workspace: {workspace_label}").into();
        let repo_picker_toggle_bounds_for_prepaint = Rc::clone(&self.repo_picker_toggle_bounds);
        let repo_picker_toggle_bounds_for_click = Rc::clone(&self.repo_picker_toggle_bounds);
        let repo_picker_toggle = div()
            .h_full()
            .flex()
            .items_center()
            .on_children_prepainted(move |children_bounds, _window, _cx| {
                repo_picker_toggle_bounds_for_prepaint.set(children_bounds.first().copied());
            })
            .child(
                div()
                    .id("repo_picker_btn")
                    .debug_selector(|| "repo_picker_toggle".to_string())
                    .h(TITLE_BAR_BUTTON_HEIGHT)
                    .max_w(px(190.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.0))
                    .rounded(px(theme.radii.control))
                    // Reserved at rest so gaining the hover outline does not
                    // shift the chevron by a pixel.
                    .border_1()
                    .border_color(gpui::transparent_black())
                    // Stay lit in the pressed/open color while the picker popover
                    // is open, mirroring the app-menu button.
                    .tab_index(0)
                    .control_interaction(
                        InteractionStyle::new(theme)
                            .persistent_background(app_menu_open_bg)
                            .hover(
                                StyleRefinement::default()
                                    .bg(app_menu_hover_bg)
                                    .border_color(titlebar_hover_border),
                            )
                            .pressed(StyleRefinement::default().bg(app_menu_active_bg)),
                        InteractionState::default().open(repo_picker_open),
                    )
                    .child(
                        div()
                            .size(px(8.0))
                            .flex_none()
                            .rounded_full()
                            .bg(workspace_dot),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(workspace_label),
                    )
                    .child(svg_icon(
                        "icons/chevron_down.svg",
                        theme.colors.foreground.primary,
                        px(TITLE_BAR_ICON_SIZE_PX),
                    ))
                    .block_mouse_except_scroll()
                    .on_activate(
                        false,
                        components::ControlActivation::Action,
                        cx.listener(move |this, e: &ClickEvent, window, cx| {
                            let anchor_bounds = repo_picker_toggle_bounds_for_click
                                .get()
                                .unwrap_or_else(|| {
                                    Bounds::new(e.position(), gpui::size(px(0.0), px(0.0)))
                                });
                            this.open_popover_for_bounds(
                                PopoverKind::RepoPicker {
                                    scope: RepoPickerScope::All,
                                },
                                anchor_bounds,
                                window,
                                cx,
                            );
                        }),
                    )
                    .gitcomet_tooltip(theme, workspace_tooltip),
            );

        // One drag surface spans the title bar underneath its controls. Each
        // visible control occludes only its painted bounds, so the uncovered
        // title above and below it behaves like ordinary window chrome.
        //
        // Deliberately *not* a `WindowControlArea::Drag`. gpui answers Windows'
        // WM_NCHITTEST with the first hovered window-control area in paint
        // order, testing membership against the whole hit-test list rather than
        // its hovered prefix — so a full-bleed drag area painted under the bar
        // claims HTCAPTION for every control on top of it, and Windows runs its
        // own SC_MOVE loop instead of delivering the click. That froze the
        // repo tabs, the app menu, and the repo picker. Marking each control
        // `occlude()` would fix the lookup but would also cut the tab strip off
        // from wheel events, which is exactly what `block_mouse_except_scroll`
        // is there to preserve. Dragging instead goes through the handlers
        // below, which is already the only path on Linux (window control areas
        // are a no-op there) and reaches the same native SC_MOVE on Windows.
        let drag_surface = div()
            .id("title_drag")
            .debug_selector(|| "titlebar_drag".to_string())
            .absolute()
            .inset_0()
            .on_activate(
                false,
                components::ControlActivation::Composite,
                cx.listener(|this, e: &ClickEvent, window, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                    if !should_handle_titlebar_double_click(e.click_count(), e.standard_click()) {
                        return;
                    }
                    cx.stop_propagation();
                    handle_titlebar_double_click(window);
                }),
            )
            // The system menu follows the same completed-click rule as controls.
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(|_this, e: &MouseDownEvent, window, cx| {
                    show_titlebar_secondary_menu(e.position, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _w, cx| {
                    crate::press_gesture::claim_press(cx);
                    crate::text_selection_owner::preserve(cx);
                    this.title_drag_state.on_left_mouse_down(e.click_count);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, _e, window, _cx| {
                if this.title_drag_state.take_move_request() {
                    crate::kit::window_chrome::begin_window_move(window);
                }
            }));

        let is_maximized = window.is_maximized();
        let control_layout = crate::kit::window_chrome::controls::resolve_visibility(
            crate::kit::window_chrome::controls::current(cx).mode,
            cx.button_layout(),
            cfg!(any(target_os = "linux", target_os = "freebsd")),
            window.window_decorations(),
            is_maximized,
        );
        let (left_controls, right_controls) = if is_macos {
            (Vec::new(), Vec::new())
        } else {
            let mut render = |button| main_window_control(button, is_maximized, theme, cx);
            (
                control_layout
                    .left
                    .into_iter()
                    .flatten()
                    .map(&mut render)
                    .collect::<Vec<_>>(),
                control_layout
                    .right
                    .into_iter()
                    .flatten()
                    .map(render)
                    .collect::<Vec<_>>(),
            )
        };
        let has_left_controls = !left_controls.is_empty();

        // Leading and trailing clusters center on the full bar height; tab
        // labels compensate for their bottom fusion (see `Tab::render`) so
        // icons and tab text share the bar's true midline.
        let leading = div()
            .flex()
            .items_center()
            .h_full()
            .gap(px(2.0))
            .when(is_macos, |d| d.pl(MACOS_TRAFFIC_LIGHTS_SAFE_INSET))
            .when(has_left_controls, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .h_full()
                        .gap(px(4.0))
                        .pl(px(8.0))
                        .children(left_controls),
                )
            })
            .when(!is_macos && app_menu_enabled, |d| d.child(menu_toggle))
            .children(self.brand.clone())
            // An empty customized workspace keeps its name and colour on Home.
            .when(repo_tab_actions_enabled || workspace_chip_visible, |d| {
                d.child(repo_picker_toggle)
            });

        // Browser-style: when repositories are open, the repo tabs live in the
        // title bar's middle. Keep a fixed draggable strip beside them so the
        // window can still be moved by the empty title-bar area.
        let repo_tabs = if repo_tabs_enabled {
            self.root_view
                .upgrade()
                .map(|root| root.read(cx).repo_tabs_bar.clone())
        } else {
            None
        };
        let middle: AnyElement = if let Some(repo_tabs) = repo_tabs {
            div()
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .overflow_hidden()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .h_full()
                        .overflow_hidden()
                        .child(repo_tabs),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .w(px(REPO_TABS_TRAILING_DRAG_WIDTH_PX))
                        .h_full(),
                )
                .into_any_element()
        } else {
            div().flex_1().h_full().into_any_element()
        };

        let frame_rounding = client_frame_corner_rounding(theme, window);
        div()
            .id("title_bar")
            .relative()
            .flex()
            .items_center()
            .h(TITLE_BAR_HEIGHT)
            .w_full()
            .text_size(px(TITLE_BAR_TEXT_SIZE_PX))
            .bg(bar_bg)
            .when_some(frame_rounding, |d, rounding| {
                d.when(rounding.top_left, |d| d.rounded_tl(rounding.radius))
                    .when(rounding.top_right, |d| d.rounded_tr(rounding.radius))
            })
            // The bar/content boundary line. Painted before the tabs so the
            // active tab (flush with the bar bottom, filled with the content
            // strip color) covers its segment and fuses into the action bar.
            .when(repo_tabs_enabled, |d| {
                d.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(1.0))
                        .bg(components::Tab::outline_color(theme)),
                )
            })
            .child(drag_surface)
            .child(leading)
            .child(middle)
            .child(window_controls_cluster(right_controls))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gpui's numbered spacing shorthands (`gap_1`, `pr_2`, `text_sm`, …) are
    /// rem-based, and the rem size follows the UI scale -- so a single one of
    /// them inside the chrome silently re-scales part of a bar that is supposed
    /// to hold one size. This is how the settings header's caption cluster kept
    /// growing after the rest of the bar was pinned. Needles are assembled at
    /// runtime so this test does not match its own source.
    #[test]
    fn the_chrome_never_reaches_for_a_rem_based_size() {
        const CHROME_SOURCES: [(&str, &str); 5] = [
            ("view/chrome.rs", include_str!("chrome.rs")),
            (
                "view/panels/repo_tabs_bar.rs",
                include_str!("panels/repo_tabs_bar.rs"),
            ),
            (
                "gitcomet-ui-kit/src/window_chrome/mod.rs",
                include_str!("../../../gitcomet-ui-kit/src/window_chrome/mod.rs"),
            ),
            (
                "gitcomet-ui-kit/src/components/tab.rs",
                include_str!("../../../gitcomet-ui-kit/src/components/tab.rs"),
            ),
            (
                "gitcomet-ui-kit/src/components/tab_bar.rs",
                include_str!("../../../gitcomet-ui-kit/src/components/tab_bar.rs"),
            ),
        ];
        // Length-taking builders only: `border_*` and `rounded_*` are px, and
        // `flex_1` is a grow factor, not a size.
        const LENGTH: &[&str] = &[
            "gap", "gap_x", "gap_y", "p", "px", "py", "pt", "pb", "pl", "pr", "m", "mx", "my",
            "mt", "mb", "ml", "mr", "w", "h", "size", "top", "bottom", "left", "right", "min_w",
            "min_h", "max_w", "max_h",
        ];
        // The `_0` step is px(0), so it is exempt.
        const STEPS: &[&str] = &[
            "0p5", "1", "1p5", "2", "2p5", "3", "3p5", "4", "5", "6", "8", "10", "12", "16", "20",
            "24",
        ];
        const TEXT: &[&str] = &["xs", "sm", "base", "lg", "xl", "2xl", "3xl"];

        let needles: Vec<String> = LENGTH
            .iter()
            .flat_map(|prefix| STEPS.iter().map(move |step| format!(".{prefix}_{step}(")))
            .chain(TEXT.iter().map(|size| format!(".text_{size}(")))
            .chain(std::iter::once(format!("{}(", "rems")))
            .collect();

        let mut offenders = Vec::new();
        for (name, source) in CHROME_SOURCES {
            for (ix, line) in source.lines().enumerate() {
                for needle in &needles {
                    if line.contains(needle.as_str()) {
                        offenders.push(format!("{name}:{} uses {needle}", ix + 1));
                    }
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "window chrome must size itself in fixed px: {offenders:#?}"
        );
    }

    #[test]
    fn configured_workspace_color_tints_the_title_bar_without_replacing_the_theme() {
        let distance = |a: gpui::Rgba, b: gpui::Rgba| {
            (a.red - b.red).abs() + (a.green - b.green).abs() + (a.blue - b.blue).abs()
        };
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let base = title_bar_background(theme, true, None);
            let blue = workspace_color(Some(gitcomet_state::session::WorkspaceColor::Blue));
            let tinted = title_bar_background(
                theme,
                true,
                Some(gitcomet_state::session::WorkspaceColor::Blue),
            );

            assert_ne!(tinted, base, "a configured group color must be visible");
            assert!(
                distance(tinted, blue) < distance(base, blue),
                "the configured title bar should move toward the selected color"
            );
            assert_eq!(
                title_bar_background(theme, true, None),
                base,
                "Default must preserve the existing theme-derived title bar"
            );
        }
    }

    #[test]
    fn default_workspace_dot_is_blue() {
        // The dot takes no theme, so Default can't borrow the theme accent.
        assert_eq!(
            workspace_color(None),
            workspace_color(Some(gitcomet_state::session::WorkspaceColor::Blue))
        );
    }

    #[test]
    fn workspace_row_tint_is_the_active_title_bar_and_opaque_in_every_theme() {
        for option in crate::theme::available_themes() {
            let theme = AppTheme::from_key(&option.key).expect("bundled theme");
            assert_eq!(workspace_row_tint(None, theme), None, "{}", option.key);
            for (color, label) in crate::workspaces::WORKSPACE_COLORS {
                let Some(color) = color else { continue };
                let tint = workspace_row_tint(Some(color), theme).expect("a chosen colour tints");
                // `InteractionStyle::on_surface` asserts an opaque backing.
                assert_eq!(tint.alpha, 1.0, "{} {label}", option.key);
                let title_bar = title_bar_background(theme, true, Some(color));
                if title_bar.alpha >= 1.0 {
                    assert_eq!(tint, title_bar, "{} {label}", option.key);
                }
            }
        }
    }
}
