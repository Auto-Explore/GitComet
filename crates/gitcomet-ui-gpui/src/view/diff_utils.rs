use super::*;

const MULTILINE_TEXT_COPY_BYTES_PER_LINE_ESTIMATE: usize = 64;

pub(in crate::view) fn multiline_text_copy_capacity_hint(line_count: usize) -> usize {
    line_count.saturating_mul(MULTILINE_TEXT_COPY_BYTES_PER_LINE_ESTIMATE)
}

/// Byte length of a line once its tabs are expanded to tab stops.
pub(in crate::view) fn diff_text_display_len(tab_width: usize, text: &str) -> usize {
    crate::view::tab_width::expanded_len(tab_width, text)
}

fn scrollbar_markers_from_bucket_flags(buckets: &[u8]) -> Vec<components::ScrollbarMarker> {
    let bucket_count = buckets.len();
    if bucket_count == 0 {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(bucket_count);
    let mut ix = 0usize;
    while ix < bucket_count {
        let flag = buckets[ix];
        if flag == 0 {
            ix += 1;
            continue;
        }

        let start = ix;
        ix += 1;
        while ix < bucket_count && buckets[ix] == flag {
            ix += 1;
        }
        let end = ix; // exclusive

        let kind = match flag {
            1 => components::ScrollbarMarkerKind::Add,
            2 => components::ScrollbarMarkerKind::Remove,
            _ => components::ScrollbarMarkerKind::Modify,
        };

        out.push(components::ScrollbarMarker {
            start: start as f32 / bucket_count as f32,
            end: end as f32 / bucket_count as f32,
            kind,
        });
    }

    out
}

pub(super) fn scrollbar_markers_from_flags(
    len: usize,
    mut flag_at_index: impl FnMut(usize) -> u8,
) -> Vec<components::ScrollbarMarker> {
    if len == 0 {
        return Vec::new();
    }

    let bucket_count = 240usize.min(len).max(1);
    let mut buckets = vec![0u8; bucket_count];
    for ix in 0..len {
        let flag = flag_at_index(ix);
        if flag == 0 {
            continue;
        }
        let b = (ix * bucket_count) / len;
        if let Some(cell) = buckets.get_mut(b) {
            *cell |= flag;
        }
    }

    scrollbar_markers_from_bucket_flags(buckets.as_slice())
}

pub(super) fn scrollbar_markers_from_visible_ranges(
    len: usize,
    ranges: impl IntoIterator<Item = (usize, usize, u8)>,
) -> Vec<components::ScrollbarMarker> {
    if len == 0 {
        return Vec::new();
    }

    let bucket_count = 240usize.min(len).max(1);
    let mut buckets = vec![0u8; bucket_count];
    for (start, end, flag) in ranges {
        if flag == 0 || start >= end || start >= len {
            continue;
        }
        let clamped_end = end.min(len);
        if clamped_end <= start {
            continue;
        }

        let bucket_start = start.saturating_mul(bucket_count) / len;
        let bucket_end = clamped_end.saturating_sub(1).saturating_mul(bucket_count) / len;
        for bucket_ix in bucket_start..=bucket_end.min(bucket_count.saturating_sub(1)) {
            if let Some(cell) = buckets.get_mut(bucket_ix) {
                *cell |= flag;
            }
        }
    }

    scrollbar_markers_from_bucket_flags(buckets.as_slice())
}

pub(super) fn diff_content_text(line: &AnnotatedDiffLine) -> &str {
    match line.kind {
        gitcomet_core::domain::DiffLineKind::Add => {
            line.text.strip_prefix('+').unwrap_or(&line.text)
        }
        gitcomet_core::domain::DiffLineKind::Remove => {
            line.text.strip_prefix('-').unwrap_or(&line.text)
        }
        gitcomet_core::domain::DiffLineKind::Context => {
            line.text.strip_prefix(' ').unwrap_or(&line.text)
        }
        gitcomet_core::domain::DiffLineKind::Header | gitcomet_core::domain::DiffLineKind::Hunk => {
            &line.text
        }
    }
}

pub(super) fn diff_content_line_text(
    line: &AnnotatedDiffLine,
) -> gitcomet_core::file_diff::FileDiffLineText {
    diff_content_line_slice(line.kind, &line.text)
}

pub(super) fn diff_content_line_slice(
    kind: gitcomet_core::domain::DiffLineKind,
    text: &gitcomet_core::domain::SharedLineText,
) -> gitcomet_core::file_diff::FileDiffLineText {
    let content_start = match kind {
        gitcomet_core::domain::DiffLineKind::Add
        | gitcomet_core::domain::DiffLineKind::Remove
        | gitcomet_core::domain::DiffLineKind::Context => 1,
        gitcomet_core::domain::DiffLineKind::Header | gitcomet_core::domain::DiffLineKind::Hunk => {
            0
        }
    };
    text.slice(content_start..text.len())
        .unwrap_or_else(|| text.clone())
        .into()
}

pub(super) fn image_format_for_path(path: &std::path::Path) -> Option<gpui::ImageFormat> {
    let ext = path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("png") {
        Some(gpui::ImageFormat::Png)
    } else if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
        Some(gpui::ImageFormat::Jpeg)
    } else if ext.eq_ignore_ascii_case("gif") {
        Some(gpui::ImageFormat::Gif)
    } else if ext.eq_ignore_ascii_case("webp") {
        Some(gpui::ImageFormat::Webp)
    } else if ext.eq_ignore_ascii_case("bmp") {
        Some(gpui::ImageFormat::Bmp)
    } else if ext.eq_ignore_ascii_case("svg") {
        Some(gpui::ImageFormat::Svg)
    } else if ext.eq_ignore_ascii_case("tif") || ext.eq_ignore_ascii_case("tiff") {
        Some(gpui::ImageFormat::Tiff)
    } else if ext.eq_ignore_ascii_case("ico") {
        Some(gpui::ImageFormat::Ico)
    } else {
        None
    }
}

