//! Per-thread CPU time from procfs, for the UI probe's attribution on Linux.
//! `schedstat`'s first field is nanoseconds on CPU: exact, unlike the
//! tick-quantized `stat` utime/stime. Elsewhere these report nothing.

/// One thread's cumulative CPU time at a sample.
pub(crate) struct ThreadCpu {
    pub tid: u64,
    pub name: String,
    pub cpu_ns: u64,
}

/// The calling thread's kernel id.
pub(crate) fn current_tid() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let link = std::fs::read_link("/proc/thread-self").ok()?;
        link.file_name()?.to_str()?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Cumulative CPU time of one thread of this process.
pub(crate) fn thread_cpu_ns(tid: u64) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        parse_schedstat(&std::fs::read_to_string(format!("/proc/self/task/{tid}/schedstat")).ok()?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = tid;
        None
    }
}

/// Every live thread of this process with its name and CPU time.
pub(crate) fn sample_process_threads() -> Vec<ThreadCpu> {
    #[cfg(target_os = "linux")]
    {
        let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
            return Vec::new();
        };
        tasks
            .filter_map(|task| {
                let task = task.ok()?;
                let tid = task.file_name().to_str()?.parse().ok()?;
                // A thread can exit between listing and reading; skip it.
                let cpu_ns =
                    parse_schedstat(&std::fs::read_to_string(task.path().join("schedstat")).ok()?)?;
                let name = std::fs::read_to_string(task.path().join("comm")).ok()?;
                Some(ThreadCpu {
                    tid,
                    name: name.trim_end().to_owned(),
                    cpu_ns,
                })
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_schedstat(text: &str) -> Option<u64> {
    text.split_ascii_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedstat_first_field_is_the_cpu_time() {
        assert_eq!(parse_schedstat("27960 0 1\n"), Some(27_960));
        assert_eq!(parse_schedstat(""), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_busy_thread_accumulates_cpu_time_under_its_own_name() {
        let (tid_tx, tid_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::Builder::new()
            .name("cpu-probe-busy".into())
            .spawn(move || {
                tid_tx.send(current_tid().unwrap()).unwrap();
                let started = std::time::Instant::now();
                while started.elapsed() < std::time::Duration::from_millis(30) {
                    std::hint::black_box(0u64.wrapping_add(1));
                }
                stop_rx.recv().unwrap();
            })
            .unwrap();
        let tid = tid_rx.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(40));
        let sample = sample_process_threads();
        let busy = sample
            .iter()
            .find(|thread| thread.tid == tid)
            .expect("worker listed");
        assert_eq!(busy.name, "cpu-probe-busy");
        assert!(
            busy.cpu_ns >= 20_000_000,
            "spun ~30 ms, saw {} ns",
            busy.cpu_ns
        );
        assert_eq!(thread_cpu_ns(tid).map(|ns| ns >= busy.cpu_ns), Some(true));
        stop_tx.send(()).unwrap();
        worker.join().unwrap();
    }
}
