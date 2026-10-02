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
#[non_exhaustive]
pub enum HostedMenuItem {
    #[non_exhaustive]
    Action {
        action: HostedAction,
        icon: Option<SharedString>,
        disabled: bool,
    },
    Header(SharedString),
    Separator,
}

impl HostedMenuItem {
    /// An enabled entry without an icon.
    pub fn action(action: HostedAction) -> Self {
        Self::Action {
            action,
            icon: None,
            disabled: false,
        }
    }

    /// Sets an action entry's icon; other entries are unchanged.
    pub fn with_icon(mut self, path: impl Into<SharedString>) -> Self {
        if let Self::Action { icon, .. } = &mut self {
            *icon = Some(path.into());
        }
        self
    }

    /// Disables an action entry; other entries are unchanged.
    pub fn disabled(mut self, value: bool) -> Self {
        if let Self::Action { disabled, .. } = &mut self {
            *disabled = value;
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotificationKind {
    Success,
    Warning,
    Error,
}
