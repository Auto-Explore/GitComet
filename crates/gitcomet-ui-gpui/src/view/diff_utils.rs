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
mod tests;
