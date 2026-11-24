use gpui::{App, Global, Result};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};

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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct FontGeneration(u64);

impl Global for FontGeneration {}

thread_local! {
    static GENERATION: Cell<FontGeneration> = const { Cell::new(FontGeneration(0)) };
    static LAYOUT_CACHES: RefCell<Vec<fn()>> = const { RefCell::new(Vec::new()) };
}

impl FontGeneration {
    pub fn current() -> Self {
        GENERATION.get()
    }
}

/// Initialize a UI-thread cache and clear it when registration changes glyphs.
/// Registering the callback here leaves unused caches unallocated.
pub fn layout_cache<T>(initial: T, clear: fn()) -> RefCell<T> {
    LAYOUT_CACHES.with_borrow_mut(|caches| caches.push(clear));
    RefCell::new(initial)
}

fn add_fonts(faces: &[&'static [u8]], cx: &mut App) -> Result<()> {
    cx.text_system()
        .add_fonts(faces.iter().map(|face| Cow::Borrowed(*face)).collect())?;
    let next = FontGeneration(FontGeneration::current().0.wrapping_add(1));
    GENERATION.set(next);
    LAYOUT_CACHES.with_borrow(|caches| caches.iter().for_each(|clear| clear()));
    cx.set_global(next);
    Ok(())
}

pub fn register(cx: &mut App) -> Result<()> {
    add_fonts(BUNDLED_FONT_BYTES, cx)?;
    cx.set_global(DeferredFonts(Vec::new()));
    Ok(())
}

struct DeferredFonts(Vec<&'static [u8]>);

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
    let result = add_fonts(&[first], cx);
    let remaining = BUNDLED_FONT_BYTES
        .iter()
        .copied()
        .filter(|face| result.is_err() || *face != first)
        .collect();
    cx.set_global(DeferredFonts(remaining));
    result
}

pub fn has_deferred_fonts(cx: &App) -> bool {
    cx.try_global::<DeferredFonts>()
        .is_some_and(|pending| !pending.0.is_empty())
}

pub fn register_after_first_frame(cx: &mut App) -> Result<()> {
    let Some(pending) = cx.try_global::<DeferredFonts>() else {
        return Ok(());
    };
    if pending.0.is_empty() {
        return Ok(());
    }
    let _stage = gitcomet_base::startup_trace::span("register_deferred_fonts");
    let faces = pending.0.clone();
    add_fonts(&faces, cx)?;
    cx.global_mut::<DeferredFonts>().0.clear();
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
                    .all(|face| *face != first_face(Some("Fira Code")))
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
    fn failed_registration_retains_pending_faces_and_only_success_invalidates_layouts(
        cx: &mut gpui::TestAppContext,
    ) {
        crate::test_support::use_real_text_backend_with_startup_font(cx);
        cx.update(|cx| {
            let first = FontGeneration::current();
            cx.global_mut::<DeferredFonts>().0 = vec![b"invalid font"];
            assert!(register_after_first_frame(cx).is_err());
            assert_eq!(cx.global::<DeferredFonts>().0, [b"invalid font".as_slice()]);
            assert_eq!(FontGeneration::current(), first);
            cx.global_mut::<DeferredFonts>().0 = vec![gitcomet_fonts::LILEX_REGULAR_BYTES];
            register_after_first_frame(cx).unwrap();
            let registered = FontGeneration::current();
            assert_ne!(registered, first);
            assert!(!has_deferred_fonts(cx));
            register_after_first_frame(cx).unwrap();
            assert_eq!(FontGeneration::current(), registered);
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
