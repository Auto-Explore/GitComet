//! A hosted file list: one list's own filter, sort, collapse, selection, and
//! scroll over a change-list session, built on the shared projection and
//! tree plan every changed-file list uses. Grouped lists keep every row one
//! height (headers included) and pin the current group's header with a list
//! decoration, so scrolling never replans.

use super::*;
use crate::view::rows::{
    CollapsedDirs, CommitFileFilter, CommitFileProjectionCache, CommitFileSort, FileListPlan,
    FileListPlanCache, FileListRow, FileTree, FileTreeItem, RowIx,
};
use gitcomet_core::domain::{CommitFileChange, CommitId, FileStatusKind};
use gitcomet_extension_api::{
    ChangeSource, FileListImpl, FileListMode, FileSelected, RepositoryHandle, StateSubscription,
    WindowHost,
};
use gitcomet_state::diff_session::{DiffSessionMsg, DiffViewId};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Change kinds in the order grouped lists show them.
const GROUP_ORDER: [FileStatusKind; 6] = [
    FileStatusKind::Conflicted,
    FileStatusKind::Added,
    FileStatusKind::Modified,
    FileStatusKind::Renamed,
    FileStatusKind::Deleted,
    FileStatusKind::Untracked,
];

fn group_of(kind: FileStatusKind) -> usize {
    GROUP_ORDER
        .iter()
        .position(|candidate| *candidate == kind)
        .unwrap_or(GROUP_ORDER.len() - 1)
}

