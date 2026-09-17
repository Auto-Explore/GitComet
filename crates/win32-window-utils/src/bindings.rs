windows_link::link!("user32.dll" "system" fn ClientToScreen(hwnd : HWND, lppoint : *mut POINT) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn CloseHandle(hobject : HANDLE) -> BOOL);
windows_link::link!("user32.dll" "system" fn EnableMenuItem(hmenu : HMENU, uidenableitem : u32, uenable : u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn ExitProcess(uexitcode : u32) -> !);
windows_link::link!("kernel32.dll" "system" fn GenerateConsoleCtrlEvent(dwctrlevent : u32, dwprocessgroupid : u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GetCurrentProcess() -> HANDLE);
windows_link::link!("kernel32.dll" "system" fn GetCurrentThreadId() -> u32);
windows_link::link!("user32.dll" "system" fn GetSystemMenu(hwnd : HWND, brevert : BOOL) -> HMENU);
windows_link::link!("kernel32.dll" "system" fn GetThreadTimes(hthread : HANDLE, lpcreationtime : *mut FILETIME, lpexittime : *mut FILETIME, lpkerneltime : *mut FILETIME, lpusertime : *mut FILETIME) -> BOOL);
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
windows_link::link!("user32.dll" "system" fn GetWindowLongPtrW(hwnd : HWND, nindex : i32) -> isize);
#[cfg(target_pointer_width = "32")]
pub use GetWindowLongW as GetWindowLongPtrW;
windows_link::link!("user32.dll" "system" fn GetWindowLongW(hwnd : HWND, nindex : i32) -> i32);
windows_link::link!("user32.dll" "system" fn IsIconic(hwnd : HWND) -> BOOL);
windows_link::link!("user32.dll" "system" fn IsZoomed(hwnd : HWND) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn OpenThread(dwdesiredaccess : u32, binherithandle : BOOL, dwthreadid : u32) -> HANDLE);
windows_link::link!("user32.dll" "system" fn PostMessageW(hwnd : HWND, msg : u32, wparam : WPARAM, lparam : LPARAM) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn SetConsoleCtrlHandler(handlerroutine : PHANDLER_ROUTINE, add : BOOL) -> BOOL);
windows_link::link!("user32.dll" "system" fn SetForegroundWindow(hwnd : HWND) -> BOOL);
windows_link::link!("user32.dll" "system" fn ShowWindowAsync(hwnd : HWND, ncmdshow : i32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn TerminateProcess(hprocess : HANDLE, uexitcode : u32) -> BOOL);
windows_link::link!("user32.dll" "system" fn TrackPopupMenuEx(hmenu : HMENU, uflags : u32, x : i32, y : i32, hwnd : HWND, lptpm : *const TPMPARAMS) -> BOOL);
pub type BOOL = i32;
pub const CTRL_BREAK_EVENT: i32 = 1;
pub const CTRL_CLOSE_EVENT: i32 = 2;
pub const CTRL_C_EVENT: i32 = 0;
pub const CTRL_LOGOFF_EVENT: i32 = 5;
pub const CTRL_SHUTDOWN_EVENT: i32 = 6;
pub const FALSE: i32 = 0;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FILETIME {
    pub dwLowDateTime: u32,
    pub dwHighDateTime: u32,
}
pub const GWL_STYLE: i32 = -16;
pub type HANDLE = *mut core::ffi::c_void;
pub type HMENU = *mut core::ffi::c_void;
pub type HWND = *mut core::ffi::c_void;
pub type LPARAM = isize;
pub const MF_BYCOMMAND: i32 = 0;
pub const MF_ENABLED: i32 = 0;
pub const MF_GRAYED: i32 = 1;
pub type PHANDLER_ROUTINE = Option<unsafe extern "system" fn(ctrltype: u32) -> BOOL>;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
pub const SC_CLOSE: i32 = 61536;
pub const SC_MAXIMIZE: i32 = 61488;
pub const SC_MINIMIZE: i32 = 61472;
pub const SC_MOVE: i32 = 61456;
pub const SC_RESTORE: i32 = 61728;
pub const SC_SIZE: i32 = 61440;
pub const SW_RESTORE: i32 = 9;
pub const THREAD_QUERY_LIMITED_INFORMATION: i32 = 2048;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TPMPARAMS {
    pub cbSize: u32,
    pub rcExclude: RECT,
}
pub const TPM_LEFTALIGN: i32 = 0;
pub const TPM_RETURNCMD: i32 = 256;
pub const TPM_RIGHTBUTTON: i32 = 2;
pub const TPM_TOPALIGN: i32 = 0;
pub const TRUE: i32 = 1;
pub const WM_NULL: i32 = 0;
pub const WM_SYSCOMMAND: i32 = 274;
pub type WPARAM = usize;
pub const WS_MAXIMIZEBOX: i32 = 65536;
pub const WS_MINIMIZEBOX: i32 = 131072;
pub const WS_SYSMENU: i32 = 524288;
pub const WS_THICKFRAME: i32 = 262144;
