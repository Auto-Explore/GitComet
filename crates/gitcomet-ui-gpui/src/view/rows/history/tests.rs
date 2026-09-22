use super::{
    DiffSearchMatchEmphasis, MarkdownChangeHint, MarkdownInlineStyle, MarkdownPreviewImageSource,
    MarkdownPreviewPictureSizes, MarkdownPreviewRow, MarkdownPreviewRowKind,
    MarkdownRemoteImageAccess, build_cached_diff_styled_text, history_message_text_left_px,
    history_scope_shows_graph_color_marker, history_worktree_node_color_ix,
    markdown_preview_alert_title_label, markdown_preview_code_background,
    markdown_preview_expanded_slice_range, markdown_preview_image_source,
    markdown_preview_inline_highlight, markdown_preview_no_picture_sizes,
    markdown_preview_picture_skeleton, markdown_preview_row_background,
    markdown_preview_row_height, markdown_preview_row_horizontal_padding,
    markdown_preview_row_layout, markdown_preview_row_marker, markdown_preview_row_styled_text,
    markdown_preview_row_typography, worktree_preview_apply_query_overlay,
};
use crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY;
use crate::view::markdown_preview::MarkdownInlineSpan;
use crate::view::panes::main::diff_search::{DiffSearchMatcher, DiffSearchOptions};
use crate::view::rows::diff_text::DIFF_WRAP_TAB_EXPANDED_COLUMNS;
use crate::view::{AppTheme, DateTimeFormat, Timezone, format_datetime, format_datetime_utc};
use crate::view::{
    HISTORY_COL_HANDLE_PX, HISTORY_MESSAGE_BORDER_GAP_PX, HISTORY_MESSAGE_BORDER_W_PX,
};
use gitcomet_core::domain::LogScope;
use gpui::{FontWeight, SharedString, px};
use palette::IntoColor;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

fn markdown_row(kind: MarkdownPreviewRowKind) -> MarkdownPreviewRow {
    MarkdownPreviewRow {
        kind,
        text: SharedString::from("text"),
        inline_spans: Arc::new(Vec::new()),
        code_language: None,
        code_block_horizontal_scroll_hint: false,
        source_line_range: 0..1,
        change_hint: MarkdownChangeHint::None,
        indent_level: 1,
        blockquote_level: 0,
        footnote_label: None,
        alert_kind: None,
        starts_alert: false,
        image: None,
        inline_images: Arc::from(Vec::new()),
        styled_text_cache: Default::default(),
        measured_width_px: Default::default(),
        table: None,
    }
}

#[test]
fn worktree_preview_query_overlay_honors_search_options_for_cached_rows() {
    let theme = AppTheme::gitcomet_dark();
    let base = build_cached_diff_styled_text(
        theme,
        "Render render cat concat cat",
        &[],
        "",
        None,
        super::DiffSyntaxMode::Auto,
        None,
    );

    let case_sensitive_options = DiffSearchOptions {
        match_case: true,
        ..Default::default()
    };
    let case_sensitive_matcher = DiffSearchMatcher::new("render", case_sensitive_options);
    let case_sensitive = worktree_preview_apply_query_overlay(
        theme,
        base.clone(),
        Some(&case_sensitive_matcher),
        DiffSearchMatchEmphasis::Other,
    );
    let case_sensitive_ranges: Vec<_> = case_sensitive
        .highlights
        .iter()
        .map(|(range, _)| range.clone())
        .collect();
    assert_eq!(case_sensitive_ranges, vec![7..13]);

    let whole_word_options = DiffSearchOptions {
        whole_word: true,
        ..Default::default()
    };
    let whole_word_matcher = DiffSearchMatcher::new("cat", whole_word_options);
    let whole_word = worktree_preview_apply_query_overlay(
        theme,
        base.clone(),
        Some(&whole_word_matcher),
        DiffSearchMatchEmphasis::Other,
    );
    let whole_word_ranges: Vec<_> = whole_word
        .highlights
        .iter()
        .map(|(range, _)| range.clone())
        .collect();
    assert_eq!(whole_word_ranges, vec![14..17, 25..28]);

    let regex_options = DiffSearchOptions {
        regex: true,
        ..Default::default()
    };
    let regex_matcher = DiffSearchMatcher::new(r"r.n.e.", regex_options);
    let regex = worktree_preview_apply_query_overlay(
        theme,
        base,
        Some(&regex_matcher),
        DiffSearchMatchEmphasis::Other,
    );
    let regex_ranges: Vec<_> = regex
        .highlights
        .iter()
        .map(|(range, _)| range.clone())
        .collect();
    assert_eq!(regex_ranges, vec![0..6, 7..13]);
}

/// The working-tree row borrows the lane colour *index* of the first commit
/// so its connector can be washed like any other lane, rather than taking a
/// resolved colour it could no longer compare against the selection.
/// The commit rows paint their text on a canvas and the two
/// uncommitted-changes rows lay theirs out as elements, so the offset they
/// agree on has to come from one place — otherwise the message column steps
/// sideways at every synthetic row.
#[test]
fn the_message_text_clears_the_lane_border_by_a_fixed_gap() {
    assert_eq!(
        history_message_text_left_px(true),
        HISTORY_MESSAGE_BORDER_W_PX + HISTORY_MESSAGE_BORDER_GAP_PX
    );
    assert!(
        history_message_text_left_px(true) > HISTORY_MESSAGE_BORDER_W_PX,
        "text that starts inside the border reads as touching it"
    );
}

/// Without the border there is nothing to clear, so the cell's own padding
/// applies and the text does not jump left when the marker is off.
#[test]
fn the_message_text_falls_back_to_the_cell_padding_without_a_border() {
    assert_eq!(
        history_message_text_left_px(false),
        HISTORY_COL_HANDLE_PX / 2.0
    );
}

#[test]
fn history_worktree_node_color_falls_back_to_the_primary_lane() {
    assert_eq!(history_worktree_node_color_ix(None), 0);
}

