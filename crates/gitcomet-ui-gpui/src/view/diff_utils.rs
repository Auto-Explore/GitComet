use super::*;

const MULTILINE_TEXT_COPY_BYTES_PER_LINE_ESTIMATE: usize = 64;
const DIFF_TEXT_TAB_WIDTH: usize = 4;

pub(in crate::view) fn multiline_text_copy_capacity_hint(line_count: usize) -> usize {
    line_count.saturating_mul(MULTILINE_TEXT_COPY_BYTES_PER_LINE_ESTIMATE)
}

pub(in crate::view) fn diff_text_display_len(text: &str) -> usize {
    if !text.contains('\t') {
        return text.len();
    }

    text.chars().fold(0usize, |len, ch| {
        len.saturating_add(match ch {
            '\t' => DIFF_TEXT_TAB_WIDTH,
            _ => ch.len_utf8(),
        })
    })
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
    let content_start = match line.kind {
        gitcomet_core::domain::DiffLineKind::Add
        | gitcomet_core::domain::DiffLineKind::Remove
        | gitcomet_core::domain::DiffLineKind::Context => 1,
        gitcomet_core::domain::DiffLineKind::Header | gitcomet_core::domain::DiffLineKind::Hunk => {
            0
        }
    };
    line.text
        .slice(content_start..line.text.len())
        .unwrap_or_else(|| line.text.clone())
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
    } else {
        None
    }
}

const SVG_PREVIEW_MIN_RASTER_WIDTH_PX: f32 = 1024.0;
const SVG_PREVIEW_MAX_RASTER_EDGE_PX: f32 = 4096.0;
static SVG_PREVIEW_USVG_OPTIONS: std::sync::LazyLock<resvg::usvg::Options<'static>> =
    std::sync::LazyLock::new(resvg::usvg::Options::default);

pub(in crate::view) fn rasterize_svg_png(
    svg_bytes: &[u8],
    target_width_px: f32,
    max_edge_px: f32,
) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &SVG_PREVIEW_USVG_OPTIONS).ok()?;
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

pub(super) fn parse_diff_git_header_path(text: &str) -> Option<String> {
    let text = text.strip_prefix("diff --git ")?;
    let mut parts = text.split_whitespace();
    let a = parts.next()?;
    let b = parts.next().unwrap_or(a);
    let b = b.strip_prefix("b/").unwrap_or(b);
    Some(b.to_string())
}

#[derive(Clone, Debug)]
pub(super) struct ParsedHunkHeader {
    pub(super) old: String,
    pub(super) new: String,
    pub(super) heading: Option<String>,
}

