//! The theme each window draws with. Code running outside any window, and a
//! window that has none yet, gets the default dark theme in the current
//! appearance.

use super::AppTheme;
use gpui::{App, WindowId};
use rustc_hash::FxHashMap;

/// Which setting of its theme a window draws with. A view remembers the one
/// its theme-derived caches were built under and drops them when it changes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ThemeGeneration(u64);

impl ThemeGeneration {
    /// Takes the generation of the window being drawn; true when it changed
    /// since the last call. Only meaningful while a window draws or handles
    /// an event.
    pub fn changed(&mut self, cx: &App) -> bool {
        let current = AppTheme::current_generation(cx);
        std::mem::replace(self, current) != current
    }
}

#[derive(Default)]
struct WindowThemes {
    themes: FxHashMap<WindowId, (AppTheme, ThemeGeneration)>,
    /// Windows (pop-outs) that draw with another window's theme.
    leaders: FxHashMap<WindowId, WindowId>,
    /// Never reused, so a theme set again after a forget still reads as a change.
    last_generation: u64,
}

impl WindowThemes {
    fn leader(&self, id: WindowId) -> WindowId {
        self.leaders.get(&id).copied().unwrap_or(id)
    }
}

impl gpui::Global for WindowThemes {}

/// The theme window `id` (or the window it follows) draws with, if set.
fn drawn_with(cx: &App, id: WindowId) -> Option<(AppTheme, ThemeGeneration)> {
    let themes = cx.try_global::<WindowThemes>()?;
    themes.themes.get(&themes.leader(id)).copied()
}

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
        drawn_with(cx, id).map(|(theme, _)| theme)
    }

    /// Changes whenever the theme [`AppTheme::current`] returns for the window
    /// being drawn changes; the default until that window has a theme.
    pub fn current_generation(cx: &App) -> ThemeGeneration {
        cx.current_window_id()
            .and_then(|id| drawn_with(cx, id))
            .map(|(_, generation)| generation)
            .unwrap_or_default()
    }
}

