//! Window-owned extension state and explicit invalidation.

use crate::{RepositoryHandle, WindowHost};
use gitcomet_ui_kit::gpui::App;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The shell surfaces whose descriptor providers may change.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Slot {
    Gate,
    RepositoryView,
    Navigation,
    ActionBar,
    Status,
    TitleBar,
    Settings,
    History,
    Sidebar,
    Details,
    BottomPanel,
}

impl Slot {
    pub const ALL: &'static [Self] = &[
        Self::Gate,
        Self::RepositoryView,
        Self::Navigation,
        Self::ActionBar,
        Self::Status,
        Self::TitleBar,
        Self::Settings,
        Self::History,
        Self::Sidebar,
        Self::Details,
        Self::BottomPanel,
    ];

    pub const fn mask(self) -> u64 {
        1 << self as u8
    }
}

/// O(1) revision for a descriptor provider. Bump after replacing its data.
#[derive(Clone, Default)]
pub struct SlotSignal(Arc<AtomicU64>);

impl SlotSignal {
    pub fn revision(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
    pub fn bump(&self) {
        self.0.fetch_add(1, Ordering::Release);
    }
}

struct NotifyState {
    pending: AtomicU64,
    closed: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}

/// A Send + Sync handle for background work to invalidate shell slots.
/// Repeated notifications coalesce until the UI consumes them; no polling.
#[derive(Clone)]
pub struct HostNotifier(Arc<NotifyState>);

impl HostNotifier {
    #[doc(hidden)]
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(NotifyState {
            pending: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            wake: Box::new(wake),
        }))
    }

    pub fn notify(&self, slot: Slot) {
        if !self.0.closed.load(Ordering::Acquire)
            && self.0.pending.fetch_or(slot.mask(), Ordering::AcqRel) == 0
        {
            (self.0.wake)();
        }
    }

    #[doc(hidden)]
    pub fn take_pending(&self) -> u64 {
        self.0.pending.swap(0, Ordering::AcqRel)
    }

    #[doc(hidden)]
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::Release);
    }
}

/// An event delivered after the host's update, never while it mutates state.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ShellEvent {
    RepositoryOpened(RepositoryHandle),
    RepositoryClosed(RepositoryHandle),
    ActiveRepositoryChanged(Option<RepositoryHandle>),
    StateChanged,
    ThemeChanged,
    ViewChanged,
    WindowClosed,
}

/// One extension's state in one window. The host drops it with the root view.
pub trait WindowExtension: 'static {
    fn on_event(&mut self, _event: &ShellEvent, _host: &WindowHost, _cx: &mut App) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_invalidations_coalesce_and_closed_hosts_stop_waking() {
        let wakes = Arc::new(AtomicU64::new(0));
        let count = wakes.clone();
        let notifier = HostNotifier::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        });
        let worker = notifier.clone();
        std::thread::spawn(move || {
            for _ in 0..100 {
                worker.notify(Slot::Sidebar);
                worker.notify(Slot::History);
            }
        })
        .join()
        .unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert_eq!(
            notifier.take_pending(),
            Slot::Sidebar.mask() | Slot::History.mask()
        );
        notifier.notify(Slot::Status);
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        notifier.close();
        notifier.take_pending();
        notifier.notify(Slot::Gate);
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
    }
}
