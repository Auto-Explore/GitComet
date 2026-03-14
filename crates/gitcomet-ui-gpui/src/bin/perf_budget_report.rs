use serde::Deserialize;
use std::env;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const NANOS_PER_MICROSECOND: f64 = 1_000.0;
const NANOS_PER_MILLISECOND: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug)]
struct PerfBudgetSpec {
    label: &'static str,
    estimate_path: &'static str,
    threshold_ns: f64,
}

const PERF_BUDGETS: &[PerfBudgetSpec] = &[
    PerfBudgetSpec {
        label: "conflict_three_way_scroll/style_window/200",
        estimate_path: "conflict_three_way_scroll/style_window/200/new/estimates.json",
        threshold_ns: 8.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_two_way_split_scroll/window_200",
        estimate_path: "conflict_two_way_split_scroll/window_200/new/estimates.json",
        threshold_ns: 6.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_search_query_update/window/200",
        estimate_path: "conflict_search_query_update/window/200/new/estimates.json",
        threshold_ns: 40.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_split_resize_step/window/200",
        estimate_path: "conflict_split_resize_step/window/200/new/estimates.json",
        threshold_ns: 25.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "markdown_preview_parse_build/single_document/medium",
        estimate_path: "markdown_preview_parse_build/single_document/medium/new/estimates.json",
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "markdown_preview_parse_build/two_sided_diff/medium",
        estimate_path: "markdown_preview_parse_build/two_sided_diff/medium/new/estimates.json",
        threshold_ns: 500.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "markdown_preview_render_single/window_rows/200",
        estimate_path: "markdown_preview_render_single/window_rows/200/new/estimates.json",
        threshold_ns: 1.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "markdown_preview_render_diff/window_rows/200",
        estimate_path: "markdown_preview_render_diff/window_rows/200/new/estimates.json",
        threshold_ns: 1.5 * NANOS_PER_MILLISECOND,
    },
];

#[derive(Debug, Clone, Deserialize)]
struct CriterionEstimates {
    mean: EstimateDistribution,
}

#[derive(Debug, Clone, Deserialize)]
struct EstimateDistribution {
    point_estimate: f64,
    confidence_interval: ConfidenceInterval,
}

#[derive(Debug, Clone, Deserialize)]
struct ConfidenceInterval {
    upper_bound: f64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BudgetStatus {
    WithinBudget,
    Alert,
}

impl BudgetStatus {
    fn icon(self) -> &'static str {
        match self {
            Self::WithinBudget => "OK",
            Self::Alert => "ALERT",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::WithinBudget => "within budget",
            Self::Alert => "alert",
        }
    }
}

#[derive(Debug, Clone)]
struct BudgetResult {
    spec: PerfBudgetSpec,
    status: BudgetStatus,
    mean_ns: Option<f64>,
    mean_upper_ns: Option<f64>,
    details: String,
}

