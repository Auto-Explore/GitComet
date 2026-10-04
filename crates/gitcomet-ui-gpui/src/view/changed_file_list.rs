//! The virtualized body used by built-in and extension-owned changed-file lists.
//!
//! A source adapter supplies row actions and presentation; this entity keeps the
//! controller, scroll and list identity together across parent renders. The
//! adapter is referenced weakly by its row callback, so unmounting releases it.
use super::file_list_controller::FileListController;
use super::*;
use std::{cell::RefCell, ops::Range, rc::Rc};

pub(in crate::view) type SharedFileListController = Rc<RefCell<FileListController>>;
type RenderRows = Rc<dyn Fn(Range<usize>, &mut Window, &mut App) -> Vec<AnyElement>>;
type Decorate = Rc<dyn Fn(gpui::UniformList) -> gpui::UniformList>;

pub(in crate::view) struct ChangedFileListView {
    controller: SharedFileListController,
    id: ElementId,
    /// Built-in status lists project richer status entries without copying them.
    row_count: Option<usize>,
    scroll: UniformListScrollHandle,
    render_rows: RenderRows,
    decorate: Option<Decorate>,
}

impl ChangedFileListView {
    pub(in crate::view) fn new(
        controller: SharedFileListController,
        id: impl Into<ElementId>,
        scroll: UniformListScrollHandle,
        render_rows: impl Fn(Range<usize>, &mut Window, &mut App) -> Vec<AnyElement> + 'static,
    ) -> Self {
        Self {
            controller,
            id: id.into(),
            row_count: None,
            scroll,
            render_rows: Rc::new(render_rows),
            decorate: None,
        }
    }

    /// Called from the parent's render. The list is mounted uncached, so it
    /// renders again right after, with the parent's selection, marks and
    /// theme. No notify: one raised during the draw dirtied the parent again
    /// on the next frame, so the parent re-rendered on every window frame.
    pub(in crate::view) fn refresh(
        &mut self,
        row_count: Option<usize>,
        decorate: Option<Decorate>,
    ) {
        self.row_count = row_count;
        self.decorate = decorate;
    }
}

impl Render for ChangedFileListView {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let rows = self
            .row_count
            .unwrap_or_else(|| self.controller.borrow_mut().row_count());
        let render = Rc::clone(&self.render_rows);
        let list = uniform_list(self.id.clone(), rows, move |range, window, cx| {
            render(range, window, cx)
        })
        .size_full()
        .flex_1()
        .min_h(px(0.0))
        .track_scroll(&self.scroll);
        restrict_scroll_to_vertical_axis(match &self.decorate {
            Some(decorate) => decorate(list),
            None => list,
        })
    }
}

pub(in crate::view) struct BuiltinFileList {
    pub controller: SharedFileListController,
    pub view: Option<Entity<ChangedFileListView>>,
}

impl Default for BuiltinFileList {
    fn default() -> Self {
        Self {
            controller: Rc::new(RefCell::new(FileListController::new(
                gitcomet_extension_api::FileListMode::Tree,
            ))),
            view: None,
        }
    }
}

type BuiltinRowRenderer = fn(
    &mut DetailsPaneView,
    Range<usize>,
    &mut Window,
    &mut gpui::Context<DetailsPaneView>,
) -> Vec<AnyElement>;

impl DetailsPaneView {
    pub(in crate::view) fn changed_file_list(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        count: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<ChangedFileListView> {
        use crate::view::rows::FileListId;
        let (id, scroll, render_rows): (ElementId, _, BuiltinRowRenderer) = match list {
            FileListId::CommitFiles => (
                ("commit_details_files_list", repo_id.0).into(),
                &self.commit_files_scroll,
                Self::render_commit_file_rows,
            ),
            FileListId::RangeFiles => (
                ("range_files_list", repo_id.0).into(),
                &self.range_files_scroll,
                Self::render_range_file_rows,
            ),
            FileListId::WorktreeFiles => (
                ("worktree_files_list", repo_id.0).into(),
                &self.worktree_files_scroll,
                Self::render_worktree_file_rows,
            ),
            FileListId::Status(StatusSection::CombinedUnstaged) => (
                "unstaged".into(),
                &self.unstaged_scroll,
                Self::render_unstaged_rows,
            ),
            FileListId::Status(StatusSection::Untracked) => (
                "untracked".into(),
                &self.untracked_scroll,
                Self::render_untracked_rows,
            ),
            FileListId::Status(StatusSection::Unstaged) => (
                "split_unstaged".into(),
                &self.unstaged_scroll,
                Self::render_split_unstaged_rows,
            ),
            FileListId::Status(StatusSection::Staged) => (
                "staged".into(),
                &self.staged_scroll,
                Self::render_staged_rows,
            ),
        };
        let sticky = self.sticky_group_headers(repo_id, list, cx);
        let mut lists = self.file_controllers.borrow_mut();
        let entry = lists.entry((repo_id, list)).or_default();
        let view = entry.view.get_or_insert_with(|| {
            let parent = cx.weak_entity();
            let controller = Rc::clone(&entry.controller);
            cx.new(|_| {
                ChangedFileListView::new(
                    controller,
                    id,
                    scroll.clone(),
                    move |range, window, cx| {
                        parent
                            .update(cx, |pane, cx| render_rows(pane, range, window, cx))
                            .unwrap_or_default()
                    },
                )
            })
        });
        let decorate = sticky.map(|sticky| {
            Rc::new(move |list: gpui::UniformList| list.with_decoration(sticky.clone())) as Decorate
        });
        view.update(cx, |view, _| view.refresh(Some(count), decorate));
        view.clone()
    }

    /// The pinned group header of `list` while its plan is grouped. The
    /// caller has just planned the list, so the cached plan is the drawn one.
    fn sticky_group_headers(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<crate::view::rows::StickyGroupHeaders> {
        use crate::view::rows::{FileListId, FileListRow, RowIx};
        let plan = self
            .file_controllers
            .borrow()
            .get(&(repo_id, list))?
            .controller
            .borrow()
            .plan_cache
            .current()?;
        let headers = plan.headers()?;
        let prefix = match list {
            FileListId::CommitFiles => format!("commit_file_group_{}", repo_id.0),
            FileListId::WorktreeFiles => format!("worktree_file_group_{}", repo_id.0),
            FileListId::RangeFiles => format!("range_file_group_{}", repo_id.0),
            FileListId::Status(section) => {
                format!("status_group_{}_{}", repo_id.0, section.id_label())
            }
        };
        let pane = cx.weak_entity();
        let theme = self.theme;
        let ui_scale = crate::ui_scale::UiScale::current(cx);
        Some(crate::view::rows::StickyGroupHeaders {
            headers,
            header: Rc::new(move |row, row_height, _cx| {
                let FileListRow::Group {
                    group,
                    label,
                    count,
                    collapsed,
                } = plan.row_at(RowIx(row))?
                else {
                    return None;
                };
                let pane = pane.clone();
                Some(crate::view::rows::group_header_row(
                    crate::view::rows::GroupHeaderProps {
                        id: ("file_list_sticky_group", group),
                        selector: format!("{prefix}_sticky_{label}"),
                        label,
                        count,
                        collapsed,
                    },
                    theme,
                    ui_scale,
                    row_height,
                    move |cx| {
                        let _ = pane.update(cx, |pane, cx| {
                            pane.toggle_file_list_group(repo_id, list, group, cx)
                        });
                    },
                ))
            }),
        })
    }
}
