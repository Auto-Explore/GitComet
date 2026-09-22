use super::{MarkdownPreviewDocument, MarkdownPreviewRowKind};
use rustc_hash::FxHashMap;

/// The id GitHub gives a heading: lowercased, spaces to `-`, and every
/// character other than a letter, digit, `-` or `_` dropped.
pub(in crate::view) fn markdown_heading_slug(text: &str) -> String {
    text.trim()
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|ch| match ch {
            ' ' => Some('-'),
            '-' | '_' => Some(ch),
            _ if ch.is_alphanumeric() => Some(ch),
            _ => None,
        })
        .collect()
}

/// Row of the heading `#fragment` names. A repeated heading takes `-1`,
/// `-2`, … in document order, as on GitHub. An exact match wins over one
/// that differs only in case.
pub(in crate::view) fn markdown_preview_anchor_row(
    document: &MarkdownPreviewDocument,
    fragment: &str,
) -> Option<usize> {
    let mut seen: FxHashMap<String, usize> = FxHashMap::default();
    let mut case_insensitive = None;
    for (row_ix, row) in document.rows.iter().enumerate() {
        if !matches!(row.kind, MarkdownPreviewRowKind::Heading { .. }) {
            continue;
        }
        let base = markdown_heading_slug(&row.text);
        let mut slug = base.clone();
        while let Some(count) = seen.get_mut(&slug) {
            *count += 1;
            slug = format!("{base}-{count}");
        }
        seen.insert(slug.clone(), 0);
        if slug == fragment {
            return Some(row_ix);
        }
        if case_insensitive.is_none() && slug.eq_ignore_ascii_case(fragment) {
            case_insensitive = Some(row_ix);
        }
    }
    case_insensitive
}