#[derive(Debug, Clone)]
struct CliArgs {
    criterion_root: PathBuf,
    strict: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CliParseResult {
    Run,
    Help,
}

fn main() {
    match parse_cli_args(env::args().skip(1)) {
        Ok((CliParseResult::Help, _)) => {
            println!("{}", usage());
        }
        Ok((CliParseResult::Run, cli)) => {
            if let Err(err) = run_report(cli) {
                eprintln!("{err}");
                std::process::exit(2);
            }
        }
        Err(err) => {
            eprintln!("{err}");
            eprintln!();
            eprintln!("{}", usage());
            std::process::exit(2);
        }
    }
}

fn run_report(cli: CliArgs) -> Result<(), String> {
    let mut results = Vec::with_capacity(PERF_BUDGETS.len());
    for &spec in PERF_BUDGETS {
        results.push(evaluate_budget(spec, &cli.criterion_root));
    }

    let markdown = build_report_markdown(&results, &cli.criterion_root, cli.strict);
    println!("{markdown}");
    append_github_summary(&markdown)?;

    let mut has_alert = false;
    for result in &results {
        if result.status == BudgetStatus::Alert {
            has_alert = true;
            emit_github_warning(&format!("{}: {}", result.spec.label, result.details));
        }
    }

    if has_alert && cli.strict {
        return Err(
            "one or more performance budgets exceeded thresholds (strict mode enabled)".to_string(),
        );
    }

    Ok(())
}

fn evaluate_budget(spec: PerfBudgetSpec, criterion_root: &Path) -> BudgetResult {
    let estimate_path = criterion_root.join(spec.estimate_path);
    if !estimate_path.exists() {
        return BudgetResult {
            spec,
            status: BudgetStatus::Alert,
            mean_ns: None,
            mean_upper_ns: None,
            details: format!("missing estimate file at {}", estimate_path.display()),
        };
    }

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

fn read_estimates(path: &Path) -> Result<CriterionEstimates, String> {
    let json = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&json).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn build_report_markdown(results: &[BudgetResult], criterion_root: &Path, strict: bool) -> String {
    let mut markdown = String::new();
    let _ = writeln!(markdown, "## View Performance Budget Report");
    let _ = writeln!(markdown);
    let _ = writeln!(markdown, "- criterion root: `{}`", criterion_root.display());
    let _ = writeln!(
        markdown,
        "- mode: {}",
        if strict {
            "strict (fails on alert)"
        } else {
            "alert-only"
        }
    );
    let _ = writeln!(markdown);
    let _ = writeln!(
        markdown,
        "| Benchmark | Threshold | Mean | Mean 95% upper | Status |"
    );
    let _ = writeln!(markdown, "| --- | --- | --- | --- | --- |");

    for result in results {
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

    let mut alert_count = 0usize;
    for result in results {
        if result.status == BudgetStatus::Alert {
            alert_count = alert_count.saturating_add(1);
        }
    }

    let _ = writeln!(markdown);
    if alert_count == 0 {
        let _ = writeln!(markdown, "All tracked view benchmarks are within budget.");
    } else {
        let _ = writeln!(markdown, "Budget alerts: {alert_count}");
        for result in results {
            if result.status == BudgetStatus::Alert {
                let _ = writeln!(markdown, "- `{}`: {}", result.spec.label, result.details);
            }
        }
    }
    markdown
}

fn append_github_summary(markdown: &str) -> Result<(), String> {
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

fn emit_github_warning(message: &str) {
    println!("::warning title=View performance budget::{message}");
}

fn format_duration_ns(ns: f64) -> String {
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

fn parse_cli_args<I>(args: I) -> Result<(CliParseResult, CliArgs), String>
where
    I: IntoIterator<Item = String>,
{
    let mut criterion_root = PathBuf::from("target/criterion");
    let mut strict = strict_from_env();

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--criterion-root" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--criterion-root requires a path argument".to_string())?;
                criterion_root = PathBuf::from(value);
            }
            "--strict" => strict = true,
            "--help" | "-h" => {
                return Ok((
                    CliParseResult::Help,
                    CliArgs {
                        criterion_root,
                        strict,
                    },
                ));
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
    }

    Ok((
        CliParseResult::Run,
        CliArgs {
            criterion_root,
            strict,
        },
    ))
}

fn strict_from_env() -> bool {
    match env::var("GITCOMET_PERF_BUDGET_STRICT") {
        Ok(value) => is_truthy(&value),
        Err(_) => false,
    }
}

fn is_truthy(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
}

fn usage() -> &'static str {
    "Usage: cargo run -p gitcomet-ui-gpui --bin perf_budget_report -- [--criterion-root PATH] [--strict]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_estimates_reads_criterion_mean_shape() {
        let json = r#"{
            "mean": {
                "confidence_interval": {
                    "confidence_level": 0.95,
                    "lower_bound": 295963.49,
                    "upper_bound": 298962.86
                },
                "point_estimate": 297427.72,
                "standard_error": 771.75
            }
        }"#;
        let parsed: CriterionEstimates =
            serde_json::from_str(json).expect("criterion estimate json should parse");
        assert!((parsed.mean.point_estimate - 297_427.72).abs() < 0.01);
        assert!((parsed.mean.confidence_interval.upper_bound - 298_962.86).abs() < 0.01);
    }

