use gpui::{App, BorrowAppContext, Pixels, Size, Window, WindowId, px, size};
use rustc_hash::FxHashMap;

pub const DEFAULT_UI_SCALE_PERCENT: u32 = 100;
pub const UI_SCALE_PRESETS: &[u32] = &[80, 90, 100, 110, 125, 150, 175, 200];

const BASE_REM_PX: f32 = 16.0;
const MIN_UI_SCALE_PERCENT: u32 = 80;
const MAX_UI_SCALE_PERCENT: u32 = 200;

/// A UI scale. As the global it is the default UI scale (a setting), which
/// new windows and windows without their own zoom use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppUiScale {
    pub percent: u32,
    pub initialized: bool,
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

/// Zoom that windows set for themselves, for this session only.
#[derive(Default)]
struct WindowUiScales {
    overrides: FxHashMap<WindowId, u32>,
    /// Windows (pop-outs) that render at another window's zoom.
    leaders: FxHashMap<WindowId, WindowId>,
}

impl WindowUiScales {
    fn leader(&self, id: WindowId) -> WindowId {
        self.leaders.get(&id).copied().unwrap_or(id)
    }
}

impl gpui::Global for WindowUiScales {}

/// The scale of the window being drawn or handling an event; the default
/// outside any window update.
pub fn current<C>(cx: &mut C) -> AppUiScale
where
    C: std::borrow::BorrowMut<App>,
{
    let default = default_scale(cx);
    let app: &App = std::borrow::Borrow::<App>::borrow(&*cx);
    app.current_window_id()
        .and_then(|id| window_override(app, id))
        .map_or(default, |percent| AppUiScale {
            percent,
            initialized: true,
        })
}

/// The default UI scale.
pub fn default_scale<C>(cx: &mut C) -> AppUiScale
where
    C: BorrowAppContext,
{
    cx.update_default_global::<AppUiScale, _>(|scale, _cx| *scale)
}

/// The default UI scale, initialized once per app from the stored percent.
pub fn default_or_initialize<C>(stored_percent: Option<u32>, cx: &mut C) -> AppUiScale
where
    C: BorrowAppContext,
{
    let default = default_scale(cx);
    if default.initialized {
        return default;
    }
    set_default(cx, sanitize_percent(stored_percent))
}

pub fn set_default<C>(cx: &mut C, percent: u32) -> AppUiScale
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

/// The zoom window `id` set for itself (or its leader did), if any.
pub fn window_override(cx: &App, id: WindowId) -> Option<u32> {
    let scales = cx.try_global::<WindowUiScales>()?;
    scales.overrides.get(&scales.leader(id)).copied()
}

/// The default UI scale percent, where the context cannot be mutated.
pub fn default_percent(cx: &App) -> u32 {
    cx.try_global::<AppUiScale>()
        .map_or(DEFAULT_UI_SCALE_PERCENT, |scale| scale.percent)
}

/// The scale window `id` renders at.
pub fn percent_for_window(cx: &App, id: WindowId) -> u32 {
    window_override(cx, id).unwrap_or_else(|| default_percent(cx))
}

/// Gives window `id` (or the window it follows) its own zoom. `None`, or the
/// default, makes it follow the default again.
pub fn set_window_percent(cx: &mut App, id: WindowId, percent: Option<u32>) {
    let default = default_scale(cx).percent;
    let percent = percent
        .map(|percent| sanitize_percent(Some(percent)))
        .filter(|percent| *percent != default);
    let scales = cx.default_global::<WindowUiScales>();
    let id = scales.leader(id);
    match percent {
        Some(percent) => scales.overrides.insert(id, percent),
        None => scales.overrides.remove(&id),
    };
}

/// Makes `follower` render at `leader`'s zoom, as a pop-out does.
pub fn follow_window(cx: &mut App, follower: WindowId, leader: WindowId) {
    let scales = cx.default_global::<WindowUiScales>();
    let leader = scales.leader(leader);
    scales.leaders.insert(follower, leader);
}

/// Drops a closed window's zoom.
pub fn forget_window(cx: &mut App, id: WindowId) {
    if let Some(scales) = cx.try_global::<WindowUiScales>()
        && (scales.overrides.contains_key(&id) || scales.leaders.contains_key(&id))
    {
        let scales = cx.default_global::<WindowUiScales>();
        scales.overrides.remove(&id);
        scales.leaders.remove(&id);
    }
}

pub fn sanitize_percent(percent: Option<u32>) -> u32 {
    percent
        .unwrap_or(DEFAULT_UI_SCALE_PERCENT)
        .clamp(MIN_UI_SCALE_PERCENT, MAX_UI_SCALE_PERCENT)
}

pub fn label(percent: u32) -> String {
    format!("{}%", sanitize_percent(Some(percent)))
}

