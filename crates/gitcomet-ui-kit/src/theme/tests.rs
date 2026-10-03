use super::{
    AMBER_DARK_THEME_KEY, AppTheme, DEFAULT_DARK_THEME_KEY, DEFAULT_LIGHT_THEME_KEY,
    EMBEDDED_THEME_FILES, GRAPH_LANE_PALETTE_SIZE, GraphLanePalette, HexColor, Rgba,
    THEME_SCHEMA_VERSION, ThemeColor, UNFILLED_COLOR_TOKENS, available_themes, bundled_theme_keys,
    composite_over, content_header_bg, derived_syntax_color, fill_missing_color_tokens,
    has_theme_key, hsla_from_hue_fraction, layer_over, load_theme_specs_from_json,
    merged_theme_options, resolved_runtime_themes_dir, runtime_theme_issues_with_dir,
    runtime_themes_with_dir, test_theme_bundle_value, test_theme_json_with_syntax, theme_label,
    theme_preview_colors, with_alpha,
};
use palette::IntoColor;
use std::{fs, path::PathBuf};
use tempfile::tempdir;

#[test]
fn layer_over_matches_painting_both_layers_in_turn() {
    let assert_close = |a: Rgba, b: Rgba| {
        for (x, y) in [
            (a.red, b.red),
            (a.green, b.green),
            (a.blue, b.blue),
            (a.alpha, b.alpha),
        ] {
            assert!((x - y).abs() < 1e-5, "{a:?} vs {b:?}");
        }
    };
    let base = Rgba::new(0.9, 0.6, 0.1, 0.10);
    let overlay = Rgba::new(1.0, 1.0, 1.0, 0.06);
    for surface in [gpui::rgb(0x1e2230), gpui::rgb(0xf4f5f7)] {
        assert_close(
            composite_over(composite_over(surface, base), overlay),
            composite_over(surface, layer_over(base, overlay)),
        );
        // An opaque base is the case `composite_over` already covers.
        assert_close(
            layer_over(surface, overlay),
            composite_over(surface, overlay),
        );
    }
}

#[test]
fn theme_hex_colors_keep_all_four_spellings_and_strict_validation() {
    for (input, expected) in [
        ("#aBc", [0xaa_u8, 0xbb, 0xcc, 0xff]),
        ("#aBcD", [0xaa, 0xbb, 0xcc, 0xdd]),
        ("  #a1B2c3\n", [0xa1, 0xb2, 0xc3, 0xff]),
        ("#a1B2c380", [0xa1, 0xb2, 0xc3, 0x80]),
    ] {
        let [r, g, b, a] = expected.map(|channel| f32::from(channel) / 255.0);
        assert_eq!(HexColor::parse(input).unwrap().0, Rgba::new(r, g, b, a));
    }
    for invalid in [
        "abc",
        "#",
        "#12",
        "#12345",
        "#123456789",
        "#aabbccddeeff",
        "##abc",
        "#ab g",
        "#gggggg",
        "#+a0000",
        "#0000+a00",
        "#éab",
        "#💙ab",
    ] {
        assert!(HexColor::parse(invalid).is_err(), "{invalid:?}");
    }
}

fn themes_markdown_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/themes.md")
}

fn readme_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../README.md")
}

fn test_theme_entry(base_key: &str) -> serde_json::Value {
    test_theme_bundle_value(base_key)["themes"][0].take()
}

fn test_theme_bundle_json(name: &str, themes: Vec<serde_json::Value>) -> String {
    serde_json::to_string(&serde_json::json!({
        "schema_version": THEME_SCHEMA_VERSION,
        "name": name,
        "themes": themes,
    }))
    .expect("theme fixture should serialize")
}

fn themes_markdown_example() -> String {
    let markdown = fs::read_to_string(themes_markdown_path())
        .expect("THEMES.md should be readable for theme docs tests");
    let start = markdown
        .find("```javascript")
        .expect("THEMES.md should include a javascript example block");
    let example = &markdown[start + "```javascript".len()..];
    let end = example
        .find("```")
        .expect("THEMES.md example block should be closed");
    example[..end].trim().to_string()
}

fn strip_json_line_comments(json_with_comments: &str) -> String {
    let mut out = String::with_capacity(json_with_comments.len());
    let mut chars = json_with_comments.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }

        if ch == '/' && chars.peek() == Some(&'/') {
            let _ = chars.next();
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }

        out.push(ch);
    }

    out
}

