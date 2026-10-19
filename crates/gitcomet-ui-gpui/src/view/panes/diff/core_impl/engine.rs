use crate::view::DiffWhitespaceMode;
use crate::view::panes::diff::DiffRenderer;
use crate::view::panes::diff::helpers::state::DiffEngineOverride;
use gitcomet_core::diff_engine::{DiffEngine, DiffEngineFallback, DiffEngineStatus};
use gpui::SharedString;

impl DiffRenderer {
    pub(in crate::view) fn effective_diff_engine(&self) -> DiffEngine {
        self.diff_engine_override()
            .unwrap_or(self.diff_engine_default)
    }

    pub(in crate::view) fn diff_engine_override(&self) -> Option<DiffEngine> {
        self.diff_engine_override.map(|chosen| chosen.engine)
    }

    pub(in crate::view) fn structural_diff_active(&self) -> bool {
        matches!(self.diff_engine_status, DiffEngineStatus::Difftastic { .. })
    }

    pub(in crate::view) fn set_diff_engine_override(
        &mut self,
        engine: Option<DiffEngine>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_engine_override() == engine {
            // Chosen again: it outlives a close still on its way.
            if let Some(chosen) = &mut self.diff_engine_override {
                chosen.ends_with_target = false;
            }
            return;
        }
        self.end_diff_engine_override(cx);
        self.diff_engine_override = engine.map(DiffEngineOverride::new);
        self.refresh_diff_engine(cx);
    }

    /// "Show line changes": the line engine with whitespace shown until its
    /// override ends.
    pub(in crate::view) fn show_line_changes(&mut self, cx: &mut gpui::Context<Self>) {
        self.end_diff_engine_override(cx);
        self.diff_engine_override = Some(DiffEngineOverride {
            restores_whitespace: Some(self.diff_whitespace_mode),
            ..DiffEngineOverride::new(DiffEngine::LineByLine)
        });
        self.set_diff_whitespace_mode(DiffWhitespaceMode::Show, cx);
        self.refresh_diff_engine(cx);
    }

    /// A whitespace mode the user picked, which an ending override must not
    /// undo.
    pub(in crate::view) fn choose_diff_whitespace_mode(
        &mut self,
        next: DiffWhitespaceMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(chosen) = &mut self.diff_engine_override {
            chosen.restores_whitespace = None;
        }
        self.set_diff_whitespace_mode(next, cx);
    }

    /// Drops the override, restoring the whitespace mode it replaced; the
    /// caller rebuilds.
    fn end_diff_engine_override(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(mode) = self
            .diff_engine_override
            .take()
            .and_then(|chosen| chosen.restores_whitespace)
        {
            self.set_diff_whitespace_mode(mode, cx);
        }
    }

    /// Closing the viewer: its override ends with the target, in
    /// [`Self::end_override_of_closed_viewer`].
    pub(in crate::view) fn end_diff_engine_override_with_target(&mut self) {
        if let Some(chosen) = &mut self.diff_engine_override {
            chosen.ends_with_target = true;
        }
    }

    /// The rendered target changed. Nothing is left to rebuild with the closed
    /// viewer's engine, so its override ends without a refresh.
    pub(in crate::view) fn end_override_of_closed_viewer(&mut self, cx: &mut gpui::Context<Self>) {
        if self
            .diff_engine_override
            .is_some_and(|chosen| chosen.ends_with_target)
        {
            self.end_diff_engine_override(cx);
        }
    }

    pub(in crate::view) fn set_diff_engine_default(
        &mut self,
        engine: DiffEngine,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_engine_default == engine {
            return;
        }
        self.diff_engine_default = engine;
        if self.diff_engine_override.is_none() {
            self.refresh_diff_engine(cx);
        }
    }

    fn refresh_diff_engine(&mut self, cx: &mut gpui::Context<Self>) {
        self.diff_engine_anchor = self.hosted_scroll_anchor();
        self.file_diff_cancellation.cancel();
        self.file_diff_cache_rev = 0;
        self.reset_file_diff_cache_data();
        self.clear_diff_text_selection_span();
        self.clear_diff_text_style_caches();
        self.ensure_file_diff_cache(cx);
        self.diff_search_recompute_matches();
        cx.notify();
    }

    pub(in crate::view) fn diff_engine_label(&self) -> SharedString {
        if self.effective_diff_engine() == DiffEngine::LineByLine {
            return "Line by line".into();
        }
        match self.diff_engine_status {
            DiffEngineStatus::Difftastic {
                syntactic_changes: false,
                text_differs: true,
            } => "Difftastic · No syntactic changes; text differs".into(),
            DiffEngineStatus::Difftastic { .. } => "Difftastic".into(),
            DiffEngineStatus::Fallback(reason) => {
                format!("Line by line · {}", reason.label()).into()
            }
            DiffEngineStatus::LineByLine if self.file_diff_cache_inflight.is_some() => {
                "Difftastic · Comparing…".into()
            }
            DiffEngineStatus::LineByLine => format!(
                "Line by line · {}",
                DiffEngineFallback::MissingSources.label()
            )
            .into(),
        }
    }
}