#[test]
fn history_graph_color_marker_is_shown_for_all_non_first_parent_modes() {
    assert!(history_scope_shows_graph_color_marker(
        LogScope::FullReachable
    ));
    assert!(!history_scope_shows_graph_color_marker(
        LogScope::FirstParent
    ));
    assert!(history_scope_shows_graph_color_marker(LogScope::NoMerges));
    assert!(history_scope_shows_graph_color_marker(LogScope::MergesOnly));
    assert!(history_scope_shows_graph_color_marker(
        LogScope::AllBranches
    ));
}

#[test]
fn commit_date_formats_as_yyyy_mm_dd_utc() {
    assert_eq!(
        format_datetime_utc(UNIX_EPOCH, DateTimeFormat::YmdHm),
        "1970-01-01 00:00 UTC"
    );
    assert_eq!(
        format_datetime_utc(
            UNIX_EPOCH + Duration::from_secs(86_400),
            DateTimeFormat::YmdHm
        ),
        "1970-01-02 00:00 UTC"
    );
    assert_eq!(
        format_datetime_utc(
            UNIX_EPOCH - Duration::from_secs(86_400),
            DateTimeFormat::YmdHm
        ),
        "1969-12-31 00:00 UTC"
    );

    // 2000-02-29 12:34:56 UTC
    assert_eq!(
        format_datetime_utc(
            UNIX_EPOCH + Duration::from_secs(951_782_400 + 12 * 3600 + 34 * 60 + 56),
            DateTimeFormat::YmdHms
        ),
        "2000-02-29 12:34:56 UTC"
    );
}

#[test]
fn format_datetime_with_timezone_offset() {
    // UTC+5:30 (19800 seconds)
    let tz = Timezone::Fixed(19800);
    assert_eq!(
        format_datetime(UNIX_EPOCH, DateTimeFormat::YmdHm, tz, true),
        "1970-01-01 05:30 UTC+5:30"
    );

    // UTC-5
    let tz_neg = Timezone::Fixed(-18000);
    assert_eq!(
        format_datetime(
            UNIX_EPOCH + Duration::from_secs(86_400),
            DateTimeFormat::YmdHm,
            tz_neg,
            true,
        ),
        "1970-01-01 19:00 UTC\u{2212}5"
    );
}

#[test]
fn format_datetime_can_hide_timezone_label() {
    let tz = Timezone::Fixed(7200);
    assert_eq!(
        format_datetime(UNIX_EPOCH, DateTimeFormat::YmdHm, tz, false),
        "1970-01-01 02:00"
    );
}

#[test]
fn timezone_key_round_trips() {
    for tz in Timezone::all() {
        let key = tz.key();
        let parsed = Timezone::from_key(&key);
        assert_eq!(parsed, Some(*tz), "round-trip failed for {key}");
    }
}

#[test]
fn worktree_preview_renderer_avoids_full_document_prepare_calls() {
    let source = include_str!("worktree_preview.rs");
    let render_start = source
        .find("fn render_worktree_preview_rows")
        .expect("render_worktree_preview_rows should exist");
    let render_source = &source[render_start..];

    assert!(
        !render_source.contains("prepare_diff_syntax_document("),
        "row renderer should not build prepared syntax documents"
    );
    assert!(
        !render_source.contains("prepare_diff_syntax_document_with_budget_reuse("),
        "row renderer should not run full-document parse prep"
    );
}

#[test]
fn markdown_preview_heading_typography_scales_above_body_text() {
    let theme = AppTheme::gitcomet_light();
    let paragraph = MarkdownPreviewRow {
        kind: MarkdownPreviewRowKind::Paragraph,
        text: SharedString::from("body"),
        inline_spans: Arc::new(Vec::new()),
        code_language: None,
        code_block_horizontal_scroll_hint: false,
        source_line_range: 0..1,
        change_hint: MarkdownChangeHint::None,
        indent_level: 1,
        blockquote_level: 0,
        footnote_label: None,
        alert_kind: None,
        starts_alert: false,
        image: None,
        inline_images: Arc::from(Vec::new()),
        styled_text_cache: Default::default(),
        measured_width_px: Default::default(),
        table: None,
    };
    let h1 = MarkdownPreviewRow {
        kind: MarkdownPreviewRowKind::Heading { level: 1 },
        ..paragraph.clone()
    };
    let h2 = MarkdownPreviewRow {
        kind: MarkdownPreviewRowKind::Heading { level: 2 },
        ..paragraph.clone()
    };
    let h6 = MarkdownPreviewRow {
        kind: MarkdownPreviewRowKind::Heading { level: 6 },
        ..paragraph.clone()
    };

    let editor_font_family: SharedString = EDITOR_MONOSPACE_FONT_FAMILY.into();
    let body_typography = markdown_preview_row_typography(
        theme,
        &paragraph,
        &editor_font_family,
        crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
    );
    let h1_typography = markdown_preview_row_typography(
        theme,
        &h1,
        &editor_font_family,
        crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
    );
    let h2_typography = markdown_preview_row_typography(
        theme,
        &h2,
        &editor_font_family,
        crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
    );
    let h6_typography = markdown_preview_row_typography(
        theme,
        &h6,
        &editor_font_family,
        crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
    );

    assert!(h1_typography.font_size > h2_typography.font_size);
    assert!(h2_typography.font_size > body_typography.font_size);
    assert!(h6_typography.font_size > body_typography.font_size);
    assert_eq!(h1_typography.font_weight, Some(FontWeight::BOLD));
    assert_eq!(h2_typography.font_weight, Some(FontWeight::BOLD));
    assert_eq!(h6_typography.font_weight, Some(FontWeight::BOLD));
}

#[test]
fn markdown_preview_list_rows_match_body_line_height_and_keep_tighter_layout() {
    let theme = AppTheme::gitcomet_light();
    let paragraph = markdown_row(MarkdownPreviewRowKind::Paragraph);
    let list_item = markdown_row(MarkdownPreviewRowKind::ListItem { number: None });

    let editor_font_family: SharedString = EDITOR_MONOSPACE_FONT_FAMILY.into();
    let paragraph_typography = markdown_preview_row_typography(
        theme,
        &paragraph,
        &editor_font_family,
        crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
    );
    let list_typography = markdown_preview_row_typography(
        theme,
        &list_item,
        &editor_font_family,
        crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
    );
    let paragraph_layout =
        markdown_preview_row_layout(&paragraph, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);
    let list_layout =
        markdown_preview_row_layout(&list_item, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);

    assert_eq!(
        list_typography.line_height,
        paragraph_typography.line_height
    );
    assert!(paragraph_layout.bottom_inset_px > list_layout.bottom_inset_px);
}