    #[test]
    fn evaluate_budget_alerts_when_estimate_file_is_missing() {
        let temp_dir = TempDir::new().expect("tempdir");
        let spec = PerfBudgetSpec {
            label: "missing",
            estimate_path: "missing/new/estimates.json",
            threshold_ns: 1_000.0,
        };
        let result = evaluate_budget(spec, temp_dir.path());
        assert_eq!(result.status, BudgetStatus::Alert);
        assert!(result.details.contains("missing estimate file"));
    }

    #[test]
    fn evaluate_budget_within_budget_when_upper_bound_is_below_threshold() {
        let temp_dir = TempDir::new().expect("tempdir");
        let spec = PerfBudgetSpec {
            label: "within",
            estimate_path: "within/new/estimates.json",
            threshold_ns: 10_000.0,
        };
        write_estimate_file(temp_dir.path(), spec.estimate_path, 9_100.0, 9_800.0);

        let result = evaluate_budget(spec, temp_dir.path());
        assert_eq!(result.status, BudgetStatus::WithinBudget);
        assert_eq!(result.mean_ns, Some(9_100.0));
        assert_eq!(result.mean_upper_ns, Some(9_800.0));
    }

    #[test]
    fn evaluate_budget_alerts_when_threshold_is_exceeded() {
        let temp_dir = TempDir::new().expect("tempdir");
        let spec = PerfBudgetSpec {
            label: "over",
            estimate_path: "over/new/estimates.json",
            threshold_ns: 10_000.0,
        };
        write_estimate_file(temp_dir.path(), spec.estimate_path, 11_000.0, 12_500.0);

        let result = evaluate_budget(spec, temp_dir.path());
        assert_eq!(result.status, BudgetStatus::Alert);
        assert_eq!(result.mean_ns, Some(11_000.0));
        assert_eq!(result.mean_upper_ns, Some(12_500.0));
        assert!(result.details.contains("exceeds threshold"));
    }

    #[test]
    fn format_duration_ns_uses_human_units() {
        assert_eq!(format_duration_ns(999.0), "999 ns");
        assert_eq!(format_duration_ns(1_250.0), "1.250 us");
        assert_eq!(format_duration_ns(2_750_000.0), "2.750 ms");
    }

    #[test]
    fn parse_cli_args_defaults_to_alert_mode() {
        let (mode, cli) = parse_cli_args(Vec::<String>::new()).expect("parse args");
        assert_eq!(mode, CliParseResult::Run);
        assert_eq!(cli.criterion_root, PathBuf::from("target/criterion"));
        assert!(!cli.strict);
    }

    #[test]
    fn parse_cli_args_supports_root_and_strict() {
        let args = vec![
            "--criterion-root".to_string(),
            "/tmp/criterion".to_string(),
            "--strict".to_string(),
        ];
        let (mode, cli) = parse_cli_args(args).expect("parse args");
        assert_eq!(mode, CliParseResult::Run);
        assert_eq!(cli.criterion_root, PathBuf::from("/tmp/criterion"));
        assert!(cli.strict);
    }

    #[test]
    fn perf_budgets_include_markdown_preview_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"markdown_preview_parse_build/single_document/medium"));
        assert!(labels.contains(&"markdown_preview_parse_build/two_sided_diff/medium"));
        assert!(labels.contains(&"markdown_preview_render_single/window_rows/200"));
        assert!(labels.contains(&"markdown_preview_render_diff/window_rows/200"));
    }

    #[test]
    fn build_report_markdown_uses_generic_view_heading() {
        let markdown = build_report_markdown(&[], Path::new("target/criterion"), false);
        assert!(markdown.contains("## View Performance Budget Report"));
        assert!(markdown.contains("All tracked view benchmarks are within budget."));
    }

    fn write_estimate_file(root: &Path, relative_path: &str, mean: f64, upper: f64) {
        let path = root.join(relative_path);
        let parent = path.parent().expect("estimate path parent");
        fs::create_dir_all(parent).expect("create estimate directories");
        let content = format!(
            r#"{{
                "mean": {{
                    "confidence_interval": {{
                        "confidence_level": 0.95,
                        "lower_bound": {mean},
                        "upper_bound": {upper}
                    }},
                    "point_estimate": {mean},
                    "standard_error": 1.0
                }}
            }}"#
        );
        fs::write(path, content).expect("write estimate file");
    }
}
