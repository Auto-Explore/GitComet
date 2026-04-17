use gitcomet_state::session;
use gpui::{BorrowAppContext, Pixels, Size, Window, px, size};

pub(crate) const DEFAULT_UI_SCALE_PERCENT: u32 = 100;
pub(crate) const UI_SCALE_PRESETS: &[u32] = &[80, 90, 100, 110, 125, 150, 175, 200];

const BASE_REM_PX: f32 = 16.0;
const MIN_UI_SCALE_PERCENT: u32 = 80;
const MAX_UI_SCALE_PERCENT: u32 = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AppUiScale {
    pub(crate) percent: u32,
    pub(crate) initialized: bool,
}

impl Default for AppUiScale {
    fn default() -> Self {
        Self {
            percent: DEFAULT_UI_SCALE_PERCENT,
            initialized: false,
        }
    }
}

impl gpui::Global for AppUiScale {}

pub(crate) fn current<C>(cx: &mut C) -> AppUiScale
where
    C: BorrowAppContext,
{
    cx.update_default_global::<AppUiScale, _>(|scale, _cx| *scale)
}

pub(crate) fn current_or_initialize_from_session<C>(
    ui_session: &session::UiSession,
    cx: &mut C,
) -> AppUiScale
where
    C: BorrowAppContext,
{
    let current = current(cx);
    if current.initialized {
        return current;
    }

    let next = AppUiScale {
        percent: sanitize_percent(ui_session.ui_scale_percent),
        initialized: true,
    };
    cx.set_global(next);
    next
}

pub(crate) fn set_current<C>(cx: &mut C, percent: u32) -> AppUiScale
where
    C: BorrowAppContext,
{
    let next = AppUiScale {
        percent: sanitize_percent(Some(percent)),
        initialized: true,
    };
    cx.set_global(next);
    next
}

pub(crate) fn sanitize_percent(percent: Option<u32>) -> u32 {
    percent
        .unwrap_or(DEFAULT_UI_SCALE_PERCENT)
        .clamp(MIN_UI_SCALE_PERCENT, MAX_UI_SCALE_PERCENT)
}

pub(crate) fn label(percent: u32) -> String {
    format!("{}%", sanitize_percent(Some(percent)))
}

pub(crate) fn step_up(current: u32) -> u32 {
    let current = sanitize_percent(Some(current));
    UI_SCALE_PRESETS
        .iter()
        .copied()
        .find(|percent| *percent > current)
        .unwrap_or(current)
}

pub(crate) fn step_down(current: u32) -> u32 {
    let current = sanitize_percent(Some(current));
    UI_SCALE_PRESETS
        .iter()
        .rev()
        .copied()
        .find(|percent| *percent < current)
        .unwrap_or(current)
}

pub(crate) fn rem_size_for_percent(percent: u32) -> Pixels {
    px(BASE_REM_PX * design_scale_factor_from_percent(percent))
}

pub(crate) fn apply_to_window(window: &mut Window, percent: u32) {
    window.set_rem_size(rem_size_for_percent(percent));
}

pub(crate) fn design_scale_factor_from_percent(percent: u32) -> f32 {
    sanitize_percent(Some(percent)) as f32 / DEFAULT_UI_SCALE_PERCENT as f32
}

pub(crate) fn design_scale_factor_from_window(window: &Window) -> f32 {
    let rem_size: f32 = window.rem_size().into();
    rem_size / BASE_REM_PX
}

pub(crate) fn design_px<C>(value: f32, cx: &mut C) -> Pixels
where
    C: BorrowAppContext,
{
    design_px_from_percent(value, current(cx).percent)
}

pub(crate) fn design_px_from_percent(value: f32, percent: u32) -> Pixels {
    px(value * design_scale_factor_from_percent(percent))
}

pub(crate) fn design_px_from_window(value: f32, window: &Window) -> Pixels {
    px(value * design_scale_factor_from_window(window))
}

pub(crate) fn design_size_from_percent(width: f32, height: f32, percent: u32) -> Size<Pixels> {
    size(
        design_px_from_percent(width, percent),
        design_px_from_percent(height, percent),
    )
}

pub(crate) fn rescale_pixels(value: Pixels, from_percent: u32, to_percent: u32) -> Pixels {
    if from_percent == to_percent {
        return value;
    }

    let raw_value: f32 = value.into();
    px(raw_value * design_scale_factor_from_percent(to_percent)
        / design_scale_factor_from_percent(from_percent))
}

pub(crate) fn rescale_optional_u32(
    value: Option<u32>,
    from_percent: u32,
    to_percent: u32,
) -> Option<u32> {
    let value = value?;
    let scaled = rescale_pixels(px(value as f32), from_percent, to_percent);
    let scaled: f32 = scaled.round().into();
    (scaled.is_finite() && scaled >= 1.0).then_some(scaled as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_scale_steps_follow_presets() {
        assert_eq!(step_down(80), 80);
        assert_eq!(step_down(100), 90);
        assert_eq!(step_up(100), 110);
        assert_eq!(step_up(150), 175);
        assert_eq!(step_up(175), 200);
        assert_eq!(step_up(200), 200);
    }

    #[test]
    fn ui_scale_rescaling_uses_percent_ratio() {
        assert_eq!(rescale_optional_u32(Some(200), 100, 125), Some(250));
        assert_eq!(rescale_optional_u32(Some(250), 125, 100), Some(200));
    }
}
