#[cfg(target_os = "linux")]
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
use std::env;
use std::fs;
use std::process;
use std::sync::Once;
use std::thread;
use std::time::Duration;

const DISABLE_ENV: &str = "GITCOMET_PERF_DISABLE_RAM_GUARD";
const RSS_LIMIT_PERCENT: u64 = 75;
const RSS_LIMIT_MAX_GIB_ENV: &str = "GITCOMET_PERF_RAM_GUARD_MAX_GIB";
const DEFAULT_RSS_LIMIT_MAX_KIB: u64 = 8 * 1024 * 1024;
const RSS_LIMIT_DESCRIPTION: &str =
    "smaller of configured GiB cap and 75% of startup available RAM";
const RSS_LIMIT_TOLERANCE_KIB: u64 = 256 * 1024;
const AS_LIMIT_MULTIPLIER: u64 = 2;
const AS_LIMIT_HEADROOM_KIB: u64 = 2 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const OVER_LIMIT_POLL_THRESHOLD: u32 = 5;

#[cfg(target_os = "linux")]
static PROCESS_GUARD: Once = Once::new();

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BenchmarkRamGuardLimits {
    startup_available_kib: u64,
    rss_limit_kib: u64,
}

#[cfg(target_os = "linux")]
pub fn install_benchmark_process_ram_guard() {
    PROCESS_GUARD.call_once(|| {
        if env_flag(DISABLE_ENV) {
            return;
        }

        let Some(limits) = benchmark_ram_guard_limits() else {
            return;
        };
        let rss_enforced_limit_kib = limits
            .rss_limit_kib
            .saturating_add(RSS_LIMIT_TOLERANCE_KIB);

        install_hard_address_space_limit(limits);

        let _ = thread::Builder::new()
            .name("perf-ram-guard".to_string())
            .spawn(move || {
                let mut over_limit_polls = 0u32;
                loop {
                if let Some(rss_kib) = process_rss_kib(process::id()) {
                    if rss_kib > rss_enforced_limit_kib {
                        over_limit_polls = over_limit_polls.saturating_add(1);
                        if over_limit_polls < OVER_LIMIT_POLL_THRESHOLD {
                            thread::sleep(POLL_INTERVAL);
                            continue;
                        }
                        eprintln!(
                            "benchmark RAM guard triggered: process RSS {} KiB exceeded enforced limit {} KiB (base limit {} KiB + tolerance {} KiB) for {} consecutive polls ({}; startup available RAM {} KiB)",
                            rss_kib,
                            rss_enforced_limit_kib,
                            limits.rss_limit_kib,
                            RSS_LIMIT_TOLERANCE_KIB,
                            OVER_LIMIT_POLL_THRESHOLD,
                            RSS_LIMIT_DESCRIPTION,
                            limits.startup_available_kib
                        );
                        process::exit(137);
                    }
                    over_limit_polls = 0;
                } else {
                    over_limit_polls = 0;
                }
                thread::sleep(POLL_INTERVAL);
                }
            });
    });
}

#[cfg(not(target_os = "linux"))]
pub fn install_benchmark_process_ram_guard() {}

#[cfg(target_os = "linux")]
pub fn benchmark_ram_limit_kib() -> Option<u64> {
    if env_flag(DISABLE_ENV) {
        return None;
    }

    benchmark_ram_guard_limits().map(|limits| limits.rss_limit_kib)
}

#[cfg(not(target_os = "linux"))]
pub fn benchmark_ram_limit_kib() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
pub fn process_rss_kib(pid: u32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    parse_status_kib(&status, "VmRSS:")
}

#[cfg(not(target_os = "linux"))]
pub fn process_rss_kib(_pid: u32) -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn process_vmsize_kib(pid: u32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    parse_status_kib(&status, "VmSize:")
}

