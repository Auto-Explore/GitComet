//! One changed file's row and one directory row, shared by every
//! changed-file list: the details pane's commit, worktree, and range lists
//! and extensions' hosted lists. Each list supplies its ids and adds its own
//! click actions.

use super::FileListRow;
use crate::theme::AppTheme;
use crate::view::components;
use gitcomet_core::domain::CommitFileChange;
use gpui::prelude::*;
use gpui::{AnyElement, Div, ElementId, SharedString, Stateful, div, px};
use std::path::Path;
use std::sync::Arc;

/// A file row's inputs. `selector` names the row in debug builds only.
pub(in crate::view) struct ChangedFileRow<'a, S> {
    pub(in crate::view) element_id: ElementId,
    pub(in crate::view) row_group: SharedString,
    pub(in crate::view) selector: S,
    pub(in crate::view) file: &'a CommitFileChange,
    pub(in crate::view) presentation: &'a crate::view::rows::CommitFileRowPresentation,
    pub(in crate::view) is_tree: bool,
    pub(in crate::view) depth: usize,
    pub(in crate::view) selected: bool,
    pub(in crate::view) context_menu_active: bool,
    pub(in crate::view) path_alignment_group: Option<components::PathTruncationAlignmentGroup>,
    pub(in crate::view) diff_stat: bool,
    /// Drawn before the file icon (a hosted list's mark glyphs).
    pub(in crate::view) leading: Option<AnyElement>,
}

/// A file row's body; the caller adds its click action and tooltip (the
/// returned label).
pub(in crate::view) fn changed_file_row<V: 'static, S: FnOnce() -> String + 'static>(
    row: ChangedFileRow<'_, S>,
    theme: AppTheme,
    ui_scale_percent: u32,
    cx: &gpui::Context<V>,
) -> (Stateful<Div>, SharedString) {
    let ChangedFileRow {
        element_id,
        row_group,
        selector,
        file: f,
        presentation,
        is_tree,
        depth,
        selected,
        context_menu_active,
        path_alignment_group,
        diff_stat,
        leading,
    } = row;
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
    let visuals = presentation.visuals;
    let path_label = if is_tree {
        SharedString::from(
            f.path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| presentation.label.to_string()),
        )
    } else {
        presentation.label.clone()
    };
    let (icon, color) = if f.is_submodule {
        (visuals.icon, visuals.color(&theme))
    } else {
        crate::view::rows::file_row_icon(&f.path, f.kind, &theme)
    };
    // The change kind rides the row wash and a badge on the icon's corner.
    let tint = crate::view::rows::file_kind_row_tint(f.kind, &theme);
    let badge = crate::view::rows::file_row_kind_badge(f.kind, &theme);
    let interaction =
        crate::view::rows::FileRowInteraction::new(theme, tint, selected, context_menu_active);
    let badge_disc = interaction.badge_disc(row_group.clone());
    let tooltip = path_label.clone();

    let element = div()
        .id(element_id)
        // Only so the badge disc can follow the row's hover fill.
        .group(row_group)
        .debug_selector(selector)
        .h(crate::view::rows::sidebar::sidebar_list_row_height(
            theme,
            ui_scale_percent,
        ))
        .flex()
        .items_center()
        .gap(scaled_px(8.0))
        .pl(if is_tree {
            crate::view::rows::file_row_indent_px(depth, ui_scale_percent)
        } else {
            scaled_px(8.0)
        })
        .pr(scaled_px(8.0))
        .w_full()
        .map(|row| interaction.apply(row))
        .children(leading)
        .child(crate::view::rows::file_row_icon_slot(
            icon,
            color,
            badge,
            badge_disc,
            14.0,
            16.0,
            ui_scale_percent,
        ))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(theme.ui_text(14.0))
                .line_height(theme.ui_text(18.0))
                .line_clamp(1)
                .whitespace_nowrap()
                .child(
                    match path_alignment_group {
                        Some(group) => components::TruncatedText::aligned_path(
                            path_label,
                            theme.ui_text(14.0),
                            group,
                        ),
                        // A tree row's label is a bare file name, so there is
                        // no path to align against.
                        None => components::TruncatedText::new(path_label, theme.ui_text(14.0)),
                    }
                    .render(cx),
                ),
        )
        .when_some(f.large_file.as_ref(), |row, state| {
            row.child(components::large_file_chip(
                theme,
                ui_scale_percent,
                state,
                false,
            ))
        })
        .when(
            diff_stat && (f.additions.is_some() || f.deletions.is_some()),
            |row| {
                row.child(div().flex_none().child(components::diff_stat(
                    theme,
                    ui_scale_percent,
                    f.additions.unwrap_or(0) as usize,
                    f.deletions.unwrap_or(0) as usize,
                )))
            },
        );
    (element, tooltip)
}

/// What clicking a directory row toggles.
pub(in crate::view) struct DirectoryToggle {
    pub(in crate::view) key: Arc<Path>,
    pub(in crate::view) chain: super::DirChain,
    pub(in crate::view) collapsed: bool,
}

/// A directory row's element (the caller adds its toggle action), or `None`
/// for a file row. `menu_open` keeps it lit while its context menu is up.
pub(in crate::view) fn changed_file_directory_row(
    element_id: ElementId,
    selector: impl FnOnce() -> String + 'static,
    row: FileListRow,
    menu_open: bool,
    theme: AppTheme,
    ui_scale_percent: u32,
) -> Option<(Stateful<Div>, DirectoryToggle)> {
    let FileListRow::Directory {
        key,
        label,
        depth,
        collapsed,
        chain,
        subtree: _,
        additions,
        deletions,
    } = row
    else {
        return None;
    };
    let element = super::directory_row(super::DirectoryRowProps {
        theme,
        ui_scale_percent,
        id: element_id,
        label: &label,
        depth,
        collapsed,
        additions,
        deletions,
        row_height: crate::view::rows::sidebar::sidebar_list_row_height(theme, ui_scale_percent),
        row_group: None,
        detail: super::directory_row_detail_for_width(
            // No width probe on these lists.
            gpui::Pixels::MAX,
            depth,
            additions.is_some() || deletions.is_some(),
            ui_scale_percent,
        ),
        menu_open,
    })
    .debug_selector(selector);
    Some((
        element,
        DirectoryToggle {
            key,
            chain,
            collapsed,
        },
    ))
}
