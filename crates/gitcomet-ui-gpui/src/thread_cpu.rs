//! Per-thread CPU time from procfs, for the UI probe's attribution on Linux.
//! `schedstat` holds nanoseconds on CPU (exact, unlike the tick-quantized
//! `stat` utime/stime), nanoseconds spent runnable but waiting for a CPU, and
//! the number of times the thread was scheduled in, i.e. its wakeups plus
//! preemptions. Elsewhere these report nothing.

/// One thread's cumulative CPU time at a sample.
pub(crate) struct ThreadCpu {
    pub tid: u64,
    pub name: String,
    pub cpu_ns: u64,
    pub runqueue_wait_ns: u64,
    pub timeslices: u64,
}

/// Cumulative CPU time of one thread of this process.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn thread_cpu_ns(tid: u64) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        parse_schedstat(&std::fs::read_to_string(format!("/proc/self/task/{tid}/schedstat")).ok()?)
            .map(|stat| stat.0)
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
                let (cpu_ns, runqueue_wait_ns, timeslices) =
                    parse_schedstat(&std::fs::read_to_string(task.path().join("schedstat")).ok()?)?;
                let name = std::fs::read_to_string(task.path().join("comm")).ok()?;
                Some(ThreadCpu {
                    tid,
                    name: name.trim_end().to_owned(),
                    cpu_ns,
                    runqueue_wait_ns,
                    timeslices,
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
fn parse_schedstat(text: &str) -> Option<(u64, u64, u64)> {
    let mut fields = text.split_ascii_whitespace().map(str::parse::<u64>);
    Some((
        fields.next()?.ok()?,
        fields.next()?.ok()?,
        fields.next()?.ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedstat_first_field_is_the_cpu_time() {
        assert_eq!(parse_schedstat("27960 510 3\n"), Some((27_960, 510, 3)));
        assert_eq!(parse_schedstat(""), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_busy_thread_accumulates_cpu_time_under_its_own_name() {
        const SPIN_CPU_NS: u64 = 20_000_000;
        let (tid_tx, tid_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::Builder::new()
            .name("cpu-probe-busy".into())
            .spawn(move || {
                let tid = gitcomet_core::op_trace::current_os_tid().unwrap();
                // Spin on CPU time, not wall time: on a loaded runner 30 ms of
                // wall-clock spinning measured only 14 ms on CPU.
                let started = std::time::Instant::now();
                while thread_cpu_ns(tid).is_some_and(|ns| ns < SPIN_CPU_NS)
                    && started.elapsed() < std::time::Duration::from_secs(10)
                {
                    std::hint::black_box(0u64.wrapping_add(1));
                }
                tid_tx.send(tid).unwrap();
                let _ = stop_rx.recv();
            })
            .unwrap();
        let tid = tid_rx.recv().unwrap();
        let sample = sample_process_threads();
        let busy = sample
            .iter()
            .find(|thread| thread.tid == tid)
            .expect("worker listed");
        assert_eq!(busy.name, "cpu-probe-busy");
        assert!(busy.timeslices >= 1);
        assert!(
            busy.cpu_ns >= SPIN_CPU_NS,
            "spun to {SPIN_CPU_NS} ns on CPU, saw {} ns",
            busy.cpu_ns
        );
        assert_eq!(thread_cpu_ns(tid).map(|ns| ns >= busy.cpu_ns), Some(true));
        stop_tx.send(()).unwrap();
        worker.join().unwrap();
    }
}