#[cfg(not(target_os = "linux"))]
fn process_vmsize_kib(_pid: u32) -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn install_hard_address_space_limit(limits: BenchmarkRamGuardLimits) {
    let desired_limit_bytes =
        desired_address_space_limit_bytes_with_headroom(limits, process_vmsize_kib(process::id()));
    if desired_limit_bytes == 0 {
        return;
    }

    let existing_limit = getrlimit(Resource::As);
    let capped_limit = cap_address_space_rlimit(existing_limit, desired_limit_bytes);
    if capped_limit == existing_limit {
        return;
    }

    if let Err(err) = setrlimit(Resource::As, capped_limit) {
        eprintln!(
            "benchmark RAM guard warning: failed to install hard address-space limit {} bytes ({}): {}",
            desired_limit_bytes, RSS_LIMIT_DESCRIPTION, err
        );
    }
}

#[cfg(target_os = "linux")]
fn desired_address_space_limit_bytes_with_headroom(
    limits: BenchmarkRamGuardLimits,
    current_vmsize_kib: Option<u64>,
) -> u64 {
    let rss_limit_bytes = limits.rss_limit_kib.saturating_mul(1024);
    let multiplier_limit_bytes = rss_limit_bytes.saturating_mul(AS_LIMIT_MULTIPLIER);
    let headroom_limit_bytes = current_vmsize_kib
        .unwrap_or(0)
        .saturating_add(AS_LIMIT_HEADROOM_KIB)
        .saturating_mul(1024);
    multiplier_limit_bytes.max(headroom_limit_bytes)
}

#[cfg(target_os = "linux")]
fn cap_address_space_rlimit(existing: Rlimit, desired_limit_bytes: u64) -> Rlimit {
    let maximum = existing
        .maximum
        .unwrap_or(u64::MAX)
        .min(desired_limit_bytes);
    let current = Some(existing.current.unwrap_or(u64::MAX).min(maximum));
    let maximum = Some(maximum);
    Rlimit { current, maximum }
}

#[cfg(target_os = "linux")]
fn benchmark_ram_guard_limits() -> Option<BenchmarkRamGuardLimits> {
    let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
    benchmark_ram_guard_limits_from_meminfo(&meminfo, configured_rss_limit_max_kib())
}

#[cfg(target_os = "linux")]
fn benchmark_ram_guard_limits_from_meminfo(
    meminfo: &str,
    rss_limit_max_kib: u64,
) -> Option<BenchmarkRamGuardLimits> {
    let startup_available_kib = parse_meminfo_kib(meminfo, "MemAvailable:")
        .or_else(|| parse_meminfo_kib(meminfo, "MemTotal:"))?;
    let rss_limit_kib = startup_available_kib.saturating_mul(RSS_LIMIT_PERCENT) / 100;
    let rss_limit_kib = rss_limit_kib.min(rss_limit_max_kib);
    (rss_limit_kib > 0).then_some(BenchmarkRamGuardLimits {
        startup_available_kib,
        rss_limit_kib,
    })
}

#[cfg(target_os = "linux")]
fn configured_rss_limit_max_kib() -> u64 {
    env_u64(RSS_LIMIT_MAX_GIB_ENV)
        .and_then(|gib| gib.checked_mul(1024 * 1024))
        .filter(|&kib| kib > 0)
        .unwrap_or(DEFAULT_RSS_LIMIT_MAX_KIB)
}

#[cfg(target_os = "linux")]
fn parse_meminfo_kib(meminfo: &str, key: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.split_whitespace().next()?;
        value.parse::<u64>().ok()
    })
}

#[cfg(target_os = "linux")]
fn parse_status_kib(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.split_whitespace().next()?;
        value.parse::<u64>().ok()
    })
}

fn env_flag(key: &str) -> bool {
    env::var(key)
        .ok()
        .as_deref()
        .map(parse_bool_flag)
        .unwrap_or(false)
}

fn env_u64(key: &str) -> Option<u64> {
    env::var(key).ok()?.trim().parse().ok()
}

