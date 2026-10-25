use super::*;

pub(crate) type LoadableMarkdownDoc =
    Loadable<Arc<crate::view::markdown_preview::MarkdownPreviewDocument>>;

pub(crate) type LoadableMarkdownDiff =
    Loadable<Arc<crate::view::markdown_preview::MarkdownPreviewDiff>>;

/// The rendered markdown surface quick search is looking at.
///
/// Each shape has its own row space and its own way of being scrolled, which
/// is why search dispatches on this rather than on the preview kind alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum MarkdownSearchSurface {
    /// Rendered file preview: one flowing document, no fixed row height.
    Worktree,
    /// Rendered markdown diff, inline: one flowing document.
    DiffInline,
    /// Rendered markdown diff, split: two aligned documents in one scroller.
    DiffSplit,
    /// Merge tool rendered preview: one unwrapped list per input column.
    Conflict,
}
