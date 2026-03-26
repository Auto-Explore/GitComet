use crate::bundled_fonts;
use gitcomet_state::session;
use gpui::{BorrowAppContext, Window};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

pub(crate) const UI_SYSTEM_FONT_FAMILY: &str = ".SystemUIFont";
pub(crate) const DEFAULT_UI_FONT_FAMILY: &str = bundled_fonts::IBM_PLEX_SANS_FONT_FAMILY;
pub(crate) const EDITOR_MONOSPACE_FONT_FAMILY: &str = bundled_fonts::LILEX_FONT_FAMILY;

const LEGACY_EDITOR_MONOSPACE_FONT_FAMILY: &str = "monospace";

static FONT_OPTION_CATALOG: OnceLock<FontOptionCatalog> = OnceLock::new();
static SYSTEM_FONT_CATALOG: OnceLock<SystemFontCatalog> = OnceLock::new();

// These follow the Monaco workbench defaults, but use resolvable real families where the
// CSS stack starts with a platform token such as -apple-system or system-ui.
#[cfg(target_os = "macos")]
const DEFAULT_UI_FONT_CANDIDATES: &[&str] = &[
    bundled_fonts::IBM_PLEX_SANS_FONT_FAMILY,
    "SF Pro Text",
    "SF Pro Display",
    "Helvetica Neue",
    "PingFang SC",
    "Hiragino Sans GB",
    "PingFang TC",
    "Hiragino Kaku Gothic Pro",
    "Apple SD Gothic Neo",
    "Nanum Gothic",
    "AppleGothic",
];
#[cfg(target_os = "windows")]
const DEFAULT_UI_FONT_CANDIDATES: &[&str] = &[
    bundled_fonts::IBM_PLEX_SANS_FONT_FAMILY,
    "Segoe WPC",
    "Segoe UI",
    "Microsoft YaHei",
    "Microsoft Jhenghei",
    "Yu Gothic UI",
    "Meiryo UI",
    "Malgun Gothic",
    "Dotom",
];
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const DEFAULT_UI_FONT_CANDIDATES: &[&str] = &[
    bundled_fonts::IBM_PLEX_SANS_FONT_FAMILY,
    "Ubuntu",
    "Droid Sans",
    "Source Han Sans SC",
    "Source Han Sans CN",
    "Source Han Sans",
    "Source Han Sans TC",
    "Source Han Sans TW",
    "Source Han Sans J",
    "Source Han Sans JP",
    "Source Han Sans K",
    "Source Han Sans JR",
    "UnDotum",
    "FBaekmuk Gulim",
];
#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd"
)))]
const DEFAULT_UI_FONT_CANDIDATES: &[&str] = &[bundled_fonts::IBM_PLEX_SANS_FONT_FAMILY];

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const SYSTEM_UI_FONT_CANDIDATES: &[&str] = &[
    "Noto Sans",
    "Cantarell",
    "Ubuntu",
    "Droid Sans",
    "Liberation Sans",
    "DejaVu Sans",
    "Arial",
    "Helvetica",
    "Source Han Sans SC",
    "Source Han Sans CN",
    "Source Han Sans",
    "Source Han Sans TC",
    "Source Han Sans TW",
    "Source Han Sans J",
    "Source Han Sans JP",
    "Source Han Sans K",
    "Source Han Sans KR",
    "UnDotum",
    "FBaekmuk Gulim",
];

