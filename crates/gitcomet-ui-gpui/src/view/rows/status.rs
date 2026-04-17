use super::*;
use std::sync::Arc;
#[cfg(any(debug_assertions, feature = "benchmarks"))]
use std::sync::atomic::{AtomicU64, Ordering};

const STATUS_ROW_HEIGHT_PX: f32 = 24.0;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct StatusSelectionBenchSnapshot {
    pub position_scan_steps: u64,
}

#[cfg(test)]
pub(in crate::view) fn bench_snapshot_status_selection() -> StatusSelectionBenchSnapshot {
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    {
        StatusSelectionBenchSnapshot {
            position_scan_steps: STATUS_SELECTION_POSITION_SCAN_STEPS.load(Ordering::Relaxed),
        }
    }
    #[cfg(not(any(debug_assertions, feature = "benchmarks")))]
    {
        StatusSelectionBenchSnapshot::default()
    }
}

#[cfg(test)]
pub(in crate::view) fn bench_reset_status_selection() {
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    {
        STATUS_SELECTION_POSITION_SCAN_STEPS.store(0, Ordering::Relaxed);
    }
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
static STATUS_SELECTION_POSITION_SCAN_STEPS: AtomicU64 = AtomicU64::new(0);

struct StatusMultiSelectionSlice<'a> {
    selected: &'a mut Vec<std::path::PathBuf>,
    anchor: &'a mut Option<std::path::PathBuf>,
    anchor_index: &'a mut Option<usize>,
    anchor_status_rev: &'a mut Option<u64>,
}

fn set_status_multi_selection_single(
    selection: StatusMultiSelectionSlice<'_>,
    clicked_path: std::path::PathBuf,
    clicked_index: Option<usize>,
    status_rev: Option<u64>,
) {
    selection.selected.clear();
    selection.selected.push(clicked_path.clone());
    *selection.anchor = Some(clicked_path);
    *selection.anchor_index = clicked_index;
    *selection.anchor_status_rev = status_rev;
}

fn apply_status_multi_selection_to_slice(
    selection: StatusMultiSelectionSlice<'_>,
    clicked_path: std::path::PathBuf,
    clicked_index: Option<usize>,
    modifiers: gpui::Modifiers,
    status_rev: Option<u64>,
    trust_clicked_index: bool,
    entries: Option<&[std::path::PathBuf]>,
) {
    if modifiers.shift {
        let Some(entries) = entries else {
            set_status_multi_selection_single(selection, clicked_path, clicked_index, status_rev);
            return;
        };

        let Some(clicked_ix) = status_selection_entry_index(
            entries,
            clicked_path.as_path(),
            clicked_index,
            trust_clicked_index,
        ) else {
            set_status_multi_selection_single(selection, clicked_path, clicked_index, status_rev);
            return;
        };

        let anchor_ix = if let Some(anchor_path) = selection.anchor.as_deref() {
            let trust_anchor_index = status_rev
                .zip(*selection.anchor_status_rev)
                .is_some_and(|(current, anchor_rev)| current == anchor_rev);
            status_selection_entry_index(
                entries,
                anchor_path,
                *selection.anchor_index,
                trust_anchor_index,
            )
            .unwrap_or(clicked_ix)
        } else {
            clicked_ix
        };
        let (a, b) = if anchor_ix <= clicked_ix {
            (anchor_ix, clicked_ix)
        } else {
            (clicked_ix, anchor_ix)
        };
        selection.selected.clear();
        selection.selected.extend(entries[a..=b].iter().cloned());
        if selection.anchor.is_none() {
            *selection.anchor = Some(clicked_path.clone());
        }
        *selection.anchor_index = Some(anchor_ix);
        *selection.anchor_status_rev = status_rev;
        return;
    }

    if modifiers.secondary() || modifiers.control || modifiers.platform {
        if let Some(ix) = selection.selected.iter().position(|p| p == &clicked_path) {
            selection.selected.remove(ix);
            if selection.selected.is_empty() {
                *selection.anchor = None;
                *selection.anchor_index = None;
                *selection.anchor_status_rev = None;
            }
        } else {
            selection.selected.push(clicked_path.clone());
            *selection.anchor = Some(clicked_path);
            *selection.anchor_index = clicked_index;
            *selection.anchor_status_rev = status_rev;
        }
        return;
    }

    set_status_multi_selection_single(selection, clicked_path, clicked_index, status_rev);
}