#[test]
fn markdown_preview_details_summary_rows_are_bold_and_marked() {
    let theme = AppTheme::gitcomet_light();
    let row = markdown_row(MarkdownPreviewRowKind::DetailsSummary);

    let editor_font_family: SharedString = EDITOR_MONOSPACE_FONT_FAMILY.into();
    let typography = markdown_preview_row_typography(
        theme,
        &row,
        &editor_font_family,
        crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
    );

    assert_eq!(typography.font_weight, Some(FontWeight::BOLD));
    assert_eq!(
        markdown_preview_row_marker(&row)
            .as_ref()
            .map(SharedString::as_ref),
        Some("v")
    );
}

#[test]
fn markdown_preview_code_rows_do_not_reserve_bottom_space_for_local_scrollbar() {
    let first_row = markdown_row(MarkdownPreviewRowKind::CodeLine {
        is_first: true,
        is_last: false,
    });
    let last_row = markdown_row(MarkdownPreviewRowKind::CodeLine {
        is_first: false,
        is_last: true,
    });

    let first_layout =
        markdown_preview_row_layout(&first_row, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);
    let last_layout =
        markdown_preview_row_layout(&last_row, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);

    assert_eq!(first_layout.top_inset_px, 5.0);
    assert_eq!(last_layout.bottom_inset_px, 5.0);
}

#[test]
fn markdown_preview_nested_code_rows_keep_small_outer_edge_gap() {
    let mut row = markdown_row(MarkdownPreviewRowKind::CodeLine {
        is_first: true,
        is_last: false,
    });
    row.indent_level = 3;

    let padding =
        markdown_preview_row_horizontal_padding(&row, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);

    assert_eq!(padding.left_px, super::MARKDOWN_PREVIEW_BOXED_EDGE_GAP_PX);
    assert_eq!(padding.right_px, super::MARKDOWN_PREVIEW_BOXED_EDGE_GAP_PX);
}

#[test]
fn markdown_preview_row_marker_preserves_ordered_item_number() {
    let row = MarkdownPreviewRow {
        kind: MarkdownPreviewRowKind::ListItem { number: Some(7) },
        text: SharedString::from("item"),
        inline_spans: Arc::new(Vec::new()),
        code_language: None,
        code_block_horizontal_scroll_hint: false,
        source_line_range: 0..1,
        change_hint: MarkdownChangeHint::None,
        indent_level: 1,
        blockquote_level: 0,
        footnote_label: None,
        alert_kind: None,
        starts_alert: false,
        image: None,
        inline_images: Arc::from(Vec::new()),
        styled_text_cache: Default::default(),
        measured_width_px: Default::default(),
        table: None,
    };

    assert_eq!(
        markdown_preview_row_marker(&row)
            .as_ref()
            .map(SharedString::as_ref),
        Some("7.")
    );
}

#[test]
fn markdown_preview_row_marker_is_none_for_blockquotes_without_list_items() {
    let row = MarkdownPreviewRow {
        kind: MarkdownPreviewRowKind::BlockquoteLine,
        text: SharedString::from("quote"),
        inline_spans: Arc::new(Vec::new()),
        code_language: None,
        code_block_horizontal_scroll_hint: false,
        source_line_range: 0..1,
        change_hint: MarkdownChangeHint::None,
        indent_level: 1,
        blockquote_level: 2,
        footnote_label: None,
        alert_kind: None,
        starts_alert: false,
        image: None,
        inline_images: Arc::from(Vec::new()),
        styled_text_cache: Default::default(),
        measured_width_px: Default::default(),
        table: None,
    };

    assert_eq!(markdown_preview_row_marker(&row), None);
}

#[test]
fn markdown_preview_row_marker_uses_footnote_label_when_present() {
    let row = MarkdownPreviewRow {
        kind: MarkdownPreviewRowKind::Paragraph,
        text: SharedString::from("reference"),
        inline_spans: Arc::new(Vec::new()),
        code_language: None,
        code_block_horizontal_scroll_hint: false,
        source_line_range: 0..1,
        change_hint: MarkdownChangeHint::None,
        indent_level: 1,
        blockquote_level: 0,
        footnote_label: Some("1".into()),
        alert_kind: None,
        starts_alert: false,
        image: None,
        inline_images: Arc::from(Vec::new()),
        styled_text_cache: Default::default(),
        measured_width_px: Default::default(),
        table: None,
    };

    assert_eq!(
        markdown_preview_row_marker(&row)
            .as_ref()
            .map(SharedString::as_ref),
        Some("[^1]:")
    );
}

#[test]
fn markdown_preview_row_marker_returns_unordered_bullet_inside_blockquote() {
    let row = MarkdownPreviewRow {
        kind: MarkdownPreviewRowKind::ListItem { number: None },
        text: SharedString::from("item"),
        inline_spans: Arc::new(Vec::new()),
        code_language: None,
        code_block_horizontal_scroll_hint: false,
        source_line_range: 0..1,
        change_hint: MarkdownChangeHint::None,
        indent_level: 1,
        blockquote_level: 1,
        footnote_label: None,
        alert_kind: None,
        starts_alert: false,
        image: None,
        inline_images: Arc::from(Vec::new()),
        styled_text_cache: Default::default(),
        measured_width_px: Default::default(),
        table: None,
    };

    assert_eq!(
        markdown_preview_row_marker(&row)
            .as_ref()
            .map(SharedString::as_ref),
        Some("•")
    );
}