/// The raster format `bytes` are in, read from their signature: for a file
/// whose name does not say.
pub(super) fn image_format_for_bytes(bytes: &[u8]) -> Option<gpui::ImageFormat> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some(gpui::ImageFormat::Png),
        image::ImageFormat::Jpeg => Some(gpui::ImageFormat::Jpeg),
        image::ImageFormat::Gif => Some(gpui::ImageFormat::Gif),
        image::ImageFormat::WebP => Some(gpui::ImageFormat::Webp),
        image::ImageFormat::Bmp => Some(gpui::ImageFormat::Bmp),
        image::ImageFormat::Tiff => Some(gpui::ImageFormat::Tiff),
        image::ImageFormat::Ico => Some(gpui::ImageFormat::Ico),
        image::ImageFormat::Pnm => Some(gpui::ImageFormat::Pnm),
        _ => None,
    }
}

const SVG_PREVIEW_MIN_RASTER_WIDTH_PX: f32 = 1024.0;
const SVG_PREVIEW_MAX_RASTER_EDGE_PX: f32 = 4096.0;

pub(in crate::view) fn fill_svg_viewport_white(pixmap: &mut resvg::tiny_skia::Pixmap) {
    pixmap.fill(resvg::tiny_skia::Color::WHITE);
}

pub(in crate::view) fn rasterize_svg_png(
    svg_bytes: &[u8],
    target_width_px: f32,
    max_edge_px: f32,
) -> Option<Vec<u8>> {
    let tree = super::image_preview::svg::parse(svg_bytes).ok()?;
    let svg_size = tree.size();
    let svg_width = svg_size.width();
    let svg_height = svg_size.height();
    if !svg_width.is_finite() || !svg_height.is_finite() || svg_width <= 0.0 || svg_height <= 0.0 {
        return None;
    }

    let upscale = if svg_width < target_width_px {
        target_width_px / svg_width
    } else {
        1.0
    };
    let mut raster_width = (svg_width * upscale).round();
    let mut raster_height = (svg_height * upscale).round();
    let max_edge = raster_width.max(raster_height);
    if max_edge > max_edge_px {
        let downscale = max_edge_px / max_edge;
        raster_width = (raster_width * downscale).round();
        raster_height = (raster_height * downscale).round();
    }

    let raster_width = raster_width.max(1.0) as u32;
    let raster_height = raster_height.max(1.0) as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(raster_width, raster_height)?;
    fill_svg_viewport_white(&mut pixmap);
    let transform = resvg::tiny_skia::Transform::from_scale(
        raster_width as f32 / svg_width,
        raster_height as f32 / svg_height,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

pub(super) fn rasterize_svg_preview_png(svg_bytes: &[u8]) -> Option<Vec<u8>> {
    rasterize_svg_png(
        svg_bytes,
        SVG_PREVIEW_MIN_RASTER_WIDTH_PX,
        SVG_PREVIEW_MAX_RASTER_EDGE_PX,
    )
}

#[derive(Clone, Debug)]
pub(super) struct ParsedHunkHeader {
    pub(super) old: String,
    pub(super) new: String,
    pub(super) heading: Option<String>,
    pub(super) old_start_line: u32,
    pub(super) old_line_count: u32,
    pub(super) new_start_line: u32,
    pub(super) new_line_count: u32,
}

pub(super) fn parse_unified_hunk_header_for_display(text: &str) -> Option<ParsedHunkHeader> {
    fn parse_range(token: &str, prefix: char) -> Option<(String, u32, u32)> {
        let body = token.strip_prefix(prefix)?;
        let (start_text, count_text) = body.split_once(',').unwrap_or((body, "1"));
        let start_line = start_text.parse::<u32>().ok()?;
        let line_count = count_text.parse::<u32>().ok()?;
        Some((token.to_string(), start_line, line_count))
    }

    let text = text.strip_prefix("@@")?.trim_start();
    let (ranges, rest) = text.split_once("@@")?;
    let ranges = ranges.trim();
    let heading = rest.trim();

    let mut it = ranges.split_whitespace();
    let (old, old_start_line, old_line_count) = parse_range(it.next()?.trim(), '-')?;
    let (new, new_start_line, new_line_count) = parse_range(it.next()?.trim(), '+')?;

    Some(ParsedHunkHeader {
        old,
        new,
        heading: (!heading.is_empty()).then_some(heading.to_string()),
        old_start_line,
        old_line_count,
        new_start_line,
        new_line_count,
    })
}

pub(super) fn compute_diff_file_stats(
    diff: &[impl UnifiedDiffLine],
) -> Vec<Option<(usize, usize)>> {
    let mut stats: Vec<Option<(usize, usize)>> = vec![None; diff.len()];

    let mut current_file_header_ix: Option<usize> = None;
    let mut adds = 0usize;
    let mut removes = 0usize;

    for (ix, line) in diff.iter().enumerate() {
        let is_file_header = matches!(line.kind(), gitcomet_core::domain::DiffLineKind::Header)
            && line.text().starts_with("diff --git ");

        if is_file_header {
            if let Some(header_ix) = current_file_header_ix.take() {
                stats[header_ix] = Some((adds, removes));
            }
            current_file_header_ix = Some(ix);
            adds = 0;
            removes = 0;
            continue;
        }

        match line.kind() {
            gitcomet_core::domain::DiffLineKind::Add => adds += 1,
            gitcomet_core::domain::DiffLineKind::Remove => removes += 1,
            _ => {}
        }
    }

    if let Some(header_ix) = current_file_header_ix {
        stats[header_ix] = Some((adds, removes));
    }

    stats
}

pub(super) fn compute_diff_file_for_src_ix(diff: &[impl UnifiedDiffLine]) -> Vec<Option<Arc<str>>> {
    let mut out: Vec<Option<Arc<str>>> = Vec::with_capacity(diff.len());
    let mut current_file: Option<Arc<str>> = None;

    for (ix, line) in diff.iter().enumerate() {
        let is_file_header = matches!(line.kind(), gitcomet_core::domain::DiffLineKind::Header)
            && line.text().starts_with("diff --git ");
        if is_file_header {
            // The name a `diff --git` line carries is only unambiguous once the
            // `---`/`+++` lines below it are taken into account, so resolve the
            // whole section at once.
            current_file =
                gitcomet_core::diff::unified_diff_file_path(diff[ix..].iter().map(|l| l.text()))
                    .map(Arc::<str>::from);
        }
        out.push(current_file.clone());
    }

    out
}

#[derive(Clone, Copy, Debug, Default)]
struct YamlBlockScalarState {
    block_scalar_indent: Option<usize>,
}

fn diff_line_content_text(kind: gitcomet_core::domain::DiffLineKind, text: &str) -> &str {
    match kind {
        gitcomet_core::domain::DiffLineKind::Add => text.strip_prefix('+').unwrap_or(text),
        gitcomet_core::domain::DiffLineKind::Remove => text.strip_prefix('-').unwrap_or(text),
        gitcomet_core::domain::DiffLineKind::Context => text.strip_prefix(' ').unwrap_or(text),
        gitcomet_core::domain::DiffLineKind::Header | gitcomet_core::domain::DiffLineKind::Hunk => {
            text
        }
    }
}

fn yaml_block_scalar_indicator_indent(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let indent = bytes.iter().take_while(|&&byte| byte == b' ').count();
    let mut ix = indent;

    if bytes.get(ix) == Some(&b'-') {
        ix += 1;
        while bytes.get(ix).is_some_and(|byte| byte.is_ascii_whitespace()) {
            ix += 1;
        }
    }

    let key_start = ix;
    while bytes
        .get(ix)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        ix += 1;
    }
    if ix == key_start || bytes.get(ix) != Some(&b':') {
        return None;
    }

    ix += 1;
    while bytes.get(ix).is_some_and(|byte| byte.is_ascii_whitespace()) {
        ix += 1;
    }

    if !matches!(bytes.get(ix), Some(b'|') | Some(b'>')) {
        return None;
    }

    ix += 1;
    while bytes
        .get(ix)
        .is_some_and(|byte| matches!(byte, b'+' | b'-') || byte.is_ascii_digit())
    {
        ix += 1;
    }

    bytes[ix..]
        .iter()
        .all(|byte| byte.is_ascii_whitespace())
        .then_some(indent)
}

fn yaml_update_block_scalar_state(state: &mut YamlBlockScalarState, text: &str) -> bool {
    let indent = text
        .as_bytes()
        .iter()
        .take_while(|&&byte| byte == b' ')
        .count();
    let trimmed = text[indent..].trim_end();
    let mut is_block_scalar_content = false;
    let mut keep_existing_block_scalar = false;

    if let Some(base_indent) = state.block_scalar_indent {
        if trimmed.is_empty() {
            keep_existing_block_scalar = true;
        } else if indent > base_indent {
            keep_existing_block_scalar = true;
            is_block_scalar_content = true;
        } else {
            state.block_scalar_indent = None;
        }
    }

    if !keep_existing_block_scalar {
        state.block_scalar_indent = yaml_block_scalar_indicator_indent(text);
    }

    is_block_scalar_content
}

pub(super) fn compute_diff_yaml_block_scalar_for_src_ix(
    diff: &[impl UnifiedDiffLine],
    diff_file_for_src_ix: &[Option<Arc<str>>],
    diff_language_for_src_ix: &[Option<rows::DiffSyntaxLanguage>],
) -> Vec<bool> {
    let mut out = vec![false; diff.len()];
    let mut current_file: Option<Arc<str>> = None;
    let mut old_state = YamlBlockScalarState::default();
    let mut new_state = YamlBlockScalarState::default();

    for (ix, line) in diff.iter().enumerate() {
        let next_file = diff_file_for_src_ix
            .get(ix)
            .and_then(|file| file.as_ref())
            .cloned();
        let file_changed = match (&current_file, &next_file) {
            (Some(current), Some(next)) => !Arc::ptr_eq(current, next),
            (None, None) => false,
            _ => true,
        };
        if file_changed {
            current_file = next_file;
            old_state = YamlBlockScalarState::default();
            new_state = YamlBlockScalarState::default();
        }

        if !matches!(
            diff_language_for_src_ix.get(ix).copied().flatten(),
            Some(rows::DiffSyntaxLanguage::Yaml)
        ) {
            continue;
        }

        if matches!(line.kind(), gitcomet_core::domain::DiffLineKind::Hunk) {
            old_state = YamlBlockScalarState::default();
            new_state = YamlBlockScalarState::default();
            continue;
        }

        let text = diff_line_content_text(line.kind(), line.text());
        match line.kind() {
            gitcomet_core::domain::DiffLineKind::Context => {
                let old = yaml_update_block_scalar_state(&mut old_state, text);
                let new = yaml_update_block_scalar_state(&mut new_state, text);
                out[ix] = old || new;
            }
            gitcomet_core::domain::DiffLineKind::Remove => {
                out[ix] = yaml_update_block_scalar_state(&mut old_state, text);
            }
            gitcomet_core::domain::DiffLineKind::Add => {
                out[ix] = yaml_update_block_scalar_state(&mut new_state, text);
            }
            gitcomet_core::domain::DiffLineKind::Header
            | gitcomet_core::domain::DiffLineKind::Hunk => {}
        }
    }

    out
}

pub(super) use gitcomet_core::partial_patch::{
    NO_NEWLINE_MARKER, UnifiedDiffLine, build_unified_patch_for_hunk,
    build_unified_patch_for_hunks, build_unified_patch_for_selected_lines_across_hunks,
    build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply,
};

/// Placeholder line for a side that is binary or not UTF-8, with the sizes
/// of whichever sides exist so a size change is still visible.
pub(in crate::view) fn binary_diff_placeholder_message(
    old_bytes: Option<u64>,
    new_bytes: Option<u64>,
) -> String {
    use gitcomet_core::text_utils::human_readable_bytes;
    match (old_bytes, new_bytes) {
        (Some(old), Some(new)) if old == new => {
            format!(
                "Binary or non-UTF-8 content is not shown as text ({}).",
                human_readable_bytes(old)
            )
        }
        (Some(old), Some(new)) => format!(
            "Binary or non-UTF-8 content is not shown as text ({} to {}).",
            human_readable_bytes(old),
            human_readable_bytes(new)
        ),
        (None, Some(bytes)) | (Some(bytes), None) => format!(
            "Binary or non-UTF-8 content is not shown as text ({}).",
            human_readable_bytes(bytes)
        ),
        (None, None) => "Binary or non-UTF-8 content is not shown as text.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_placeholder_names_the_sizes_that_exist() {
        assert_eq!(
            binary_diff_placeholder_message(Some(1_000), Some(2_500)),
            "Binary or non-UTF-8 content is not shown as text (1 KB to 2.5 KB)."
        );
        assert_eq!(
            binary_diff_placeholder_message(Some(700), Some(700)),
            "Binary or non-UTF-8 content is not shown as text (700 B)."
        );
        assert_eq!(
            binary_diff_placeholder_message(None, Some(3)),
            "Binary or non-UTF-8 content is not shown as text (3 B)."
        );
        assert_eq!(
            binary_diff_placeholder_message(None, None),
            "Binary or non-UTF-8 content is not shown as text."
        );
    }
    use gitcomet_core::domain::DiffLineKind as K;

    fn dl(kind: K, text: &str) -> AnnotatedDiffLine {
        AnnotatedDiffLine {
            kind,
            text: text.into(),
            old_line: None,
            new_line: None,
        }
    }

    fn example_two_hunk_diff() -> Vec<AnnotatedDiffLine> {
        vec![
            dl(K::Header, "diff --git a/file.txt b/file.txt"),
            dl(K::Header, "index 1111111..2222222 100644"),
            dl(K::Header, "--- a/file.txt"),
            dl(K::Header, "+++ b/file.txt"),
            dl(K::Hunk, "@@ -1,3 +1,3 @@"),
            dl(K::Context, " line1"),
            dl(K::Remove, "-line2"),
            dl(K::Add, "+line2_mod"),
            dl(K::Context, " line3"),
            dl(K::Hunk, "@@ -5,3 +5,4 @@"),
            dl(K::Context, " line5"),
            dl(K::Context, " line6"),
            dl(K::Add, "+line6_5"),
            dl(K::Context, " line7"),
        ]
    }

    fn example_two_file_diff() -> Vec<AnnotatedDiffLine> {
        vec![
            dl(K::Header, "diff --git a/a.txt b/a.txt"),
            dl(K::Header, "--- a/a.txt"),
            dl(K::Header, "+++ b/a.txt"),
            dl(K::Hunk, "@@ -1,0 +1,1 @@"),
            dl(K::Add, "+a"),
            dl(K::Header, "diff --git a/b.txt b/b.txt"),
            dl(K::Header, "--- a/b.txt"),
            dl(K::Header, "+++ b/b.txt"),
            dl(K::Hunk, "@@ -1,0 +1,1 @@"),
            dl(K::Add, "+b"),
        ]
    }

    fn example_two_mods_one_hunk_diff() -> Vec<AnnotatedDiffLine> {
        vec![
            dl(K::Header, "diff --git a/file.txt b/file.txt"),
            dl(K::Header, "index 1111111..2222222 100644"),
            dl(K::Header, "--- a/file.txt"),
            dl(K::Header, "+++ b/file.txt"),
            dl(K::Hunk, "@@ -1,4 +1,4 @@"),
            dl(K::Context, " line1"),
            dl(K::Remove, "-line2"),
            dl(K::Add, "+line2_mod"),
            dl(K::Remove, "-line3"),
            dl(K::Add, "+line3_mod"),
            dl(K::Context, " line4"),
        ]
    }

    #[test]
    fn yaml_block_scalar_state_survives_blank_lines_between_added_content() {
        let diff = vec![
            dl(K::Header, "diff --git a/workflow.yml b/workflow.yml"),
            dl(K::Header, "--- a/workflow.yml"),
            dl(K::Header, "+++ b/workflow.yml"),
            dl(K::Hunk, "@@ -1,0 +1,5 @@"),
            dl(K::Add, "+        run: |"),
            dl(K::Add, "+          $ErrorActionPreference = \"Stop\""),
            dl(K::Add, "+"),
            dl(K::Add, "+          $required = @("),
            dl(K::Add, "+            \"AZURE_CREDENTIALS\""),
        ];
        let files = vec![Some(Arc::<str>::from("workflow.yml")); diff.len()];
        let languages = vec![Some(rows::DiffSyntaxLanguage::Yaml); diff.len()];

        let flags = compute_diff_yaml_block_scalar_for_src_ix(
            diff.as_slice(),
            files.as_slice(),
            languages.as_slice(),
        );

        assert_eq!(
            flags.get(4),
            Some(&false),
            "indicator line should not be forced"
        );
        assert_eq!(
            flags.get(5),
            Some(&true),
            "first block-scalar content line should be forced"
        );
        assert_eq!(
            flags.get(6),
            Some(&false),
            "blank block-scalar line should not need forcing"
        );
        assert_eq!(
            flags.get(7),
            Some(&true),
            "content after a blank block-scalar line should still be forced"
        );
        assert_eq!(
            flags.get(8),
            Some(&true),
            "nested content after a blank block-scalar line should still be forced"
        );
    }

    #[test]
    fn yaml_block_scalar_indicator_accepts_list_items_and_chomping_modifiers() {
        assert_eq!(yaml_block_scalar_indicator_indent("- script: |-2"), Some(0));
        assert_eq!(
            yaml_block_scalar_indicator_indent("  - script: >+"),
            Some(2)
        );
        assert_eq!(yaml_block_scalar_indicator_indent("script: value"), None);
    }

    #[test]
    fn yaml_block_scalar_state_resets_when_diff_switches_files() {
        let file_a: Arc<str> = Arc::from("a.yml");
        let file_b: Arc<str> = Arc::from("b.yml");
        let diff = vec![
            dl(K::Add, "+script: |"),
            dl(K::Add, "+  echo one"),
            dl(K::Add, "+plain: value"),
        ];
        let files = vec![
            Some(Arc::clone(&file_a)),
            Some(Arc::clone(&file_a)),
            Some(file_b),
        ];
        let languages = vec![
            Some(rows::DiffSyntaxLanguage::Yaml),
            Some(rows::DiffSyntaxLanguage::Yaml),
            Some(rows::DiffSyntaxLanguage::Yaml),
        ];

        let flags = compute_diff_yaml_block_scalar_for_src_ix(
            diff.as_slice(),
            files.as_slice(),
            languages.as_slice(),
        );

        assert_eq!(flags, vec![false, true, false]);
    }

    /// A path containing spaces cannot be recovered from the `diff --git` line
    /// alone, and getting it wrong used to collapse every such file onto the
    /// same bogus name — which broke every row-to-patch-line lookup.
    #[test]
    fn file_for_src_ix_resolves_paths_containing_spaces() {
        let diff = vec![
            dl(K::Header, "diff --git a/my notes.md b/my notes.md"),
            dl(K::Header, "index 1111111..2222222 100644"),
            dl(K::Header, "--- a/my notes.md\t"),
            dl(K::Header, "+++ b/my notes.md\t"),
            dl(K::Hunk, "@@ -1 +1 @@"),
            dl(K::Add, "+one"),
            dl(K::Header, "diff --git a/other notes.md b/other notes.md"),
            dl(K::Header, "--- a/other notes.md\t"),
            dl(K::Header, "+++ b/other notes.md\t"),
            dl(K::Hunk, "@@ -1 +1 @@"),
            dl(K::Add, "+two"),
        ];

        let files = compute_diff_file_for_src_ix(diff.as_slice());
        let names = files
            .iter()
            .map(|file| file.as_deref().unwrap_or("<none>"))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "my notes.md",
                "my notes.md",
                "my notes.md",
                "my notes.md",
                "my notes.md",
                "my notes.md",
                "other notes.md",
                "other notes.md",
                "other notes.md",
                "other notes.md",
                "other notes.md",
            ]
        );

        // The two files must stay distinct identities: state that resets on a
        // file boundary compares them by pointer.
        let first = files[0].clone().expect("first file path");
        let second = files[6].clone().expect("second file path");
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn file_for_src_ix_keeps_the_post_image_name_of_a_rename() {
        let diff = vec![
            dl(K::Header, "diff --git a/old.txt b/new name.txt"),
            dl(K::Header, "similarity index 94%"),
            dl(K::Header, "rename from old.txt"),
            dl(K::Header, "rename to new name.txt"),
            dl(K::Header, "--- a/old.txt"),
            dl(K::Header, "+++ b/new name.txt\t"),
            dl(K::Hunk, "@@ -1 +1 @@"),
            dl(K::Add, "+one"),
        ];

        let files = compute_diff_file_for_src_ix(diff.as_slice());
        assert!(
            files
                .iter()
                .all(|file| file.as_deref() == Some("new name.txt")),
            "{files:?}"
        );
    }

    #[test]
    fn parse_hunk_headers_reject_malformed_inputs() {
        for text in [
            "",
            "@@",
            "@@ -1,2",
            "@@ -1,2 @@",
            "@@ -1,2 +3,4",
            "@@ start @@ heading",
        ] {
            assert!(
                parse_unified_hunk_header_for_display(text).is_none(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn yaml_block_scalar_state_tolerates_short_metadata_arrays() {
        let diff = vec![
            dl(K::Header, "diff --git a/workflow.yml b/workflow.yml"),
            dl(K::Add, "+script: |"),
            dl(K::Add, "+  echo one"),
            dl(K::Add, "+  echo two"),
        ];
        let files = vec![Some(Arc::<str>::from("workflow.yml"))];
        let languages = vec![
            Some(rows::DiffSyntaxLanguage::Yaml),
            Some(rows::DiffSyntaxLanguage::Yaml),
        ];

        let flags = compute_diff_yaml_block_scalar_for_src_ix(
            diff.as_slice(),
            files.as_slice(),
            languages.as_slice(),
        );

        assert_eq!(flags, vec![false, false, false, false]);
    }

    #[test]
    fn build_unified_patch_for_hunks_includes_multiple_hunks() {
        let diff = example_two_hunk_diff();
        let patch = build_unified_patch_for_hunks(&diff, &[4, 9]).expect("patch");
        let patch = String::from_utf8(patch).expect("UTF-8 patch");
        assert!(patch.contains("@@ -1,3 +1,3 @@"));
        assert!(patch.contains("@@ -5,3 +5,4 @@"));
    }

    #[test]
    fn build_unified_patch_for_selected_lines_across_hunks_includes_all_selected_lines() {
        let diff = example_two_hunk_diff();
        let selected: FxHashSet<usize> = [6, 7, 12].into_iter().collect();

        let patch =
            build_unified_patch_for_selected_lines_across_hunks(&diff, &selected).expect("patch");
        let patch = String::from_utf8(patch).expect("UTF-8 patch");
        assert!(patch.contains("@@ -1,3 +1,3 @@"));
        assert!(patch.contains("@@ -5,3 +5,4 @@"));
        assert!(patch.contains("-line2"));
        assert!(patch.contains("+line2_mod"));
        assert!(patch.contains("+line6_5"));
    }

    #[test]
    fn build_unified_patch_for_selected_lines_across_hunks_ignores_context_only_selection() {
        let diff = example_two_hunk_diff();
        let selected: FxHashSet<usize> = [5, 8, 10].into_iter().collect();

        assert!(build_unified_patch_for_selected_lines_across_hunks(&diff, &selected).is_none());
    }

    #[test]
    fn build_unified_patch_for_selected_lines_across_hunks_supports_multiple_files() {
        let diff = example_two_file_diff();
        let selected: FxHashSet<usize> = [4, 9].into_iter().collect();

        let patch =
            build_unified_patch_for_selected_lines_across_hunks(&diff, &selected).expect("patch");
        let patch = String::from_utf8(patch).expect("UTF-8 patch");
        assert!(patch.contains("diff --git a/a.txt b/a.txt"));
        assert!(patch.contains("diff --git a/b.txt b/b.txt"));
        assert!(patch.contains("+a"));
        assert!(patch.contains("+b"));
    }

    #[test]
    fn build_unified_patch_for_selected_lines_across_hunks_keeps_unselected_preimage_as_context() {
        let diff = example_two_mods_one_hunk_diff();
        let selected: FxHashSet<usize> = [6, 7].into_iter().collect();

        let patch =
            build_unified_patch_for_selected_lines_across_hunks(&diff, &selected).expect("patch");
        let patch = String::from_utf8(patch).expect("UTF-8 patch");

        assert!(patch.contains("-line2"));
        assert!(patch.contains("+line2_mod"));
        assert!(!patch.contains("-line3"));
        assert!(!patch.contains("+line3_mod"));
        assert!(patch.contains(" line3"));
    }

    #[derive(Clone, Copy)]
    enum LineAction {
        Stage,
        Unstage,
        Discard,
    }

    struct EditedFile {
        dir: tempfile::TempDir,
        opened: std::sync::Arc<dyn gitcomet_core::services::GitRepository>,
        text: String,
        diff: Vec<AnnotatedDiffLine>,
    }

    /// A repository with `name` committed as `head` and then changed by
    /// `edit`, with the pane's diff of it for `area` (staged first for Staged).
    fn edited_file_diff(
        name: &str,
        head: &str,
        edit: impl FnOnce(&std::path::Path),
        area: gitcomet_core::domain::DiffArea,
    ) -> EditedFile {
        use gitcomet_core::domain::{Diff, DiffArea, DiffTarget};
        use gitcomet_core::services::GitBackend as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        crate::test_support::git(repo, &["init", "-q"]);
        // Git for Windows' system config sets autocrlf, so applying a patch
        // to the worktree would write CRLF.
        crate::test_support::git(repo, &["config", "core.autocrlf", "false"]);
        std::fs::write(repo.join(name), head).expect("write head");
        crate::test_support::git(repo, &["add", name]);
        crate::test_support::git(
            repo,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@e.st",
                "commit",
                "-qm",
                "base",
            ],
        );
        edit(&repo.join(name));
        if area == DiffArea::Staged {
            crate::test_support::git(repo, &["add", name]);
        }
        let opened = gitcomet_git_gix::GixBackend.open(repo).expect("open repo");
        let target = DiffTarget::working_tree(std::path::PathBuf::from(name), area);
        let text = opened.diff_unified(&target).expect("diff");
        let diff = gitcomet_core::diff::annotate_unified(&Diff::from_unified(target, &text));
        EditedFile {
            dir,
            opened,
            text,
            diff,
        }
    }

    /// Runs one line action the way the diff pane does: the real git diff,
    /// the pane's patch builder for that action, and the backend apply. Returns
    /// the content of the side the action writes (index or worktree).
    fn apply_line_action(head: &str, worktree: &str, line: &str, action: LineAction) -> String {
        use gitcomet_core::domain::DiffArea;

        let area = match action {
            LineAction::Stage | LineAction::Discard => DiffArea::Unstaged,
            LineAction::Unstage => DiffArea::Staged,
        };
        let EditedFile {
            dir,
            opened,
            text,
            diff,
        } = edited_file_diff(
            "f.txt",
            head,
            |path| std::fs::write(path, worktree).expect("write worktree"),
            area,
        );
        let repo = dir.path();
        let selected: FxHashSet<usize> = diff
            .iter()
            .enumerate()
            .filter(|(_, l)| l.text.as_ref() == line)
            .map(|(ix, _)| ix)
            .collect();
        assert_eq!(selected.len(), 1, "{line:?} must be one line of:\n{text}");

        match action {
            LineAction::Stage => {
                let patch = build_unified_patch_for_selected_lines_across_hunks(&diff, &selected)
                    .expect("patch");
                opened
                    .apply_unified_patch_to_index_with_output(&patch, false)
                    .expect("apply");
            }
            LineAction::Unstage => {
                let patch = build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
                    &diff, &selected,
                )
                .expect("patch");
                opened
                    .apply_unified_patch_to_index_with_output(&patch, true)
                    .expect("apply");
            }
            LineAction::Discard => {
                let patch = build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
                    &diff, &selected,
                )
                .expect("patch");
                opened
                    .apply_unified_patch_to_worktree_with_output(&patch, true)
                    .expect("apply");
            }
        }

        match action {
            LineAction::Stage | LineAction::Unstage => {
                String::from_utf8(crate::test_support::git(repo, &["show", ":f.txt"]))
                    .expect("UTF-8 index")
            }
            LineAction::Discard => std::fs::read_to_string(repo.join("f.txt")).expect("worktree"),
        }
    }

    /// A script made executable and edited shows `old mode`/`new mode` above
    /// its hunk. Hunk and line patches copied those lines, so "Stage hunk"
    /// also staged the mode and "Discard hunk" also reset the executable bit.
    #[cfg(unix)]
    #[test]
    fn hunk_and_line_actions_leave_the_file_mode_alone() {
        use gitcomet_core::domain::DiffArea;
        use std::os::unix::fs::PermissionsExt as _;

        let edited_executable = || {
            let edited = edited_file_diff(
                "run.sh",
                "echo one\n",
                |path| {
                    std::fs::write(path, "echo two\n").expect("write worktree");
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                        .expect("chmod +x");
                },
                DiffArea::Unstaged,
            );
            assert!(edited.text.contains("new mode 100755"), "{}", edited.text);
            (edited.dir, edited.opened, edited.diff)
        };
        let index_entry = |repo: &std::path::Path| {
            String::from_utf8(crate::test_support::git(
                repo,
                &["ls-files", "-s", "run.sh"],
            ))
            .expect("UTF-8 ls-files")
        };
        let worktree_mode = |repo: &std::path::Path| {
            std::fs::metadata(repo.join("run.sh"))
                .expect("worktree file")
                .permissions()
                .mode()
                & 0o777
        };

        let (dir, opened, diff) = edited_executable();
        let hunk_ix = diff.iter().position(|line| line.kind == K::Hunk).unwrap();
        let patch = build_unified_patch_for_hunk(&diff, hunk_ix).expect("hunk patch");
        opened
            .apply_unified_patch_to_index_with_output(&patch, false)
            .expect("stage hunk");
        assert!(index_entry(dir.path()).starts_with("100644 "), "stage hunk");

        let (dir, opened, diff) = edited_executable();
        let selected: FxHashSet<usize> = diff
            .iter()
            .position(|line| line.text.as_ref() == "+echo two")
            .into_iter()
            .collect();
        let patch =
            build_unified_patch_for_selected_lines_across_hunks(&diff, &selected).expect("patch");
        opened
            .apply_unified_patch_to_index_with_output(&patch, false)
            .expect("stage line");
        assert!(index_entry(dir.path()).starts_with("100644 "), "stage line");

        let (dir, opened, diff) = edited_executable();
        let hunk_ix = diff.iter().position(|line| line.kind == K::Hunk).unwrap();
        let patch = build_unified_patch_for_hunk(&diff, hunk_ix).expect("hunk patch");
        opened
            .apply_unified_patch_to_worktree_with_output(&patch, true)
            .expect("discard hunk");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("run.sh")).unwrap(),
            "echo one\n"
        );
        // Git rewrites the file under the umask, so only the owner's
        // executable bit is the same on every machine.
        assert_ne!(worktree_mode(dir.path()) & 0o100, 0, "discard hunk");
    }

    /// A removed `-- comment` line reads `--- comment` in the diff. Parsed as a
    /// file header, it was hidden from the view and staged along with any
    /// selected line of its hunk.
    #[test]
    fn staging_a_line_leaves_an_unselected_removed_dash_dash_line_alone() {
        assert_eq!(
            apply_line_action(
                "select 1;\n-- old comment\nselect 2;\n",
                "select 1;\nselect 2;\nselect 3;\n",
                "+select 3;",
                LineAction::Stage,
            ),
            "select 1;\n-- old comment\nselect 2;\nselect 3;\n"
        );
    }

    /// An edited last line without a trailing newline shows as `-b`, `\ No
    /// newline`, `+B`, `\ No newline`. Keeping the unselected side as context
    /// used to keep its marker too, so git glued the two lines into `bB`.
    #[test]
    fn line_actions_on_a_last_line_without_newline_keep_lines_apart() {
        assert_eq!(
            apply_line_action("a\nb", "a\nB", "+B", LineAction::Stage),
            "a\nb\nB"
        );
        assert_eq!(
            apply_line_action("a\nb", "a\nB", "-b", LineAction::Stage),
            "a\n"
        );
        assert_eq!(
            apply_line_action("a\nb", "a\nB", "-b", LineAction::Unstage),
            "a\nb\nB"
        );
        assert_eq!(
            apply_line_action("a\nb", "a\nB", "+B", LineAction::Unstage),
            "a\n"
        );
        assert_eq!(
            apply_line_action("a\nb", "a\nB", "-b", LineAction::Discard),
            "a\nb\nB"
        );
        assert_eq!(
            apply_line_action("a\nb", "a\nB", "+B", LineAction::Discard),
            "a\n"
        );
    }

    #[test]
    fn reverse_apply_patch_keeps_unselected_additions_as_context() {
        let diff = example_two_mods_one_hunk_diff();
        let selected: FxHashSet<usize> = [6, 7].into_iter().collect();

        let patch =
            build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(&diff, &selected)
                .expect("patch");
        let patch = String::from_utf8(patch).expect("UTF-8 patch");

        assert!(patch.contains("-line2"));
        assert!(patch.contains("+line2_mod"));
        assert!(!patch.contains("-line3"));
        assert!(!patch.contains("+line3_mod"));
        assert!(patch.contains(" line3_mod"));
    }

    fn text_svg(family: &str) -> Vec<u8> {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="80">
            <text x="10" y="55" font-family="{family}" font-size="40">Labels</text></svg>"#
        )
        .into_bytes()
    }

    /// Conflict previews parsed SVG without fonts, so their text disappeared
    /// until detail tiles arrived.
    #[test]
    fn svg_text_survives_conflict_preview() {
        for family in [
            "IBM Plex Sans",
            "Lilex",
            "sans-serif",
            "monospace",
            "system-ui",
        ] {
            let png = rasterize_svg_preview_png(&text_svg(family)).unwrap();
            let image = image::load_from_memory(&png).unwrap().to_rgba8();
            assert!(
                image.pixels().filter(|p| p[0] < 128).count() > 100,
                "conflict SVG preview lost font-family={family}"
            );
        }
    }

    #[test]
    fn svg_text_survives_bounded_image_preview() {
        for family in [
            "IBM Plex Sans",
            "Lilex",
            "sans-serif",
            "monospace",
            "system-ui",
        ] {
            let tree = crate::view::image_preview::svg::parse(&text_svg(family)).unwrap();
            let preview =
                crate::view::panes::diff::diff_cache::render_svg_image_diff_preview(&tree).unwrap();
            assert!(
                preview
                    .as_bytes(0)
                    .unwrap()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[0] < 128)
                    .count()
                    > 100,
                "bounded SVG preview lost font-family={family}"
            );
        }
    }

    #[test]
    fn rasterize_svg_preview_png_scales_small_previews_to_2x_floor() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 8">