fn group_label(group: usize) -> &'static str {
    match GROUP_ORDER.get(group) {
        Some(FileStatusKind::Conflicted) => "Conflicted",
        Some(FileStatusKind::Added) => "Added",
        Some(FileStatusKind::Modified) => "Modified",
        Some(FileStatusKind::Renamed) => "Renamed",
        Some(FileStatusKind::Deleted) => "Deleted",
        _ => "Untracked",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum GroupedRow {
    Header {
        group: usize,
        count: usize,
        collapsed: bool,
    },
    /// The file at this position of the shown files.
    File { ordinal: usize },
}

/// A grouped list's rows, all one height.
#[derive(Debug, Default)]
pub(in crate::view) struct GroupedRows {
    pub(in crate::view) rows: Vec<GroupedRow>,
    /// The row of each header, ascending.
    headers: Vec<usize>,
}

impl GroupedRows {
    /// The header of the group holding `row`.
    pub(in crate::view) fn header_for(&self, row: usize) -> Option<usize> {
        let after = self.headers.partition_point(|&header| header <= row);
        after.checked_sub(1).map(|ix| self.headers[ix])
    }

    /// The first header below `row`.
    pub(in crate::view) fn next_header(&self, row: usize) -> Option<usize> {
        let after = self.headers.partition_point(|&header| header <= row);
        self.headers.get(after).copied()
    }
}

/// One list's presentation state, independent of every other list.
pub(in crate::view) struct FileListController {
    files: Arc<Vec<CommitFileChange>>,
    files_rev: u64,
    sort: CommitFileSort,
    kind_filter: CommitFileFilter,
    query: SharedString,
    mode: FileListMode,
    collapsed: CollapsedDirs,
    collapsed_groups: [bool; GROUP_ORDER.len()],
    selected: Option<PathBuf>,
    projection_cache: CommitFileProjectionCache<u64>,
    plan_cache: FileListPlanCache,
    shown: Option<(u64, Arc<[usize]>)>,
    grouped: Option<(u64, Arc<GroupedRows>)>,
    #[cfg(test)]
    pub(in crate::view) group_builds: usize,
}

impl FileListController {
    pub(in crate::view) fn new(mode: FileListMode) -> Self {
        Self {
            files: Arc::default(),
            files_rev: 0,
            sort: CommitFileSort::default(),
            kind_filter: CommitFileFilter::default(),
            query: SharedString::default(),
            mode,
            collapsed: CollapsedDirs::default(),
            collapsed_groups: [false; GROUP_ORDER.len()],
            selected: None,
            projection_cache: CommitFileProjectionCache::default(),
            plan_cache: FileListPlanCache::default(),
            shown: None,
            grouped: None,
            #[cfg(test)]
            group_builds: 0,
        }
    }

    pub(in crate::view) fn set_mode(&mut self, mode: FileListMode) {
        self.mode = mode;
    }

    fn plan_layout(&self) -> FileListLayout {
        match self.mode {
            FileListMode::Tree => FileListLayout::Tree,
            _ => FileListLayout::Flat,
        }
    }

    /// Rows in the current mode.
    pub(in crate::view) fn row_count(&mut self) -> usize {
        match self.mode {
            FileListMode::Grouped => self.grouped().rows.len(),
            _ => self.plan().row_len(),
        }
    }

    /// The shown files by change kind; rebuilt only when the shown files or
    /// the collapsed groups change.
    pub(in crate::view) fn grouped(&mut self) -> Arc<GroupedRows> {
        let mut hasher = rustc_hash::FxHasher::default();
        self.projection_key().hash(&mut hasher);
        self.collapsed_groups.hash(&mut hasher);
        let key = hasher.finish();
        if let Some((cached, grouped)) = &self.grouped
            && *cached == key
        {
            return Arc::clone(grouped);
        }
        let shown = self.shown();
        let mut groups = vec![Vec::new(); GROUP_ORDER.len()];
        for (ordinal, &ix) in shown.iter().enumerate() {
            groups[group_of(self.files[ix].kind)].push(ordinal);
        }
        let mut grouped = GroupedRows::default();
        for (group, ordinals) in groups.into_iter().enumerate() {
            if ordinals.is_empty() {
                continue;
            }
            let collapsed = self.collapsed_groups[group];
            grouped.headers.push(grouped.rows.len());
            grouped.rows.push(GroupedRow::Header {
                group,
                count: ordinals.len(),
                collapsed,
            });
            if !collapsed {
                grouped.rows.extend(
                    ordinals
                        .into_iter()
                        .map(|ordinal| GroupedRow::File { ordinal }),
                );
            }
        }
        #[cfg(test)]
        {
            self.group_builds += 1;
        }
        let grouped = Arc::new(grouped);
        self.grouped = Some((key, Arc::clone(&grouped)));
        grouped
    }

    pub(in crate::view) fn toggle_group(&mut self, group: usize) {
        if let Some(collapsed) = self.collapsed_groups.get_mut(group) {
            *collapsed = !*collapsed;
        }
    }

    pub(in crate::view) fn set_files(&mut self, files: Arc<Vec<CommitFileChange>>, rev: u64) {
        self.files = files;
        self.files_rev = rev;
    }

    pub(in crate::view) fn set_query(&mut self, query: SharedString) {
        self.query = query;
    }

    pub(in crate::view) fn set_sort(&mut self, sort: CommitFileSort) {
        self.sort = sort;
    }

    fn projection_key(&self) -> u64 {
        let mut hasher = rustc_hash::FxHasher::default();
        self.files_rev.hash(&mut hasher);
        Arc::as_ptr(&self.files).hash(&mut hasher);
        self.sort.hash(&mut hasher);
        self.kind_filter.hash(&mut hasher);
        self.query.hash(&mut hasher);
        hasher.finish()
    }

    /// Source indices of the files shown, in display order before grouping.
    pub(in crate::view) fn shown(&mut self) -> Arc<[usize]> {
        let key = self.projection_key();
        if let Some((cached, shown)) = &self.shown
            && *cached == key
        {
            return Arc::clone(shown);
        }
        let projection =
            self.projection_cache
                .projection_for(&key, &self.files, self.sort, self.kind_filter);
        let query = self.query.to_lowercase();
        let shown: Arc<[usize]> = if query.is_empty() {
            Arc::clone(&projection.source_indices)
        } else {
            projection
                .source_indices
                .iter()
                .copied()
                .filter(|&ix| {
                    self.files[ix]
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
                })
                .collect()
        };
        self.shown = Some((key, Arc::clone(&shown)));
        shown
    }

    pub(in crate::view) fn plan(&mut self) -> Arc<FileListPlan> {
        let shown = self.shown();
        let key = self.projection_key();
        let files = Arc::clone(&self.files);
        let sort = self.sort;
        self.plan_cache.plan_for(
            key,
            self.plan_layout(),
            &self.collapsed,
            shown.len(),
            || {
                FileTree::build(
                    shown.iter().map(|&ix| FileTreeItem {
                        path: &files[ix].path,
                        additions: files[ix].additions,
                        deletions: files[ix].deletions,
                    }),
                    sort,
                )
            },
        )
    }

    /// The change a file row shows.
    pub(in crate::view) fn change_at_ordinal(
        &mut self,
        ordinal: usize,
    ) -> Option<CommitFileChange> {
        let shown = self.shown();
        shown
            .get(ordinal)
            .and_then(|&ix| self.files.get(ix))
            .cloned()
    }

    pub(in crate::view) fn shown_changes(&mut self) -> Vec<CommitFileChange> {
        let shown = self.shown();
        shown.iter().map(|&ix| self.files[ix].clone()).collect()
    }

    pub(in crate::view) fn toggle_dir(
        &mut self,
        key: Arc<Path>,
        chain: &[Arc<Path>],
        collapsed: bool,
    ) {
        if collapsed {
            self.collapsed.expand(chain);
        } else {
            self.collapsed.collapse(key, chain);
        }
    }

    /// Selects `path` if it is shown.
    pub(in crate::view) fn select(&mut self, path: &Path) -> Option<CommitFileChange> {
        let change = self
            .shown_changes()
            .into_iter()
            .find(|change| change.path == path)?;
        self.selected = Some(change.path.clone());
        Some(change)
    }

    pub(in crate::view) fn selected(&self) -> Option<CommitFileChange> {
        let selected = self.selected.as_ref()?;
        self.files
            .iter()
            .find(|change| &change.path == selected)
            .cloned()
    }
}

pub(crate) struct FileListView {
    host: WindowHost,
    store: Arc<AppStore>,
    repository: RepositoryHandle,
    view_id: DiffViewId,
    controller: FileListController,
    source: ChangeSource,
    base: Option<CommitId>,
    list_rev: Option<u64>,
    loading: bool,
    on_select: FileSelected,
    scroll: UniformListScrollHandle,
    _state: Option<StateSubscription>,
}

impl FileListView {
    pub(crate) fn new(
        host: WindowHost,
        store: Arc<AppStore>,
        repository: RepositoryHandle,
        source: ChangeSource,
        on_select: FileSelected,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let view_id = DiffViewId::next();
        store.dispatch(Msg::DiffSession(DiffSessionMsg::OpenChanges {
            repo_id: repository.repo_id(),
            view: view_id,
            source: source.clone(),
        }));
        let weak = cx.weak_entity();
        let state = host
            .observe_state(move |_, cx| {
                let _ = weak.update(cx, |list, cx| list.sync(cx));
            })
            .ok();
        Self {
            host,
            store,
            repository,
            view_id,
            controller: FileListController::new(FileListMode::Tree),
            source,
            base: None,
            list_rev: None,
            loading: true,
            on_select,
            scroll: UniformListScrollHandle::default(),
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
        else {
            return;
        };
        if self.list_rev == Some(list.rev) {
            return;
        }
        self.list_rev = Some(list.rev);
        self.loading = matches!(list.files, Loadable::Loading | Loadable::NotLoaded);
        self.base = list.base.clone();
        if let Loadable::Ready(files) = &list.files {
            self.controller.set_files(Arc::clone(files), list.rev);
        }
        cx.notify();
    }

    pub(crate) fn set_source(&mut self, source: ChangeSource, cx: &mut gpui::Context<Self>) {
        self.source = source.clone();
        self.loading = true;
        self.store
            .dispatch(Msg::DiffSession(DiffSessionMsg::OpenChanges {
                repo_id: self.repository.repo_id(),
                view: self.view_id,
                source,
            }));
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn test_parts(&self) -> (u64, UniformListScrollHandle, usize) {
        (
            self.view_id.0,
            self.scroll.clone(),
            self.controller.group_builds,
        )
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
        name_only: bool,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let theme = self.host.theme(cx);
        let ui_scale = ui_scale::UiScale::current(cx);
        let list_id = self.view_id.0;
        let change = self.controller.change_at_ordinal(ordinal)?;
        let (icon, color) = crate::view::rows::file_row_icon(&change.path, change.kind, &theme);
        let is_selected = self.controller.selected.as_ref() == Some(&change.path);
        let name = if name_only {
            change
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            change.path.to_string_lossy().into_owned()
        };
        let path = change.path.clone();
        Some(
            div()
                .id(("hosted_file_list_file", ix))
                .debug_selector(move || {
                    format!("hosted_file_list_{list_id}_file_{}", path.display())
                })
                .h(ui_scale.px(ROW_HEIGHT_PX))
                .flex()
                .items_center()
                .gap(ui_scale.px(6.0))
                .pl(ui_scale.px(8.0 + 14.0 * depth as f32))
                .when(is_selected, |row| {
                    row.bg(theme.colors.interaction.selected_background)
                })
                .text_size(theme.ui_text(13.0))
                .text_color(theme.colors.foreground.primary)
                .child(crate::view::svg_icon(icon, color, ui_scale.px(14.0)))
                .child(div().truncate().child(name))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &gpui::MouseDownEvent, _, cx| {
                        if let Some(change) = this.controller.select(&change.path) {
                            this.pick(change, cx);
                        }
                    }),
                )
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
        let list_id = self.view_id.0;
        if self.controller.mode == FileListMode::Grouped {
            let grouped = self.controller.grouped();
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
                    )),
                    GroupedRow::File { ordinal } => self.file_row(ix, ordinal, 0, false, cx),
                })
                .collect();
        }
        let plan = self.controller.plan();
        range
            .filter_map(|ix| {
                let row = plan.row_at(RowIx(ix))?;
                match row {
                    FileListRow::Directory {
                        key,
                        label,
                        depth,
                        collapsed,
                        chain,
                        ..
                    } => Some(
                        div()
                            .id(("hosted_file_list_dir", ix))
                            .debug_selector(move || format!("hosted_file_list_{list_id}_dir_{ix}"))
                            .h(ui_scale.px(ROW_HEIGHT_PX))
                            .flex()
                            .items_center()
                            .pl(ui_scale.px(8.0 + 14.0 * depth as f32))
                            .text_size(theme.ui_text(13.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(if collapsed { "▸ " } else { "▾ " })
                            .child(label)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &gpui::MouseDownEvent, _, cx| {
                                    this.controller
                                        .toggle_dir(Arc::clone(&key), &chain, collapsed);
                                    cx.notify();
                                }),
                            )
                            .into_any_element(),
                    ),
                    FileListRow::File { ordinal, depth } => {
                        self.file_row(ix, ordinal.0, depth, plan.is_tree(), cx)
                    }
                }
            })
            .collect()
    }
}

