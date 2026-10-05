use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::atomic::{AtomicBool, Ordering},
    thread::{ThreadId, current},
    time::Duration,
};

use crate::bindings::Windows::Win32::{
    CloseThreadpoolTimer, CreateThreadpoolTimer, FILETIME, GetCurrentThread, LPARAM,
    PTP_CALLBACK_INSTANCE, PTP_TIMER, PostMessageW, SetThreadPriority, SetThreadpoolTimer,
    THREAD_PRIORITY_TIME_CRITICAL, TP_CALLBACK_ENVIRON_V3, TP_CALLBACK_PRIORITY,
    TP_CALLBACK_PRIORITY_HIGH, TP_CALLBACK_PRIORITY_LOW, TP_CALLBACK_PRIORITY_NORMAL,
    TrySubmitThreadpoolCallback, WPARAM, timeBeginPeriod, timeEndPeriod,
};
use anyhow::Context;
use gpui_util::ResultExt;

use crate::{HWND, SafeHwnd, WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD};
use gpui::{
    PlatformDispatcher, Priority, PriorityQueueReceiver, PriorityQueueSender, RunnableVariant,
    TimerResolutionGuard,
};

/// How long the main thread runs queued foreground tasks before it goes back
/// to painting and input, both in the ordinary message loop and in native
/// modal loops.
pub(crate) const MAIN_TASK_BUDGET: Duration = Duration::from_millis(10);

pub(crate) struct WindowsDispatcher {
    pub(crate) wake_posted: AtomicBool,
    main_sender: PriorityQueueSender<RunnableVariant>,
    main_thread_id: ThreadId,
    pub(crate) platform_window_handle: SafeHwnd,
    validation_number: usize,
}

impl WindowsDispatcher {
    pub(crate) fn new(
        main_sender: PriorityQueueSender<RunnableVariant>,
        platform_window_handle: HWND,
        validation_number: usize,
    ) -> Self {
        let main_thread_id = current().id();
        let platform_window_handle = platform_window_handle.into();

        WindowsDispatcher {
            main_sender,
            main_thread_id,
            platform_window_handle,
            validation_number,
            wake_posted: AtomicBool::new(false),
        }
    }

    fn dispatch_on_threadpool(&self, priority: TP_CALLBACK_PRIORITY, runnable: RunnableVariant) {
        let environ = TP_CALLBACK_ENVIRON_V3 {
            Version: crate::bindings::Windows::Win32::TP_VERSION(3),
            CallbackPriority: priority,
            Size: size_of::<TP_CALLBACK_ENVIRON_V3>() as u32,
            ..Default::default()
        };

        // If the thread pool never runs our callback, the matching `from_raw` is never called, which leaks the runnable.
        // Dropping the scheduled runnable would cancel its task and make the next poll of any awaiter panic. Since we expect
        // the scenario to usually happen during shutdown, this leak is acceptable.
        let context = runnable.into_raw().as_ptr() as *mut c_void;

        unsafe {
            TrySubmitThreadpoolCallback(Some(run_work_callback), Some(context), Some(&environ))
                .ok()
                .log_err();
        }
    }

    fn dispatch_on_threadpool_after(&self, runnable: RunnableVariant, duration: Duration) {
        let context = runnable.into_raw().as_ptr() as *mut c_void;

        unsafe {
            let timer = CreateThreadpoolTimer(Some(run_timer_callback), Some(context), None);
            if !timer.is_null() {
                // Negative FILETIME expresses a relative delay in 100ns ticks
                let ticks = (duration.as_nanos() / 100).min(i64::MAX as u128) as i64;
                let due = (-ticks) as u64;
                let due_time = FILETIME {
                    dwLowDateTime: due as u32,
                    dwHighDateTime: (due >> 32) as u32,
                };
                SetThreadpoolTimer(timer, Some(&due_time), 0, None);
            }
        }
    }

    #[inline(always)]
    pub(crate) fn execute_runnable(runnable: RunnableVariant) {
        let location = runnable.metadata().location;
        let spawned = runnable.metadata().spawned;
        gpui::profiler::update_running_task(spawned, location);
        runnable.run();
        gpui::profiler::save_task_timing();
    }

    /// Runs queued foreground tasks from inside a native modal loop (window
    /// drag or resize, menu tracking) until the queue is empty or
    /// [`MAIN_TASK_BUDGET`] is spent. At least one task runs if any are queued.
    ///
    /// Tasks left over stay queued for the modal loop's next timer tick, or
    /// for the ordinary loop once the modal loop exits. `wake_posted` is left
    /// alone: only the ordinary loop's `run_foreground_task` clears it.
    pub(crate) fn drain_modal_tasks(receiver: &mut PriorityQueueReceiver<RunnableVariant>) {
        let start = std::time::Instant::now();
        while start.elapsed() < MAIN_TASK_BUDGET {
            let Ok(Some(runnable)) = receiver.try_pop() else {
                break;
            };
            Self::execute_runnable(runnable);
        }
    }
}

