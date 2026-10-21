//! Main-window large-file tool discovery.
use super::*;
use gitcomet_core::large_file_tools::LargeFileToolsState;
use gitcomet_core::process::GitRuntimeState;
use gitcomet_core::services::CancellationToken;

/// Validity for deferred Settings publications, independent of worker lifetime.
#[derive(Default)]
struct SettingsToolProbeGeneration(u64);

impl gpui::Global for SettingsToolProbeGeneration {}

pub(super) fn settings_tool_probe_generation(cx: &App) -> u64 {
    cx.try_global::<SettingsToolProbeGeneration>()
        .map_or(0, |generation| generation.0)
}

pub(super) fn supersede_settings_tool_probe(cx: &mut App) -> u64 {
    let generation = settings_tool_probe_generation(cx).wrapping_add(1);
    cx.set_global(SettingsToolProbeGeneration(generation));
    generation
}

#[derive(Clone, Debug)]
pub(super) struct LargeFileToolsPublication {
    pub(super) runtime: GitRuntimeState,
    pub(super) generation: u64,
    pub(super) tools: LargeFileToolsState,
}

/// One window's request lifetime; results continue to live in AppState.
#[derive(Default)]
pub(super) struct LargeFileToolsProbeOwner {
    initial_frame_complete: bool,
    sequence: u64,
    closed: bool,
    runtime: Option<GitRuntimeState>,
    cancellation: Option<CancellationToken>,
}

impl LargeFileToolsProbeOwner {
    fn cancel(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.sequence = self.sequence.wrapping_add(1);
        self.runtime = None;
    }

    fn invalidate_for_runtime(&mut self, runtime: &GitRuntimeState) {
        if self
            .runtime
            .as_ref()
            .is_some_and(|previous| previous != runtime)
        {
            self.cancel();
        }
    }

    fn begin(&mut self, runtime: GitRuntimeState) -> Option<LargeFileToolsProbeRequest> {
        if self.closed || self.runtime.as_ref() == Some(&runtime) {
            return None;
        }
        self.cancel();
        let cancellation = CancellationToken::new();
        self.runtime = Some(runtime.clone());
        self.cancellation = Some(cancellation.clone());
        Some(LargeFileToolsProbeRequest {
            runtime,
            cancellation,
            sequence: self.sequence,
        })
    }

    fn close(&mut self) {
        self.closed = true;
        self.cancel();
    }

    fn settle(&mut self, runtime: GitRuntimeState) -> bool {
        if self.closed {
            return false;
        }
        self.cancel();
        // Keep the completed runtime while its store publication is pending:
        // an older NotChecked snapshot must not start another request.
        self.runtime = Some(runtime);
        true
    }

    fn finish(&mut self, request: &LargeFileToolsProbeRequest, runtime: &GitRuntimeState) -> bool {
        if self.sequence != request.sequence
            || self.runtime.as_ref() != Some(&request.runtime)
            || runtime != &request.runtime
            || self.cancellation.is_none()
            || request.cancellation.is_cancelled()
        {
            return false;
        }
        self.cancellation = None;
        true
    }
}

impl Drop for LargeFileToolsProbeOwner {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Clone)]
pub(super) struct LargeFileToolsProbeRequest {
    pub(super) runtime: GitRuntimeState,
    pub(super) cancellation: CancellationToken,
    sequence: u64,
}

/// Opt-in native command factory for resource-count tests. Ordinary lifecycle
/// tests keep holding requests without scheduling work.
#[cfg(test)]
pub(super) struct NativeLargeFileToolsDetectorForTest(
    pub(super) std::sync::Arc<dyn Fn() -> std::process::Command + Send + Sync>,
);

#[cfg(test)]
impl gpui::Global for NativeLargeFileToolsDetectorForTest {}

#[cfg(test)]
pub(super) struct HeldLargeFileToolsDetector {
    pub(super) runtime: GitRuntimeState,
    pub(super) requests: Vec<LargeFileToolsProbeRequest>,
}

impl GitCometView {
    pub(super) fn schedule_large_file_tools_after_initial_frame(
        &self,
        window: &mut Window,
        cx: &gpui::Context<Self>,
    ) {
        let view = cx.weak_entity();
        // GPUI draws once while opening a window without presenting it. Its
        // next-frame callbacks run before that platform frame's presentation,
        // so one callback would still compete with the first submitted frame.
        // A second callback leaves that platform presentation attempt free to
        // finish, even when startup is slow. This is not a GPU completion fence:
        // a lost device or stale atlas can still require a later presentation.
        window.on_next_frame(move |window, _cx| {
            window.on_next_frame(move |_window, cx| {
                let _ = view.update(cx, |view, cx| {
                    view.large_file_tools_probe.initial_frame_complete = true;
                    // Read the current snapshot/runtime: preferences or a
                    // Settings result may have changed while frames were pending.
                    view.refresh_large_file_tools(cx);
                });
            });
        });
    }

