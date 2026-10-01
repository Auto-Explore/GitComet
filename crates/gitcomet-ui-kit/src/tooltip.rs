use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{AnyView, App, SharedString, Window, div, px};

const TOOLTIP_CURSOR_OFFSET_X_PX: f32 = 11.0;
const TOOLTIP_CURSOR_OFFSET_Y_PX: f32 = 17.0;
#[cfg(any(test, feature = "test-support"))]
use std::cell::RefCell;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static VISIBLE_TOOLTIP_TEXT_FOR_TEST: RefCell<Option<SharedString>> = const { RefCell::new(None) };
}

/// Bumped on every mouse-down (see [`dismiss_tooltips_on_mouse_down`]); a
/// visible tooltip bubble compares against the epoch it was built at and
/// renders empty once the epoch moves on, so clicks always hide tooltips.
#[derive(Default)]
pub struct TooltipDismissEpoch(u64);

impl gpui::Global for TooltipDismissEpoch {}

/// True while a popover / context menu is open. The overlay's anchor stays
/// hovered after the opening click, so its tooltip would re-show painted on
/// top of the open surface; bubbles render empty while this is set.
#[derive(Default)]
pub struct TooltipOverlaySuppression(bool);

impl gpui::Global for TooltipOverlaySuppression {}

pub fn set_tooltips_suppressed_by_overlay(open: bool, cx: &mut App) {
    if tooltips_suppressed_by_overlay(cx) != open {
        cx.set_global(TooltipOverlaySuppression(open));
    }
}

pub fn tooltips_suppressed_by_overlay(cx: &App) -> bool {
    cx.try_global::<TooltipOverlaySuppression>()
        .is_some_and(|state| state.0)
}

pub fn current_tooltip_dismiss_epoch(cx: &App) -> u64 {
    cx.try_global::<TooltipDismissEpoch>()
        .map(|epoch| epoch.0)
        .unwrap_or(0)
}

/// Hides every visible gpui-managed tooltip bubble. Registered on window
/// roots via `capture_any_mouse_down` so it runs for clicks anywhere.
pub fn dismiss_tooltips_on_mouse_down(cx: &mut App) {
    let next = current_tooltip_dismiss_epoch(cx).wrapping_add(1);
    cx.set_global(TooltipDismissEpoch(next));
}

pub trait GitCometTooltipExt: gpui::StatefulInteractiveElement + Sized {
    fn gitcomet_tooltip(self, theme: AppTheme, text: SharedString) -> Self {
        self.tooltip(move |_window, cx| {
            let epoch = current_tooltip_dismiss_epoch(cx);
            AnyView::from(cx.new(|cx| {
                let epoch_observer = cx.observe_global::<TooltipDismissEpoch>(|_, cx| cx.notify());
                let overlay_observer =
                    cx.observe_global::<TooltipOverlaySuppression>(|_, cx| cx.notify());
                TooltipBubbleView {
                    theme,
                    text: text.clone(),
                    epoch,
                    _epoch_observer: epoch_observer,
                    _overlay_observer: overlay_observer,
                }
            }))
        })
    }
}

impl<T: gpui::StatefulInteractiveElement> GitCometTooltipExt for T {}

struct TooltipBubbleView {
    theme: AppTheme,
    text: SharedString,
    /// Dismiss epoch at build time; a later epoch means a click happened
    /// while this bubble was up, so it must disappear.
    epoch: u64,
    _epoch_observer: gpui::Subscription,
    _overlay_observer: gpui::Subscription,
}

impl Render for TooltipBubbleView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if current_tooltip_dismiss_epoch(cx) != self.epoch {
            return div();
        }
        if tooltips_suppressed_by_overlay(cx) {
            return div();
        }

        #[cfg(any(test, feature = "test-support"))]
        VISIBLE_TOOLTIP_TEXT_FOR_TEST.with(|value| {
            value.replace(Some(self.text.clone()));
        });

        // Offset from the cursor hotspot, scaled so the bubble clears a
        // larger pointer.
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        div()
            .pl(scaled_px(TOOLTIP_CURSOR_OFFSET_X_PX))
            .pt(scaled_px(TOOLTIP_CURSOR_OFFSET_Y_PX))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .bg(self.theme.colors.tooltip.background)
                    .rounded(px(self.theme.radii.row))
                    .shadow(crate::theme::shadow_popover(self.theme))
                    .text_size(self.theme.ui_text(12.0))
                    .text_color(self.theme.colors.tooltip.foreground)
                    .child(self.text.clone()),
            )
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn clear_visible_tooltip_text_for_test() {
    VISIBLE_TOOLTIP_TEXT_FOR_TEST.with(|value| {
        value.replace(None);
    });
}

#[cfg(any(test, feature = "test-support"))]
pub fn tooltip_text_for_test() -> Option<SharedString> {
    VISIBLE_TOOLTIP_TEXT_FOR_TEST.with(|value| value.borrow().clone())
}
