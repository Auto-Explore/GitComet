use super::*;

/// A dialog's size: a share of the window, kept between a floor and a cap
/// and never larger than the window minus a margin.
pub(super) struct DialogSize {
    pub(super) width_px: f32,
    pub(super) height_px: f32,
    pub(super) max_width_px: f32,
    pub(super) max_height_px: f32,
    pub(super) width_fraction: f32,
    pub(super) height_fraction: f32,
}

const DIALOG_MARGIN_PX: f32 = 16.0;

impl DialogSize {
    /// The dialog's width and height in `window` at the popover's UI scale.
    pub(super) fn resolve(
        &self,
        window: &Window,
        cx: &mut gpui::Context<PopoverHost>,
    ) -> (Pixels, Pixels) {
        let scaled_px = crate::ui_scale::scaler(popover_ui_scale(cx));
        let window_size = crate::kit::window_chrome::window_surface_bounds(window).size;
        let margin = scaled_px(DIALOG_MARGIN_PX);
        let width = extent(
            (window_size.width - margin * 2.0).max(px(0.0)),
            scaled_px(self.width_px),
            scaled_px(self.max_width_px),
            self.width_fraction,
        );
        let height = extent(
            (window_size.height - margin * 2.0).max(px(0.0)),
            scaled_px(self.height_px),
            scaled_px(self.max_height_px),
            self.height_fraction,
        );
        (width, height)
    }
}

fn extent(available: Pixels, preferred: Pixels, max: Pixels, fraction: f32) -> Pixels {
    (available * fraction)
        .max(preferred)
        .min(max)
        .min(available)
}

/// `time` in the user's date, time and timezone settings.
pub(super) fn timestamp_label(this: &PopoverHost, time: std::time::SystemTime) -> String {
    let mut label = String::with_capacity(24);
    format_datetime_into(
        &mut label,
        time,
        this.date_time_format,
        this.timezone,
        this.show_timezone,
    );
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialog_grows_with_the_window_between_its_floor_and_cap() {
        let extent = |available: f32| extent(px(available), px(720.0), px(1200.0), 0.6);
        assert_eq!(extent(500.0), px(500.0), "never wider than the window");
        assert_eq!(extent(1000.0), px(720.0), "keeps its floor while it fits");
        assert_eq!(
            extent(1800.0),
            px(1080.0),
            "takes its share of a big window"
        );
        assert_eq!(extent(4000.0), px(1200.0), "and stops at the cap");
    }
}