    pub(super) fn close_large_file_tools_probe(&mut self) {
        self.large_file_tools_probe.close();
    }

    pub(super) fn cancel_large_file_tools_probe(&mut self) {
        self.large_file_tools_probe.cancel();
    }

    pub(super) fn invalidate_large_file_tools_probe_for_runtime(
        &mut self,
        runtime: &GitRuntimeState,
    ) {
        self.large_file_tools_probe.invalidate_for_runtime(runtime);
    }

    pub(super) fn reconcile_large_file_tools_runtime(&mut self) {
        let runtime = self.large_file_tools_runtime();
        // A newer request or Settings result can arrive before its store snapshot.
        if self.state.git_runtime == runtime {
            self.large_file_tools_probe.invalidate_for_runtime(&runtime);
        }
    }

    pub(super) fn accept_large_file_tools_publication(
        &mut self,
        publication: LargeFileToolsPublication,
        cx: &App,
    ) {
        if publication.runtime != self.large_file_tools_runtime()
            || publication.generation != settings_tool_probe_generation(cx)
        {
            return;
        }
        if self.large_file_tools_probe.settle(publication.runtime) {
            self.store
                .dispatch(Msg::SetLargeFileToolsState(publication.tools));
        }
    }

    fn large_file_tools_runtime(&self) -> GitRuntimeState {
        #[cfg(test)]
        if let Some(driver) = &self.large_file_tools_detector_for_test {
            return driver.runtime.clone();
        }
        current_git_runtime()
    }

    /// Automatically probe `git lfs` / `git annex` once per Git runtime, after
    /// the initial platform frame. Settings rechecks remain immediate and can
    /// publish a fresher result without waiting for this automatic discovery.
    /// No result is persisted across processes or reused solely by Git version:
    /// rechecking Settings still discovers PATH/tool installation changes.
    pub(super) fn refresh_large_file_tools(&mut self, cx: &mut gpui::Context<Self>) {
        #[cfg(test)]
        if self.large_file_tools_detector_for_test.is_none() {
            return;
        }
        if !self.large_file_tools_probe.initial_frame_complete
            || self.state.repos.is_empty()
            || !matches!(
                self.state.large_file_tools.git_lfs,
                gitcomet_core::signing_tools::SigningToolAvailability::NotChecked
            )
        {
            return;
        }
        let runtime = self.large_file_tools_runtime();
        if self.state.git_runtime != runtime {
            return;
        }
        if !runtime.is_available() {
            self.cancel_large_file_tools_probe();
            return;
        }
        let Some(request) = self.large_file_tools_probe.begin(runtime) else {
            return;
        };
        // The test driver holds the same request and releases it through the
        // production completion method. Ordinary tests never enable discovery.
        #[cfg(test)]
        let native_factory = cx
            .try_global::<NativeLargeFileToolsDetectorForTest>()
            .map(|factory| std::sync::Arc::clone(&factory.0));
        #[cfg(test)]
        if let Some(driver) = &mut self.large_file_tools_detector_for_test {
            driver.requests.push(request.clone());
            if native_factory.is_none() {
                return;
            }
        }
        let cancellation = request.cancellation.clone();
        let runtime = request.runtime.clone();
        let detection = cx.background_spawn(async move {
            #[cfg(test)]
            if let Some(factory) = native_factory {
                return gitcomet_core::large_file_tools::detect_large_file_tools_with(
                    factory.as_ref(),
                    &cancellation,
                );
            }
            gitcomet_core::large_file_tools::detect_large_file_tools_for_runtime_cancellable(
                &runtime,
                &cancellation,
            )
        });
        cx.spawn(async move |view, cx| {
            let tools = detection.await;
            let _ = view.update(cx, |this, _cx| {
                this.complete_large_file_tools_probe(&request, tools);
            });
        })
        .detach();
    }

    pub(super) fn complete_large_file_tools_probe(
        &mut self,
        request: &LargeFileToolsProbeRequest,
        tools: LargeFileToolsState,
    ) {
        let runtime = self.large_file_tools_runtime();
        if self.large_file_tools_probe.finish(request, &runtime) {
            self.store.dispatch(Msg::SetLargeFileToolsState(tools));
        }
    }
}