#[cfg(target_os = "macos")]
const DEFAULT_EDITOR_FONT_CANDIDATES: &[&str] = &[
    bundled_fonts::LILEX_FONT_FAMILY,
    bundled_fonts::FIRA_CODE_FONT_FAMILY,
    "SF Mono",
    "Monaco",
    "Menlo",
    "Courier",
];
#[cfg(target_os = "windows")]
const DEFAULT_EDITOR_FONT_CANDIDATES: &[&str] = &[
    bundled_fonts::LILEX_FONT_FAMILY,
    bundled_fonts::FIRA_CODE_FONT_FAMILY,
    "Consolas",
    "Courier New",
];
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const DEFAULT_EDITOR_FONT_CANDIDATES: &[&str] = &[
    bundled_fonts::LILEX_FONT_FAMILY,
    bundled_fonts::FIRA_CODE_FONT_FAMILY,
    "Ubuntu Mono",
    "Liberation Mono",
    "DejaVu Sans Mono",
    "Courier New",
];
#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd"
)))]
const DEFAULT_EDITOR_FONT_CANDIDATES: &[&str] = &[
    bundled_fonts::LILEX_FONT_FAMILY,
    bundled_fonts::FIRA_CODE_FONT_FAMILY,
    "Courier New",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppFontPreferences {
    pub(crate) ui_font_family: String,
    pub(crate) editor_font_family: String,
    initialized: bool,
}

impl Default for AppFontPreferences {
    fn default() -> Self {
        Self {
            ui_font_family: DEFAULT_UI_FONT_FAMILY.to_string(),
            editor_font_family: EDITOR_MONOSPACE_FONT_FAMILY.to_string(),
            initialized: false,
        }
    }
}

impl gpui::Global for AppFontPreferences {}

#[derive(Clone, Debug)]
struct FontOptionCatalog {
    ui_options: Arc<[String]>,
    editor_options: Arc<[String]>,
}

#[derive(Clone, Debug)]
struct SystemFontCatalog {
    all_families: Arc<[String]>,
    monospace_families: Arc<[String]>,
    resolved_system_ui_family: String,
}

pub(crate) fn display_label(font_family: &str) -> String {
    match font_family {
        UI_SYSTEM_FONT_FAMILY => "System Default".to_string(),
        _ => font_family.to_string(),
    }
}

pub(crate) fn ui_font_options(window: &Window) -> Arc<[String]> {
    Arc::clone(&font_option_catalog(window).ui_options)
}

pub(crate) fn editor_font_options(window: &Window) -> Arc<[String]> {
    Arc::clone(&font_option_catalog(window).editor_options)
}

pub(crate) fn applied_ui_font_family(selection: &str) -> String {
    resolve_applied_font_family(selection, &system_font_catalog().resolved_system_ui_family)
}

pub(crate) fn applied_editor_font_family(selection: &str) -> String {
    resolve_applied_font_family(selection, &system_font_catalog().resolved_system_ui_family)
}

pub(crate) fn normalize_ui_font_family(candidate: Option<&str>, options: &[String]) -> String {
    normalize_font_family(candidate, options).unwrap_or_else(|| default_ui_font_family(options))
}

pub(crate) fn normalize_editor_font_family(candidate: Option<&str>, options: &[String]) -> String {
    normalize_editor_font_family_with_monospace_options(
        candidate,
        options,
        system_font_catalog().monospace_families.as_ref(),
    )
}

pub(crate) fn current<C>(cx: &mut C) -> AppFontPreferences
where
    C: BorrowAppContext,
{
    cx.update_default_global::<AppFontPreferences, _>(|prefs, _cx| prefs.clone())
}

pub(crate) fn current_editor_font_family<C>(cx: &mut C) -> String
where
    C: BorrowAppContext,
{
    let selection = current(cx).editor_font_family;
    applied_editor_font_family(&selection)
}

pub(crate) fn current_or_initialize_from_session<C>(
    window: &Window,
    ui_session: &session::UiSession,
    cx: &mut C,
) -> AppFontPreferences
where
    C: BorrowAppContext,
{
    let current = current(cx);
    let next = if current.initialized {
        resolve_for_window(
            window,
            Some(current.ui_font_family.as_str()),
            Some(current.editor_font_family.as_str()),
        )
    } else {
        resolve_for_window(
            window,
            ui_session.ui_font_family.as_deref(),
            ui_session.editor_font_family.as_deref(),
        )
    };
    cx.set_global(next.clone());
    next
}

pub(crate) fn set_current<C>(
    cx: &mut C,
    ui_font_family: String,
    editor_font_family: String,
) -> AppFontPreferences
where
    C: BorrowAppContext,
{
    let next = AppFontPreferences {
        ui_font_family,
        editor_font_family,
        initialized: true,
    };
    cx.set_global(next.clone());
    next
}

fn build_font_options(_window: &Window, specials: &[&str]) -> Vec<String> {
    build_font_options_from_names(system_font_catalog().all_families.as_ref(), specials)
}

fn system_font_catalog() -> &'static SystemFontCatalog {
    SYSTEM_FONT_CATALOG.get_or_init(collect_system_font_catalog)
}

