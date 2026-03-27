#[cfg(feature = "benchmarks")]
mod harness {
    use gitcomet_ui_gpui::benchmarks::{
        IdleResourceConfig, IdleResourceFixture, IdleResourceMetrics, IdleResourceScenario,
    };
    use gitcomet_ui_gpui::perf_alloc::{TRACKING_MIMALLOC, measure_allocations};
    use gitcomet_ui_gpui::perf_ram_guard::install_benchmark_process_ram_guard;
    use gitcomet_ui_gpui::perf_sidecar::{PerfSidecarReport, write_criterion_sidecar};
    use serde_json::{Map, Value, json};
    use std::env;
    use std::time::Duration;

    const DEFAULT_BENCH: &str = "idle/cpu_usage_single_repo_60s";
    const CPU_WINDOW_MS_ENV: &str = "GITCOMET_PERF_IDLE_CPU_WINDOW_MS";
    const MEMORY_WINDOW_MS_ENV: &str = "GITCOMET_PERF_IDLE_MEMORY_WINDOW_MS";
    const SAMPLE_INTERVAL_MS_ENV: &str = "GITCOMET_PERF_IDLE_SAMPLE_INTERVAL_MS";
    const REFRESH_CYCLES_ENV: &str = "GITCOMET_PERF_IDLE_REFRESH_CYCLES";
    const WAKE_GAP_MS_ENV: &str = "GITCOMET_PERF_IDLE_WAKE_GAP_MS";
    const TRACKED_FILES_ENV: &str = "GITCOMET_PERF_IDLE_TRACKED_FILES_PER_REPO";

    #[global_allocator]
    static GLOBAL: &gitcomet_ui_gpui::perf_alloc::PerfTrackingAllocator = &TRACKING_MIMALLOC;

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct CliArgs {
        pub bench: String,
    }

    pub fn run() -> Result<(), String> {
        install_benchmark_process_ram_guard();
        let args = parse_cli_args(env::args().skip(1))?;
        let scenario = scenario_from_bench(&args.bench)?;
        let fixture = IdleResourceFixture::with_config(scenario, config_for_scenario(scenario));
        let ((_hash, metrics), alloc_metrics) = measure_allocations(|| fixture.run_with_metrics());
        let sidecar_path = emit_idle_sidecar(&args.bench, &metrics, alloc_metrics)?;

        println!(
            "{} avg_cpu_pct={:.3} peak_cpu_pct={:.3} rss_delta_kib={} refresh_cycles={} sidecar={}",
            args.bench,
            metrics.avg_cpu_pct,
            metrics.peak_cpu_pct,
            metrics.rss_delta_kib,
            metrics.refresh_cycles,
            sidecar_path.display()
        );
        Ok(())
    }

