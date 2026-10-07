//! A hosted file list: one list's own filter, sort, collapse, selection, and
//! scroll over a change-list session, built on the shared projection and
//! tree plan every changed-file list uses. Grouped lists keep every row one
//! height (headers included) and pin the current group's header with a list
//! decoration, so scrolling never replans.

use super::*;
use crate::kit::click::PointerClickExt as _;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::view::rows::{CommitFileFilter, CommitFileSort, FileListRow, RowIx};
use gitcomet_core::domain::{CommitFileChange, CommitId};
use gitcomet_extension_api::{
    ChangeSource, FileListMode, FileSelected, RepositoryHandle, StateSubscription, WindowHost,
    panes::FileListImpl,
};
use gitcomet_state::diff_session::{DiffSessionMsg, DiffViewId};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::rc::Rc;

use crate::view::changed_file_list::{ChangedFileListView, SharedFileListController};
use crate::view::file_list_controller::*;

pub(crate) struct FileListView {
    host: WindowHost,
    store: std::sync::Weak<AppStore>,
    repository: RepositoryHandle,
    view_id: DiffViewId,
    controller: SharedFileListController,
    body: Entity<ChangedFileListView>,
    source: ChangeSource,
    marks: gitcomet_extension_api::FileListMarks,
    /// Some mark has a glyph, so every file row reserves the glyph column.
    marks_glyphs: bool,
    chips: Vec<gitcomet_extension_api::FileListFilterChip>,
    base: Option<CommitId>,
    /// The change list's revision and, for a worktree source, its lane's
    /// line-stats revision: the counts arrive after the files.
    list_rev: Option<(u64, Option<u64>)>,
    loading: bool,
    error: Option<SharedString>,
    on_select: FileSelected,
    #[cfg(test)]
    scroll: UniformListScrollHandle,
    _state: Option<StateSubscription>,
}