fn relative_luminance(color: Rgba) -> f32 {
    fn linear_channel(channel: f32) -> f32 {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    0.2126 * linear_channel(color.red)
        + 0.7152 * linear_channel(color.green)
        + 0.0722 * linear_channel(color.blue)
}

fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
    let a = relative_luminance(a);
    let b = relative_luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn assert_min_contrast(
    theme_key: &str,
    token: &str,
    foreground: Rgba,
    background: Rgba,
    minimum: f32,
) {
    let actual = contrast_ratio(foreground, background);
    assert!(
        actual >= minimum,
        "{theme_key} {token} contrast was {actual:.2}, expected at least {minimum:.2}"
    );
}

fn syntax_foregrounds(theme: AppTheme) -> Vec<(&'static str, Rgba)> {
    let syntax = theme.syntax;
    vec![
        ("comment", syntax.comment),
        ("comment_doc", syntax.comment_doc),
        ("string", syntax.string),
        ("string_escape", syntax.string_escape),
        ("string_regex", syntax.string_regex),
        ("string_special", syntax.string_special),
        ("keyword", syntax.keyword),
        ("keyword_control", syntax.keyword_control),
        ("preproc", syntax.preproc),
        ("number", syntax.number),
        ("boolean", syntax.boolean),
        ("function", syntax.function),
        ("function_method", syntax.function_method),
        ("function_special", syntax.function_special),
        ("constructor", syntax.constructor),
        ("type", syntax.type_name),
        ("type_builtin", syntax.type_builtin),
        ("type_interface", syntax.type_interface),
        ("namespace", syntax.namespace),
        (
            "variable",
            syntax.variable.unwrap_or(theme.colors.foreground.primary),
        ),
        ("variable_parameter", syntax.variable_parameter),
        ("variable_special", syntax.variable_special),
        ("variable_builtin", syntax.variable_builtin),
        ("property", syntax.property),
        (
            "label",
            syntax.label.unwrap_or(theme.colors.foreground.primary),
        ),
        ("constant", syntax.constant),
        ("constant_builtin", syntax.constant_builtin),
        ("operator", syntax.operator),
        ("punctuation", syntax.punctuation),
        ("punctuation_bracket", syntax.punctuation_bracket),
        ("punctuation_delimiter", syntax.punctuation_delimiter),
        ("punctuation_special", syntax.punctuation_special),
        ("punctuation_list_marker", syntax.punctuation_list_marker),
        ("tag", syntax.tag),
        ("attribute", syntax.attribute),
        ("markup_heading", syntax.markup_heading),
        ("markup_link", syntax.markup_link),
        ("text_literal", syntax.text_literal),
        ("diff_plus", syntax.diff_plus),
        ("diff_minus", syntax.diff_minus),
        ("diff_delta", syntax.diff_delta),
        ("lifetime", syntax.lifetime),
    ]
}

#[test]
fn with_alpha_preserves_rgb_and_overwrites_alpha() {
    let color = Rgba::new(0.1, 0.2, 0.3, 0.4);

    let adjusted = with_alpha(color, 0.75);

    assert_eq!(adjusted.red, color.red);
    assert_eq!(adjusted.green, color.green);
    assert_eq!(adjusted.blue, color.blue);
    assert_eq!(adjusted.alpha, 0.75);
}

#[test]
fn rejects_theme_bundle_without_schema_version() {
    // Carries a real theme, not `"themes": []`: an empty bundle parses
    // structurally whatever the schema, so it cannot tell whether the
    // version was checked before the structure or after it.
    let json = serde_json::to_string(&serde_json::json!({
        "name": "Missing version",
        "themes": [test_theme_entry(DEFAULT_DARK_THEME_KEY)],
    }))
    .expect("theme fixture should serialize");
    let error = load_theme_specs_from_json(&json)
        .err()
        .expect("a theme bundle without schema_version must be rejected");

    assert!(
        error.to_string().contains("schema_version"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_unsupported_theme_schema_versions() {
    let json = serde_json::to_string(&serde_json::json!({
        "schema_version": 999,
        "name": "Future",
        "themes": [test_theme_entry(DEFAULT_DARK_THEME_KEY)],
    }))
    .expect("theme fixture should serialize");
    let error = load_theme_specs_from_json(&json)
        .err()
        .expect("unsupported schema version must be rejected");

    assert_eq!(
        error.to_string(),
        format!("unsupported theme schema version 999; expected {THEME_SCHEMA_VERSION}")
    );
}

/// The v1 -> v2 break reshaped the colour groups, and every struct in the
/// chain is `deny_unknown_fields`. A v1 file therefore fails the structural
/// parse first unless the version is read off the raw JSON, and the author
/// gets `invalid type: string ..., expected struct ThemeFileAccentColors`
/// instead of being told which version their file is.
#[test]
fn a_v1_theme_reports_its_version_rather_than_a_type_error() {
    let error = load_theme_specs_from_json(
        r##"{
                "schema_version": 1,
                "name": "Old",
                "themes": [{
                    "key": "old",
                    "name": "Old",
                    "appearance": "dark",
                    "colors": { "accent": "#59b7ffff", "background": "#101014ff" }
                }]
            }"##,
    )
    .err()
    .expect("a v1 theme must be rejected");

    assert_eq!(
        error.to_string(),
        format!("unsupported theme schema version 1; expected {THEME_SCHEMA_VERSION}")
    );
}

/// `AppTheme` is `Copy` and captured by value into every per-row paint
/// closure, so its size is paid per visible row per frame. The bound is
/// deliberately loose -- it is a tripwire for a field that brings a large
/// array along, not a budget to tune against.
#[test]
fn app_theme_stays_small_enough_to_copy_per_row() {
    let size = std::mem::size_of::<AppTheme>();
    assert!(
        size <= 2048,
        "AppTheme grew to {size} bytes; interning large fields (see \
             `intern_lane_palette`) keeps per-row paint closures cheap"
    );
}

/// Two themes built from the same palette share one interned copy, so
/// reloading a theme file does not leak another kilobyte each time.
#[test]
fn lane_palettes_are_interned_across_theme_loads() {
    let first = AppTheme::from_key(DEFAULT_DARK_THEME_KEY).expect("bundled theme should load");
    let reparsed = AppTheme::from_json_str(
        &serde_json::to_string(&test_theme_bundle_value(DEFAULT_DARK_THEME_KEY))
            .expect("fixture should serialize"),
    )
    .expect("fixture should parse");

    assert!(
        std::ptr::eq(first.graph_lane_palette, reparsed.graph_lane_palette),
        "an identical palette must reuse the interned one"
    );
}

/// The one exclusion in the token fill. A theme that names neither lane token
/// must keep the palette generated for its appearance -- filling these from
/// the bundled theme would repaint the graph of every custom theme in the
/// wild with gitcomet's own lane colours.
#[test]
fn omitting_both_lane_tokens_generates_a_palette_instead_of_inheriting_one() {
    let bundled = AppTheme::from_key(DEFAULT_DARK_THEME_KEY).expect("bundled theme should load");
    let mut fixture = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
    let colors = fixture["themes"][0]["colors"]
        .as_object_mut()
        .expect("colors should be an object");
    for token in UNFILLED_COLOR_TOKENS {
        assert!(
            colors.contains_key(*token),
            "fixture should start with {token} so removing it means something"
        );
        colors.remove(*token);
    }

    let theme = AppTheme::from_json_str(
        &serde_json::to_string(&fixture).expect("fixture should serialize"),
    )
    .expect("a theme without lane tokens should load");

    assert_eq!(
        theme.graph_lane_palette.as_slice(),
        GraphLanePalette::generated(true).as_slice(),
        "lane colours must be generated for the appearance, not inherited"
    );
    assert_ne!(
        theme.graph_lane_palette.as_slice(),
        bundled.graph_lane_palette.as_slice(),
        "and specifically not the bundled theme's hand-picked lanes"
    );
}

/// Interning must reuse an allocation only for palettes that are actually the
/// same: a looser comparison would collapse every theme onto one palette.
#[test]
fn different_lane_palettes_are_interned_separately() {
    use serde_json::json;

    let theme_with_hues = |hues: serde_json::Value| {
        let mut fixture = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
        fixture["themes"][0]["colors"]["graph_lane_palette"] = serde_json::Value::Null;
        fixture["themes"][0]["colors"]["graph_lane_hues"] = hues;
        AppTheme::from_json_str(&serde_json::to_string(&fixture).expect("fixture should serialize"))
            .expect("fixture should parse")
    };

    let one = theme_with_hues(json!([0.1, 0.2]));
    let other = theme_with_hues(json!([0.6, 0.8]));
    assert_ne!(
        one.graph_lane_palette.as_slice(),
        other.graph_lane_palette.as_slice(),
        "fixture must actually produce two different palettes"
    );
    assert!(
        !std::ptr::eq(one.graph_lane_palette, other.graph_lane_palette),
        "distinct palettes must not share an interned allocation"
    );
}

/// A palette carrying NaN (an out-of-range hue reaches `rem_euclid` as
/// infinity) is never `PartialEq` to itself, so a `PartialEq`-based interner
/// would leak a fresh copy on every parse.
#[test]
fn a_palette_with_non_finite_channels_still_interns_once() {
    use serde_json::json;

    let parse_nan_theme = || {
        let mut fixture = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
        fixture["themes"][0]["colors"]["graph_lane_palette"] = serde_json::Value::Null;
        fixture["themes"][0]["colors"]["graph_lane_hues"] = json!([1e40]);
        AppTheme::from_json_str(&serde_json::to_string(&fixture).expect("fixture should serialize"))
            .expect("fixture should parse")
    };

    let first = parse_nan_theme();
    assert!(
        first
            .graph_lane_palette
            .as_slice()
            .iter()
            .any(|color| color.red.is_nan() || color.green.is_nan() || color.blue.is_nan()),
        "fixture must actually produce a non-finite channel"
    );
    assert!(
        std::ptr::eq(
            first.graph_lane_palette,
            parse_nan_theme().graph_lane_palette
        ),
        "re-parsing the same palette must reuse the interned copy, NaN or not"
    );
}

/// A theme file may leave tokens out; it may not misspell them. The first
/// half is what keeps themes written against an older token set loading, the
/// second is what stops a typo from quietly doing nothing.
#[test]
fn missing_semantic_tokens_fall_back_and_unknown_ones_are_rejected() {
    use serde_json::json;

    for (base_key, appearance) in [
        (DEFAULT_DARK_THEME_KEY, "dark"),
        (DEFAULT_LIGHT_THEME_KEY, "light"),
    ] {
        let bundled = AppTheme::from_key(base_key).expect("bundled theme should load");
        let mut missing = test_theme_bundle_value(base_key);
        missing["themes"][0]["colors"]["surface"]
            .as_object_mut()
            .expect("surface should be an object")
            .remove("input");
        // Not the bundled palette: a token filled from the wrong appearance
        // would still parse, so the fixture has to differ somewhere visible.
        missing["themes"][0]["colors"]["surface"]["canvas"] = json!("#123456ff");
        let filled = AppTheme::from_json_str(
            &serde_json::to_string(&missing).expect("fixture should serialize"),
        )
        .expect("a theme that omits a token should load");
        assert_eq!(
            filled.colors.surface.input, bundled.colors.surface.input,
            "omitted token should come from the bundled {appearance} theme"
        );
        assert_eq!(
            filled.colors.surface.canvas,
            gpui::rgba(0x123456ff),
            "a token the file does define must survive the fill"
        );
    }

    // A colour is filled or not filled whole. Half of one is still an error:
    // inheriting the bundled `alpha` would render a colour the author *did*
    // write at an opacity they never asked for, with nothing to show for it.
    let mut half_color = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
    assert!(
        half_color["themes"][0]["colors"]["interaction"]["focus_background"]
            .get("alpha")
            .is_some(),
        "fixture token must use the {{hex, alpha}} form for this to mean anything"
    );
    half_color["themes"][0]["colors"]["interaction"]["focus_background"] =
        json!({ "hex": "#5ac1feff" });
    let half_error = AppTheme::from_json_str(
        &serde_json::to_string(&half_color).expect("fixture should serialize"),
    )
    .expect_err("a colour missing its alpha must fail rather than inherit one");
    assert!(
        half_error.to_string().contains("ThemeColor"),
        "unexpected error: {half_error}"
    );

    // Identity is never filled in: a file that omits its key would otherwise
    // inherit the bundled theme's and collide with it in the picker.
    let mut no_key = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
    no_key["themes"][0]
        .as_object_mut()
        .expect("theme entry should be an object")
        .remove("key");
    let key_error =
        AppTheme::from_json_str(&serde_json::to_string(&no_key).expect("fixture should serialize"))
            .expect_err("a theme without a key must fail");
    assert!(
        key_error.to_string().contains("missing field `key`"),
        "unexpected error: {key_error}"
    );

    let mut unknown = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
    unknown["themes"][0]["colors"]["surface"]["mystery"] = json!("#000000ff");
    let unknown_error = AppTheme::from_json_str(
        &serde_json::to_string(&unknown).expect("fixture should serialize"),
    )
    .expect_err("unknown semantic token must fail");
    assert!(
        unknown_error
            .to_string()
            .contains("unknown field `mystery`"),
        "unexpected error: {unknown_error}"
    );
}

#[test]
fn parses_theme_json_with_alpha_overrides() {
    use serde_json::json;

    let mut fixture = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
    let theme = &mut fixture["themes"][0];
    theme["key"] = json!("fixture");
    theme["name"] = json!("Fixture");
    theme["colors"]["surface"]["canvas"] = json!("#0d1016ff");
    theme["colors"]["stroke"]["default"] = json!("#2d2f34ff");
    theme["colors"]["tooltip"]["background"] = json!("#000000ff");
    theme["colors"]["tooltip"]["foreground"] = json!("#ffffffff");
    theme["colors"]["interaction"]["pressed_background"] =
        json!({ "hex": "#2d2f34ff", "alpha": 0.78 });
    theme["colors"]["scrollbar"]["thumb_pressed"] = json!({ "hex": "#8a8986ff", "alpha": 0.52 });
    theme["colors"]["diff"]["added"]["background"] = json!("#102030ff");
    theme["colors"]["diff"]["added"]["foreground"] = json!("#405060ff");
    theme["colors"]["diff"]["removed"]["background"] = json!("#203040ff");
    theme["colors"]["diff"]["removed"]["foreground"] = json!("#506070ff");
    theme["colors"]["foreground"]["placeholder"] = json!("#708090ff");
    theme["colors"]["accent"]["on_solid"] = json!("#112233ff");
    theme["colors"]["foreground"]["emphasis"] = json!("#a1b2c3ff");
    theme["colors"]["graph_lane_palette"] = serde_json::Value::Null;
    theme["colors"]["graph_lane_hues"] = json!([0.25, 0.75]);
    theme["radii"] = json!({ "panel": 2.0, "pill": 2.0, "row": 2.0 });
    theme
        .as_object_mut()
        .expect("theme should be an object")
        .remove("syntax");

    let theme = AppTheme::from_json_str(
        &serde_json::to_string(&fixture).expect("fixture should serialize"),
    )
    .expect("theme JSON should parse");

    assert!(theme.is_dark);
    assert_eq!(theme.colors.surface.canvas, gpui::rgba(0x0d1016ff));
    assert_eq!(theme.colors.stroke.default, gpui::rgba(0x2d2f34ff));
    assert_eq!(theme.colors.tooltip.background, gpui::rgba(0x000000ff));
    assert_eq!(theme.colors.tooltip.foreground, gpui::rgba(0xffffffff));
    assert_eq!(
        theme.colors.interaction.pressed_background,
        with_alpha(gpui::rgba(0x2d2f34ff), 0.78)
    );
    assert_eq!(
        theme.colors.scrollbar.thumb_pressed,
        with_alpha(gpui::rgba(0x8a8986ff), 0.52)
    );
    assert_eq!(theme.colors.diff.added.background, gpui::rgba(0x102030ff));
    assert_eq!(theme.colors.diff.added.foreground, gpui::rgba(0x405060ff));
    assert_eq!(theme.colors.diff.removed.background, gpui::rgba(0x203040ff));
    assert_eq!(theme.colors.diff.removed.foreground, gpui::rgba(0x506070ff));
    assert_eq!(theme.colors.foreground.placeholder, gpui::rgba(0x708090ff));
    assert_eq!(theme.colors.accent.on_solid, gpui::rgba(0x112233ff));
    assert_eq!(theme.colors.foreground.emphasis, gpui::rgba(0xa1b2c3ff));
    assert_eq!(theme.graph_lane_palette.as_slice().len(), 2);
    assert_eq!(
        theme.graph_lane_palette.as_slice()[0],
        hsla_from_hue_fraction(0.25, 0.75, 0.62, 1.0).into_color()
    );
    assert_eq!(theme.syntax.comment, theme.colors.foreground.secondary);
    assert_eq!(
        theme.syntax.keyword,
        derived_syntax_color(theme.is_dark, &theme.colors, theme.colors.accent.foreground)
    );
    assert_eq!(theme.syntax.variable, None);
    assert_eq!(theme.radii.panel, 2.0);
}

#[test]
fn short_custom_lane_palettes_wrap_instead_of_painting_transparent_lanes() {
    let palette = GraphLanePalette::from_theme_colors(
        true,
        Some(vec![
            ThemeColor::Hex(HexColor(gpui::rgba(0x112233ff))),
            ThemeColor::Hex(HexColor(gpui::rgba(0x445566ff))),
            ThemeColor::Hex(HexColor(gpui::rgba(0x778899ff))),
        ]),
        None,
    );
    assert_eq!(palette.as_slice().len(), 3);

    // Lane indices are handed out cyclically over the full palette size, so
    // a three-colour theme must keep reusing its own colours rather than
    // reading the transparent tail of the backing array.
    for ix in 0..(GRAPH_LANE_PALETTE_SIZE as u8) {
        let color = palette.color_at(ix);
        assert_eq!(color, palette.as_slice()[usize::from(ix) % 3]);
        assert!(color.alpha > 0.0, "lane {ix} would be invisible");
    }
}

#[test]
fn generated_lane_palette_matches_the_theme_ramp() {
    // `lane_color` reads the theme palette; themes that ship no explicit
    // lane colours must fall back to the generated ramp.
    for is_dark in [true, false] {
        let palette = GraphLanePalette::generated(is_dark);
        let light = if is_dark { 0.62 } else { 0.33 };
        for ix in 0..(GRAPH_LANE_PALETTE_SIZE as u8) {
            let hue = (f32::from(ix) * 0.13) % 1.0;
            assert_eq!(
                palette.color_at(ix),
                hsla_from_hue_fraction(hue, 0.75, light, 1.0).into_color(),
                "lane {ix} (is_dark={is_dark})"
            );
        }
    }
}

/// The ramp tests above compare [`hsla_from_hue_fraction`] against itself,
/// so they hold just as well when every lane is the same colour. This pins
/// the mapping itself: a hue fraction has to reach the primary it names.
///
/// `gpui::hsla` fails this — it clamps the fraction and then reads it as
/// degrees, so 1/3 and 2/3 both come back red.
#[test]
fn hue_fractions_reach_the_primaries_they_name() {
    let cases = [
        (0.0 / 3.0, [1.0, 0.0, 0.0]),
        (1.0 / 3.0, [0.0, 1.0, 0.0]),
        (2.0 / 3.0, [0.0, 0.0, 1.0]),
        // A hue is cyclic, so a full turn is the same red it started on.
        (3.0 / 3.0, [1.0, 0.0, 0.0]),
    ];

    for (hue, [r, g, b]) in cases {
        let color: Rgba = hsla_from_hue_fraction(hue, 1.0, 0.5, 1.0).into_color();
        let actual = [color.red, color.green, color.blue];
        for (channel, (actual, expected)) in actual.iter().zip([r, g, b]).enumerate() {
            assert!(
                (actual - expected).abs() < 1e-4,
                "hue {hue}: channel {channel} was {actual}, expected {expected} \
                     (got {actual:?} for the whole colour)"
            );
        }
    }
}

/// Distinct authors must land on distinct hues, not four shades of one.
#[test]
fn author_colors_spread_across_the_hue_wheel() {
    let theme = AppTheme::gitcomet_dark();
    let names = [
        "Ada Lovelace",
        "Grace Hopper",
        "Alan Turing",
        "Barbara Liskov",
    ];
    let mut hues = names.map(|name| {
        let color: gpui::Hsla = crate::components::author_color(theme, name).into_color();
        color.hue.into_positive_degrees()
    });
    hues.sort_by(f32::total_cmp);

    // Under the bug this guards, every hue lands under one degree, so the
    // whole spread collapses and adjacent authors become indistinguishable.
    let spread = hues[hues.len() - 1] - hues[0];
    assert!(
        spread > 90.0,
        "author hues span only {spread}°, so they have collapsed onto one colour: {hues:?}"
    );
    for pair in hues.windows(2) {
        assert!(
            pair[1] - pair[0] > 1.0,
            "author hues {:?} and {:?} are within a degree of each other: {hues:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn parses_theme_json_with_optional_syntax_overrides() {
    let theme = AppTheme::from_json_str(&test_theme_json_with_syntax(
        DEFAULT_LIGHT_THEME_KEY,
        r##"{
                "keyword": "#112233ff",
                "variable": "#445566ff",
                "comment_doc": "#778899ff",
                "diff_plus": "#aabbccff",
                "label": "#998877ff"
            }"##,
    ))
    .expect("theme JSON should parse");

    assert_eq!(theme.syntax.keyword, gpui::rgba(0x112233ff));
    assert_eq!(theme.syntax.variable, Some(gpui::rgba(0x445566ff)));
    assert_eq!(theme.syntax.comment_doc, gpui::rgba(0x778899ff));
    assert_eq!(theme.syntax.diff_plus, gpui::rgba(0xaabbccff));
    assert_eq!(theme.syntax.label, Some(gpui::rgba(0x998877ff)));
    assert_eq!(theme.syntax.comment, theme.colors.foreground.secondary);
    assert_eq!(
        theme.syntax.string,
        derived_syntax_color(
            theme.is_dark,
            &theme.colors,
            theme.colors.status.warning.foreground
        )
    );
}

#[test]
fn specialized_syntax_categories_fallback_to_base_categories() {
    let theme = AppTheme::from_json_str(&test_theme_json_with_syntax(
        DEFAULT_LIGHT_THEME_KEY,
        r##"{
                "string": "#112233ff",
                "keyword": "#223344ff",
                "type": "#334455ff",
                "variable": "#445566ff",
                "variable_special": "#556677ff",
                "constant": "#667788ff",
                "punctuation": "#778899ff"
            }"##,
    ))
    .expect("theme JSON should parse");

    assert_eq!(theme.syntax.string_regex, gpui::rgba(0x112233ff));
    assert_eq!(theme.syntax.string_special, gpui::rgba(0x112233ff));
    assert_eq!(theme.syntax.preproc, gpui::rgba(0x223344ff));
    assert_eq!(theme.syntax.namespace, gpui::rgba(0x334455ff));
    assert_eq!(theme.syntax.label, Some(gpui::rgba(0x445566ff)));
    assert_eq!(theme.syntax.variable_builtin, gpui::rgba(0x556677ff));
    assert_eq!(theme.syntax.constant_builtin, gpui::rgba(0x667788ff));
    assert_eq!(theme.syntax.punctuation_special, gpui::rgba(0x778899ff));
    assert_eq!(theme.syntax.punctuation_list_marker, gpui::rgba(0x778899ff));
    assert_eq!(theme.syntax.markup_heading, gpui::rgba(0x223344ff));
    assert_eq!(theme.syntax.markup_link, gpui::rgba(0x112233ff));
    assert_eq!(theme.syntax.text_literal, gpui::rgba(0x112233ff));
    assert_eq!(theme.syntax.diff_plus, gpui::rgba(0x112233ff));
    assert_eq!(theme.syntax.diff_minus, gpui::rgba(0x223344ff));
    assert_eq!(theme.syntax.diff_delta, gpui::rgba(0x334455ff));
}

#[test]
fn specialized_syntax_overrides_beat_base_category_fallbacks() {
    let theme = AppTheme::from_json_str(&test_theme_json_with_syntax(
        DEFAULT_DARK_THEME_KEY,
        r##"{
                "string": "#111111ff",
                "keyword": "#222222ff",
                "type": "#333333ff",
                "variable": "#444444ff",
                "variable_special": "#555555ff",
                "constant": "#666666ff",
                "punctuation": "#777777ff",
                "function": "#888888ff",
                "string_regex": "#010101ff",
                "string_special": "#020202ff",
                "preproc": "#030303ff",
                "constructor": "#040404ff",
                "namespace": "#050505ff",
                "variable_builtin": "#060606ff",
                "label": "#070707ff",
                "constant_builtin": "#080808ff",
                "punctuation_special": "#090909ff",
                "punctuation_list_marker": "#0a0a0aff",
                "markup_heading": "#0b0b0bff",
                "markup_link": "#0c0c0cff",
                "text_literal": "#0d0d0dff",
                "diff_plus": "#0e0e0eff",
                "diff_minus": "#0f0f0fff",
                "diff_delta": "#101010ff"
            }"##,
    ))
    .expect("theme JSON should parse");

    assert_eq!(theme.syntax.string_regex, gpui::rgba(0x010101ff));
    assert_eq!(theme.syntax.string_special, gpui::rgba(0x020202ff));
    assert_eq!(theme.syntax.preproc, gpui::rgba(0x030303ff));
    assert_eq!(theme.syntax.constructor, gpui::rgba(0x040404ff));
    assert_eq!(theme.syntax.namespace, gpui::rgba(0x050505ff));
    assert_eq!(theme.syntax.variable_builtin, gpui::rgba(0x060606ff));
    assert_eq!(theme.syntax.label, Some(gpui::rgba(0x070707ff)));
    assert_eq!(theme.syntax.constant_builtin, gpui::rgba(0x080808ff));
    assert_eq!(theme.syntax.punctuation_special, gpui::rgba(0x090909ff));
    assert_eq!(theme.syntax.punctuation_list_marker, gpui::rgba(0x0a0a0aff));
    assert_eq!(theme.syntax.markup_heading, gpui::rgba(0x0b0b0bff));
    assert_eq!(theme.syntax.markup_link, gpui::rgba(0x0c0c0cff));
    assert_eq!(theme.syntax.text_literal, gpui::rgba(0x0d0d0dff));
    assert_eq!(theme.syntax.diff_plus, gpui::rgba(0x0e0e0eff));
    assert_eq!(theme.syntax.diff_minus, gpui::rgba(0x0f0f0fff));
    assert_eq!(theme.syntax.diff_delta, gpui::rgba(0x101010ff));
}

#[test]
fn loads_theme_json_from_file() {
    let dir = tempdir().expect("temp dir should exist");
    let path = dir.path().join("theme.json");
    let fixture = test_theme_bundle_value(DEFAULT_LIGHT_THEME_KEY);
    fs::write(
        &path,
        serde_json::to_string(&fixture).expect("fixture should serialize"),
    )
    .expect("theme file should be written");

    let theme = AppTheme::from_json_path(&path).expect("theme file should load");

    assert!(!theme.is_dark);
    assert_eq!(theme.colors.surface.canvas, gpui::rgba(0xffffffff));
    assert_eq!(theme.colors.foreground.primary, gpui::rgba(0x111827ff));
    assert_eq!(
        theme.graph_lane_palette.as_slice().len(),
        GRAPH_LANE_PALETTE_SIZE
    );
}

#[test]
fn built_in_themes_load_from_embedded_json() {
    let dark = AppTheme::gitcomet_dark();
    let light = AppTheme::gitcomet_light();

    assert!(dark.is_dark);
    assert!(!light.is_dark);
    assert_eq!(
        dark.colors.interaction.focus_ring,
        with_alpha(gpui::rgba(0x4f8ef7ff), 0.74)
    );
    assert_eq!(light.colors.surface.canvas, gpui::rgba(0xffffffff));
    assert_eq!(light.colors.surface.panel, gpui::rgba(0xf2f4f7ff));
    assert_eq!(light.colors.surface.raised, gpui::rgba(0xf8fafcff));
    assert_eq!(light.colors.surface.chrome, gpui::rgba(0xdfe3eaff));
    assert_eq!(light.colors.stroke.default, gpui::rgba(0xaeb7c4ff));
    assert_eq!(light.colors.foreground.primary, gpui::rgba(0x111827ff));
    assert_eq!(light.colors.foreground.secondary, gpui::rgba(0x465166ff));
    assert_eq!(light.colors.accent.foreground, gpui::rgba(0x365bb7ff));
    assert_eq!(
        light.colors.scrollbar.thumb_hover,
        with_alpha(gpui::rgba(0x465166ff), 0.52)
    );
    assert_eq!(
        dark.colors.diff.added.background,
        with_alpha(gpui::rgba(0x76d39cff), 0.15)
    );
    assert_eq!(light.colors.diff.removed.foreground, gpui::rgba(0xa52a35ff));
    assert_eq!(dark.colors.foreground.placeholder, gpui::rgba(0x8d94a3ff));
    assert_eq!(light.colors.accent.on_solid, gpui::rgba(0xffffffff));
    assert_eq!(dark.colors.foreground.emphasis, gpui::rgba(0xffffffff));
    assert_eq!(light.colors.foreground.emphasis, gpui::rgba(0x000000ff));
    assert_eq!(dark.syntax.comment, gpui::rgba(0x6f7b94ff));
    assert_eq!(dark.syntax.keyword, gpui::rgba(0xedb981ff));
    assert_eq!(dark.syntax.keyword_control, dark.syntax.keyword);
    assert_eq!(dark.syntax.preproc, gpui::rgba(0xa79aebff));
    assert_eq!(dark.syntax.string, gpui::rgba(0xbbd57fff));
    assert_eq!(dark.syntax.string_regex, dark.syntax.string);
    assert_eq!(dark.syntax.function_method, gpui::rgba(0x5ac1feff));
    assert_eq!(dark.syntax.function_special, dark.syntax.function_method);
    assert_eq!(dark.syntax.property, dark.syntax.function_method);
    assert_eq!(dark.syntax.namespace, dark.syntax.function_method);
    assert_eq!(dark.syntax.markup_link, dark.syntax.function_method);
    assert_eq!(dark.syntax.type_name, gpui::rgba(0xbbd57fff));
    assert_eq!(dark.syntax.type_builtin, dark.syntax.type_name);
    assert_eq!(dark.syntax.number, gpui::rgba(0xe4a688ff));
    assert_eq!(dark.syntax.constant, gpui::rgba(0xde9fc1ff));
    assert_eq!(dark.syntax.constant_builtin, dark.syntax.constant);
    assert_eq!(dark.syntax.variable, Some(dark.colors.foreground.primary));
    assert_eq!(
        dark.syntax.variable_parameter,
        dark.colors.foreground.primary
    );
    assert_eq!(dark.syntax.variable_special, dark.colors.foreground.primary);
    assert_eq!(dark.syntax.operator, gpui::rgba(0x8d96aaff));
    assert_eq!(dark.syntax.punctuation, dark.syntax.operator);
    assert_eq!(dark.syntax.diff_delta, dark.syntax.function_method);
    assert_eq!(dark.syntax.diff_plus, gpui::rgba(0xbbf7d0ff));
    assert_eq!(dark.syntax.diff_minus, gpui::rgba(0xfecacaff));
    assert_eq!(light.syntax.comment, gpui::rgba(0x4b556aff));
    assert_eq!(light.syntax.keyword, gpui::rgba(0x7f470cff));
    assert_eq!(light.syntax.keyword_control, light.syntax.keyword);
    assert_eq!(light.syntax.preproc, gpui::rgba(0x5745a7ff));
    assert_eq!(light.syntax.string, gpui::rgba(0x455c0eff));
    assert_eq!(light.syntax.string_special, light.syntax.string);
    assert_eq!(light.syntax.function, gpui::rgba(0x005b80ff));
    assert_eq!(light.syntax.function_method, light.syntax.function);
    assert_eq!(light.syntax.function_special, light.syntax.function);
    assert_eq!(light.syntax.property, light.syntax.function);
    assert_eq!(light.syntax.namespace, light.syntax.function);
    assert_eq!(light.syntax.markup_link, light.syntax.function);
    assert_eq!(light.syntax.type_name, gpui::rgba(0x455c0eff));
    assert_eq!(light.syntax.type_builtin, light.syntax.type_name);
    assert_eq!(light.syntax.constructor, light.syntax.function);
    assert_eq!(light.syntax.constant, gpui::rgba(0x7c4261ff));
    assert_eq!(light.syntax.constant_builtin, light.syntax.constant);
    assert_eq!(light.syntax.number, gpui::rgba(0x814431ff));
    assert_eq!(light.syntax.variable, Some(light.colors.foreground.primary));
    assert_eq!(
        light.syntax.variable_parameter,
        light.colors.foreground.primary
    );
    assert_eq!(
        light.syntax.variable_special,
        light.colors.foreground.primary
    );
    assert_eq!(light.syntax.operator, gpui::rgba(0x49556bff));
    assert_eq!(light.syntax.punctuation, light.syntax.operator);
    assert_eq!(light.syntax.diff_delta, light.syntax.function);
    assert_eq!(
        dark.graph_lane_palette.as_slice().len(),
        GRAPH_LANE_PALETTE_SIZE
    );
}

#[test]
fn gitcomet_dark_uses_the_tuned_neutral_and_diff_palette() {
    let theme = AppTheme::gitcomet_dark();
    let colors = theme.colors;

    assert_eq!(colors.surface.canvas, gpui::rgba(0x0d0f13ff));
    assert_eq!(colors.surface.chrome, gpui::rgba(0x1f232bff));
    assert_eq!(colors.surface.panel, gpui::rgba(0x1b1e25ff));
    assert_eq!(colors.surface.raised, gpui::rgba(0x232731ff));
    assert_eq!(colors.interaction.hover_background, gpui::rgba(0x222632ff));
    assert_eq!(
        colors.interaction.pressed_background,
        with_alpha(gpui::rgba(0x2c3242ff), 0.80)
    );
    assert_eq!(
        colors.interaction.selected_background,
        gpui::rgba(0x2c3242ff)
    );
    assert_eq!(colors.accent.foreground, gpui::rgba(0x5393fcff));
    assert_eq!(colors.status.danger.foreground, gpui::rgba(0xf0625dff));
    assert_eq!(colors.status.warning.foreground, gpui::rgba(0xf2a53aff));
    assert_eq!(colors.status.success.foreground, gpui::rgba(0x33c06bff));
    assert_eq!(colors.diff.added.foreground, gpui::rgba(0x76d39cff));
    assert_eq!(
        colors.diff.added.background,
        with_alpha(gpui::rgba(0x76d39cff), 0.15)
    );
    assert_eq!(
        colors.diff.removed.background,
        with_alpha(gpui::rgba(0xe78782ff), 0.15)
    );
    assert_eq!(colors.tooltip.background, gpui::rgba(0x232731ff));
}

#[test]
fn built_in_tokyo_night_theme_loads_from_embedded_json() {
    let theme = AppTheme::from_key("tokyo_night").expect("Tokyo Night theme should load");

    assert!(theme.is_dark);
    assert_eq!(theme.colors.surface.canvas, gpui::rgba(0x0d0f13ff));
    assert_eq!(theme.colors.foreground.emphasis, gpui::rgba(0xffffffff));
    assert_eq!(theme.syntax.keyword, gpui::rgba(0xbb9af7ff));
    assert_eq!(theme.syntax.string, gpui::rgba(0x9ece6aff));
    assert_eq!(theme.syntax.string_regex, gpui::rgba(0xff9e64ff));
    assert_eq!(theme.syntax.diff_minus, gpui::rgba(0xf7768eff));
    assert_eq!(theme.syntax.variable, Some(gpui::rgba(0xc0caf5ff)));
}

#[test]
fn built_in_amber_dark_theme_loads_from_embedded_json() {
    let theme = AppTheme::from_key(AMBER_DARK_THEME_KEY).expect("Amber Dark theme should load");

    assert!(theme.is_dark);
    assert_eq!(theme.colors.surface.canvas, gpui::rgba(0x0d0f13ff));
    assert_eq!(theme.colors.surface.panel, gpui::rgba(0x161922ff));
    assert_eq!(
        theme.colors.interaction.hover_background,
        gpui::rgba(0x1e2230ff)
    );
    assert_eq!(theme.colors.foreground.primary, gpui::rgba(0xe7e8ecff));
    assert_eq!(theme.colors.accent.foreground, gpui::rgba(0xe3a64bff));
    assert_eq!(
        theme.colors.interaction.selected_background,
        with_alpha(gpui::rgba(0xe3a64bff), 0.32)
    );
    assert_eq!(
        theme.colors.interaction.selected_indicator,
        theme.colors.accent.solid
    );
    assert_eq!(theme.colors.diff.added.foreground, gpui::rgba(0x5fa779ff));
    assert_eq!(theme.syntax.keyword, gpui::rgba(0xe3a64bff));
    assert_eq!(theme.syntax.function, gpui::rgba(0x6cb8e8ff));
    assert_eq!(theme.radii.panel, 8.0);
    assert_eq!(theme.radii.row, 4.0);
    assert_eq!(
        theme_label(AMBER_DARK_THEME_KEY),
        Some("Amber Dark".to_string())
    );
}

#[test]
fn built_in_sunset_veil_theme_loads_from_embedded_json() {
    let theme = AppTheme::from_key("sunset_veil").expect("Sunset Veil theme should load");

    assert!(!theme.is_dark);
    assert_eq!(theme.colors.surface.canvas, gpui::rgba(0xfff7edff));
    assert_eq!(theme.colors.surface.chrome, gpui::rgba(0xe6d8c9ff));
    assert_eq!(theme.colors.accent.foreground, gpui::rgba(0x854718ff));
    assert_eq!(theme.colors.diff.added.foreground, gpui::rgba(0x2f682bff));
    assert_eq!(theme.syntax.keyword, gpui::rgba(0x22586aff));
    assert_eq!(theme.syntax.markup_heading, gpui::rgba(0x26586aff));
    assert_eq!(theme.syntax.diff_plus, gpui::rgba(0x225d2bff));
    assert_eq!(theme.syntax.variable, Some(gpui::rgba(0x211a14ff)));
    assert_eq!(theme_label("sunset_veil"), Some("Sunset Veil".to_string()));
}

#[test]
fn bundled_themes_keep_the_canvas_and_chrome_hierarchy_for_their_appearance() {
    assert_eq!(
        AppTheme::gitcomet_light().colors.surface.canvas,
        gpui::rgba(0xffffffff),
        "GitComet Light should keep its pure-white canvas"
    );
    assert_eq!(
        AppTheme::from_key("sunset_veil")
            .expect("Sunset Veil theme should load")
            .colors
            .surface
            .canvas,
        gpui::rgba(0xfff7edff),
        "Sunset Veil should use a warm light-orange canvas"
    );

    for (key, _) in bundled_theme_keys().into_iter().filter(|(_, dark)| !dark) {
        let key = key.as_str();
        let theme = AppTheme::from_key(key).expect("light theme should load");
        let colors = theme.colors;

        assert!(
            relative_luminance(colors.surface.chrome) < relative_luminance(colors.surface.panel),
            "{key}: surrounding chrome should be darker than panel surfaces"
        );
        assert!(
            relative_luminance(colors.surface.panel) < relative_luminance(colors.surface.raised),
            "{key}: elevated surfaces should remain distinguishable"
        );
        assert!(
            relative_luminance(colors.surface.raised) < relative_luminance(colors.surface.canvas),
            "{key}: the main canvas should remain the brightest area"
        );
    }

    for (key, _) in bundled_theme_keys().into_iter().filter(|(_, dark)| *dark) {
        let key = key.as_str();
        let theme = AppTheme::from_key(key).expect("dark theme should load");
        assert!(
            relative_luminance(theme.colors.surface.canvas)
                < relative_luminance(theme.colors.surface.chrome),
            "{key}: surrounding chrome should remain lighter than the dark canvas"
        );
    }
}

/// The house dark palettes share one canvas; ported themes keep their own.
#[test]
fn bundled_dark_themes_share_the_darker_canvas_and_compact_radii() {
    for key in [AMBER_DARK_THEME_KEY, "gitcomet_dark", "tokyo_night"] {
        let theme = AppTheme::from_key(key).expect("dark theme should load");

        assert_eq!(theme.colors.surface.canvas, gpui::rgba(0x0d0f13ff), "{key}");
        assert_eq!(
            theme.colors.editor.background,
            gpui::rgba(0x0d0f13ff),
            "{key}"
        );
        assert_eq!(
            theme.colors.editor.gutter_background,
            gpui::rgba(0x0d0f13ff),
            "{key}"
        );
        assert_eq!(theme.radii.panel, 8.0, "{key}");
        assert_eq!(theme.radii.row, 4.0, "{key}");
        assert_eq!(theme.radii.control, 4.0, "{key}");
        assert_eq!(theme.radii.popover, 8.0, "{key}");
        assert_eq!(theme.radii.window, 8.0, "{key}");
    }
}

#[test]
fn bundled_themes_define_their_notice_colors() {
    let dark = AppTheme::gitcomet_dark();
    let light = AppTheme::gitcomet_light();

    assert_eq!(
        dark.colors.notice.background,
        with_alpha(gpui::rgba(0xf2a53aff), 0.13)
    );
    assert_eq!(
        dark.colors.notice.border,
        with_alpha(gpui::rgba(0xf2a53aff), 0.30)
    );
    assert_eq!(dark.colors.notice.foreground, gpui::rgba(0xeff1f5ff));
    assert_eq!(dark.colors.notice.secondary, gpui::rgba(0x9ea5b4ff));
    assert_eq!(light.colors.notice.background, gpui::rgba(0xf8fafcff));
    assert_eq!(light.colors.notice.border, gpui::rgba(0x96701eff));
    assert_eq!(light.colors.notice.foreground, gpui::rgba(0x111827ff));
    assert_eq!(light.colors.notice.secondary, gpui::rgba(0x465166ff));

    for (key, hue) in [
        ("tokyo_night", 0xe0af68ff),
        (AMBER_DARK_THEME_KEY, 0xe3a64bff),
    ] {
        let theme = AppTheme::from_key(key).expect("bundled theme should load");
        assert_eq!(
            theme.colors.notice.background,
            with_alpha(gpui::rgba(hue), 0.13),
            "{key}"
        );
        assert_eq!(
            theme.colors.notice.border,
            with_alpha(gpui::rgba(hue), 0.30),
            "{key}"
        );
    }
    let sunset = AppTheme::from_key("sunset_veil").expect("Sunset Veil should load");
    assert_eq!(sunset.colors.notice.background, gpui::rgba(0xfcf3e8ff));
    assert_eq!(sunset.colors.notice.border, gpui::rgba(0x956f24ff));
}

/// The notice sits on the pane's content background; its tint is a wash
/// over that, so text is measured against the two composited.
#[test]
fn bundled_theme_notice_text_is_readable_on_its_background() {
    for (key, _) in bundled_theme_keys() {
        let key = key.as_str();
        let theme = AppTheme::from_key(key).expect("bundled theme should load");
        let notice = theme.colors.notice;
        let background = composite_over(content_header_bg(theme), notice.background);
        println!(
            "{key}: title {:.2}, secondary {:.2}",
            contrast_ratio(notice.foreground, background),
            contrast_ratio(notice.secondary, background)
        );
        assert_min_contrast(key, "notice.foreground", notice.foreground, background, 7.0);
        assert_min_contrast(key, "notice.secondary", notice.secondary, background, 4.5);
    }
}

/// A primary call-to-action's label stays readable in every state.
#[test]
fn bundled_theme_call_to_action_text_is_readable() {
    for key in [
        DEFAULT_DARK_THEME_KEY,
        DEFAULT_LIGHT_THEME_KEY,
        "tokyo_night",
        AMBER_DARK_THEME_KEY,
        "sunset_veil",
    ] {
        let theme = AppTheme::from_key(key).expect("bundled theme should load");
        let primary = theme.colors.interstitial.primary;
        for (state, background) in [
            ("background", primary.background),
            ("background_hover", primary.background_hover),
            ("background_active", primary.background_active),
        ] {
            assert_min_contrast(
                key,
                &format!("interstitial.primary.text on {state}"),
                primary.text,
                background,
                4.5,
            );
        }
    }
}

#[test]
fn custom_themes_use_their_own_notice_colors_and_inherit_a_missing_group() {
    use serde_json::json;

    let mut fixture = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
    let theme = &mut fixture["themes"][0];
    theme["key"] = json!("notice_fixture");
    theme["name"] = json!("Notice Fixture");
    theme["colors"]["notice"] = json!({
        "background": { "hex": "#336699ff", "alpha": 0.20 },
        "border": "#336699ff",
        "foreground": "#ffffffff",
        "secondary": "#ccddeeff"
    });
    let custom = AppTheme::from_json_str(
        &serde_json::to_string(&fixture).expect("fixture should serialize"),
    )
    .expect("a theme with notice colors should load");
    assert_eq!(
        custom.colors.notice.background,
        with_alpha(gpui::rgba(0x336699ff), 0.20)
    );
    assert_eq!(custom.colors.notice.border, gpui::rgba(0x336699ff));
    assert_eq!(custom.colors.notice.foreground, gpui::rgba(0xffffffff));
    assert_eq!(custom.colors.notice.secondary, gpui::rgba(0xccddeeff));

    // A theme written before the group existed still loads.
    fixture["themes"][0]["colors"]
        .as_object_mut()
        .expect("colors should be an object")
        .remove("notice");
    let older = AppTheme::from_json_str(
        &serde_json::to_string(&fixture).expect("fixture should serialize"),
    )
    .expect("a theme without notice colors should still load");
    assert_eq!(older.colors.notice, AppTheme::gitcomet_dark().colors.notice);
}

#[test]
fn bundled_themes_share_the_dark_theme_radii() {
    let dark_radii = AppTheme::gitcomet_dark().radii;

    for (key, is_dark) in bundled_theme_keys() {
        let theme = AppTheme::from_key(&key).expect("bundled theme should load");

        assert_eq!(theme.is_dark, is_dark, "{key}");
        assert_eq!(theme.radii, dark_radii, "{key}");
    }
}

#[test]
fn amber_dark_semantic_foregrounds_have_strong_canvas_contrast() {
    let theme = AppTheme::from_key(AMBER_DARK_THEME_KEY).expect("Amber Dark theme should load");
    let colors = theme.colors;
    let canvas = colors.surface.canvas;

    for (token, color, minimum) in [
        ("primary", colors.foreground.primary, 7.0),
        ("secondary", colors.foreground.secondary, 4.5),
        ("accent", colors.accent.foreground, 4.5),
        ("info", colors.status.info.foreground, 4.5),
        ("danger", colors.status.danger.foreground, 4.5),
        ("warning", colors.status.warning.foreground, 4.5),
        ("success", colors.status.success.foreground, 4.5),
        ("diff.added", colors.diff.added.foreground, 4.5),
        ("diff.removed", colors.diff.removed.foreground, 4.5),
    ] {
        assert_min_contrast(AMBER_DARK_THEME_KEY, token, color, canvas, minimum);
    }

    assert_min_contrast(
        AMBER_DARK_THEME_KEY,
        "accent.on_solid",
        colors.accent.on_solid,
        colors.accent.solid,
        4.5,
    );

    assert_min_contrast(
        AMBER_DARK_THEME_KEY,
        "status.danger on surface.raised",
        colors.status.danger.foreground,
        colors.surface.raised,
        4.5,
    );

    for (token, background) in [
        ("diff.removed", colors.diff.removed.background),
        (
            "diff.removed.focused",
            colors.diff.removed.focused_background,
        ),
        ("diff.removed.word", colors.diff.removed.word_background),
    ] {
        assert_min_contrast(
            AMBER_DARK_THEME_KEY,
            token,
            colors.diff.removed.foreground,
            composite_over(colors.editor.background, background),
            4.5,
        );
    }
}

/// One readability requirement: the theme token `token` drawn over `background`.
struct ReadabilityCheck {
    name: String,
    /// JSON path of the foreground, e.g. `colors.foreground.secondary`.
    token: String,
    foreground: Rgba,
    background: Rgba,
    minimum: f32,
}

impl ReadabilityCheck {
    /// Translucent text blends into what it is drawn on.
    fn ratio(&self) -> f32 {
        contrast_ratio(
            composite_over(self.background, self.foreground),
            self.background,
        )
    }
}

/// WCAG AA: 4.5:1 for text; 3:1 for focus/selection indicators, graph lanes and
/// syntax on a diff, selection or search wash. Washes are composited over what they tint.
/// Control borders (`stroke.control`) are deliberately not held to 3:1.
fn readability_checks(theme: AppTheme) -> Vec<ReadabilityCheck> {
    const TEXT: f32 = 4.5;
    const NON_TEXT: f32 = 3.0;
    let colors = theme.colors;
    let canvas = colors.surface.canvas;
    let editor = colors.editor.background;
    let surfaces = [
        ("canvas", canvas),
        ("chrome", colors.surface.chrome),
        ("panel", colors.surface.panel),
        ("raised", colors.surface.raised),
        ("input", colors.surface.input),
    ];
    // Where selectable, hoverable rows sit.
    let row_surfaces = &surfaces[..3];
    let mut checks = Vec::new();
    let mut check = |name: String, token: &str, foreground, background, minimum| {
        checks.push(ReadabilityCheck {
            name,
            token: token.to_string(),
            foreground,
            background,
            minimum,
        });
    };

    for (surface_name, surface) in surfaces {
        for (name, token, foreground) in [
            (
                "primary",
                "colors.foreground.primary",
                colors.foreground.primary,
            ),
            (
                "secondary",
                "colors.foreground.secondary",
                colors.foreground.secondary,
            ),
            (
                "accent.foreground",
                "colors.accent.foreground",
                colors.accent.foreground,
            ),
        ] {
            check(
                format!("{name}/{surface_name}"),
                token,
                foreground,
                surface,
                TEXT,
            );
        }
        check(
            format!("focus_ring/{surface_name}"),
            "colors.interaction.focus_ring",
            colors.interaction.focus_ring,
            surface,
            NON_TEXT,
        );
    }
    for (surface_name, surface) in row_surfaces.iter().copied() {
        let selected = composite_over(surface, colors.interaction.selected_background);
        let hovered = composite_over(surface, colors.interaction.hover_background);
        for (name, token, foreground, background) in [
            (
                "selected_foreground/selected",
                "colors.interaction.selected_foreground",
                colors.interaction.selected_foreground,
                selected,
            ),
            (
                "secondary/selected",
                "colors.foreground.secondary",
                colors.foreground.secondary,
                selected,
            ),
            (
                "primary/hover",
                "colors.foreground.primary",
                colors.foreground.primary,
                hovered,
            ),
            (
                "secondary/hover",
                "colors.foreground.secondary",
                colors.foreground.secondary,
                hovered,
            ),
        ] {
            check(
                format!("{name}@{surface_name}"),
                token,
                foreground,
                background,
                TEXT,
            );
        }
        check(
            format!("selected_indicator/{surface_name}"),
            "colors.interaction.selected_indicator",
            colors.interaction.selected_indicator,
            surface,
            NON_TEXT,
        );
    }
    for (name, token, foreground, background) in [
        (
            "emphasis/canvas",
            "colors.foreground.emphasis",
            colors.foreground.emphasis,
            canvas,
        ),
        (
            "placeholder/input",
            "colors.foreground.placeholder",
            colors.foreground.placeholder,
            colors.surface.input,
        ),
        (
            "accent.foreground/subtle",
            "colors.accent.foreground",
            colors.accent.foreground,
            composite_over(canvas, colors.accent.subtle_background),
        ),
        (
            "accent.on_solid",
            "colors.accent.on_solid",
            colors.accent.on_solid,
            colors.accent.solid,
        ),
        (
            "tooltip",
            "colors.tooltip.foreground",
            colors.tooltip.foreground,
            composite_over(canvas, colors.tooltip.background),
        ),
        (
            "notice.foreground",
            "colors.notice.foreground",
            colors.notice.foreground,
            composite_over(canvas, colors.notice.background),
        ),
        (
            "notice.secondary",
            "colors.notice.secondary",
            colors.notice.secondary,
            composite_over(canvas, colors.notice.background),
        ),
        (
            "editor.foreground",
            "colors.editor.foreground",
            colors.editor.foreground,
            editor,
        ),
        (
            "editor.foreground/selection",
            "colors.editor.foreground",
            colors.editor.foreground,
            composite_over(editor, colors.editor.selection_background),
        ),
        // Diff header rows draw their text in the line-number colour too.
        (
            "line_number/editor",
            "colors.editor.line_number",
            colors.editor.line_number,
            editor,
        ),
        (
            "line_number/gutter",
            "colors.editor.line_number",
            colors.editor.line_number,
            composite_over(editor, colors.editor.gutter_background),
        ),
    ] {
        check(name.into(), token, foreground, background, TEXT);
    }
    let search_match = composite_over(editor, colors.editor.search_match_background);
    if !theme.is_dark {
        // Dark themes keep the syntax colours on a match (checked below).
        check(
            "search_match".into(),
            "colors.editor.search_match_foreground",
            colors.editor.search_match_foreground,
            search_match,
            TEXT,
        );
    }
    for (name, set) in [
        ("info", colors.status.info),
        ("success", colors.status.success),
        ("warning", colors.status.warning),
        ("danger", colors.status.danger),
    ] {
        let token = format!("colors.status.{name}.foreground");
        for (background_name, background) in [
            ("wash", composite_over(canvas, set.background)),
            ("chrome", colors.surface.chrome),
            ("raised", colors.surface.raised),
        ] {
            check(
                format!("status.{name}/{background_name}"),
                &token,
                set.foreground,
                background,
                TEXT,
            );
        }
    }
    for (name, set) in [
        ("added", colors.diff.added),
        ("removed", colors.diff.removed),
        ("modified", colors.diff.modified),
    ] {
        let token = format!("colors.diff.{name}.foreground");
        check(
            format!("diff.{name}"),
            &token,
            set.foreground,
            composite_over(editor, set.background),
            TEXT,
        );
        check(
            format!("diff.{name}.word"),
            &token,
            set.foreground,
            composite_over(editor, set.word_background),
            TEXT,
        );
    }
    for (name, color) in syntax_foregrounds(theme) {
        let token = format!("syntax.{name}");
        check(format!("{token}/editor"), &token, color, editor, TEXT);
        // Text selection and the current search match share one wash.
        let mut washes = vec![
            (
                "diff.added",
                composite_over(editor, colors.diff.added.background),
            ),
            (
                "diff.removed",
                composite_over(editor, colors.diff.removed.background),
            ),
            (
                "selection",
                composite_over(editor, colors.editor.selection_background),
            ),
        ];
        if theme.is_dark {
            washes.push(("search_match", search_match));
        }
        for (wash, background) in washes {
            check(
                format!("{token}/{wash}"),
                &token,
                color,
                background,
                NON_TEXT,
            );
        }
    }
    for (index, color) in theme.graph_lane_palette.as_slice().iter().enumerate() {
        check(
            format!("graph_lane_palette[{index}]"),
            "colors.graph_lane_palette",
            *color,
            canvas,
            NON_TEXT,
        );
    }
    checks
}

#[test]
fn every_bundled_theme_meets_the_readability_floor() {
    let mut failures = Vec::new();
    for (key, _) in bundled_theme_keys() {
        let theme = AppTheme::from_key(&key).expect("bundled theme should load");
        for check in readability_checks(theme) {
            let actual = check.ratio();
            if actual < check.minimum {
                failures.push(format!(
                    "{key} {} ({}): {actual:.2} < {:.2}",
                    check.name, check.token, check.minimum
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn bundled_light_theme_foregrounds_have_strong_canvas_contrast() {
    for key in ["gitcomet_light", "sunset_veil"] {
        let theme = AppTheme::from_key(key).expect("light theme should load");
        let colors = theme.colors;
        let canvas = colors.surface.canvas;

        for (token, color, minimum) in [
            ("primary", colors.foreground.primary, 7.0),
            ("secondary", colors.foreground.secondary, 4.5),
            ("accent", colors.accent.foreground, 4.5),
            ("danger", colors.status.danger.foreground, 4.5),
            ("warning", colors.status.warning.foreground, 4.5),
            ("success", colors.status.success.foreground, 4.5),
            ("diff.added", colors.diff.added.foreground, 4.5),
            ("diff.removed", colors.diff.removed.foreground, 4.5),
        ] {
            assert_min_contrast(key, token, color, canvas, minimum);
        }

        for (surface_name, surface) in [
            ("canvas", colors.surface.canvas),
            ("chrome", colors.surface.chrome),
            ("panel", colors.surface.panel),
            ("raised", colors.surface.raised),
            ("input", colors.surface.input),
        ] {
            assert_min_contrast(
                key,
                &format!("primary/{surface_name}"),
                colors.foreground.primary,
                surface,
                7.0,
            );
            assert_min_contrast(
                key,
                &format!("secondary/{surface_name}"),
                colors.foreground.secondary,
                surface,
                4.5,
            );
        }

        assert_min_contrast(
            key,
            "accent.on_solid",
            colors.accent.on_solid,
            colors.accent.solid,
            4.5,
        );
        for (name, set) in [
            ("status.info", colors.status.info),
            ("status.success", colors.status.success),
            ("status.warning", colors.status.warning),
            ("status.danger", colors.status.danger),
        ] {
            assert_min_contrast(key, name, set.foreground, set.background, 4.5);
            assert_min_contrast(
                key,
                &format!("{name}.border"),
                set.border,
                set.background,
                3.0,
            );
        }
        for (name, set) in [
            ("diff.added", colors.diff.added),
            ("diff.removed", colors.diff.removed),
            ("diff.modified", colors.diff.modified),
        ] {
            assert_min_contrast(key, name, set.foreground, set.background, 4.5);
            assert_min_contrast(
                key,
                &format!("{name}.word"),
                set.foreground,
                set.word_background,
                4.5,
            );
        }
        assert_min_contrast(
            key,
            "stroke.control",
            colors.stroke.control,
            colors.surface.input,
            3.0,
        );
        assert_min_contrast(
            key,
            "focus_ring",
            colors.interaction.focus_ring,
            colors.surface.input,
            3.0,
        );
        assert_min_contrast(
            key,
            "selected_indicator",
            colors.interaction.selected_indicator,
            colors.interaction.selected_background,
            3.0,
        );

        for (token, color) in syntax_foregrounds(theme) {
            assert_min_contrast(
                key,
                &format!("syntax.{token}/editor"),
                color,
                colors.editor.background,
                7.0,
            );

            for (surface_name, surface) in [
                ("editor.selection", colors.editor.selection_background),
                ("editor.search_match", colors.editor.search_match_background),
                (
                    "editor.bracket_match",
                    colors.editor.bracket_match_background,
                ),
                (
                    "editor.occurrence_highlight",
                    colors.editor.occurrence_highlight_background,
                ),
            ] {
                assert_min_contrast(
                    key,
                    &format!("syntax.{token}/{surface_name}"),
                    color,
                    surface,
                    5.5,
                );
            }

            for (surface_name, surface) in [
                ("diff.added", colors.diff.added.background),
                ("diff.added.word", colors.diff.added.word_background),
                ("diff.removed", colors.diff.removed.background),
                ("diff.removed.word", colors.diff.removed.word_background),
                ("diff.modified", colors.diff.modified.background),
                ("diff.modified.word", colors.diff.modified.word_background),
            ] {
                assert_min_contrast(
                    key,
                    &format!("syntax.{token}/{surface_name}"),
                    color,
                    surface,
                    6.0,
                );
            }
        }

        for (index, color) in theme.graph_lane_palette.as_slice().iter().enumerate() {
            assert_min_contrast(
                key,
                &format!("graph_lane_palette[{index}]"),
                *color,
                canvas,
                3.0,
            );
        }
    }
}

#[test]
fn content_header_bg_matches_the_canvas_on_dark_and_is_distinct_on_light() {
    for (key, is_dark) in bundled_theme_keys() {
        let theme = AppTheme::from_key(&key).expect("bundled theme should load");
        if is_dark {
            assert_eq!(
                content_header_bg(theme),
                theme.colors.surface.canvas,
                "{key}: header band should be the canvas color"
            );
        } else {
            assert_eq!(
                content_header_bg(theme),
                theme.colors.surface.raised,
                "{key}: header band should stay raised"
            );
        }
    }
}

#[test]
fn bundled_theme_assets_explicitly_define_new_syntax_keys() {
    const REQUIRED_KEYS: &[&str] = &[
        "\"string_regex\"",
        "\"string_special\"",
        "\"preproc\"",
        "\"constructor\"",
        "\"namespace\"",
        "\"variable_builtin\"",
        "\"label\"",
        "\"constant_builtin\"",
        "\"punctuation_special\"",
        "\"punctuation_list_marker\"",
        "\"markup_heading\"",
        "\"markup_link\"",
        "\"text_literal\"",
        "\"diff_plus\"",
        "\"diff_minus\"",
        "\"diff_delta\"",
    ];

    for file in EMBEDDED_THEME_FILES {
        for key in REQUIRED_KEYS {
            assert!(
                file.json.contains(key),
                "embedded theme file {} should explicitly define {}",
                file.stem,
                key
            );
        }
    }
}

/// Dotted paths of every token `filled` gained over `original`.
fn collect_filled_token_paths(
    original: &serde_json::Value,
    filled: &serde_json::Value,
    path: &mut String,
    out: &mut Vec<String>,
) {
    let Some(filled) = filled.as_object() else {
        return;
    };
    for (key, filled_value) in filled {
        let original_value = original.get(key);
        let len = path.len();
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(key);
        match original_value {
            None => out.push(path.clone()),
            Some(original_value) => {
                collect_filled_token_paths(original_value, filled_value, path, out)
            }
        }
        path.truncate(len);
    }
}

/// The colour-token counterpart of the syntax-key guard above.
///
/// Before `fill_missing_color_tokens` existed, a bundled theme missing a
/// colour token was a parse error that `embedded_theme_cache` panicked on, so
/// an incomplete one could not ship. The fill exists for *custom* themes
/// written against an older token set; applied to the bundled themes it
/// quietly hands them colours tuned against a different palette. Asserting
/// the fill is a no-op restores the startup guarantee without exempting them
/// from it.
#[test]
fn bundled_themes_explicitly_define_every_color_token() {
    for file in EMBEDDED_THEME_FILES {
        let bundle: serde_json::Value = serde_json::from_str(file.json)
            .unwrap_or_else(|err| panic!("embedded theme file {} is not JSON: {err}", file.stem));
        let themes = bundle["themes"]
            .as_array()
            .unwrap_or_else(|| panic!("embedded theme file {} has no themes", file.stem));

        for theme in themes {
            let key = theme["key"].as_str().unwrap_or("<unnamed>").to_string();
            let before = theme["colors"].clone();
            let mut filled = serde_json::json!({ "themes": [theme.clone()] });
            fill_missing_color_tokens(&mut filled);

            // Reported as paths rather than by diffing the two objects: the
            // palettes are large enough that an `assert_eq!` dump buries the
            // one token that is actually missing.
            let mut missing = Vec::new();
            collect_filled_token_paths(
                &before,
                &filled["themes"][0]["colors"],
                &mut String::new(),
                &mut missing,
            );

            assert!(
                missing.is_empty(),
                "bundled theme {key} in {} inherits {} from another theme; define them \
                     explicitly",
                file.stem,
                missing.join(", ")
            );
        }
    }
}

#[test]
fn bundled_theme_file_exposes_multiple_themes() {
    use serde_json::json;

    let mut light = test_theme_entry(DEFAULT_LIGHT_THEME_KEY);
    light["key"] = json!("classic_light");
    light["name"] = json!("Classic Light");

    let mut dark = test_theme_entry(DEFAULT_DARK_THEME_KEY);
    dark["key"] = json!("classic_dark");
    dark["name"] = json!("Classic Dark");

    let json = test_theme_bundle_json("Classic", vec![light, dark]);
    let specs = load_theme_specs_from_json(&json).expect("bundle should parse");

    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].option.key, "classic_light");
    assert_eq!(specs[0].option.label, "Classic Light");
    assert!(!specs[0].theme.is_dark);
    assert_eq!(specs[1].option.key, "classic_dark");
    assert_eq!(specs[1].option.label, "Classic Dark");
    assert!(specs[1].theme.is_dark);
}
#[test]
fn every_available_theme_has_preview_colors_from_its_chrome_accent_and_keyword() {
    let options = available_themes();
    assert!(options.len() >= bundled_theme_keys().len());
    for option in options {
        let theme = AppTheme::from_key(&option.key).expect("listed theme should load");
        let preview = theme_preview_colors(&option.key).expect("listed theme has a preview");
        assert_eq!(preview.is_dark, option.is_dark, "{}", option.key);
        assert_eq!(preview.base, theme.colors.surface.chrome, "{}", option.key);
        assert_eq!(preview.glow, theme.colors.accent.solid, "{}", option.key);
        assert_eq!(preview.secondary, theme.syntax.keyword, "{}", option.key);
        assert!(!option.custom, "{} is bundled", option.key);
    }
    assert_eq!(theme_preview_colors("no_such_theme"), None);
}

#[test]
fn embedded_theme_registry_exposes_default_keys() {
    let themes = available_themes();

    assert!(!themes.is_empty());
    assert!(has_theme_key(DEFAULT_DARK_THEME_KEY));
    assert!(has_theme_key(DEFAULT_LIGHT_THEME_KEY));
    assert!(has_theme_key(AMBER_DARK_THEME_KEY));
    assert_eq!(
        theme_label(DEFAULT_DARK_THEME_KEY),
        Some("GitComet Dark".to_string())
    );
    assert_eq!(
        theme_label(DEFAULT_LIGHT_THEME_KEY),
        Some("GitComet Light".to_string())
    );
    assert_eq!(
        theme_label(AMBER_DARK_THEME_KEY),
        Some("Amber Dark".to_string())
    );
    let ordered_keys = themes
        .iter()
        .map(|theme| theme.key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ordered_keys.get(..3),
        Some(
            [
                DEFAULT_DARK_THEME_KEY,
                DEFAULT_LIGHT_THEME_KEY,
                AMBER_DARK_THEME_KEY,
            ]
            .as_slice()
        )
    );
}

#[test]
fn ensure_runtime_theme_dir_creates_missing_directory() {
    let dir = tempdir().expect("temp dir should exist");
    let path = dir.path().join("themes");

    assert!(!path.exists(), "theme subdirectory should start absent");

    let resolved = resolved_runtime_themes_dir(Some(&path))
        .expect("runtime theme helper should resolve a writable directory");

    assert_eq!(resolved, path);
    assert!(resolved.is_dir(), "theme directory should be created");
}

#[test]
fn runtime_theme_dir_extends_embedded_themes_with_custom_entries() {
    use serde_json::json;

    let dir = tempdir().expect("temp dir should exist");
    let mut custom = test_theme_entry(DEFAULT_DARK_THEME_KEY);
    custom["key"] = json!("custom_theme");
    custom["name"] = json!("Custom Theme");
    fs::write(
        dir.path().join("custom_theme.json"),
        test_theme_bundle_json("Custom Theme", vec![custom]),
    )
    .expect("custom theme file should be written");

    let themes = merged_theme_options(Some(dir.path()));
    let custom = themes
        .iter()
        .find(|theme| theme.key == "custom_theme")
        .expect("custom theme should be discovered");

    assert_eq!(custom.label, "Custom Theme");
    assert!(
        themes
            .iter()
            .any(|theme| theme.key == DEFAULT_DARK_THEME_KEY)
    );
}
/// The settings theme list asks for the runtime themes from inside a
/// `uniform_list` processor, so an unmemoized load re-reads and re-parses
/// every theme file per frame. The cache has to hold across calls -- and
/// still notice an edit, because theme authors expect a save to show up
/// without restarting.
#[test]
fn runtime_themes_are_reused_until_the_directory_changes() {
    use serde_json::json;

    let dir = tempdir().expect("temp dir should exist");
    let write_theme = |label: &str| {
        let mut custom = test_theme_entry(DEFAULT_DARK_THEME_KEY);
        custom["key"] = json!("custom_theme");
        custom["name"] = json!(label);
        fs::write(
            dir.path().join("custom_theme.json"),
            test_theme_bundle_json(label, vec![custom]),
        )
        .expect("custom theme file should be written");
    };

    write_theme("First");
    let first = runtime_themes_with_dir(Some(dir.path()));
    let again = runtime_themes_with_dir(Some(dir.path()));
    assert!(
        std::sync::Arc::ptr_eq(&first, &again),
        "an unchanged theme directory must not be re-read and re-parsed"
    );
    assert_eq!(first["custom_theme"].option.label, "First");

    // Rewriting the file changes its size, so the stat signature moves even
    // where the filesystem's mtime resolution is coarse.
    write_theme("Second edit");
    let reloaded = runtime_themes_with_dir(Some(dir.path()));
    assert!(
        !std::sync::Arc::ptr_eq(&first, &reloaded),
        "an edited theme file must invalidate the cache"
    );
    assert_eq!(reloaded["custom_theme"].option.label, "Second edit");
}

#[test]
fn runtime_theme_dir_ignores_reserved_system_theme_filenames() {
    let dir = tempdir().expect("temp dir should exist");
    fs::write(dir.path().join("gitcomet.json"), "not parsed")
        .expect("reserved theme file should be written");

    let themes = merged_theme_options(Some(dir.path()));

    assert_eq!(
        themes,
        available_themes(),
        "custom themes in reserved bundled filenames should be ignored"
    );
}
#[test]
fn runtime_theme_dir_ignores_every_reserved_system_theme_filename() {
    let dir = tempdir().expect("temp dir should exist");

    for file in EMBEDDED_THEME_FILES {
        fs::write(dir.path().join(format!("{}.json", file.stem)), "not parsed")
            .expect("reserved theme file should be written");
    }

    assert!(
        runtime_themes_with_dir(Some(dir.path())).is_empty(),
        "runtime themes should ignore every reserved bundled filename"
    );
    assert_eq!(
        merged_theme_options(Some(dir.path())),
        available_themes(),
        "reserved files should not change the available theme list"
    );
}
#[test]
fn runtime_theme_dir_ignores_embedded_theme_key_collisions_but_keeps_custom_entries() {
    use serde_json::json;

    let dir = tempdir().expect("temp dir should exist");
    let mut collision = test_theme_entry(DEFAULT_DARK_THEME_KEY);
    collision["name"] = json!("Fake GitComet Dark");

    let mut custom = test_theme_entry(DEFAULT_LIGHT_THEME_KEY);
    custom["key"] = json!("custom_keep");
    custom["name"] = json!("Custom Keep");

    fs::write(
        dir.path().join("mixed_theme.json"),
        test_theme_bundle_json("Mixed Theme", vec![collision, custom]),
    )
    .expect("mixed theme file should be written");

    let runtime_themes = runtime_themes_with_dir(Some(dir.path()));
    assert!(
        !runtime_themes.contains_key(DEFAULT_DARK_THEME_KEY),
        "runtime themes should ignore entries that reuse embedded system keys"
    );
    assert!(
        runtime_themes.contains_key("custom_keep"),
        "runtime themes should keep valid custom entries from mixed bundles"
    );

    let themes = merged_theme_options(Some(dir.path()));
    assert_eq!(
        themes
            .iter()
            .find(|theme| theme.key == DEFAULT_DARK_THEME_KEY)
            .map(|theme| theme.label.as_str()),
        Some("GitComet Dark"),
        "embedded theme labels should remain authoritative"
    );
    assert_eq!(
        themes
            .iter()
            .filter(|theme| theme.key == DEFAULT_DARK_THEME_KEY)
            .count(),
        1,
        "embedded system keys should appear only once in the merged theme list"
    );
    assert!(
        themes.iter().any(|theme| theme.key == "custom_keep"),
        "valid custom themes should still appear in available theme options"
    );
}

/// Bundling a theme reserves its file name and keys, which a user's
/// existing custom theme may already use; the Appearance page must say so.
#[test]
fn runtime_theme_issues_explain_reserved_filenames_and_bundled_key_clashes() {
    use serde_json::json;

    let dir = tempdir().expect("temp dir should exist");
    let mut own = test_theme_entry(DEFAULT_DARK_THEME_KEY);
    own["key"] = json!("my_nord");
    fs::write(
        dir.path().join("nord.json"),
        test_theme_bundle_json("My Nord", vec![own]),
    )
    .expect("reserved theme file should be written");
    let mut clash = test_theme_entry(DEFAULT_DARK_THEME_KEY);
    clash["key"] = json!("monokai");
    let mut keep = test_theme_entry(DEFAULT_DARK_THEME_KEY);
    keep["key"] = json!("custom_keep");
    fs::write(
        dir.path().join("mine.json"),
        test_theme_bundle_json("Mine", vec![clash, keep]),
    )
    .expect("clashing theme file should be written");

    let issues = runtime_theme_issues_with_dir(Some(dir.path()));
    let issue_for = |name: &str| {
        issues
            .iter()
            .find(|issue| issue.path.file_name() == Some(std::ffi::OsStr::new(name)))
            .unwrap_or_else(|| panic!("{name} should be reported: {issues:?}"))
    };
    assert!(
        issue_for("nord.json").message.contains("bundled theme"),
        "{issues:?}"
    );
    assert!(
        issue_for("mine.json").message.contains("`monokai`"),
        "{issues:?}"
    );
    assert_eq!(issues.len(), 2, "{issues:?}");
    assert!(runtime_themes_with_dir(Some(dir.path())).contains_key("custom_keep"));
}

#[test]
fn themes_markdown_example_matches_current_theme_parser() {
    let example = themes_markdown_example();
    let json = strip_json_line_comments(&example);
    let themes = load_theme_specs_from_json(&json)
        .expect("THEMES.md example should stay in sync with the runtime parser");

    assert_eq!(themes.len(), 1, "docs example should define a single theme");
    assert_eq!(themes[0].option.key, "my_theme_dark");
}

#[test]
fn themes_markdown_lists_current_supported_syntax_keys() {
    const REQUIRED_DOC_KEYS: &[&str] = &[
        "comment",
        "comment_doc",
        "string",
        "string_escape",
        "string_regex",
        "string_special",
        "keyword",
        "keyword_control",
        "preproc",
        "number",
        "boolean",
        "function",
        "function_method",
        "function_special",
        "constructor",
        "type",
        "type_builtin",
        "type_interface",
        "namespace",
        "variable",
        "variable_parameter",
        "variable_special",
        "variable_builtin",
        "property",
        "label",
        "constant",
        "constant_builtin",
        "operator",
        "punctuation",
        "punctuation_bracket",
        "punctuation_delimiter",
        "punctuation_special",
        "punctuation_list_marker",
        "tag",
        "attribute",
        "markup_heading",
        "markup_link",
        "text_literal",
        "diff_plus",
        "diff_minus",
        "diff_delta",
        "lifetime",
    ];

    let markdown = fs::read_to_string(themes_markdown_path())
        .expect("THEMES.md should be readable for supported-key checks");

    for key in REQUIRED_DOC_KEYS {
        assert!(
            markdown.contains(&format!("`{key}`")),
            "THEMES.md should mention the supported syntax key `{key}`"
        );
    }
}

/// Matches whole names in the table's Dark and Light cells: a substring
/// check passes "Tokyo Night" on "Tokyo Night Storm" or the Source link.
#[test]
fn themes_markdown_lists_every_bundled_theme() {
    let markdown = fs::read_to_string(themes_markdown_path())
        .expect("THEMES.md should be readable for the built-in theme list");
    let mut listed = [Vec::new(), Vec::new()];
    let rows = markdown
        .lines()
        .skip_while(|line| !line.starts_with("| Dark | Light |"))
        .skip(2)
        .take_while(|line| line.starts_with('|'));
    for row in rows {
        for (column, cell) in row.split('|').skip(1).take(2).enumerate() {
            listed[column].extend(
                cell.split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty()),
            );
        }
    }
    for (key, is_dark) in bundled_theme_keys() {
        let label = theme_label(&key).expect("bundled theme has a label");
        let (column, heading) = if is_dark { (0, "Dark") } else { (1, "Light") };
        assert!(
            listed[column].contains(&label.as_str()),
            "THEMES.md should list the bundled theme `{label}` under {heading}"
        );
    }
}

#[test]
fn themes_markdown_documents_custom_theme_override_rules() {
    let markdown = fs::read_to_string(themes_markdown_path())
        .expect("THEMES.md should be readable for override behavior checks");

    for snippet in [
        "GitComet creates the user themes directory on startup",
        "ignores files whose basename matches a bundled system theme file",
        "cannot override built-in system theme keys",
    ] {
        assert!(
            markdown.contains(snippet),
            "THEMES.md should document `{snippet}`"
        );
    }
}

#[test]
fn readme_themes_section_points_to_theme_guide() {
    let readme =
        fs::read_to_string(readme_path()).expect("README.md should be readable for docs tests");

    for snippet in [
        "Custom themes are loaded from JSON bundle files in your per-user themes directory",
        "creates on startup",
        "[THEMES.md](docs/themes.md)",
    ] {
        assert!(
            readme.contains(snippet),
            "README.md theme section should mention `{snippet}`"
        );
    }
}

/// A theme built here carries the default `Appearance`, so any render path
/// that constructs one silently sizes itself for a 13px editor font and a
/// Compact density. Rendering code must take the caller's theme.
#[test]
fn render_code_never_builds_its_own_theme() {
    fn walk(dir: &std::path::Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("read src") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                if path
                    .file_name()
                    .is_some_and(|name| name == "tests" || name == "benchmarks")
                {
                    continue;
                }
                walk(&path, offenders);
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !name.ends_with(".rs")
                || name.ends_with("tests.rs")
                || name == "theme.rs"
                || name == "smoke_tests.rs"
                // TextInput's constructor supplies a default until set_theme is called.
                // Match path components before formatting platform-specific diagnostics.
                || path.ends_with(std::path::Path::new("text_input/editing.rs"))
            {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read source");
            let production = match source.find("#[cfg(test)]") {
                Some(cut) => &source[..cut],
                None => &source[..],
            };
            for (ix, line) in production.lines().enumerate() {
                if line.contains("AppTheme::gitcomet_") {
                    offenders.push(format!("{}:{}", path.display(), ix + 1));
                }
            }
        }
    }

    let mut offenders = Vec::new();
    walk(std::path::Path::new("src"), &mut offenders);

    assert!(
        offenders.is_empty(),
        "these must take the theme they are handed: {offenders:?}"
    );
}
