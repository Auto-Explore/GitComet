pub(crate) use gitcomet_ui_gpui::perf_sidecar::{
    PerfSidecarReport, criterion_sidecar_path, read_sidecar,
};
pub(crate) use serde::Deserialize;
pub(crate) use std::env;
pub(crate) use std::fmt::Write as _;
pub(crate) use std::fs::{self, OpenOptions};
pub(crate) use std::io::Write;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::time::SystemTime;

mod artifacts;
mod budgets;
mod cli;
mod evaluate;
mod model;
mod report;
#[cfg(test)]
mod tests;

pub(crate) use artifacts::*;
pub(crate) use budgets::*;
pub(crate) use cli::*;
pub(crate) use evaluate::*;
pub(crate) use model::*;
pub(crate) use report::*;

pub(crate) fn main() {
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
    let freshness_reference = cli
        .fresh_reference
        .as_deref()
        .map(load_artifact_freshness_reference)
        .transpose()?;
    let mut timing_results = Vec::with_capacity(PERF_BUDGETS.len());
    for &spec in PERF_BUDGETS {
        timing_results.push(evaluate_budget(
            spec,
            &cli.criterion_roots,
            cli.skip_missing,
            freshness_reference.as_ref(),
        ));
    }
    let mut structural_results = Vec::with_capacity(STRUCTURAL_BUDGETS.len());
    for &spec in STRUCTURAL_BUDGETS {
        structural_results.push(evaluate_structural_budget(
            spec,
            &cli.criterion_roots,
            cli.skip_missing,
            freshness_reference.as_ref(),
        ));
    }

    let markdown = build_report_markdown(
        &timing_results,
        &structural_results,
        &cli.criterion_roots,
        cli.strict,
        freshness_reference.as_ref(),
    );
    println!("{markdown}");
    append_github_summary(&markdown)?;

    let mut has_alert = false;
    for result in &timing_results {
        if result.status == BudgetStatus::Alert {
            has_alert = true;
            emit_github_warning(&format!("{}: {}", result.spec.label, result.details));
        }
    }
    for result in &structural_results {
        if result.status == BudgetStatus::Alert {
            has_alert = true;
            emit_github_warning(&format!(
                "{} [{}]: {}",
                result.spec.bench, result.spec.metric, result.details
            ));
        }
    }

    if has_alert && cli.strict {
        return Err(
            "one or more performance budgets exceeded thresholds (strict mode enabled)".to_string(),
        );
    }

    Ok(())
}
