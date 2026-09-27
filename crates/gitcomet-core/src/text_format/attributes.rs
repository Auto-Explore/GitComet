use super::{LineEnding, TextEncoding};
use std::sync::Arc;

/// The `text` attribute (or the legacy `crlf` one when `text` is unset).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAttr {
    #[default]
    Unspecified,
    Set,
    Unset,
    Auto,
    /// `text=input` / `crlf=input`: normalize on commit, no checkout conversion.
    Input,
}

/// `core.autocrlf`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AutoCrlf {
    #[default]
    False,
    True,
    Input,
}

/// An encoding named by an attribute or config value, kept even when the
/// label is not one we can decode so the UI can say so.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EncodingAttr {
    pub label: Arc<str>,
    pub encoding: Option<TextEncoding>,
}

impl EncodingAttr {
    pub fn from_label(label: &str) -> Self {
        Self {
            label: Arc::from(label.trim()),
            encoding: TextEncoding::from_label(label),
        }
    }
}

/// Where a tab width came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TabWidthSource {
    /// `whitespace=…tabwidth=N` in `.gitattributes`.
    Attribute,
    /// `tabwidth=N` in `core.whitespace`.
    CoreWhitespace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabWidth {
    pub columns: u8,
    pub source: TabWidthSource,
}

/// Why git converts (or leaves) line endings for a path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EolSource {
    /// Nothing asks for conversion.
    #[default]
    None,
    /// `eol=lf|crlf` (or `text=input`) in `.gitattributes`.
    EolAttribute,
    /// `text` / `text=auto` in `.gitattributes`, with the ending from
    /// `core.eol` / `core.autocrlf`.
    TextAttribute,
    /// `core.autocrlf`, no attribute.
    CoreAutocrlf,
}

/// Line-ending conversion git applies to a path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EolPolicy {
    /// Git stores LF (CRLF is normalized on add).
    pub normalized: bool,
    /// `text=auto` semantics: binary content and files already committed
    /// with CRLF are left alone.
    pub auto: bool,
    /// What a checkout writes; `None` when git leaves bytes untouched.
    pub checkout: Option<LineEnding>,
    pub source: EolSource,
}

impl EolPolicy {
    /// Git's `convert_attrs` + `output_eol` (convert.c). `core_eol` is `None`
    /// for "native".
    pub fn resolve(
        text: TextAttr,
        eol_attr: Option<LineEnding>,
        auto_crlf: AutoCrlf,
        core_eol: Option<LineEnding>,
    ) -> Self {
        let text_eol_is_crlf = match auto_crlf {
            AutoCrlf::True => true,
            AutoCrlf::Input => false,
            AutoCrlf::False => core_eol.unwrap_or(LineEnding::platform()) == LineEnding::CrLf,
        };
        let ending = |crlf: bool| {
            if crlf {
                LineEnding::CrLf
            } else {
                LineEnding::Lf
            }
        };
        let auto = text == TextAttr::Auto
            || (text == TextAttr::Unspecified
                && eol_attr.is_none()
                && auto_crlf != AutoCrlf::False);
        let normalized = |checkout, source| Self {
            normalized: true,
            auto,
            checkout: Some(checkout),
            source,
        };

        if text == TextAttr::Unset {
            return Self::default();
        }
        match eol_attr {
            Some(eol) if eol != LineEnding::Cr => {
                return normalized(eol, EolSource::EolAttribute);
            }
            _ => {}
        }
        match text {
            TextAttr::Set | TextAttr::Auto => {
                normalized(ending(text_eol_is_crlf), EolSource::TextAttribute)
            }
            TextAttr::Input => normalized(LineEnding::Lf, EolSource::EolAttribute),
            TextAttr::Unspecified => match auto_crlf {
                AutoCrlf::False => Self::default(),
                AutoCrlf::True => normalized(LineEnding::CrLf, EolSource::CoreAutocrlf),
                AutoCrlf::Input => normalized(LineEnding::Lf, EolSource::CoreAutocrlf),
            },
            TextAttr::Unset => Self::default(),
        }
    }
}

