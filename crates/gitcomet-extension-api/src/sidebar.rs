//! Revisioned rows and file sets for the repository sidebar.
use crate::{HostedAction, RepositoryViewContext, RowMark, SlotSignal};
use gitcomet_ui_kit::gpui::{App, SharedString};
use std::{path::PathBuf, rc::Rc, sync::Arc};

#[derive(Clone)]
#[non_exhaustive]
pub struct SidebarRow {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Option<SharedString>,
    pub mark: Option<RowMark>,
    pub action: HostedAction,
}

impl SidebarRow {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: HostedAction,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            mark: None,
            action,
        }
    }

    pub fn with_icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_mark(mut self, mark: RowMark) -> Self {
        self.mark = Some(mark);
        self
    }
}

pub type SidebarFileOpen = Rc<dyn Fn(&PathBuf, &mut App)>;
pub type SidebarSectionsProvider =
    Rc<dyn Fn(&RepositoryViewContext, &App) -> Vec<SidebarSectionRows>>;

#[derive(Clone)]
#[non_exhaustive]
pub struct SidebarFileSet {
    pub paths: Arc<[PathBuf]>,
    pub open: SidebarFileOpen,
}

#[derive(Clone)]
#[non_exhaustive]
pub struct SidebarSectionRows {
    pub id: SharedString,
    pub title: SharedString,
    pub rows: Vec<SidebarRow>,
    pub files: Option<SidebarFileSet>,
}

impl SidebarSectionRows {
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        rows: Vec<SidebarRow>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            rows,
            files: None,
        }
    }

    pub fn with_files(mut self, files: SidebarFileSet) -> Self {
        self.files = Some(files);
        self
    }
}

/// Invoked when this provider's revision or the repository lifetime changes.
#[derive(Clone)]
#[non_exhaustive]
pub struct SidebarProvider {
    pub signal: SlotSignal,
    pub sections: SidebarSectionsProvider,
}

impl SidebarProvider {
    pub fn new(
        signal: SlotSignal,
        sections: impl Fn(&RepositoryViewContext, &App) -> Vec<SidebarSectionRows> + 'static,
    ) -> Self {
        Self {
            signal,
            sections: Rc::new(sections),
        }
    }
}

impl SidebarFileSet {
    pub fn new(
        paths: impl Into<Arc<[PathBuf]>>,
        open: impl Fn(&PathBuf, &mut App) + 'static,
    ) -> Self {
        Self {
            paths: paths.into(),
            open: Rc::new(open),
        }
    }

    /// File rows use the same action contract as other sidebar contributions.
    pub fn rows(&self) -> impl Iterator<Item = SidebarRow> + '_ {
        self.paths.iter().map(|path| {
            let label: SharedString = path.to_string_lossy().into_owned().into();
            let path = path.clone();
            let open = self.open.clone();
            SidebarRow::new(
                label.clone(),
                label,
                HostedAction::new("Open file", move |cx| open(&path, cx)),
            )
            .with_icon("icons/file.svg")
        })
    }
}
