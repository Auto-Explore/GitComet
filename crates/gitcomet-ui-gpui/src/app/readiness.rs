use crate::launch_guard::UiLaunchError;
use gpui::App;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub(super) type ReadyCallback = Box<dyn FnOnce() -> Result<(), String>>;

enum ReadinessState {
    Disabled,
    Waiting(ReadyCallback),
    Complete(Result<(), String>),
}

#[derive(Clone)]
pub(super) struct StartupReadiness(Rc<RefCell<ReadinessState>>);

impl gpui::Global for StartupReadiness {}

impl StartupReadiness {
    pub(super) fn new(callback: Option<ReadyCallback>) -> Self {
        Self(Rc::new(RefCell::new(match callback {
            Some(callback) => ReadinessState::Waiting(callback),
            None => ReadinessState::Disabled,
        })))
    }

    fn pending(&self) -> bool {
        matches!(*self.0.borrow(), ReadinessState::Waiting(_))
    }

    fn notify_if_window_exists(&self, exists: bool) {
        if !exists || !self.pending() {
            return;
        }
        let state = self.0.replace(ReadinessState::Complete(Err(
            "startup readiness callback did not complete".into(),
        )));
        if let ReadinessState::Waiting(callback) = state {
            *self.0.borrow_mut() = ReadinessState::Complete(callback());
        }
    }

    pub(super) fn observe(&self, cx: &mut App) -> bool {
        self.notify_if_window_exists(!cx.windows().is_empty());
        if matches!(*self.0.borrow(), ReadinessState::Complete(Err(_))) {
            cx.quit();
        }
        !self.pending()
    }

    pub(super) fn watch(&self, cx: &mut App) {
        if self.observe(cx) {
            return;
        }
        cx.set_global(self.clone());
        let readiness = self.clone();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(25))
                    .await;
                if cx.update(|cx| readiness.observe(cx)) {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn result(&self) -> Result<(), UiLaunchError> {
        match &*self.0.borrow() {
            ReadinessState::Disabled | ReadinessState::Complete(Ok(())) => Ok(()),
            ReadinessState::Waiting(_) => Err(UiLaunchError::from_launch_failure(
                "main GPUI window launch",
                "application exited before creating its first window",
            )),
            ReadinessState::Complete(Err(message)) => Err(UiLaunchError::from_launch_failure(
                "main GPUI window launch",
                message.clone(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn readiness_requires_a_window_and_notifies_only_once() {
        let calls = Rc::new(Cell::new(0));
        let seen = calls.clone();
        let readiness = StartupReadiness::new(Some(Box::new(move || {
            seen.set(seen.get() + 1);
            Ok(())
        })));
        readiness.notify_if_window_exists(false);
        assert_eq!(calls.get(), 0);
        assert!(readiness.result().is_err());
        readiness.notify_if_window_exists(true);
        readiness.notify_if_window_exists(true);
        assert_eq!(calls.get(), 1);
        assert!(readiness.result().is_ok());
    }

    #[test]
    fn readiness_failure_is_retained_without_retrying_the_callback() {
        let readiness =
            StartupReadiness::new(Some(Box::new(|| Err("launcher disconnected".into()))));
        readiness.notify_if_window_exists(true);
        readiness.notify_if_window_exists(true);
        assert!(
            readiness
                .result()
                .unwrap_err()
                .to_string()
                .contains("launcher disconnected")
        );
    }

    #[test]
    fn unsupervised_launch_does_not_require_a_readiness_callback() {
        let readiness = StartupReadiness::new(None);
        assert!(!readiness.pending());
        assert!(readiness.result().is_ok());
    }

    #[gpui::test]
    fn extension_window_notifies_before_browser_startup_resumes(cx: &mut gpui::TestAppContext) {
        let notified = Rc::new(Cell::new(false));
        let seen = notified.clone();
        let readiness = StartupReadiness::new(Some(Box::new(move || {
            seen.set(true);
            Ok(())
        })));
        cx.update(|cx| readiness.watch(cx));
        cx.run_until_parked();
        assert!(!notified.get());
        let _window = cx.add_empty_window();
        cx.executor().advance_clock(Duration::from_millis(25));
        cx.run_until_parked();
        assert!(notified.get());
        assert!(readiness.result().is_ok());
    }
}
