use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null;
use windows_sys::Win32::Foundation::{FALSE, WAIT_FAILED};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessW, INFINITE, PROCESS_INFORMATION, STARTUPINFOW,
    WaitForSingleObject,
};

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
