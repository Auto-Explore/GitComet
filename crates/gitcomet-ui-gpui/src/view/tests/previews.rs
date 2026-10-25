//! Rendered-preview detection and modes.

use super::*;

#[test]
fn is_markdown_path_detects_common_extensions() {
    use std::path::Path;
    assert!(is_markdown_path(Path::new("README.md")));
    assert!(is_markdown_path(Path::new("doc.markdown")));
    assert!(is_markdown_path(Path::new("notes.mdown")));
    assert!(is_markdown_path(Path::new("CHANGES.mkd")));
    assert!(is_markdown_path(Path::new("file.mkdn")));
    assert!(is_markdown_path(Path::new("file.mdwn")));
    assert!(is_markdown_path(Path::new("UPPER.MD")));
}

#[test]
fn is_markdown_path_rejects_non_markdown() {
    use std::path::Path;
    assert!(!is_markdown_path(Path::new("file.txt")));
    assert!(!is_markdown_path(Path::new("file.rs")));
    assert!(!is_markdown_path(Path::new("file")));
}

#[test]
fn should_bypass_text_file_preview_for_path_detects_supported_image_types() {
    use std::path::Path;

    for path in [
        "image.png",
        "image.JPEG",
        "image.gif",
        "image.webp",
        "image.bmp",
        "image.ico",
        "image.svg",
        "image.tif",
        "image.tiff",
    ] {
        assert!(
            should_bypass_text_file_preview_for_path(Path::new(path)),
            "expected {path} to bypass text file preview"
        );
    }

    for path in ["image.heic", "README.md", "notes.txt", "image"] {
        assert!(
            !should_bypass_text_file_preview_for_path(Path::new(path)),
            "did not expect {path} to bypass text file preview"
        );
    }
}

#[test]
fn preview_path_rendered_kind_detects_supported_preview_kinds() {
    use std::path::Path;

    assert_eq!(
        preview_path_rendered_kind(Path::new("diagram.svg")),
        Some(RenderedPreviewKind::Svg)
    );
    assert_eq!(
        preview_path_rendered_kind(Path::new("README.md")),
        Some(RenderedPreviewKind::Markdown)
    );
    assert_eq!(preview_path_rendered_kind(Path::new("notes.txt")), None);
}

#[test]
fn diff_target_rendered_preview_kind_reads_diff_target_paths() {
    let svg_target = DiffTarget::working_tree(PathBuf::from("diagram.svg"), DiffArea::Unstaged);
    assert_eq!(
        diff_target_rendered_preview_kind(Some(&svg_target)),
        Some(RenderedPreviewKind::Svg)
    );

    let markdown_target =
        DiffTarget::commit(CommitId("deadbeef".into()), PathBuf::from("README.md"));
    assert_eq!(
        diff_target_rendered_preview_kind(Some(&markdown_target)),
        Some(RenderedPreviewKind::Markdown)
    );

    let no_path_target = DiffTarget::commit_range(
        CommitId("parent".into()),
        Some(CommitId("deadbeef".into())),
        None,
    );
    assert_eq!(
        diff_target_rendered_preview_kind(Some(&no_path_target)),
        None
    );
}

#[test]
fn main_diff_rendered_preview_toggle_kind_matches_supported_modes() {
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(true, false, false, Some(RenderedPreviewKind::Svg),),
        Some(RenderedPreviewKind::Svg)
    );
    // The SVG Image/Code toggle is independent of the Full/Collapsed diff mode.
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(false, true, false, Some(RenderedPreviewKind::Svg),),
        Some(RenderedPreviewKind::Svg)
    );
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(false, false, false, Some(RenderedPreviewKind::Svg),),
        None
    );
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(
            true,
            false,
            false,
            Some(RenderedPreviewKind::Markdown),
        ),
        Some(RenderedPreviewKind::Markdown)
    );
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(
            false,
            false,
            true,
            Some(RenderedPreviewKind::Markdown),
        ),
        Some(RenderedPreviewKind::Markdown)
    );
}

#[test]
fn rendered_preview_modes_track_each_kind_independently() {
    let mut modes = RenderedPreviewModes::default();

    assert_eq!(
        modes.get(RenderedPreviewKind::Svg),
        RenderedPreviewMode::Rendered
    );
    assert_eq!(
        modes.get(RenderedPreviewKind::Markdown),
        RenderedPreviewMode::Rendered
    );

    modes.set(RenderedPreviewKind::Svg, RenderedPreviewMode::Source);
    modes.set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Source);

    assert_eq!(
        modes.get(RenderedPreviewKind::Svg),
        RenderedPreviewMode::Source
    );
    assert_eq!(
        modes.get(RenderedPreviewKind::Markdown),
        RenderedPreviewMode::Source
    );
}

#[test]
fn conflict_resolver_preview_mode_defaults_to_text() {
    assert_eq!(
        ConflictResolverPreviewMode::default(),
        ConflictResolverPreviewMode::Text
    );
}
