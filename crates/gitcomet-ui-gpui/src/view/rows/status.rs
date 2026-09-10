use super::*;
use gitcomet_core::domain::SubmoduleStatus;
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
    anchor_order_rev: &'a mut Option<u64>,
}

fn set_status_multi_selection_single(
    selection: StatusMultiSelectionSlice<'_>,
    clicked_path: std::path::PathBuf,
    clicked_index: Option<usize>,
    order_rev: Option<u64>,
) {
    selection.selected.clear();
    selection.selected.push(clicked_path.clone());
    *selection.anchor = Some(clicked_path);
    *selection.anchor_index = clicked_index;
    *selection.anchor_order_rev = order_rev;
}

fn apply_status_multi_selection_to_slice(
    selection: StatusMultiSelectionSlice<'_>,
    clicked_path: std::path::PathBuf,
    clicked_index: Option<usize>,
    modifiers: gpui::Modifiers,
    order_rev: Option<u64>,
    trust_clicked_index: bool,
    entries: Option<&[std::path::PathBuf]>,
) {
    if modifiers.shift {
        let Some(entries) = entries else {
            set_status_multi_selection_single(selection, clicked_path, clicked_index, order_rev);
            return;
        };

        let Some(clicked_ix) = status_selection_entry_index(
            entries,
            clicked_path.as_path(),
            clicked_index,
            trust_clicked_index,
        ) else {
            set_status_multi_selection_single(selection, clicked_path, clicked_index, order_rev);
            return;
        };

        let anchor_ix = if let Some(anchor_path) = selection.anchor.as_deref() {
            let trust_anchor_index = order_rev
                .zip(*selection.anchor_order_rev)
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
        *selection.anchor_order_rev = order_rev;
        return;
    }

    if modifiers.secondary() || modifiers.control || modifiers.platform {
        if let Some(ix) = selection.selected.iter().position(|p| p == &clicked_path) {
            selection.selected.remove(ix);
            if selection.selected.is_empty() {
                *selection.anchor = None;
                *selection.anchor_index = None;
                *selection.anchor_order_rev = None;
            }
        } else {
            selection.selected.push(clicked_path.clone());
            *selection.anchor = Some(clicked_path);
            *selection.anchor_index = clicked_index;
            *selection.anchor_order_rev = order_rev;
        }
        return;
    }

    set_status_multi_selection_single(selection, clicked_path, clicked_index, order_rev);
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
    order_rev: Option<u64>,
    trust_clicked_index: bool,
    entries: Option<&[std::path::PathBuf]>,
) {
    selection.explicit_section = Some(section);
    match section {
        StatusSection::CombinedUnstaged | StatusSection::Unstaged => {
            selection.untracked.clear();
            selection.untracked_anchor = None;
            selection.staged.clear();
            selection.staged_anchor = None;
            selection.staged_anchor_index = None;
            selection.staged_anchor_order_rev = None;
            apply_status_multi_selection_to_slice(
                StatusMultiSelectionSlice {
                    selected: &mut selection.unstaged,
                    anchor: &mut selection.unstaged_anchor,
                    anchor_index: &mut selection.unstaged_anchor_index,
                    anchor_order_rev: &mut selection.unstaged_anchor_order_rev,
                },
                clicked_path,
                clicked_index,
                modifiers,
                order_rev,
                trust_clicked_index,
                entries,
            );
        }
        StatusSection::Untracked => {
            selection.unstaged.clear();
            selection.unstaged_anchor = None;
            selection.unstaged_anchor_index = None;
            selection.unstaged_anchor_order_rev = None;
            selection.staged.clear();
            selection.staged_anchor = None;
            selection.staged_anchor_index = None;
            selection.staged_anchor_order_rev = None;
            let mut untracked_anchor_index = None;
            let mut untracked_anchor_order_rev = None;
            apply_status_multi_selection_to_slice(
                StatusMultiSelectionSlice {
                    selected: &mut selection.untracked,
                    anchor: &mut selection.untracked_anchor,
                    anchor_index: &mut untracked_anchor_index,
                    anchor_order_rev: &mut untracked_anchor_order_rev,
                },
                clicked_path,
                clicked_index,
                modifiers,
                order_rev,
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
            selection.unstaged_anchor_order_rev = None;
            apply_status_multi_selection_to_slice(
                StatusMultiSelectionSlice {
                    selected: &mut selection.staged,
                    anchor: &mut selection.staged_anchor,
                    anchor_index: &mut selection.staged_anchor_index,
                    anchor_order_rev: &mut selection.staged_anchor_order_rev,
                },
                clicked_path,
                clicked_index,
                modifiers,
                order_rev,
                trust_clicked_index,
                entries,
            );
        }
    }
}

fn submodule_status_lookup(repo: &RepoState) -> FxHashMap<&std::path::Path, SubmoduleStatus> {
    let mut lookup = FxHashMap::default();
    if let Loadable::Ready(submodules) = &repo.submodules {
        lookup.reserve(submodules.len());
        for submodule in submodules.iter() {
            lookup.insert(submodule.path.as_path(), submodule.status);
        }
    }
    lookup
}

impl DetailsPaneView {
    /// Drop the row selection because an action has gone ahead with it. Kept
    /// separate from reading it: an action that can still be called off — a
    /// confirmation dialog the user cancels — must not cost the user a selection
    /// they spent clicks building.
    pub(in crate::view) fn clear_status_multi_selection(&mut self, repo_id: RepoId) {
        self.status_multi_selection.remove(&repo_id);
    }

    /// The paths an action on `clicked_path` covers, plus whether they came from
    /// the row selection rather than from the clicked path alone. Reads only;
    /// [`Self::clear_status_multi_selection`] is what consumes the selection, and
    /// callers owe it that call once the action is settled.
    pub(in crate::view) fn status_selected_paths_for_action(
        &self,
        repo_id: RepoId,
        area: DiffArea,
        clicked_path: &std::path::PathBuf,
    ) -> (Vec<std::path::PathBuf>, bool) {
        let selection = self.status_selected_paths_for_area(repo_id, area);
        let use_selection = selection.len() > 1 && selection.iter().any(|p| p == clicked_path);
        if !use_selection {
            return (vec![clicked_path.clone()], false);
        }
        (selection.to_vec(), true)
    }

    fn status_multi_selection_for_repo_mut(
        &mut self,
        repo_id: RepoId,
    ) -> &mut StatusMultiSelection {
        self.status_multi_selection.entry(repo_id).or_default()
    }

    pub(in crate::view) fn status_selected_paths_for_area(
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
        let order_rev = self
            .active_repo()
            .filter(|repo| repo.id == repo_id)
            .map(|repo| self.status_anchor_order_rev(repo, section));
        let sel = self.status_multi_selection_for_repo_mut(repo_id);
        apply_status_multi_selection_click(
            sel,
            section,
            clicked_path,
            clicked_index,
            modifiers,
            order_rev,
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
    let Some(entries) = this.status_section_entries(repo, section) else {
        return Vec::new();
    };
    let plan = this.status_file_plan(repo, section);
    let is_tree = plan.is_tree();
    let line_stats = status_section_line_stats(repo, section);
    // Measured from the section's prepaint probe; unmeasured reads as roomy.
    let detail_width = this
        .current_status_sections_bounds()
        .map(|bounds| bounds.size.width)
        .unwrap_or(gpui::Pixels::MAX);
    let repo_id = repo.id;
    let selected = repo.diff_state.diff_target.as_ref();
    // Hashed once per batch: a scan per visible row made multi-select cost
    // visible × selected path compares per frame.
    let selected_paths: FxHashSet<&std::path::Path> = this
        .status_selected_paths_for_area(repo.id, section.diff_area())
        .iter()
        .map(std::path::PathBuf::as_path)
        .collect();
    let multi_select_active = this
        .status_multi_selection
        .get(&repo.id)
        .is_some_and(|selection| selection.explicit_section.is_some())
        || !selected_paths.is_empty();
    let submodule_statuses = submodule_status_lookup(repo);
    let theme = this.theme;
    let ui_scale = this.ui_scale();
    let visible_signature = this.status_visible_signature(repo, section, &range, entries.len());
    // Leaf names have no shared prefix to align, and a row that reported into
    // the group would anchor it on the shortest file name.
    let path_alignment_group = (!is_tree).then(|| {
        this.status_path_alignment_group(section)
            .visible_rows(visible_signature)
    });

    let rows: Vec<(usize, crate::view::rows::FileListRow)> = range
        .filter_map(|row_ix| {
            plan.row_at(crate::view::rows::RowIx(row_ix))
                .map(|row| (row_ix, row))
        })
        .collect();

    rows.into_iter()
        .filter_map(|(ix, row)| {
            let (ordinal, depth) = match row {
                crate::view::rows::FileListRow::Directory {
                    key,
                    label,
                    depth,
                    collapsed,
                    chain,
                    subtree: _,
                    counts,
                    additions: subtree_additions,
                    deletions: subtree_deletions,
                } => {
                    let group: SharedString =
                        format!("status_dir_{}_{}_{}", repo_id.0, section.id_label(), ix).into();
                    let detail: crate::view::rows::DirectoryRowDetail =
                        crate::view::rows::directory_row_detail_for_width(
                            detail_width,
                            depth,
                            counts,
                            subtree_additions.is_some() || subtree_deletions.is_some(),
                            this.ui_scale_percent,
                        );
                    let action = status_folder_action(
                        theme,
                        ix,
                        section,
                        repo_id,
                        Arc::clone(&key),
                        group.clone(),
                        cx,
                    );
                    return Some(
                        crate::view::rows::directory_row(crate::view::rows::DirectoryRowProps {
                            theme,
                            ui_scale_percent: this.ui_scale_percent,
                            id: ("status_dir", ix).into(),
                            label: &label,
                            depth,
                            collapsed,
                            counts,
                            additions: subtree_additions,
                            deletions: subtree_deletions,
                            row_height_px: STATUS_ROW_HEIGHT_PX,
                            row_group: Some(group),
                            detail,
                        })
                        .debug_selector(move || {
                            format!("status_dir_{}_{}_{}", repo_id.0, section.id_label(), ix)
                        })
                        .child(action)
                        .on_click(cx.listener(move |this, e: &ClickEvent, _window, cx| {
                            if !e.standard_click() {
                                return;
                            }
                            this.toggle_file_list_dir(
                                repo_id,
                                crate::view::rows::FileListId::Status(section),
                                Arc::clone(&key),
                                Arc::clone(&chain),
                                collapsed,
                                cx,
                            );
                        }))
                        .into_any_element(),
                    );
                }
                crate::view::rows::FileListRow::File { ordinal, depth } => (ordinal, depth),
            };
            let entry = entries.get(ordinal.0)?;
            let path_display = if is_tree {
                SharedString::from(
                    entry
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| entry.path.display().to_string()),
                )
            } else {
                this.cached_path_display(&entry.path)
            };
            let is_selected = if multi_select_active {
                selected_paths.contains(entry.path.as_path())
            } else {
                selected.is_some_and(|t| match t {
                    DiffTarget::WorkingTree { path, area } => {
                        *area == section.diff_area() && path == &entry.path
                    }
                    _ => false,
                })
            };
            let submodule_status = (entry.kind != FileStatusKind::Untracked)
                .then(|| submodule_statuses.get(entry.path.as_path()).copied())
                .flatten();
            let is_submodule = submodule_status.is_some();
            Some(status_row(
                StatusRowCtx {
                    theme,
                    ui_scale,
                    row_ix: ix,
                    display_position: plan.display_position(ordinal).unwrap_or(ordinal.0),
                    depth,
                    is_tree,
                    section,
                    repo_id,
                    is_selected,
                    is_submodule,
                    submodule_status,
                    line_stats: line_stats.and_then(|stats| stats.get(&entry.path)).copied(),
                },
                entry,
                path_display,
                this.tooltip_host.clone(),
                path_alignment_group.clone(),
                this.active_context_menu_invoker.as_ref(),
                cx,
            ))
        })
        .collect()
}

fn status_file_menu_invoker(
    repo_id: RepoId,
    section: StatusSection,
    path: &std::path::Path,
) -> SharedString {
    format!(
        "status_file_menu_{}_{}_{}",
        repo_id.0,
        section.id_label(),
        path.display()
    )
    .into()
}

/// `active == status_file_menu_invoker(repo_id, section, path)` without
/// building the string.
fn status_file_menu_invoker_matches(
    active: &str,
    repo_id: RepoId,
    section: StatusSection,
    path: &std::path::Path,
) -> bool {
    let Some(rest) = active.strip_prefix("status_file_menu_") else {
        return false;
    };
    let Some((repo, rest)) = rest.split_once('_') else {
        return false;
    };
    if repo.parse::<u64>() != Ok(repo_id.0) {
        return false;
    }
    let Some(rest) = rest.strip_prefix(section.id_label()) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('_') else {
        return false;
    };
    path.to_str().is_some_and(|path| path == rest) || path.display().to_string() == rest
}

struct StatusRowCtx {
    theme: AppTheme,
    ui_scale: crate::ui_scale::UiScale,
    /// Display row, which is what element ids and debug selectors key on.
    row_ix: usize,
    /// Position among the drawn file rows, which selection ranges span.
    display_position: usize,
    depth: usize,
    is_tree: bool,
    section: StatusSection,
    repo_id: RepoId,
    is_selected: bool,
    is_submodule: bool,
    submodule_status: Option<SubmoduleStatus>,
    /// `None` for untracked, binary, or before the counts have loaded.
    line_stats: Option<gitcomet_core::domain::LineStats>,
}

/// Stage/Unstage a whole folder, revealed at the row's right edge on hover.
///
/// Acts on the subtree alone, ignoring any multi-selection: clicking Stage on
/// `src/` and having it stage files in `docs/` would be a surprise. Passes
/// explicit paths, not a directory pathspec — `unstage_impl` filters conflicted
/// paths by exact match, and a directory would slip past it.
fn status_folder_action(
    theme: AppTheme,
    ix: usize,
    section: StatusSection,
    repo_id: RepoId,
    key: Arc<std::path::Path>,
    row_group: SharedString,
    cx: &mut gpui::Context<DetailsPaneView>,
) -> AnyElement {
    let area = section.diff_area();
    let label = match area {
        DiffArea::Unstaged => "Stage",
        DiffArea::Staged => "Unstage",
    };
    let button = components::Button::new(format!("status_dir_action_btn_{ix}"), label)
        .style(components::ButtonStyle::Solid)
        .on_click(theme, cx, move |this, e, window, cx| {
            cx.stop_propagation();
            let paths = this.status_folder_subtree_paths(repo_id, section, key.as_ref());
            if paths.is_empty() {
                return;
            }

            if area == DiffArea::Unstaged
                && let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
                    &this.state,
                    repo_id,
                    paths.clone(),
                    // No selection was consumed, so cancelling must leave it.
                    false,
                )
            {
                this.open_popover_at(confirm, e.position(), window, cx);
                cx.notify();
                return;
            }

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
            cx.notify();
        })
        .gitcomet_tooltip(theme, format!("{label} this folder").into());

    div()
        .debug_selector(move || {
            format!(
                "status_dir_action_{}_{}_{}",
                repo_id.0,
                section.id_label(),
                ix
            )
        })
        .absolute()
        .right_0()
        .top_0()
        .bottom_0()
        .flex()
        .items_center()
        .invisible()
        .group_hover(row_group, |d| d.visible())
        .child(button)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn status_row(
    ctx: StatusRowCtx,
    entry: &FileStatus,
    path_display: SharedString,
    tooltip_host: WeakEntity<TooltipHost>,
    path_alignment_group: Option<components::PathTruncationAlignmentGroup>,
    active_context_menu_invoker: Option<&SharedString>,
    cx: &mut gpui::Context<DetailsPaneView>,
) -> AnyElement {
    let StatusRowCtx {
        theme,
        ui_scale,
        row_ix,
        display_position,
        depth,
        is_tree,
        section,
        repo_id,
        is_selected: selected,
        is_submodule,
        submodule_status,
        line_stats,
    } = ctx;
    let ix = row_ix;
    let scaled_px = |value: f32| ui_scale.px(value);
    // Untracked is in neither index lane, so the column would always be blank.
    let show_line_stats = crate::view::status_section_has_line_stats(section);
    let area = section.diff_area();
    let (icon, color) = if is_submodule {
        let color = match submodule_status {
            Some(SubmoduleStatus::NotInitialized) => with_alpha(
                theme.colors.foreground.secondary,
                if theme.is_dark { 0.78 } else { 0.92 },
            ),
            Some(SubmoduleStatus::HeadMismatch) => theme.colors.status.warning.foreground,
            Some(SubmoduleStatus::MergeConflict | SubmoduleStatus::MissingMapping) => {
                theme.colors.status.danger.foreground
            }
            Some(SubmoduleStatus::UpToDate | SubmoduleStatus::Unknown(_)) | None => {
                theme.colors.accent.foreground
            }
        };
        ("icons/box.svg", color)
    } else {
        match entry.kind {
            FileStatusKind::Untracked => match area {
                DiffArea::Unstaged => ("icons/plus.svg", theme.colors.status.success.foreground),
                DiffArea::Staged => ("icons/question.svg", theme.colors.status.warning.foreground),
            },
            FileStatusKind::Modified => {
                ("icons/pencil.svg", theme.colors.status.warning.foreground)
            }
            FileStatusKind::Added => ("icons/plus.svg", theme.colors.status.success.foreground),
            FileStatusKind::Deleted => ("icons/minus.svg", theme.colors.status.danger.foreground),
            FileStatusKind::Renamed => ("icons/swap.svg", theme.colors.accent.foreground),
            FileStatusKind::Conflicted => {
                ("icons/warning.svg", theme.colors.status.danger.foreground)
            }
        }
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
    let stage_tooltip: SharedString = match stage_label {
        "Stage" => "Stage file".into(),
        "Unstage" => "Unstage file".into(),
        "Resolve…" => "Resolve… file".into(),
        _ => format!("{stage_label} file").into(),
    };
    // The invoker string is only needed when a menu is open (to mark its row)
    // or when this row opens one; the handlers build it on demand instead of
    // every row paying for it each frame.
    let context_menu_active = active_context_menu_invoker.is_some_and(|active| {
        status_file_menu_invoker_matches(active, repo_id, section, &entry.path)
    });
    let row_group: SharedString =
        format!("status_row_{}_{}_{}", repo_id.0, section.id_label(), ix).into();

    let stage_button = components::Button::new(format!("stage_btn_{ix}"), stage_label)
        .style(components::ButtonStyle::Solid)
        .on_click(theme, cx, move |this, e, window, cx| {
            cx.stop_propagation();
            this.focus_diff_panel(window, cx);

            if is_conflicted {
                this.activate_context_menu_invoker(
                    status_file_menu_invoker(repo_id, section, &path_for_stage),
                    cx,
                );
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

            // Staging is what marks a conflict resolved, so confirm first if any
            // of these files still has conflict markers in the worktree. The
            // selection is read but not consumed until past that point, so
            // cancelling the dialog leaves it standing.
            let (paths, used_selection) =
                this.status_selected_paths_for_action(repo_id, area, path_for_stage.as_ref());

            if area == DiffArea::Unstaged
                && let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
                    &this.state,
                    repo_id,
                    paths.clone(),
                    used_selection,
                )
            {
                this.open_popover_at(confirm, e.position(), window, cx);
                cx.notify();
                return;
            }
            if used_selection {
                this.clear_status_multi_selection(repo_id);
            }

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
        .gitcomet_tooltip(theme, stage_tooltip.clone());

    let path_display_for_label = path_display.clone();

    div()
        .id(ix)
        .debug_selector(move || format!("status_row_{}_{}_{}", repo_id.0, section.id_label(), ix))
        .relative()
        .group(row_group.clone())
        .flex()
        .items_center()
        .gap(scaled_px(8.0))
        .pl(if is_tree {
            crate::view::rows::file_row_indent_px(depth, ui_scale.percent())
        } else {
            scaled_px(8.0)
        })
        .pr(scaled_px(8.0))
        .h(scaled_px(STATUS_ROW_HEIGHT_PX))
        .w_full()
        .cursor(CursorStyle::PointingHand)
        .when(selected, |s| {
            s.bg(theme.colors.interaction.hover_background)
        })
        // Light themes keep selection legible with a ring: `hover_background`
        // alone is barely a shade off the panel it sits on, so a selected row
        // and a merely hovered one look identical.
        .when_some(
            selected
                .then(|| components::light_theme_selection_outline(theme))
                .flatten(),
            |s, outline| s.shadow(vec![outline]),
        )
        .when(context_menu_active, |s| {
            s.bg(theme.colors.interaction.pressed_background)
        })
        .hover(move |s| {
            if context_menu_active {
                s.bg(theme.colors.interaction.pressed_background)
            } else {
                s.bg(theme.colors.interaction.hover_background)
            }
        })
        .active(move |s| s.bg(theme.colors.interaction.pressed_background))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                // Right-click only opens the menu: it must not open the diff (that is
                // left-click's job) and must not touch the left-click selection. The
                // right-clicked row is marked separately via the context menu invoker.
                this.activate_context_menu_invoker(
                    status_file_menu_invoker(repo_id, section, &path_for_menu),
                    cx,
                );
                this.open_popover_at(
                    PopoverKind::StatusFileMenu {
                        repo_id,
                        area,
                        path: (*path_for_menu).clone(),
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
                .child(
                    match path_alignment_group {
                        Some(group) => components::TruncatedText::aligned_path(
                            path_display_for_label.clone(),
                            group,
                        ),
                        None => components::TruncatedText::new(path_display_for_label.clone()),
                    }
                    .text_sm()
                    .id(("status_row_path", ix))
                    .full_text_tooltip(tooltip_host)
                    .render(cx),
                ),
        )
        .when(show_line_stats, |row| {
            row.child(div().flex_none().child(components::diff_stat_optional(
                theme,
                ui_scale,
                line_stats.and_then(|stats| stats.additions),
                line_stats.and_then(|stats| stats.deletions),
            )))
        })
        .child(
            div()
                .debug_selector(move || {
                    format!(
                        "status_row_action_{}_{}_{}",
                        repo_id.0,
                        section.id_label(),
                        ix
                    )
                })
                .absolute()
                .right_0()
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
            let modifiers = _e.modifiers();
            this.focus_status_section(section, window, cx);
            let modifies_selection = modifiers.shift || modifiers.control || modifiers.platform;
            let target = DiffTarget::WorkingTree {
                path: (*path_for_row).clone(),
                area,
            };
            let should_unselect = _e.standard_click()
                && this.status_selected_paths_for_area(repo_id, area)
                    == std::slice::from_ref(path_for_row.as_ref())
                && this.active_repo().is_some_and(|repo| {
                    repo.id == repo_id && repo.diff_state.diff_target.as_ref() == Some(&target)
                });
            let entries = if modifiers.shift {
                Some(this.status_display_order_paths(repo_id, section))
            } else {
                None
            };
            this.status_selection_apply_click(
                repo_id,
                section,
                (*path_for_row).clone(),
                // Drawn-row position: not the row index (directories count
                // too), not the ordinal (a tree reorders it).
                Some(display_position),
                modifiers,
                entries.as_deref(),
            );
            if modifies_selection {
                cx.notify();
                return;
            }
            if should_unselect {
                this.clear_status_multi_selection(repo_id);
                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
            } else if is_conflicted && area == DiffArea::Unstaged {
                this.store.dispatch(Msg::SelectConflictDiff {
                    repo_id,
                    path: (*path_for_row).clone(),
                });
            } else {
                this.store.dispatch(Msg::SelectDiff { repo_id, target });
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

    /// Status entries arrive byte-sorted from the backend, where `Zoo.rs` beats
    /// `apple.rs`. The UI sorts case-insensitively instead, the same way the
    /// committed-file list always has.
    #[test]
    fn status_sort_is_case_insensitive_in_both_directions() {
        let entries = vec![
            file_status("Zoo.rs", FileStatusKind::Modified),
            file_status("apple.rs", FileStatusKind::Modified),
            file_status("Banana.rs", FileStatusKind::Modified),
        ];
        let all: Vec<usize> = (0..entries.len()).collect();

        let ascending = crate::view::rows::status_section_sorted_indexes(
            &entries,
            &all,
            crate::view::rows::CommitFileSort::PathAscending,
            None,
        );
        assert_eq!(ascending.as_ref(), &[1, 2, 0], "apple, Banana, Zoo");

        let descending = crate::view::rows::status_section_sorted_indexes(
            &entries,
            &all,
            crate::view::rows::CommitFileSort::PathDescending,
            None,
        );
        assert_eq!(descending.as_ref(), &[0, 2, 1], "Zoo, Banana, apple");
    }

    /// Status entries have no edit counts, so the edit-size modes have nothing
    /// to order by and must not shuffle the list into an arbitrary order.
    #[test]
    fn edit_size_sorts_fall_back_to_path_order_for_status_entries() {
        let entries = vec![
            file_status("b.rs", FileStatusKind::Modified),
            file_status("a.rs", FileStatusKind::Modified),
        ];
        let all: Vec<usize> = (0..entries.len()).collect();
        for sort in [
            crate::view::rows::CommitFileSort::EditSizeAscending,
            crate::view::rows::CommitFileSort::EditSizeDescending,
        ] {
            let ordered =
                crate::view::rows::status_section_sorted_indexes(&entries, &all, sort, None);
            assert_eq!(ordered.as_ref(), &[1, 0], "{sort:?} falls back to path A-Z");
        }
    }

    /// Mirrors the committed-file list: unknown sizes last, path breaks ties.
    #[test]
    fn edit_size_sort_orders_by_size_with_unknowns_last() {
        use gitcomet_core::domain::LineStats;

        let entries = vec![
            file_status("small.rs", FileStatusKind::Modified),
            file_status("big.rs", FileStatusKind::Modified),
            file_status("unknown.rs", FileStatusKind::Modified),
        ];
        let all: Vec<usize> = (0..entries.len()).collect();
        let mut stats = rustc_hash::FxHashMap::default();
        stats.insert(
            pb("small.rs"),
            LineStats {
                additions: Some(1),
                deletions: Some(1),
            },
        );
        stats.insert(
            pb("big.rs"),
            LineStats {
                additions: Some(90),
                deletions: Some(10),
            },
        );
        // `unknown.rs` is absent on purpose — a binary file, say.

        let largest = crate::view::rows::status_section_sorted_indexes(
            &entries,
            &all,
            crate::view::rows::CommitFileSort::EditSizeDescending,
            Some(&stats),
        );
        assert_eq!(largest.as_ref(), &[1, 0, 2], "big, small, then unknown");

        let smallest = crate::view::rows::status_section_sorted_indexes(
            &entries,
            &all,
            crate::view::rows::CommitFileSort::EditSizeAscending,
            Some(&stats),
        );
        assert_eq!(
            smallest.as_ref(),
            &[0, 1, 2],
            "unknowns stay last in both directions"
        );
    }

    /// With nothing to order by the list must stay in a stable path order.
    #[test]
    fn edit_size_sort_falls_back_to_path_order_without_stats() {
        let entries = vec![
            file_status("b.rs", FileStatusKind::Modified),
            file_status("a.rs", FileStatusKind::Modified),
        ];
        let all: Vec<usize> = (0..entries.len()).collect();
        let ordered = crate::view::rows::status_section_sorted_indexes(
            &entries,
            &all,
            crate::view::rows::CommitFileSort::EditSizeDescending,
            None,
        );
        assert_eq!(ordered.as_ref(), &[1, 0]);
    }

    fn file_status(path: &str, kind: FileStatusKind) -> FileStatus {
        FileStatus {
            path: pb(path),
            kind,
            conflict: None,
        }
    }

    fn repo_with_status_entries(entries: Vec<FileStatus>) -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            gitcomet_core::domain::RepoSpec {
                workdir: pb("/tmp/status-section-entries-test"),
            },
        );
        repo.worktree_status = Loadable::Ready(Arc::new(entries));
        repo.worktree_status_rev = 1;
        repo
    }

    fn commit_id(id: &str) -> gitcomet_core::domain::CommitId {
        gitcomet_core::domain::CommitId(Arc::from(id))
    }

    #[test]
    fn status_section_entries_index_filtered_sections_directly() {
        let repo = repo_with_status_entries(vec![
            file_status("tracked-a.txt", FileStatusKind::Modified),
            file_status("new-a.txt", FileStatusKind::Untracked),
            file_status("tracked-b.txt", FileStatusKind::Deleted),
            file_status("new-b.txt", FileStatusKind::Untracked),
        ]);

        let untracked = StatusSectionEntries::from_repo(&repo, StatusSection::Untracked).unwrap();
        assert_eq!(untracked.len(), 2);
        assert_eq!(untracked.get(0).unwrap().path, pb("new-a.txt"));
        assert_eq!(untracked.get(1).unwrap().path, pb("new-b.txt"));
        assert!(untracked.get(2).is_none());

        let unstaged = StatusSectionEntries::from_repo(&repo, StatusSection::Unstaged).unwrap();
        assert_eq!(unstaged.len(), 2);
        assert_eq!(unstaged.get(0).unwrap().path, pb("tracked-a.txt"));
        assert_eq!(unstaged.get(1).unwrap().path, pb("tracked-b.txt"));
        assert!(unstaged.get(2).is_none());
    }

    #[test]
    fn submodule_status_lookup_maps_ready_submodules_by_path() {
        let mut repo = repo_with_status_entries(Vec::new());
        repo.submodules = Loadable::Ready(Arc::new(vec![
            gitcomet_core::domain::Submodule {
                path: pb("vendor/ready"),
                recorded_head: commit_id("1111111"),
                checked_out_head: Some(commit_id("2222222")),
                status: SubmoduleStatus::HeadMismatch,
            },
            gitcomet_core::domain::Submodule {
                path: pb("vendor/missing"),
                recorded_head: commit_id("3333333"),
                checked_out_head: None,
                status: SubmoduleStatus::NotInitialized,
            },
        ]));

        let lookup = submodule_status_lookup(&repo);
        assert_eq!(
            lookup.get(std::path::Path::new("vendor/ready")).copied(),
            Some(SubmoduleStatus::HeadMismatch)
        );
        assert_eq!(
            lookup.get(std::path::Path::new("vendor/missing")).copied(),
            Some(SubmoduleStatus::NotInitialized)
        );
        assert_eq!(lookup.get(std::path::Path::new("vendor/other")), None);
    }

    #[test]
    fn status_select_all_preserves_the_anchor_and_supports_range_replacement() {
        for section in [
            StatusSection::CombinedUnstaged,
            StatusSection::Untracked,
            StatusSection::Unstaged,
            StatusSection::Staged,
        ] {
            let mut selection = StatusMultiSelection::default();
            apply_status_multi_selection_click(
                &mut selection,
                section,
                pb("b"),
                Some(1),
                gpui::Modifiers::default(),
                Some(1),
                true,
                None,
            );
            let order = vec![pb("d"), pb("c"), pb("b"), pb("a")];
            selection.select_all(section, order.clone(), 2);
            assert_eq!(
                selection.selected_paths_for_area(section.diff_area()),
                &order
            );
            apply_status_multi_selection_click(
                &mut selection,
                section,
                pb("d"),
                Some(0),
                gpui::Modifiers {
                    shift: true,
                    ..Default::default()
                },
                Some(2),
                true,
                Some(&order),
            );
            assert_eq!(
                selection.selected_paths_for_area(section.diff_area()),
                &order[..3]
            );
            for (ix, path) in order[..3].iter().enumerate() {
                apply_status_multi_selection_click(
                    &mut selection,
                    section,
                    path.clone(),
                    Some(ix),
                    gpui::Modifiers {
                        control: true,
                        ..Default::default()
                    },
                    Some(2),
                    true,
                    None,
                );
            }
            assert!(selection.is_empty());
            assert_eq!(selection.explicit_section, Some(section));
        }
    }

    #[test]
    fn status_select_all_switches_sections_and_uses_the_first_file_without_an_anchor() {
        let mut selection = StatusMultiSelection::default();
        selection.select_all(StatusSection::Untracked, vec![pb("new")], 1);
        selection.select_all(StatusSection::Staged, vec![pb("z"), pb("a")], 2);
        assert!(selection.untracked.is_empty());
        assert!(selection.untracked_anchor.is_none());
        assert_eq!(selection.staged_anchor, Some(pb("z")));
        assert_eq!(selection.staged_anchor_index, Some(0));
        assert_eq!(selection.explicit_section, Some(StatusSection::Staged));
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
        assert_eq!(sel.unstaged_anchor_order_rev, Some(1));
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
        assert!(sel.unstaged_anchor_order_rev.is_none());
        assert_eq!(sel.staged, vec![pb("staged.txt")]);
        assert_eq!(sel.staged_anchor, Some(pb("staged.txt")));
        assert_eq!(sel.staged_anchor_index, Some(2));
        assert_eq!(sel.staged_anchor_order_rev, Some(3));
    }
}