pub(super) fn parse_unified_hunk_header_for_display(text: &str) -> Option<ParsedHunkHeader> {
    let text = text.strip_prefix("@@")?.trim_start();
    let (ranges, rest) = text.split_once("@@")?;
    let ranges = ranges.trim();
    let heading = rest.trim();

    let mut it = ranges.split_whitespace();
    let old = it.next()?.trim().to_string();
    let new = it.next()?.trim().to_string();

    Some(ParsedHunkHeader {
        old,
        new,
        heading: (!heading.is_empty()).then_some(heading.to_string()),
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

    for line in diff {
        let is_file_header = matches!(line.kind(), gitcomet_core::domain::DiffLineKind::Header)
            && line.text().starts_with("diff --git ");
        if is_file_header {
            current_file = parse_diff_git_header_path(line.text()).map(Arc::<str>::from);
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

pub(super) fn enclosing_hunk_src_ix(diff: &[AnnotatedDiffLine], src_ix: usize) -> Option<usize> {
    let src_ix = src_ix.min(diff.len().saturating_sub(1));
    for ix in (0..=src_ix).rev() {
        let line = diff.get(ix)?;
        if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
            && line.text.starts_with("diff --git ")
        {
            break;
        }
        if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk) {
            return Some(ix);
        }
    }
    None
}

pub(super) trait UnifiedDiffLine {
    fn kind(&self) -> gitcomet_core::domain::DiffLineKind;
    fn text(&self) -> &str;
}

impl UnifiedDiffLine for AnnotatedDiffLine {
    fn kind(&self) -> gitcomet_core::domain::DiffLineKind {
        self.kind
    }

    fn text(&self) -> &str {
        self.text.as_ref()
    }
}

impl UnifiedDiffLine for gitcomet_core::domain::DiffLine {
    fn kind(&self) -> gitcomet_core::domain::DiffLineKind {
        self.kind
    }

    fn text(&self) -> &str {
        self.text.as_ref()
    }
}

fn unified_patch_file_and_hunk_bounds<T: UnifiedDiffLine>(
    lines: &[T],
    hunk_src_ix: usize,
) -> Option<(usize, usize, usize)> {
    let hunk = lines.get(hunk_src_ix)?;
    if !matches!(hunk.kind(), gitcomet_core::domain::DiffLineKind::Hunk) {
        return None;
    }

    let file_start = (0..=hunk_src_ix).rev().find(|&ix| {
        lines
            .get(ix)
            .is_some_and(|l| l.text().starts_with("diff --git "))
    })?;

    let first_hunk = (file_start + 1..lines.len())
        .find(|&ix| {
            let Some(line) = lines.get(ix) else {
                return false;
            };
            matches!(line.kind(), gitcomet_core::domain::DiffLineKind::Hunk)
                || line.text().starts_with("diff --git ")
        })
        .unwrap_or(lines.len());

    let header_end = first_hunk.min(hunk_src_ix);
    let hunk_end = (hunk_src_ix + 1..lines.len())
        .find(|&ix| {
            let Some(line) = lines.get(ix) else {
                return false;
            };
            matches!(line.kind(), gitcomet_core::domain::DiffLineKind::Hunk)
                || line.text().starts_with("diff --git ")
        })
        .unwrap_or(lines.len());

    Some((file_start, header_end, hunk_end))
}

fn unified_patch_capacity<T: UnifiedDiffLine>(
    lines: &[T],
    file_start: usize,
    header_end: usize,
    hunk_start: usize,
    hunk_end: usize,
) -> usize {
    lines[file_start..header_end]
        .iter()
        .map(|l| l.text().len().saturating_add(1))
        .sum::<usize>()
        .saturating_add(
            lines[hunk_start..hunk_end]
                .iter()
                .map(|l| l.text().len().saturating_add(1))
                .sum::<usize>(),
        )
}

pub(super) fn build_unified_patch_for_hunk(
    diff: &[impl UnifiedDiffLine],
    hunk_src_ix: usize,
) -> Option<String> {
    let lines = diff;
    let (file_start, header_end, hunk_end) =
        unified_patch_file_and_hunk_bounds(lines, hunk_src_ix)?;

    let capacity = unified_patch_capacity(lines, file_start, header_end, hunk_src_ix, hunk_end);
    let mut out = String::with_capacity(capacity);
    for line in &lines[file_start..header_end] {
        out.push_str(line.text());
        out.push('\n');
    }
    for line in &lines[hunk_src_ix..hunk_end] {
        out.push_str(line.text());
        out.push('\n');
    }
    (!out.trim().is_empty()).then_some(out)
}

pub(super) fn build_unified_patch_for_hunks(
    diff: &[AnnotatedDiffLine],
    hunk_src_ixs: &[usize],
) -> Option<String> {
    if hunk_src_ixs.is_empty() {
        return None;
    }

    let mut hunks = hunk_src_ixs.to_vec();
    hunks.sort_unstable();
    hunks.dedup();

    let mut out = String::new();
    for hunk_src_ix in hunks {
        let Some(patch) = build_unified_patch_for_hunk(diff, hunk_src_ix) else {
            continue;
        };
        out.push_str(&patch);
    }

    (!out.trim().is_empty()).then_some(out)
}

#[derive(Clone, Copy)]
enum UnselectedHunkLineBehavior {
    Drop,
    KeepAsContext,
}

fn append_unselected_hunk_line(
    out: &mut String,
    line_text: &str,
    expected_prefix: char,
    behavior: UnselectedHunkLineBehavior,
    prev_included: &mut bool,
) {
    match behavior {
        UnselectedHunkLineBehavior::Drop => {
            *prev_included = false;
        }
        UnselectedHunkLineBehavior::KeepAsContext => {
            let content = line_text.strip_prefix(expected_prefix).unwrap_or(line_text);
            out.push(' ');
            out.push_str(content);
            out.push('\n');
            *prev_included = true;
        }
    }
}

fn build_unified_patch_for_hunk_selection_with_unselected_behavior(
    diff: &[AnnotatedDiffLine],
    hunk_src_ix: usize,
    selected_src_ixs: &HashSet<usize>,
    unselected_add: UnselectedHunkLineBehavior,
    unselected_remove: UnselectedHunkLineBehavior,
) -> Option<String> {
    if selected_src_ixs.is_empty() {
        return None;
    }

    let lines = diff;
    let (file_start, header_end, hunk_end) =
        unified_patch_file_and_hunk_bounds(lines, hunk_src_ix)?;

    let capacity = unified_patch_capacity(lines, file_start, header_end, hunk_src_ix, hunk_end);
    let mut out = String::with_capacity(capacity);
    for line in &lines[file_start..header_end] {
        out.push_str(line.text());
        out.push('\n');
    }

    // Keep the original hunk header; `git apply --recount` will adjust counts.
    out.push_str(lines[hunk_src_ix].text());
    out.push('\n');

    let mut has_change = false;
    let mut prev_included = false;
    for (ix, line) in lines
        .iter()
        .enumerate()
        .take(hunk_end)
        .skip(hunk_src_ix + 1)
    {
        if line.text().starts_with("\\") {
            if prev_included {
                out.push_str(line.text());
                out.push('\n');
            }
            continue;
        }

        match line.kind {
            gitcomet_core::domain::DiffLineKind::Add => {
                if selected_src_ixs.contains(&ix) {
                    out.push_str(line.text());
                    out.push('\n');
                    has_change = true;
                    prev_included = true;
                } else {
                    append_unselected_hunk_line(
                        &mut out,
                        line.text(),
                        '+',
                        unselected_add,
                        &mut prev_included,
                    );
                }
            }
            gitcomet_core::domain::DiffLineKind::Remove => {
                if selected_src_ixs.contains(&ix) {
                    out.push_str(line.text());
                    out.push('\n');
                    has_change = true;
                    prev_included = true;
                } else {
                    append_unselected_hunk_line(
                        &mut out,
                        line.text(),
                        '-',
                        unselected_remove,
                        &mut prev_included,
                    );
                }
            }
            gitcomet_core::domain::DiffLineKind::Context => {
                out.push_str(line.text());
                out.push('\n');
                prev_included = true;
            }
            gitcomet_core::domain::DiffLineKind::Header
            | gitcomet_core::domain::DiffLineKind::Hunk => {
                out.push_str(line.text());
                out.push('\n');
                prev_included = true;
            }
        }
    }

    has_change.then_some(out)
}

pub(super) fn build_unified_patch_for_hunk_selection(
    diff: &[AnnotatedDiffLine],
    hunk_src_ix: usize,
    selected_src_ixs: &HashSet<usize>,
) -> Option<String> {
    build_unified_patch_for_hunk_selection_with_unselected_behavior(
        diff,
        hunk_src_ix,
        selected_src_ixs,
        UnselectedHunkLineBehavior::Drop,
        UnselectedHunkLineBehavior::KeepAsContext,
    )
}

pub(super) fn build_unified_patch_for_hunk_selection_for_worktree_discard(
    diff: &[AnnotatedDiffLine],
    hunk_src_ix: usize,
    selected_src_ixs: &HashSet<usize>,
) -> Option<String> {
    build_unified_patch_for_hunk_selection_with_unselected_behavior(
        diff,
        hunk_src_ix,
        selected_src_ixs,
        UnselectedHunkLineBehavior::KeepAsContext,
        UnselectedHunkLineBehavior::Drop,
    )
}

pub(super) fn build_unified_patch_for_selected_lines_across_hunks(
    diff: &[AnnotatedDiffLine],
    selected_src_ixs: &HashSet<usize>,
) -> Option<String> {
    use gitcomet_core::domain::DiffLineKind as K;
    use std::collections::BTreeMap;

    if selected_src_ixs.is_empty() {
        return None;
    }

    let mut by_hunk: BTreeMap<usize, HashSet<usize>> = BTreeMap::new();
    for &src_ix in selected_src_ixs {
        let Some(line) = diff.get(src_ix) else {
            continue;
        };
        if !matches!(line.kind, K::Add | K::Remove) {
            continue;
        }
        let Some(hunk_src_ix) = enclosing_hunk_src_ix(diff, src_ix) else {
            continue;
        };
        by_hunk.entry(hunk_src_ix).or_default().insert(src_ix);
    }

    let mut out = String::new();
    for (hunk_src_ix, src_ixs) in by_hunk {
        let Some(patch) = build_unified_patch_for_hunk_selection(diff, hunk_src_ix, &src_ixs)
        else {
            continue;
        };
        out.push_str(&patch);
    }

    (!out.trim().is_empty()).then_some(out)
}

pub(super) fn build_unified_patch_for_selected_lines_across_hunks_for_worktree_discard(
    diff: &[AnnotatedDiffLine],
    selected_src_ixs: &HashSet<usize>,
) -> Option<String> {
    use gitcomet_core::domain::DiffLineKind as K;
    use std::collections::BTreeMap;

    if selected_src_ixs.is_empty() {
        return None;
    }

    let mut by_hunk: BTreeMap<usize, HashSet<usize>> = BTreeMap::new();
    for &src_ix in selected_src_ixs {
        let Some(line) = diff.get(src_ix) else {
            continue;
        };
        if !matches!(line.kind, K::Add | K::Remove) {
            continue;
        }
        let Some(hunk_src_ix) = enclosing_hunk_src_ix(diff, src_ix) else {
            continue;
        };
        by_hunk.entry(hunk_src_ix).or_default().insert(src_ix);
    }

    let mut out = String::new();
    for (hunk_src_ix, src_ixs) in by_hunk {
        let Some(patch) = build_unified_patch_for_hunk_selection_for_worktree_discard(
            diff,
            hunk_src_ix,
            &src_ixs,
        ) else {
            continue;
        };
        out.push_str(&patch);
    }

    (!out.trim().is_empty()).then_some(out)
}

pub(super) fn context_menu_selection_range_from_diff_text(
    selection: Option<(DiffTextPos, DiffTextPos)>,
    diff_view: DiffViewMode,
    clicked_visible_ix: usize,
    clicked_region: DiffTextRegion,
) -> Option<(usize, usize)> {
    let (start, end) = selection?;
    if start == end {
        return None;
    }
    if clicked_visible_ix < start.visible_ix || clicked_visible_ix > end.visible_ix {
        return None;
    }

    let restrict_region = (diff_view == DiffViewMode::Split
        && start.region == end.region
        && matches!(
            start.region,
            DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight
        ))
    .then_some(start.region);
    if restrict_region.is_some_and(|r| r != clicked_region) {
        return None;
    }

    Some((start.visible_ix, end.visible_ix))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn parse_diff_and_hunk_headers_reject_malformed_inputs() {
        for text in ["", "diff --git", "diff --git ", "index 123..456 100644"] {
            assert_eq!(parse_diff_git_header_path(text), None, "{text:?}");
        }

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
        assert!(patch.contains("@@ -1,3 +1,3 @@"));
        assert!(patch.contains("@@ -5,3 +5,4 @@"));
    }

    #[test]
    fn build_unified_patch_for_selected_lines_across_hunks_includes_all_selected_lines() {
        let diff = example_two_hunk_diff();
        let selected: HashSet<usize> = [6, 7, 12].into_iter().collect();

        let patch =
            build_unified_patch_for_selected_lines_across_hunks(&diff, &selected).expect("patch");
        assert!(patch.contains("@@ -1,3 +1,3 @@"));
        assert!(patch.contains("@@ -5,3 +5,4 @@"));
        assert!(patch.contains("-line2"));
        assert!(patch.contains("+line2_mod"));
        assert!(patch.contains("+line6_5"));
    }

    #[test]
    fn build_unified_patch_for_selected_lines_across_hunks_ignores_context_only_selection() {
        let diff = example_two_hunk_diff();
        let selected: HashSet<usize> = [5, 8, 10].into_iter().collect();

        assert!(build_unified_patch_for_selected_lines_across_hunks(&diff, &selected).is_none());
    }

    #[test]
    fn build_unified_patch_for_selected_lines_across_hunks_supports_multiple_files() {
        let diff = example_two_file_diff();
        let selected: HashSet<usize> = [4, 9].into_iter().collect();

        let patch =
            build_unified_patch_for_selected_lines_across_hunks(&diff, &selected).expect("patch");
        assert!(patch.contains("diff --git a/a.txt b/a.txt"));
        assert!(patch.contains("diff --git a/b.txt b/b.txt"));
        assert!(patch.contains("+a"));
        assert!(patch.contains("+b"));
    }

    #[test]
    fn build_unified_patch_for_selected_lines_across_hunks_keeps_unselected_preimage_as_context() {
        let diff = example_two_mods_one_hunk_diff();
        let selected: HashSet<usize> = [6, 7].into_iter().collect();

        let patch =
            build_unified_patch_for_selected_lines_across_hunks(&diff, &selected).expect("patch");

        assert!(patch.contains("-line2"));
        assert!(patch.contains("+line2_mod"));
        assert!(!patch.contains("-line3"));
        assert!(!patch.contains("+line3_mod"));
        assert!(patch.contains(" line3"));
    }

    #[test]
    fn build_unified_patch_for_selected_lines_across_hunks_for_worktree_discard_keeps_unselected_changes_as_worktree_context()
     {
        let diff = example_two_mods_one_hunk_diff();
        let selected: HashSet<usize> = [6, 7].into_iter().collect();

        let patch = build_unified_patch_for_selected_lines_across_hunks_for_worktree_discard(
            &diff, &selected,
        )
        .expect("patch");

        assert!(patch.contains("-line2"));
        assert!(patch.contains("+line2_mod"));
        assert!(!patch.contains("-line3"));
        assert!(!patch.contains("+line3_mod"));
        assert!(patch.contains(" line3_mod"));
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
    fn context_menu_selection_range_from_diff_text_requires_click_in_selection() {
        let selection = Some((
            DiffTextPos {
                visible_ix: 2,
                region: DiffTextRegion::Inline,
                offset: 0,
            },
            DiffTextPos {
                visible_ix: 5,
                region: DiffTextRegion::Inline,
                offset: 3,
            },
        ));
        assert_eq!(
            context_menu_selection_range_from_diff_text(
                selection,
                DiffViewMode::Inline,
                4,
                DiffTextRegion::Inline
            ),
            Some((2, 5))
        );
        assert_eq!(
            context_menu_selection_range_from_diff_text(
                selection,
                DiffViewMode::Inline,
                1,
                DiffTextRegion::Inline
            ),
            None
        );
    }

    #[test]
    fn context_menu_selection_range_from_diff_text_restricts_split_region() {
        let selection = Some((
            DiffTextPos {
                visible_ix: 1,
                region: DiffTextRegion::SplitLeft,
                offset: 0,
            },
            DiffTextPos {
                visible_ix: 3,
                region: DiffTextRegion::SplitLeft,
                offset: 2,
            },
        ));
        assert_eq!(
            context_menu_selection_range_from_diff_text(
                selection,
                DiffViewMode::Split,
                2,
                DiffTextRegion::SplitLeft
            ),
            Some((1, 3))
        );
        assert_eq!(
            context_menu_selection_range_from_diff_text(
                selection,
                DiffViewMode::Split,
                2,
                DiffTextRegion::SplitRight
            ),
            None
        );
    }

    #[test]
    fn diff_text_display_len_expands_tabs_without_allocating() {
        assert_eq!(diff_text_display_len("hello"), 5);
        assert_eq!(diff_text_display_len("\twide\tcell"), 16);
    }
}