impl FileListView {
    /// Measured and tested in the tree layout and path order, whatever the
    /// user's defaults are.
    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) fn benchmark_snapshot(
        host: WindowHost,
        repository: RepositoryHandle,
        files: Arc<Vec<CommitFileChange>>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let view = Self::snapshot(host, repository, files, cx);
        view.controller.borrow_mut().set_mode(FileListMode::Tree);
        view.controller
            .borrow_mut()
            .set_sort(CommitFileSort::default());
        view
    }

    /// A list over `files` as given, from the user's defaults.
    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) fn snapshot(
        host: WindowHost,
        repository: RepositoryHandle,
        files: Arc<Vec<CommitFileChange>>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let mut view = Self::new(
            host,
            std::sync::Weak::new(),
            repository,
            ChangeSource::Commit(CommitId("HEAD".into())),
            Rc::new(|_, _, _| {}),
            cx,
        );
        view.controller.borrow_mut().set_files(files, 1);
        view.loading = false;
        view
    }
    #[cfg(feature = "benchmarks")]
    pub(in crate::view) fn benchmark_plan(&mut self, rebuild: bool) -> usize {
        if rebuild {
            let mut controller = self.controller.borrow_mut();
            let files = controller.files.clone();
            let revision = controller.files_rev.wrapping_add(1);
            controller.set_files(files, revision);
        }
        self.controller.borrow_mut().plan().row_len()
    }
    /// Regroups under `groups` without changing the mode; returns the rows.
    #[cfg(feature = "benchmarks")]
    pub(in crate::view) fn benchmark_regroup(
        &mut self,
        groups: gitcomet_extension_api::FileListGroups,
    ) -> usize {
        let mut controller = self.controller.borrow_mut();
        controller.set_groups(Some(groups));
        controller.grouped().rows.len()
    }
    #[cfg(feature = "benchmarks")]
    pub(in crate::view) fn benchmark_plan_builds(&self) -> u64 {
        self.controller.borrow().plan_cache.builds()
    }
    #[cfg(feature = "benchmarks")]
    pub(in crate::view) fn benchmark_window(
        &mut self,
        decor: bool,
        cx: &mut gpui::Context<Self>,
    ) -> usize {
        let plan = self.controller.borrow_mut().plan();
        if decor {
            self.marks.revision = self.marks.revision.wrapping_add(1);
        }
        let rows = self.render_rows(0..60, cx);
        assert!(Arc::ptr_eq(&plan, &self.controller.borrow_mut().plan()));
        std::hint::black_box(rows).len()
    }

    pub(crate) fn new(
        host: WindowHost,
        store: std::sync::Weak<AppStore>,
        repository: RepositoryHandle,
        source: ChangeSource,
        on_select: FileSelected,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let view_id = DiffViewId::next();
        if let Some(store) = store.upgrade() {
            store.dispatch(Msg::DiffSession(DiffSessionMsg::OpenChanges {
                repo_id: repository.repo_id(),
                lifetime: repository.lifetime(),
                view: view_id,
                source: source.clone(),
            }));
        }
        let weak = cx.weak_entity();
        let state = host
            .observe_state(move |_, cx| {
                let _ = weak.update(cx, |list, cx| list.sync(cx));
            })
            .ok();
        // The user's defaults; an extension's `set_mode` or `set_sort` wins.
        let defaults = crate::view::FileListDefaults::current(cx);
        let mut controller = FileListController::new(mode_for(defaults.layout));
        controller.set_sort(defaults.sort);
        let controller = Rc::new(std::cell::RefCell::new(controller));
        let scroll = UniformListScrollHandle::default();
        let parent = cx.weak_entity();
        let body = cx.new(|_| {
            ChangedFileListView::new(
                Rc::clone(&controller),
                "hosted_file_list_rows",
                scroll.clone(),
                move |range, _, cx| {
                    parent
                        .update(cx, |list, cx| list.render_rows(range, cx))
                        .unwrap_or_default()
                },
            )
        });
        Self {
            host,
            store,
            repository,
            view_id,
            controller,
            body,
            source,
            marks: Default::default(),
            marks_glyphs: false,
            chips: Vec::new(),
            base: None,
            list_rev: None,
            loading: true,
            error: None,
            on_select,
            #[cfg(test)]
            scroll,
            _state: state,
        }
    }

    fn sync(&mut self, cx: &mut gpui::Context<Self>) {
        let Ok(state) = self.host.state(cx) else {
            return;
        };
        let Some(repo) = state.repos.iter().find(|repo| {
            repo.id == self.repository.repo_id() && repo.lifetime() == self.repository.lifetime()
        }) else {
            return;
        };
        let Some(list) = repo
            .change_lists
            .get(&self.view_id)
            .filter(|list| list.source == self.source)
        else {
            return;
        };
        let line_stats = worktree_line_stats(repo, &self.source);
        let key = (list.rev, line_stats.map(|(rev, _)| rev));
        if self.list_rev == Some(key) {
            return;
        }
        self.list_rev = Some(key);
        self.loading = list.is_loading() || matches!(list.files, Loadable::NotLoaded);
        self.error = None;
        match &list.files {
            Loadable::Ready(files) => {
                self.base = list.base.clone();
                let files = match line_stats {
                    Some((_, stats)) => with_line_stats(files, stats),
                    None => Arc::clone(files),
                };
                self.controller.borrow_mut().set_files(files, list.rev);
            }
            Loadable::Error(error) => {
                self.base = None;
                self.controller.borrow_mut().selected = None;
                self.controller
                    .borrow_mut()
                    .set_files(Arc::default(), list.rev);
                self.error = Some(error.clone().into());
            }
            Loadable::Loading | Loadable::NotLoaded => {}
        }
        cx.notify();
    }

    pub(crate) fn set_source(&mut self, source: ChangeSource, cx: &mut gpui::Context<Self>) {
        self.source = source.clone();
        self.loading = true;
        self.error = None;
        self.base = None;
        self.controller.borrow_mut().selected = None;
        let revision = self.controller.borrow().files_rev.wrapping_add(1);
        self.controller
            .borrow_mut()
            .set_files(Arc::default(), revision);
        if let Some(store) = self.store.upgrade() {
            store.dispatch(Msg::DiffSession(DiffSessionMsg::OpenChanges {
                repo_id: self.repository.repo_id(),
                lifetime: self.repository.lifetime(),
                view: self.view_id,
                source,
            }));
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn test_parts(&self) -> (u64, UniformListScrollHandle, usize) {
        (
            self.view_id.0,
            self.scroll.clone(),
            self.controller.borrow_mut().group_builds,
        )
    }

    /// Regroup passes and grouped-row builds so far.
    #[cfg(test)]
    pub(crate) fn test_group_builds(&self) -> (usize, usize) {
        let controller = self.controller.borrow();
        (controller.bucket_builds, controller.group_builds)
    }

    #[cfg(test)]
    pub(crate) fn load_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// A click steps to the next layout, as in every changed-file list.
    fn set_layout(&mut self, layout: crate::view::FileListLayout, cx: &mut gpui::Context<Self>) {
        self.controller.borrow_mut().set_mode(mode_for(layout));
        cx.notify();
    }

    fn set_sort(&mut self, sort: CommitFileSort, cx: &mut gpui::Context<Self>) {
        self.controller.borrow_mut().set_sort(sort);
        cx.notify();
    }

    /// The layout icon and the sort button, the controls every changed-file
    /// list carries. Their menus go through the host, like an extension's.
    fn controls(&self, list_id: u64, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.host.theme(cx);
        let ui_scale = ui_scale::UiScale::current(cx);
        let (layout, sort) = {
            let controller = self.controller.borrow();
            (layout_for(controller.mode), controller.sort)
        };
        let layout_button = components::list_layout_button(
            format!("hosted_file_list_{list_id}_layout_button"),
            layout,
            theme,
            ui_scale,
        )
        .on_click(theme, cx, move |list, event, _window, cx| {
            if event.standard_click() {
                list.set_layout(layout.next(), cx);
            }
        })
        .on_pointer_click(
            gpui::MouseButton::Right,
            cx.listener(move |list, event: &gpui::MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                let weak = cx.weak_entity();
                let items = menu(
                    "Layout",
                    crate::view::FileListLayout::ALL.map(|option| {
                        let weak = weak.clone();
                        (option.label(), option == layout, move |cx: &mut App| {
                            let _ = weak.update(cx, |list, cx| list.set_layout(option, cx));
                        })
                    }),
                );
                let _ = list.host.open_menu(event.position, items, cx);
            }),
        )
        .debug_selector(move || format!("hosted_file_list_{list_id}_layout_button"))
        .gitcomet_tooltip(theme, layout.tooltip());

        let sort_button = components::Button::new(format!("hosted_file_list_{list_id}_sort"), "")
            .style(components::ButtonStyle::Transparent)
            .start_slot(svg_icon(
                "icons/sort.svg",
                theme.colors.foreground.secondary,
                ui_scale.px(14.0),
            ))
            .on_click_with_bounds(theme, cx, move |list, event, bounds, _window, cx| {
                if !event.standard_click() {
                    return;
                }
                let weak = cx.weak_entity();
                let items = menu(
                    "Sort files",
                    CommitFileSort::ALL.map(|option| {
                        let weak = weak.clone();
                        (option.label(), option == sort, move |cx: &mut App| {
                            let _ = weak.update(cx, |list, cx| list.set_sort(option, cx));
                        })
                    }),
                );
                let _ = list.host.open_menu(bounds.bottom_left(), items, cx);
            })
            .debug_selector(move || format!("hosted_file_list_{list_id}_sort"))
            .gitcomet_tooltip(theme, format!("Sort: {}", sort.label()).into());

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(layout_button)
            .child(sort_button)
            .into_any_element()
    }

    fn pick(&mut self, change: CommitFileChange, cx: &mut gpui::Context<Self>) {
        let target = self.source.target_for(&change, self.base.as_ref());
        let on_select = Rc::clone(&self.on_select);
        cx.notify();
        // Deferred: the callback may update other panes, or this list.
        cx.defer(move |cx| on_select(&change, target, cx));
    }

    fn file_row(
        &mut self,
        ix: usize,
        ordinal: usize,
        depth: usize,
        is_tree: bool,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let theme = self.host.theme(cx);
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let list_id = self.view_id.0;
        let (change, presentation) = self
            .controller
            .borrow_mut()
            .presentation_at_ordinal(ordinal)?;
        let selected = self.controller.borrow_mut().selected.as_ref() == Some(&change.path);
        let path = change.path.clone();
        let (row, tooltip) = crate::view::rows::changed_file_row(
            crate::view::rows::ChangedFileRow {
                element_id: ("hosted_file_list_file", ix).into(),
                row_group: format!("hosted_file_list_{list_id}_row_{ix}").into(),
                selector: move || format!("hosted_file_list_{list_id}_file_{}", path.display()),
                file: &change,
                presentation: &presentation,
                is_tree,
                depth,
                selected,
                context_menu_active: false,
                path_alignment_group: None,
                diff_stat: true,
                leading: self.marks_glyphs.then(|| {
                    let path = change.path.clone();
                    mark_glyph(
                        self.marks.rows.get(&change.path),
                        move || format!("hosted_file_list_{list_id}_glyph_{}", path.display()),
                        theme,
                        ui_scale::UiScale::current(cx),
                    )
                }),
            },
            theme,
            ui_scale_percent,
            cx,
        );
        let picked = change.path.clone();
        let mark = self.marks.rows.get(&change.path).cloned();
        let label = mark
            .as_ref()
            .and_then(|mark| Some((mark.label.clone()?, mark.color)));
        Some(
            row.when_some(label, |row, (label, color)| {
                row.child(div().flex_none().text_color(color).child(label))
            })
            .on_activate(
                false,
                controls::ControlActivation::Composite,
                cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                    if !e.standard_click() {
                        return;
                    }
                    let change = this.controller.borrow_mut().select(&picked);
                    if let Some(change) = change {
                        this.pick(change, cx);
                    }
                }),
            )
            .gitcomet_tooltip(theme, tooltip)
            .into_any_element(),
        )
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = self.host.theme(cx);
        let ui_scale = ui_scale::UiScale::current(cx);
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let row_height =
            crate::view::rows::sidebar::sidebar_list_row_height(theme, ui_scale_percent);
        let list_id = self.view_id.0;
        if self.controller.borrow_mut().mode == FileListMode::Grouped {
            let grouped = self.controller.borrow_mut().grouped();
            let list = cx.weak_entity();
            return range
                .filter_map(|ix| match *grouped.rows.get(ix)? {
                    GroupedRow::Header { .. } => group_header(
                        &list, list_id, &grouped, ix, false, theme, ui_scale, row_height,
                    ),
                    GroupedRow::File { ordinal } => self.file_row(ix, ordinal, 0, false, cx),
                })
                .collect();
        }
        let plan = self.controller.borrow_mut().plan();
        range
            .filter_map(|ix| {
                let row = plan.row_at(RowIx(ix))?;
                match row {
                    FileListRow::File { ordinal, depth } => {
                        self.file_row(ix, ordinal.0, depth, plan.is_tree(), cx)
                    }
                    directory => {
                        let (element, toggle) = crate::view::rows::changed_file_directory_row(
                            ("hosted_file_list_dir", ix).into(),
                            move || format!("hosted_file_list_{list_id}_dir_{ix}"),
                            directory,
                            false,
                            theme,
                            ui_scale_percent,
                        )?;
                        let crate::view::rows::DirectoryToggle {
                            key,
                            chain,
                            collapsed,
                        } = toggle;
                        Some(
                            element
                                .on_activate(
                                    false,
                                    controls::ControlActivation::Composite,
                                    cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                                        if !e.standard_click() {
                                            return;
                                        }
                                        this.controller.borrow_mut().toggle_dir(
                                            Arc::clone(&key),
                                            &chain,
                                            collapsed,
                                        );
                                        cx.notify();
                                    }),
                                )
                                .into_any_element(),
                        )
                    }
                }
            })
            .collect()
    }
}

