"""Optional native collectors for the shared performance harness (stdlib only).

All memory fields carry their native meaning. Missing counters are None, never
zero. Linux PSS, Windows private bytes and macOS RSS are not interchangeable.
"""
import os
from pathlib import Path
import platform
import shutil
import signal
import stat
import subprocess
import time


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
        return values
    finally:
        kernel.CloseHandle(handle)


FIELDS = ("rss_kib", "pss_kib", "private_kib", "threads", "fds", "handles",
          "cpu_s", "children_cpu_s", "voluntary_switches", "read_bytes", "write_bytes")


def capabilities():
    system = platform.system()
    return {"os": system, "native_validation": "run_on_this_host",
            "process_sampler": {"Linux": "procfs", "Windows": "win32",
                                "Darwin": "ps"}.get(system),
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
        if platform.system() == "Darwin":
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
