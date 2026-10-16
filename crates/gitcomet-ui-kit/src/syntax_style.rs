//! Theme styles for syntax token kinds.
use crate::theme::AppTheme;
use gitcomet_text::{SyntaxPalette, SyntaxTokenKind};
use palette::IntoColor;

pub fn syntax_highlight_color(theme: AppTheme, kind: SyntaxTokenKind) -> Option<gpui::Rgba> {
    match kind {
        SyntaxTokenKind::None => None,
        SyntaxTokenKind::Comment => Some(theme.syntax.comment),
        SyntaxTokenKind::CommentDoc => Some(theme.syntax.comment_doc),
        SyntaxTokenKind::String => Some(theme.syntax.string),
        SyntaxTokenKind::StringEscape => Some(theme.syntax.string_escape),
        SyntaxTokenKind::StringRegex => Some(theme.syntax.string_regex),
        SyntaxTokenKind::StringSpecial => Some(theme.syntax.string_special),
        SyntaxTokenKind::Keyword => Some(theme.syntax.keyword),
        SyntaxTokenKind::KeywordControl => Some(theme.syntax.keyword_control),
        SyntaxTokenKind::Preproc => Some(theme.syntax.preproc),
        SyntaxTokenKind::Number => Some(theme.syntax.number),
        SyntaxTokenKind::Boolean => Some(theme.syntax.boolean),
        SyntaxTokenKind::Function => Some(theme.syntax.function),
        SyntaxTokenKind::FunctionMethod => Some(theme.syntax.function_method),
        SyntaxTokenKind::FunctionSpecial => Some(theme.syntax.function_special),
        SyntaxTokenKind::Constructor => Some(theme.syntax.constructor),
        SyntaxTokenKind::Type => Some(theme.syntax.type_name),
        SyntaxTokenKind::TypeBuiltin => Some(theme.syntax.type_builtin),
        SyntaxTokenKind::TypeInterface => Some(theme.syntax.type_interface),
        SyntaxTokenKind::Namespace => Some(theme.syntax.namespace),
        SyntaxTokenKind::Variable => theme.syntax.variable,
        SyntaxTokenKind::VariableParameter => Some(theme.syntax.variable_parameter),
        SyntaxTokenKind::VariableSpecial => Some(theme.syntax.variable_special),
        SyntaxTokenKind::VariableBuiltin => Some(theme.syntax.variable_builtin),
        SyntaxTokenKind::Property => Some(theme.syntax.property),
        SyntaxTokenKind::Label => theme.syntax.label,
        SyntaxTokenKind::Constant => Some(theme.syntax.constant),
        SyntaxTokenKind::ConstantBuiltin => Some(theme.syntax.constant_builtin),
        SyntaxTokenKind::Operator => Some(theme.syntax.operator),
        SyntaxTokenKind::Punctuation => Some(theme.syntax.punctuation),
        SyntaxTokenKind::PunctuationBracket => Some(theme.syntax.punctuation_bracket),
        SyntaxTokenKind::PunctuationDelimiter => Some(theme.syntax.punctuation_delimiter),
        SyntaxTokenKind::PunctuationSpecial => Some(theme.syntax.punctuation_special),
        SyntaxTokenKind::PunctuationListMarker => Some(theme.syntax.punctuation_list_marker),
        SyntaxTokenKind::Tag => Some(theme.syntax.tag),
        SyntaxTokenKind::Attribute => Some(theme.syntax.attribute),
        SyntaxTokenKind::MarkupHeading => Some(theme.syntax.markup_heading),
        SyntaxTokenKind::MarkupLink => Some(theme.syntax.markup_link),
        SyntaxTokenKind::TextLiteral => Some(theme.syntax.text_literal),
        SyntaxTokenKind::DiffPlus => Some(theme.syntax.diff_plus),
        SyntaxTokenKind::DiffMinus => Some(theme.syntax.diff_minus),
        SyntaxTokenKind::DiffDelta => Some(theme.syntax.diff_delta),
        SyntaxTokenKind::Lifetime => Some(theme.syntax.lifetime),
    }
}

