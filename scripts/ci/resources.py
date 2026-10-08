"""Optional process-tree diagnostics. Sampled peaks/CPU are lower bounds.

Short-lived children may escape sampling. RSS sums shared pages per process;
this is not PSS. Instrumented executions never qualify as acceptance samples.
"""

import os
from collections import Counter
from pathlib import Path
import subprocess
import sys
import threading


def linux_processes():
    ticks = os.sysconf("SC_CLK_TCK")
    page = os.sysconf("SC_PAGE_SIZE")
    rows = {}
    for path in Path("/proc").iterdir():
        if not path.name.isdecimal():
            continue
        try:
            # comm can contain whitespace and parentheses; fields start after it.
            fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
            rows[int(path.name)] = dict(parent=int(fields[1]), identity=fields[19],
                                        rss=int(fields[21]) * page, threads=int(fields[17]),
                                        cpu=(int(fields[11]) + int(fields[12])) / ticks)
        except (OSError, ValueError, IndexError):  # Process exited during snapshot.
            continue
    return rows


def clock_seconds(value):
    days, clock = value.split("-", 1) if "-" in value else ("0", value)
    seconds = 0.0
    for part in clock.split(":"):
        seconds = seconds * 60 + float(part)
    return seconds + int(days) * 86400


def macos_processes():
    output = subprocess.check_output(["ps", "-axo", "pid=,ppid=,rss=,time=,lstart="],
                                     text=True, timeout=3)
    rows = {}
    for line in output.splitlines():
        try:
            pid, parent, rss, cpu, identity = line.strip().split(None, 4)
            rows[int(pid)] = dict(parent=int(parent), identity=identity, rss=int(rss) * 1024,
                                  threads=None, cpu=clock_seconds(cpu))
        except ValueError:
            continue
    # Apple's ps enumerates Mach threads with -M. Some releases enforce an
    # entitlement and print one process row instead: probe this multithreaded
    # sampler process before treating those rows as thread counts.
    # https://github.com/apple-oss-distributions/adv_cmds/blob/main/ps/ps.c
    output = subprocess.check_output(["ps", "-M", "-axo", "pid="], text=True, timeout=3)
    threads = Counter(int(pid) for pid in output.split() if pid.isdecimal())
    if threads[os.getpid()] >= 2:
        for pid, row in rows.items():
            row["threads"] = threads.get(pid)  # Missing task access is unavailable.
    return rows


def windows_processes():
    import ctypes as c
    from ctypes import wintypes as w

    class Entry(c.Structure):
        _fields_ = [("size", w.DWORD), ("usage", w.DWORD), ("pid", w.DWORD),
                    ("heap", c.c_size_t), ("module", w.DWORD), ("threads", w.DWORD),
                    ("parent", w.DWORD), ("priority", w.LONG), ("flags", w.DWORD),
                    ("exe", w.WCHAR * 260)]

    class Memory(c.Structure):
        _fields_ = [("size", w.DWORD), ("faults", w.DWORD)] + [
            (name, c.c_size_t) for name in ("peak_rss", "rss", "peak_paged", "paged",
                                          "peak_nonpaged", "nonpaged", "pagefile", "peak_pagefile")]

    kernel, psapi = c.WinDLL("kernel32", use_last_error=True), c.WinDLL("psapi", use_last_error=True)
    kernel.CreateToolhelp32Snapshot.argtypes = [w.DWORD, w.DWORD]
    kernel.CreateToolhelp32Snapshot.restype = w.HANDLE
    kernel.Process32FirstW.argtypes = kernel.Process32NextW.argtypes = [w.HANDLE, c.POINTER(Entry)]
    kernel.OpenProcess.argtypes, kernel.OpenProcess.restype = [w.DWORD, w.BOOL, w.DWORD], w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    kernel.GetProcessTimes.argtypes = [w.HANDLE, *([c.POINTER(w.FILETIME)] * 4)]
    psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Memory), w.DWORD]
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    if snapshot == c.c_void_p(-1).value:
        raise c.WinError(c.get_last_error())
    rows, entry = {}, Entry()
    entry.size = c.sizeof(entry)
    try:
        more = kernel.Process32FirstW(snapshot, c.byref(entry))
        while more:
            handle = kernel.OpenProcess(0x0400 | 0x0010, False, entry.pid)
            if handle:
                try:
                    times, memory = [w.FILETIME() for _ in range(4)], Memory()
                    memory.size = c.sizeof(memory)
                    if kernel.GetProcessTimes(handle, *[c.byref(t) for t in times]):
                        values = [(t.dwHighDateTime << 32) | t.dwLowDateTime for t in times]
                        rss = memory.rss if psapi.GetProcessMemoryInfo(handle, c.byref(memory), memory.size) else None
                        rows[entry.pid] = dict(parent=entry.parent, identity=values[0], rss=rss,
                                               threads=entry.threads, cpu=sum(values[2:]) / 1e7)
                finally:
                    kernel.CloseHandle(handle)
            more = kernel.Process32NextW(snapshot, c.byref(entry))
    finally:
        kernel.CloseHandle(snapshot)
    return rows


class ProcessSampler:
    def __init__(self, interval=0.1, collector=None):
        self.interval = interval
        self.collector = collector or (windows_processes if os.name == "nt" else
                                       macos_processes if sys.platform == "darwin" else linux_processes)
        self.roots, self.identities, self.cpu = set(), {}, {}
        self.lock, self.stopped = threading.Lock(), threading.Event()
        self.peak_rss = self.peak_threads = None
        self.samples, self.errors = 0, set()
        self.thread = threading.Thread(target=self._run, name="test-resource-sampler", daemon=True)

    def start(self):
        self.thread.start()

    def add(self, pid):
        with self.lock:
            self.identities.pop(pid, None)  # A newly launched root may reuse a PID.
            self.roots.add(pid)

    def remove(self, pid):
        with self.lock:
            self.roots.discard(pid)

    def _sample(self):
        rows = self.collector()
        with self.lock:
            selected = {pid for pid, row in rows.items()
                        if (pid in self.roots and self.identities.get(pid, row["identity"]) == row["identity"])
                        or self.identities.get(pid) == row["identity"]}
            while True:
                children = {pid for pid, row in rows.items() if row["parent"] in selected and pid not in selected
                            and self.identities.get(pid, row["identity"]) == row["identity"]}
                if not children:
                    break
                selected.update(children)
            if not selected:
                return
            for pid in selected:
                row = rows[pid]
                self.identities[pid] = row["identity"]
                self.cpu[(pid, row["identity"])] = max(self.cpu.get((pid, row["identity"]), 0), row["cpu"])
            for field, attribute in (("rss", "peak_rss"), ("threads", "peak_threads")):
                values = [rows[pid][field] for pid in selected]
                if all(value is not None for value in values):
                    setattr(self, attribute, max(getattr(self, attribute) or 0, sum(values)))
            self.samples += 1

    def _run(self):
        while not self.stopped.is_set():
            try:
                self._sample()
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                with self.lock:
                    self.errors.add(f"{type(error).__name__}: {error}")
            self.stopped.wait(self.interval)

    def stop(self):
        self.stopped.set()
        self.thread.join(timeout=7)
        if self.thread.is_alive():
            with self.lock:
                self.errors.add("collector did not stop within 7 seconds")

    def summary(self):
        with self.lock:
            return dict(collector_version=1, interval_seconds=self.interval, samples=self.samples,
                        observed_peak_rss_bytes=self.peak_rss, observed_peak_threads=self.peak_threads,
                        observed_cpu_seconds=round(sum(self.cpu.values()), 3) if self.cpu else None,
                        errors=sorted(self.errors))
