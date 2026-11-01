use super::*;

pub(crate) fn invalid_tracked_sidecar_details(report: &PerfSidecarReport) -> Option<String> {
    if !report.bench.starts_with("app_launch/") {
        return None;
    }

    let missing_metrics = REQUIRED_APP_LAUNCH_ALLOCATION_METRICS
        .iter()
        .copied()
        .filter(|metric| sidecar_metric_value(report, metric).is_none())
        .collect::<Vec<_>>();
    if missing_metrics.is_empty() {
        return None;
    }

    Some(format!(
        "sidecar is missing required launch allocation metrics ({}) and is not a valid current app_launch baseline; timing-only launch sidecars must not be treated as comparable results",
        missing_metrics.join(", ")
    ))
}

pub(crate) fn sidecar_metric_value(report: &PerfSidecarReport, metric: &str) -> Option<f64> {
    report.metrics.get(metric).and_then(|value| match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    })
}

pub(crate) fn read_estimates(path: &Path) -> Result<CriterionEstimates, String> {
    let json = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&json).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

pub(crate) fn estimate_search_paths(
    criterion_roots: &[PathBuf],
    relative_path: &str,
) -> Vec<PathBuf> {
    criterion_roots
        .iter()
        .map(|root| root.join(relative_path))
        .collect()
}

pub(crate) fn sidecar_search_paths(criterion_roots: &[PathBuf], bench: &str) -> Vec<PathBuf> {
    criterion_roots
        .iter()
        .map(|root| criterion_sidecar_path(root, bench))
        .collect()
}

#[derive(Debug, Clone)]
pub(crate) enum ArtifactSelection {
    Fresh(PathBuf),
    Missing,
    Stale(Vec<PathBuf>),
}

pub(crate) fn load_artifact_freshness_reference(
    path: &Path,
) -> Result<ArtifactFreshnessReference, String> {
    let metadata = fs::metadata(path).map_err(|err| {
        format!(
            "failed to read freshness reference {}: {err}",
            path.display()
        )
    })?;
    let modified = metadata.modified().map_err(|err| {
        format!(
            "failed to read freshness reference timestamp {}: {err}",
            path.display()
        )
    })?;
    Ok(ArtifactFreshnessReference {
        path: path.to_path_buf(),
        modified,
    })
}

pub(crate) fn select_artifact_path(
    searched_paths: &[PathBuf],
    freshness_reference: Option<&ArtifactFreshnessReference>,
) -> Result<ArtifactSelection, String> {
    let mut stale_paths = Vec::new();

    for path in searched_paths {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(format!("failed to read artifact {}: {err}", path.display())),
        };

        if let Some(freshness_reference) = freshness_reference {
            let modified = metadata.modified().map_err(|err| {
                format!(
                    "failed to read artifact timestamp {}: {err}",
                    path.display()
                )
            })?;
            if modified < freshness_reference.modified {
                stale_paths.push(path.clone());
                continue;
            }
        }

        return Ok(ArtifactSelection::Fresh(path.clone()));
    }

    if stale_paths.is_empty() {
        Ok(ArtifactSelection::Missing)
    } else {
        Ok(ArtifactSelection::Stale(stale_paths))
    }
}

pub(crate) fn missing_or_stale_status(skip_missing: bool) -> BudgetStatus {
    if skip_missing {
        BudgetStatus::Skipped
    } else {
        BudgetStatus::Alert
    }
}

pub(crate) fn format_missing_paths_message(kind: &str, searched_paths: &[PathBuf]) -> String {
    if searched_paths.len() == 1 {
        return format!("missing {kind} at {}", searched_paths[0].display());
    }

    let mut details = format!("missing {kind}; looked in ");
    for (ix, path) in searched_paths.iter().enumerate() {
        if ix > 0 {
            details.push_str(", ");
        }
        let _ = write!(details, "{}", path.display());
    }
    details
}

pub(crate) fn format_stale_paths_message(
    kind: &str,
    stale_paths: &[PathBuf],
    freshness_reference: &ArtifactFreshnessReference,
) -> String {
    if stale_paths.len() == 1 {
        return format!(
            "stale {kind} at {}; older than freshness reference {}",
            stale_paths[0].display(),
            freshness_reference.path.display()
        );
    }

    let mut details = format!(
        "stale {kind}; older than freshness reference {}; found only ",
        freshness_reference.path.display()
    );
    for (ix, path) in stale_paths.iter().enumerate() {
        if ix > 0 {
            details.push_str(", ");
        }
        let _ = write!(details, "{}", path.display());
    }
    details
}
