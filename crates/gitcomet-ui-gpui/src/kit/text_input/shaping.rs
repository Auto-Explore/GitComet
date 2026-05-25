use super::*;

#[cfg(any(test, feature = "benchmarks"))]
pub(super) const TEXT_INPUT_SHAPING_FINGERPRINT_SAMPLE_BYTES: usize = 64;
#[cfg(any(test, feature = "benchmarks"))]
pub(super) const TEXT_INPUT_SHAPING_FINGERPRINT_MID_SAMPLES_TRUNCATED: usize = 3;
#[cfg(any(test, feature = "benchmarks"))]
pub(super) const TEXT_INPUT_SHAPING_FINGERPRINT_MID_SAMPLES_UNTRUNCATED: usize = 1;

#[derive(Clone, Copy)]
pub(super) struct ShapingSliceInfo<'a> {
    prefix: &'a str,
    capped_len: usize,
    truncated: bool,
}

impl<'a> ShapingSliceInfo<'a> {
    #[inline]
    fn new(line_text: &'a str, max_bytes: usize) -> Self {
        if line_text.len() <= max_bytes {
            return Self {
                prefix: line_text,
                capped_len: line_text.len(),
                truncated: false,
            };
        }

        let suffix_len = TEXT_INPUT_TRUNCATION_SUFFIX.len();
        let mut end = max_bytes.saturating_sub(suffix_len).min(line_text.len());
        while end > 0 && !line_text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }

        Self {
            prefix: &line_text[..end],
            capped_len: end.saturating_add(suffix_len),
            truncated: true,
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    #[inline]
    fn hash(self) -> u64 {
        hash_shaping_prefix_bytes(self.prefix.as_bytes(), self.capped_len, self.truncated)
    }

    #[inline]
    fn into_shared_string(self) -> SharedString {
        if !self.truncated {
            return self.prefix.to_string().into();
        }

        let mut truncated = String::with_capacity(self.capped_len);
        truncated.push_str(self.prefix);
        truncated.push_str(TEXT_INPUT_TRUNCATION_SUFFIX);
        truncated.into()
    }

    #[inline]
    fn into_cow(self) -> Cow<'a, str> {
        if !self.truncated {
            return Cow::Borrowed(self.prefix);
        }

        let mut truncated = String::with_capacity(self.capped_len);
        truncated.push_str(self.prefix);
        truncated.push_str(TEXT_INPUT_TRUNCATION_SUFFIX);
        Cow::Owned(truncated)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[inline]
pub(super) fn hash_shaping_prefix_bytes(
    prefix_bytes: &[u8],
    capped_len: usize,
    truncated: bool,
) -> u64 {
    let mut hasher = FxHasher::default();
    hasher.write_usize(capped_len);

    if prefix_bytes.len() <= TEXT_INPUT_SHAPING_FINGERPRINT_SAMPLE_BYTES * 4 {
        hasher.write(prefix_bytes);
        return hasher.finish();
    }

    let sample_len = TEXT_INPUT_SHAPING_FINGERPRINT_SAMPLE_BYTES;
    let mid_samples = if truncated {
        TEXT_INPUT_SHAPING_FINGERPRINT_MID_SAMPLES_TRUNCATED
    } else {
        // The uncapped path only needs a cheap stable whole-line sketch for
        // benchmark/test helpers, not the denser truncated-line sampling.
        TEXT_INPUT_SHAPING_FINGERPRINT_MID_SAMPLES_UNTRUNCATED
    };
    hasher.write(&prefix_bytes[..sample_len]);

    let last_start = prefix_bytes.len().saturating_sub(sample_len);
    if mid_samples > 0 {
        let gap = last_start.saturating_sub(sample_len);
        for sample_ix in 1..=mid_samples {
            let start = sample_len + gap.saturating_mul(sample_ix) / (mid_samples + 1);
            hasher.write_usize(start);
            hasher.write(&prefix_bytes[start..start + sample_len]);
        }
    }

    hasher.write_usize(last_start);
    hasher.write(&prefix_bytes[last_start..]);
    hasher.finish()
}

#[inline]
pub(super) fn shaping_slice_info(line_text: &str, max_bytes: usize) -> ShapingSliceInfo<'_> {
    ShapingSliceInfo::new(line_text, max_bytes)
}

/// Compute a stable fingerprint and capped byte length for a line that may need truncation.
/// This does NOT allocate, and on very long lines it samples representative chunks instead of
/// rescanning the full shaping prefix.
#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn hash_shaping_slice(line_text: &str, max_bytes: usize) -> (u64, usize) {
    let info = shaping_slice_info(line_text, max_bytes);
    (info.hash(), info.capped_len)
}

