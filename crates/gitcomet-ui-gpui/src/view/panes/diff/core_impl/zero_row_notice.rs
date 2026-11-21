use crate::view::panes::diff::{DiffBody, DiffRenderer};
use gitcomet_core::diff::is_rename_source_header;
use gitcomet_core::diff_engine::DiffEngineStatus;
use gitcomet_core::domain::{DiffLineKind, DiffTarget};
use gpui::SharedString;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum ZeroRowNotice {
    OnlyTextDiffers,
    NoChanges,
    EmptyFile {
        renamed: bool,
    },
    NoChangedLines {
        compared_with: Option<SharedString>,
        renamed: bool,
    },
}

impl ZeroRowNotice {
    pub(in crate::view) fn title(&self) -> &'static str {
        match self {
            Self::OnlyTextDiffers => "Difftastic - Collapsed mode",
            Self::EmptyFile { renamed: true } | Self::NoChangedLines { renamed: true, .. } => {
                "Renamed"
            }
            Self::NoChanges
            | Self::EmptyFile { renamed: false }
            | Self::NoChangedLines { renamed: false, .. } => "Diff",
        }
    }

    pub(in crate::view) fn message(&self) -> SharedString {
        match self {
            Self::OnlyTextDiffers => "No syntactic changes; text differs".into(),
            Self::NoChanges => "No changes.".into(),
            Self::EmptyFile { .. } => "Empty file.".into(),
            Self::NoChangedLines {
                compared_with: Some(path),
                ..
            } => format!("No changed lines compared with {path}.").into(),
            Self::NoChangedLines {
                compared_with: None,
                ..
            } => "No changed lines.".into(),
        }
    }

    pub(in crate::view) fn debug_selector(&self) -> &'static str {
        match self {
            Self::OnlyTextDiffers => "difftastic_text_only_canvas",
            Self::NoChanges => "diff_no_changes",
            Self::EmptyFile { .. } => "diff_empty_file",
            Self::NoChangedLines { .. } => "diff_no_changed_lines",
        }
    }
}

impl DiffRenderer {
    pub(in crate::view) fn zero_row_notice(&self) -> Option<ZeroRowNotice> {
        if self.collapsed_difftastic_has_only_text_changes() {
            return Some(ZeroRowNotice::OnlyTextDiffers);
        }
        if self.diff_surface().body == DiffBody::Patch {
            return (self.patch_diff_row_len() == 0).then_some(ZeroRowNotice::NoChanges);
        }
        if self.file_diff_row_len() == 0 {
            return Some(ZeroRowNotice::EmptyFile {
                renamed: self.patch_is_a_rename(),
            });
        }
        let collapsed_to_nothing = self.is_collapsed_diff_projection_active()
            && self.collapsed_diff_visible_rows.is_empty();
        collapsed_to_nothing.then(|| ZeroRowNotice::NoChangedLines {
            compared_with: self
                .rendered_diff_target()
                .and_then(DiffTarget::old_file_path)
                .map(|old_path| self.cached_path_display(old_path)),
            renamed: self.patch_is_a_rename(),
        })
    }

    fn patch_is_a_rename(&self) -> bool {
        self.rendered_patch_diff_cache_is_current()
            && (0..self.patch_diff_row_len())
                .map_while(|row_ix| self.patch_diff_row(row_ix))
                .take_while(|line| line.kind != DiffLineKind::Hunk)
                .any(|line| is_rename_source_header(line.text.as_ref()))
    }

    pub(in crate::view) fn collapsed_difftastic_has_only_text_changes(&self) -> bool {
        self.is_collapsed_diff_projection_active()
            && matches!(
                self.diff_engine_status,
                DiffEngineStatus::Difftastic {
                    syntactic_changes: false,
                    text_differs: true,
                }
            )
    }
}
