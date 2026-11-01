//! The theme each window draws with. Code running outside any window, and a
//! window that has none yet, gets the default dark theme in the current
//! appearance.

use super::AppTheme;
use gpui::{App, WindowId};
use rustc_hash::FxHashMap;

#[derive(Default)]
struct WindowThemes {
    themes: FxHashMap<WindowId, AppTheme>,
    /// Windows (pop-outs) that draw with another window's theme.
    leaders: FxHashMap<WindowId, WindowId>,
}

impl WindowThemes {
    fn leader(&self, id: WindowId) -> WindowId {
        self.leaders.get(&id).copied().unwrap_or(id)
    }
}

impl gpui::Global for WindowThemes {}

impl AppTheme {
    /// The theme of the window being drawn or handling an event.
    pub fn current(cx: &App) -> AppTheme {
        cx.current_window_id()
            .and_then(|id| Self::for_window(cx, id))
            .unwrap_or_else(|| {
                AppTheme::gitcomet_dark().with_appearance(crate::appearance::current(cx))
            })
    }

    /// The theme window `id` (or the window it follows) draws with, if set.
    pub fn for_window(cx: &App, id: WindowId) -> Option<AppTheme> {
        let themes = cx.try_global::<WindowThemes>()?;
        themes.themes.get(&themes.leader(id)).copied()
    }
}

/// Sets the theme window `id` (or the window it follows) draws with. Views
/// that cache theme colours still need their owner to invalidate them.
pub fn set_window_theme(cx: &mut App, id: WindowId, theme: AppTheme) {
    let themes = cx.default_global::<WindowThemes>();
    let id = themes.leader(id);
    themes.themes.insert(id, theme);
}

/// Makes `follower` draw with `leader`'s theme, as a pop-out does.
pub fn follow_window_theme(cx: &mut App, follower: WindowId, leader: WindowId) {
    let themes = cx.default_global::<WindowThemes>();
    let leader = themes.leader(leader);
    themes.leaders.insert(follower, leader);
}

/// Drops a closed window's theme.
pub fn forget_window_theme(cx: &mut App, id: WindowId) {
    if let Some(themes) = cx.try_global::<WindowThemes>()
        && (themes.themes.contains_key(&id) || themes.leaders.contains_key(&id))
    {
        let themes = cx.default_global::<WindowThemes>();
        themes.themes.remove(&id);
        themes.leaders.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn windows_draw_with_their_own_or_their_leaders_theme(cx: &mut gpui::TestAppContext) {
        let light = AppTheme::gitcomet_light();
        let (first, second, follower) = (WindowId::from(1), WindowId::from(2), WindowId::from(3));
        cx.update(|cx| {
            assert_eq!(AppTheme::for_window(cx, first), None);
            set_window_theme(cx, first, light);
            follow_window_theme(cx, follower, first);
            assert_eq!(AppTheme::for_window(cx, first), Some(light));
            assert_eq!(AppTheme::for_window(cx, follower), Some(light));
            assert_eq!(AppTheme::for_window(cx, second), None);
            // Outside a window update there is no current window.
            assert_eq!(
                AppTheme::current(cx),
                AppTheme::gitcomet_dark().with_appearance(crate::appearance::current(cx))
            );
            forget_window_theme(cx, first);
            assert_eq!(AppTheme::for_window(cx, first), None);
        });
    }
}
