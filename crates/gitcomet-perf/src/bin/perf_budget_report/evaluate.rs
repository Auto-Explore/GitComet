use super::*;

pub(crate) fn evaluate_budget(
    spec: PerfBudgetSpec,
    criterion_roots: &[PathBuf],
    skip_missing: bool,
    freshness_reference: Option<&ArtifactFreshnessReference>,
) -> BudgetResult {
    if let Some(metric) = spec.estimate_path.strip_prefix(SIDECAR_TIMING_MS_PREFIX) {
        return evaluate_sidecar_timing_budget(
            spec,
            criterion_roots,
            skip_missing,
            freshness_reference,
            metric,
        );
    }

    let searched_paths = estimate_search_paths(criterion_roots, spec.estimate_path);
    let estimate_path = match select_artifact_path(&searched_paths, freshness_reference) {
        Ok(ArtifactSelection::Fresh(path)) => path,
        Ok(ArtifactSelection::Missing) => {
            return BudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                mean_ns: None,
                mean_upper_ns: None,
                details: format_missing_paths_message("estimate file", &searched_paths),
            };
        }
        Ok(ArtifactSelection::Stale(stale_paths)) => {
            return BudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                mean_ns: None,
                mean_upper_ns: None,
                details: format_stale_paths_message(
                    "estimate file",
                    &stale_paths,
                    freshness_reference.expect("stale artifacts require freshness reference"),
                ),
            };
        }
        Err(err) => {
            return BudgetResult {
                spec,
                status: BudgetStatus::Alert,
                mean_ns: None,
                mean_upper_ns: None,
                details: err,
            };
        }
    };

    match read_estimates(&estimate_path) {
        Ok(estimates) => {
            let mean_ns = estimates.mean.point_estimate;
            let mean_upper_ns = estimates.mean.confidence_interval.upper_bound;
            if mean_upper_ns <= spec.threshold_ns {
                BudgetResult {
                    spec,
                    status: BudgetStatus::WithinBudget,
                    mean_ns: Some(mean_ns),
                    mean_upper_ns: Some(mean_upper_ns),
                    details: format!(
                        "mean upper bound {} <= threshold {}",
                        format_duration_ns(mean_upper_ns),
                        format_duration_ns(spec.threshold_ns)
                    ),
                }
            } else {
                BudgetResult {
                    spec,
                    status: BudgetStatus::Alert,
                    mean_ns: Some(mean_ns),
                    mean_upper_ns: Some(mean_upper_ns),
                    details: format!(
                        "mean upper bound {} exceeds threshold {}",
                        format_duration_ns(mean_upper_ns),
                        format_duration_ns(spec.threshold_ns)
                    ),
                }
            }
        }
        Err(err) => BudgetResult {
            spec,
            status: BudgetStatus::Alert,
            mean_ns: None,
            mean_upper_ns: None,
            details: err,
        },
    }
}

pub(crate) fn evaluate_sidecar_timing_budget(
    spec: PerfBudgetSpec,
    criterion_roots: &[PathBuf],
    skip_missing: bool,
    freshness_reference: Option<&ArtifactFreshnessReference>,
    metric: &str,
) -> BudgetResult {
    let searched_paths = sidecar_search_paths(criterion_roots, spec.label);
    let sidecar_path = match select_artifact_path(&searched_paths, freshness_reference) {
        Ok(ArtifactSelection::Fresh(path)) => path,
        Ok(ArtifactSelection::Missing) => {
            return BudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                mean_ns: None,
                mean_upper_ns: None,
                details: format_missing_paths_message("sidecar file", &searched_paths),
            };
        }
        Ok(ArtifactSelection::Stale(stale_paths)) => {
            return BudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                mean_ns: None,
                mean_upper_ns: None,
                details: format_stale_paths_message(
                    "sidecar file",
                    &stale_paths,
                    freshness_reference.expect("stale artifacts require freshness reference"),
                ),
            };
        }
        Err(err) => {
            return BudgetResult {
                spec,
                status: BudgetStatus::Alert,
                mean_ns: None,
                mean_upper_ns: None,
                details: err,
            };
        }
    };

    match read_sidecar(&sidecar_path) {
        Ok(report) => {
            if report.bench != spec.label {
                return BudgetResult {
                    spec,
                    status: BudgetStatus::Alert,
                    mean_ns: None,
                    mean_upper_ns: None,
                    details: format!(
                        "sidecar bench label {:?} does not match expected {:?}",
                        report.bench, spec.label
                    ),
                };
            }

            if let Some(details) = invalid_tracked_sidecar_details(&report) {
                return BudgetResult {
                    spec,
                    status: BudgetStatus::Alert,
                    mean_ns: None,
                    mean_upper_ns: None,
                    details,
                };
            }

            let Some(observed_ms) = sidecar_metric_value(&report, metric) else {
                return BudgetResult {
                    spec,
                    status: BudgetStatus::Alert,
                    mean_ns: None,
                    mean_upper_ns: None,
                    details: format!("missing numeric metric {:?}", metric),
                };
            };

            let observed_ns = observed_ms * NANOS_PER_MILLISECOND;
            if observed_ns <= spec.threshold_ns {
                BudgetResult {
                    spec,
                    status: BudgetStatus::WithinBudget,
                    mean_ns: Some(observed_ns),
                    mean_upper_ns: Some(observed_ns),
                    details: format!(
                        "sidecar metric {:?} {} <= threshold {}",
                        metric,
                        format_duration_ns(observed_ns),
                        format_duration_ns(spec.threshold_ns)
                    ),
                }
            } else {
                BudgetResult {
                    spec,
                    status: BudgetStatus::Alert,
                    mean_ns: Some(observed_ns),
                    mean_upper_ns: Some(observed_ns),
                    details: format!(
                        "sidecar metric {:?} {} exceeds threshold {}",
                        metric,
                        format_duration_ns(observed_ns),
                        format_duration_ns(spec.threshold_ns)
                    ),
                }
            }
        }
        Err(err) => BudgetResult {
            spec,
            status: BudgetStatus::Alert,
            mean_ns: None,
            mean_upper_ns: None,
            details: err,
        },
    }
}

