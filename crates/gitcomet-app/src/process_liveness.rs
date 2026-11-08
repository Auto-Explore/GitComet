// Unknown counts as running: callers only act on a process proven gone.
pub(crate) fn process_is_running(pid: u32) -> bool {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = match sysinfo::System::new() {
        Ok(system) => system,
        Err(err) => {
            eprintln!("Failed to check whether process {pid} is running: {err}");
            return true;
        }
    };
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    system.process(pid).is_some()
}
