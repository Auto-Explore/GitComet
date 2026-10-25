//! App-wide density and typography, independent of the window's UI scale.
use gpui::{App, Pixels, Window};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UiDensity {
    /// The neutral baseline (`Default`): chrome and scale helpers that must
    /// not take density measure against it.
    #[default]
    Compact,
    Comfortable,
    Spacious,
}

impl UiDensity {
    /// What a user who never chose a density gets.
    pub const PREFERENCE_DEFAULT: Self = Self::Comfortable;

    pub const ALL: [Self; 3] = [Self::Compact, Self::Comfortable, Self::Spacious];

    /// Position on the compact-to-comfortable ramp. Spacious continues the same
    /// step rather than carrying a third hand-tuned number per element.
    fn step(self) -> f32 {
        match self {
            Self::Compact => 0.0,
            Self::Comfortable => 1.0,
            Self::Spacious => 1.6,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
            Self::Spacious => "spacious",
        }
    }

    /// Anything unrecognised is the caller's cue to fall back to the default, so
    /// a session written by a newer build still loads.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
            Self::Spacious => "Spacious",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontRole {
    Ui,
    Editor,
    Markdown,
}

impl FontRole {
    pub const ALL: [Self; 3] = [Self::Ui, Self::Editor, Self::Markdown];
    pub fn index(self) -> usize {
        match self {
            Self::Ui => 0,
            Self::Editor => 1,
            Self::Markdown => 2,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Ui => "UI Font size",
            Self::Editor => "Editor Font size",
            Self::Markdown => "Markdown preview size",
        }
    }
    pub fn default_size(self) -> u32 {
        match self {
            Self::Ui => 14,
            Self::Editor => 14,
            // Prose, not code: it reads at a size the other two would be too
            // dense at.
            Self::Markdown => 16,
        }
    }
    pub fn range(self) -> std::ops::RangeInclusive<u32> {
        match self {
            Self::Ui => 10..=24,
            Self::Editor => 8..=32,
            Self::Markdown => 10..=32,
        }
    }
    pub fn sanitize(self, value: Option<u32>) -> u32 {
        value
            .unwrap_or(self.default_size())
            .clamp(*self.range().start(), *self.range().end())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Appearance {
    pub density: UiDensity,
    pub ui_font_size_px: u32,
    pub editor_font_size_px: u32,
    pub markdown_preview_font_size_px: u32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            density: UiDensity::default(),
            ui_font_size_px: FontRole::Ui.default_size(),
            editor_font_size_px: FontRole::Editor.default_size(),
            markdown_preview_font_size_px: FontRole::Markdown.default_size(),
        }
    }
}
impl gpui::Global for Appearance {}

/// Stored appearance choices, as a settings store keeps them; `None` means
/// the user never chose.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AppearancePreferences<'a> {
    /// A [`UiDensity::key`]; unknown keys fall back to the default.
    pub density: Option<&'a str>,
    pub ui_font_size_px: Option<u32>,
    pub editor_font_size_px: Option<u32>,
    pub markdown_preview_font_size_px: Option<u32>,
}

impl Appearance {
    pub fn from_preferences(preferences: AppearancePreferences<'_>) -> Self {
        Self {
            density: preferences
                .density
                .and_then(UiDensity::from_key)
                .unwrap_or(UiDensity::PREFERENCE_DEFAULT),
            ui_font_size_px: FontRole::Ui.sanitize(preferences.ui_font_size_px),
            editor_font_size_px: FontRole::Editor.sanitize(preferences.editor_font_size_px),
            markdown_preview_font_size_px: FontRole::Markdown
                .sanitize(preferences.markdown_preview_font_size_px),
        }
    }

    pub fn size(self, role: FontRole) -> u32 {
        match role {
            FontRole::Ui => self.ui_font_size_px,
            FontRole::Editor => self.editor_font_size_px,
            FontRole::Markdown => self.markdown_preview_font_size_px,
        }
    }

    pub fn set_size(&mut self, role: FontRole, value: u32) {
        let value = role.sanitize(Some(value));
        match role {
            FontRole::Ui => self.ui_font_size_px = value,
            FontRole::Editor => self.editor_font_size_px = value,
            FontRole::Markdown => self.markdown_preview_font_size_px = value,
        }
    }

