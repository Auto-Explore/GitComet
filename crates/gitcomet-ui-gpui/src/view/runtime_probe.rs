//! Shared Git discovery. All process creation and waiting happens on workers.
use super::*;
use gitcomet_core::process::{begin_git_runtime_probe, current_git_runtime};

pub(super) fn request(cx: &mut gpui::App, force: bool) {
    if cfg!(test) {
        return;
    }
    let Some(probe) = begin_git_runtime_probe(force) else {
        return;
    };
    let work = cx.background_executor().spawn(async move { probe.run() });
    cx.spawn(async move |cx| {
        let Some(runtime) = work.await else {
            return;
        };
        cx.update(|cx| {
            // A path may have changed between worker publication and this UI turn.
            if current_git_runtime() != runtime {
                return;
            }
            for handle in cx.windows() {
                if let Some(handle) = handle.downcast::<GitCometView>() {
                    let _ = handle.update(cx, |view, _, cx| {
                        view.store
                            .dispatch(Msg::SetGitRuntimeState(runtime.clone()));
                        view.refresh_signing_tools(true, cx);
                    });
                } else if let Some(handle) =
                    handle.downcast::<settings_window::SettingsWindowView>()
                {
                    let _ = handle.update(cx, |view, _, cx| {
                        view.apply_probed_runtime(runtime.clone(), cx);
                    });
                }
            }
        });
    })
    .detach();
}
