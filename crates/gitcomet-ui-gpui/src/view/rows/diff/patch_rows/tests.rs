use super::*;

/// Collapsed hunk headers and split meta rows are painted with the editor
/// font, so their boxes have to follow it or the text clips.
#[test]
fn diff_rows_grow_with_the_editor_font() {
    let theme = |editor_font_size_px| {
        AppTheme::gitcomet_dark().with_appearance(crate::appearance::Appearance {
            editor_font_size_px,
            ..crate::appearance::Appearance::default()
        })
    };

    for percent in [100, 150] {
        let small = collapsed_hunk_header_row_height(theme(8), percent);
        let default = collapsed_hunk_header_row_height(theme(13), percent);
        let large = collapsed_hunk_header_row_height(theme(32), percent);

        assert!(small < default && default < large, "at {percent}%");
        assert!(
            large >= theme(32).editor_font_size(percent),
            "the row must clear the glyphs it paints at {percent}%"
        );
    }
}

/// The chevron tracks the editor row it sits in, not the UI density, and
/// stops growing before a large editor font turns it into a slab.
#[test]
fn collapsed_hunk_reveal_buttons_track_the_editor_row_and_cap_out() {
    let at = |editor_font_size_px: u32, density, percent| {
        collapsed_hunk_reveal_button_size(
            AppTheme::gitcomet_dark().with_appearance(crate::appearance::Appearance {
                editor_font_size_px,
                density,
                ..crate::appearance::Appearance::default()
            }),
            percent,
        )
    };
    use crate::appearance::UiDensity::{Comfortable, Compact};

    assert!(at(8, Compact, 100) < at(13, Compact, 100));
    assert!(at(13, Compact, 100) < at(100, Compact, 100));
    assert_eq!(at(32, Compact, 100), at(13, Compact, 100).max(px(24.0)));
    assert_eq!(
        at(13, Compact, 100),
        at(13, Comfortable, 100),
        "the diff body follows the editor font, not the density"
    );
    assert_eq!(at(13, Compact, 200), at(13, Compact, 100) * 2.0);

    for size in [8, 13, 32] {
        assert!(
            at(size, Compact, 100)
                <= AppTheme::gitcomet_dark()
                    .with_appearance(crate::appearance::Appearance {
                        editor_font_size_px: size,
                        ..crate::appearance::Appearance::default()
                    })
                    .editor_row_height(100),
            "the chevron must never outgrow its row at {size}px"
        );
    }
}

fn collapsed_hunk(has_removals: bool, has_additions: bool) -> CollapsedDiffHunk {
    CollapsedDiffHunk {
        src_ix: 0,
        base_row_start: 0,
        base_row_end_exclusive: 1,
        has_additions,
        has_removals,
        reveal_up_lines: 0,
        reveal_down_lines: 0,
    }
}

#[test]
fn focused_diff_row_backgrounds_are_semantic_and_not_text_selection() {
    for (key, _) in crate::theme::bundled_theme_keys() {
        let theme = AppTheme::from_key(&key).expect("bundled theme should load");
        let text_selection_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.28 } else { 0.18 },
        );
        let add_focus = focused_diff_line_bg(theme, DiffLineKind::Add);
        let remove_focus = focused_diff_line_bg(theme, DiffLineKind::Remove);
        let neutral_focus = focused_diff_line_bg(theme, DiffLineKind::Context);
        let (add_bg, _, _) = diff_line_colors(theme, DiffLineKind::Add);
        let (remove_bg, _, _) = diff_line_colors(theme, DiffLineKind::Remove);
        let (context_bg, _, _) = diff_line_colors(theme, DiffLineKind::Context);

        // The focused row is the diff palette's own token flattened onto
        // the editor surface, not a tint mixed from the status palette.
        // Canvas-backed rows reuse the result, so it must already be opaque.
        assert_eq!(
            add_focus,
            crate::theme::composite_over(
                theme.colors.editor.background,
                theme.colors.diff.added.focused_background,
            )
        );
        assert_eq!(
            remove_focus,
            crate::theme::composite_over(
                theme.colors.editor.background,
                theme.colors.diff.removed.focused_background,
            )
        );
        assert_eq!(
            add_bg,
            crate::theme::composite_over(
                theme.colors.editor.background,
                theme.colors.diff.added.background,
            )
        );
        assert_eq!(
            remove_bg,
            crate::theme::composite_over(
                theme.colors.editor.background,
                theme.colors.diff.removed.background,
            )
        );
        for background in [
            add_bg,
            remove_bg,
            context_bg,
            add_focus,
            remove_focus,
            neutral_focus,
        ] {
            assert_eq!(
                background.alpha, 1.0,
                "diff row backgrounds passed to the canvas must be opaque"
            );
        }

        assert_ne!(add_focus, text_selection_bg);
        assert_ne!(remove_focus, text_selection_bg);
        assert_ne!(neutral_focus, text_selection_bg);
        assert_ne!(neutral_focus, theme.colors.diff.modified.focused_background);
        assert_ne!(add_focus, add_bg);
        assert_ne!(remove_focus, remove_bg);
        assert_ne!(neutral_focus, context_bg);
        assert_ne!(add_focus, remove_focus);
        assert_ne!(add_focus, neutral_focus);
        assert_ne!(remove_focus, neutral_focus);

        let collapsed_focus = focused_collapsed_hunk_bg(theme, None);
        let expected_collapsed_focus = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.22 } else { 0.16 },
        );
        assert_eq!(collapsed_focus, expected_collapsed_focus);
        assert_ne!(collapsed_focus, add_focus);
        assert_ne!(collapsed_focus, remove_focus);
        assert_ne!(collapsed_focus, neutral_focus);
        assert_ne!(collapsed_focus, text_selection_bg);
        assert_eq!(
            focused_collapsed_hunk_bg(theme, Some(collapsed_hunk(false, true))),
            collapsed_focus
        );
        assert_eq!(
            focused_collapsed_hunk_bg(theme, Some(collapsed_hunk(true, false))),
            collapsed_focus
        );
        assert_eq!(
            focused_collapsed_hunk_bg(theme, Some(collapsed_hunk(true, true))),
            collapsed_focus
        );
        assert_eq!(focused_collapsed_hunk_bg(theme, None), collapsed_focus);
    }
}

