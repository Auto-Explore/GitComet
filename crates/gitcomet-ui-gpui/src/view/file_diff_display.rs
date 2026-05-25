use gpui::SharedString;
use std::ops::Range;

pub(in crate::view) const LARGE_DIFF_TEXT_MIN_BYTES: usize = 64 * 1024;

const LARGE_FILE_DIFF_DISPLAY_PREFIX_BYTES: usize = 16 * 1024;
const LARGE_FILE_DIFF_DISPLAY_TRUNCATION_SUFFIX: &str = " ...";

pub(in crate::view) fn expanded_diff_display_text<'a>(
    text: &'a str,
    expanded_tabs: &'a mut String,
) -> &'a str {
    if !text.contains('\t') {
        return text;
    }

    expanded_tabs.clear();
    expanded_tabs.reserve(crate::view::diff_utils::diff_text_display_len(text));
    for ch in text.chars() {
        match ch {
            '\t' => expanded_tabs.push_str("    "),
            _ => expanded_tabs.push(ch),
        }
    }
    expanded_tabs.as_str()
}

pub(in crate::view) fn append_diff_display_text_slice(
    out: &mut String,
    text: &str,
    range: Range<usize>,
    expanded_tabs: &mut String,
) {
    if range.start >= range.end {
        return;
    }

    let display = expanded_diff_display_text(text, expanded_tabs);
    let start = range.start.min(display.len());
    let end = range.end.min(display.len());
    if start < end
        && let Some(slice) = display.get(start..end)
    {
        out.push_str(slice);
    }
}

pub(in crate::view) fn is_large_file_diff_text(
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
) -> bool {
    raw_text.len() >= LARGE_DIFF_TEXT_MIN_BYTES
}

pub(in crate::view) fn should_truncate_file_diff_display(
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
) -> bool {
    is_large_file_diff_text(raw_text) && raw_text.has_tabs_without_loading()
}

pub(in crate::view) fn file_diff_display_text(
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
) -> SharedString {
    if should_truncate_file_diff_display(raw_text) {
        return truncated_file_diff_display_text(raw_text);
    }

    if !raw_text.has_tabs_without_loading() {
        return raw_text
            .slice_text(0..raw_text.len())
            .map(|text| SharedString::from(text.as_ref().to_owned()))
            .unwrap_or_default();
    }

    let text = raw_text.as_ref();
    let mut out = String::with_capacity(crate::view::diff_utils::diff_text_display_len(text));
    append_expanded_tabs(&mut out, text);
    out.into()
}

pub(in crate::view) fn file_diff_display_len(
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
) -> usize {
    if should_truncate_file_diff_display(raw_text) {
        return truncated_file_diff_display_len(raw_text);
    }

    if raw_text.has_tabs_without_loading() {
        crate::view::diff_utils::diff_text_display_len(raw_text.as_ref())
    } else {
        raw_text.len()
    }
}

pub(in crate::view) fn append_file_diff_display_text_slice(
    out: &mut String,
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    range: Range<usize>,
    expanded_tabs: &mut String,
) {
    if range.start >= range.end {
        return;
    }

    if should_truncate_file_diff_display(raw_text) {
        append_large_file_diff_display_text_slice(out, raw_text, range);
        return;
    }

    if !raw_text.has_tabs_without_loading() {
        let start = range.start.min(raw_text.len());
        let end = range.end.min(raw_text.len());
        if start < end
            && let Some(slice) = raw_text.slice_text(start..end)
        {
            out.push_str(slice.as_ref());
            return;
        }
    }

    append_diff_display_text_slice(out, raw_text.as_ref(), range, expanded_tabs);
}

fn truncated_file_diff_display_text(
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
) -> SharedString {
    let mut out = String::new();
    append_truncated_file_diff_display_text(&mut out, raw_text);
    out.into()
}

fn truncated_file_diff_display_len(raw_text: &gitcomet_core::file_diff::FileDiffLineText) -> usize {
    let prefix_len = raw_text.len().min(LARGE_FILE_DIFF_DISPLAY_PREFIX_BYTES);
    let Some((prefix, _)) = raw_text.slice_text_resolved(0..prefix_len) else {
        return 0;
    };

    let mut len = if prefix.contains('\t') {
        crate::view::diff_utils::diff_text_display_len(prefix.as_ref())
    } else {
        prefix.len()
    };
    if prefix_len < raw_text.len() {
        len = len.saturating_add(LARGE_FILE_DIFF_DISPLAY_TRUNCATION_SUFFIX.len());
    }
    len
}

fn append_truncated_file_diff_display_text(
    out: &mut String,
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
) {
    let prefix_len = raw_text.len().min(LARGE_FILE_DIFF_DISPLAY_PREFIX_BYTES);
    let Some((prefix, _)) = raw_text.slice_text_resolved(0..prefix_len) else {
        return;
    };

    append_expanded_tabs(out, prefix.as_ref());
    if prefix_len < raw_text.len() {
        out.push_str(LARGE_FILE_DIFF_DISPLAY_TRUNCATION_SUFFIX);
    }
}

fn append_large_file_diff_display_text_slice(
    out: &mut String,
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    range: Range<usize>,
) {
    let mut display = String::new();
    append_truncated_file_diff_display_text(&mut display, raw_text);

    let start = range.start.min(display.len());
    let end = range.end.min(display.len());
    if start >= end {
        return;
    }

    if let Some(slice) = display.get(start..end) {
        out.push_str(slice);
    }
}

fn append_expanded_tabs(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '\t' => out.push_str("    "),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_file_diff_display_is_bounded() {
        let raw_text = gitcomet_core::file_diff::FileDiffLineText::from(format!(
            "start\t{}",
            "x".repeat(LARGE_DIFF_TEXT_MIN_BYTES)
        ));

        let display = file_diff_display_text(&raw_text);

        assert!(should_truncate_file_diff_display(&raw_text));
        assert!(display.len() < raw_text.len());
        assert!(display.ends_with(LARGE_FILE_DIFF_DISPLAY_TRUNCATION_SUFFIX));
        assert_eq!(file_diff_display_len(&raw_text), display.len());
    }

    #[test]
    fn large_tabbed_file_diff_slice_uses_display_coordinates() {
        let raw_text = gitcomet_core::file_diff::FileDiffLineText::from(format!(
            "abc\tdef{}",
            "x".repeat(LARGE_DIFF_TEXT_MIN_BYTES)
        ));
        let display = file_diff_display_text(&raw_text);
        let display_text = display.as_ref();
        let mut out = String::new();
        let mut expanded_tabs = String::new();

        append_file_diff_display_text_slice(&mut out, &raw_text, 4..9, &mut expanded_tabs);

        assert_eq!(out.as_str(), &display_text[4..9]);
        assert_eq!(out, "   de");
    }

    #[test]
    fn large_tabbed_file_diff_full_slice_includes_truncation_suffix() {
        let raw_text = gitcomet_core::file_diff::FileDiffLineText::from(format!(
            "abc\tdef{}",
            "x".repeat(LARGE_DIFF_TEXT_MIN_BYTES)
        ));
        let display = file_diff_display_text(&raw_text);
        let display_text = display.as_ref();
        let mut out = String::new();
        let mut expanded_tabs = String::new();

        append_file_diff_display_text_slice(
            &mut out,
            &raw_text,
            0..file_diff_display_len(&raw_text),
            &mut expanded_tabs,
        );

        assert_eq!(out.as_str(), display_text);
        assert!(out.ends_with(LARGE_FILE_DIFF_DISPLAY_TRUNCATION_SUFFIX));
    }
}
