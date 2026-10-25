//! An optional first-run workflow before loading the saved workspace.
use gitcomet_ui_kit::gpui::App;
use std::rc::Rc;

/// A linear continuation: resume it after the user finishes the workflow.
/// Dropping it cancels startup, without reading or overwriting saved state.
#[must_use]
pub struct StartupContinuation(Box<dyn FnOnce(&mut App)>);
impl StartupContinuation {
    #[doc(hidden)]
    pub fn new(resume: impl FnOnce(&mut App) + 'static) -> Self {
        Self(Box::new(resume))
    }
    pub fn resume(self, cx: &mut App) {
        (self.0)(cx);
    }
}
pub type StartupWorkflow = Rc<dyn Fn(StartupContinuation, &mut App)>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[gpui::test]
    fn only_resuming_the_continuation_runs_startup(cx: &mut gpui::TestAppContext) {
        let calls = Rc::new(Cell::new(0));
        let deferred = calls.clone();
        let continuation = StartupContinuation::new(move |_| deferred.set(deferred.get() + 1));
        assert_eq!(calls.get(), 0);
        cx.update(|cx| continuation.resume(cx));
        assert_eq!(calls.get(), 1);
        let cancelled = calls.clone();
        drop(StartupContinuation::new(move |_| cancelled.set(99)));
        assert_eq!(
            calls.get(),
            1,
            "closing the workflow must not start restoration"
        );
    }
}