const ROW_HEIGHT_PX: f32 = 22.0;

struct GroupHeader {
    list: gpui::WeakEntity<FileListView>,
    list_id: u64,
    group: usize,
    count: usize,
    collapsed: bool,
    /// Drawn pinned over the rows rather than as a row.
    sticky: bool,
}

/// A group's header row, the same height as a file row; clicking it
/// collapses or expands the group.
fn group_header(header: GroupHeader, theme: AppTheme, ui_scale: ui_scale::UiScale) -> AnyElement {
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
        .h(ui_scale.px(ROW_HEIGHT_PX))
        .w_full()
        .flex()
        .items_center()
        .gap(ui_scale.px(4.0))
        .px(ui_scale.px(8.0))
        .bg(theme.colors.surface.panel)
        .text_size(theme.ui_text(12.0))
        .text_color(theme.colors.foreground.secondary)
        .child(if collapsed { "▸" } else { "▾" })
        .child(format!("{label} ({count})"))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            let _ = list.update(cx, |list, cx| {
                list.controller.toggle_group(group);
                cx.notify();
            });
        })
        .into_any_element()
}

/// Pins the header of the group at the top of the list over its rows; the
/// next header pushes it up as it arrives. Computed per frame from the
/// grouped rows it was given, so scrolling never replans.
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
            ))
            .into_any_element()
    }
}

