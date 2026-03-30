use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) enum StructuralBudgetComparator {
    AtMost,
    AtLeast,
    Exactly,
}

impl StructuralBudgetComparator {
    pub(crate) fn matches(self, observed: f64, threshold: f64) -> bool {
        match self {
            Self::AtMost => observed <= threshold,
            Self::AtLeast => observed >= threshold,
            Self::Exactly => (observed - threshold).abs() <= f64::EPSILON,
        }
    }

    pub(crate) fn format_expectation(self, threshold: f64) -> String {
        match self {
            Self::AtMost => format!("<= {}", format_metric_value(threshold)),
            Self::AtLeast => format!(">= {}", format_metric_value(threshold)),
            Self::Exactly => format!("== {}", format_metric_value(threshold)),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CriterionEstimates {
    pub(crate) mean: EstimateDistribution,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EstimateDistribution {
    pub(crate) point_estimate: f64,
    pub(crate) confidence_interval: ConfidenceInterval,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ConfidenceInterval {
    pub(crate) upper_bound: f64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum BudgetStatus {
    WithinBudget,
    Alert,
    /// Benchmark data was not found and `--skip-missing` was active.
    Skipped,
}

impl BudgetStatus {
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Self::WithinBudget => "OK",
            Self::Alert => "ALERT",
            Self::Skipped => "SKIP",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::WithinBudget => "within budget",
            Self::Alert => "alert",
            Self::Skipped => "skipped (not run)",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BudgetResult {
    pub(crate) spec: PerfBudgetSpec,
    pub(crate) status: BudgetStatus,
    pub(crate) mean_ns: Option<f64>,
    pub(crate) mean_upper_ns: Option<f64>,
    pub(crate) details: String,
}

#[derive(Debug, Clone)]
pub(crate) struct StructuralBudgetResult {
    pub(crate) spec: StructuralBudgetSpec,
    pub(crate) status: BudgetStatus,
    pub(crate) observed: Option<f64>,
    pub(crate) details: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ArtifactFreshnessReference {
    pub(crate) path: PathBuf,
    pub(crate) modified: SystemTime,
}

#[derive(Debug, Clone)]
pub(crate) struct CliArgs {
    pub(crate) criterion_roots: Vec<PathBuf>,
    pub(crate) strict: bool,
    /// When true, benchmarks whose estimate/sidecar files are missing are
    /// silently skipped instead of treated as alerts. Useful for PR CI that
    /// only runs a subset of the full suite.
    pub(crate) skip_missing: bool,
    /// Optional freshness gate. Artifacts older than this file's mtime are
    /// treated like missing data.
    pub(crate) fresh_reference: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum CliParseResult {
    Run,
    Help,
}

pub(crate) fn format_duration_ns(ns: f64) -> String {
    if !ns.is_finite() || ns < 0.0 {
        return "n/a".to_string();
    }
    if ns >= NANOS_PER_MILLISECOND {
        return format!("{:.3} ms", ns / NANOS_PER_MILLISECOND);
    }
    if ns >= NANOS_PER_MICROSECOND {
        return format!("{:.3} us", ns / NANOS_PER_MICROSECOND);
    }
    format!("{ns:.0} ns")
}

pub(crate) fn format_metric_value(value: f64) -> String {
    if !value.is_finite() {
        return "n/a".to_string();
    }
    if (value.fract()).abs() <= f64::EPSILON {
        return format!("{value:.0}");
    }
    format!("{value:.3}")
}
