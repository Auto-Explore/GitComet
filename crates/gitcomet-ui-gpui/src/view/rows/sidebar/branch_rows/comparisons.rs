use super::*;
use gitcomet_core::ref_comparison::{RefComparisonCounts, RefIntegration};

impl SidebarPaneView {
    pub(super) fn comparison_heading(&self, label: &str) -> (SharedString, SharedString) {
        let Some(repo) = self.active_repo() else {
            return (label.to_owned().into(), label.to_owned().into());
        };
        match &repo.ref_comparisons.target {
            Loadable::Ready(target) => (
                format!("{label} · {}", target.reference).into(),
                format!("Comparing with locally available {} ({}). Fetch updates local remote refs. Right-click to choose a comparison ref or run a deeper check.", target.reference, target.oid).into(),
            ),
            Loadable::Error(error) => (format!("{label} · comparison unavailable").into(), error.clone().into()),
            Loadable::Loading => (label.to_owned().into(), "Resolving the locally available comparison target…".into()),
            Loadable::NotLoaded => (label.to_owned().into(), label.to_owned().into()),
        }
    }

    pub(super) fn comparison_badge(
        &self,
        ctx: &SidebarRowContext,
        subject: Option<&CommitId>,
    ) -> Option<AnyElement> {
        let repo = self.active_repo()?;
        let subject = subject?;
        let Loadable::Ready(target) = &repo.ref_comparisons.target else {
            return None;
        };
        let (label, detail) = match repo.ref_comparisons.decorations.get(subject) {
            None => ("Comparing…".to_string(), "Comparison pending.".to_string()),
            Some(result) => {
                let (counts, count_detail) = match result.counts {
                    RefComparisonCounts::Exact { ahead, behind } => (
                        format!("↑{ahead} ↓{behind}"),
                        format!(
                            "{ahead} ahead, {behind} behind by ancestry. These counts can remain nonzero after a squash or rebase patch match."
                        ),
                    ),
                    RefComparisonCounts::Pending => (String::new(), "Counts pending.".to_string()),
                    RefComparisonCounts::Unavailable(reason) => (
                        String::new(),
                        format!("Exact counts unavailable. {}", reason.description()),
                    ),
                };
                let (status, evidence) = match result.integration {
                    RefIntegration::Pending => ("Checking…", "Integration evidence pending."),
                    RefIntegration::Confirmed(evidence) => {
                        (evidence.label(), evidence.description())
                    }
                    RefIntegration::NotConfirmed(reason) => ("Not confirmed", reason.description()),
                };
                let label = if counts.is_empty() {
                    status.to_string()
                } else {
                    format!("{counts} · {status}")
                };
                (label, format!("{count_detail}\n{evidence}"))
            }
        };
        let tooltip = format!(
            "Compared with locally available {} ({}).\nSubject: {}\n{}\nUncommitted files are separate from committed-history evidence.",
            target.reference, target.oid, subject, detail
        );
        Some(
            div()
                .id(SharedString::from(format!("ref_comparison_{subject}")))
                .flex_none()
                .max_w_1_2()
                .min_w(px(0.0))
                .overflow_hidden()
                .text_size(ctx.theme.ui_text(11.0))
                .text_color(ctx.theme.colors.foreground.secondary)
                .whitespace_nowrap()
                .text_ellipsis()
                .child(label)
                .gitcomet_tooltip(ctx.theme, tooltip.into())
                .into_any_element(),
        )
    }
}
