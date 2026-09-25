use super::{MarkdownPreviewDocument, MarkdownPreviewRowKind};
use rustc_hash::FxHashMap;
use std::sync::{Arc, OnceLock};

/// The id GitHub gives a heading: lowercased, spaces to `-`, and every
/// character other than a letter, mark, digit, `-` or `_` dropped. Marks stay:
/// they spell the word in scripts such as Devanagari (and in decomposed
/// accents).
pub(in crate::view) fn markdown_heading_slug(text: &str) -> String {
    #[cfg(test)]
    HEADING_SLUGS.with(|slugs| slugs.set(slugs.get() + 1));
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

#[cfg(test)]
thread_local! {
    static HEADING_SLUGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Headings slugged since the last call, which resets the count.
#[cfg(test)]
pub(in crate::view) fn take_heading_slugs_for_tests() -> usize {
    HEADING_SLUGS.with(|slugs| slugs.replace(0))
}

/// A document's heading ids, built once — while parsing when the document
/// links to a heading, else on the first lookup — where every hover over a
/// link and every click on one used to slug each heading again. Derived from
/// the rows, so equality ignores it.
#[derive(Clone, Debug, Default)]
pub(in crate::view) struct MarkdownAnchorIndexCell(OnceLock<Arc<MarkdownAnchorIndex>>);

impl PartialEq for MarkdownAnchorIndexCell {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for MarkdownAnchorIndexCell {}

#[derive(Debug, Default)]
pub(in crate::view) struct MarkdownAnchorIndex {
    /// Each id and its heading's row. Ids are unique: repeats are numbered.
    exact: FxHashMap<String, usize>,
    /// ASCII-lowercased ids, each with the first heading that has it.
    folded: FxHashMap<String, usize>,
}

impl MarkdownAnchorIndex {
    /// A repeated heading takes `-1`, `-2`, … in document order, numbered the
    /// way github-slugger does: the count belongs to the heading's own slug,
    /// so `Example 1` after two `Example`s is still `example-1-1`.
    fn build(document: &MarkdownPreviewDocument) -> Self {
        let mut occurrences: FxHashMap<String, usize> = FxHashMap::default();
        let mut index = Self::default();
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
            index
                .folded
                .entry(slug.to_ascii_lowercase())
                .or_insert(row_ix);
            index.exact.entry(slug).or_insert(row_ix);
        }
        index
    }
}

impl MarkdownPreviewDocument {
    /// Build the heading ids now when a link in the document will look them
    /// up, so the first hover over one does not: parsing runs off the UI
    /// thread.
    pub(super) fn index_anchors_if_linked(&self) {
        let is_anchor = |url: &Option<gpui::SharedString>| {
            url.as_deref()
                .is_some_and(|url| url.trim_start().starts_with('#'))
        };
        let linked = self.rows.iter().any(|row| {
            row.inline_spans
                .iter()
                .any(|span| is_anchor(&span.link_url))
                || row
                    .inline_images
                    .iter()
                    .any(|image| is_anchor(&image.link_url))
        });
        if linked {
            self.anchors
                .0
                .get_or_init(|| Arc::new(MarkdownAnchorIndex::build(self)));
        }
    }
}

/// Row of the heading `#fragment` names. An exact match wins over one that
/// differs only in case.
pub(in crate::view) fn markdown_preview_anchor_row(
    document: &MarkdownPreviewDocument,
    fragment: &str,
) -> Option<usize> {
    let index = document
        .anchors
        .0
        .get_or_init(|| Arc::new(MarkdownAnchorIndex::build(document)));
    index
        .exact
        .get(fragment)
        .or_else(|| index.folded.get(&fragment.to_ascii_lowercase()))
        .copied()
}
