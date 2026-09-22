//! Explains a Git LFS or git-annex file in the diff pane: what each side is,
//! how big it is, and whether its content is here. Replaces the pointer diff
//! when content is missing; sits above the real diff when it is present.

use crate::theme::AppTheme;
use gitcomet_core::large_files::{LargeFileContent, LargeFilePointer, LargeFileSide};
use gitcomet_core::text_utils::human_readable_bytes;
use gpui::prelude::*;
use gpui::{AnyElement, SharedString, div, px};

pub(in crate::view) fn large_file_card_title(
    old: Option<&LargeFileSide>,
    new: Option<&LargeFileSide>,
) -> &'static str {
    match new.or(old).map(|side| &side.pointer) {
        Some(LargeFilePointer::Annex(_)) => "git-annex file",
        _ => "Git LFS file",
    }
}

/// One line per side that exists: label, then size, short id and presence.
pub(in crate::view) fn large_file_card_lines(
    old: Option<&LargeFileSide>,
    new: Option<&LargeFileSide>,
) -> Vec<(&'static str, String)> {
    let describe = |side: &LargeFileSide| {
        let size = side
            .pointer
            .size()
            .map(human_readable_bytes)
            .unwrap_or_else(|| "size unknown".to_string());
        let id = match &side.pointer {
            LargeFilePointer::Lfs(pointer) => format!("sha256 {}", &pointer.oid.to_hex()[..12]),
            LargeFilePointer::Annex(key) => key.backend.to_string(),
        };
        let presence = match side.content {
            LargeFileContent::Available => "content here".to_string(),
            LargeFileContent::MissingLocally => "not downloaded".to_string(),
            LargeFileContent::Unknown => "presence unknown".to_string(),
            LargeFileContent::TooLarge { bytes } => {
                format!(
                    "content here, too large to diff ({})",
                    human_readable_bytes(bytes)
                )
            }
        };
        format!("{size} · {id} · {presence}")
    };
    [("Before", old), ("After", new)]
        .into_iter()
        .filter_map(|(label, side)| side.map(|side| (label, describe(side))))
        .collect()
}

/// Why the real diff is not shown, when it is not.
pub(in crate::view) fn large_file_card_message(
    old: Option<&LargeFileSide>,
    new: Option<&LargeFileSide>,
) -> Option<&'static str> {
    let collected: Vec<LargeFileContent> = [old, new]
        .into_iter()
        .flatten()
        .map(|side| side.content)
        .collect();
    if collected.contains(&LargeFileContent::MissingLocally) {
        Some("The content is not in this clone. Download it to compare the files.")
    } else if collected.contains(&LargeFileContent::Unknown) {
        Some("git-annex can tell whether the content is here; the pointers are shown instead.")
    } else if collected
        .iter()
        .any(|c| matches!(c, LargeFileContent::TooLarge { .. }))
    {
        Some("The content is too large to compare as text.")
    } else {
        None
    }
}

pub(in crate::view) fn large_file_card(
    theme: AppTheme,
    old: Option<&LargeFileSide>,
    new: Option<&LargeFileSide>,
    as_header: bool,
    action: Option<AnyElement>,
) -> AnyElement {
    let title = large_file_card_title(old, new);
    let mut body = div()
        .id("large_file_card")
        .debug_selector(|| "large_file_card".to_string())
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .py_2()
        .child(
            div()
                .text_size(theme.ui_text(13.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.colors.foreground.primary)
                .child(title),
        );
    for (label, line) in large_file_card_lines(old, new) {
        body = body.child(
            div()
                .flex()
                .gap_2()
                .text_size(theme.ui_text(12.0))
                .child(
                    div()
                        .w(px(48.0))
                        .flex_none()
                        .text_color(theme.colors.foreground.secondary)
                        .child(label),
                )
                .child(
                    div()
                        .min_w(px(0.0))
                        .text_color(theme.colors.foreground.primary)
                        .child(SharedString::from(line)),
                ),
        );
    }
    if let Some(message) = large_file_card_message(old, new) {
        body = body.child(
            div()
                .pt_1()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(message),
        );
    }
    if let Some(action) = action {
        body = body.child(div().pt_1().flex().child(action));
    }
    if as_header {
        div()
            .flex_none()
            .border_b_1()
            .border_color(theme.colors.stroke.default)
            .bg(theme.colors.surface.canvas)
            .child(body)
            .into_any_element()
    } else {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.colors.surface.canvas)
            .child(div().max_w(px(560.0)).child(body))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lfs(size: u64, content: LargeFileContent) -> LargeFileSide {
        LargeFileSide {
            pointer: LargeFilePointer::Lfs(gitcomet_core::lfs::LfsPointer {
                oid: gitcomet_core::lfs::LfsOid([0xab; 32]),
                size,
            }),
            content,
        }
    }

    #[test]
    fn lines_name_size_id_and_presence_per_side() {
        let old = lfs(1_500, LargeFileContent::Available);
        let new = lfs(2_000_000, LargeFileContent::MissingLocally);
        assert_eq!(
            large_file_card_lines(Some(&old), Some(&new)),
            [
                (
                    "Before",
                    "1.5 KB · sha256 abababababab · content here".to_string()
                ),
                (
                    "After",
                    "2 MB · sha256 abababababab · not downloaded".to_string()
                ),
            ]
        );
        assert_eq!(
            large_file_card_lines(None, Some(&new)).len(),
            1,
            "added file"
        );
        assert_eq!(large_file_card_title(None, Some(&new)), "Git LFS file");
    }

    #[test]
    fn message_explains_why_the_diff_is_hidden() {
        let here = lfs(1, LargeFileContent::Available);
        let missing = lfs(1, LargeFileContent::MissingLocally);
        let big = lfs(1, LargeFileContent::TooLarge { bytes: 20_000_000 });
        assert_eq!(large_file_card_message(Some(&here), Some(&here)), None);
        assert!(
            large_file_card_message(Some(&here), Some(&missing))
                .unwrap()
                .contains("Download")
        );
        assert!(
            large_file_card_message(Some(&big), None)
                .unwrap()
                .contains("too large")
        );
    }
}
