use super::GixRepo;
use gitcomet_core::services::Result;
use gitcomet_core::text_format::{
    AutoCrlf, EncodingAttr, EolPolicy, LineEnding, TabWidth, TabWidthSource, TextAttr,
    TextAttributes, parse_whitespace_tab_width,
};
use gix::attrs::StateRef;
use std::path::Path;

/// Attributes consulted, in the order `iter_selected` returns them.
const TEXT_ATTRIBUTES: [&str; 9] = [
    "text",
    "crlf",
    "eol",
    "diff",
    "filter",
    "working-tree-encoding",
    "encoding",
    "whitespace",
    "ident",
];

impl GixRepo {
    pub(super) fn text_attributes_impl(&self, path: &Path) -> Result<TextAttributes> {
        let repo_path = super::diff::to_repo_path(path, &self.spec.workdir)?;
        // Configuration snapshots belong to the opened handle. Attribute
        // refreshes must also observe config changes made while it was open.
        Ok(resolve_text_attributes(
            &self.repo_with_current_config()?,
            &repo_path,
        ))
    }
}

/// `.gitattributes` and config for `path` (repo-relative). Unreadable
/// attribute files or config degrade to defaults, as git does for invalid
/// values, rather than failing the load that asked.
pub(super) fn resolve_text_attributes(repo: &gix::Repository, path: &Path) -> TextAttributes {
    let config = repo.config_snapshot();
    let config_string = |key: &str| config.string(key).map(|value| value.to_string());
    let auto_crlf = parse_auto_crlf(config_string("core.autocrlf").as_deref());
    let core_eol = match config_string("core.eol").as_deref().map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("crlf") => Some(LineEnding::CrLf),
        Some(value) if value.eq_ignore_ascii_case("lf") => Some(LineEnding::Lf),
        _ => None,
    };

    let mut attributes = TextAttributes {
        gui_encoding: config_string("gui.encoding")
            .filter(|label| !label.trim().is_empty())
            .map(|label| EncodingAttr::from_label(&label)),
        tab_width: config_string("core.whitespace")
            .as_deref()
            .and_then(parse_whitespace_tab_width)
            .map(|columns| TabWidth {
                columns,
                source: TabWidthSource::CoreWhitespace,
            }),
        ..TextAttributes::default()
    };

    let mut eol_attr = None;
    if let Some(matches) = selected_attribute_values(repo, path) {
        let [
            text,
            crlf,
            eol,
            diff,
            filter,
            wte,
            encoding,
            whitespace,
            ident,
        ] = matches;
        attributes.text = match text_attr(&text) {
            TextAttr::Unspecified => text_attr(&crlf),
            other => other,
        };
        eol_attr = match &eol {
            AttrValue::Value(value) if value.eq_ignore_ascii_case("lf") => Some(LineEnding::Lf),
            AttrValue::Value(value) if value.eq_ignore_ascii_case("crlf") => Some(LineEnding::CrLf),
            _ => None,
        };
        attributes.diff_unset = matches!(diff, AttrValue::Unset);
        attributes.has_filter = matches!(filter, AttrValue::Value(_) | AttrValue::Set);
        attributes.ident = matches!(ident, AttrValue::Set);
        if let AttrValue::Value(label) = wte {
            attributes.working_tree_encoding = Some(EncodingAttr::from_label(&label));
        }
        if let AttrValue::Value(label) = encoding {
            attributes.encoding = Some(EncodingAttr::from_label(&label));
        }
        if let AttrValue::Value(value) = whitespace
            && let Some(columns) = parse_whitespace_tab_width(&value)
        {
            attributes.tab_width = Some(TabWidth {
                columns,
                source: TabWidthSource::Attribute,
            });
        }
    }
    attributes.eol = eol_attr;
    attributes.eol_policy = EolPolicy::resolve(attributes.text, eol_attr, auto_crlf, core_eol);
    attributes
}

enum AttrValue {
    Unspecified,
    Set,
    Unset,
    Value(String),
}

fn selected_attribute_values(
    repo: &gix::Repository,
    path: &Path,
) -> Option<[AttrValue; TEXT_ATTRIBUTES.len()]> {
    let index = repo.index_or_empty().ok()?;
    let mut stack = repo
        .attributes_only(
            &index,
            gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
        )
        .ok()?;
    let mut outcome = stack.selected_attribute_matches(TEXT_ATTRIBUTES);
    stack
        .at_path(path, None)
        .ok()?
        .matching_attributes(&mut outcome);
    let mut values = outcome
        .iter_selected()
        .map(|matched| match matched.assignment.state {
            StateRef::Unspecified => AttrValue::Unspecified,
            StateRef::Set => AttrValue::Set,
            StateRef::Unset => AttrValue::Unset,
            StateRef::Value(value) => AttrValue::Value(value.as_bstr().to_string()),
        });
    Some(std::array::from_fn(|_| {
        values.next().unwrap_or(AttrValue::Unspecified)
    }))
}

/// Git's `git_path_check_crlf`.
fn text_attr(value: &AttrValue) -> TextAttr {
    match value {
        AttrValue::Set => TextAttr::Set,
        AttrValue::Unset => TextAttr::Unset,
        AttrValue::Value(value) if value == "auto" => TextAttr::Auto,
        AttrValue::Value(value) if value == "input" => TextAttr::Input,
        AttrValue::Value(_) | AttrValue::Unspecified => TextAttr::Unspecified,
    }
}

fn parse_auto_crlf(value: Option<&str>) -> AutoCrlf {
    let Some(value) = value.map(str::trim) else {
        return AutoCrlf::False;
    };
    if value.eq_ignore_ascii_case("input") {
        AutoCrlf::Input
    } else if value.is_empty()
        || ["true", "yes", "on", "1"]
            .iter()
            .any(|truthy| value.eq_ignore_ascii_case(truthy))
    {
        AutoCrlf::True
    } else {
        AutoCrlf::False
    }
}
