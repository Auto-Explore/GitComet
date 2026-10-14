"""Optional native collectors for the shared performance harness (stdlib only).

All memory fields carry their native meaning. Missing counters are None, never
zero. Linux PSS, Windows private bytes and macOS RSS are not interchangeable.
"""
import os
from functools import lru_cache
from pathlib import Path
import platform
import shutil
import signal
import stat
import subprocess
import time

import perf_macos


@lru_cache(maxsize=1)
def _windows_process_entry_type():
    """PROCESSENTRY32W's ABI: DWORD/LONG are 32-bit, WCHAR is UTF-16.

    ULONG_PTR alone follows pointer size. Fixed-width ctypes keep the layout
    testable on other hosts, where c_long or c_wchar need not match Windows.
    https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/ns-tlhelp32-processentry32w
    """
    import ctypes as c

    class ProcessEntry(c.Structure):
        _fields_ = [("dwSize", c.c_uint32), ("cntUsage", c.c_uint32),
                    ("th32ProcessID", c.c_uint32), ("th32DefaultHeapID", c.c_size_t),
                    ("th32ModuleID", c.c_uint32), ("cntThreads", c.c_uint32),
                    ("th32ParentProcessID", c.c_uint32), ("pcPriClassBase", c.c_int32),
                    ("dwFlags", c.c_uint32), ("szExeFile", c.c_uint16 * 260)]
    return ProcessEntry


def _windows_process_thread_count(pid, kernel, process_handle=None):
    """Read the PID's execution-thread count from one process snapshot.

    TH32CS_SNAPPROCESS includes cntThreads without enumerating every thread.
    The count describes the snapshot, not an atomic sample of all counters.
    When supplied, the existing process handle is checked before and after
    enumeration to reject an exited process, including a subsequently reused
    PID. Unavailable APIs, a missing PID or failed snapshot/enumeration/cleanup
    return None. Every valid snapshot is closed exactly once.
    https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/nf-tlhelp32-createtoolhelp32snapshot
    """
    import ctypes as c

    if isinstance(pid, bool) or not isinstance(pid, int) or not 0 < pid <= 0xffffffff:
        return None
    entry_type = _windows_process_entry_type()
    try:
        create = kernel.CreateToolhelp32Snapshot
        first, next_entry = kernel.Process32FirstW, kernel.Process32NextW
        close = kernel.CloseHandle
        create.argtypes, create.restype = [c.c_uint32, c.c_uint32], c.c_void_p
        for function in (first, next_entry):
            function.argtypes, function.restype = [c.c_void_p, c.POINTER(entry_type)], c.c_int32
        close.argtypes, close.restype = [c.c_void_p], c.c_int32
        if process_handle is not None:
            get_exit = kernel.GetExitCodeProcess
            get_exit.argtypes, get_exit.restype = [c.c_void_p, c.POINTER(c.c_uint32)], c.c_int32

        def alive():
            if process_handle is None:
                return True
            code = c.c_uint32()
            return bool(get_exit(process_handle, c.byref(code))) and code.value == 259  # STILL_ACTIVE

        if not alive():
            return None
        snapshot = create(0x2, 0)  # TH32CS_SNAPPROCESS; PID argument is ignored.
    except (AttributeError, OSError):
        return None
    if not snapshot or snapshot == c.c_void_p(-1).value:
        return None
    count = None
    try:
        entry = entry_type()
        entry.dwSize = c.sizeof(entry)
        present = first(snapshot, c.byref(entry))
        while present:
            if entry.th32ProcessID == pid:
                count = int(entry.cntThreads) if entry.cntThreads else None
                break
            present = next_entry(snapshot, c.byref(entry))
        if not alive():
            count = None
    except OSError:
        count = None
    finally:
        try:
            if not close(snapshot):
                count = None
        except OSError:
            count = None
    return count


