use super::color::with_alpha;
use crate::theme::AppTheme;
use gitcomet_state::model::{CloneOpState, CloneOpStatus, CloneProgressStage};
use std::path::Path;

pub(crate) fn clone_progress_loading_color(theme: AppTheme) -> gpui::Rgba {
    with_alpha(theme.colors.text, if theme.is_dark { 0.42 } else { 0.34 })
}

pub(crate) fn clone_progress_title(op: &CloneOpState) -> &'static str {
    match op.status {
        CloneOpStatus::Cancelling => "Aborting clone…",
        _ => "Cloning repository…",
    }
}

pub(crate) fn clone_progress_phase_label(op: &CloneOpState) -> &'static str {
    match op.status {
        CloneOpStatus::Cancelling => "Stopping clone",
        _ => match op.progress.stage {
            CloneProgressStage::Loading => "Loading",
            CloneProgressStage::RemoteObjects => "Remote objects",
        },
    }
}

pub(crate) fn clone_progress_color(theme: AppTheme, op: &CloneOpState) -> gpui::Rgba {
    match op.status {
        CloneOpStatus::Cancelling => {
            with_alpha(theme.colors.text, if theme.is_dark { 0.60 } else { 0.48 })
        }
        _ => match op.progress.stage {
            CloneProgressStage::Loading => clone_progress_loading_color(theme),
            CloneProgressStage::RemoteObjects => {
                with_alpha(theme.colors.text, if theme.is_dark { 0.78 } else { 0.62 })
            }
        },
    }
}

pub(crate) fn clone_progress_bar_fill_color(theme: AppTheme, op: &CloneOpState) -> gpui::Rgba {
    match op.status {
        CloneOpStatus::Cancelling => with_alpha(
            theme.colors.warning,
            if theme.is_dark { 0.92 } else { 0.84 },
        ),
        _ => match op.progress.stage {
            CloneProgressStage::Loading => {
                with_alpha(theme.colors.accent, if theme.is_dark { 0.82 } else { 0.74 })
            }
            CloneProgressStage::RemoteObjects => {
                with_alpha(theme.colors.accent, if theme.is_dark { 0.94 } else { 0.86 })
            }
        },
    }
}

pub(crate) fn clone_progress_bar_track_color(theme: AppTheme) -> gpui::Rgba {
    with_alpha(theme.colors.border, if theme.is_dark { 0.40 } else { 0.22 })
}

pub(crate) fn clone_progress_bar_border_color(theme: AppTheme) -> gpui::Rgba {
    with_alpha(theme.colors.border, if theme.is_dark { 0.72 } else { 0.42 })
}

pub(crate) fn clone_progress_fill_ratio(percent: u8) -> f32 {
    f32::from(percent.min(100)) / 100.0
}

pub(crate) fn clone_progress_segment_weights(percent: u8) -> (f32, f32) {
    let fill = clone_progress_fill_ratio(percent);
    (fill, (1.0 - fill).max(0.0))
}

