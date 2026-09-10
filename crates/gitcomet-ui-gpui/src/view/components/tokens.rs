use crate::ui_scale::UiScale;

pub const CONTROL_HEIGHT_PX: f32 = 22.0;
/// Medium control height
pub const CONTROL_HEIGHT_MD_PX: f32 = 28.0;

/// Default horizontal padding for text buttons.
pub const CONTROL_PAD_X_PX: f32 = 10.0;
pub const CONTROL_PAD_X_COMFORTABLE_PX: f32 = 12.0;
/// Default vertical padding for text buttons.
pub const CONTROL_PAD_Y_PX: f32 = 3.0;

/// Horizontal padding for icon-only buttons.
pub const ICON_PAD_X_PX: f32 = 6.0;

/// Horizontal inset applied to a list row's selection/hover highlight so the
/// rounded background reads as an inset pill/card rather than a full-bleed band.
pub const ROW_HIGHLIGHT_INSET_PX: f32 = 6.0;

/// Height of the divider between a split button's two halves. Deliberately far
/// short of the control height: each half now draws its own hover border, so a
/// full-height rule would read as a third frame rather than a seam.
pub const SPLIT_BUTTON_DIVIDER_HEIGHT_PX: f32 = 11.0;

/// Trailing close/remove affordance shared by repository tabs and the picker
/// rows that can drop an entry: a small hit box holding a danger-tinted X,
/// whose plate is the danger colour at these alphas. Both live off the same
/// tokens, so the two buttons read as one control -- but only the picker's
/// takes the density ramp on top. The tab's sits in the window chrome, which
/// holds `REMOVE_BUTTON_SIZE_PX` at every setting.
pub const REMOVE_BUTTON_ICON: &str = "icons/repo_tab_close.svg";
pub const REMOVE_BUTTON_SIZE_PX: f32 = 18.0;
pub const REMOVE_BUTTON_ICON_SIZE_PX: f32 = 12.0;
pub const REMOVE_BUTTON_HOVER_ALPHA: f32 = 0.18;
pub const REMOVE_BUTTON_PRESSED_ALPHA: f32 = 0.26;

pub fn control_height(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().row_height(CONTROL_HEIGHT_PX, 32.0)
}

pub fn control_height_md(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().row_height(CONTROL_HEIGHT_MD_PX, 32.0)
}

/// Height of a content header bar: the tallest control it holds plus equal
/// breathing room above and below, so a denser control lifts the bar with it.
pub fn content_header_height(scale: impl Into<UiScale>) -> gpui::Pixels {
    let scale = scale.into();
    control_height(scale) + control_pad_y(scale) * 2.0
}

/// Height for a control nested inside a list row or a tab: shorter than a
/// standalone one, so the row stays visible around it.
pub const IN_ROW_CONTROL_HEIGHT_PX: f32 = 20.0;
pub const IN_ROW_CONTROL_COMFORTABLE_HEIGHT_PX: f32 = 26.0;

pub fn in_row_control_height(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().row_height(
        IN_ROW_CONTROL_HEIGHT_PX,
        IN_ROW_CONTROL_COMFORTABLE_HEIGHT_PX,
    )
}

pub fn control_pad_x(scale: impl Into<UiScale>) -> gpui::Pixels {
    let scale = scale.into();
    scale.px(scale
        .appearance
        .ramp(CONTROL_PAD_X_PX, CONTROL_PAD_X_COMFORTABLE_PX))
}

pub fn control_pad_y(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().px(CONTROL_PAD_Y_PX)
}

pub fn icon_pad_x(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().px(ICON_PAD_X_PX)
}

pub fn split_button_divider_height(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().px(SPLIT_BUTTON_DIVIDER_HEIGHT_PX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::{Appearance, UiDensity};

    fn scale(density: UiDensity) -> UiScale {
        UiScale::from_percent(100).with_appearance(Appearance {
            density,
            ..Appearance::default()
        })
    }

    /// A header sized to its own controls leaves them flush against both edges.
    #[test]
    fn a_content_header_clears_the_controls_it_holds() {
        for density in UiDensity::ALL {
            let scale = scale(density);

            assert!(
                content_header_height(scale) > control_height(scale),
                "{density:?} header must leave room around a control"
            );
            assert_eq!(
                content_header_height(scale) - control_height(scale),
                control_pad_y(scale) * 2.0,
                "{density:?} header must keep equal air above and below"
            );
        }
    }

    /// A control nested in a row must leave the row visible around it, or the
    /// row reads as one solid block.
    #[test]
    fn an_in_row_control_leaves_air_in_the_row() {
        for density in UiDensity::ALL {
            let scale = scale(density);
            let row = scale.row_height(24.0, 32.0);

            assert!(
                in_row_control_height(scale) < row,
                "{density:?} in-row control must stop short of its {row:?} row"
            );
            assert!(
                in_row_control_height(scale) < control_height(scale),
                "{density:?} in-row control must be shorter than a standalone one"
            );
        }
        assert!(
            in_row_control_height(scale(UiDensity::Comfortable))
                > in_row_control_height(scale(UiDensity::Compact))
        );
    }

    /// A density step is only useful if it reaches the controls.
    #[test]
    fn every_density_step_grows_controls_and_headers() {
        let tokens: [(&str, fn(UiScale) -> gpui::Pixels); 5] = [
            ("control_height", control_height),
            ("control_height_md", control_height_md),
            ("content_header_height", content_header_height),
            ("control_pad_x", control_pad_x),
            ("in_row_control_height", in_row_control_height),
        ];

        for (name, measure) in tokens {
            let sizes: Vec<_> = UiDensity::ALL
                .into_iter()
                .map(|density| measure(scale(density)))
                .collect();

            assert!(
                sizes.windows(2).all(|step| step[1] > step[0]),
                "{name} must grow at every density step, got {sizes:?}"
            );
        }
    }
}
