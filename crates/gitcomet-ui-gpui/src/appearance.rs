//! App-wide density and typography, independent of the window's UI scale.
use gitcomet_state::session::UiSession;
use gpui::{App, Pixels, Window};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum UiDensity {
    #[default]
    Compact,
    Comfortable,
}

impl UiDensity {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FontRole {
    Ui,
    Editor,
    Markdown,
}

impl FontRole {
    pub(crate) const ALL: [Self; 3] = [Self::Ui, Self::Editor, Self::Markdown];
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Ui => 0,
            Self::Editor => 1,
            Self::Markdown => 2,
        }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ui => "UI Font size",
            Self::Editor => "Editor Font size",
            Self::Markdown => "Markdown preview size",
        }
    }
    pub(crate) fn default_size(self) -> u32 {
        match self {
            Self::Ui => 14,
            Self::Editor => 14,
            // Prose, not code: it reads at a size the other two would be too
            // dense at.
            Self::Markdown => 16,
        }
    }
    pub(crate) fn range(self) -> std::ops::RangeInclusive<u32> {
        match self {
            Self::Ui => 10..=24,
            Self::Editor => 8..=32,
            Self::Markdown => 10..=32,
        }
    }
    pub(crate) fn sanitize(self, value: Option<u32>) -> u32 {
        value
            .unwrap_or(self.default_size())
            .clamp(*self.range().start(), *self.range().end())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Appearance {
    pub(crate) density: UiDensity,
    pub(crate) ui_font_size_px: u32,
    pub(crate) editor_font_size_px: u32,
    pub(crate) markdown_preview_font_size_px: u32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            density: UiDensity::Compact,
            ui_font_size_px: FontRole::Ui.default_size(),
            editor_font_size_px: FontRole::Editor.default_size(),
            markdown_preview_font_size_px: FontRole::Markdown.default_size(),
        }
    }
}
impl gpui::Global for Appearance {}

impl Appearance {
    pub(crate) fn from_session(session: &UiSession) -> Self {
        Self {
            density: if session.ui_density.as_deref() == Some("comfortable") {
                UiDensity::Comfortable
            } else {
                UiDensity::Compact
            },
            ui_font_size_px: FontRole::Ui.sanitize(session.ui_font_size_px),
            editor_font_size_px: FontRole::Editor.sanitize(session.editor_font_size_px),
            markdown_preview_font_size_px: FontRole::Markdown
                .sanitize(session.markdown_preview_font_size_px),
        }
    }

    pub(crate) fn size(self, role: FontRole) -> u32 {
        match role {
            FontRole::Ui => self.ui_font_size_px,
            FontRole::Editor => self.editor_font_size_px,
            FontRole::Markdown => self.markdown_preview_font_size_px,
        }
    }

    pub(crate) fn set_size(&mut self, role: FontRole, value: u32) {
        let value = role.sanitize(Some(value));
        match role {
            FontRole::Ui => self.ui_font_size_px = value,
            FontRole::Editor => self.editor_font_size_px = value,
            FontRole::Markdown => self.markdown_preview_font_size_px = value,
        }
    }

    pub(crate) fn ui_text(self, design_px: f32) -> f32 {
        design_px * self.ui_font_size_px as f32 / 14.0
    }

    pub(crate) fn row_height(self, compact: f32, comfortable: f32) -> f32 {
        let baseline = if self.density == UiDensity::Comfortable {
            comfortable
        } else {
            compact
        };
        baseline + (self.ui_text(20.0) - 20.0).max(0.0)
    }

    pub(crate) fn editor_line_height(self) -> f32 {
        (self.editor_font_size_px as f32 * 20.0 / 13.0).ceil()
    }
}

pub(crate) fn current(cx: &App) -> Appearance {
    cx.try_global::<Appearance>().copied().unwrap_or_default()
}

pub(crate) fn initialize(session: &UiSession, cx: &mut App) {
    if cx.try_global::<Appearance>().is_none() {
        cx.set_global(Appearance::from_session(session));
    }
}

pub(crate) fn editor_size(window: &Window, cx: &App) -> Pixels {
    crate::ui_scale::design_px_from_window(current(cx).editor_font_size_px as f32, window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sessions_and_invalid_values_have_bounded_defaults() {
        assert_eq!(
            Appearance::from_session(&UiSession::default()),
            Appearance::default()
        );
        let session = UiSession {
            ui_density: Some("unknown".into()),
            ui_font_size_px: Some(0),
            editor_font_size_px: Some(200),
            markdown_preview_font_size_px: Some(13),
            ..UiSession::default()
        };
        let appearance = Appearance::from_session(&session);
        assert_eq!(appearance.density, UiDensity::Compact);
        assert_eq!(
            (appearance.ui_font_size_px, appearance.editor_font_size_px),
            (10, 32)
        );
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
                Appearance::from_session(&UiSession::default()).size(role),
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