pub fn step_up(current: u32) -> u32 {
    let current = sanitize_percent(Some(current));
    UI_SCALE_PRESETS
        .iter()
        .copied()
        .find(|percent| *percent > current)
        .unwrap_or(current)
}

pub fn step_down(current: u32) -> u32 {
    let current = sanitize_percent(Some(current));
    UI_SCALE_PRESETS
        .iter()
        .rev()
        .copied()
        .find(|percent| *percent < current)
        .unwrap_or(current)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiScale {
    percent: u32,
    factor: f32,
    pub appearance: crate::appearance::Appearance,
}

impl From<u32> for UiScale {
    fn from(percent: u32) -> Self {
        Self::from_percent(percent)
    }
}

impl UiScale {
    pub fn from_percent(percent: u32) -> Self {
        let percent = sanitize_percent(Some(percent));
        Self {
            percent,
            factor: percent as f32 / DEFAULT_UI_SCALE_PERCENT as f32,
            appearance: crate::appearance::Appearance::default(),
        }
    }

    pub fn from_window(window: &Window) -> Self {
        let factor = design_scale_factor_from_window(window);
        let percent = sanitize_percent(Some(
            (factor * DEFAULT_UI_SCALE_PERCENT as f32).round() as u32
        ));
        Self {
            percent,
            factor,
            appearance: crate::appearance::Appearance::default(),
        }
    }

    pub fn current<C>(cx: &mut C) -> Self
    where
        C: std::borrow::BorrowMut<App>,
    {
        let scale = Self::from_percent(current(cx).percent);
        let appearance = cx
            .update_default_global::<crate::appearance::Appearance, _>(|appearance, _| *appearance);
        scale.with_appearance(appearance)
    }

    pub fn with_appearance(mut self, appearance: crate::appearance::Appearance) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn row_height(self, compact: f32, comfortable: f32) -> Pixels {
        self.px(self.appearance.row_height(compact, comfortable))
    }

    pub fn ui_text(self, design_px: f32) -> Pixels {
        self.px(self.appearance.ui_text(design_px))
    }

    pub fn percent(self) -> u32 {
        self.percent
    }

    pub fn scale_f32(self, value: f32) -> f32 {
        value * self.factor
    }

    pub fn px(self, value: f32) -> Pixels {
        px(self.scale_f32(value))
    }

    pub fn size(self, width: f32, height: f32) -> Size<Pixels> {
        size(self.px(width), self.px(height))
    }

    pub fn design_units_from_pixels(self, value: Pixels) -> f32 {
        let raw: f32 = value.into();
        raw / self.factor.max(f32::EPSILON)
    }

    pub fn design_units_from_optional_pixels(self, value: Option<Pixels>) -> Option<f32> {
        value.map(|value| self.design_units_from_pixels(value))
    }

    pub fn pixels_from_design_units(self, value: Option<f32>) -> Option<Pixels> {
        value.map(|value| self.px(value))
    }
}

pub fn design_units_from_stored(value: Option<u32>) -> Option<f32> {
    value.map(|value| value as f32)
}

pub fn stored_design_units(value: Option<f32>) -> Option<u32> {
    let value = value?.round();
    (value.is_finite() && value >= 1.0).then_some(value as u32)
}

pub fn rem_size_for_percent(percent: u32) -> Pixels {
    px(BASE_REM_PX * design_scale_factor_from_percent(percent))
}

pub fn apply_to_window(window: &mut Window, percent: u32) {
    window.set_rem_size(rem_size_for_percent(percent));
}

pub fn design_scale_factor_from_percent(percent: u32) -> f32 {
    sanitize_percent(Some(percent)) as f32 / DEFAULT_UI_SCALE_PERCENT as f32
}

pub fn design_scale_factor_from_window(window: &Window) -> f32 {
    let rem_size: f32 = window.rem_size().into();
    rem_size / BASE_REM_PX
}

pub fn design_px<C>(value: f32, cx: &mut C) -> Pixels
where
    C: std::borrow::BorrowMut<App>,
{
    UiScale::current(cx).px(value)
}

pub fn design_px_from_percent(value: f32, percent: u32) -> Pixels {
    UiScale::from_percent(percent).px(value)
}

/// A reusable `design px -> Pixels`. Captures the scale by value, so it does
/// not borrow the view for the whole render.
pub fn scaler<S: Into<UiScale>>(scale: S) -> impl Fn(f32) -> Pixels + Copy + use<S> {
    let scale = scale.into();
    move |value: f32| scale.px(value)
}

pub fn design_px_from_window(value: f32, window: &Window) -> Pixels {
    UiScale::from_window(window).px(value)
}

pub fn design_size_from_percent(width: f32, height: f32, percent: u32) -> Size<Pixels> {
    UiScale::from_percent(percent).size(width, height)
}

#[cfg(any(test, feature = "test-support"))]
pub fn rescale_pixels(value: Pixels, from_percent: u32, to_percent: u32) -> Pixels {
    if from_percent == to_percent {
        return value;
    }

    let design_units = UiScale::from_percent(from_percent).design_units_from_pixels(value);
    UiScale::from_percent(to_percent).px(design_units)
}

#[cfg(any(test, feature = "test-support"))]
pub fn rescale_optional_u32(value: Option<u32>, from_percent: u32, to_percent: u32) -> Option<u32> {
    let value = value?;
    let scaled = rescale_pixels(px(value as f32), from_percent, to_percent);
    let scaled: f32 = scaled.round().into();
    (scaled.is_finite() && scaled >= 1.0).then_some(scaled as u32)
}

/// The scale window chrome draws at: the default percent, Compact density.
pub fn chrome_scale() -> UiScale {
    UiScale::from_percent(DEFAULT_UI_SCALE_PERCENT).with_appearance(crate::appearance::Appearance {
        density: crate::appearance::UiDensity::Compact,
        ..crate::appearance::Appearance::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn a_window_zoom_overrides_the_default_until_reset(cx: &mut gpui::TestAppContext) {
        let window = cx
            .add_empty_window()
            .update(|window, _| window.window_handle());
        let id = window.window_id();
        cx.update(|cx| {
            set_default(cx, 110);
            set_window_percent(cx, id, Some(125));
            assert_eq!(percent_for_window(cx, id), 125);
            assert_eq!(current(cx).percent, 110, "outside any window: the default");

            set_window_percent(cx, id, Some(110));
            assert_eq!(window_override(cx, id), None, "the default is no override");
            set_window_percent(cx, id, Some(150));
            set_window_percent(cx, id, None);
            assert_eq!(percent_for_window(cx, id), 110);

            set_window_percent(cx, id, Some(150));
        });
        let in_window = window.update(cx, |_, _, cx| current(cx).percent).unwrap();
        assert_eq!(in_window, 150, "reads inside the window resolve its zoom");
    }

    #[gpui::test]
    fn a_follower_renders_at_its_leaders_zoom(cx: &mut gpui::TestAppContext) {
        let leader = cx
            .add_empty_window()
            .update(|window, _| window.window_handle().window_id());
        let follower = cx
            .add_empty_window()
            .update(|window, _| window.window_handle().window_id());
        cx.update(|cx| {
            follow_window(cx, follower, leader);
            set_window_percent(cx, leader, Some(150));
            assert_eq!(percent_for_window(cx, follower), 150);

            // Zooming the follower zooms its leader.
            set_window_percent(cx, follower, Some(175));
            assert_eq!(percent_for_window(cx, leader), 175);

            // Closing the follower leaves the leader's zoom alone.
            forget_window(cx, follower);
            assert_eq!(percent_for_window(cx, leader), 175);
            assert_eq!(percent_for_window(cx, follower), DEFAULT_UI_SCALE_PERCENT);
        });
    }

    #[test]
    fn ui_scale_steps_follow_presets() {
        assert_eq!(step_down(80), 80);
        assert_eq!(step_down(100), 90);
        assert_eq!(step_up(100), 110);
        assert_eq!(step_up(150), 175);
        assert_eq!(step_up(175), 200);
        assert_eq!(step_up(200), 200);
    }

    /// A `scaler` must agree with the scale it was built from, appearance
    /// included.
    #[test]
    fn scaler_matches_the_scale_it_was_built_from() {
        let comfortable =
            UiScale::from_percent(150).with_appearance(crate::appearance::Appearance {
                density: crate::appearance::UiDensity::Comfortable,
                ..crate::appearance::Appearance::default()
            });

        for scale in [
            UiScale::from_percent(100),
            UiScale::from_percent(150),
            comfortable,
        ] {
            let scaled = scaler(scale);
            for value in [0.0, 1.0, 13.5, 220.0] {
                assert_eq!(scaled(value), scale.px(value));
            }
        }

        assert_eq!(scaler(150u32)(10.0), design_px_from_percent(10.0, 150));
    }

    #[test]
    fn ui_scale_rescaling_uses_percent_ratio() {
        assert_eq!(rescale_optional_u32(Some(200), 100, 125), Some(250));
        assert_eq!(rescale_optional_u32(Some(250), 125, 100), Some(200));
    }

    #[test]
    fn ui_scale_round_trips_design_units_without_drift() {
        let width = 273.63635;
        let scale = UiScale::from_percent(110);
        let px = scale.px(width);
        let round_trip = scale.design_units_from_pixels(px);
        assert!((round_trip - width).abs() < 1e-3);
        assert_eq!(stored_design_units(Some(round_trip)), Some(274));
    }
}
