//! External filters are per worker: unrestricted status parallelism can launch
//! dozens of LFS processes on Windows. Keep tuning separate from status semantics.

pub(super) fn for_repo(repo: &gix::Repository) -> Option<usize> {
    #[cfg(all(windows, feature = "benchmarks"))]
    {
        if !has_external_filter(&repo.config_snapshot()) {
            return None;
        }
        // Available only in benchmark builds; normal builds do not read an override.
        if let Ok(value) = std::env::var("GITCOMET_BENCH_STATUS_WORKERS") {
            return parse_limit(
                &value,
                std::thread::available_parallelism().map_or(1, usize::from),
            );
        }
    }
    #[cfg(not(all(windows, feature = "benchmarks")))]
    let _ = repo;
    // Keep automatic scheduling until Windows CPU AND latency measurements
    // justify a production limit. See docs/performance/lfs-status.md.
    None
}

#[cfg(any(all(windows, feature = "benchmarks"), test))]
fn has_external_filter(config: &gix::config::File) -> bool {
    config
        .sections_by_name("filter")
        .is_some_and(|mut sections| {
            sections.any(|section| {
                section.header().subsection_name().is_some()
                    && ["clean", "process"]
                        .iter()
                        .any(|key| section.value(key).is_some_and(|value| !value.is_empty()))
            })
        })
}

#[cfg(any(all(windows, feature = "benchmarks"), test))]
fn parse_limit(value: &str, cores: usize) -> Option<usize> {
    match value {
        "1" => Some(1),
        "4" => Some(4.min(cores.max(1))),
        "8" => Some(8.min(cores.max(1))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_external_clean_and_process_filters_only() {
        for (config, expected) in [
            ("[core]\n autocrlf = true\n", false),
            (
                "[filter \"lfs\"]\n process = git-lfs filter-process\n",
                true,
            ),
            ("[filter \"custom\"]\n clean = cat\n", true),
            ("[filter \"custom\"]\n smudge = cat\n", false),
            ("[filter \"custom\"]\n clean =\n process =\n", false),
        ] {
            let file = gix::config::File::try_from(config).unwrap();
            assert_eq!(has_external_filter(&file), expected, "{config}");
        }
    }

    #[test]
    fn benchmark_limits_are_positive_bounded_and_explicit() {
        for cores in [1, 2, 64] {
            for value in ["1", "4", "8"] {
                assert_eq!(
                    parse_limit(value, cores),
                    Some(value.parse::<usize>().unwrap().min(cores))
                );
            }
            for value in ["auto", "", "0", "-1", "100", "invalid"] {
                assert_eq!(parse_limit(value, cores), None);
            }
        }
    }
}
