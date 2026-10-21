use super::DiffRenderer;
use crate::view::documents::buffers::DocumentOwner;
use crate::view::tooltip_host::TooltipHost;
use crate::view::{
    ChangeTrackingView, DiffContentMode, DiffViewMode, DiffWhitespaceMode, ErrorReport,
    PopoverRequest, StatusSection, components,
};
use gitcomet_core::domain::{DiffArea, DiffTarget};
use gitcomet_state::model::RepoId;
use gpui::{AnyWindowHandle, App, Bounds, Entity, Pixels, Point, SharedString, WeakEntity};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

pub(in crate::view) enum DiffPreference {
    Content(DiffContentMode),
    Whitespace(DiffWhitespaceMode),
    RevealWhitespace(bool),
    Wrap(bool),
    LineNumbers(bool),
    Layout(DiffViewMode),
    Annotate(bool),
}

pub(in crate::view) enum DiffPopoverAnchor {
    Point(Point<Pixels>),
    Bounds(Bounds<Pixels>),
}

pub(in crate::view) enum DiffCommand {
    PersistSettings,
    Preference(DiffPreference),
    Error(ErrorReport),
    Toast(components::ToastKind, SharedString),
    OpenExternal(PathBuf),
    OpenSubmodule {
        path: PathBuf,
        target: DiffTarget,
    },
    ClearStatusSelection {
        repo: RepoId,
        multi_only: bool,
    },
    RevealStatus {
        section: StatusSection,
        path: PathBuf,
    },
    RevealCommitFile {
        position: usize,
        range: bool,
    },
    OpenFileHistory {
        repo: RepoId,
        path: PathBuf,
    },
    OpenPopover {
        request: Box<PopoverRequest>,
        anchor: DiffPopoverAnchor,
        pane: Option<Entity<DiffRenderer>>,
        window: AnyWindowHandle,
    },
    OpenHostedMenu {
        id: u64,
        items: Vec<gitcomet_extension_api::HostedMenuItem>,
        anchor: Point<Pixels>,
        window: AnyWindowHandle,
    },
}

pub(in crate::view) trait DiffNavigation {
    fn layout(&self, cx: &App) -> Option<(Pixels, Pixels, bool, bool)>;
    fn selected_paths(
        &self,
        repo: RepoId,
        area: DiffArea,
        path: &Path,
        cx: &App,
    ) -> Option<Vec<PathBuf>>;
    fn selected_single(&self, repo: RepoId, area: DiffArea, path: &Path, cx: &App) -> bool;
    fn change_tracking(&self, cx: &App) -> ChangeTrackingView;
    /// Outer `None` means the list owner is unavailable, so the renderer can
    /// fall back to source order. `Some(None)` is a resolved missing file.
    fn status_position(
        &self,
        _repo: &gitcomet_state::model::RepoState,
        _path: &Path,
        _area: DiffArea,
        _view: ChangeTrackingView,
        _cx: &App,
    ) -> Option<Option<(StatusSection, usize, usize)>> {
        None
    }
    fn status_order(&self, repo: RepoId, section: StatusSection, cx: &App) -> Option<Arc<[usize]>>;
    fn file_order(&self, repo: RepoId, range: bool, cx: &App) -> Option<Arc<[usize]>>;
    fn worktree_order(&self, repo: RepoId, path: &Path, cx: &App) -> Option<Arc<[usize]>>;
}

type DiffCommandDispatch = dyn Fn(DiffCommand, &mut App);

#[derive(Clone, Default)]
pub(in crate::view) struct DiffCommands {
    pub dispatch: Option<Rc<DiffCommandDispatch>>,
    pub navigation: Option<Rc<dyn DiffNavigation>>,
}

impl DiffCommands {
    pub fn send(&self, command: DiffCommand, cx: &mut App) {
        if let Some(dispatch) = &self.dispatch {
            dispatch(command, cx);
        }
    }
    pub fn defer(&self, command: DiffCommand, cx: &mut App) {
        if let Some(dispatch) = self.dispatch.clone() {
            cx.defer(move |cx| dispatch(command, cx));
        }
    }
}

#[derive(Clone)]
pub(in crate::view) struct DiffServices {
    pub store: crate::view::pane_store::PaneStore,
    pub commands: DiffCommands,
    pub documents: DocumentOwner,
    pub tooltip: WeakEntity<TooltipHost>,
}