fn status_selection_entry_index_hint(
    entries: &[std::path::PathBuf],
    target: &std::path::Path,
    index_hint: Option<usize>,
    trust_hint: bool,
) -> Option<usize> {
    if trust_hint {
        return index_hint.filter(|&ix| entries.get(ix).is_some());
    }
    index_hint.filter(|&ix| entries.get(ix).is_some_and(|path| path.as_path() == target))
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
fn status_selection_entry_index(
    entries: &[std::path::PathBuf],
    target: &std::path::Path,
    index_hint: Option<usize>,
    trust_hint: bool,
) -> Option<usize> {
    if let Some(ix) = status_selection_entry_index_hint(entries, target, index_hint, trust_hint) {
        return Some(ix);
    }
    for (ix, path) in entries.iter().enumerate() {
        STATUS_SELECTION_POSITION_SCAN_STEPS.fetch_add(1, Ordering::Relaxed);
        if path.as_path() == target {
            return Some(ix);
        }
    }
    None
}

#[cfg(not(any(debug_assertions, feature = "benchmarks")))]
fn status_selection_entry_index(
    entries: &[std::path::PathBuf],
    target: &std::path::Path,
    index_hint: Option<usize>,
    trust_hint: bool,
) -> Option<usize> {
    status_selection_entry_index_hint(entries, target, index_hint, trust_hint)
        .or_else(|| entries.iter().position(|path| path.as_path() == target))
}

pub(super) fn apply_status_multi_selection_click(
    selection: &mut StatusMultiSelection,
    section: StatusSection,
    clicked_path: std::path::PathBuf,
    clicked_index: Option<usize>,
    modifiers: gpui::Modifiers,
    status_rev: Option<u64>,
    trust_clicked_index: bool,
    entries: Option<&[std::path::PathBuf]>,
) {
    match section {
        StatusSection::CombinedUnstaged | StatusSection::Unstaged => {
            selection.untracked.clear();
            selection.untracked_anchor = None;
            selection.staged.clear();
            selection.staged_anchor = None;
            selection.staged_anchor_index = None;
            selection.staged_anchor_status_rev = None;
            apply_status_multi_selection_to_slice(
                StatusMultiSelectionSlice {
                    selected: &mut selection.unstaged,
                    anchor: &mut selection.unstaged_anchor,
                    anchor_index: &mut selection.unstaged_anchor_index,
                    anchor_status_rev: &mut selection.unstaged_anchor_status_rev,
                },
                clicked_path,
                clicked_index,
                modifiers,
                status_rev,
                trust_clicked_index,
                entries,
            );
        }
        StatusSection::Untracked => {
            selection.unstaged.clear();
            selection.unstaged_anchor = None;
            selection.unstaged_anchor_index = None;
            selection.unstaged_anchor_status_rev = None;
            selection.staged.clear();
            selection.staged_anchor = None;
            selection.staged_anchor_index = None;
            selection.staged_anchor_status_rev = None;
            let mut untracked_anchor_index = None;
            let mut untracked_anchor_status_rev = None;
            apply_status_multi_selection_to_slice(
                StatusMultiSelectionSlice {
                    selected: &mut selection.untracked,
                    anchor: &mut selection.untracked_anchor,
                    anchor_index: &mut untracked_anchor_index,
                    anchor_status_rev: &mut untracked_anchor_status_rev,
                },
                clicked_path,
                clicked_index,
                modifiers,
                status_rev,
                trust_clicked_index,
                entries,
            );
        }
        StatusSection::Staged => {
            selection.untracked.clear();
            selection.untracked_anchor = None;
            selection.unstaged.clear();
            selection.unstaged_anchor = None;
            selection.unstaged_anchor_index = None;
            selection.unstaged_anchor_status_rev = None;
            apply_status_multi_selection_to_slice(
                StatusMultiSelectionSlice {
                    selected: &mut selection.staged,
                    anchor: &mut selection.staged_anchor,
                    anchor_index: &mut selection.staged_anchor_index,
                    anchor_status_rev: &mut selection.staged_anchor_status_rev,
                },
                clicked_path,
                clicked_index,
                modifiers,
                status_rev,
                trust_clicked_index,
                entries,
            );
        }
    }
}

fn status_paths_for_section(repo: &RepoState, section: StatusSection) -> Vec<std::path::PathBuf> {
    StatusSectionEntries::from_repo(repo, section)
        .map_or_else(Vec::new, StatusSectionEntries::path_vec)
}

impl DetailsPaneView {
    fn clear_status_multi_selection(&mut self, repo_id: RepoId) {
        self.status_multi_selection.remove(&repo_id);
    }

    fn take_status_selected_paths_for_action(
        &mut self,
        repo_id: RepoId,
        area: DiffArea,
        clicked_path: &std::path::PathBuf,
    ) -> (Vec<std::path::PathBuf>, bool) {
        let selection = self.status_selected_paths_for_area(repo_id, area);
        let use_selection = selection.len() > 1 && selection.iter().any(|p| p == clicked_path);
        if !use_selection {
            return (vec![clicked_path.clone()], false);
        }

        let sel = self
            .status_multi_selection
            .remove(&repo_id)
            .unwrap_or_default();
        let paths = sel.take_selected_paths_for_area(area);
        if paths.is_empty() {
            (vec![clicked_path.clone()], false)
        } else {
            (paths, true)
        }
    }

    fn status_multi_selection_for_repo_mut(
        &mut self,
        repo_id: RepoId,
    ) -> &mut StatusMultiSelection {
        self.status_multi_selection.entry(repo_id).or_default()
    }

    fn status_selected_paths_for_area(
        &self,
        repo_id: RepoId,
        area: DiffArea,
    ) -> &[std::path::PathBuf] {
        let Some(sel) = self.status_multi_selection.get(&repo_id) else {
            return &[];
        };
        sel.selected_paths_for_area(area)
    }

    fn status_selection_apply_click(
        &mut self,
        repo_id: RepoId,
        section: StatusSection,
        clicked_path: std::path::PathBuf,
        clicked_index: Option<usize>,
        modifiers: gpui::Modifiers,
        entries: Option<&[std::path::PathBuf]>,
    ) {
        let status_rev = self
            .active_repo()
            .filter(|repo| repo.id == repo_id)
            .map(|repo| status_section_rev(repo, section));
        let sel = self.status_multi_selection_for_repo_mut(repo_id);
        apply_status_multi_selection_click(
            sel,
            section,
            clicked_path,
            clicked_index,
            modifiers,
            status_rev,
            true,
            entries,
        );
    }

    pub(in super::super) fn render_unstaged_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        render_status_rows_for_section(this, range, StatusSection::CombinedUnstaged, cx)
    }

    pub(in super::super) fn render_untracked_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        render_status_rows_for_section(this, range, StatusSection::Untracked, cx)
    }

    pub(in super::super) fn render_split_unstaged_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        render_status_rows_for_section(this, range, StatusSection::Unstaged, cx)
    }

    pub(in super::super) fn render_staged_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        render_status_rows_for_section(this, range, StatusSection::Staged, cx)
    }
}