impl Render for FileListView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.host.theme(cx);
        let rows = self.controller.row_count();
        let list_id = self.view_id.0;
        let sticky = (self.controller.mode == FileListMode::Grouped).then(|| StickyGroupHeader {
            list: cx.weak_entity(),
            list_id,
            grouped: self.controller.grouped(),
            theme,
            ui_scale: ui_scale::UiScale::current(cx),
        });
        div()
            .id(("hosted_file_list", self.view_id.0))
            .debug_selector(move || format!("hosted_file_list_{list_id}"))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface.canvas)
            .child(
                uniform_list(
                    "hosted_file_list_rows",
                    rows,
                    cx.processor(|this, range, _window, cx| this.render_rows(range, cx)),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .map(|list| match sticky {
                    Some(sticky) => list.with_decoration(sticky),
                    None => list,
                }),
            )
    }
}

impl Drop for FileListView {
    fn drop(&mut self) {
        self.store
            .dispatch(Msg::DiffSession(DiffSessionMsg::CloseChanges {
                repo_id: self.repository.repo_id(),
                view: self.view_id,
            }));
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
            list.controller.set_mode(mode);
            cx.notify();
        });
    }

    fn set_filter(&self, query: SharedString, cx: &mut App) {
        self.entity.update(cx, |list, cx| {
            list.controller.set_query(query);
            cx.notify();
        });
    }

    fn files(&self, cx: &App) -> Vec<CommitFileChange> {
        // Reading needs the projection cache; clone the controller's inputs.
        let list = self.entity.read(cx);
        let mut controller = FileListController::new(list.controller.mode);
        controller.set_files(
            Arc::clone(&list.controller.files),
            list.controller.files_rev,
        );
        controller.set_query(list.controller.query.clone());
        controller.set_sort(list.controller.sort);
        controller.shown_changes()
    }

    fn selected(&self, cx: &App) -> Option<CommitFileChange> {
        self.entity.read(cx).controller.selected()
    }

    fn select_path(&self, path: &Path, cx: &mut App) -> bool {
        self.entity
            .update(cx, |list, cx| match list.controller.select(path) {
                Some(change) => {
                    list.pick(change, cx);
                    true
                }
                None => false,
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