/// Sets the theme window `id` (or the window it follows) draws with. A real
/// change starts a new [`ThemeGeneration`] and repaints the windows drawing
/// with it, so views read the theme when they render instead of being told.
pub fn set_window_theme(cx: &mut App, id: WindowId, theme: AppTheme) {
    if drawn_with(cx, id).is_some_and(|(current, _)| current == theme) {
        return;
    }
    let themes = cx.default_global::<WindowThemes>();
    let id = themes.leader(id);
    themes.last_generation += 1;
    let generation = ThemeGeneration(themes.last_generation);
    themes.themes.insert(id, (theme, generation));
    // Deferred: a window ignores a refresh while it is drawing, and cannot be
    // reached while it is the one being updated.
    cx.defer(move |cx| {
        for window in cx.windows() {
            let follows = cx
                .try_global::<WindowThemes>()
                .is_some_and(|themes| themes.leader(window.window_id()) == id);
            if follows {
                let _ = window.update(cx, |_, window, _| window.refresh());
            }
        }
    });
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

    /// Records the theme and generation each render saw.
    struct Leaf {
        renders: Vec<(AppTheme, ThemeGeneration)>,
        cached: ThemeGeneration,
        cache_drops: usize,
    }

    impl gpui::Render for Leaf {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            if self.cached.changed(cx) {
                self.cache_drops += 1;
            }
            self.renders
                .push((AppTheme::current(cx), AppTheme::current_generation(cx)));
            gpui::div()
        }
    }

    /// Mounts the leaf cached, as the main window mounts its panes.
    struct Root(gpui::Entity<Leaf>);

    impl gpui::Render for Root {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{ParentElement as _, Styled as _};
            gpui::div().size_full().child(
                gpui::AnyView::from(self.0.clone())
                    .cached(gpui::StyleRefinement::default().size_full()),
            )
        }
    }

    fn leaf_window(
        cx: &mut gpui::TestAppContext,
    ) -> (gpui::Entity<Leaf>, WindowId, &mut gpui::VisualTestContext) {
        use gpui::AppContext as _;
        let (root, cx) = cx.add_window_view(|_, cx| {
            Root(cx.new(|_| Leaf {
                renders: Vec::new(),
                cached: ThemeGeneration::default(),
                cache_drops: 0,
            }))
        });
        let leaf = cx.update(|_, app| root.read(app).0.clone());
        let id = cx.update(|window, _| window.window_handle().window_id());
        (leaf, id, cx)
    }

    #[gpui::test]
    fn a_theme_change_repaints_cached_views_that_nothing_notified(cx: &mut gpui::TestAppContext) {
        let (dark, light) = (AppTheme::gitcomet_dark(), AppTheme::gitcomet_light());
        let (leaf, id, cx) = leaf_window(cx);
        cx.update(|_, app| set_window_theme(app, id, dark));
        cx.run_until_parked();
        let before = cx.update(|_, app| leaf.read(app).renders.len());

        cx.update(|_, app| set_window_theme(app, id, light));
        cx.run_until_parked();
        cx.update(|_, app| {
            let leaf = leaf.read(app);
            assert!(
                leaf.renders.len() > before,
                "the cached leaf was not redrawn"
            );
            assert_eq!(leaf.renders.last().map(|(theme, _)| *theme), Some(light));
        });

        // Setting the same theme again is not a change: no repaint.
        let before = cx.update(|_, app| leaf.read(app).renders.len());
        cx.update(|_, app| set_window_theme(app, id, light));
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(leaf.read(app).renders.len(), before));
    }

    #[gpui::test]
    fn the_generation_changes_only_when_the_windows_theme_does(cx: &mut gpui::TestAppContext) {
        let (dark, light) = (AppTheme::gitcomet_dark(), AppTheme::gitcomet_light());
        let (leaf, id, cx) = leaf_window(cx);
        let generation = |cx: &mut gpui::VisualTestContext| {
            cx.run_until_parked();
            cx.update(|_, app| {
                (
                    AppTheme::current_generation(app),
                    leaf.read(app).cache_drops,
                )
            })
        };
        assert_eq!(generation(cx), (ThemeGeneration::default(), 0));

        cx.update(|_, app| set_window_theme(app, id, dark));
        let (first, drops) = generation(cx);
        assert_ne!(first, ThemeGeneration::default());
        assert_eq!(drops, 1);

        cx.update(|_, app| set_window_theme(app, id, dark));
        assert_eq!(generation(cx), (first, 1), "the same theme is no change");

        // Another window's theme leaves this window's generation alone.
        cx.update(|_, app| set_window_theme(app, WindowId::from(u64::MAX), light));
        assert_eq!(generation(cx), (first, 1));

        cx.update(|_, app| set_window_theme(app, id, light));
        let (second, drops) = generation(cx);
        assert_ne!(second, first);
        assert_eq!(drops, 2);

        // Back to an earlier theme is a change too: caches hold the last one.
        cx.update(|_, app| set_window_theme(app, id, dark));
        let (third, drops) = generation(cx);
        assert!(third != first && third != second);
        assert_eq!(drops, 3);
    }

    #[gpui::test]
    fn a_follower_window_repaints_with_its_leader(cx: &mut gpui::TestAppContext) {
        let light = AppTheme::gitcomet_light();
        let (leaf, follower, cx) = leaf_window(cx);
        let leader = WindowId::from(u64::MAX);
        cx.update(|_, app| follow_window_theme(app, follower, leader));
        cx.run_until_parked();
        let before = cx.update(|_, app| leaf.read(app).renders.len());

        cx.update(|_, app| set_window_theme(app, leader, light));
        cx.run_until_parked();
        cx.update(|_, app| {
            let leaf = leaf.read(app);
            assert!(leaf.renders.len() > before, "the follower was not redrawn");
            assert_eq!(leaf.renders.last().map(|(theme, _)| *theme), Some(light));
        });
    }
}
