//! A hosted file list: one list's own filter, sort, collapse, selection, and
//! scroll over a change-list session, built on the shared projection and
//! tree plan every changed-file list uses.

use super::*;
use crate::view::rows::{
    CollapsedDirs, CommitFileFilter, CommitFileProjectionCache, CommitFileSort, FileListPlan,
    FileListPlanCache, FileListRow, FileTree, FileTreeItem, RowIx,
};
use gitcomet_core::domain::{CommitFileChange, CommitId};
use gitcomet_extension_api::{
    ChangeSource, FileListImpl, FileSelected, RepositoryHandle, StateSubscription, WindowHost,
};
use gitcomet_state::diff_session::{DiffSessionMsg, DiffViewId};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// One list's presentation state, independent of every other list.
pub(in crate::view) struct FileListController {
    files: Arc<Vec<CommitFileChange>>,
    files_rev: u64,
    sort: CommitFileSort,
    kind_filter: CommitFileFilter,
    query: SharedString,
    layout: FileListLayout,
    collapsed: CollapsedDirs,
    selected: Option<PathBuf>,
    projection_cache: CommitFileProjectionCache<u64>,
    plan_cache: FileListPlanCache,
    shown: Option<(u64, Arc<[usize]>)>,
}

impl FileListController {
    pub(in crate::view) fn new(layout: FileListLayout) -> Self {
        Self {
            files: Arc::default(),
            files_rev: 0,
            sort: CommitFileSort::default(),
            kind_filter: CommitFileFilter::default(),
            query: SharedString::default(),
            layout,
            collapsed: CollapsedDirs::default(),
            selected: None,
            projection_cache: CommitFileProjectionCache::default(),
            plan_cache: FileListPlanCache::default(),
            shown: None,
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
        self.plan_cache
            .plan_for(key, self.layout, &self.collapsed, shown.len(), || {
                FileTree::build(
                    shown.iter().map(|&ix| FileTreeItem {
                        path: &files[ix].path,
                        additions: files[ix].additions,
                        deletions: files[ix].deletions,
                    }),
                    sort,
                )
            })
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
            controller: FileListController::new(FileListLayout::Tree),
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

    fn pick(&mut self, change: CommitFileChange, cx: &mut gpui::Context<Self>) {
        let target = self.source.target_for(&change, self.base.as_ref());
        let on_select = Rc::clone(&self.on_select);
        cx.notify();
        // Deferred: the callback may update other panes, or this list.
        cx.defer(move |cx| on_select(&change, target, cx));
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = self.host.theme(cx);
        let ui_scale = ui_scale::UiScale::current(cx);
        let plan = self.controller.plan();
        let selected = self.controller.selected.clone();
        let list_id = self.view_id.0;
        range
            .filter_map(|ix| {
                let row = plan.row_at(RowIx(ix))?;
                Some(match row {
                    FileListRow::Directory {
                        key,
                        label,
                        depth,
                        collapsed,
                        chain,
                        ..
                    } => div()
                        .id(("hosted_file_list_dir", ix))
                        .debug_selector(move || format!("hosted_file_list_{list_id}_dir_{ix}"))
                        .h(ui_scale.px(22.0))
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
                    FileListRow::File { ordinal, depth } => {
                        let change = self.controller.change_at_ordinal(ordinal.0)?;
                        let (icon, color) =
                            crate::view::rows::file_row_icon(&change.path, change.kind, &theme);
                        let is_selected = selected.as_ref() == Some(&change.path);
                        let name = if plan.is_tree() {
                            change
                                .path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        } else {
                            change.path.to_string_lossy().into_owned()
                        };
                        let path = change.path.clone();
                        div()
                            .id(("hosted_file_list_file", ix))
                            .debug_selector(move || {
                                format!("hosted_file_list_{list_id}_file_{}", path.display())
                            })
                            .h(ui_scale.px(22.0))
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
                            .into_any_element()
                    }
                })
            })
            .collect()
    }
}

impl Render for FileListView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.host.theme(cx);
        let rows = self.controller.plan().row_len();
        let list_id = self.view_id.0;
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
                .flex_1(),
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

    fn set_filter(&self, query: SharedString, cx: &mut App) {
        self.entity.update(cx, |list, cx| {
            list.controller.set_query(query);
            cx.notify();
        });
    }

    fn files(&self, cx: &App) -> Vec<CommitFileChange> {
        // Reading needs the projection cache; clone the controller's inputs.
        let list = self.entity.read(cx);
        let mut controller = FileListController::new(list.controller.layout);
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
        let mut left = FileListController::new(FileListLayout::Tree);
        let mut right = FileListController::new(FileListLayout::Flat);
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
}
