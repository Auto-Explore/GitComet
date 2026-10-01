//! Status-section layout: header action labels, action selection, split
//! heights and resizing, and the per-section file lists.

use super::*;

pub(super) const STATUS_SECTION_MIN_HEIGHT_PX: f32 = 80.0;

/// The "Unstaged" / "Untracked" chip: the section's click target, so its hit
/// area follows the header bar's density ramp.
pub(super) const CHANGE_TRACKING_HEADER_CHIP_HEIGHT_PX: f32 = 18.0;
pub(super) const CHANGE_TRACKING_HEADER_CHIP_COMFORTABLE_HEIGHT_PX: f32 = 28.0;

pub(super) fn min_change_tracking_stack_height(
    split_change_tracking: bool,
    handle_h: Pixels,
) -> Pixels {
    let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
    if split_change_tracking {
        section_min_h * 2.0 + handle_h
    } else {
        section_min_h
    }
}

pub(super) fn clamp_vertical_split_height(
    requested_top: Pixels,
    total_height: Pixels,
    min_top: Pixels,
    min_bottom: Pixels,
) -> Pixels {
    if total_height <= px(0.0) {
        return px(0.0);
    }

    let min_total = min_top + min_bottom;
    if total_height <= min_total {
        return (total_height - min_bottom).max(px(0.0));
    }

    requested_top.max(min_top).min(total_height - min_bottom)
}

pub(super) fn resolved_vertical_split_height(
    requested_top: Option<Pixels>,
    total_height: Pixels,
    min_top: Pixels,
    min_bottom: Pixels,
) -> Pixels {
    if total_height <= px(0.0) {
        return px(0.0);
    }

    let default_top = (total_height * 0.5)
        .max(min_top)
        .min((total_height - min_bottom).max(px(0.0)));
    clamp_vertical_split_height(
        requested_top.unwrap_or(default_top),
        total_height,
        min_top,
        min_bottom,
    )
}

/// Which wording the changed-files section headers draw. The full labels stop
/// fitting once the details pane is dragged narrow, and an over-wide action
/// group shoves the section title out of the panel — so the labels collapse to
/// initials before that happens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatusActionLabels {
    Full,
    Compact,
}

/// Average glyph advances at `text_sm` (14px at 100% zoom), regular and bold.
/// Budgeted rather than measured, the same way the history columns decide what
/// to drop (`view/panes/history.rs`).
///
/// Per character at the default 14px UI font, measured not guessed: at 100%
/// zoom `Stage all changes` renders 108px of ink over 17 characters (6.35/char)
/// and the bold `Unstaged` renders 59px over 8 (7.4/char). Both go through
/// `ui_text`, so a larger UI font widens the budget with the ink. Keep them
/// honest: a budget that runs long silently withholds the full wording.
const STATUS_ACTION_CHAR_WIDTH_PX: f32 = 6.4;
const STATUS_HEADER_TITLE_CHAR_WIDTH_PX: f32 = 7.4;
/// The header's own `px_2`, and the `gap_2` between the title and the action
/// group and between the buttons themselves.
const STATUS_HEADER_PAD_X_PX: f32 = 8.0;
const STATUS_HEADER_GAP_PX: f32 = 8.0;
/// A change-tracking dropdown title's `px_1` either side, its `gap_1`, and the
/// 12px chevron.
const STATUS_HEADER_DROPDOWN_EXTRA_PX: f32 = 24.0;
const STATUS_HEADER_SPINNER_PX: f32 = 14.0;
/// The layout toggle and sort menu: two icon-only buttons (`control_pad_x` each
/// side plus a 1px border and a 14px icon), their `gap_1`, and the `gap_2` to
/// the action buttons beside them.
const STATUS_HEADER_CONTROLS_PX: f32 =
    2.0 * (2.0 * components::CONTROL_PAD_X_PX + 2.0 + 14.0) + 4.0 + STATUS_HEADER_GAP_PX;

fn status_action_button_width_px(
    label_chars: usize,
    metrics: crate::appearance::Appearance,
) -> f32 {
    // `control_pad_x` each side, plus the 1px border every style reserves.
    2.0 * components::CONTROL_PAD_X_PX
        + 2.0
        + label_chars as f32 * metrics.ui_text(STATUS_ACTION_CHAR_WIDTH_PX)
}

