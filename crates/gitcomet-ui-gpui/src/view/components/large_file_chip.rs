use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gitcomet_core::large_files::LargeFileState;
use gpui::prelude::*;
use gpui::{Div, div};

/// What a large-file chip says, separate from rendering so tests can pin it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LargeFileChipTone {
    /// Managed and present here.
    Present,
    /// Managed, but the content is not in the local store.
    Missing,
    /// Managed; presence needs the tool to tell (annex unlocked files).
    Unknown,
}

pub fn large_file_chip_label(
    state: &LargeFileState,
    locked: bool,
) -> (&'static str, LargeFileChipTone) {
    let label = match (state.pointer.is_lfs(), locked) {
        (true, true) => "LFS locked",
        (true, false) => "LFS",
        (false, _) => "annex",
    };
    let tone = match state.in_local_store {
        _ if state.content_missing() => LargeFileChipTone::Missing,
        Some(_) => LargeFileChipTone::Present,
        None => LargeFileChipTone::Unknown,
    };
    (label, tone)
}

/// Small text chip naming the extension that manages a file. Missing content
/// uses the warning colour so un-downloaded files stand out in a long list.
pub fn large_file_chip(
    theme: AppTheme,
    scale: impl Into<UiScale>,
    state: &LargeFileState,
    locked: bool,
) -> Div {
    let scale = scale.into();
    let (label, tone) = large_file_chip_label(state, locked);
    let color = match tone {
        LargeFileChipTone::Missing => theme.colors.status.warning.foreground,
        LargeFileChipTone::Present | LargeFileChipTone::Unknown => {
            theme.colors.foreground.secondary
        }
    };
    div()
        .flex_none()
        .px(scale.px(4.0))
        .rounded(scale.px(3.0))
        .border_1()
        .border_color(crate::theme::with_alpha(color, 0.45))
        // Explicit size: this renders inside rows whose inherited size differs.
        .text_size(scale.ui_text(10.0))
        .line_height(scale.px(14.0))
        .text_color(color)
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::large_files::{LargeFilePointer, LargeFileWorktree};

    fn state(
        lfs: bool,
        in_store: Option<bool>,
        worktree: Option<LargeFileWorktree>,
    ) -> LargeFileState {
        let pointer = if lfs {
            LargeFilePointer::Lfs(gitcomet_core::lfs::LfsPointer {
                oid: gitcomet_core::lfs::LfsOid([1; 32]),
                size: 3,
            })
        } else {
            LargeFilePointer::Annex(gitcomet_core::annex::parse_key("SHA256E-s3--ab.bin").unwrap())
        };
        LargeFileState {
            pointer,
            in_local_store: in_store,
            worktree,
            lockable: false,
        }
    }

    #[test]
    fn label_names_the_extension_and_tone_follows_presence() {
        use LargeFileChipTone::*;
        assert_eq!(
            large_file_chip_label(&state(true, Some(true), None), true),
            ("LFS locked", Present)
        );
        assert_eq!(
            large_file_chip_label(&state(true, Some(true), None), false),
            ("LFS", Present)
        );
        assert_eq!(
            large_file_chip_label(
                &state(true, Some(false), Some(LargeFileWorktree::Pointer)),
                false
            ),
            ("LFS", Missing)
        );
        // Content checked out in the worktree is not missing even if the store lacks it.
        assert_eq!(
            large_file_chip_label(
                &state(true, Some(false), Some(LargeFileWorktree::Content)),
                false
            ),
            ("LFS", Present)
        );
        assert_eq!(
            large_file_chip_label(&state(false, None, None), false),
            ("annex", Unknown)
        );
    }
}
