use super::*;

pub(crate) const NANOS_PER_MICROSECOND: f64 = 1_000.0;
pub(crate) const NANOS_PER_MILLISECOND: f64 = 1_000_000.0;
pub(crate) const SIDECAR_TIMING_MS_PREFIX: &str = "@sidecar_ms:";
pub(crate) const REQUIRED_APP_LAUNCH_ALLOCATION_METRICS: &[&str] = &[
    "first_paint_alloc_ops",
    "first_paint_alloc_bytes",
    "first_interactive_alloc_ops",
    "first_interactive_alloc_bytes",
];
pub(crate) const LARGE_HTML_BACKGROUND_PREPARE_BUDGET_NS: f64 = 225.0 * NANOS_PER_MILLISECOND;
pub(crate) const LARGE_HTML_VISIBLE_WINDOW_PENDING_BUDGET_NS: f64 = 150.0 * NANOS_PER_MICROSECOND;
pub(crate) const LARGE_HTML_VISIBLE_WINDOW_STEADY_BUDGET_NS: f64 = 125.0 * NANOS_PER_MICROSECOND;
pub(crate) const LARGE_HTML_VISIBLE_WINDOW_SWEEP_BUDGET_NS: f64 = 150.0 * NANOS_PER_MICROSECOND;
pub(crate) const EXTERNAL_HTML_BACKGROUND_PREPARE_BUDGET_NS: f64 = 1500.0 * NANOS_PER_MILLISECOND;
pub(crate) const EXTERNAL_HTML_VISIBLE_WINDOW_PENDING_BUDGET_NS: f64 =
    150.0 * NANOS_PER_MICROSECOND;
pub(crate) const EXTERNAL_HTML_VISIBLE_WINDOW_STEADY_BUDGET_NS: f64 = 750.0 * NANOS_PER_MICROSECOND;
pub(crate) const EXTERNAL_HTML_VISIBLE_WINDOW_SWEEP_BUDGET_NS: f64 = 150.0 * NANOS_PER_MICROSECOND;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PerfBudgetSpec {
    pub(crate) label: &'static str,
    pub(crate) estimate_path: &'static str,
    pub(crate) threshold_ns: f64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StructuralBudgetSpec {
    pub(crate) bench: &'static str,
    pub(crate) metric: &'static str,
    pub(crate) comparator: StructuralBudgetComparator,
    pub(crate) threshold: f64,
}