fn collect_system_font_catalog() -> SystemFontCatalog {
    let mut db = fontdb::Database::new();
    bundled_fonts::load_into_fontdb(&mut db);
    db.load_system_fonts();
    let all_families = normalize_font_names(
        db.faces()
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone())),
    );
    let monospace_families = normalize_font_names(
        db.faces()
            .filter(|face| face.monospaced)
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone())),
    );
    let resolved_system_ui_family = resolved_system_ui_font_family(&all_families);

    SystemFontCatalog {
        all_families: all_families.into(),
        monospace_families: monospace_families.into(),
        resolved_system_ui_family,
    }
}

fn normalize_font_names(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut names_by_key = BTreeMap::new();
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if bundled_fonts::should_skip_font_option_alias(trimmed) {
            continue;
        }

        names_by_key
            .entry(trimmed.to_ascii_lowercase())
            .or_insert_with(|| trimmed.to_string());
    }

    names_by_key.into_values().collect()
}

fn build_font_options_from_names(names: &[String], specials: &[&str]) -> Vec<String> {
    let mut options = Vec::with_capacity(specials.len() + names.len());
    options.extend(specials.iter().map(|font| (*font).to_string()));
    options.extend(
        names
            .iter()
            .filter(|name| !specials.contains(&name.as_str()))
            .cloned(),
    );
    options
}

fn normalize_font_family(candidate: Option<&str>, options: &[String]) -> Option<String> {
    candidate
        .filter(|candidate| options.iter().any(|option| option == candidate))
        .map(ToOwned::to_owned)
}

fn normalize_editor_font_family_with_monospace_options(
    candidate: Option<&str>,
    options: &[String],
    monospace_options: &[String],
) -> String {
    normalize_font_family(
        candidate.filter(|candidate| *candidate != LEGACY_EDITOR_MONOSPACE_FONT_FAMILY),
        options,
    )
    .unwrap_or_else(|| default_editor_font_family(options, monospace_options))
}

fn default_ui_font_family(options: &[String]) -> String {
    first_matching_font_family(options, DEFAULT_UI_FONT_CANDIDATES)
        .unwrap_or_else(|| UI_SYSTEM_FONT_FAMILY.to_string())
}

fn default_editor_font_family(options: &[String], monospace_options: &[String]) -> String {
    first_matching_font_family(options, DEFAULT_EDITOR_FONT_CANDIDATES)
        .or_else(|| first_installed_font_family(monospace_options))
        .or_else(|| first_installed_font_family(options))
        .unwrap_or_else(|| EDITOR_MONOSPACE_FONT_FAMILY.to_string())
}

fn first_matching_font_family(options: &[String], candidates: &[&str]) -> Option<String> {
    candidates.iter().find_map(|candidate| {
        options
            .iter()
            .find(|option| option.as_str() == *candidate)
            .cloned()
    })
}

fn first_installed_font_family(options: &[String]) -> Option<String> {
    options
        .iter()
        .find(|option| option.as_str() != UI_SYSTEM_FONT_FAMILY)
        .cloned()
}

fn resolve_applied_font_family(selection: &str, resolved_system_ui_family: &str) -> String {
    if should_resolve_system_ui_font(selection) {
        resolved_system_ui_family.to_string()
    } else {
        selection.to_string()
    }
}

