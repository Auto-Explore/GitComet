//! Windows accounting shared by linker replay, Cargo, and the linker proxy.
//! All native handles are owned here; children enter the job before execution.
use crate::{Result, RunSpec, hash_bytes, response, save_json};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Write},
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{null, null_mut},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::*,
    System::{
        Diagnostics::ToolHelp::*, IO::*, JobObjects::*, ProcessStatus::*, SystemInformation::*,
        Threading::*,
    },
};

struct Handle(HANDLE);
impl Handle {
    fn new(raw: HANDLE) -> Result<Self> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error().into())
        } else {
            Ok(Self(raw))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn wide(path: &Path) -> Result<Vec<u16>> {
    let mut result: Vec<u16> = path.as_os_str().encode_wide().collect();
    if result.contains(&0) {
        return Err("embedded NUL in path".into());
    }
    result.push(0);
    Ok(result)
}
fn checked(value: i32) -> Result<()> {
    if value == 0 {
        Err(io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}
fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

fn inheritable_file(path: &Path, read: bool) -> Result<Handle> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    unsafe {
        Handle::new(CreateFileW(
            wide(path)?.as_ptr(),
            if read { GENERIC_READ } else { GENERIC_WRITE },
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &attributes,
            if read { OPEN_EXISTING } else { CREATE_ALWAYS },
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        ))
    }
}

fn job_query<T: Default>(job: HANDLE, class: JOBOBJECTINFOCLASS) -> Result<T> {
    let mut value = T::default();
    unsafe {
        checked(QueryInformationJobObject(
            job,
            class,
            (&mut value as *mut T).cast(),
            size_of::<T>() as u32,
            null_mut(),
        ))?;
    }
    Ok(value)
}

fn process_times(process: HANDLE) -> Result<[u64; 4]> {
    let (mut created, mut exited, mut kernel, mut user) =
        unsafe { (zeroed(), zeroed(), zeroed(), zeroed()) };
    unsafe {
        checked(GetProcessTimes(
            process,
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        ))?;
    }
    Ok([ticks(created), ticks(exited), ticks(kernel), ticks(user)])
}
fn system_times() -> Option<[u64; 3]> {
    let (mut idle, mut kernel, mut user) = unsafe { (zeroed(), zeroed(), zeroed()) };
    unsafe {
        if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
            return None;
        }
    }
    Some([ticks(idle), ticks(kernel), ticks(user)])
}
fn memory(process: HANDLE) -> Option<PROCESS_MEMORY_COUNTERS_EX> {
    let mut value = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    unsafe {
        if K32GetProcessMemoryInfo(
            process,
            (&mut value as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            value.cb,
        ) == 0
        {
            return None;
        }
    }
    Some(value)
}
fn thread_count(pid: u32) -> Option<u32> {
    let snapshot = unsafe { Handle::new(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)).ok()? };
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut count = 0;
    unsafe {
        if Thread32First(snapshot.0, &mut entry) == 0 {
            return None;
        }
        loop {
            if entry.th32OwnerProcessID == pid {
                count += 1;
            }
            if Thread32Next(snapshot.0, &mut entry) == 0 {
                break;
            }
        }
    }
    Some(count)
}
fn image_name(pid: u32) -> Option<String> {
    let process =
        unsafe { Handle::new(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid)).ok()? };
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    unsafe {
        if QueryFullProcessImageNameW(process.0, 0, buffer.as_mut_ptr(), &mut length) == 0 {
            return None;
        }
    }
    Some(String::from_utf16_lossy(&buffer[..length as usize]))
}
fn drain_events(port: HANDLE, start: Instant, events: &mut Vec<Value>) {
    loop {
        let (mut message, mut key, mut overlapped) = (0, 0, null_mut());
        unsafe {
            if GetQueuedCompletionStatus(port, &mut message, &mut key, &mut overlapped, 0) == 0 {
                break;
            }
        }
        let pid = overlapped as usize as u32;
        events.push(
            json!({"elapsed_s": start.elapsed().as_secs_f64(), "message": message,
            "pid": pid, "image": if message == 6 { image_name(pid) } else { None }}),
        );
    }
}
fn io_json(value: &IO_COUNTERS) -> Value {
    json!({"read_bytes": value.ReadTransferCount, "write_bytes": value.WriteTransferCount,
        "other_bytes": value.OtherTransferCount, "read_operations": value.ReadOperationCount,
        "write_operations": value.WriteOperationCount, "other_operations": value.OtherOperationCount})
}