pub(crate) fn evaluate_structural_budget(
    spec: StructuralBudgetSpec,
    criterion_roots: &[PathBuf],
    skip_missing: bool,
    freshness_reference: Option<&ArtifactFreshnessReference>,
) -> StructuralBudgetResult {
    let searched_paths = sidecar_search_paths(criterion_roots, spec.bench);
    let sidecar_path = match select_artifact_path(&searched_paths, freshness_reference) {
        Ok(ArtifactSelection::Fresh(path)) => path,
        Ok(ArtifactSelection::Missing) => {
            return StructuralBudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                observed: None,
                details: format_missing_paths_message("sidecar file", &searched_paths),
            };
        }
        Ok(ArtifactSelection::Stale(stale_paths)) => {
            return StructuralBudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                observed: None,
                details: format_stale_paths_message(
                    "sidecar file",
                    &stale_paths,
                    freshness_reference.expect("stale artifacts require freshness reference"),
                ),
            };
        }
        Err(err) => {
            return StructuralBudgetResult {
                spec,
                status: BudgetStatus::Alert,
                observed: None,
                details: err,
            };
        }
    };

    match read_sidecar(&sidecar_path) {
        Ok(report) => evaluate_structural_budget_from_report(spec, &report),
        Err(err) => StructuralBudgetResult {
            spec,
            status: BudgetStatus::Alert,
            observed: None,
            details: err,
        },
    }
}

pub(crate) fn evaluate_structural_budget_from_report(
    spec: StructuralBudgetSpec,
    report: &PerfSidecarReport,
) -> StructuralBudgetResult {
    if report.bench != spec.bench {
        return StructuralBudgetResult {
            spec,
            status: BudgetStatus::Alert,
            observed: None,
            details: format!(
                "sidecar bench label {:?} does not match expected {:?}",
                report.bench, spec.bench
            ),
        };
    }

    if let Some(details) = invalid_tracked_sidecar_details(report) {
        return StructuralBudgetResult {
            spec,
            status: BudgetStatus::Alert,
            observed: None,
            details,
        };
    }

    let Some(observed) = sidecar_metric_value(report, spec.metric) else {
        return StructuralBudgetResult {
            spec,
            status: BudgetStatus::Alert,
            observed: None,
            details: format!("missing numeric metric {:?}", spec.metric),
        };
    };

    let expectation = spec.comparator.format_expectation(spec.threshold);
    if spec.comparator.matches(observed, spec.threshold) {
        StructuralBudgetResult {
            spec,
            status: BudgetStatus::WithinBudget,
            observed: Some(observed),
            details: format!(
                "observed {} satisfies {}",
                format_metric_value(observed),
                expectation
            ),
        }
    } else {
        StructuralBudgetResult {
            spec,
            status: BudgetStatus::Alert,
            observed: Some(observed),
            details: format!("{} violates {}", format_metric_value(observed), expectation),
        }
    }
}