    fn parse_cli_args(args: impl Iterator<Item = String>) -> Result<CliArgs, String> {
        let mut bench = DEFAULT_BENCH.to_string();
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--bench" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "missing value for --bench".to_string())?;
                    bench = value;
                }
                "--help" | "-h" => return Err(usage()),
                other => {
                    return Err(format!("unrecognized argument {other:?}\n\n{}", usage()));
                }
            }
        }

        Ok(CliArgs { bench })
    }

    fn usage() -> String {
        format!(
            "Usage: perf_idle_resource [--bench <label>]\n\nSupported labels:\n  idle/cpu_usage_single_repo_60s\n  idle/cpu_usage_ten_repos_60s\n  idle/memory_growth_single_repo_10min\n  idle/memory_growth_ten_repos_10min\n  idle/background_refresh_cost_per_cycle\n  idle/wake_from_sleep_resume\n\nDefaults to {DEFAULT_BENCH}."
        )
    }

    fn scenario_from_bench(bench: &str) -> Result<IdleResourceScenario, String> {
        match bench {
            "idle/cpu_usage_single_repo_60s" => Ok(IdleResourceScenario::CpuUsageSingleRepo60s),
            "idle/cpu_usage_ten_repos_60s" => Ok(IdleResourceScenario::CpuUsageTenRepos60s),
            "idle/memory_growth_single_repo_10min" => {
                Ok(IdleResourceScenario::MemoryGrowthSingleRepo10Min)
            }
            "idle/memory_growth_ten_repos_10min" => {
                Ok(IdleResourceScenario::MemoryGrowthTenRepos10Min)
            }
            "idle/background_refresh_cost_per_cycle" => {
                Ok(IdleResourceScenario::BackgroundRefreshCostPerCycle)
            }
            "idle/wake_from_sleep_resume" => Ok(IdleResourceScenario::WakeFromSleepResume),
            _ => Err(format!("unsupported idle benchmark {bench:?}")),
        }
    }

    fn config_for_scenario(scenario: IdleResourceScenario) -> IdleResourceConfig {
        let mut config = match scenario {
            IdleResourceScenario::CpuUsageSingleRepo60s => {
                IdleResourceConfig::cpu_usage_single_repo()
            }
            IdleResourceScenario::CpuUsageTenRepos60s => IdleResourceConfig::cpu_usage_ten_repos(),
            IdleResourceScenario::MemoryGrowthSingleRepo10Min => {
                IdleResourceConfig::memory_growth_single_repo()
            }
            IdleResourceScenario::MemoryGrowthTenRepos10Min => {
                IdleResourceConfig::memory_growth_ten_repos()
            }
            IdleResourceScenario::BackgroundRefreshCostPerCycle => {
                IdleResourceConfig::background_refresh_cost_per_cycle()
            }
            IdleResourceScenario::WakeFromSleepResume => {
                IdleResourceConfig::wake_from_sleep_resume()
            }
        };

        if let Some(tracked_files) = env_usize(TRACKED_FILES_ENV) {
            config.tracked_files_per_repo = tracked_files.max(1);
        }
        if let Some(sample_interval_ms) = env_u64(SAMPLE_INTERVAL_MS_ENV) {
            config.sample_interval = Duration::from_millis(sample_interval_ms.max(1));
        }

        match scenario {
            IdleResourceScenario::CpuUsageSingleRepo60s
            | IdleResourceScenario::CpuUsageTenRepos60s => {
                if let Some(window_ms) = env_u64(CPU_WINDOW_MS_ENV) {
                    config.sample_window = Duration::from_millis(window_ms.max(1));
                }
            }
            IdleResourceScenario::MemoryGrowthSingleRepo10Min
            | IdleResourceScenario::MemoryGrowthTenRepos10Min => {
                if let Some(window_ms) = env_u64(MEMORY_WINDOW_MS_ENV) {
                    config.sample_window = Duration::from_millis(window_ms.max(1));
                }
            }
            IdleResourceScenario::BackgroundRefreshCostPerCycle => {
                if let Some(refresh_cycles) = env_usize(REFRESH_CYCLES_ENV) {
                    config.refresh_cycles = refresh_cycles.max(1);
                }
            }
            IdleResourceScenario::WakeFromSleepResume => {
                if let Some(wake_gap_ms) = env_u64(WAKE_GAP_MS_ENV) {
                    config.wake_gap = Duration::from_millis(wake_gap_ms);
                }
            }
        }

        config
    }

    fn emit_idle_sidecar(
        bench: &str,
        metrics: &IdleResourceMetrics,
        alloc_metrics: gitcomet_ui_gpui::perf_alloc::PerfAllocMetrics,
    ) -> Result<std::path::PathBuf, String> {
        let mut payload = Map::<String, Value>::new();
        payload.insert("open_repos".to_string(), json!(metrics.open_repos));
        payload.insert(
            "tracked_files_per_repo".to_string(),
            json!(metrics.tracked_files_per_repo),
        );
        payload.insert(
            "sample_duration_ms".to_string(),
            json!(metrics.sample_duration_ms),
        );
        payload.insert("sample_count".to_string(), json!(metrics.sample_count));
        payload.insert("avg_cpu_pct".to_string(), json!(metrics.avg_cpu_pct));
        payload.insert("peak_cpu_pct".to_string(), json!(metrics.peak_cpu_pct));
        payload.insert("rss_start_kib".to_string(), json!(metrics.rss_start_kib));
        payload.insert("rss_end_kib".to_string(), json!(metrics.rss_end_kib));
        payload.insert("rss_delta_kib".to_string(), json!(metrics.rss_delta_kib));
        payload.insert("refresh_cycles".to_string(), json!(metrics.refresh_cycles));
        payload.insert(
            "repos_refreshed".to_string(),
            json!(metrics.repos_refreshed),
        );
        payload.insert("status_calls".to_string(), json!(metrics.status_calls));
        payload.insert("status_ms".to_string(), json!(metrics.status_ms));
        payload.insert(
            "avg_refresh_cycle_ms".to_string(),
            json!(metrics.avg_refresh_cycle_ms),
        );
        payload.insert(
            "max_refresh_cycle_ms".to_string(),
            json!(metrics.max_refresh_cycle_ms),
        );
        payload.insert("wake_resume_ms".to_string(), json!(metrics.wake_resume_ms));
        alloc_metrics.append_to_payload(&mut payload);

        let report = PerfSidecarReport::new(bench, payload);
        write_criterion_sidecar(&report)
    }

    fn env_u64(key: &str) -> Option<u64> {
        env::var(key)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
    }

    fn env_usize(key: &str) -> Option<usize> {
        env::var(key)
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parse_cli_args_defaults_to_single_repo_cpu_case() {
            let parsed = parse_cli_args(std::iter::empty()).expect("parse args");
            assert_eq!(parsed.bench, DEFAULT_BENCH);
        }

        #[test]
        fn scenario_from_bench_supports_all_idle_cases() {
            for bench in [
                "idle/cpu_usage_single_repo_60s",
                "idle/cpu_usage_ten_repos_60s",
                "idle/memory_growth_single_repo_10min",
                "idle/memory_growth_ten_repos_10min",
                "idle/background_refresh_cost_per_cycle",
                "idle/wake_from_sleep_resume",
            ] {
                assert!(scenario_from_bench(bench).is_ok(), "missing {bench}");
            }
        }
    }
}

#[cfg(feature = "benchmarks")]
fn main() {
    if let Err(err) = harness::run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

#[cfg(not(feature = "benchmarks"))]
fn main() {
    eprintln!("perf_idle_resource requires --features benchmarks");
    std::process::exit(2);
}