fn collect(spec: &RunSpec) -> Result<Value> {
    if spec.timeout_s == 0 {
        return Err("timeout_s must be positive".into());
    }
    let stdout = inheritable_file(&spec.output_dir.join("stdout.log"), false)?;
    let stderr = inheritable_file(&spec.output_dir.join("stderr.log"), false)?;
    let stdin = inheritable_file(Path::new("NUL"), true)?;
    let job = unsafe { Handle::new(CreateJobObjectW(null(), null()))? };
    let port = unsafe {
        Handle::new(CreateIoCompletionPort(
            INVALID_HANDLE_VALUE,
            null_mut(),
            0,
            1,
        ))?
    };
    let association = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
        CompletionKey: std::ptr::dangling_mut(),
        CompletionPort: port.0,
    };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        checked(SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of_val(&limits) as u32,
        ))?;
        checked(SetInformationJobObject(
            job.0,
            JobObjectAssociateCompletionPortInformation,
            (&association as *const JOBOBJECT_ASSOCIATE_COMPLETION_PORT).cast(),
            size_of_val(&association) as u32,
        ))?;
    }
    let executable = spec
        .executable
        .to_str()
        .ok_or("non-Unicode executable path")?;
    let command = std::iter::once(executable)
        .chain(spec.arguments.iter().map(String::as_str))
        .map(response::quote)
        .collect::<Vec<_>>()
        .join(" ");
    if command.contains('\0') {
        return Err("embedded NUL in command".into());
    }
    let mut command_wide: Vec<u16> = command.encode_utf16().chain([0]).collect();
    let executable_wide = wide(&spec.executable)?;
    let cwd_wide = wide(&spec.cwd)?;
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: stdin.0,
        hStdOutput: stdout.0,
        hStdError: stderr.0,
        ..Default::default()
    };
    let mut information = PROCESS_INFORMATION::default();
    let mut samples = Vec::new();
    let mut events = Vec::new();
    let system_before = system_times();
    let started_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let start = Instant::now(); // Windows Instant uses QueryPerformanceCounter.
    unsafe {
        checked(CreateProcessW(
            executable_wide.as_ptr(),
            command_wide.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED | CREATE_NO_WINDOW,
            null(),
            cwd_wide.as_ptr(),
            &startup,
            &mut information,
        ))?;
    }
    let process = Handle::new(information.hProcess)?;
    let thread = Handle::new(information.hThread)?;
    unsafe {
        if AssignProcessToJobObject(job.0, process.0) == 0 {
            let error = io::Error::last_os_error();
            TerminateProcess(process.0, 125);
            return Err(format!("AssignProcessToJobObject failed: {error}").into());
        }
    }
    // A dedicated waiter prevents memory/thread sampling from delaying the
    // primary timestamp. Own a duplicate handle so error paths remain safe.
    let mut waiter_handle = null_mut();
    unsafe {
        checked(DuplicateHandle(
            GetCurrentProcess(),
            process.0,
            GetCurrentProcess(),
            &mut waiter_handle,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        ))?;
    }
    let waiter_raw = waiter_handle as usize;
    let root_waiter = std::thread::spawn(move || {
        let owned = Handle(waiter_raw as HANDLE);
        let status = unsafe { WaitForSingleObject(owned.0, INFINITE) };
        (status, start.elapsed().as_secs_f64())
    });
    unsafe {
        if ResumeThread(thread.0) == u32::MAX {
            return Err(io::Error::last_os_error().into());
        }
    }
    let (mut affinity, mut system_affinity) = (0, 0);
    unsafe {
        checked(GetProcessAffinityMask(
            process.0,
            &mut affinity,
            &mut system_affinity,
        ))?;
    }
    let logical_cpus = affinity.count_ones();
    let priority_class = unsafe { GetPriorityClass(process.0) };
    let mut root_wall = None;
    let mut timed_out = false;
    let mut lingering_processes = 0;
    let mut tree_complete_before_cleanup = true;
    let poll_ms = if spec.sample_ms == 0 {
        100
    } else {
        spec.sample_ms
    };
    let mut next_sample = Duration::ZERO;
    let mut peak_ws: Option<usize> = None;
    let mut peak_private: Option<usize> = None;
    let mut peak_threads: Option<u32> = None;
    let mut memory_failures = 0;
    loop {
        let elapsed = start.elapsed();
        if elapsed.as_secs() >= spec.timeout_s && !timed_out {
            unsafe {
                checked(TerminateJobObject(job.0, 124))?;
            }
            timed_out = true;
        }
        if spec.sample_ms > 0 && elapsed >= next_sample && root_wall.is_none() {
            let mem = memory(process.0);
            if mem.is_none() {
                memory_failures += 1;
            }
            let threads = thread_count(information.dwProcessId);
            if let Some(m) = &mem {
                peak_ws = Some(peak_ws.unwrap_or(0).max(m.PeakWorkingSetSize));
                peak_private = Some(peak_private.unwrap_or(0).max(m.PrivateUsage));
            }
            if let Some(t) = threads {
                peak_threads = Some(peak_threads.unwrap_or(0).max(t));
            }
            let mut global = MEMORYSTATUSEX {
                dwLength: size_of::<MEMORYSTATUSEX>() as u32,
                ..Default::default()
            };
            let available =
                unsafe { (GlobalMemoryStatusEx(&mut global) != 0).then_some(global.ullAvailPhys) };
            let times = process_times(process.0).ok();
            samples.push(json!({"elapsed_s": elapsed.as_secs_f64(),
                "working_set_bytes": mem.as_ref().map(|m| m.WorkingSetSize),
                "private_commit_bytes": mem.as_ref().map(|m| m.PrivateUsage),
                "page_faults": mem.as_ref().map(|m| m.PageFaultCount), "threads": threads,
                "cpu_user_s": times.map(|t| t[3] as f64 / 1e7), "cpu_kernel_s": times.map(|t| t[2] as f64 / 1e7),
                "system_available_memory_bytes": available, "system_cpu_ticks": system_times()}));
            next_sample = start.elapsed() + Duration::from_millis(u64::from(spec.sample_ms));
        }
        drain_events(port.0, start, &mut events);
        let accounting: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION =
            job_query(job.0, JobObjectBasicAndIoAccountingInformation)?;
        if accounting.BasicInfo.ActiveProcesses == 0 {
            break;
        }
        // Toolsets can launch persistent services (for example vctip.exe).
        // Main-process latency stays independent of these services. Bound the
        // drain, record it, and clean up only descendants in this private job.
        if root_wall.is_some_and(|ended| start.elapsed().as_secs_f64() - ended > 0.010)
            && lingering_processes == 0
        {
            lingering_processes = accounting.BasicInfo.ActiveProcesses;
            tree_complete_before_cleanup = false;
            unsafe {
                checked(TerminateJobObject(job.0, 0))?;
            }
        }
        if root_wall.is_none() {
            let wait = unsafe { WaitForSingleObject(process.0, poll_ms) };
            if wait == WAIT_OBJECT_0 {
                root_wall = Some(start.elapsed().as_secs_f64());
            } else if wait == WAIT_FAILED {
                return Err(io::Error::last_os_error().into());
            }
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let tree_wall = start.elapsed().as_secs_f64();
    let (wait_status, root_wall) = root_waiter.join().map_err(|_| "process waiter panicked")?;
    if wait_status != WAIT_OBJECT_0 {
        return Err("process waiter failed".into());
    }
    let system_after = system_times();
    let times = process_times(process.0)?;
    let accounting: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION =
        job_query(job.0, JobObjectBasicAndIoAccountingInformation)?;
    let peaks: JOBOBJECT_EXTENDED_LIMIT_INFORMATION =
        job_query(job.0, JobObjectExtendedLimitInformation)?;
    let mut root_io = IO_COUNTERS::default();
    let root_io =
        unsafe { (GetProcessIoCounters(process.0, &mut root_io) != 0).then_some(root_io) };
    let mut exit = 125;
    unsafe {
        checked(GetExitCodeProcess(process.0, &mut exit))?;
    }
    drain_events(port.0, start, &mut events);
    let tree_cpu =
        (accounting.BasicInfo.TotalUserTime + accounting.BasicInfo.TotalKernelTime) as f64 / 1e7;
    let system_busy = system_before
        .zip(system_after)
        .map(|(a, b)| ((b[1] - a[1]) + (b[2] - a[2]) - (b[0] - a[0])) as f64 / 1e7);
    let mut csv = io::BufWriter::new(fs::File::create(spec.output_dir.join("samples.csv"))?);
    writeln!(
        csv,
        "elapsed_s,working_set_bytes,private_commit_bytes,page_faults,threads,cpu_user_s,cpu_kernel_s,system_available_memory_bytes"
    )?;
    for sample in &samples {
        let fields = [
            "elapsed_s",
            "working_set_bytes",
            "private_commit_bytes",
            "page_faults",
            "threads",
            "cpu_user_s",
            "cpu_kernel_s",
            "system_available_memory_bytes",
        ];
        writeln!(
            csv,
            "{}",
            fields
                .iter()
                .map(|k| if sample[*k].is_null() {
                    String::new()
                } else {
                    sample[*k].to_string()
                })
                .collect::<Vec<_>>()
                .join(",")
        )?;
    }
    csv.flush()?;
    save_json(&spec.output_dir.join("samples.json"), &json!(samples))?;
    save_json(&spec.output_dir.join("process-events.json"), &json!(events))?;
    Ok(json!({
        "schema_version": 1, "valid": !timed_out, "exit_code": exit, "timed_out": timed_out,
        "started_unix_ms": started_unix_ms, "pid": information.dwProcessId,
        "wall_s": root_wall, "tree_wall_s": tree_wall, "process_lifetime_s": (times[1] - times[0]) as f64 / 1e7,
        "process_created_filetime": times[0], "process_exited_filetime": times[1],
        "cpu_user_s": times[3] as f64 / 1e7, "cpu_kernel_s": times[2] as f64 / 1e7,
        "tree_cpu_user_s": accounting.BasicInfo.TotalUserTime as f64 / 1e7,
        "tree_cpu_kernel_s": accounting.BasicInfo.TotalKernelTime as f64 / 1e7,
        "tree_cpu_s": tree_cpu, "average_logical_cpus": tree_cpu / tree_wall,
        "allowed_logical_cpus": logical_cpus, "average_machine_cpu_percent": 100.0 * tree_cpu / tree_wall / f64::from(logical_cpus),
        "affinity_mask": format!("{affinity:x}"), "priority_class": priority_class,
        "peak_tree_commit_bytes": peaks.PeakJobMemoryUsed, "peak_single_process_commit_bytes": peaks.PeakProcessMemoryUsed,
        "observed_peak_working_set_bytes": peak_ws, "sampled_peak_private_commit_bytes": peak_private,
        "observed_peak_threads": peak_threads, "tree_page_faults": accounting.BasicInfo.TotalPageFaultCount,
        "tree_process_count": accounting.BasicInfo.TotalProcesses,
        "tree_complete_before_cleanup": tree_complete_before_cleanup,
        "lingering_processes_terminated": lingering_processes,
        "post_exit_grace_ms": 10,
        "root_io": root_io.as_ref().map(io_json), "tree_io": io_json(&accounting.IoInfo),
        "system_busy_cpu_s": system_busy, "estimated_other_cpu_s": system_busy.map(|s| (s - tree_cpu).max(0.0)),
        "sample_count": samples.len(), "memory_sample_failures": memory_failures,
        "availability": {"resident_peak": if spec.sample_ms == 0 { "sampling disabled" } else { "observed high-water mark; final peak may be missed" },
            "physical_disk_io": "requires separate ETW capture", "hard_faults": "requires separate ETW capture",
            "instructions_energy_temperature": "not collected", "process_names": "best effort from job notifications"},
    }))
}

pub fn run(spec: &RunSpec, wrapper_start: Instant) -> Result<u32> {
    fs::create_dir_all(&spec.output_dir)?;
    if spec.output_dir.join("metrics.json").exists() {
        return Err("result already exists; refuse to overwrite".into());
    }
    let command =
        json!({"executable": spec.executable, "arguments": spec.arguments, "cwd": spec.cwd});
    save_json(&spec.output_dir.join("command.json"), &command)?;
    let mut result = match collect(spec) {
        Ok(value) => value,
        Err(error) => {
            json!({"schema_version": 1, "valid": false, "exit_code": 125, "collector_error": error.to_string()})
        }
    };
    result["metadata"] = spec.metadata.clone();
    result["sample_ms"] = json!(spec.sample_ms);
    result["command_sha256"] = json!(hash_bytes(&serde_json::to_vec(&command)?));
    result["output_dir"] = json!(spec.output_dir);
    result["wrapper_elapsed_before_result_write_s"] = json!(wrapper_start.elapsed().as_secs_f64());
    save_json(&spec.output_dir.join("metrics.json"), &result)?;
    if let Some(error) = result.get("collector_error") {
        eprintln!("linker-metrics: {error}");
    }
    Ok(result["exit_code"].as_u64().unwrap_or(125) as u32)
}