    pub fn ui_text(self, design_px: f32) -> f32 {
        design_px * self.ui_font_size_px as f32 / 14.0
    }

    /// Places a per-element (compact, comfortable) pair on the density ramp.
    /// Each element's own delta was measured, so scaling it keeps that intent.
    /// Rounded to whole design pixels: Spacious's 1.6 step would otherwise
    /// put rows and gaps on sub-pixel sizes (a 36.8 px history row).
    pub fn ramp(self, compact: f32, comfortable: f32) -> f32 {
        (compact + (comfortable - compact) * self.density.step()).round()
    }

    /// Position on the density ramp (Compact 0, Comfortable 1, Spacious 1.6),
    /// for callers that scale by a factor rather than size in pixels.
    pub fn density_step(self) -> f32 {
        self.density.step()
    }

    pub fn row_height(self, compact: f32, comfortable: f32) -> f32 {
        self.ramp(compact, comfortable) + (self.ui_text(20.0) - 20.0).max(0.0)
    }

    pub fn editor_line_height(self) -> f32 {
        (self.editor_font_size_px as f32 * 20.0 / 13.0).ceil()
    }
}

pub fn current(cx: &App) -> Appearance {
    cx.try_global::<Appearance>().copied().unwrap_or_default()
}

/// Tracks whether the preferences have been applied. The presence of the global
/// cannot answer that: `UiScale::current` reads the appearance through
/// `update_default_global`, which installs the Default when none is set.
#[derive(Default)]
struct AppearanceInitialized(bool);
impl gpui::Global for AppearanceInitialized {}

/// Applies the stored preferences once per app; later calls keep what is live.
pub fn initialize(preferences: AppearancePreferences<'_>, cx: &mut App) {
    if cx
        .try_global::<AppearanceInitialized>()
        .is_some_and(|initialized| initialized.0)
    {
        return;
    }
    cx.set_global(Appearance::from_preferences(preferences));
    cx.set_global(AppearanceInitialized(true));
}

/// Tests that measure Compact layouts install it before building a view, so
/// the view's own initialization (Comfortable for a fresh session) is skipped.
#[cfg(any(test, feature = "test-support"))]
pub fn pin_compact_for_test(cx: &mut App) {
    pin_density_for_test(cx, UiDensity::Compact);
}

#[cfg(any(test, feature = "test-support"))]
pub fn pin_density_for_test(cx: &mut App, density: UiDensity) {
    cx.set_global(Appearance {
        density,
        ..Appearance::default()
    });
    cx.set_global(AppearanceInitialized(true));
}

