//! Scrollbar markers for markdown previews, built from the model's bucket flags.
use super::components::ScrollbarMarker;
use super::diff_utils::scrollbar_markers_from_flags;
use super::markdown_preview::{
    MarkdownPreviewDiff, MarkdownPreviewDocument, scrollbar_bucket_flags,
    scrollbar_bucket_flags_for_extents,
};

pub(super) fn scrollbar_markers_for_diff_preview(
    preview: &MarkdownPreviewDiff,
) -> Vec<ScrollbarMarker> {
    markers(&scrollbar_bucket_flags(&[&preview.old, &preview.new]))
}

pub(super) fn scrollbar_markers_for_document(
    document: &MarkdownPreviewDocument,
) -> Vec<ScrollbarMarker> {
    markers(&scrollbar_bucket_flags(&[document]))
}

/// Markers for changes measured at `(top, bottom, flag)` in a scroll content
/// `content_height` tall.
pub(super) fn scrollbar_markers_for_extents(
    extents: &[(f32, f32, u8)],
    content_height: f32,
) -> Vec<ScrollbarMarker> {
    markers(&scrollbar_bucket_flags_for_extents(extents, content_height))
}

fn markers(flags: &[u8]) -> Vec<ScrollbarMarker> {
    scrollbar_markers_from_flags(flags.len(), |ix| flags[ix])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::markdown_preview::build_markdown_diff_preview;

    #[test]
    fn markdown_diff_scrollbar_markers_show_added_rows_for_one_sided_preview() {
        let preview = build_markdown_diff_preview("", "# New\n").unwrap();

        assert_eq!(
            scrollbar_markers_for_diff_preview(&preview),
            vec![crate::view::components::ScrollbarMarker {
                start: 0.0,
                end: 1.0,
                kind: crate::view::components::ScrollbarMarkerKind::Add,
            }]
        );
    }

    #[test]
    fn markdown_diff_scrollbar_markers_show_removed_rows_for_one_sided_preview() {
        let preview = build_markdown_diff_preview("# Gone\n", "").unwrap();

        assert_eq!(
            scrollbar_markers_for_diff_preview(&preview),
            vec![crate::view::components::ScrollbarMarker {
                start: 0.0,
                end: 1.0,
                kind: crate::view::components::ScrollbarMarkerKind::Remove,
            }]
        );
    }

    #[test]
    fn markdown_diff_scrollbar_markers_merge_replacements_into_modify_markers() {
        let preview = build_markdown_diff_preview("old\n", "new\n").unwrap();

        assert_eq!(
            scrollbar_markers_for_diff_preview(&preview),
            vec![crate::view::components::ScrollbarMarker {
                start: 0.0,
                end: 1.0,
                kind: crate::view::components::ScrollbarMarkerKind::Modify,
            }]
        );
    }

    #[test]
    fn markdown_diff_scrollbar_markers_split_disjoint_change_regions() {
        let preview = build_markdown_diff_preview(
            "- old one\n- keep two\n- keep three\n- keep four\n- old five\n",
            "- new one\n- keep two\n- keep three\n- keep four\n- new five\n",
        )
        .unwrap();

        assert_eq!(
            scrollbar_markers_for_diff_preview(&preview),
            vec![
                crate::view::components::ScrollbarMarker {
                    start: 0.0,
                    end: 0.2,
                    kind: crate::view::components::ScrollbarMarkerKind::Modify,
                },
                crate::view::components::ScrollbarMarker {
                    start: 0.8,
                    end: 1.0,
                    kind: crate::view::components::ScrollbarMarkerKind::Modify,
                },
            ]
        );
    }

    #[test]
    fn measured_change_extents_become_markers_where_they_were_drawn() {
        // 1000px of content, an addition drawn at 800..850.
        let markers = scrollbar_markers_for_extents(&[(800.0, 850.0, 1)], 1000.0);
        assert_eq!(markers.len(), 1, "{markers:?}");
        assert!((markers[0].start - 0.8).abs() < 0.01, "{markers:?}");
        assert!((markers[0].end - 0.85).abs() < 0.01, "{markers:?}");
    }
}
