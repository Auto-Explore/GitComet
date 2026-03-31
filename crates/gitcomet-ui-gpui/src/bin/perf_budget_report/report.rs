use super::*;

pub(crate) fn build_report_markdown(
    timing_results: &[BudgetResult],
    structural_results: &[StructuralBudgetResult],
    criterion_roots: &[PathBuf],
    strict: bool,
    freshness_reference: Option<&ArtifactFreshnessReference>,
) -> String {
    let mut markdown = String::new();
    let _ = writeln!(markdown, "## View Performance Budget Report");
    let _ = writeln!(markdown);
    let criterion_label = if criterion_roots.len() == 1 {
        "criterion root"
    } else {
        "criterion roots"
    };
    let _ = writeln!(
        markdown,
        "- {criterion_label}: {}",
        format_criterion_roots_markdown(criterion_roots)
    );
    let _ = writeln!(
        markdown,
        "- mode: {}",
        if strict {
            "strict (fails on alert)"
        } else {
            "alert-only"
        }
    );
    if let Some(freshness_reference) = freshness_reference {
        let _ = writeln!(
            markdown,
            "- freshness reference: `{}`",
            freshness_reference.path.display()
        );
    }
    let _ = writeln!(markdown);

    if !timing_results.is_empty() {
        let _ = writeln!(markdown, "### Timing Budgets");
        let _ = writeln!(
            markdown,
            "| Benchmark | Threshold | Mean | Mean 95% upper | Status |"
        );
        let _ = writeln!(markdown, "| --- | --- | --- | --- | --- |");

        for result in timing_results {
            let mean = result
                .mean_ns
                .map(format_duration_ns)
                .unwrap_or_else(|| "n/a".to_string());
            let mean_upper = result
                .mean_upper_ns
                .map(format_duration_ns)
                .unwrap_or_else(|| "n/a".to_string());
            let _ = writeln!(
                markdown,
                "| `{}` | <= {} | {} | {} | {} {} |",
                result.spec.label,
                format_duration_ns(result.spec.threshold_ns),
                mean,
                mean_upper,
                result.status.icon(),
                result.status.label()
            );
        }
        let _ = writeln!(markdown);
    }

    if !structural_results.is_empty() {
        let _ = writeln!(markdown, "### Structural Budgets");
        let _ = writeln!(
            markdown,
            "| Benchmark | Metric | Expectation | Observed | Status |"
        );
        let _ = writeln!(markdown, "| --- | --- | --- | --- | --- |");

        for result in structural_results {
            let observed = result
                .observed
                .map(format_metric_value)
                .unwrap_or_else(|| "n/a".to_string());
            let _ = writeln!(
                markdown,
                "| `{}` | `{}` | {} | {} | {} {} |",
                result.spec.bench,
                result.spec.metric,
                result
                    .spec
                    .comparator
                    .format_expectation(result.spec.threshold),
                observed,
                result.status.icon(),
                result.status.label()
            );
        }
        let _ = writeln!(markdown);
    }

    let mut alert_count = 0usize;
    let mut skipped_count = 0usize;
    for result in timing_results {
        match result.status {
            BudgetStatus::Alert => alert_count = alert_count.saturating_add(1),
            BudgetStatus::Skipped => skipped_count = skipped_count.saturating_add(1),
            _ => {}
        }
    }
    for result in structural_results {
        match result.status {
            BudgetStatus::Alert => alert_count = alert_count.saturating_add(1),
            BudgetStatus::Skipped => skipped_count = skipped_count.saturating_add(1),
            _ => {}
        }
    }
    let total_budget_count = timing_results
        .len()
        .saturating_add(structural_results.len());

    if skipped_count > 0 {
        let skipped_reason = if freshness_reference.is_some() {
            "benchmark data not present or older than the freshness reference"
        } else {
            "benchmark data not present"
        };
        let _ = writeln!(
            markdown,
            "Skipped {skipped_count} budget(s) ({skipped_reason})."
        );
    }

    if alert_count == 0 {
        if skipped_count == total_budget_count && total_budget_count > 0 {
            let _ = writeln!(
                markdown,
                "No fresh benchmark data matched the requested report inputs; all tracked budgets were skipped."
            );
        } else if skipped_count > 0 {
            let _ = writeln!(
                markdown,
                "All non-skipped tracked view benchmarks are within budget."
            );
        } else {
            let _ = writeln!(markdown, "All tracked view benchmarks are within budget.");
        }
    } else {
        let _ = writeln!(markdown, "Budget alerts: {alert_count}");
        for result in timing_results {
            if result.status == BudgetStatus::Alert {
                let _ = writeln!(markdown, "- `{}`: {}", result.spec.label, result.details);
            }
        }
        for result in structural_results {
            if result.status == BudgetStatus::Alert {
                let _ = writeln!(
                    markdown,
                    "- `{}` / `{}`: {}",
                    result.spec.bench, result.spec.metric, result.details
                );
            }
        }
    }
    markdown
}

pub(crate) fn format_criterion_roots_markdown(criterion_roots: &[PathBuf]) -> String {
    let mut roots = String::new();
    for (ix, root) in criterion_roots.iter().enumerate() {
        if ix > 0 {
            roots.push_str(", ");
        }
        let _ = write!(roots, "`{}`", root.display());
    }
    roots
}

pub(crate) fn append_github_summary(markdown: &str) -> Result<(), String> {
    let Some(path) = env::var_os("GITHUB_STEP_SUMMARY") else {
        return Ok(());
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("failed to open {}: {err}", PathBuf::from(path).display()))?;
    file.write_all(markdown.as_bytes())
        .map_err(|err| format!("failed to append report to GITHUB_STEP_SUMMARY: {err}"))?;
    file.write_all(b"\n")
        .map_err(|err| format!("failed to append newline to GITHUB_STEP_SUMMARY: {err}"))?;
    Ok(())
}

pub(crate) fn emit_github_warning(message: &str) {
    println!("::warning title=View performance budget::{message}");
}