#[test]
fn markdown_preview_alert_title_label_requires_alert_start_row() {
    for (kind, label) in [
        (super::MarkdownAlertKind::Note, "NOTE"),
        (super::MarkdownAlertKind::Tip, "TIP"),
        (super::MarkdownAlertKind::Important, "IMPORTANT"),
        (super::MarkdownAlertKind::Warning, "WARNING"),
        (super::MarkdownAlertKind::Caution, "CAUTION"),
    ] {
        let mut row = markdown_row(MarkdownPreviewRowKind::BlockquoteLine);
        row.alert_kind = Some(kind);
        row.starts_alert = true;
        assert_eq!(markdown_preview_alert_title_label(&row), Some(label));

        row.starts_alert = false;
        assert_eq!(markdown_preview_alert_title_label(&row), None);
    }

    let mut row = markdown_row(MarkdownPreviewRowKind::BlockquoteLine);
    row.starts_alert = true;
    assert_eq!(markdown_preview_alert_title_label(&row), None);
}

#[test]
fn markdown_preview_row_background_change_hints_override_alert_and_fallback_states() {
    let theme = AppTheme::gitcomet_light();

    let mut added_row = markdown_row(MarkdownPreviewRowKind::Paragraph);
    added_row.change_hint = MarkdownChangeHint::Added;

    let mut added_alert_row = added_row.clone();
    added_alert_row.alert_kind = Some(super::MarkdownAlertKind::Warning);
    assert_eq!(
        markdown_preview_row_background(theme, &added_alert_row),
        markdown_preview_row_background(theme, &added_row)
    );

    let mut removed_row = markdown_row(MarkdownPreviewRowKind::Paragraph);
    removed_row.change_hint = MarkdownChangeHint::Removed;

    let mut removed_fallback_row = removed_row.clone();
    removed_fallback_row.kind = MarkdownPreviewRowKind::PlainFallback;
    assert_eq!(
        markdown_preview_row_background(theme, &removed_fallback_row),
        markdown_preview_row_background(theme, &removed_row)
    );
}

#[test]
fn markdown_preview_row_background_uses_alert_and_fallback_only_when_unchanged() {
    let theme = AppTheme::gitcomet_dark();

    let plain_row = markdown_row(MarkdownPreviewRowKind::Paragraph);
    assert_eq!(markdown_preview_row_background(theme, &plain_row), None);

    let mut alert_row = plain_row.clone();
    alert_row.alert_kind = Some(super::MarkdownAlertKind::Tip);

    let fallback_row = markdown_row(MarkdownPreviewRowKind::PlainFallback);
    let alert_bg = markdown_preview_row_background(theme, &alert_row);
    let fallback_bg = markdown_preview_row_background(theme, &fallback_row);

    assert!(alert_bg.is_some());
    assert!(fallback_bg.is_some());
    assert_ne!(alert_bg, fallback_bg);
}

#[test]
fn markdown_preview_row_styled_text_maps_inline_styles_and_skips_normal_spans() {
    let theme = AppTheme::gitcomet_light();

    let mut row = markdown_row(MarkdownPreviewRowKind::Paragraph);
    row.text = SharedString::from("link under strike plain");
    row.inline_spans = Arc::new(vec![
        MarkdownInlineSpan {
            byte_range: 0..4,
            style: MarkdownInlineStyle::Link,
            link_url: None,
        },
        MarkdownInlineSpan {
            byte_range: 5..10,
            style: MarkdownInlineStyle::Underline,
            link_url: None,
        },
        MarkdownInlineSpan {
            byte_range: 11..17,
            style: MarkdownInlineStyle::Strikethrough,
            link_url: None,
        },
        MarkdownInlineSpan {
            byte_range: 18..23,
            style: MarkdownInlineStyle::Normal,
            link_url: None,
        },
    ]);

    let styled = markdown_preview_row_styled_text(theme, &row);
    let highlights = styled.highlights.as_ref();

    assert_eq!(styled.text.as_ref(), "link under strike plain");
    assert_eq!(highlights.len(), 3);
    assert_eq!(highlights[0].0, 0..4);
    assert_eq!(
        highlights[0].1,
        markdown_preview_inline_highlight(theme, MarkdownInlineStyle::Link)
    );
    assert_eq!(highlights[1].0, 5..10);
    assert_eq!(
        highlights[1].1,
        markdown_preview_inline_highlight(theme, MarkdownInlineStyle::Underline)
    );
    assert_eq!(highlights[2].0, 11..17);
    assert_eq!(
        highlights[2].1,
        markdown_preview_inline_highlight(theme, MarkdownInlineStyle::Strikethrough)
    );
}

#[test]
fn amber_inline_code_spans_use_the_neutral_code_surface() {
    let theme = AppTheme::from_key(crate::theme::AMBER_DARK_THEME_KEY)
        .expect("Amber Dark theme should load");
    let highlight = markdown_preview_inline_highlight(theme, MarkdownInlineStyle::Code);

    assert_eq!(
        highlight.background_color,
        Some(markdown_preview_code_background(theme).into_color())
    );
    assert_ne!(
        highlight.background_color,
        Some(
            crate::theme::with_alpha(theme.colors.interaction.selected_background, 0.75)
                .into_color()
        ),
        "inline backticks must not use Amber's orange selection color"
    );
}

#[test]
fn wrapped_slices_map_onto_the_tab_expanded_painted_text() {
    // Wrap ranges are measured on `row.text`, where a tab is one byte, but
    // the painted text expands each tab to four spaces. Slicing the
    // painted text with raw offsets shifted every wrapped row and dropped
    // the tail of the line.
    let raw = "\tab\tcd";
    let expanded_len = raw.len() + raw.matches('\t').count() * (DIFF_WRAP_TAB_EXPANDED_COLUMNS - 1);

    // "\tab" -> "    ab", "\tcd" -> "    cd"
    assert_eq!(
        markdown_preview_expanded_slice_range(raw, expanded_len, &(0..3)),
        0..6
    );
    assert_eq!(
        markdown_preview_expanded_slice_range(raw, expanded_len, &(3..raw.len())),
        6..expanded_len
    );
    // A row without tabs keeps its ranges untouched.
    assert_eq!(
        markdown_preview_expanded_slice_range("abcd", 4, &(1..3)),
        1..3
    );
}