fn parse_bool_flag(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn parse_meminfo_uses_memavailable_when_present() {
        let limits = benchmark_ram_guard_limits_from_meminfo(
            "MemTotal:       16384000 kB\nMemAvailable:    12288000 kB\n",
            DEFAULT_RSS_LIMIT_MAX_KIB,
        )
        .expect("parse meminfo");
        assert_eq!(limits.startup_available_kib, 12_288_000);
        assert_eq!(limits.rss_limit_kib, 8_388_608);
    }

    #[test]
    fn parse_meminfo_falls_back_to_memtotal() {
        let limits = benchmark_ram_guard_limits_from_meminfo(
            "MemTotal:       8000 kB\n",
            DEFAULT_RSS_LIMIT_MAX_KIB,
        )
        .expect("limit");
        assert_eq!(limits.startup_available_kib, 8_000);
        assert_eq!(limits.rss_limit_kib, 6_000);
    }

    #[test]
    fn parse_meminfo_keeps_percent_limit_when_below_cap() {
        let limits = benchmark_ram_guard_limits_from_meminfo(
            "MemTotal:       10000000 kB\nMemAvailable:    6000000 kB\n",
            DEFAULT_RSS_LIMIT_MAX_KIB,
        )
        .expect("parse meminfo");
        assert_eq!(limits.startup_available_kib, 6_000_000);
        assert_eq!(limits.rss_limit_kib, 4_500_000);
    }

    #[test]
    fn parse_meminfo_honors_configured_cap_override() {
        let limits = benchmark_ram_guard_limits_from_meminfo(
            "MemTotal:       32000000 kB\nMemAvailable:    24000000 kB\n",
            12 * 1024 * 1024,
        )
        .expect("parse meminfo");
        assert_eq!(limits.startup_available_kib, 24_000_000);
        assert_eq!(limits.rss_limit_kib, 12 * 1024 * 1024);
    }

    #[test]
    fn desired_address_space_limit_matches_rss_cap() {
        let limits = BenchmarkRamGuardLimits {
            startup_available_kib: 12_288_000,
            rss_limit_kib: 8_388_608,
        };
        assert_eq!(
            desired_address_space_limit_bytes_with_headroom(limits, None),
            17_179_869_184
        );
    }

    #[test]
    fn desired_address_space_limit_accounts_for_virtual_headroom() {
        let limits = BenchmarkRamGuardLimits {
            startup_available_kib: 12_288_000,
            rss_limit_kib: 8_388_608,
        };
        assert_eq!(
            desired_address_space_limit_bytes_with_headroom(limits, Some(15 * 1024 * 1024)),
            18_253_611_008
        );
    }

    #[test]
    fn address_space_rlimit_caps_unlimited_process_limit() {
        let capped = cap_address_space_rlimit(
            Rlimit {
                current: None,
                maximum: None,
            },
            17_179_869_184,
        );
        assert_eq!(
            capped,
            Rlimit {
                current: Some(17_179_869_184),
                maximum: Some(17_179_869_184),
            }
        );
    }

    #[test]
    fn address_space_rlimit_preserves_stricter_existing_limit() {
        let capped = cap_address_space_rlimit(
            Rlimit {
                current: Some(4_294_967_296),
                maximum: Some(6_442_450_944),
            },
            17_179_869_184,
        );
        assert_eq!(
            capped,
            Rlimit {
                current: Some(4_294_967_296),
                maximum: Some(6_442_450_944),
            }
        );
    }

    #[test]
    fn parse_status_reads_vmrss() {
        let rss =
            parse_status_kib("Name:\tbench\nVmRSS:\t   12345 kB\n", "VmRSS:").expect("parse rss");
        assert_eq!(rss, 12_345);
    }

    #[test]
    fn parse_status_reads_vmsize() {
        let vmsize =
            parse_status_kib("Name:\tbench\nVmSize:\t 1048576 kB\n", "VmSize:").expect("parse");
        assert_eq!(vmsize, 1_048_576);
    }
}
