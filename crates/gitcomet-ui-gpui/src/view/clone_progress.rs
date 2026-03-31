use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::Path;

pub(crate) fn build_clone_progress_header(title: &str, url: &str, dest: &Path) -> String {
    let mut header =
        String::with_capacity(title.len().saturating_add(url.len()).saturating_add(64));
    append_clone_progress_header(&mut header, title, url, dest);
    header
}

pub(crate) fn append_clone_progress_header(buf: &mut String, title: &str, url: &str, dest: &Path) {
    buf.clear();
    buf.reserve(title.len().saturating_add(url.len()).saturating_add(64));
    buf.push_str(title);
    buf.push('\n');
    buf.push_str(url);
    buf.push('\n');
    buf.push_str("-> ");
    let _ = write!(buf, "{}", dest.display());
}

pub(crate) fn reset_clone_progress_message(buf: &mut String, header: &str) {
    buf.clear();
    buf.reserve(header.len().saturating_add(64));
    buf.push_str(header);
}

pub(crate) fn append_clone_progress_tail_window(
    buf: &mut String,
    tail_lines: &VecDeque<String>,
    visible_lines: usize,
) {
    if visible_lines == 0 || tail_lines.is_empty() {
        return;
    }
    buf.push('\n');
    buf.push('\n');

    let visible_lines = visible_lines.min(tail_lines.len());
    let (front, back) = tail_lines.as_slices();
    let back_visible = back.len().min(visible_lines);
    let front_visible = visible_lines.saturating_sub(back_visible);
    let front_start = front.len().saturating_sub(front_visible);
    let back_start = back.len().saturating_sub(back_visible);
    let mut wrote_line = false;

    for line in &front[front_start..] {
        if wrote_line {
            buf.push('\n');
        } else {
            wrote_line = true;
        }
        buf.push_str(line);
    }

    for line in &back[back_start..] {
        if wrote_line {
            buf.push('\n');
        } else {
            wrote_line = true;
        }
        buf.push_str(line);
    }
}