def windows_sample(pid):
    """Query the process directly; starting PowerShell per sample perturbs it."""
    import ctypes as c
    import ctypes.wintypes as w
    kernel = c.WinDLL("kernel32", use_last_error=True)
    psapi = c.WinDLL("psapi", use_last_error=True)
    kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
    kernel.OpenProcess.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    kernel.GetProcessTimes.argtypes = [w.HANDLE, *([c.POINTER(w.FILETIME)] * 4)]
    kernel.GetProcessHandleCount.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]
    class Memory(c.Structure):
        _fields_ = [("cb", w.DWORD), ("faults", w.DWORD)] + [(name, c.c_size_t) for name in
            ("peak_rss", "rss", "peak_paged", "paged", "peak_nonpaged", "nonpaged", "pagefile", "peak_pagefile", "private")]
    psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Memory), w.DWORD]
    handle = kernel.OpenProcess(0x0400 | 0x0010, False, pid)
    if not handle:
        return None
    try:
        times = [w.FILETIME() for _ in range(4)]
        memory, handles = Memory(), w.DWORD()
        memory.cb = c.sizeof(memory)
        values = {}
        if kernel.GetProcessTimes(handle, *[c.byref(t) for t in times]):
            values["cpu_s"] = sum((t.dwHighDateTime << 32) | t.dwLowDateTime for t in times[2:]) / 1e7
        if psapi.GetProcessMemoryInfo(handle, c.byref(memory), memory.cb):
            values.update(rss_kib=memory.rss / 1024, private_kib=memory.private / 1024)
        if kernel.GetProcessHandleCount(handle, c.byref(handles)):
            values["handles"] = handles.value
        values["threads"] = _windows_process_thread_count(pid, kernel, handle)
        return values
    finally:
        kernel.CloseHandle(handle)


FIELDS = ("rss_kib", "pss_kib", "private_kib", "threads", "fds", "handles",
          "cpu_s", "children_cpu_s", "voluntary_switches", "read_bytes", "write_bytes")


def capabilities():
    system = platform.system()
    return {"os": system, "native_validation": "run_on_this_host",
            "process_sampler": {"Linux": "procfs", "Windows": "win32",
                                "Darwin": "proc_pid_rusage" if perf_macos.available() else "ps"}.get(system),
            "pss": system == "Linux", "private_bytes": system == "Windows",
            "per_thread_cpu": system == "Linux", "main_thread_cpu": system in ("Linux", "Windows"),
            "display_completion": False,
            "profilers": {tool: shutil.which(tool) for tool in
                          ("perf", "strace", "heaptrack", "wpr", "xctrace", "sample")}}


def load_average():
    try:
        return list(os.getloadavg())
    except (AttributeError, OSError):
        return None


def stop_tree(process):
    if os.name == "nt":
        if process.poll() is not None:
            return
        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            # The process group may have already exited; nothing remains to kill.
            pass
    process.wait(timeout=15)


def remove_owned_tree(path):
    """Annex intentionally makes its owned content directories read-only."""
    def writable_retry(function, name, error):
        if not isinstance(error[1], PermissionError):
            raise error[1]
        item = Path(name)
        parent = item.parent
        parent.chmod(stat.S_IMODE(parent.stat().st_mode) | stat.S_IWUSR | stat.S_IXUSR)
        if not item.is_symlink():
            mode = stat.S_IMODE(item.stat().st_mode) | stat.S_IWUSR | stat.S_IRUSR
            item.chmod(mode | (stat.S_IXUSR if item.is_dir() else 0))
        function(name)
    shutil.rmtree(path, onerror=writable_retry)


def sample_process(pid):
    result = dict.fromkeys(FIELDS)
    result.update(unix_ms=time.time() * 1000, pid=pid)
    try:
        if platform.system() == "Darwin" and perf_macos.available():
            sample = perf_macos.process_sample(pid)
            if sample is None:
                return None
            result.update(sample)
        elif platform.system() == "Darwin":
            row = subprocess.run(["ps", "-p", str(pid), "-o", "rss=,time="],
                                 capture_output=True, text=True, timeout=3, check=True).stdout.split()
            if len(row) < 2:
                return None
            # BSD ps time is [[days-]hours:]minutes:seconds.
            days, clock = (row[1].split("-", 1) if "-" in row[1] else ("0", row[1]))
            seconds = 0.0
            for part in clock.split(":"):
                seconds = seconds * 60 + float(part)
            result.update(rss_kib=int(row[0]), cpu_s=seconds + int(days) * 86400)
        elif os.name == "nt":
            values = windows_sample(pid)
            if values is None:
                return None
            result.update(values)
        else:
            return None  # Linux uses the more detailed existing procfs collector.
    except (OSError, subprocess.SubprocessError, ValueError):
        return None
    return result


def profile_directories(output):
    """Use application-supported paths without replacing the user's HOME."""
    profile = Path(output) / "profile"
    dirs = {name: profile / name for name in ("config", "data", "state", "cache")}
    for directory in dirs.values():
        directory.mkdir(parents=True, exist_ok=True)
    return {"GITCOMET_PROFILE_ROOT": str(profile),
            "XDG_CONFIG_HOME": str(dirs["config"]), "XDG_DATA_HOME": str(dirs["data"]),
            "XDG_STATE_HOME": str(dirs["state"]), "XDG_CACHE_HOME": str(dirs["cache"]),
            "LOCALAPPDATA": str(dirs["data"]), "APPDATA": str(dirs["config"])}
