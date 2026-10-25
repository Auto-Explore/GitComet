use super::*;
use crate::ui_scale::chrome_scale;

/// AppKit centres the traffic lights inside a container it sizes as
/// `button_height + 2 * position.y` and pins to the top of the window (gpui
/// `move_traffic_light`). So the y inset is the only thing that decides
/// whether the lights share a midline with our bar -- get it wrong and they
/// sit high in the bar with no other symptom.
#[test]
fn the_macos_traffic_lights_share_the_title_bars_midline() {
    let position = macos_traffic_light_position();
    let container = px(MACOS_TRAFFIC_LIGHT_BUTTON_HEIGHT_PX) + position.y * 2.0;

    assert_eq!(
        container, TITLE_BAR_HEIGHT,
        "the container AppKit builds must be exactly the bar the lights sit in"
    );
}

/// `chrome_scale` is what pins every shared component drawn into a bar, so
/// it has to be a genuine no-op on both axes -- if the app's defaults ever
/// move under it, the chrome moves with them.
#[test]
fn the_chrome_scale_is_the_identity_on_both_axes() {
    let scale = chrome_scale();

    assert_eq!(scale.px(20.0), px(20.0), "the chrome must not zoom");
    assert_eq!(
        scale.appearance,
        crate::appearance::Appearance {
            density: crate::appearance::UiDensity::Compact,
            ..crate::appearance::Appearance::default()
        },
        "the chrome must not take density or font size"
    );
    assert_eq!(scale.row_height(24.0, 32.0), px(24.0));
    assert_eq!(scale.ui_text(15.0), px(15.0));
}

#[test]
fn titlebar_buttons_do_not_double_set_hover_style() {
    let theme = AppTheme::gitcomet_dark();
    assert!(
        std::panic::catch_unwind(|| {
            let _ = titlebar_control_button(
                "test_btn_1",
                "icons/generic_minimize.svg",
                theme.colors.foreground.secondary,
                theme.colors.foreground.primary,
            );
        })
        .is_ok()
    );
    assert!(
        std::panic::catch_unwind(|| {
            let _ = titlebar_control_button(
                "test_btn_2",
                "icons/generic_close.svg",
                theme.colors.foreground.secondary,
                theme.colors.status.danger.foreground,
            );
        })
        .is_ok()
    );
}

/// Popovers fit inside the same padding the frame draws, so the two can
/// only agree if a tiled edge loses its decoration inset in both.
#[test]
fn frame_insets_pad_only_the_untiled_edges_of_a_client_frame() {
    let inset = px(10.0);
    assert_eq!(
        frame_insets(Decorations::Server, inset),
        gpui::Edges::default()
    );
    for bits in 0..16u8 {
        let tiling = gpui::Tiling {
            top: bits & 1 != 0,
            left: bits & 2 != 0,
            right: bits & 4 != 0,
            bottom: bits & 8 != 0,
        };
        let side = |tiled: bool| if tiled { px(0.0) } else { inset };
        assert_eq!(
            frame_insets(Decorations::Client { tiling }, inset),
            gpui::Edges {
                top: side(tiling.top),
                right: side(tiling.right),
                bottom: side(tiling.bottom),
                left: side(tiling.left),
            },
            "{tiling:?}"
        );
    }
}

#[test]
fn window_frame_visual_inset_matches_platform_chrome_strategy() {
    #[cfg(target_os = "macos")]
    assert_eq!(
        window_frame_visual_inset(ui_scale::DEFAULT_UI_SCALE_PERCENT),
        px(0.0)
    );
    #[cfg(not(target_os = "macos"))]
    assert_eq!(
        window_frame_visual_inset(ui_scale::DEFAULT_UI_SCALE_PERCENT),
        CLIENT_SIDE_DECORATION_INSET
    );
}

#[test]
fn window_frame_outline_color_tracks_platform_and_theme() {
    let dark = AppTheme::gitcomet_dark();
    let light = AppTheme::gitcomet_light();

    #[cfg(target_os = "macos")]
    {
        assert_eq!(
            window_frame_outline_color(dark),
            with_alpha(dark.colors.stroke.default, 0.96)
        );
        assert_eq!(
            window_frame_outline_color(light),
            with_alpha(light.colors.stroke.default, 0.90)
        );
    }

    #[cfg(not(target_os = "macos"))]
    {
        assert_eq!(window_frame_outline_color(dark), dark.colors.stroke.default);
        assert_eq!(
            window_frame_outline_color(light),
            light.colors.stroke.default
        );
    }
}

#[test]
fn window_frame_outline_is_omitted_on_windows() {
    #[cfg(target_os = "windows")]
    assert!(!should_draw_window_frame_outline());
    #[cfg(not(target_os = "windows"))]
    assert!(should_draw_window_frame_outline());
}

#[test]
fn titlebar_drag_state_tracks_single_clicks_and_suppresses_double_click_drags() {
    let mut state = TitleBarDragState::default();

    state.on_left_mouse_down(1);
    assert!(state.should_move, "single click should arm a window move");

    state.on_left_mouse_down(2);
    assert!(
        !state.should_move,
        "double click should suppress drag tracking so it can toggle zoom instead"
    );
}

#[test]
fn titlebar_drag_state_move_request_is_consumed_once() {
    let mut state = TitleBarDragState::default();
    state.on_left_mouse_down(1);

    assert!(
        state.take_move_request(),
        "the first mouse move after pressing the title bar should start a window move"
    );
    assert!(
        !state.take_move_request(),
        "move tracking should clear after the move request is consumed"
    );
}

#[test]
fn titlebar_double_click_requires_standard_double_click() {
    assert!(should_handle_titlebar_double_click(2, true));
    assert!(!should_handle_titlebar_double_click(1, true));
    assert!(!should_handle_titlebar_double_click(3, true));
    assert!(!should_handle_titlebar_double_click(2, false));
}

#[test]
fn titlebar_double_click_action_matches_platform_convention() {
    let expected = if cfg!(target_os = "macos") {
        TitleBarDoubleClickAction::PlatformDefault
    } else {
        TitleBarDoubleClickAction::ToggleZoom
    };

    assert_eq!(titlebar_double_click_action(), expected);
}

#[test]
fn window_zoom_action_restores_only_on_windows_when_already_maximized() {
    assert_eq!(window_zoom_action(false), WindowZoomAction::Zoom);

    let expected = if cfg!(target_os = "windows") {
        WindowZoomAction::Restore
    } else {
        WindowZoomAction::Zoom
    };
    assert_eq!(window_zoom_action(true), expected);
}

#[test]
fn window_menu_position_scales_logical_pixels_to_device_pixels() {
    assert_eq!(
        window_menu_position(point(px(12.4), px(7.6)), 1.25),
        (16, 10)
    );
}