pub(crate) fn clone_progress_dest_label(dest: &Path) -> String {
    dest.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::with_alpha;
    use gitcomet_state::model::{CloneProgressMeter, CloneProgressStage};
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn clone_op(status: CloneOpStatus, stage: CloneProgressStage, percent: u8) -> CloneOpState {
        CloneOpState {
            url: Arc::<str>::from("file:///tmp/repo.git"),
            dest: Arc::new(PathBuf::from("/tmp/repo")),
            status,
            progress: CloneProgressMeter { stage, percent },
            seq: 1,
            output_tail: VecDeque::new(),
        }
    }

    #[test]
    fn clone_progress_copy_uses_stage_labels_for_running_clone() {
        let loading = clone_op(CloneOpStatus::Running, CloneProgressStage::Loading, 18);
        let remote = clone_op(
            CloneOpStatus::Running,
            CloneProgressStage::RemoteObjects,
            82,
        );

        assert_eq!(clone_progress_title(&loading), "Cloning repository…");
        assert_eq!(clone_progress_phase_label(&loading), "Loading");
        assert_eq!(clone_progress_phase_label(&remote), "Remote objects");
    }

    #[test]
    fn clone_progress_copy_switches_to_abort_language_when_cancelling() {
        let op = clone_op(
            CloneOpStatus::Cancelling,
            CloneProgressStage::RemoteObjects,
            64,
        );

        assert_eq!(clone_progress_title(&op), "Aborting clone…");
        assert_eq!(clone_progress_phase_label(&op), "Stopping clone");
    }

    #[test]
    fn clone_progress_fill_width_clamps_to_bar_bounds() {
        assert_eq!(clone_progress_fill_ratio(0), 0.0);
        assert_eq!(clone_progress_fill_ratio(50), 0.5);
        assert_eq!(clone_progress_fill_ratio(255), 1.0);
    }

    #[test]
    fn clone_progress_segment_weights_split_fill_and_remainder() {
        assert_eq!(clone_progress_segment_weights(0), (0.0, 1.0));
        assert_eq!(clone_progress_segment_weights(50), (0.5, 0.5));
        assert_eq!(clone_progress_segment_weights(255), (1.0, 0.0));
    }

    #[test]
    fn clone_progress_color_uses_neutral_light_theme_alphas() {
        let theme = AppTheme::gitcomet_light();
        let loading = clone_op(CloneOpStatus::Running, CloneProgressStage::Loading, 10);
        let remote = clone_op(
            CloneOpStatus::Running,
            CloneProgressStage::RemoteObjects,
            75,
        );
        let cancelling = clone_op(CloneOpStatus::Cancelling, CloneProgressStage::Loading, 75);

        assert_eq!(
            clone_progress_color(theme, &loading),
            with_alpha(theme.colors.text, 0.34)
        );
        assert_eq!(
            clone_progress_color(theme, &remote),
            with_alpha(theme.colors.text, 0.62)
        );
        assert_eq!(
            clone_progress_color(theme, &cancelling),
            with_alpha(theme.colors.text, 0.48)
        );
        assert_eq!(
            clone_progress_loading_color(theme),
            with_alpha(theme.colors.text, 0.34)
        );
        assert_eq!(
            clone_progress_bar_fill_color(theme, &loading),
            with_alpha(theme.colors.accent, 0.74)
        );
        assert_eq!(
            clone_progress_bar_fill_color(theme, &remote),
            with_alpha(theme.colors.accent, 0.86)
        );
        assert_eq!(
            clone_progress_bar_fill_color(theme, &cancelling),
            with_alpha(theme.colors.warning, 0.84)
        );
        assert_eq!(
            clone_progress_bar_track_color(theme),
            with_alpha(theme.colors.border, 0.22)
        );
        assert_eq!(
            clone_progress_bar_border_color(theme),
            with_alpha(theme.colors.border, 0.42)
        );
    }

    #[test]
    fn clone_progress_color_uses_neutral_dark_theme_alphas() {
        let theme = AppTheme::gitcomet_dark();
        let loading = clone_op(CloneOpStatus::Running, CloneProgressStage::Loading, 10);
        let remote = clone_op(
            CloneOpStatus::Running,
            CloneProgressStage::RemoteObjects,
            75,
        );
        let cancelling = clone_op(CloneOpStatus::Cancelling, CloneProgressStage::Loading, 75);

        assert_eq!(
            clone_progress_color(theme, &loading),
            with_alpha(theme.colors.text, 0.42)
        );
        assert_eq!(
            clone_progress_color(theme, &remote),
            with_alpha(theme.colors.text, 0.78)
        );
        assert_eq!(
            clone_progress_color(theme, &cancelling),
            with_alpha(theme.colors.text, 0.60)
        );
        assert_eq!(
            clone_progress_loading_color(theme),
            with_alpha(theme.colors.text, 0.42)
        );
        assert_eq!(
            clone_progress_bar_fill_color(theme, &loading),
            with_alpha(theme.colors.accent, 0.82)
        );
        assert_eq!(
            clone_progress_bar_fill_color(theme, &remote),
            with_alpha(theme.colors.accent, 0.94)
        );
        assert_eq!(
            clone_progress_bar_fill_color(theme, &cancelling),
            with_alpha(theme.colors.warning, 0.92)
        );
        assert_eq!(
            clone_progress_bar_track_color(theme),
            with_alpha(theme.colors.border, 0.40)
        );
        assert_eq!(
            clone_progress_bar_border_color(theme),
            with_alpha(theme.colors.border, 0.72)
        );
    }

    #[test]
    fn clone_progress_dest_label_uses_display_path() {
        assert_eq!(
            clone_progress_dest_label(Path::new("/tmp/example repo")),
            "/tmp/example repo"
        );
    }
}
