//! Adapters from the persisted `UiSession` to the kit's preference inputs.
//!
//! Appearance, UI scale, and fonts know nothing about how the host stores
//! them; this module is the one place that reads the session for them.

use crate::appearance::{self, Appearance, AppearancePreferences};
use crate::font_preferences::{self, AppFontPreferences, StoredFontPreferences};
use crate::ui_scale::{self, AppUiScale};
use gitcomet_state::session::UiSession;
use gpui::{App, BorrowAppContext, Window};

pub(crate) fn appearance_preferences(session: &UiSession) -> AppearancePreferences<'_> {
    AppearancePreferences {
        density: session.ui_density.as_deref(),
        ui_font_size_px: session.ui_font_size_px,
        editor_font_size_px: session.editor_font_size_px,
        markdown_preview_font_size_px: session.markdown_preview_font_size_px,
    }
}

pub(crate) fn appearance(session: &UiSession) -> Appearance {
    Appearance::from_preferences(appearance_preferences(session))
}

pub(crate) fn initialize_appearance(session: &UiSession, cx: &mut App) {
    appearance::initialize(appearance_preferences(session), cx);
}

/// The default UI scale new windows open at, seeded from the session once.
pub(crate) fn ui_scale<C: BorrowAppContext>(session: &UiSession, cx: &mut C) -> AppUiScale {
    ui_scale::default_or_initialize(session.ui_scale_percent, cx)
}

pub(crate) fn font_preferences<C: BorrowAppContext>(
    window: &Window,
    session: &UiSession,
    cx: &mut C,
) -> AppFontPreferences {
    font_preferences::current_or_initialize(
        window,
        StoredFontPreferences {
            ui_font_family: session.ui_font_family.as_deref(),
            editor_font_family: session.editor_font_family.as_deref(),
            use_font_ligatures: session.use_font_ligatures,
        },
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::UiDensity;

    #[test]
    fn a_session_maps_every_stored_appearance_field() {
        let session = UiSession {
            ui_density: Some(UiDensity::Spacious.key().to_string()),
            ui_font_size_px: Some(20),
            editor_font_size_px: Some(200),
            markdown_preview_font_size_px: Some(13),
            ..UiSession::default()
        };
        let appearance = appearance(&session);
        assert_eq!(appearance.density, UiDensity::Spacious);
        assert_eq!(
            (
                appearance.ui_font_size_px,
                appearance.editor_font_size_px,
                appearance.markdown_preview_font_size_px
            ),
            (20, 32, 13)
        );
        assert_eq!(
            super::appearance(&UiSession::default()),
            Appearance::from_preferences(AppearancePreferences::default())
        );
    }
}