pub fn syntax_highlight_style(
    theme: AppTheme,
    kind: SyntaxTokenKind,
) -> Option<gpui::HighlightStyle> {
    let fg = syntax_highlight_color(theme, kind)?;
    let mut style = gpui::HighlightStyle {
        color: Some(fg.into_color()),
        ..gpui::HighlightStyle::default()
    };
    match kind {
        // Doc comments render italic to distinguish from regular comments.
        SyntaxTokenKind::CommentDoc => {
            style.font_style = Some(gpui::FontStyle::Italic);
        }
        // Control-flow keywords (if/else/for/while/return/match) render semibold.
        SyntaxTokenKind::KeywordControl => {
            style.font_weight = Some(gpui::FontWeight::SEMIBOLD);
        }
        _ => {}
    }
    Some(style)
}

/// Every token kind's style for one theme, resolved once.
#[derive(Clone)]
pub struct SyntaxHighlightPalette {
    styles: [Option<gpui::HighlightStyle>; SyntaxTokenKind::ALL.len()],
}

impl SyntaxHighlightPalette {
    pub fn new(theme: AppTheme) -> Self {
        Self {
            styles: std::array::from_fn(|ix| {
                syntax_highlight_style(theme, SyntaxTokenKind::ALL[ix])
            }),
        }
    }

    pub fn style(&self, kind: SyntaxTokenKind) -> Option<gpui::HighlightStyle> {
        self.styles[kind as usize]
    }
}

impl SyntaxPalette for SyntaxHighlightPalette {
    type Style = gpui::HighlightStyle;

    fn style(&self, kind: SyntaxTokenKind) -> Option<gpui::HighlightStyle> {
        SyntaxHighlightPalette::style(self, kind)
    }
}

