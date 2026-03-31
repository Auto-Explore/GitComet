use gpui::SharedString;
use rustc_hash::FxHashMap as HashMap;
use std::path::{Path, PathBuf};

#[cfg(feature = "benchmarks")]
pub(in crate::view) type PathDisplayCache = HashMap<PathBuf, SharedString>;

#[cfg(feature = "benchmarks")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct PathDisplayBenchCounters {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_clears: u64,
}

#[cfg(feature = "benchmarks")]
thread_local! {
    static PATH_DISPLAY_BENCH_COUNTERS: std::cell::Cell<PathDisplayBenchCounters> =
        const { std::cell::Cell::new(PathDisplayBenchCounters {
            cache_hits: 0,
            cache_misses: 0,
            cache_clears: 0,
        }) };
}

pub(super) fn path_display_string(path: &Path) -> String {
    format_windows_path_for_display(path.display().to_string())
}

pub(super) fn path_display_shared(path: &Path) -> SharedString {
    path_display_string(path).into()
}

pub(super) fn cached_path_display(
    cache: &mut HashMap<PathBuf, SharedString>,
    path: &PathBuf,
) -> SharedString {
    const MAX_ENTRIES: usize = 8_192;
    if cache.len() > MAX_ENTRIES {
        cache.clear();
        #[cfg(feature = "benchmarks")]
        PATH_DISPLAY_BENCH_COUNTERS.with(|counters| {
            let mut snapshot = counters.get();
            snapshot.cache_clears = snapshot.cache_clears.saturating_add(1);
            counters.set(snapshot);
        });
    }
    if let Some(s) = cache.get(path) {
        #[cfg(feature = "benchmarks")]
        PATH_DISPLAY_BENCH_COUNTERS.with(|counters| {
            let mut snapshot = counters.get();
            snapshot.cache_hits = snapshot.cache_hits.saturating_add(1);
            counters.set(snapshot);
        });
        return s.clone();
    }
    let s = path_display_shared(path);
    cache.insert(path.clone(), s.clone());
    #[cfg(feature = "benchmarks")]
    PATH_DISPLAY_BENCH_COUNTERS.with(|counters| {
        let mut snapshot = counters.get();
        snapshot.cache_misses = snapshot.cache_misses.saturating_add(1);
        counters.set(snapshot);
    });
    s
}

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn bench_reset() {
    PATH_DISPLAY_BENCH_COUNTERS.with(|counters| counters.set(PathDisplayBenchCounters::default()));
}

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn bench_snapshot() -> PathDisplayBenchCounters {
    PATH_DISPLAY_BENCH_COUNTERS.with(std::cell::Cell::get)
}

#[cfg(windows)]
fn format_windows_path_for_display(mut path: String) -> String {
    if let Some(stripped) = path.strip_prefix(r"\\?\UNC\") {
        path = format!(r"\\{stripped}");
    } else if let Some(stripped) = path.strip_prefix(r"\\?\") {
        path = stripped.to_string();
    }
    path.replace('\\', "/")
}

#[cfg(not(windows))]
fn format_windows_path_for_display(path: String) -> String {
    path
}

#[cfg(test)]
mod tests {
    use super::format_windows_path_for_display;

    #[cfg(windows)]
    #[test]
    fn strips_verbatim_disk_prefix_and_uses_forward_slashes() {
        let formatted =
            format_windows_path_for_display(r"\\?\C:\Users\sanni\git\GitComet".to_string());
        assert_eq!(formatted, "C:/Users/sanni/git/GitComet");
    }

    #[cfg(windows)]
    #[test]
    fn strips_verbatim_unc_prefix_and_uses_forward_slashes() {
        let formatted = format_windows_path_for_display(r"\\?\UNC\server\share\repo".to_string());
        assert_eq!(formatted, "//server/share/repo");
    }

    #[cfg(not(windows))]
    #[test]
    fn leaves_non_windows_path_unchanged() {
        let formatted = format_windows_path_for_display("/tmp/repo".to_string());
        assert_eq!(formatted, "/tmp/repo");
    }
}
