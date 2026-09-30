//! A hosted file list: one list's own filter, sort, collapse, selection, and
//! scroll over a change-list session, built on the shared projection and
//! tree plan every changed-file list uses. Grouped lists keep every row one
//! height (headers included) and pin the current group's header with a list
//! decoration, so scrolling never replans.

use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::view::rows::{CommitFileFilter, CommitFileSort, FileListRow, RowIx};
use gitcomet_core::domain::{CommitFileChange, CommitId};
use gitcomet_extension_api::{
    ChangeSource, FileListImpl, FileListMode, FileSelected, RepositoryHandle, StateSubscription,
    WindowHost,
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
    chips: Vec<gitcomet_extension_api::FileListFilterChip>,
    base: Option<CommitId>,
    list_rev: Option<u64>,
    loading: bool,
    error: Option<SharedString>,
    on_select: FileSelected,
    #[cfg(test)]
    scroll: UniformListScrollHandle,
    _state: Option<StateSubscription>,
}

impl FileListView {
    #[cfg(feature = "benchmarks")]
    pub(in crate::view) fn benchmark_snapshot(
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
        let controller = Rc::new(std::cell::RefCell::new(FileListController::new(
            FileListMode::Tree,
        )));
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
        let Some(list) = state
            .repos
            .iter()
            .find(|repo| {
                repo.id == self.repository.repo_id()
                    && repo.lifetime() == self.repository.lifetime()
            })
            .and_then(|repo| repo.change_lists.get(&self.view_id))
            .filter(|list| list.source == self.source)
        else {
            return;
        };
        if self.list_rev == Some(list.rev) {
            return;
        }
        self.list_rev = Some(list.rev);
        self.loading = list.is_loading() || matches!(list.files, Loadable::NotLoaded);
        self.error = None;
        match &list.files {
            Loadable::Ready(files) => {
                self.base = list.base.clone();
                self.controller
                    .borrow_mut()
                    .set_files(Arc::clone(files), list.rev);
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

    #[cfg(test)]
    pub(crate) fn load_error(&self) -> Option<&str> {
        self.error.as_deref()
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
            },
            theme,
            ui_scale_percent,
            cx,
        );
        let picked = change.path.clone();
        let mark = self.marks.rows.get(&change.path).cloned();
        Some(
            row.when_some(mark, |row, mark| {
                row.child(div().flex_none().text_color(mark.color).child(mark.label))
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
                    GroupedRow::Header {
                        group,
                        count,
                        collapsed,
                    } => Some(group_header(
                        GroupHeader {
                            list: list.clone(),
                            list_id,
                            group,
                            count,
                            collapsed,
                            sticky: false,
                        },
                        theme,
                        ui_scale,
                        row_height,
                    )),
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

struct GroupHeader {
    list: gpui::WeakEntity<FileListView>,
    list_id: u64,
    group: usize,
    count: usize,
    collapsed: bool,
    /// Drawn pinned over the rows rather than as a row.
    sticky: bool,
}

/// A group's header row, `row_height` like every file row (the list is
/// uniform); clicking it collapses or expands the group.
fn group_header(
    header: GroupHeader,
    theme: AppTheme,
    ui_scale: ui_scale::UiScale,
    row_height: Pixels,
) -> AnyElement {
    let GroupHeader {
        list,
        list_id,
        group,
        count,
        collapsed,
        sticky,
    } = header;
    let label = group_label(group);
    let role = if sticky { "sticky" } else { "group" };
    div()
        .id((
            if sticky {
                "hosted_file_list_sticky"
            } else {
                "hosted_file_list_group"
            },
            group,
        ))
        .debug_selector(move || format!("hosted_file_list_{list_id}_{role}_{label}"))
        .h(row_height)
        .w_full()
        .flex()
        .items_center()
        .gap(ui_scale.px(4.0))
        .px(ui_scale.px(8.0))
        .bg(theme.colors.surface.panel)
        .control_interaction(
            controls::InteractionStyle::new(theme),
            controls::InteractionState::default(),
        )
        .text_size(theme.ui_text(12.0))
        .text_color(theme.colors.foreground.secondary)
        .child(if collapsed { "▸" } else { "▾" })
        .child(format!("{label} ({count})"))
        .on_activate(
            false,
            controls::ControlActivation::Action,
            move |_, _, cx| {
                cx.stop_propagation();
                let _ = list.update(cx, |list, cx| {
                    list.controller.borrow_mut().toggle_group(group);
                    cx.notify();
                });
            },
        )
        .into_any_element()
}

/// Pins the header of the group at the top of the list over its rows; the
/// next header pushes it up as it arrives. Computed per frame from the
/// grouped rows it was given, so scrolling never replans.
#[derive(Clone)]
struct StickyGroupHeader {
    list: gpui::WeakEntity<FileListView>,
    list_id: u64,
    grouped: Arc<GroupedRows>,
    theme: AppTheme,
    ui_scale: ui_scale::UiScale,
}

impl gpui::UniformListDecoration for StickyGroupHeader {
    fn compute(
        &self,
        _visible_range: std::ops::Range<usize>,
        _bounds: gpui::Bounds<Pixels>,
        scroll_offset: gpui::Point<Pixels>,
        item_height: Pixels,
        _item_count: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        let scrolled = -scroll_offset.y;
        if scrolled <= px(0.0) || item_height <= px(0.0) {
            return div().into_any_element();
        }
        let first = (scrolled / item_height).floor() as usize;
        let Some(GroupedRow::Header {
            group,
            count,
            collapsed,
        }) = self
            .grouped
            .header_for(first)
            .and_then(|header| self.grouped.rows.get(header).copied())
        else {
            return div().into_any_element();
        };
        // The decoration's origin scrolls with the rows; `scrolled` is the
        // viewport's top in its coordinates.
        let mut top = scrolled;
        if let Some(next) = self.grouped.next_header(first) {
            let next_top = item_height * next as f32 - scrolled;
            if next_top < item_height {
                top -= item_height - next_top;
            }
        }
        div()
            .absolute()
            .top(top)
            .left_0()
            .right_0()
            .occlude()
            .child(group_header(
                GroupHeader {
                    list: self.list.clone(),
                    list_id: self.list_id,
                    group,
                    count,
                    collapsed,
                    sticky: true,
                },
                self.theme,
                self.ui_scale,
                item_height,
            ))
            .into_any_element()
    }
}

impl Render for FileListView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.host.theme(cx);
        let rows = self.controller.borrow_mut().row_count();
        let list_id = self.view_id.0;
        let grouped = self.controller.borrow().mode == FileListMode::Grouped;
        let sticky = grouped.then(|| StickyGroupHeader {
            list: cx.weak_entity(),
            list_id,
            grouped: self.controller.borrow_mut().grouped(),
            theme,
            ui_scale: ui_scale::UiScale::current(cx),
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
            .when(!self.chips.is_empty(), |list| {
                list.child(div().flex().flex_wrap().gap_1().children(
                    self.chips.iter().enumerate().map(|(ix, chip)| {
                        let query = chip.query.clone();
                        components::Button::new(
                            format!("file_filter_{list_id}_{ix}"),
                            chip.label.clone(),
                        )
                        .on_click(theme, cx, move |list, _, _, cx| {
                            list.controller.borrow_mut().set_query(query.clone());
                            cx.notify();
                        })
                    }),
                ))
            })
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
            list.chips = chips;
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