#[test]
fn image_paths_resolve_only_inside_the_documents_own_directory() {
    let dir = std::env::temp_dir().join(format!(
        "gitcomet_md_image_path_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let nested = dir.join("assets");
    std::fs::create_dir_all(&nested).expect("create fixture dirs");
    let image = nested.join("shot.png");
    std::fs::write(&image, b"not really a png").expect("write fixture image");
    let outside = dir.parent().expect("temp dir parent").join("outside.png");
    std::fs::write(&outside, b"not really a png").expect("write outside fixture");

    let resolve = |source: &str| markdown_preview_image_source(Some(dir.as_path()), source);
    let file = |path: &std::path::Path| Some(MarkdownPreviewImageSource::File(path.to_owned()));
    let remote = |url: &str| {
        Some(MarkdownPreviewImageSource::Remote(SharedString::from(
            url.to_owned(),
        )))
    };

    assert_eq!(resolve("assets/shot.png"), file(&image));
    assert_eq!(resolve("./assets/shot.png"), file(&image));
    // Query and fragment suffixes are common in markdown image sources and
    // are not part of the file name.
    assert_eq!(resolve("assets/shot.png?v=2"), file(&image));
    assert_eq!(resolve("assets/shot.png#frag"), file(&image));

    // Badges and hosted screenshots resolve to the URL, query string and
    // all — that is what identifies the image.
    assert_eq!(
        resolve("https://img.shields.io/badge/a-b.svg?logo=x"),
        remote("https://img.shields.io/badge/a-b.svg?logo=x")
    );
    assert_eq!(
        resolve("http://example.com/a.png"),
        remote("http://example.com/a.png")
    );
    // Remote sources resolve without a base directory, since nothing is
    // resolved against the document's location.
    assert_eq!(
        markdown_preview_image_source(None, "https://example.com/a.png"),
        remote("https://example.com/a.png")
    );

    // A file that exists but sits outside the document's tree is refused,
    // so document content cannot aim the preview at arbitrary files.
    assert_eq!(resolve("../outside.png"), None);
    // Schemes a preview has no business dereferencing.
    assert_eq!(resolve("data:image/png;base64,AAAA"), None);
    assert_eq!(resolve("file:///etc/passwd"), None);
    assert_eq!(resolve("javascript:alert(1)"), None);
    // Missing files, empty sources, and a missing base directory resolve
    // to nothing.
    assert_eq!(resolve("assets/absent.png"), None);
    assert_eq!(resolve("   "), None);
    assert_eq!(markdown_preview_image_source(None, "assets/shot.png"), None);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&outside);
}

#[test]
fn markdown_remote_image_access_requires_exact_url_approval_in_ask_mode() {
    let approved = [SharedString::from("https://example.com/image.png")]
        .into_iter()
        .collect();
    let access = MarkdownRemoteImageAccess {
        policy: crate::view::RemoteMarkdownImagePolicy::AskBeforeLoading,
        approved_urls: Arc::new(approved),
        approval_view: None,
    };

    assert!(access.permits(&SharedString::from("https://example.com/image.png")));
    assert!(!access.permits(&SharedString::from("https://example.com/other.png")));

    let url = SharedString::from("https://example.com/other.png");
    assert!(MarkdownRemoteImageAccess::default().permits(&url));
    assert!(
        !MarkdownRemoteImageAccess {
            policy: crate::view::RemoteMarkdownImagePolicy::NeverLoad,
            approved_urls: access.approved_urls.clone(),
            approval_view: None,
        }
        .permits(&url)
    );
}

/// A picture row carrying `source`, and whatever size the document declared.
fn picture_row(source: &str, width_px: Option<u32>, height_px: Option<u32>) -> MarkdownPreviewRow {
    let mut row = markdown_row(MarkdownPreviewRowKind::Image {
        slice_ix: 0,
        slice_count: 8,
    });
    row.image = Some(Arc::new(crate::view::markdown_preview::MarkdownImage {
        source: SharedString::from(source.to_owned()),
        width_px,
        height_px,
    }));
    row
}

fn measured(source: &str, width: u32, height: u32) -> MarkdownPreviewPictureSizes {
    Arc::new(
        [(SharedString::from(source.to_owned()), (width, height))]
            .into_iter()
            .collect(),
    )
}

#[test]
fn a_skeleton_holds_the_box_the_picture_will_fill() {
    // The whole point of measuring a picture's header is that the space it
    // is going to take is reserved before it has been decoded, so the
    // document does not jump when it arrives.
    let empty = markdown_preview_no_picture_sizes();

    // Read from the file: the picture's own pixels, which is what an
    // undeclared picture lays out at.
    let skeleton = markdown_preview_picture_skeleton(
        AppTheme::gitcomet_dark(),
        &picture_row("demo.gif", None, None),
        100,
        &measured("demo.gif", 1280, 720),
    );
    assert_eq!(skeleton.width, Some(px(1280.0)));
    assert_eq!(skeleton.aspect_ratio, Some(1280.0 / 720.0));

    // A declared size wins, and scales with the UI the way the picture will.
    let skeleton = markdown_preview_picture_skeleton(
        AppTheme::gitcomet_dark(),
        &picture_row("demo.gif", Some(200), Some(100)),
        200,
        &measured("demo.gif", 1280, 720),
    );
    assert_eq!(skeleton.width, Some(px(400.0)));
    assert_eq!(skeleton.aspect_ratio, Some(2.0));

    // Nothing to go on: fall back to the rows the parser set aside, which
    // is all the row grid ever had.
    let skeleton = markdown_preview_picture_skeleton(
        AppTheme::gitcomet_dark(),
        &picture_row("demo.gif", None, None),
        100,
        empty,
    );
    assert_eq!(skeleton.width, None);
    assert_eq!(skeleton.aspect_ratio, None);
    assert_eq!(
        skeleton.reserved_height,
        markdown_preview_row_height(AppTheme::gitcomet_dark(), 100) * 8.0
    );
}

#[test]
fn a_height_only_skeleton_scales_the_measured_width_with_the_picture() {
    let skeleton = markdown_preview_picture_skeleton(
        AppTheme::gitcomet_dark(),
        &picture_row("wide.gif", None, Some(60)),
        100,
        &measured("wide.gif", 1280, 720),
    );

    let expected_ratio = 1280.0 / 720.0;
    let expected_width = px(60.0 * expected_ratio);
    assert_eq!(skeleton.aspect_ratio, Some(expected_ratio));
    assert!(
        (skeleton.width.expect("measured width") - expected_width).abs() <= px(0.01),
        "the placeholder must reserve the same scaled width as the height-only decoded image"
    );
}

#[test]
fn a_picture_is_named_the_same_way_wherever_it_is_asked_about() {
    // The element that draws a picture and the pane waiting to hear that it
    // decoded look it up in the same cache, so both have to arrive at the
    // key `gpui` filed it under. Building the element one way and the key
    // another would leave the pane waiting on an entry nobody writes.
    let path = std::path::PathBuf::from("assets").join("shot.png");
    assert_eq!(
        MarkdownPreviewImageSource::File(path.clone()).to_resource(),
        gpui::Resource::Path(path.as_path().into())
    );
    assert_eq!(
        MarkdownPreviewImageSource::Remote(SharedString::from("https://example.com/a.png"))
            .to_resource(),
        gpui::Resource::Uri(gpui::SharedUri::from(
            "https://example.com/a.png".to_owned()
        ))
    );
}

#[test]
fn heading_rows_are_inset_evenly_above_and_below() {
    // Headings used to carry more space below than above, so the text rode
    // high in its row instead of sitting centred in the break.
    for level in 1..=6u8 {
        let row = markdown_row(MarkdownPreviewRowKind::Heading { level });
        let layout = markdown_preview_row_layout(&row, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);
        assert_eq!(
            layout.top_inset_px, layout.bottom_inset_px,
            "h{level} should be inset evenly: {layout:?}"
        );
    }
}

#[test]
fn markdown_preview_row_styled_text_repairs_spans_that_split_a_multibyte_char() {
    // A span pointing inside a multi-byte character used to reach `gpui`
    // as a text run whose length splits that character, aborting the
    // process inside `str::split_at` while shaping the line.
    let theme = AppTheme::gitcomet_light();

    let mut row = markdown_row(MarkdownPreviewRowKind::Paragraph);
    row.text = SharedString::from("— dash —");
    row.inline_spans = Arc::new(vec![
        MarkdownInlineSpan {
            byte_range: 0..1,
            style: MarkdownInlineStyle::Bold,
            link_url: None,
        },
        MarkdownInlineSpan {
            byte_range: 6..9,
            style: MarkdownInlineStyle::Italic,
            link_url: None,
        },
    ]);

    let styled = markdown_preview_row_styled_text(theme, &row);
    let text = styled.text.as_ref();

    for (range, _) in styled.highlights.iter() {
        assert!(
            text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
            "highlight {range:?} splits a char in {text:?}"
        );
    }
    assert_eq!(styled.highlights[0].0, 0..3);
}

#[test]
fn markdown_preview_table_rows_use_monospace_typography_and_only_headers_are_bold() {
    let theme = AppTheme::gitcomet_light();
    let header = markdown_row(MarkdownPreviewRowKind::TableRow { is_header: true });
    let body = markdown_row(MarkdownPreviewRowKind::TableRow { is_header: false });

    let editor_font_family: SharedString = EDITOR_MONOSPACE_FONT_FAMILY.into();
    let header_typography = markdown_preview_row_typography(
        theme,
        &header,
        &editor_font_family,
        crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
    );
    let body_typography = markdown_preview_row_typography(
        theme,
        &body,
        &editor_font_family,
        crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
    );

    assert_eq!(
        header_typography
            .font_family
            .as_ref()
            .map(SharedString::as_ref),
        Some(EDITOR_MONOSPACE_FONT_FAMILY)
    );
    assert_eq!(
        body_typography
            .font_family
            .as_ref()
            .map(SharedString::as_ref),
        Some(EDITOR_MONOSPACE_FONT_FAMILY)
    );
    assert_eq!(header_typography.font_weight, Some(FontWeight::BOLD));
    assert_eq!(body_typography.font_weight, None);
    assert_eq!(header_typography.font_size, body_typography.font_size);
    assert_eq!(header_typography.line_height, body_typography.line_height);
}

#[test]
fn markdown_preview_code_rows_reuse_diff_syntax_highlighting() {
    let theme = AppTheme::gitcomet_dark();
    let row = MarkdownPreviewRow {
        kind: MarkdownPreviewRowKind::CodeLine {
            is_first: true,
            is_last: true,
        },
        text: SharedString::from("fn\tmain() { let x = 1; }"),
        inline_spans: Arc::new(Vec::new()),
        code_language: Some(crate::view::rows::DiffSyntaxLanguage::Rust),
        code_block_horizontal_scroll_hint: false,
        source_line_range: 0..1,
        change_hint: MarkdownChangeHint::None,
        indent_level: 1,
        blockquote_level: 0,
        footnote_label: None,
        alert_kind: None,
        starts_alert: false,
        image: None,
        inline_images: Arc::from(Vec::new()),
        styled_text_cache: Default::default(),
        measured_width_px: Default::default(),
        table: None,
    };

    let dark_highlights = Arc::clone(&markdown_preview_row_styled_text(theme, &row).highlights);
    let dark = markdown_preview_row_styled_text(theme, &row);
    let light = markdown_preview_row_styled_text(AppTheme::gitcomet_light(), &row);

    assert_eq!(dark.text.as_ref(), "fn    main() { let x = 1; }");
    assert!(
        !dark.highlights.is_empty(),
        "code rows should reuse syntax highlights from the diff text renderer"
    );
    assert!(
        Arc::ptr_eq(&dark_highlights, &dark.highlights),
        "same-theme markdown code rows should reuse cached styled text"
    );
    assert!(
        !Arc::ptr_eq(&dark.highlights, &light.highlights),
        "light and dark markdown preview caches should stay separate"
    );
}

#[test]
fn markdown_preview_spacer_rows_have_no_extra_layout_or_background() {
    let theme = AppTheme::gitcomet_light();
    let row = markdown_row(MarkdownPreviewRowKind::Spacer);

    let layout = markdown_preview_row_layout(&row, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);

    assert_eq!(layout.top_inset_px, 0.0);
    assert_eq!(layout.bottom_inset_px, 0.0);
    assert_eq!(markdown_preview_row_background(theme, &row), None);
    assert_eq!(markdown_preview_row_marker(&row), None);
}

#[test]
fn local_markdown_links_resolve_against_the_document_directory() {
    use super::markdown_preview_local_link_path;
    let doc = std::path::Path::new("docs/preview.md");
    let resolve = |destination: &str| markdown_preview_local_link_path(doc, destination);
    let path = |p: &str| Some(std::path::PathBuf::from(p));

    assert_eq!(resolve("./other.md"), path("docs/other.md"));
    assert_eq!(resolve("guide.md"), path("docs/guide.md"));
    assert_eq!(resolve("sub/../guide.md"), path("docs/guide.md"));
    // `../README.md` is how a docs page links to the root.
    assert_eq!(resolve("../README.md"), path("README.md"));
    // A leading slash is repository-root-relative, as GitHub reads it.
    assert_eq!(resolve("/docs/x.md"), path("docs/x.md"));
    // Fragment and query address something inside the file.
    assert_eq!(resolve("other.md#a"), path("docs/other.md"));
    assert_eq!(resolve("other.md?q"), path("docs/other.md"));
    assert_eq!(resolve("other.md#a?q"), path("docs/other.md"));
    assert_eq!(resolve("  other.md "), path("docs/other.md"));
    // Percent escapes are how a space is written in a link.
    assert_eq!(resolve("my%20file.md"), path("docs/my file.md"));
    // A malformed escape is kept as written, which then names no file.
    assert_eq!(resolve("bad%zz.md"), path("docs/bad%zz.md"));
    // Relative to `docs/`, so this climbs to the root and no further.
    assert_eq!(resolve("docs/../../x.md"), path("x.md"));
    // A directory link still says where it points; the pane then reports
    // that it is not a file.
    assert_eq!(resolve("sub/"), path("docs/sub"));

    // Climbing out of the repository, or into `.git`, names nothing.
    for inert in [
        "../../outside.md",
        "/../x.md",
        ".git/config",
        "../.git/HEAD",
        "/.GIT/config",
        ".",
        "./",
        "..",
        "../",
        "/",
        "",
        "%2e%2e/%2e%2e/outside.md",
    ] {
        assert_eq!(resolve(inert), None, "{inert:?} must resolve to nothing");
    }

    // A document at the root has nowhere to climb to.
    let root_doc = std::path::Path::new("README.md");
    assert_eq!(
        markdown_preview_local_link_path(root_doc, "docs/x.md"),
        path("docs/x.md")
    );
    assert_eq!(markdown_preview_local_link_path(root_doc, "../x.md"), None);
}

#[cfg(windows)]
#[test]
fn local_markdown_links_refuse_os_absolute_paths() {
    use super::markdown_preview_local_link_path;
    let doc = std::path::Path::new("docs/preview.md");
    for absolute in [
        r"C:\x.md",
        r"\\srv\share\x.md",
        r"\docs\x.md",
        // A drive prefix hidden behind `./` would replace the document
        // directory once the components are collected into a path.
        "./C:../outside.txt",
        "sub/C:outside.txt",
    ] {
        assert_eq!(
            markdown_preview_local_link_path(doc, absolute),
            None,
            "{absolute:?} is not a repository path"
        );
    }
}

#[test]
fn local_link_percent_escapes_decode_before_the_path_is_walked() {
    use super::markdown_preview_local_link_path;
    let doc = std::path::Path::new("docs/preview.md");
    let resolve = |destination: &str| markdown_preview_local_link_path(doc, destination);
    let path = |p: &str| Some(std::path::PathBuf::from(p));

    // Multi-byte UTF-8 decodes to the character it spells.
    assert_eq!(resolve("%C3%A4iti.md"), path("docs/äiti.md"));
    // Upper and lower case hex both decode.
    assert_eq!(resolve("a%2db.md"), path("docs/a-b.md"));
    assert_eq!(resolve("a%2Db.md"), path("docs/a-b.md"));
    // The fragment is cut before decoding, so an escaped `#` is part of the
    // file name rather than the start of a fragment.
    assert_eq!(resolve("a%23b.md"), path("docs/a#b.md"));
    assert_eq!(resolve("a%3Fb.md"), path("docs/a?b.md"));
    // An escaped leading slash is still repository-root-relative.
    assert_eq!(resolve("%2Fdocs%2Fx.md"), path("docs/x.md"));

    // Anything that does not decode cleanly is kept exactly as written.
    assert_eq!(resolve("a%2.md"), path("docs/a%2.md"));
    assert_eq!(resolve("trailing%"), path("docs/trailing%"));
    assert_eq!(resolve("%FF.md"), path("docs/%FF.md"));

    // Escaped traversal is walked like the plain spelling, so it cannot
    // climb out of the repository or into `.git`.
    assert_eq!(resolve("%2e%2e/%2e%2e/outside.md"), None);
    assert_eq!(resolve("..%2F..%2Foutside.md"), None);
    assert_eq!(resolve("%2Egit/config"), None);
}

#[test]
fn local_link_target_reads_from_the_tree_the_document_came_from() {
    use super::markdown_preview_local_link_target;
    use gitcomet_core::domain::{CommitId, DiffArea, DiffTarget, FileSource};
    use std::path::{Path, PathBuf};

    let workdir = Path::new("/repo");
    let resolve = |target: &DiffTarget, destination: &str| {
        markdown_preview_local_link_target(workdir, target, destination)
    };

    let working_tree = DiffTarget::WorkingTree {
        path: PathBuf::from("docs/preview.md"),
        area: DiffArea::Unstaged,
    };
    assert_eq!(
        resolve(&working_tree, "../README.md"),
        Some((FileSource::WorkingDirectory, PathBuf::from("README.md")))
    );
    // A staged document still links into the working tree it lives in.
    let staged = DiffTarget::WorkingTree {
        path: PathBuf::from("docs/preview.md"),
        area: DiffArea::Staged,
    };
    assert_eq!(
        resolve(&staged, "other.md"),
        Some((FileSource::WorkingDirectory, PathBuf::from("docs/other.md")))
    );

    // A document shown at a commit links into that commit.
    let at_commit = DiffTarget::Commit {
        commit_id: CommitId("deadbeef".into()),
        path: Some(PathBuf::from("docs/preview.md")),
    };
    assert_eq!(
        resolve(&at_commit, "./other.md"),
        Some((
            FileSource::Commit(CommitId("deadbeef".into())),
            PathBuf::from("docs/other.md")
        ))
    );

    // Neither a whole-commit view nor a range has one document to resolve from.
    let whole_commit = DiffTarget::Commit {
        commit_id: CommitId("deadbeef".into()),
        path: None,
    };
    assert_eq!(resolve(&whole_commit, "other.md"), None);
    let range = DiffTarget::CommitRange {
        from_commit_id: CommitId("aaaa".into()),
        to_commit_id: Some(CommitId("bbbb".into())),
        path: Some(PathBuf::from("docs/preview.md")),
    };
    assert_eq!(resolve(&range, "other.md"), None);
    let range_to_worktree = DiffTarget::CommitRange {
        from_commit_id: CommitId("aaaa".into()),
        to_commit_id: None,
        path: Some(PathBuf::from("docs/preview.md")),
    };
    assert_eq!(resolve(&range_to_worktree, "other.md"), None);

    // Absolute document paths are taken relative to the workdir…
    let absolute = DiffTarget::WorkingTree {
        path: workdir.join("docs/preview.md"),
        area: DiffArea::Unstaged,
    };
    assert_eq!(
        resolve(&absolute, "other.md"),
        Some((FileSource::WorkingDirectory, PathBuf::from("docs/other.md")))
    );
    // …and one outside it has no repository to link into.
    let elsewhere = DiffTarget::WorkingTree {
        path: PathBuf::from("/elsewhere/docs/preview.md"),
        area: DiffArea::Unstaged,
    };
    assert_eq!(resolve(&elsewhere, "other.md"), None);

    // A link the path resolver refuses stays refused whatever the source.
    assert_eq!(resolve(&working_tree, "../../outside.md"), None);
    assert_eq!(resolve(&at_commit, ".git/config"), None);
}

#[test]
fn local_link_availability_follows_the_tree_the_link_reads_from() {
    use super::markdown_preview_local_link_missing;
    use gitcomet_core::domain::{CommitId, FileSource};
    use std::path::Path;

    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_local_link_missing_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join("docs")).expect("create workdir");
    std::fs::write(workdir.join("docs/present.md"), "x").expect("write file");

    let worktree = FileSource::WorkingDirectory;
    assert_eq!(
        markdown_preview_local_link_missing(&workdir, &worktree, Path::new("docs/present.md")),
        Some(false)
    );
    assert_eq!(
        markdown_preview_local_link_missing(&workdir, &worktree, Path::new("docs/gone.md")),
        Some(true)
    );
    assert_eq!(
        markdown_preview_local_link_missing(&workdir, &worktree, Path::new("docs")),
        Some(true),
        "a directory is not a file to open"
    );
    // Deleted or renamed since: the commit still has it, so let the
    // commit-backed load try rather than greying the entry out.
    let at_commit = FileSource::Commit(CommitId("deadbeef".into()));
    assert_eq!(
        markdown_preview_local_link_missing(&workdir, &at_commit, Path::new("docs/gone.md")),
        Some(false)
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn local_link_availability_refuses_symlinks_out_of_the_repository() {
    use super::markdown_preview_local_link_missing;
    use gitcomet_core::domain::{CommitId, FileSource};
    use std::path::Path;

    let root = std::env::temp_dir().join(format!(
        "gitcomet_local_link_symlink_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let workdir = root.join("repo");
    let outside = root.join("outside");
    std::fs::create_dir_all(workdir.join("docs/real")).expect("create workdir");
    std::fs::create_dir_all(workdir.join(".git")).expect("create .git");
    std::fs::create_dir_all(&outside).expect("create outside");
    std::fs::write(outside.join("secret.txt"), "secret").expect("write outside");
    std::fs::write(workdir.join(".git/config"), "[core]").expect("write config");
    std::fs::write(workdir.join("docs/real/inside.md"), "x").expect("write inside");
    std::os::unix::fs::symlink(&outside, workdir.join("docs/alias")).expect("alias");
    std::os::unix::fs::symlink(workdir.join(".git"), workdir.join("docs/meta")).expect("meta");
    std::os::unix::fs::symlink("real", workdir.join("docs/inner")).expect("inner");

    let worktree = FileSource::WorkingDirectory;
    let missing =
        |path: &str| markdown_preview_local_link_missing(&workdir, &worktree, Path::new(path));
    // Lexically inside the repository, but the file is not.
    assert_eq!(missing("docs/alias/secret.txt"), None);
    assert_eq!(missing("docs/meta/config"), None);
    // A symlink that stays inside the repository is an ordinary link.
    assert_eq!(missing("docs/inner/inside.md"), Some(false));
    // Git stores a symlink as a blob, so a commit's tree has nothing to follow.
    assert_eq!(
        markdown_preview_local_link_missing(
            &workdir,
            &FileSource::Commit(CommitId("deadbeef".into())),
            Path::new("docs/alias/secret.txt"),
        ),
        Some(false)
    );

    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn the_read_only_row_list_draws_tables_padded_into_columns() {
    use super::markdown_preview_list_row;
    let doc = crate::view::markdown_preview::parse_markdown(
        "Intro.\n\n| Name | Age |\n|---|---|\n| Alexander | 3 |\n",
    )
    .expect("parses");
    let intro = &doc.rows[0];
    assert!(
        matches!(
            markdown_preview_list_row(intro),
            std::borrow::Cow::Borrowed(_)
        ),
        "a row that is not a table is drawn as is"
    );
    let texts: Vec<String> = doc
        .rows
        .iter()
        .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::TableRow { .. }))
        .map(|row| markdown_preview_list_row(row).text.to_string())
        .collect();
    assert_eq!(texts, vec!["Name      | Age", "Alexander | 3"]);
}
