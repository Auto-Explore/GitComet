//! Actions shared by hosted menus, notifications, and error reports.

use gitcomet_ui_kit::gpui::{App, SharedString};
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

/// An action owned by its presentation. Invocation is always deferred, so a
/// callback may update or replace the view that invoked it.
#[derive(Clone)]
pub struct HostedAction {
    id: u64,
    label: SharedString,
    run: Rc<dyn Fn(&mut App)>,
}

impl HostedAction {
    pub fn new(label: impl Into<SharedString>, run: impl Fn(&mut App) + 'static) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            label: label.into(),
            run: Rc::new(run),
        }
    }

    pub fn label(&self) -> &SharedString {
        &self.label
    }

    pub fn invoke(&self, cx: &mut App) {
        let run = self.run.clone();
        cx.defer(move |cx| run(cx));
    }
}

impl fmt::Debug for HostedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostedAction")
            .field("id", &self.id)
            .field("label", &self.label)
            .finish()
    }
}

impl PartialEq for HostedAction {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for HostedAction {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostedMenuItem {
    Action {
        action: HostedAction,
        icon: Option<SharedString>,
        disabled: bool,
    },
    Header(SharedString),
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationKind {
    Success,
    Warning,
    Error,
}