/// The header at row `row` of a grouped list, drawn in place or pinned
/// (`sticky`); clicking it collapses or expands the group.
#[allow(clippy::too_many_arguments)]
fn group_header(
    list: &gpui::WeakEntity<FileListView>,
    list_id: u64,
    grouped: &GroupedRows,
    row: usize,
    sticky: bool,
    theme: AppTheme,
    ui_scale: ui_scale::UiScale,
    row_height: Pixels,
) -> Option<AnyElement> {
    let GroupedRow::Header {
        group,
        count,
        collapsed,
    } = *grouped.rows.get(row)?
    else {
        return None;
    };
    let label = grouped.labels.get(group).cloned().unwrap_or_default();
    let role = if sticky { "sticky" } else { "group" };
    let list = list.clone();
    Some(crate::view::rows::group_header_row(
        crate::view::rows::GroupHeaderProps {
            id: (
                if sticky {
                    "hosted_file_list_sticky"
                } else {
                    "hosted_file_list_group"
                },
                group,
            ),
            selector: format!("hosted_file_list_{list_id}_{role}_{label}"),
            label,
            count,
            collapsed,
        },
        theme,
        ui_scale,
        row_height,
        move |cx| {
            let _ = list.update(cx, |list, cx| {
                list.controller.borrow_mut().toggle_group(group);
                cx.notify();
            });
        },
    ))
}