<rect width="16" height="8" fill="#00aaff"/>
</svg>"##;

        let png = rasterize_svg_preview_png(svg).expect("svg should rasterize");
        assert!(png.len() >= 24, "png should contain an IHDR chunk");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);

        assert_eq!(width, SVG_PREVIEW_MIN_RASTER_WIDTH_PX as u32);
        assert_eq!(height, (SVG_PREVIEW_MIN_RASTER_WIDTH_PX / 2.0) as u32);
    }

    #[test]
    fn rasterize_svg_preview_png_fills_transparent_viewport_white() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4">
<rect x="1" y="1" width="2" height="2" fill="#00aaff"/>
</svg>"##;

        let png = rasterize_svg_preview_png(svg).expect("svg should rasterize");
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .expect("decode png")
            .into_rgba8();

        assert_eq!(decoded.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(
            decoded
                .get_pixel(decoded.width() / 2, decoded.height() / 2)
                .0,
            [0, 170, 255, 255]
        );
    }

    #[test]
    fn diff_text_display_len_expands_tabs_without_allocating() {
        let tab_width = 4;

        assert_eq!(diff_text_display_len(tab_width, "hello"), 5);
        // Tab stops of 4: "    wide" then a tab to column 12.
        assert_eq!(diff_text_display_len(tab_width, "\twide\tcell"), 16);
        assert_eq!(diff_text_display_len(tab_width, "ab\tc"), 5);
    }

    #[test]
    fn scrollbar_markers_from_visible_ranges_matches_per_row_flags() {
        let per_row = scrollbar_markers_from_flags(20, |ix| {
            if (3..7).contains(&ix) {
                1
            } else if (10..12).contains(&ix) {
                2
            } else {
                0
            }
        });
        let ranged = scrollbar_markers_from_visible_ranges(20, [(3, 7, 1), (10, 12, 2)]);
        assert_eq!(ranged, per_row);
    }
}