impl PlatformDispatcher for WindowsDispatcher {
    fn is_main_thread(&self) -> bool {
        current().id() == self.main_thread_id
    }

    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        let priority = match priority {
            Priority::RealtimeAudio => {
                panic!("RealtimeAudio priority should use spawn_realtime, not dispatch")
            }
            Priority::High => TP_CALLBACK_PRIORITY_HIGH,
            Priority::Medium => TP_CALLBACK_PRIORITY_NORMAL,
            Priority::Low => TP_CALLBACK_PRIORITY_LOW,
        };
        self.dispatch_on_threadpool(priority, runnable);
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority) {
        match self.main_sender.send(priority, runnable) {
            Ok(_) => {
                if !self.wake_posted.swap(true, Ordering::AcqRel) {
                    unsafe {
                        PostMessageW(
                            Some(self.platform_window_handle.as_raw()),
                            WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD as u32,
                            WPARAM(self.validation_number),
                            LPARAM(0),
                        )
                        .ok()
                        .log_err();
                    }
                }
            }
            Err(runnable) => {
                // NOTE: Runnable may wrap a Future that is !Send.
                //
                // This is usually safe because we only poll it on the main thread.
                // However if the send fails, we know that:
                // 1. main_receiver has been dropped (which implies the app is shutting down)
                // 2. we are on a background thread.
                // It is not safe to drop something !Send on the wrong thread, and
                // the app will exit soon anyway, so we must forget the runnable.
                std::mem::forget(runnable);
            }
        }
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        self.dispatch_on_threadpool_after(runnable, duration);
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        std::thread::spawn(move || {
            // SAFETY: always safe to call
            let thread_handle = unsafe { GetCurrentThread() };

            // SAFETY: thread_handle is a valid handle to the current thread
            unsafe { SetThreadPriority(thread_handle, THREAD_PRIORITY_TIME_CRITICAL).ok() }
                .context("thread priority")
                .log_err();

            f();
        });
    }

    fn increase_timer_resolution(&self) -> TimerResolutionGuard {
        unsafe {
            timeBeginPeriod(1);
        }
        gpui_util::defer(Box::new(|| unsafe {
            timeEndPeriod(1);
        }))
    }
}

unsafe extern "system" fn run_work_callback(
    _instance: PTP_CALLBACK_INSTANCE,
    context: *mut c_void,
) {
    let runnable = unsafe { RunnableVariant::from_raw(NonNull::new_unchecked(context as *mut ())) };
    WindowsDispatcher::execute_runnable(runnable);
}

unsafe extern "system" fn run_timer_callback(
    _instance: PTP_CALLBACK_INSTANCE,
    context: *mut c_void,
    timer: PTP_TIMER,
) {
    let runnable = unsafe { RunnableVariant::from_raw(NonNull::new_unchecked(context as *mut ())) };
    WindowsDispatcher::execute_runnable(runnable);
    unsafe { CloseThreadpoolTimer(timer) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::Windows::Win32::{MSG, PeekMessageW};
    use std::{
        cell::Cell,
        pin::Pin,
        rc::Rc,
        sync::Arc,
        task::{self, Poll},
    };

    /// A main-thread queue whose wakes post to this test thread's message queue.
    struct MainThread {
        dispatcher: Arc<WindowsDispatcher>,
        executor: gpui::ForegroundExecutor,
        receiver: PriorityQueueReceiver<RunnableVariant>,
    }

    impl MainThread {
        fn new() -> Self {
            let (sender, receiver) = PriorityQueueReceiver::new();
            let dispatcher = Arc::new(WindowsDispatcher::new(sender, HWND::default(), 0));
            let executor = gpui::ForegroundExecutor::new(dispatcher.clone());
            Self {
                dispatcher,
                executor,
                receiver,
            }
        }

        fn spawn_counted(&self, completed: &Rc<Cell<usize>>, work: Duration) {
            let completed = completed.clone();
            self.executor
                .spawn(async move {
                    std::thread::sleep(work);
                    completed.set(completed.get() + 1);
                })
                .detach();
        }

        fn drain_modal_tasks(&mut self) {
            WindowsDispatcher::drain_modal_tasks(&mut self.receiver);
        }

        /// Whether the ordinary message loop has a wake waiting to run tasks.
        fn wake_pending(&self) -> bool {
            let mut msg = MSG::default();
            let wake = WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD as u32;
            // A flag of 0 is PM_NOREMOVE.
            unsafe { PeekMessageW(&mut msg, None, wake, wake, 0) }.as_bool()
        }
    }

    /// Wakes itself on every poll, so it is always queued again.
    struct Reschedules(Rc<Cell<usize>>);

    impl Future for Reschedules {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<()> {
            self.0.set(self.0.get() + 1);
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    #[test]
    fn modal_drain_runs_every_task_that_fits_in_the_budget() {
        let mut main = MainThread::new();
        let completed = Rc::new(Cell::new(0));
        for _ in 0..100 {
            main.spawn_counted(&completed, Duration::ZERO);
        }
        main.drain_modal_tasks();
        assert_eq!(completed.get(), 100);
        assert!(main.receiver.is_empty());
    }

    #[test]
    fn modal_drain_leaves_tasks_past_the_budget_for_later() {
        let mut main = MainThread::new();
        let completed = Rc::new(Cell::new(0));
        // Each task alone overruns the budget, so each drain runs one.
        for _ in 0..2 {
            main.spawn_counted(&completed, MAIN_TASK_BUDGET + Duration::from_millis(1));
        }
        main.drain_modal_tasks();
        assert_eq!(completed.get(), 1);
        assert!(!main.receiver.is_empty());
        // The leftover task still runs if the modal loop exits before its next
        // tick: the ordinary loop's wake stays posted, and stays claimed so no
        // duplicate is posted.
        assert!(main.wake_pending());
        assert!(main.dispatcher.wake_posted.load(Ordering::Acquire));

        main.drain_modal_tasks();
        assert_eq!(completed.get(), 2);
        assert!(main.receiver.is_empty());
    }

    #[test]
    fn modal_drain_returns_while_a_task_keeps_rescheduling_itself() {
        // Draining until the queue emptied never returned here, which froze
        // window drags and resizes.
        let mut main = MainThread::new();
        let polls = Rc::new(Cell::new(0));
        main.executor.spawn(Reschedules(polls.clone())).detach();
        main.drain_modal_tasks();
        assert!(
            polls.get() > 1,
            "the task must keep running within the budget"
        );
        assert!(!main.receiver.is_empty());
    }
}