/// Whether `chip`'s filter is the one applied.
fn chip_active(
    controller: &FileListController,
    chip: &gitcomet_extension_api::FileListFilterChip,
) -> bool {
    use gitcomet_extension_api::FileListFilterChip as Chip;
    match chip {
        Chip::Query { query, .. } => !query.is_empty() && controller.query == *query,
        Chip::Visible { visible, .. } => controller.shows_only(visible),
        _ => false,
    }
}

fn mode_for(layout: crate::view::FileListLayout) -> FileListMode {
    match layout {
        crate::view::FileListLayout::Flat => FileListMode::Flat,
        crate::view::FileListLayout::Tree => FileListMode::Tree,
        crate::view::FileListLayout::Groups => FileListMode::Grouped,
    }
}

fn layout_for(mode: FileListMode) -> crate::view::FileListLayout {
    match mode {
        FileListMode::Flat => crate::view::FileListLayout::Flat,
        FileListMode::Grouped => crate::view::FileListLayout::Groups,
        _ => crate::view::FileListLayout::Tree,
    }
}

/// A choice menu: a header, then one entry per option, the current checked.
fn menu<const N: usize>(
    header: &'static str,
    options: [(&'static str, bool, impl Fn(&mut App) + 'static); N],
) -> Vec<gitcomet_extension_api::HostedMenuItem> {
    use gitcomet_extension_api::{HostedAction, HostedMenuItem};
    let mut items = vec![
        HostedMenuItem::Header(header.into()),
        HostedMenuItem::Separator,
    ];
    for (label, current, run) in options {
        let item = HostedMenuItem::action(HostedAction::new(label, run));
        items.push(if current {
            item.with_icon("icons/check.svg")
        } else {
            item
        });
    }
    items
}

/// A worktree source's counts and edits, which its change list does not
/// carry: they come from the lane its status rows read, with that lane's
/// revision. `None` for the other sources, or while the lane is unloaded.
fn worktree_line_stats<'a>(
    repo: &'a gitcomet_state::model::RepoState,
    source: &ChangeSource,
) -> Option<(
    u64,
    &'a rustc_hash::FxHashMap<std::path::PathBuf, gitcomet_core::domain::LineStats>,
)> {
    match source {
        ChangeSource::Worktree { area, .. } => {
            Some((repo.line_stats_rev(*area), repo.line_stats_for_area(*area)?))
        }
        ChangeSource::LinkedWorktree { path, area, .. } => {
            let Loadable::Ready(dirty) = &repo.worktree_dirty else {
                return None;
            };
            let summary = dirty.iter().find(|summary| &summary.path == path)?;
            Some((repo.worktree_dirty_rev, summary.line_stats.for_area(*area)))
        }
        _ => None,
    }
}

/// `files` with the counts and edits `stats` has for them. A new vector, so
/// every cache keyed on the files sees them change.
fn with_line_stats(
    files: &[CommitFileChange],
    stats: &rustc_hash::FxHashMap<std::path::PathBuf, gitcomet_core::domain::LineStats>,
) -> Arc<Vec<CommitFileChange>> {
    Arc::new(
        files
            .iter()
            .map(|file| match stats.get(&file.path) {
                Some(stats) => file.clone().with_line_stats(*stats),
                None => file.clone(),
            })
            .collect(),
    )
}