pub fn syntax_highlight_palette(theme: AppTheme) -> SyntaxHighlightPalette {
    SyntaxHighlightPalette::new(theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_comment_renders_italic() {
        let theme = AppTheme::gitcomet_dark();
        let style = syntax_highlight_style(theme, SyntaxTokenKind::CommentDoc);
        assert!(style.is_some());
        let style = style.unwrap();
        assert_eq!(style.font_style, Some(gpui::FontStyle::Italic));
        // Regular comments should not be italic.
        let plain = syntax_highlight_style(theme, SyntaxTokenKind::Comment).unwrap();
        assert_eq!(plain.font_style, None);
    }

    #[test]
    fn keyword_control_renders_semibold() {
        let theme = AppTheme::gitcomet_dark();
        let style = syntax_highlight_style(theme, SyntaxTokenKind::KeywordControl);
        assert!(style.is_some());
        let style = style.unwrap();
        assert_eq!(style.font_weight, Some(gpui::FontWeight::SEMIBOLD));
        // Regular keywords should not have font weight.
        let plain = syntax_highlight_style(theme, SyntaxTokenKind::Keyword).unwrap();
        assert_eq!(plain.font_weight, None);
    }

    #[test]
    fn syntax_highlight_palette_matches_direct_styles() {
        let theme = AppTheme::gitcomet_dark();
        let palette = syntax_highlight_palette(theme);

        for kind in SyntaxTokenKind::ALL {
            let direct = syntax_highlight_style(theme, kind);
            let cached = palette.style(kind);
            assert_eq!(
                cached.is_some(),
                direct.is_some(),
                "style presence mismatch for {kind:?}"
            );
            if let (Some(cached), Some(direct)) = (cached, direct) {
                assert_eq!(cached.color, direct.color, "color mismatch for {kind:?}");
                assert_eq!(
                    cached.background_color, direct.background_color,
                    "background mismatch for {kind:?}"
                );
                assert_eq!(
                    cached.font_style, direct.font_style,
                    "font_style mismatch for {kind:?}"
                );
                assert_eq!(
                    cached.font_weight, direct.font_weight,
                    "font_weight mismatch for {kind:?}"
                );
            }
        }
    }

    #[test]
    fn syntax_highlight_style_uses_theme_syntax_overrides() {
        let theme = AppTheme::from_json_str(&crate::theme::test_theme_json_with_syntax(
            "gitcomet_dark",
            r##"{
                "keyword": "#112233ff",
                "variable": "#445566ff",
                "diff_plus": "#abcdefff",
                "label": "#fedcbaff"
            }"##,
        ))
        .expect("theme JSON should parse");

        let keyword = syntax_highlight_style(theme, SyntaxTokenKind::Keyword)
            .expect("keyword style should be present");
        assert_eq!(keyword.color, Some(gpui::rgba(0x112233ff).into_color()));

        let variable = syntax_highlight_style(theme, SyntaxTokenKind::Variable)
            .expect("variable style should be present when overridden");
        assert_eq!(variable.color, Some(gpui::rgba(0x445566ff).into_color()));

        let diff_plus = syntax_highlight_style(theme, SyntaxTokenKind::DiffPlus)
            .expect("diff_plus style should be present");
        assert_eq!(diff_plus.color, Some(gpui::rgba(0xabcdefff).into_color()));

        let label = syntax_highlight_style(theme, SyntaxTokenKind::Label)
            .expect("label style should be present when overridden");
        assert_eq!(label.color, Some(gpui::rgba(0xfedcbaff).into_color()));
    }

    #[test]
    fn syntax_highlight_style_kind_table_covers_new_token_kinds() {
        for kind in [
            SyntaxTokenKind::StringRegex,
            SyntaxTokenKind::StringSpecial,
            SyntaxTokenKind::Preproc,
            SyntaxTokenKind::Constructor,
            SyntaxTokenKind::Namespace,
            SyntaxTokenKind::VariableBuiltin,
            SyntaxTokenKind::Label,
            SyntaxTokenKind::ConstantBuiltin,
            SyntaxTokenKind::PunctuationSpecial,
            SyntaxTokenKind::PunctuationListMarker,
            SyntaxTokenKind::MarkupHeading,
            SyntaxTokenKind::MarkupLink,
            SyntaxTokenKind::TextLiteral,
            SyntaxTokenKind::DiffPlus,
            SyntaxTokenKind::DiffMinus,
            SyntaxTokenKind::DiffDelta,
        ] {
            assert!(
                SyntaxTokenKind::ALL.contains(&kind),
                "syntax highlight palette should cover {kind:?}"
            );
        }
    }

    #[test]
    fn syntax_highlight_style_uses_all_new_theme_syntax_overrides() {
        let theme = AppTheme::from_json_str(&crate::theme::test_theme_json_with_syntax(
            "gitcomet_dark",
            r##"{
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

        for (kind, color) in [
            (SyntaxTokenKind::StringRegex, 0x010101ff),
            (SyntaxTokenKind::StringSpecial, 0x020202ff),
            (SyntaxTokenKind::Preproc, 0x030303ff),
            (SyntaxTokenKind::Constructor, 0x040404ff),
            (SyntaxTokenKind::Namespace, 0x050505ff),
            (SyntaxTokenKind::VariableBuiltin, 0x060606ff),
            (SyntaxTokenKind::Label, 0x070707ff),
            (SyntaxTokenKind::ConstantBuiltin, 0x080808ff),
            (SyntaxTokenKind::PunctuationSpecial, 0x090909ff),
            (SyntaxTokenKind::PunctuationListMarker, 0x0a0a0aff),
            (SyntaxTokenKind::MarkupHeading, 0x0b0b0bff),
            (SyntaxTokenKind::MarkupLink, 0x0c0c0cff),
            (SyntaxTokenKind::TextLiteral, 0x0d0d0dff),
            (SyntaxTokenKind::DiffPlus, 0x0e0e0eff),
            (SyntaxTokenKind::DiffMinus, 0x0f0f0fff),
            (SyntaxTokenKind::DiffDelta, 0x101010ff),
        ] {
            let style = syntax_highlight_style(theme, kind)
                .unwrap_or_else(|| panic!("{kind:?} style should be present"));
            assert_eq!(
                style.color,
                Some(gpui::rgba(color).into_color()),
                "{kind:?} should use its explicit syntax override"
            );
        }
    }
}