/// Build the (possibly truncated) SharedString for shaping. Only call on cache miss.
pub(super) fn build_shaping_text(line_text: &str, max_bytes: usize) -> SharedString {
    shaping_slice_info(line_text, max_bytes).into_shared_string()
}

pub(super) fn build_shaping_line_slice<'a>(line_text: &'a str, max_bytes: usize) -> Cow<'a, str> {
    shaping_slice_info(line_text, max_bytes).into_cow()
}

#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn truncate_line_for_shaping(line_text: &str, max_bytes: usize) -> (SharedString, u64) {
    let info = shaping_slice_info(line_text, max_bytes);
    let hash = info.hash();
    let text = info.into_shared_string();
    (text, hash)
}

#[cfg(feature = "benchmarks")]
#[inline]
pub(crate) fn benchmark_text_input_shaping_slice(text: &str, max_bytes: usize) -> (u64, usize) {
    hash_shaping_slice(text, max_bytes)
}

pub(super) fn with_alpha(mut color: Rgba, alpha: f32) -> Rgba {
    color.a = alpha;
    color
}

#[cfg(target_os = "macos")]
pub(super) fn primary_modifier_label() -> &'static str {
    "Cmd"
}

#[cfg(not(target_os = "macos"))]
pub(super) fn primary_modifier_label() -> &'static str {
    "Ctrl"
}

pub(super) fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(8);
    starts.push(0);
    for (ix, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(ix + 1);
        }
    }
    starts
}

pub(super) fn line_text_for_index<'a>(text: &'a str, starts: &[usize], line_ix: usize) -> &'a str {
    let text_len = text.len();
    let Some(start) = starts.get(line_ix).copied() else {
        return "";
    };
    if start >= text_len {
        return "";
    }

    let mut end = starts
        .get(line_ix + 1)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if end > start && text.as_bytes().get(end - 1) == Some(&b'\n') {
        end -= 1;
    }
    text.get(start..end).unwrap_or("")
}

pub(super) fn mask_text_for_display(text: &str) -> String {
    let mut masked = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        match byte {
            b'\n' => masked.push('\n'),
            b'\r' => masked.push('\r'),
            _ => masked.push('*'),
        }
    }
    masked
}

pub(super) fn truncated_line_x_for_source_offset(
    line: &TruncatedLineLayout,
    source_offset: usize,
) -> Pixels {
    if let Some((hidden_range, display_range)) = line
        .projection
        .ellipsis_segment_for_source_offset(source_offset)
    {
        let hidden_mid =
            hidden_range.start + (hidden_range.end.saturating_sub(hidden_range.start) / 2);
        let display_offset = if source_offset <= hidden_mid {
            display_range.start
        } else {
            display_range.end
        };
        return line.shaped_line.x_for_index(display_offset);
    }

    let display_offset = line.projection.source_to_display_offset(source_offset);
    line.shaped_line.x_for_index(display_offset)
}

pub(super) fn truncated_line_source_offset_for_x(line: &TruncatedLineLayout, x: Pixels) -> usize {
    let display_offset = line.shaped_line.closest_index_for_x(x.max(px(0.0)));
    if let Some((hidden_range, display_range)) = line
        .projection
        .ellipsis_segment_at_display_offset(display_offset)
    {
        let x0 = line.shaped_line.x_for_index(display_range.start);
        let x1 = line.shaped_line.x_for_index(display_range.end);
        let midpoint = x0 + (x1 - x0) / 2.0;
        return if x <= midpoint {
            hidden_range.start
        } else {
            hidden_range.end
        };
    }

    line.projection
        .display_to_source_start_offset(display_offset)
}

pub(super) fn line_for_offset(
    starts: &[usize],
    lines: &[ShapedLine],
    offset: usize,
) -> (usize, usize) {
    let mut ix = starts.partition_point(|&s| s <= offset);
    if ix == 0 {
        ix = 1;
    }
    let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
    let start = starts.get(line_ix).copied().unwrap_or(0);
    let local = offset.saturating_sub(start).min(lines[line_ix].len());
    (line_ix, local)
}