fn resolved_system_ui_font_family(options: &[String]) -> String {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        first_matching_font_family(options, SYSTEM_UI_FONT_CANDIDATES)
            .or_else(|| first_installed_non_bundled_font_family(options))
            .unwrap_or_else(|| DEFAULT_UI_FONT_FAMILY.to_string())
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        UI_SYSTEM_FONT_FAMILY.to_string()
    }
}

fn first_installed_non_bundled_font_family(options: &[String]) -> Option<String> {
    options
        .iter()
        .find(|option| !is_special_font_family(option) && !is_bundled_font_family(option))
        .cloned()
}

fn is_special_font_family(font_family: &str) -> bool {
    font_family == UI_SYSTEM_FONT_FAMILY
}

fn is_bundled_font_family(font_family: &str) -> bool {
    matches!(
        font_family,
        bundled_fonts::FIRA_CODE_FONT_FAMILY
            | bundled_fonts::IBM_PLEX_SANS_FONT_FAMILY
            | bundled_fonts::LILEX_FONT_FAMILY
    )
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn should_resolve_system_ui_font(selection: &str) -> bool {
    selection == UI_SYSTEM_FONT_FAMILY
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn should_resolve_system_ui_font(_selection: &str) -> bool {
    false
}

fn resolve_for_window(
    window: &Window,
    ui_font_family: Option<&str>,
    editor_font_family: Option<&str>,
) -> AppFontPreferences {
    let catalog = font_option_catalog(window);
    AppFontPreferences {
        ui_font_family: normalize_ui_font_family(ui_font_family, &catalog.ui_options),
        editor_font_family: normalize_editor_font_family(
            editor_font_family,
            &catalog.editor_options,
        ),
        initialized: true,
    }
}

fn font_option_catalog(window: &Window) -> &'static FontOptionCatalog {
    FONT_OPTION_CATALOG.get_or_init(|| FontOptionCatalog {
        ui_options: build_font_options(window, &[UI_SYSTEM_FONT_FAMILY, DEFAULT_UI_FONT_FAMILY])
            .into(),
        editor_options: build_font_options(
            window,
            &[EDITOR_MONOSPACE_FONT_FAMILY, UI_SYSTEM_FONT_FAMILY],
        )
        .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_label_maps_special_font_families() {
        assert_eq!(display_label(UI_SYSTEM_FONT_FAMILY), "System Default");
        assert_eq!(
            display_label(DEFAULT_UI_FONT_FAMILY),
            DEFAULT_UI_FONT_FAMILY
        );
        assert_eq!(
            display_label(EDITOR_MONOSPACE_FONT_FAMILY),
            EDITOR_MONOSPACE_FONT_FAMILY
        );
        assert_eq!(display_label("JetBrains Mono"), "JetBrains Mono");
    }

    #[test]
    fn default_ui_font_family_prefers_platform_candidates() {
        let expected = DEFAULT_UI_FONT_CANDIDATES
            .first()
            .copied()
            .unwrap_or(UI_SYSTEM_FONT_FAMILY);
        let options = vec![
            UI_SYSTEM_FONT_FAMILY.to_string(),
            "JetBrains Mono".to_string(),
            DEFAULT_UI_FONT_CANDIDATES
                .get(1)
                .unwrap_or(&"Inter")
                .to_string(),
            DEFAULT_UI_FONT_CANDIDATES
                .first()
                .unwrap_or(&"Inter")
                .to_string(),
        ];

        assert_eq!(default_ui_font_family(&options), expected.to_string());
    }

    #[test]
    fn normalize_ui_font_family_preserves_explicit_selection_and_falls_back_to_platform_default() {
        let expected = DEFAULT_UI_FONT_CANDIDATES
            .first()
            .copied()
            .unwrap_or(UI_SYSTEM_FONT_FAMILY);
        let options = vec![
            UI_SYSTEM_FONT_FAMILY.to_string(),
            DEFAULT_UI_FONT_CANDIDATES
                .first()
                .unwrap_or(&"Inter")
                .to_string(),
            "JetBrains Mono".to_string(),
        ];

        assert_eq!(
            normalize_ui_font_family(Some("JetBrains Mono"), &options),
            "JetBrains Mono".to_string()
        );
        assert_eq!(
            normalize_ui_font_family(Some("Missing Font"), &options),
            expected.to_string()
        );
        assert_eq!(
            normalize_ui_font_family(None, &options),
            expected.to_string()
        );
        assert_eq!(
            normalize_ui_font_family(Some(UI_SYSTEM_FONT_FAMILY), &options),
            UI_SYSTEM_FONT_FAMILY.to_string()
        );
    }

    #[test]
    fn default_editor_font_family_prefers_platform_code_fonts() {
        let options = vec![
            UI_SYSTEM_FONT_FAMILY.to_string(),
            "JetBrains Mono".to_string(),
            DEFAULT_EDITOR_FONT_CANDIDATES
                .get(1)
                .unwrap_or(&bundled_fonts::FIRA_CODE_FONT_FAMILY)
                .to_string(),
            DEFAULT_EDITOR_FONT_CANDIDATES
                .first()
                .unwrap_or(&bundled_fonts::LILEX_FONT_FAMILY)
                .to_string(),
        ];
        let monospace_options = vec![
            DEFAULT_EDITOR_FONT_CANDIDATES
                .get(1)
                .unwrap_or(&bundled_fonts::FIRA_CODE_FONT_FAMILY)
                .to_string(),
            DEFAULT_EDITOR_FONT_CANDIDATES
                .first()
                .unwrap_or(&bundled_fonts::LILEX_FONT_FAMILY)
                .to_string(),
        ];

        assert_eq!(
            default_editor_font_family(&options, &monospace_options),
            DEFAULT_EDITOR_FONT_CANDIDATES
                .first()
                .unwrap_or(&bundled_fonts::LILEX_FONT_FAMILY)
                .to_string()
        );
    }

    #[test]
    fn default_editor_font_family_falls_back_to_first_installed_monospace_font() {
        let options = vec![
            UI_SYSTEM_FONT_FAMILY.to_string(),
            "Inter".to_string(),
            bundled_fonts::LILEX_FONT_FAMILY.to_string(),
            "JetBrains Mono".to_string(),
        ];
        let monospace_options = vec![
            bundled_fonts::LILEX_FONT_FAMILY.to_string(),
            "JetBrains Mono".to_string(),
        ];

        assert_eq!(
            default_editor_font_family(&options, &monospace_options),
            bundled_fonts::LILEX_FONT_FAMILY.to_string()
        );
    }

    #[test]
    fn normalize_editor_font_family_migrates_legacy_monospace_alias_to_real_font() {
        let options = vec![
            UI_SYSTEM_FONT_FAMILY.to_string(),
            DEFAULT_EDITOR_FONT_CANDIDATES
                .first()
                .unwrap_or(&bundled_fonts::LILEX_FONT_FAMILY)
                .to_string(),
            "JetBrains Mono".to_string(),
        ];
        let monospace_options = vec![
            DEFAULT_EDITOR_FONT_CANDIDATES
                .first()
                .unwrap_or(&bundled_fonts::LILEX_FONT_FAMILY)
                .to_string(),
            "JetBrains Mono".to_string(),
        ];

        assert_eq!(
            normalize_editor_font_family_with_monospace_options(
                Some(LEGACY_EDITOR_MONOSPACE_FONT_FAMILY),
                &options,
                &monospace_options,
            ),
            DEFAULT_EDITOR_FONT_CANDIDATES
                .first()
                .unwrap_or(&bundled_fonts::LILEX_FONT_FAMILY)
                .to_string()
        );
    }

    #[test]
    fn normalize_editor_font_family_preserves_explicit_selection() {
        let options = vec![
            UI_SYSTEM_FONT_FAMILY.to_string(),
            DEFAULT_EDITOR_FONT_CANDIDATES
                .first()
                .unwrap_or(&bundled_fonts::LILEX_FONT_FAMILY)
                .to_string(),
            "JetBrains Mono".to_string(),
        ];
        let monospace_options = vec![
            DEFAULT_EDITOR_FONT_CANDIDATES
                .first()
                .unwrap_or(&bundled_fonts::LILEX_FONT_FAMILY)
                .to_string(),
            "JetBrains Mono".to_string(),
        ];

        assert_eq!(
            normalize_editor_font_family_with_monospace_options(
                Some("JetBrains Mono"),
                &options,
                &monospace_options,
            ),
            "JetBrains Mono".to_string()
        );
        assert_eq!(
            normalize_editor_font_family_with_monospace_options(
                Some(UI_SYSTEM_FONT_FAMILY),
                &options,
                &monospace_options,
            ),
            UI_SYSTEM_FONT_FAMILY.to_string()
        );
        assert_eq!(
            normalize_editor_font_family_with_monospace_options(
                Some("Missing Font"),
                &options,
                &monospace_options,
            ),
            DEFAULT_EDITOR_FONT_CANDIDATES
                .first()
                .unwrap_or(&bundled_fonts::LILEX_FONT_FAMILY)
                .to_string()
        );
        assert_eq!(
            normalize_editor_font_family_with_monospace_options(None, &options, &monospace_options,),
            DEFAULT_EDITOR_FONT_CANDIDATES
                .first()
                .unwrap_or(&bundled_fonts::LILEX_FONT_FAMILY)
                .to_string()
        );
    }

    #[test]
    fn normalize_font_names_keeps_only_unique_non_empty_system_families() {
        let names = normalize_font_names([
            "  ".to_string(),
            "Fira Code SemiBold".to_string(),
            "Fira Code".to_string(),
            "IBM Plex Sans SmBld".to_string(),
            "IBM Plex Sans".to_string(),
            "JetBrains Mono".to_string(),
            "jetbrains mono".to_string(),
            "Lilex SemiBold".to_string(),
            "Lilex Italic".to_string(),
            "Lilex".to_string(),
            " Inter ".to_string(),
            "Inter".to_string(),
        ]);

        assert_eq!(
            names,
            vec![
                "Fira Code".to_string(),
                "IBM Plex Sans".to_string(),
                "Inter".to_string(),
                "JetBrains Mono".to_string(),
                "Lilex".to_string(),
            ]
        );
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn resolved_system_ui_font_family_prefers_real_linux_ui_fonts() {
        let options = vec![
            bundled_fonts::IBM_PLEX_SANS_FONT_FAMILY.to_string(),
            "Noto Sans".to_string(),
            bundled_fonts::LILEX_FONT_FAMILY.to_string(),
        ];

        assert_eq!(
            resolved_system_ui_font_family(&options),
            "Noto Sans".to_string()
        );
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn resolved_system_ui_font_family_skips_bundled_fonts_when_falling_back() {
        let options = vec![
            bundled_fonts::FIRA_CODE_FONT_FAMILY.to_string(),
            bundled_fonts::IBM_PLEX_SANS_FONT_FAMILY.to_string(),
            bundled_fonts::LILEX_FONT_FAMILY.to_string(),
            "Inter".to_string(),
        ];

        assert_eq!(
            resolved_system_ui_font_family(&options),
            "Inter".to_string()
        );
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn applied_ui_font_family_maps_system_token_to_resolved_linux_font() {
        assert_eq!(
            resolve_applied_font_family(UI_SYSTEM_FONT_FAMILY, "Noto Sans"),
            "Noto Sans".to_string()
        );
        assert_eq!(
            resolve_applied_font_family("IBM Plex Sans", "Noto Sans"),
            "IBM Plex Sans".to_string()
        );
    }
}