pub fn editor_size(window: &Window, cx: &App) -> Pixels {
    crate::ui_scale::design_px_from_window(current(cx).editor_font_size_px as f32, window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sessions_and_invalid_values_have_bounded_defaults() {
        assert_eq!(
            Appearance::from_preferences(AppearancePreferences::default()),
            Appearance {
                density: UiDensity::PREFERENCE_DEFAULT,
                ..Appearance::default()
            },
            "a fresh session gets the preferred density over the baseline"
        );
        let appearance = Appearance::from_preferences(AppearancePreferences {
            density: Some("unknown"),
            ui_font_size_px: Some(0),
            editor_font_size_px: Some(200),
            markdown_preview_font_size_px: Some(13),
        });
        assert_eq!(appearance.density, UiDensity::Comfortable);
        assert_eq!(
            (appearance.ui_font_size_px, appearance.editor_font_size_px),
            (10, 32)
        );
    }

    /// Existing users must not shift: the two measured anchors have to come back
    /// out of the ramp exactly, and each step has to be strictly roomier.
    #[test]
    fn the_density_ramp_keeps_its_anchors_and_only_grows() {
        let at = |density| Appearance {
            density,
            ..Appearance::default()
        };

        assert_eq!(at(UiDensity::Compact).ramp(24.0, 32.0), 24.0);
        assert_eq!(at(UiDensity::Comfortable).ramp(24.0, 32.0), 32.0);

        for pair in [(22.0, 32.0), (24.0, 32.0), (34.0, 40.0), (18.0, 24.0)] {
            let ramped: Vec<f32> = UiDensity::ALL
                .into_iter()
                .map(|density| at(density).ramp(pair.0, pair.1))
                .collect();
            assert!(
                ramped.windows(2).all(|w| w[1] > w[0]),
                "{pair:?} must grow at every step, got {ramped:?}"
            );
        }

        assert_eq!(at(UiDensity::Spacious).ramp(20.0, 20.0), 20.0);
    }

    /// Treating an installed Default as "already configured" would silently
    /// drop the saved session.
    #[gpui::test]
    fn initialize_still_applies_the_session_after_something_installed_a_default(
        cx: &mut gpui::TestAppContext,
    ) {
        let preferences = AppearancePreferences {
            density: Some(UiDensity::Spacious.key()),
            ui_font_size_px: Some(20),
            ..AppearancePreferences::default()
        };

        cx.update(|cx| {
            // Anything that asks for the scale before the first view is built.
            let _ = crate::ui_scale::UiScale::current(cx);

            initialize(preferences, cx);

            assert_eq!(current(cx).density, UiDensity::Spacious);
            assert_eq!(current(cx).ui_font_size_px, 20);
        });
    }

    #[test]
    fn density_keys_round_trip_and_unknown_falls_back() {
        for density in UiDensity::ALL {
            assert_eq!(UiDensity::from_key(density.key()), Some(density));
            assert_eq!(
                Appearance::from_preferences(AppearancePreferences {
                    density: Some(density.key()),
                    ..AppearancePreferences::default()
                })
                .density,
                density
            );
        }

        assert_eq!(UiDensity::from_key("nonsense"), None);
        assert_eq!(
            UiDensity::default(),
            UiDensity::Compact,
            "the neutral baseline"
        );
        assert_eq!(UiDensity::PREFERENCE_DEFAULT, UiDensity::Comfortable);
    }

    #[test]
    fn every_density_sizes_controls_in_whole_design_pixels() {
        // Spacious's 1.6 step put the history row at 36.8 px, which left
        // scroll anchors a fraction of a pixel off and rows on sub-pixel edges.
        for density in UiDensity::ALL {
            let metrics = Appearance {
                density,
                ..Appearance::default()
            };
            for (compact, comfortable) in [(24.0, 32.0), (2.0, 6.0), (4.0, 6.0), (10.0, 12.0)] {
                let size = metrics.ramp(compact, comfortable);
                assert_eq!(
                    size.fract(),
                    0.0,
                    "{density:?} ramp({compact}, {comfortable}) = {size}"
                );
                assert!(size >= compact, "{density:?} never shrinks below Compact");
            }
        }
    }

    #[test]
    fn font_roles_and_density_are_independent() {
        let mut appearance = Appearance {
            density: UiDensity::Comfortable,
            ..Appearance::default()
        };
        appearance.set_size(FontRole::Editor, 26);
        assert_eq!(appearance.row_height(24.0, 32.0), 32.0);
        assert_eq!(appearance.editor_line_height(), 40.0);
        assert_eq!(appearance.ui_text(14.0), 14.0);
        assert_eq!(
            appearance.markdown_preview_font_size_px,
            FontRole::Markdown.default_size()
        );
    }

    /// One source of truth: a role's reset value and the default appearance a
    /// session without a stored size falls back to must be the same number.
    #[test]
    fn the_default_appearance_is_the_roles_own_defaults() {
        let default = Appearance::default();

        for role in FontRole::ALL {
            assert_eq!(default.size(role), role.default_size());
            assert!(
                role.range().contains(&role.default_size()),
                "{role:?} default must sit in its own range"
            );
            assert_eq!(
                Appearance::from_preferences(AppearancePreferences::default()).size(role),
                role.default_size(),
                "a session with no stored size must land on the default"
            );
        }

        assert_eq!(
            (
                default.ui_font_size_px,
                default.editor_font_size_px,
                default.markdown_preview_font_size_px
            ),
            (14, 14, 16)
        );
    }
}