fn render_status_rows_for_section(
    this: &mut DetailsPaneView,
    range: Range<usize>,
    section: StatusSection,
    cx: &mut gpui::Context<DetailsPaneView>,
) -> Vec<AnyElement> {
    let Some(repo) = this.active_repo() else {
        return Vec::new();
    };
    let Some(entries) = StatusSectionEntries::from_repo(repo, section) else {
        return Vec::new();
    };
    let selected = repo.diff_state.diff_target.as_ref();
    let selected_paths = this.status_selected_paths_for_area(repo.id, section.diff_area());
    let multi_select_active = !selected_paths.is_empty();
    let theme = this.theme;
    range
        .filter_map(|ix| entries.get(ix).map(|entry| (ix, entry)))
        .map(|(ix, entry)| {
            let path_display = this.cached_path_display(&entry.path);
            let is_selected = if multi_select_active {
                selected_paths.iter().any(|p| p == &entry.path)
            } else {
                selected.is_some_and(|t| match t {
                    DiffTarget::WorkingTree { path, area } => {
                        *area == section.diff_area() && path == &entry.path
                    }
                    _ => false,
                })
            };
            status_row(
                theme,
                ix,
                entry,
                path_display,
                section,
                repo.id,
                is_selected,
                this.active_context_menu_invoker.as_ref(),
                cx,
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn status_row(
    theme: AppTheme,
    ix: usize,
    entry: &FileStatus,
    path_display: SharedString,
    section: StatusSection,
    repo_id: RepoId,
    selected: bool,
    active_context_menu_invoker: Option<&SharedString>,
    cx: &mut gpui::Context<DetailsPaneView>,
) -> AnyElement {
    let ui_scale_percent = crate::ui_scale::current(cx).percent;
    let scaled_px = |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
    let area = section.diff_area();
    let (icon, color) = match entry.kind {
        FileStatusKind::Untracked => match area {
            DiffArea::Unstaged => ("icons/plus.svg", theme.colors.success),
            DiffArea::Staged => ("icons/question.svg", theme.colors.warning),
        },
        FileStatusKind::Modified => ("icons/pencil.svg", theme.colors.warning),
        FileStatusKind::Added => ("icons/plus.svg", theme.colors.success),
        FileStatusKind::Deleted => ("icons/minus.svg", theme.colors.danger),
        FileStatusKind::Renamed => ("icons/swap.svg", theme.colors.accent),
        FileStatusKind::Conflicted => ("icons/warning.svg", theme.colors.danger),
    };

    let path = Arc::new(entry.path.clone());
    let path_for_stage = Arc::clone(&path);
    let path_for_row = Arc::clone(&path);
    let path_for_menu = Arc::clone(&path);
    let is_conflicted = entry.kind == FileStatusKind::Conflicted;
    let stage_label = if is_conflicted {
        "Resolve…"
    } else {
        match area {
            DiffArea::Unstaged => "Stage",
            DiffArea::Staged => "Unstage",
        }
    };
    let row_tooltip = path_display.clone();
    let stage_tooltip: SharedString = match stage_label {
        "Stage" => "Stage file".into(),
        "Unstage" => "Unstage file".into(),
        "Resolve…" => "Resolve… file".into(),
        _ => format!("{stage_label} file").into(),
    };
    let context_menu_invoker: SharedString = {
        format!(
            "status_file_menu_{}_{}_{}",
            repo_id.0,
            section.id_label(),
            entry.path.display()
        )
        .into()
    };
    let context_menu_active = active_context_menu_invoker == Some(&context_menu_invoker);
    let context_menu_invoker_for_stage = context_menu_invoker.clone();
    let context_menu_invoker_for_row = context_menu_invoker.clone();
    let row_group: SharedString =
        format!("status_row_{}_{}_{}", repo_id.0, section.id_label(), ix).into();
    let row_debug_selector = format!("status_row_{}_{}_{}", repo_id.0, section.id_label(), ix);

    let stage_button = components::Button::new(format!("stage_btn_{ix}"), stage_label)
        .style(components::ButtonStyle::Solid)
        .on_click(theme, cx, move |this, e, window, cx| {
            cx.stop_propagation();
            this.focus_diff_panel(window, cx);

            if is_conflicted {
                this.activate_context_menu_invoker(context_menu_invoker_for_stage.clone(), cx);
                this.open_popover_at(
                    PopoverKind::StatusFileMenu {
                        repo_id,
                        area,
                        path: (*path_for_stage).clone(),
                    },
                    e.position(),
                    window,
                    cx,
                );
                return;
            }

            let (paths, _used_selection) =
                this.take_status_selected_paths_for_action(repo_id, area, path_for_stage.as_ref());

            match area {
                DiffArea::Unstaged => this.store.dispatch(Msg::StagePaths {
                    repo_id,
                    paths: paths.into(),
                }),
                DiffArea::Staged => this.store.dispatch(Msg::UnstagePaths {
                    repo_id,
                    paths: paths.into(),
                }),
            }

            this.clear_status_multi_selection(repo_id);
            this.store.dispatch(Msg::ClearDiffSelection { repo_id });

            cx.notify();
        })
        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
            let mut changed = false;
            if *hovering {
                changed |= this.set_tooltip_text_if_changed(Some(stage_tooltip.clone()), cx);
            } else {
                changed |= this.clear_tooltip_if_matches(&stage_tooltip, cx);
            }
            if changed {
                cx.notify();
            }
        }));

    let path_display_for_label = path_display.clone();

    div()
        .id(ix)
        .debug_selector(move || row_debug_selector.clone())
        .relative()
        .group(row_group.clone())
        .flex()
        .items_center()
        .gap(scaled_px(8.0))
        .px(scaled_px(8.0))
        .h(scaled_px(STATUS_ROW_HEIGHT_PX))
        .w_full()
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .when(selected, |s| s.bg(theme.colors.hover))
        .when(context_menu_active, |s| s.bg(theme.colors.active))
        .hover(move |s| {
            if context_menu_active {
                s.bg(theme.colors.active)
            } else {
                s.bg(theme.colors.hover)
            }
        })
        .active(move |s| s.bg(theme.colors.active))
        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
            let mut changed = false;
            if *hovering {
                changed |= this.set_tooltip_text_if_changed(Some(row_tooltip.clone()), cx);
            } else {
                changed |= this.clear_tooltip_if_matches(&row_tooltip, cx);
            }
            if changed {
                cx.notify();
            }
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                let clicked_path = (*path_for_menu).clone();
                let clicked_in_multiselect = this
                    .status_selected_paths_for_area(repo_id, area)
                    .iter()
                    .any(|p| p == &clicked_path);
                if !clicked_in_multiselect {
                    this.status_selection_apply_click(
                        repo_id,
                        section,
                        clicked_path.clone(),
                        Some(ix),
                        gpui::Modifiers::default(),
                        None,
                    );
                }
                if is_conflicted && area == DiffArea::Unstaged {
                    this.store.dispatch(Msg::SelectConflictDiff {
                        repo_id,
                        path: clicked_path.clone(),
                    });
                } else {
                    this.store.dispatch(Msg::SelectDiff {
                        repo_id,
                        target: DiffTarget::WorkingTree {
                            path: clicked_path.clone(),
                            area,
                        },
                    });
                }
                this.activate_context_menu_invoker(context_menu_invoker_for_row.clone(), cx);
                this.open_popover_at(
                    PopoverKind::StatusFileMenu {
                        repo_id,
                        area,
                        path: clicked_path,
                    },
                    e.position,
                    window,
                    cx,
                );
                cx.notify();
            }),
        )
        .child(
            div()
                .w(scaled_px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(icon, color, scaled_px(14.0))),
        )
        .child(
            div()
                .text_sm()
                .line_height(scaled_px(18.0))
                .flex_1()
                .min_w(px(0.0))
                .line_clamp(1)
                .child(path_display_for_label.clone()),
        )
        .child(
            div()
                .absolute()
                .right(scaled_px(6.0))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .invisible()
                .group_hover(row_group.clone(), |d| d.visible())
                .gap(scaled_px(4.0))
                .child(stage_button),
        )
        .on_click(cx.listener(move |this, _e: &ClickEvent, window, cx| {
            this.focus_diff_panel(window, cx);
            let modifiers = _e.modifiers();
            let entries = if modifiers.shift {
                this.active_repo()
                    .filter(|r| r.id == repo_id)
                    .map(|repo| status_paths_for_section(repo, section))
            } else {
                None
            };
            this.status_selection_apply_click(
                repo_id,
                section,
                (*path_for_row).clone(),
                Some(ix),
                modifiers,
                entries.as_deref(),
            );
            if is_conflicted && area == DiffArea::Unstaged {
                this.store.dispatch(Msg::SelectConflictDiff {
                    repo_id,
                    path: (*path_for_row).clone(),
                });
            } else {
                this.store.dispatch(Msg::SelectDiff {
                    repo_id,
                    target: DiffTarget::WorkingTree {
                        path: (*path_for_row).clone(),
                        area,
                    },
                });
            }
            cx.notify();
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb(s: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(s)
    }

    #[test]
    fn status_selection_ctrl_click_toggles() {
        let mut sel = StatusMultiSelection::default();
        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::CombinedUnstaged,
            pb("a"),
            None,
            gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            None,
            false,
            None,
        );
        assert_eq!(sel.unstaged, vec![pb("a")]);

        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::CombinedUnstaged,
            pb("b"),
            None,
            gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            None,
            false,
            None,
        );
        assert_eq!(sel.unstaged, vec![pb("a"), pb("b")]);

        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::CombinedUnstaged,
            pb("a"),
            None,
            gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            None,
            false,
            None,
        );
        assert_eq!(sel.unstaged, vec![pb("b")]);
    }

    #[test]
    fn status_selection_shift_click_selects_range() {
        let mut sel = StatusMultiSelection::default();
        let entries = vec![pb("a"), pb("b"), pb("c"), pb("d")];

        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::CombinedUnstaged,
            pb("b"),
            None,
            gpui::Modifiers::default(),
            Some(1),
            false,
            Some(&entries),
        );
        assert_eq!(sel.unstaged, vec![pb("b")]);

        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::CombinedUnstaged,
            pb("d"),
            None,
            gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            Some(1),
            false,
            Some(&entries),
        );
        assert_eq!(sel.unstaged, vec![pb("b"), pb("c"), pb("d")]);
    }

    #[test]
    fn split_untracked_selection_clears_tracked_selection() {
        let mut sel = StatusMultiSelection {
            unstaged: vec![pb("tracked.txt")],
            unstaged_anchor: Some(pb("tracked.txt")),
            ..Default::default()
        };

        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::Untracked,
            pb("new.txt"),
            None,
            gpui::Modifiers::default(),
            None,
            false,
            None,
        );

        assert!(sel.unstaged.is_empty());
        assert_eq!(sel.untracked, vec![pb("new.txt")]);
    }

    #[test]
    fn status_selection_shift_click_uses_index_hints_without_scanning() {
        bench_reset_status_selection();

        let mut sel = StatusMultiSelection::default();
        let entries = vec![pb("a"), pb("b"), pb("c"), pb("d")];

        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::CombinedUnstaged,
            pb("b"),
            Some(1),
            gpui::Modifiers::default(),
            Some(1),
            true,
            Some(&entries),
        );
        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::CombinedUnstaged,
            pb("d"),
            Some(3),
            gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            Some(1),
            true,
            Some(&entries),
        );

        assert_eq!(sel.unstaged, vec![pb("b"), pb("c"), pb("d")]);
        assert_eq!(sel.unstaged_anchor, Some(pb("b")));
        assert_eq!(sel.unstaged_anchor_index, Some(1));
        assert_eq!(sel.unstaged_anchor_status_rev, Some(1));
        assert_eq!(bench_snapshot_status_selection().position_scan_steps, 0);
    }

    #[test]
    fn status_selection_shift_click_falls_back_when_index_hint_is_stale() {
        bench_reset_status_selection();

        let mut sel = StatusMultiSelection::default();
        let entries = vec![pb("a"), pb("b"), pb("c"), pb("d")];

        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::CombinedUnstaged,
            pb("b"),
            Some(1),
            gpui::Modifiers::default(),
            Some(1),
            true,
            Some(&entries),
        );
        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::CombinedUnstaged,
            pb("d"),
            Some(0),
            gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            Some(2),
            false,
            Some(&entries),
        );

        assert_eq!(sel.unstaged, vec![pb("b"), pb("c"), pb("d")]);
        assert_eq!(sel.unstaged_anchor, Some(pb("b")));
        assert_eq!(sel.unstaged_anchor_index, Some(1));
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        assert!(
            bench_snapshot_status_selection().position_scan_steps > 0,
            "stale index hints should fall back to a path scan"
        );
    }

    #[test]
    fn staged_selection_clears_other_section_anchor_indexes() {
        let mut sel = StatusMultiSelection {
            untracked: vec![pb("new.txt")],
            untracked_anchor: Some(pb("new.txt")),
            unstaged: vec![pb("tracked.txt")],
            unstaged_anchor: Some(pb("tracked.txt")),
            unstaged_anchor_index: Some(4),
            ..Default::default()
        };

        apply_status_multi_selection_click(
            &mut sel,
            StatusSection::Staged,
            pb("staged.txt"),
            Some(2),
            gpui::Modifiers::default(),
            Some(3),
            false,
            None,
        );

        assert!(sel.untracked.is_empty());
        assert!(sel.unstaged.is_empty());
        assert!(sel.untracked_anchor.is_none());
        assert!(sel.unstaged_anchor.is_none());
        assert!(sel.unstaged_anchor_index.is_none());
        assert!(sel.unstaged_anchor_status_rev.is_none());
        assert_eq!(sel.staged, vec![pb("staged.txt")]);
        assert_eq!(sel.staged_anchor, Some(pb("staged.txt")));
        assert_eq!(sel.staged_anchor_index, Some(2));
        assert_eq!(sel.staged_anchor_status_rev, Some(3));
    }
}
