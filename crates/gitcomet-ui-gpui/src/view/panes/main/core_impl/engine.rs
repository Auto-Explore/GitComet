use super::*;

impl MainPaneView {
    pub(in crate::view) fn effective_diff_engine(&self) -> DiffEngine {
        self.diff_engine_override
            .unwrap_or(self.diff_engine_default)
    }

    pub(in crate::view) fn structural_diff_active(&self) -> bool {
        matches!(self.diff_engine_status, DiffEngineStatus::Difftastic { .. })
    }

    pub(in crate::view) fn set_diff_engine_override(
        &mut self,
        engine: Option<DiffEngine>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_engine_override == engine {
            return;
        }
        self.diff_engine_override = engine;
        self.refresh_diff_engine(cx);
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