/// Applies `chip`'s filter, or clears it.
fn apply_chip(
    controller: &mut FileListController,
    chip: &gitcomet_extension_api::FileListFilterChip,
    apply: bool,
) {
    use gitcomet_extension_api::FileListFilterChip as Chip;
    match chip {
        Chip::Query { query, .. } => {
            controller.set_query(if apply { query.clone() } else { "".into() })
        }
        Chip::Visible { visible, .. } => controller.set_visible(apply.then(|| visible.clone())),
        _ => {}
    }
}

/// A file row's glyph column: icon-wide, empty without a glyph.
fn mark_glyph(
    mark: Option<&gitcomet_extension_api::RowMark>,
    selector: impl FnOnce() -> String + 'static,
    theme: AppTheme,
    ui_scale: ui_scale::UiScale,
) -> AnyElement {
    use gitcomet_extension_api::RowGlyph;
    let slot = div()
        .flex_none()
        .w(ui_scale.px(14.0))
        .flex()
        .justify_center();
    let Some((glyph, color)) = mark.and_then(|mark| Some((mark.glyph.as_ref()?, mark.color)))
    else {
        return slot.into_any_element();
    };
    let slot = slot.debug_selector(selector).text_color(color);
    match glyph {
        RowGlyph::Icon(path) => slot
            .child(
                gpui::svg()
                    .path(path.clone())
                    .size(ui_scale.px(14.0))
                    .text_color(color),
            )
            .into_any_element(),
        RowGlyph::Text(text) => slot
            .text_size(theme.ui_text(11.0))
            .child(text.clone())
            .into_any_element(),
        _ => slot.into_any_element(),
    }
}

impl Render for FileListView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.host.theme(cx);
        let rows = self.controller.borrow_mut().row_count();
        let list_id = self.view_id.0;
        let grouped = self.controller.borrow().mode == FileListMode::Grouped;
        let sticky = grouped.then(|| {
            let rows = self.controller.borrow_mut().grouped();
            let list = cx.weak_entity();
            let ui_scale = ui_scale::UiScale::current(cx);
            crate::view::rows::StickyGroupHeaders {
                headers: rows.headers(),
                header: Rc::new(move |row, row_height, _cx| {
                    group_header(
                        &list, list_id, &rows, row, true, theme, ui_scale, row_height,
                    )
                }),
            }
        });
        self.body.update(cx, |body, _| {
            body.refresh(
                None,
                sticky.map(|sticky| {
                    Rc::new(move |list: gpui::UniformList| list.with_decoration(sticky.clone()))
                        as _
                }),
            )
        });
        div()
            .id(("hosted_file_list", self.view_id.0))
            .debug_selector(move || format!("hosted_file_list_{list_id}"))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface.canvas)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .flex_wrap()
                            .gap_1()
                            .children(self.chips.iter().enumerate().map(|(ix, chip)| {
                                let active = chip_active(&self.controller.borrow(), chip);
                                let chip = chip.clone();
                                components::Button::new(
                                    format!("file_filter_{list_id}_{ix}"),
                                    chip.label().clone(),
                                )
                                .selected(active)
                                .on_click(
                                    theme,
                                    cx,
                                    move |list, _, _, cx| {
                                        apply_chip(
                                            &mut list.controller.borrow_mut(),
                                            &chip,
                                            !active,
                                        );
                                        cx.notify();
                                    },
                                )
                            })),
                    )
                    .child(self.controls(list_id, cx)),
            )
            .when_some(
                self.error.clone().or_else(|| {
                    (rows == 0).then(|| {
                        if self.loading {
                            "Loading…".into()
                        } else {
                            "No changes".into()
                        }
                    })
                }),
                |list, status| {
                    list.child(
                        div()
                            .p_2()
                            .text_color(theme.colors.foreground.secondary)
                            .child(status),
                    )
                },
            )
            .child(self.body.clone())
    }
}

impl Drop for FileListView {
    fn drop(&mut self) {
        if let Some(store) = self.store.upgrade() {
            store.dispatch(Msg::DiffSession(DiffSessionMsg::CloseChanges {
                repo_id: self.repository.repo_id(),
                lifetime: self.repository.lifetime(),
                view: self.view_id,
            }));
        }
    }
}

/// The extension-facing handle; it owns the list.
pub(crate) struct HostedFileList {
    pub(crate) entity: Entity<FileListView>,
}

impl FileListImpl for HostedFileList {
    fn view(&self) -> gpui::AnyView {
        self.entity.clone().into()
    }

    fn set_source(&self, source: ChangeSource, cx: &mut App) {
        self.entity
            .update(cx, |list, cx| list.set_source(source, cx));
    }

    fn set_mode(&self, mode: FileListMode, cx: &mut App) {
        self.entity.update(cx, |list, cx| {
            list.controller.borrow_mut().set_mode(mode);
            cx.notify();
        });
    }

