use gpui::{App, Global, Result};
use std::borrow::Cow;

pub use gitcomet_fonts::{
    BUNDLED_FONT_BYTES, FIRA_CODE_FONT_FAMILY, IBM_PLEX_SANS_FONT_FAMILY, LILEX_FONT_FAMILY,
};

const FILTERED_FONT_ALIASES: &[&str] = &[
    "Fira Code SemiBold",
    "IBM Plex Sans SmBld",
    "IBM Plex Sans SemiBold",
    "Lilex Italic",
    "Lilex SemiBold",
    "Lilex SemiBold Italic",
];

pub fn register(cx: &mut App) -> Result<()> {
    let result = cx.text_system().add_fonts(
        BUNDLED_FONT_BYTES
            .iter()
            .map(|bytes| Cow::Borrowed(*bytes))
            .collect(),
    );
    if result.is_ok() {
        cx.set_global(DeferredFonts(Vec::new()));
    }
    result
}

struct DeferredFonts(Vec<Cow<'static, [u8]>>);

impl Global for DeferredFonts {}

fn first_face(ui_family: Option<&str>) -> &'static [u8] {
    use gitcomet_fonts::{
        FIRA_CODE_REGULAR_BYTES, IBM_PLEX_SANS_REGULAR_BYTES, LILEX_REGULAR_BYTES,
        NOTO_EMOJI_BYTES, NOTO_EMOJI_FONT_FAMILY,
    };
    match ui_family {
        Some(FIRA_CODE_FONT_FAMILY) => FIRA_CODE_REGULAR_BYTES,
        Some(LILEX_FONT_FAMILY) => LILEX_REGULAR_BYTES,
        Some(NOTO_EMOJI_FONT_FAMILY) => NOTO_EMOJI_BYTES,
        _ => IBM_PLEX_SANS_REGULAR_BYTES,
    }
}

pub fn register_ui_face(ui_family: Option<&str>, cx: &mut App) -> Result<()> {
    if cx.has_global::<DeferredFonts>() {
        return Ok(());
    }
    let first = first_face(ui_family);
    let result = cx.text_system().add_fonts(vec![Cow::Borrowed(first)]);
    let remaining = BUNDLED_FONT_BYTES
        .iter()
        .copied()
        .filter(|face| result.is_err() || *face != first)
        .map(Cow::Borrowed)
        .collect();
    cx.set_global(DeferredFonts(remaining));
    result
}

pub fn has_deferred_fonts(cx: &App) -> bool {
    cx.try_global::<DeferredFonts>()
        .is_some_and(|pending| !pending.0.is_empty())
}

pub fn register_after_first_frame(cx: &mut App) -> Result<()> {
    if !cx.has_global::<DeferredFonts>() {
        return Ok(());
    }
    let pending = std::mem::take(&mut cx.global_mut::<DeferredFonts>().0);
    if pending.is_empty() {
        return Ok(());
    }
    let _stage = gitcomet_base::startup_trace::span("register_deferred_fonts");
    cx.text_system().add_fonts(pending)?;
    cx.refresh_windows();
    Ok(())
}

pub fn load_into_fontdb(db: &mut fontdb::Database) {
    for bytes in BUNDLED_FONT_BYTES {
        db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(*bytes)));
    }
}

pub fn should_skip_font_option_alias(font_family: &str) -> bool {
    FILTERED_FONT_ALIASES.contains(&font_family)
}

/// The monospace family UI text uses (paths, hashes, shortcuts).
pub const UI_MONOSPACE_FONT_FAMILY: &str = LILEX_FONT_FAMILY;

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_first_window_defers_other_faces_and_later_windows_use_the_registered_fonts(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            register_ui_face(Some("Fira Code"), cx).unwrap();
            let pending = &cx.global::<DeferredFonts>().0;
            assert_eq!(pending.len(), BUNDLED_FONT_BYTES.len() - 1);
            assert!(
                pending
                    .iter()
                    .all(|face| face.as_ref() != first_face(Some("Fira Code")))
            );
            register_ui_face(Some("Lilex"), cx).unwrap();
            assert_eq!(
                cx.global::<DeferredFonts>().0.len(),
                BUNDLED_FONT_BYTES.len() - 1
            );
            register_after_first_frame(cx).unwrap();
            assert!(!has_deferred_fonts(cx));
            register_ui_face(None, cx).unwrap();
            assert!(!has_deferred_fonts(cx));
        });
    }

    #[gpui::test]
    fn a_startup_workflow_registers_its_fonts_before_main_window_creation(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            register(cx).unwrap();
            register_ui_face(None, cx).unwrap();
            assert!(!has_deferred_fonts(cx));
        });
    }

    #[test]
    fn the_selected_regular_face_is_available_before_the_remaining_fonts() {
        use gitcomet_fonts::{
            FIRA_CODE_REGULAR_BYTES, IBM_PLEX_SANS_REGULAR_BYTES, LILEX_REGULAR_BYTES,
            NOTO_EMOJI_BYTES,
        };
        for (selection, expected) in [
            (None, IBM_PLEX_SANS_REGULAR_BYTES),
            (Some("IBM Plex Sans"), IBM_PLEX_SANS_REGULAR_BYTES),
            (Some("Fira Code"), FIRA_CODE_REGULAR_BYTES),
            (Some("Lilex"), LILEX_REGULAR_BYTES),
            (Some("Noto Emoji"), NOTO_EMOJI_BYTES),
            (Some("system font"), IBM_PLEX_SANS_REGULAR_BYTES),
        ] {
            let face = first_face(selection);
            assert!(face == expected);
            assert_eq!(
                BUNDLED_FONT_BYTES
                    .iter()
                    .filter(|bytes| **bytes == face)
                    .count(),
                1
            );
        }
    }
}
