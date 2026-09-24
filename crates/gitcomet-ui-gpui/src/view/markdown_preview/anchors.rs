use super::{MarkdownPreviewDocument, MarkdownPreviewRowKind};
use rustc_hash::FxHashMap;

/// The id GitHub gives a heading: lowercased, spaces to `-`, and every
/// character other than a letter, mark, digit, `-` or `_` dropped. Marks stay:
/// they spell the word in scripts such as Devanagari (and in decomposed
/// accents).
pub(in crate::view) fn markdown_heading_slug(text: &str) -> String {
    text.trim()
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|ch| match ch {
            ' ' => Some('-'),
            '-' | '_' => Some(ch),
            _ if ch.is_alphanumeric() || unicode_normalization::char::is_combining_mark(ch) => {
                Some(ch)
            }
            _ => None,
        })
        .collect()
}

/// Row of the heading `#fragment` names. A repeated heading takes `-1`,
/// `-2`, … in document order, numbered the way github-slugger does: the count
/// belongs to the heading's own slug, so `Example 1` after two `Example`s is
/// still `example-1-1`. An exact match wins over one that differs only in case.
pub(in crate::view) fn markdown_preview_anchor_row(
    document: &MarkdownPreviewDocument,
    fragment: &str,
) -> Option<usize> {
    let mut occurrences: FxHashMap<String, usize> = FxHashMap::default();
    let mut case_insensitive = None;
    for (row_ix, row) in document.rows.iter().enumerate() {
        if !matches!(row.kind, MarkdownPreviewRowKind::Heading { .. }) {
            continue;
        }
        let base = markdown_heading_slug(&row.text);
        let mut slug = base.clone();
        while occurrences.contains_key(&slug) {
            let count = occurrences.entry(base.clone()).or_default();
            *count += 1;
            slug = format!("{base}-{count}");
        }
        occurrences.insert(slug.clone(), 0);
        if slug == fragment {
            return Some(row_ix);
        }
        if case_insensitive.is_none() && slug.eq_ignore_ascii_case(fragment) {
            case_insensitive = Some(row_ix);
        }
    }
    case_insensitive
}