    fn set_sort(&self, sort: gitcomet_extension_api::FileListSort, cx: &mut App) {
        let sort = match sort {
            gitcomet_extension_api::FileListSort::PathAscending => CommitFileSort::PathAscending,
            gitcomet_extension_api::FileListSort::PathDescending => CommitFileSort::PathDescending,
            gitcomet_extension_api::FileListSort::FileTypeAscending => {
                CommitFileSort::FileTypeAscending
            }
            gitcomet_extension_api::FileListSort::FileTypeDescending => {
                CommitFileSort::FileTypeDescending
            }
            gitcomet_extension_api::FileListSort::EditSizeAscending => {
                CommitFileSort::EditSizeAscending
            }
            gitcomet_extension_api::FileListSort::EditSizeDescending => {
                CommitFileSort::EditSizeDescending
            }
            gitcomet_extension_api::FileListSort::Edits => CommitFileSort::Edits,
            _ => CommitFileSort::PathAscending,
        };
        self.entity.update(cx, |list, cx| {
            list.controller.borrow_mut().set_sort(sort);
            cx.notify();
        });
    }
    fn set_kind_filter(&self, filter: gitcomet_extension_api::FileListFilter, cx: &mut App) {
        let filter = match filter {
            gitcomet_extension_api::FileListFilter::All => CommitFileFilter::All,
            gitcomet_extension_api::FileListFilter::Modified => CommitFileFilter::Modified,
            gitcomet_extension_api::FileListFilter::Removed => CommitFileFilter::Removed,
            gitcomet_extension_api::FileListFilter::Added => CommitFileFilter::Added,
            gitcomet_extension_api::FileListFilter::Renamed => CommitFileFilter::Renamed,
            _ => CommitFileFilter::All,
        };
        self.entity.update(cx, |list, cx| {
            list.controller.borrow_mut().kind_filter = filter;
            cx.notify();
        });
    }
    fn set_marks(&self, marks: gitcomet_extension_api::FileListMarks, cx: &mut App) {
        self.entity.update(cx, |list, cx| {
            if list.marks.revision != marks.revision || !Arc::ptr_eq(&list.marks.rows, &marks.rows)
            {
                list.marks_glyphs = marks.rows.values().any(|mark| mark.glyph.is_some());
                list.marks = marks;
                cx.notify();
            }
        });
    }
    fn set_filter_chips(
        &self,
        chips: Vec<gitcomet_extension_api::FileListFilterChip>,
        cx: &mut App,
    ) {
        self.entity.update(cx, |list, cx| {
            // An active chip follows its replacement of the same label.
            let mut controller = list.controller.borrow_mut();
            for old in &list.chips {
                if let Some(new) = chips.iter().find(|new| new.label() == old.label())
                    && chip_active(&controller, old)
                    && !chip_active(&controller, new)
                {
                    apply_chip(&mut controller, new, true);
                }
            }
            drop(controller);
            list.chips = chips;
            cx.notify();
        });
    }

    fn set_groups(&self, groups: Option<gitcomet_extension_api::FileListGroups>, cx: &mut App) {
        self.entity.update(cx, |list, cx| {
            list.controller.borrow_mut().set_groups(groups);
            cx.notify();
        });
    }

    fn set_visible(&self, visible: Option<gitcomet_extension_api::FileListVisible>, cx: &mut App) {
        self.entity.update(cx, |list, cx| {
            list.controller.borrow_mut().set_visible(visible);
            cx.notify();
        });
    }

    fn set_filter(&self, query: SharedString, cx: &mut App) {
        self.entity.update(cx, |list, cx| {
            list.controller.borrow_mut().set_query(query);
            cx.notify();
        });
    }

    fn files(&self, cx: &App) -> Vec<CommitFileChange> {
        self.entity.read(cx).controller.borrow_mut().shown_changes()
    }

    fn selected(&self, cx: &App) -> Option<CommitFileChange> {
        self.entity.read(cx).controller.borrow().selected()
    }

    fn select_path(&self, path: &Path, cx: &mut App) -> bool {
        self.entity.update(cx, |list, cx| {
            let change = list.controller.borrow_mut().select(path);
            match change {
                Some(change) => {
                    list.pick(change, cx);
                    true
                }
                None => false,
            }
        })
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.entity.read(cx).loading
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::FileStatusKind;

    fn change(path: &str, kind: FileStatusKind) -> CommitFileChange {
        CommitFileChange::new(PathBuf::from(path), kind)
    }

    /// A worktree source's list carries no counts; they join from its lane,
    /// keyed on that lane's revision, so the edit-size and Edits sorts have
    /// something to read.
    #[test]
    fn worktree_sources_join_their_lanes_counts_and_edits() {
        use gitcomet_core::domain::{DiffArea, LineStats, UncommittedLineStats};
        let mut repo = gitcomet_state::model::RepoState::new_opening(
            gitcomet_state::model::RepoId(1),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/hosted-line-stats"),
            },
        );
        let mut stats = LineStats::from((Some(2), Some(1)));
        let mut edit = gitcomet_core::edit_signature::EditSignatureBuilder::default();
        edit.added(b"x");
        stats.edit = edit.finish();
        repo.uncommitted_line_stats = Loadable::Ready(Arc::new(UncommittedLineStats {
            staged: Default::default(),
            unstaged: [(PathBuf::from("a.rs"), stats)].into_iter().collect(),
        }));
        repo.unstaged_line_stats_rev = 7;

        let unstaged = ChangeSource::worktree(DiffArea::Unstaged, false);
        let (rev, lane) = worktree_line_stats(&repo, &unstaged).expect("the lane is loaded");
        assert_eq!(rev, 7);
        let files = with_line_stats(
            &[
                change("a.rs", FileStatusKind::Modified),
                change("b.rs", FileStatusKind::Modified),
            ],
            lane,
        );
        assert_eq!((files[0].additions, files[0].deletions), (Some(2), Some(1)));
        assert_eq!(files[0].edit, stats.edit);
        assert_eq!(files[1].edit, None, "a file the lane lacks is left alone");

        assert!(
            worktree_line_stats(&repo, &ChangeSource::Commit(CommitId("HEAD".into()))).is_none(),
            "a commit's list carries its own counts"
        );
    }