/// The attributes and config that decide how a path's content is read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextAttributes {
    pub text: TextAttr,
    /// `eol` attribute.
    pub eol: Option<LineEnding>,
    /// `-diff` or `binary`: git shows no textual diff.
    pub diff_unset: bool,
    /// A `filter=<driver>` attribute is set (LFS, annex, …).
    pub has_filter: bool,
    /// Contract expanded `$Id: ... $` markers when normalizing for Git.
    pub ident: bool,
    pub working_tree_encoding: Option<EncodingAttr>,
    /// The `encoding` attribute git-gui and gitk display files with.
    pub encoding: Option<EncodingAttr>,
    /// `gui.encoding`, the display fallback for files that are not UTF-8.
    pub gui_encoding: Option<EncodingAttr>,
    pub eol_policy: EolPolicy,
    pub tab_width: Option<TabWidth>,
}

impl TextAttributes {
    /// Only these resolved values affect decoding. Display metadata and label
    /// aliases must not invalidate decoded content.
    pub fn decoding_encodings(&self) -> [Option<TextEncoding>; 3] {
        [
            self.working_tree_encoding(),
            self.encoding.as_ref().and_then(|attr| attr.encoding),
            self.gui_encoding.as_ref().and_then(|attr| attr.encoding),
        ]
    }

    /// The `working-tree-encoding` when it names an encoding we can decode.
    /// Git stores such files as UTF-8.
    pub fn working_tree_encoding(&self) -> Option<TextEncoding> {
        self.working_tree_encoding
            .as_ref()
            .and_then(|attr| attr.encoding)
            .filter(|encoding| !encoding.is_utf8())
    }
}

/// `tabwidth=N` from a `whitespace` attribute value or `core.whitespace`.
/// Git accepts 1..=63.
pub fn parse_whitespace_tab_width(value: &str) -> Option<u8> {
    value
        .split(',')
        .filter_map(|part| part.trim().strip_prefix("tabwidth="))
        .next_back()
        .and_then(|width| width.trim().parse::<u8>().ok())
        .filter(|width| (1..=63).contains(width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_tab_width() {
        assert_eq!(parse_whitespace_tab_width("tabwidth=2"), Some(2));
        assert_eq!(
            parse_whitespace_tab_width("indent-with-non-tab, tabwidth=8"),
            Some(8)
        );
        assert_eq!(parse_whitespace_tab_width("tabwidth=0"), None);
        assert_eq!(parse_whitespace_tab_width("tabwidth=64"), None);
        assert_eq!(parse_whitespace_tab_width("trailing-space"), None);
    }

    #[test]
    fn eol_policy_follows_git() {
        use LineEnding::*;
        let lf_native = |text, eol, auto| EolPolicy::resolve(text, eol, auto, Some(Lf));
        // -text: never converted, even with autocrlf.
        assert_eq!(
            lf_native(TextAttr::Unset, Some(CrLf), AutoCrlf::True),
            EolPolicy::default()
        );
        // eol=crlf implies text.
        assert_eq!(
            lf_native(TextAttr::Unspecified, Some(CrLf), AutoCrlf::False).checkout,
            Some(CrLf)
        );
        // text + core.eol.
        assert_eq!(
            EolPolicy::resolve(TextAttr::Set, None, AutoCrlf::False, Some(CrLf)).checkout,
            Some(CrLf)
        );
        // text=auto + autocrlf=true.
        assert_eq!(
            lf_native(TextAttr::Auto, None, AutoCrlf::True).checkout,
            Some(CrLf)
        );
        // Nothing set: bytes untouched.
        assert_eq!(
            lf_native(TextAttr::Unspecified, None, AutoCrlf::False),
            EolPolicy::default()
        );
        // autocrlf=input normalizes without converting on checkout.
        let input = lf_native(TextAttr::Unspecified, None, AutoCrlf::Input);
        assert!(input.normalized);
        assert_eq!(input.checkout, Some(Lf));
        assert_eq!(input.source, EolSource::CoreAutocrlf);
    }

    #[test]
    fn utf8_working_tree_encoding_is_not_a_conversion() {
        let attrs = TextAttributes {
            working_tree_encoding: Some(EncodingAttr::from_label("UTF-8")),
            ..TextAttributes::default()
        };
        assert_eq!(attrs.working_tree_encoding(), None);
        let attrs = TextAttributes {
            working_tree_encoding: Some(EncodingAttr::from_label("UTF-16LE-BOM")),
            ..TextAttributes::default()
        };
        assert_eq!(attrs.working_tree_encoding(), Some(TextEncoding::UTF_16LE));
    }
}