pub(super) fn status_action_labels_for_width(
    available_width: Pixels,
    title_chars: usize,
    title_is_dropdown: bool,
    action_label_chars: &[usize],
    has_spinner: bool,
    ui_scale_percent: u32,
    metrics: crate::appearance::Appearance,
) -> StatusActionLabels {
    if available_width <= px(0.0) || action_label_chars.is_empty() {
        return StatusActionLabels::Full;
    }

    let mut needed = 2.0 * STATUS_HEADER_PAD_X_PX
        + title_chars as f32 * metrics.ui_text(STATUS_HEADER_TITLE_CHAR_WIDTH_PX)
        + if title_is_dropdown {
            STATUS_HEADER_DROPDOWN_EXTRA_PX
        } else {
            0.0
        }
        // Between the title and the action group.
        + STATUS_HEADER_GAP_PX
        + STATUS_HEADER_CONTROLS_PX;
    if has_spinner {
        needed += STATUS_HEADER_SPINNER_PX + STATUS_HEADER_GAP_PX;
    }
    for (ix, chars) in action_label_chars.iter().enumerate() {
        if ix > 0 {
            needed += STATUS_HEADER_GAP_PX;
        }
        needed += status_action_button_width_px(*chars, metrics);
    }

    if crate::ui_scale::design_px_from_percent(needed, ui_scale_percent) <= available_width {
        StatusActionLabels::Full
    } else {
        StatusActionLabels::Compact
    }
}

/// Clipped forms of the action verbs. A bare initial was ambiguous — `S` and
/// `U` read as the same family, and the two of them plus `D` gave no clue which
/// button did what. These stay pronounceable at a glance.
fn status_action_short_word(word: &'static str) -> &'static str {
    match word {
        "Stage" => "Stg",
        "Discard" => "Disc",
        "Unstage" => "Ustg",
        other => other,
    }
}

/// `Stage (3)` → `Stg (3)`, `Stage all changes` → `All`.
pub(super) fn status_action_count_label(
    labels: StatusActionLabels,
    word: &'static str,
    count: usize,
) -> String {
    match labels {
        StatusActionLabels::Full => format!("{word} ({count})"),
        StatusActionLabels::Compact => {
            format!("{} ({count})", status_action_short_word(word))
        }
    }
}

pub(super) fn status_action_all_label(
    labels: StatusActionLabels,
    full: &'static str,
) -> &'static str {
    match labels {
        StatusActionLabels::Full => full,
        StatusActionLabels::Compact => "All",
    }
}

