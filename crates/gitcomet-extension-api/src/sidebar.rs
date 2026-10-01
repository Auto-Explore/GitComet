//! Revisioned rows and file sets for the repository sidebar.
use crate::{HostedAction, RepositoryViewContext, RowMark, SlotSignal};
use gitcomet_ui_kit::gpui::{App, SharedString};
use std::{path::PathBuf, rc::Rc, sync::Arc};

#[derive(Clone)]
pub struct SidebarRow {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Option<SharedString>,
    pub mark: Option<RowMark>,
    pub action: HostedAction,
}

pub type SidebarFileOpen = Rc<dyn Fn(&PathBuf, &mut App)>;
pub type SidebarSectionsProvider =
    Rc<dyn Fn(&RepositoryViewContext, &App) -> Vec<SidebarSectionRows>>;

#[derive(Clone)]
pub struct SidebarFileSet {
    pub paths: Arc<[PathBuf]>,
    pub open: SidebarFileOpen,
}

#[derive(Clone)]
pub struct SidebarSectionRows {
    pub id: SharedString,
    pub title: SharedString,
    pub rows: Vec<SidebarRow>,
    pub files: Option<SidebarFileSet>,
}

/// Invoked when this provider's revision or the repository lifetime changes.
#[derive(Clone)]
pub struct SidebarProvider {
    pub signal: SlotSignal,
    pub sections: SidebarSectionsProvider,
}

impl SidebarFileSet {
    /// File rows use the same action contract as other sidebar contributions.
    pub fn rows(&self) -> impl Iterator<Item = SidebarRow> + '_ {
        self.paths.iter().map(|path| {
            let label: SharedString = path.to_string_lossy().into_owned().into();
            let path = path.clone();
            let open = self.open.clone();
            SidebarRow {
                id: label.clone(),
                label,
                icon: Some("icons/file.svg".into()),
                mark: None,
                action: HostedAction::new("Open file", move |cx| open(&path, cx)),
            }
        })
    }
}
