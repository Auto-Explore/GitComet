use std::io;
use std::os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::Child;
use std::ptr::null;
use std::time::Duration;
use windows_sys::Win32::Foundation::{FALSE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessW, INFINITE, PROCESS_INFORMATION, STARTUPINFOW,
    WaitForSingleObject,
};

fn finite_wait_millis(timeout: Duration) -> u32 {
    timeout.as_millis().min(u128::from(INFINITE - 1)) as u32
}

/// Wait for a child to exit, using its existing process handle. Returns `false`
/// when the timeout expires. This does not reap the child or retrieve its status.
///
/// Timeouts are rounded down to whole milliseconds and never become infinite.
/// Callers that need a sub-millisecond delay should use their usual sleep.
pub fn wait_for_process_exit(child: &Child, timeout: Duration) -> io::Result<bool> {
    let handle = child.as_handle();
    // SAFETY: Child owns this process handle with synchronization access. The
    // borrow prevents its owner from being dropped until the bounded wait ends.
    match unsafe { WaitForSingleObject(handle.as_raw_handle(), finite_wait_millis(timeout)) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        result => Err(io::Error::other(format!(
            "unexpected process wait result: {result}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};

    #[test]
    fn process_wait_timeouts_are_finite_and_never_rounded_up() {
        assert_eq!(finite_wait_millis(Duration::ZERO), 0);
        assert_eq!(finite_wait_millis(Duration::from_micros(999)), 0);
        assert_eq!(finite_wait_millis(Duration::from_millis(1)), 1);
        assert_eq!(finite_wait_millis(Duration::from_micros(1999)), 1);
        assert_eq!(finite_wait_millis(Duration::MAX), INFINITE - 1);
    }

    #[test]
    fn process_wait_observes_exit_without_consuming_status() {
        let mut child = Command::new("cmd.exe")
            .args(["/D", "/C", "exit 7"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let status = child.wait().unwrap();
        assert!(wait_for_process_exit(&child, Duration::from_secs(1)).unwrap());
        assert_eq!(child.try_wait().unwrap(), Some(status));
        assert_eq!(status.code(), Some(7));
    }

    #[test]
    fn process_wait_times_out_while_child_is_alive() {
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let result = wait_for_process_exit(&child, Duration::from_millis(1));
        let still_alive = child.try_wait().unwrap().is_none();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!result.unwrap());
        assert!(still_alive);
    }
}

/// Run Windows' process-tree termination helper without inheriting any handles.
/// In particular, asynchronous cleanup must not retain the caller's output pipes.
pub fn terminate_process_tree(process_id: u32) -> io::Result<()> {
    let mut command: Vec<u16> = format!("taskkill.exe /PID {process_id} /T /F\0")
        .encode_utf16()
        .collect();
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: the command is writable and NUL-terminated, both structures are
    // initialized, and no handles or external buffers are passed to the child.
    if unsafe {
        CreateProcessW(
            null(),
            command.as_mut_ptr(),
            null(),
            null(),
            FALSE,
            CREATE_NO_WINDOW,
            null(),
            null(),
            &startup,
            &mut process,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful CreateProcessW returns two distinct owned handles.
    let (process, thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(process.hProcess),
            OwnedHandle::from_raw_handle(process.hThread),
        )
    };
    drop(thread);
    // SAFETY: the process handle remains owned until the wait completes.
    if unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) } == WAIT_FAILED {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