    #[test]
    fn replacing_files_at_the_same_revision_updates_row_presentations() {
        let mut list = FileListController::new(FileListMode::Flat);
        list.set_files(
            Arc::new(vec![change("before.rs", FileStatusKind::Added)]),
            1,
        );
        let before = list.presentation_at_ordinal(0).unwrap().1;
        assert_eq!(before.label, "before.rs");
        list.set_files(
            Arc::new(vec![change("after.rs", FileStatusKind::Deleted)]),
            1,
        );
        let (file, presentation) = list.presentation_at_ordinal(0).unwrap();
        assert_eq!(file.path, Path::new("after.rs"));
        assert_eq!(presentation.label, "after.rs");
        assert_ne!(presentation.visuals.icon, before.visuals.icon);
    }

    #[test]
    fn each_list_filters_groups_collapses_and_selects_on_its_own() {
        let files = Arc::new(vec![
            change("src/b.rs", FileStatusKind::Modified),
            change("src/a.rs", FileStatusKind::Added),
            change("README.md", FileStatusKind::Modified),
        ]);
        let mut left = FileListController::new(FileListMode::Tree);
        let mut right = FileListController::new(FileListMode::Flat);
        left.set_files(Arc::clone(&files), 1);
        right.set_files(Arc::clone(&files), 1);

        let paths = |list: &mut FileListController| {
            list.shown_changes()
                .into_iter()
                .map(|change| change.path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(&mut right), vec!["README.md", "src/a.rs", "src/b.rs"]);

        left.set_query("src".into());
        assert_eq!(paths(&mut left), vec!["src/a.rs", "src/b.rs"]);
        assert_eq!(
            paths(&mut right).len(),
            3,
            "the other list keeps its filter"
        );

        // The tree groups `src/`; collapsing it hides its files.
        let plan = left.plan();
        assert!(plan.is_tree());
        let rows = plan.row_len();
        let Some(FileListRow::Directory {
            key,
            chain,
            collapsed,
            ..
        }) = plan.row_at(RowIx(0))
        else {
            panic!("the first row is the src directory");
        };
        left.toggle_dir(key, &chain, collapsed);
        assert!(left.plan().row_len() < rows);

        assert!(
            left.select(Path::new("README.md")).is_none(),
            "filtered out"
        );
        assert!(right.select(Path::new("README.md")).is_some());
        assert_eq!(
            right.selected().map(|change| change.path),
            Some(PathBuf::from("README.md"))
        );
        assert!(left.selected().is_none());
    }

    fn by_role() -> gitcomet_extension_api::FileListGroups {
        gitcomet_extension_api::FileListGroups::new(
            1,
            vec!["Code".into(), "Docs".into()],
            |path: &Path| match path.extension()?.to_str()? {
                "rs" => Some(0),
                "md" => Some(1),
                // Out of range: shown under "Other".
                "toml" => Some(7),
                _ => None,
            },
        )
    }

    fn header_labels(list: &mut FileListController) -> Vec<String> {
        let grouped = list.grouped();
        grouped
            .rows
            .iter()
            .filter_map(|row| match row {
                GroupedRow::Header { group, count, .. } => {
                    Some(format!("{} ({count})", grouped.labels[*group]))
                }
                GroupedRow::File { .. } => None,
            })
            .collect()
    }

    #[test]
    fn custom_groups_keep_their_order_and_show_the_rest_last() {
        let mut list = FileListController::new(FileListMode::Grouped);
        list.set_files(
            Arc::new(vec![
                change("Cargo.toml", FileStatusKind::Modified),
                change("README.md", FileStatusKind::Modified),
                change("build.sh", FileStatusKind::Added),
                change("src/a.rs", FileStatusKind::Added),
                change("src/b.rs", FileStatusKind::Deleted),
            ]),
            1,
        );
        list.set_groups(Some(by_role()));
        assert_eq!(
            header_labels(&mut list),
            vec!["Code (2)", "Docs (1)", "Other (2)"],
            "empty groups are skipped; unknown and out-of-range go last"
        );
        let paths: Vec<_> = list
            .shown_changes()
            .into_iter()
            .map(|change| change.path)
            .collect();
        assert_eq!(
            paths,
            [
                "src/a.rs",
                "src/b.rs",
                "README.md",
                "build.sh",
                "Cargo.toml"
            ]
            .map(PathBuf::from)
            .to_vec(),
            "navigation follows the groups, each in the list's sort"
        );
        list.set_groups(None);
        assert_eq!(
            header_labels(&mut list),
            vec!["Added (2)", "Modified (2)", "Deleted (1)"]
        );
    }

    #[test]
    fn groups_collapse_on_their_own_and_new_labels_expand_them() {
        let mut list = FileListController::new(FileListMode::Grouped);
        list.set_files(
            Arc::new(vec![
                change("a.rs", FileStatusKind::Modified),
                change("b.md", FileStatusKind::Modified),
            ]),
            1,
        );
        list.set_groups(Some(by_role()));
        assert_eq!(list.row_count(), 4);
        list.toggle_group(0);
        assert_eq!(list.row_count(), 3, "only Code collapses");

        let mut next = by_role();
        next.revision = 2;
        list.set_groups(Some(next));
        assert_eq!(list.row_count(), 3, "the same labels keep the collapse");

        list.set_groups(Some(gitcomet_extension_api::FileListGroups::new(
            3,
            vec!["Rust".into(), "Docs".into()],
            |_: &Path| Some(0),
        )));
        assert_eq!(list.row_count(), 3, "new labels: one group, expanded");
    }

    #[test]
    fn a_new_grouping_revision_regroups_once_without_replanning() {
        let mut list = FileListController::new(FileListMode::Grouped);
        list.set_files(
            Arc::new(
                (0..100)
                    .map(|n| change(&format!("src/f{n}.rs"), FileStatusKind::Modified))
                    .collect(),
            ),
            1,
        );
        list.set_groups(Some(by_role()));
        list.plan();
        list.grouped();
        let (plans, buckets, rows) = (
            list.plan_cache.builds(),
            list.bucket_builds,
            list.group_builds,
        );
        list.grouped();
        assert_eq!(list.bucket_builds, buckets, "unchanged input is cached");

        let mut next = by_role();
        next.revision = 2;
        list.set_groups(Some(next));
        list.grouped();
        list.plan();
        assert_eq!(list.bucket_builds, buckets + 1, "one regroup pass");
        assert_eq!(list.group_builds, rows + 1);
        assert_eq!(list.plan_cache.builds(), plans, "groups never replan");

        list.toggle_group(0);
        list.grouped();
        assert_eq!(
            list.bucket_builds,
            buckets + 1,
            "collapse reuses the groups"
        );
        assert_eq!(list.group_builds, rows + 2);
    }

    #[test]
    fn the_visible_set_composes_with_the_kind_filter_and_query() {
        let mut list = FileListController::new(FileListMode::Flat);
        list.set_files(
            Arc::new(vec![
                change("src/a.rs", FileStatusKind::Modified),
                change("src/b.rs", FileStatusKind::Added),
                change("src/c.rs", FileStatusKind::Modified),
                change("docs/c.md", FileStatusKind::Modified),
            ]),
            1,
        );
        let paths = |list: &mut FileListController| {
            list.shown_changes()
                .into_iter()
                .map(|change| change.path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        let visible = gitcomet_extension_api::FileListVisible::new(
            1,
            ["src/a.rs", "src/b.rs", "docs/c.md"]
                .map(PathBuf::from)
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
        );
        list.set_visible(Some(visible.clone()));
        assert_eq!(paths(&mut list), vec!["docs/c.md", "src/a.rs", "src/b.rs"]);
        assert!(list.shows_only(&visible));

        list.kind_filter = CommitFileFilter::Modified;
        assert_eq!(paths(&mut list), vec!["docs/c.md", "src/a.rs"]);
        list.set_query("src".into());
        assert_eq!(paths(&mut list), vec!["src/a.rs"]);

        list.set_visible(None);
        assert_eq!(paths(&mut list), vec!["src/a.rs", "src/c.rs"]);
        assert!(!list.shows_only(&visible));
    }

    #[test]
    fn grouped_lists_order_groups_collapse_them_and_find_headers() {
        let files = Arc::new(vec![
            change("m1.rs", FileStatusKind::Modified),
            change("a1.rs", FileStatusKind::Added),
            change("m2.rs", FileStatusKind::Modified),
            change("d1.rs", FileStatusKind::Deleted),
        ]);
        let mut list = FileListController::new(FileListMode::Grouped);
        list.set_files(files, 1);
        let grouped = list.grouped();
        let header = |group, count| GroupedRow::Header {
            group,
            count,
            collapsed: false,
        };
        let added = group_of(FileStatusKind::Added);
        let modified = group_of(FileStatusKind::Modified);
        let deleted = group_of(FileStatusKind::Deleted);
        assert_eq!(
            grouped.rows,
            vec![
                header(added, 1),
                GroupedRow::File { ordinal: 0 },
                // Ordinals follow the shown (path-sorted) order.
                header(modified, 2),
                GroupedRow::File { ordinal: 2 },
                GroupedRow::File { ordinal: 3 },
                header(deleted, 1),
                GroupedRow::File { ordinal: 1 },
            ]
        );
        assert_eq!(grouped.header_for(4), Some(2));
        assert_eq!(grouped.next_header(4), Some(5));
        assert_eq!(grouped.next_header(5), None);

        let builds = list.group_builds;
        list.grouped();
        assert_eq!(list.group_builds, builds, "unchanged input is cached");
        list.toggle_group(modified);
        assert_eq!(list.row_count(), 5);
        assert_eq!(list.group_builds, builds + 1);
    }
}
