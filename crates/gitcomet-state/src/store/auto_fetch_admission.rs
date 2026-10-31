use gitcomet_core::services::{CancellationToken, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const CANCELLATION_POLL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct Slot {
    foreground: usize,
    background: Option<CancellationToken>,
}

#[derive(Default)]
pub(super) struct Admission {
    slots: Mutex<HashMap<PathBuf, Slot>>,
    stopped: Condvar,
    pub(super) changed: Arc<Condvar>,
}

impl Admission {
    pub(super) fn reserve(self: &Arc<Self>, key: PathBuf) -> ForegroundReservation {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        let slot = slots.entry(key.clone()).or_default();
        slot.foreground += 1;
        if let Some(background) = &slot.background {
            background.cancel();
        }
        ForegroundReservation {
            admission: Arc::clone(self),
            key,
        }
    }

    pub(super) fn blocked(&self, key: &Path) -> bool {
        self.slots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            .is_some_and(|slot| slot.foreground > 0)
    }

    pub(super) fn try_background(
        self: &Arc<Self>,
        key: PathBuf,
        cancellation: CancellationToken,
    ) -> Option<BackgroundLease> {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        let slot = slots.entry(key.clone()).or_default();
        if slot.foreground > 0 || slot.background.is_some() {
            return None;
        }
        slot.background = Some(cancellation);
        Some(BackgroundLease {
            admission: Arc::clone(self),
            key,
        })
    }
}

pub(super) struct ForegroundReservation {
    admission: Arc<Admission>,
    key: PathBuf,
}

impl ForegroundReservation {
    pub(super) fn wait(&self, cancellation: &CancellationToken) -> Result<()> {
        let mut slots = self
            .admission
            .slots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            cancellation.check_cancelled()?;
            if slots
                .get(&self.key)
                .is_none_or(|slot| slot.background.is_none())
            {
                return Ok(());
            }
            slots = self
                .admission
                .stopped
                .wait_timeout(slots, CANCELLATION_POLL)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}

impl Drop for ForegroundReservation {
    fn drop(&mut self) {
        let mut slots = self
            .admission
            .slots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(slot) = slots.get_mut(&self.key) {
            slot.foreground -= 1;
            if slot.foreground == 0 && slot.background.is_none() {
                slots.remove(&self.key);
            }
        }
        self.admission.changed.notify_all();
    }
}

pub(super) struct BackgroundLease {
    admission: Arc<Admission>,
    key: PathBuf,
}

impl Drop for BackgroundLease {
    fn drop(&mut self) {
        let mut slots = self
            .admission
            .slots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(slot) = slots.get_mut(&self.key) {
            slot.background = None;
            if slot.foreground == 0 {
                slots.remove(&self.key);
            }
        }
        self.admission.stopped.notify_all();
        self.admission.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn foreground_cancels_background_and_waits_until_its_process_releases_the_lease() {
        let admission = Arc::new(Admission::default());
        let key = PathBuf::from("/repository/.git");
        let cancellation = CancellationToken::new();
        let background = admission
            .try_background(key.clone(), cancellation.clone())
            .unwrap();
        let foreground = admission.reserve(key.clone());
        assert!(cancellation.is_cancelled());
        assert!(
            admission
                .try_background(key.clone(), CancellationToken::new())
                .is_none()
        );
        let (entered, entering) = mpsc::channel();
        let (finished, finishing) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            entered.send(()).unwrap();
            foreground.wait(&CancellationToken::new()).unwrap();
            finished.send(foreground).unwrap();
        });
        entering.recv().unwrap();
        assert!(finishing.try_recv().is_err());
        drop(background);
        let foreground = finishing.recv().unwrap();
        assert!(
            admission
                .try_background(key.clone(), CancellationToken::new())
                .is_none()
        );
        drop(foreground);
        assert!(
            admission
                .try_background(key, CancellationToken::new())
                .is_some()
        );
        worker.join().unwrap();
    }

    #[test]
    fn foreground_reservations_preserve_foreground_concurrency_and_other_repositories() {
        let admission = Arc::new(Admission::default());
        let first = admission.reserve(PathBuf::from("/one/.git"));
        let second = admission.reserve(PathBuf::from("/one/.git"));
        first.wait(&CancellationToken::new()).unwrap();
        second.wait(&CancellationToken::new()).unwrap();
        assert!(
            admission
                .try_background(PathBuf::from("/two/.git"), CancellationToken::new())
                .is_some()
        );
        drop(first);
        assert!(admission.blocked(Path::new("/one/.git")));
        drop(second);
        assert!(!admission.blocked(Path::new("/one/.git")));
    }
}