#[test]
fn collapsed_hunk_headers_use_uniform_diff_row_height() {
    for ui_scale_percent in [75, 100, 125, 150, 200] {
        assert_eq!(
            collapsed_hunk_header_row_height(AppTheme::gitcomet_dark(), ui_scale_percent),
            AppTheme::gitcomet_dark().editor_row_height(ui_scale_percent)
        );
        assert_ne!(
            collapsed_hunk_header_row_height(AppTheme::gitcomet_dark(), ui_scale_percent),
            diff_hunk_header_height(AppTheme::gitcomet_dark(), ui_scale_percent)
        );
    }
}

#[test]
fn collapsed_inline_hunk_headers_use_neutral_colors() {
    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        let neutral = collapsed_hunk_header_bg(theme);

        assert_eq!(
            collapsed_inline_hunk_bg(
                theme,
                Some(collapsed_hunk(true, false)),
                CollapsedDiffExpansionKind::Up,
            ),
            neutral
        );
        assert_eq!(
            collapsed_inline_hunk_bg(
                theme,
                Some(collapsed_hunk(false, true)),
                CollapsedDiffExpansionKind::Up,
            ),
            neutral
        );
        assert_eq!(
            collapsed_inline_hunk_bg(
                theme,
                Some(collapsed_hunk(true, true)),
                CollapsedDiffExpansionKind::Up,
            ),
            neutral
        );
        assert_eq!(
            collapsed_inline_hunk_bg(
                theme,
                Some(collapsed_hunk(true, true)),
                CollapsedDiffExpansionKind::Both,
            ),
            neutral
        );
        assert_eq!(
            collapsed_inline_hunk_bg(
                theme,
                Some(collapsed_hunk(true, true)),
                CollapsedDiffExpansionKind::Short,
            ),
            neutral
        );
        assert_eq!(
            collapsed_inline_hunk_bg(
                theme,
                Some(collapsed_hunk(true, true)),
                CollapsedDiffExpansionKind::Down,
            ),
            neutral
        );
        assert_eq!(
            collapsed_inline_hunk_bg(theme, None, CollapsedDiffExpansionKind::Up),
            neutral
        );
        assert_eq!(
            collapsed_inline_hunk_fg(theme, Some(collapsed_hunk(true, false))),
            theme.colors.foreground.secondary
        );
        assert_eq!(
            collapsed_inline_hunk_fg(theme, Some(collapsed_hunk(false, true))),
            theme.colors.foreground.secondary
        );
        assert_eq!(
            collapsed_inline_hunk_fg(theme, Some(collapsed_hunk(true, true))),
            theme.colors.foreground.secondary
        );
        assert_eq!(
            collapsed_inline_hunk_fg(theme, None),
            theme.colors.foreground.secondary
        );
    }
}

#[test]
fn collapsed_split_hunk_headers_use_neutral_colors_for_both_sides() {
    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        let neutral = collapsed_hunk_header_bg(theme);

        assert_eq!(
            collapsed_split_hunk_bg(
                theme,
                Some(collapsed_hunk(true, false)),
                PatchSplitColumn::Left,
            ),
            neutral
        );
        assert_eq!(
            collapsed_split_hunk_bg(
                theme,
                Some(collapsed_hunk(true, false)),
                PatchSplitColumn::Right,
            ),
            neutral
        );
        assert_eq!(
            collapsed_split_hunk_bg(
                theme,
                Some(collapsed_hunk(false, true)),
                PatchSplitColumn::Left,
            ),
            neutral
        );
        assert_eq!(
            collapsed_split_hunk_bg(
                theme,
                Some(collapsed_hunk(false, true)),
                PatchSplitColumn::Right,
            ),
            neutral
        );
        assert_eq!(
            collapsed_split_hunk_bg(
                theme,
                Some(collapsed_hunk(true, true)),
                PatchSplitColumn::Left,
            ),
            neutral
        );
        assert_eq!(
            collapsed_split_hunk_bg(
                theme,
                Some(collapsed_hunk(true, true)),
                PatchSplitColumn::Right,
            ),
            neutral
        );
        assert_eq!(
            collapsed_split_hunk_bg(theme, None, PatchSplitColumn::Left),
            neutral
        );
        assert_eq!(
            collapsed_split_hunk_bg(theme, None, PatchSplitColumn::Right),
            neutral
        );
        assert_eq!(
            collapsed_split_hunk_fg(theme, PatchSplitColumn::Left),
            theme.colors.foreground.secondary
        );
        assert_eq!(
            collapsed_split_hunk_fg(theme, PatchSplitColumn::Right),
            theme.colors.foreground.secondary
        );
    }
}