pub(super) fn status_action_file_count(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct StatusSectionActionSelection {
    pub(super) paths: Vec<std::path::PathBuf>,
    pub(super) from_explicit_selection: bool,
}

impl StatusSectionActionSelection {
    pub(super) fn popover_path(&self) -> Option<std::path::PathBuf> {
        (!self.from_explicit_selection && self.paths.len() == 1).then(|| self.paths[0].clone())
    }
}

fn explicit_status_section_action_paths(
    selection: &StatusMultiSelection,
    section: StatusSection,
) -> Vec<std::path::PathBuf> {
    match section {
        StatusSection::CombinedUnstaged => selection
            .selected_paths_for_area(DiffArea::Unstaged)
            .to_vec(),
        StatusSection::Untracked => selection.untracked.clone(),
        StatusSection::Unstaged => selection.unstaged.clone(),
        StatusSection::Staged => selection.staged.clone(),
    }
}

fn active_status_section_action_path(
    repo: &RepoState,
    diff_target: Option<&DiffTarget>,
    section: StatusSection,
) -> Option<std::path::PathBuf> {
    let DiffTarget::WorkingTree { path, area, .. } = diff_target? else {
        return None;
    };
    if *area != section.diff_area() {
        return None;
    }

    StatusSectionEntries::from_repo(repo, section)
        .is_some_and(|entries| entries.contains_path(path.as_path()))
        .then(|| path.clone())
}

pub(super) fn status_section_action_selection(
    repo: &RepoState,
    diff_target: Option<&DiffTarget>,
    selection: Option<&StatusMultiSelection>,
    section: StatusSection,
) -> StatusSectionActionSelection {
    if let Some(selection) = selection {
        let paths = explicit_status_section_action_paths(selection, section);
        if selection.explicit_section == Some(section) || !paths.is_empty() {
            return StatusSectionActionSelection {
                paths,
                from_explicit_selection: true,
            };
        }
    }

    active_status_section_action_path(repo, diff_target, section)
        .map(|path| StatusSectionActionSelection {
            paths: vec![path],
            from_explicit_selection: false,
        })
        .unwrap_or_default()
}

/// How many paths `status_section_action_selection` picks, without building
/// the list: the header reads it on every render of the details pane.
pub(super) fn status_section_action_count(
    repo: &RepoState,
    diff_target: Option<&DiffTarget>,
    selection: Option<&StatusMultiSelection>,
    section: StatusSection,
) -> usize {
    if let Some(selection) = selection {
        let count = match section {
            StatusSection::CombinedUnstaged => {
                selection.selected_paths_for_area(DiffArea::Unstaged).len()
            }
            StatusSection::Untracked => selection.untracked.len(),
            StatusSection::Unstaged => selection.unstaged.len(),
            StatusSection::Staged => selection.staged.len(),
        };
        if selection.explicit_section == Some(section) || count > 0 {
            return count;
        }
    }
    let Some(DiffTarget::WorkingTree { path, area, .. }) = diff_target else {
        return 0;
    };
    if *area != section.diff_area() {
        return 0;
    }
    // One lookup instead of a filtered copy of the section per call.
    let Some(entry) = repo.status_entry_for_path(*area, path.as_path()) else {
        return 0;
    };
    usize::from(match section {
        StatusSection::Untracked => entry.kind == FileStatusKind::Untracked,
        StatusSection::Unstaged => entry.kind != FileStatusKind::Untracked,
        StatusSection::CombinedUnstaged | StatusSection::Staged => true,
    })
}

impl DetailsPaneView {
    pub(in crate::view) fn handle_status_section_shortcut(
        &mut self,
        section: StatusSection,
        keystroke: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if !is_status_section_shortcut(keystroke)
            || !self.status_section_focus_handle(section).is_focused(window)
        {
            return false;
        }
        let Some(repo) = self.active_repo() else {
            return true;
        };
        let repo_id = repo.id;
        let area = section.diff_area();
        let loading = match area {
            DiffArea::Unstaged => repo.worktree_status_is_loading(),
            DiffArea::Staged => repo.staged_status_is_loading(),
        };
        if loading || StatusSectionEntries::from_repo(repo, section).is_none() {
            return true;
        }
        if keystroke.key == "a" {
            let paths = self.status_display_order_paths(repo_id, section);
            let order_rev = self.status_anchor_order_rev(repo, section);
            self.status_multi_selection
                .entry(repo_id)
                .or_default()
                .select_all(section, paths, order_rev);
            cx.notify();
            return true;
        }
        if repo.local_actions_in_flight > 0
            || (keystroke.key == "s" && area != DiffArea::Unstaged)
            || (keystroke.key == "u" && area != DiffArea::Staged)
        {
            return true;
        }
        let paths = self.status_section_action_selection(repo_id, section).paths;
        // Empty path lists mean "all" to the backend, never "none".
        if paths.is_empty() {
            return true;
        }
        match area {
            DiffArea::Unstaged => {
                self.stage_all_with_conflict_confirmation(repo_id, paths, window, cx)
            }
            DiffArea::Staged => {
                self.clear_status_multi_selection(repo_id);
                crate::view::status_actions::stage_or_unstage_paths(
                    &self.store,
                    repo_id,
                    DiffArea::Staged,
                    paths,
                );
                cx.notify();
            }
        }
        true
    }

    pub(super) fn status_section_action_selection(
        &self,
        repo_id: RepoId,
        section: StatusSection,
    ) -> StatusSectionActionSelection {
        let Some(repo) = self.active_repo().filter(|repo| repo.id == repo_id) else {
            return StatusSectionActionSelection::default();
        };

        status_section_action_selection(
            repo,
            repo.diff_state.diff_target.as_ref(),
            self.status_multi_selection.get(&repo_id),
            section,
        )
    }

    pub(super) fn status_section_action_count(
        &self,
        repo_id: RepoId,
        section: StatusSection,
    ) -> usize {
        let Some(repo) = self.active_repo().filter(|repo| repo.id == repo_id) else {
            return 0;
        };
        status_section_action_count(
            repo,
            repo.diff_state.diff_target.as_ref(),
            self.status_multi_selection.get(&repo_id),
            section,
        )
    }

    pub(super) fn take_status_section_action_selection(
        &mut self,
        repo_id: RepoId,
        section: StatusSection,
    ) -> StatusSectionActionSelection {
        let selection = self.status_section_action_selection(repo_id, section);
        if selection.from_explicit_selection {
            self.status_multi_selection.remove(&repo_id);
        }
        selection
    }

    pub(super) fn measured_status_sections_total_height(
        &self,
        resize_handle_h: Pixels,
    ) -> Option<Pixels> {
        self.current_status_sections_bounds()
            .map(|bounds| (bounds.size.height - resize_handle_h).max(px(0.0)))
    }

    fn resolved_measured_change_tracking_section_height(
        &self,
        resize_handle_h: Pixels,
    ) -> Option<Pixels> {
        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let min_height = min_change_tracking_stack_height(
            self.change_tracking_view == ChangeTrackingView::SplitUntracked,
            resize_handle_h,
        );

        self.measured_status_sections_total_height(resize_handle_h)
            .map(|total_height| {
                resolved_vertical_split_height(
                    self.change_tracking_height,
                    total_height,
                    min_height,
                    section_min_h,
                )
            })
    }

    pub(super) fn resolved_measured_change_tracking_stack_total_height(
        &self,
        resize_handle_h: Pixels,
    ) -> Option<Pixels> {
        self.resolved_measured_change_tracking_section_height(resize_handle_h)
            .map(|section_height| (section_height - resize_handle_h).max(px(0.0)))
            .or_else(|| {
                self.current_change_tracking_stack_bounds()
                    .map(|bounds| (bounds.size.height - resize_handle_h).max(px(0.0)))
            })
    }

    pub(in crate::view) fn sanitized_restored_change_tracking_height_design(
        view: ChangeTrackingView,
        height: Option<u32>,
    ) -> Option<f32> {
        let min_height: f32 = min_change_tracking_stack_height(
            view == ChangeTrackingView::SplitUntracked,
            px(PANE_RESIZE_HANDLE_PX),
        )
        .into();
        height.map(|value| (value as f32).max(min_height))
    }

    #[cfg(test)]
    pub(in crate::view) fn sanitized_restored_change_tracking_height(
        view: ChangeTrackingView,
        height: Option<u32>,
    ) -> Option<Pixels> {
        Self::sanitized_restored_change_tracking_height_design(view, height).map(px)
    }

    pub(in crate::view) fn sanitized_restored_untracked_height_design(
        height: Option<u32>,
    ) -> Option<f32> {
        height.map(|value| (value as f32).max(STATUS_SECTION_MIN_HEIGHT_PX))
    }

    #[cfg(test)]
    pub(in crate::view) fn sanitized_restored_untracked_height(
        height: Option<u32>,
    ) -> Option<Pixels> {
        Self::sanitized_restored_untracked_height_design(height).map(px)
    }

    fn status_resize_total_height(
        &self,
        handle: StatusSectionResizeHandle,
        resize_handle_h: Pixels,
    ) -> Option<Pixels> {
        match handle {
            StatusSectionResizeHandle::ChangeTrackingAndStaged => {
                self.measured_status_sections_total_height(resize_handle_h)
            }
            StatusSectionResizeHandle::UntrackedAndUnstaged => {
                self.resolved_measured_change_tracking_stack_total_height(resize_handle_h)
            }
        }
    }

    pub(super) fn start_status_section_resize(
        &mut self,
        handle: StatusSectionResizeHandle,
        start_y: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let resize_handle_h = px(PANE_RESIZE_HANDLE_PX);
        let total_height = self.status_resize_total_height(handle, resize_handle_h);
        let start_height = match handle {
            StatusSectionResizeHandle::ChangeTrackingAndStaged => total_height
                .map(|total_height| {
                    resolved_vertical_split_height(
                        self.change_tracking_height,
                        total_height,
                        min_change_tracking_stack_height(
                            self.change_tracking_view == ChangeTrackingView::SplitUntracked,
                            resize_handle_h,
                        ),
                        section_min_h,
                    )
                })
                .or(self.change_tracking_height)
                .unwrap_or(section_min_h),
            StatusSectionResizeHandle::UntrackedAndUnstaged => total_height
                .map(|total_height| {
                    resolved_vertical_split_height(
                        self.untracked_height,
                        total_height,
                        section_min_h,
                        section_min_h,
                    )
                })
                .or(self.untracked_height)
                .unwrap_or(section_min_h),
        };

        self.status_section_resize = Some(StatusSectionResizeState {
            handle,
            start_y,
            start_height,
        });
        cx.notify();
    }

    pub(in crate::view) fn update_status_section_resize(
        &mut self,
        current_y: Pixels,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(state) = self.status_section_resize else {
            return false;
        };

        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let resize_handle_h = px(PANE_RESIZE_HANDLE_PX);
        let total_height = self.status_resize_total_height(state.handle, resize_handle_h);

        let delta_y = current_y - state.start_y;
        let mut changed = false;
        match state.handle {
            StatusSectionResizeHandle::ChangeTrackingAndStaged => {
                let min_top = min_change_tracking_stack_height(
                    self.change_tracking_view == ChangeTrackingView::SplitUntracked,
                    resize_handle_h,
                );
                let next_height = if let Some(total_height) = total_height {
                    clamp_vertical_split_height(
                        state.start_height + delta_y,
                        total_height,
                        min_top,
                        section_min_h,
                    )
                } else {
                    (state.start_height + delta_y).max(min_top)
                };
                if self.change_tracking_height != Some(next_height) {
                    self.set_change_tracking_height_from_pixels(Some(next_height));
                    changed = true;
                }
            }
            StatusSectionResizeHandle::UntrackedAndUnstaged => {
                let next_height = if let Some(total_height) = total_height {
                    clamp_vertical_split_height(
                        state.start_height + delta_y,
                        total_height,
                        section_min_h,
                        section_min_h,
                    )
                } else {
                    (state.start_height + delta_y).max(section_min_h)
                };
                if self.untracked_height != Some(next_height) {
                    self.set_untracked_height_from_pixels(Some(next_height));
                    changed = true;
                }
            }
        }

        if changed {
            cx.notify();
        }
        changed
    }

    pub(in crate::view) fn finish_status_section_resize(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.status_section_resize.take().is_some() {
            let pane = cx.entity();
            self.schedule_ui_settings_persist(cx);
            cx.notify();
            cx.defer(move |cx| {
                pane.update(cx, |_this, cx| {
                    cx.notify();
                });
            });
            true
        } else {
            false
        }
    }

    pub(in crate::view) fn status_list(
        &mut self,
        cx: &mut gpui::Context<Self>,
        section: StatusSection,
        count: usize,
    ) -> AnyElement {
        let theme = self.theme;
        if count == 0 {
            return components::empty_state_message(theme, "Working tree clean.")
                .into_any_element();
        }
        // `count` is the file count the header shows; the list is indexed in
        // display rows, which a tree pads with directories.
        let count = self
            .active_repo()
            .map(|repo| self.status_file_plan(repo, section).row_len())
            .unwrap_or(count);
        let Some(repo_id) = self.active_repo_id() else {
            return div().into_any_element();
        };
        let (container, scrollbar, scroll) = match section {
            StatusSection::CombinedUnstaged => (
                "unstaged_scroll_container",
                "unstaged_scrollbar",
                &self.unstaged_scroll,
            ),
            StatusSection::Untracked => (
                "untracked_scroll_container",
                "untracked_scrollbar",
                &self.untracked_scroll,
            ),
            StatusSection::Unstaged => (
                "split_unstaged_scroll_container",
                "split_unstaged_scrollbar",
                &self.unstaged_scroll,
            ),
            StatusSection::Staged => (
                "staged_scroll_container",
                "staged_scrollbar",
                &self.staged_scroll,
            ),
        };
        let list = self.changed_file_list(
            repo_id,
            crate::view::rows::FileListId::Status(section),
            count,
            cx,
        );
        Self::vertical_scroll_frame_content(theme, container, scrollbar, scroll, list)
            .into_any_element()
    }
}
