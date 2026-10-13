"""Windows PDH telemetry. One collection feeds app, DWM and desktop totals.

Engine percentages describe scheduler occupancy, not power or shader throughput.
Empty/invalid counters are unavailable, never idle zero. Shared process memory
cannot be added across processes because shared surfaces would be counted twice.
"""
import ctypes as c
import ctypes.wintypes as w
from collections import defaultdict
import json
import math
import os
from pathlib import Path
import re
import threading
import time

INSTANCE = re.compile(r"pid_(\d+)_(luid_0x[\da-f]+_0x[\da-f]+)_phys_(\d+)(?:_eng_(\d+)_engtype_(.+))?", re.I)
PATHS = {
    "engines": r"\GPU Engine(*)\Utilization Percentage",
    "dedicated_mib": r"\GPU Process Memory(*)\Dedicated Usage",
    "shared_mib": r"\GPU Process Memory(*)\Shared Usage",
    "committed_mib": r"\GPU Process Memory(*)\Total Committed",
    "device_dedicated_mib": r"\GPU Adapter Memory(*)\Dedicated Usage",
    "device_shared_mib": r"\GPU Adapter Memory(*)\Shared Usage",
    "device_committed_mib": r"\GPU Adapter Memory(*)\Total Committed",
}


class PdhQuery:
    def __init__(self):
        if os.name != "nt":
            raise OSError("Windows PDH requires Windows")
        self.pdh = c.WinDLL("pdh")
        self.query, self.handles, self.errors = w.HANDLE(), {}, {}
        declarations = {
            "PdhOpenQueryW": [w.LPCWSTR, c.c_size_t, c.POINTER(w.HANDLE)],
            "PdhAddEnglishCounterW": [w.HANDLE, w.LPCWSTR, c.c_size_t, c.POINTER(w.HANDLE)],
            "PdhCollectQueryData": [w.HANDLE], "PdhCloseQuery": [w.HANDLE],
            "PdhGetFormattedCounterArrayW": [w.HANDLE, w.DWORD, c.POINTER(w.DWORD), c.POINTER(w.DWORD), c.c_void_p],
        }
        for name, args in declarations.items():
            function = getattr(self.pdh, name)
            function.argtypes, function.restype = args, w.DWORD
        status = self.pdh.PdhOpenQueryW(None, 0, c.byref(self.query))
        if status:
            raise OSError(f"PdhOpenQueryW: 0x{status:08x}")
        for name, path in PATHS.items():
            handle = w.HANDLE()
            status = self.pdh.PdhAddEnglishCounterW(self.query, path, 0, c.byref(handle))
            if status:
                self.errors[name] = f"0x{status:08x}"
            else:
                self.handles[name] = handle

    def collect(self):
        class Number(c.Union):
            _fields_ = [("double", c.c_double), ("integer", c.c_int64)]
        class Value(c.Structure):
            _fields_ = [("status", w.DWORD), ("number", Number)]
        class Item(c.Structure):
            _fields_ = [("name", w.LPWSTR), ("value", Value)]
        result = {"unix_ms": time.time() * 1000, "errors": dict(self.errors)}
        before, after, frequency = c.c_int64(), c.c_int64(), c.c_int64()
        c.windll.kernel32.QueryPerformanceFrequency(c.byref(frequency))
        c.windll.kernel32.QueryPerformanceCounter(c.byref(before))
        status = self.pdh.PdhCollectQueryData(self.query)
        c.windll.kernel32.QueryPerformanceCounter(c.byref(after))
        result.update(qpc_before=before.value, qpc_after=after.value, qpc_frequency=frequency.value)
        for name in PATHS:
            result[name] = None
            if status or name not in self.handles:
                result["errors"][name] = f"0x{status:08x}" if status else self.errors[name]
                continue
            integer = name != "engines"
            flags = (0x1400 if integer else 0x200) | 0x8000
            size, count = w.DWORD(), w.DWORD()
            code = self.pdh.PdhGetFormattedCounterArrayW(self.handles[name], flags, c.byref(size), c.byref(count), None)
            if code != 0x800007d2:
                result["errors"][name] = f"0x{code:08x}"
                continue
            buffer = c.create_string_buffer(size.value)
            code = self.pdh.PdhGetFormattedCounterArrayW(self.handles[name], flags, c.byref(size), c.byref(count), buffer)
            if code:
                result["errors"][name] = f"0x{code:08x}"
                continue
            rows = c.cast(buffer, c.POINTER(Item))
            result[name] = []
            for ix in range(count.value):
                value = rows[ix].value
                number = value.number.integer if integer else value.number.double
                valid = value.status in (0, 1) and math.isfinite(number) and number >= 0
                result[name].append({"instance": rows[ix].name, "status": f"0x{value.status:08x}",
                                     "value": number if valid else None})
        return result

    def close(self):
        if self.query:
            self.pdh.PdhCloseQuery(self.query)
            self.query = w.HANDLE()


def wts_dwm_pid(session, candidates):
    """Fallback when ProcessIdToSessionId denies access to protected DWM."""
    library = c.WinDLL("wtsapi32",use_last_error=True)
    class Info(c.Structure):
        _fields_ = [("session",w.DWORD),("pid",w.DWORD),("name",w.LPWSTR),("sid",c.c_void_p)]
    library.WTSEnumerateProcessesW.argtypes = [w.HANDLE,w.DWORD,w.DWORD,c.POINTER(c.POINTER(Info)),c.POINTER(w.DWORD)]
    library.WTSEnumerateProcessesW.restype = w.BOOL
    library.WTSFreeMemory.argtypes = [c.c_void_p]
    entries,count = c.POINTER(Info)(),w.DWORD()
    if not library.WTSEnumerateProcessesW(None,0,1,c.byref(entries),c.byref(count)):
        raise OSError(c.get_last_error(),"WTS process enumeration failed")
    try:
        return next((entries[i].pid for i in range(count.value) if entries[i].pid in candidates
                     and entries[i].session==session and entries[i].name
                     and entries[i].name.lower()=="dwm.exe"),None)
    finally:
        library.WTSFreeMemory(entries)


def dwm_identity():
    """Toolhelp names do not require opening the protected DWM process."""
    result = {"pid": None, "session_id": None, "discovery": "Toolhelp32 process name and ProcessIdToSessionId"}
    if os.name != "nt":
        return dict(result, unavailable="DWM discovery requires Windows")
    try:
        kernel = c.WinDLL("kernel32", use_last_error=True)
        class Entry(c.Structure):
            _fields_ = [("size", w.DWORD), ("usage", w.DWORD), ("pid", w.DWORD),
                        ("heap", c.c_size_t), ("module", w.DWORD), ("threads", w.DWORD),
                        ("parent", w.DWORD), ("priority", w.LONG), ("flags", w.DWORD),
                        ("name", w.WCHAR * 260)]
        kernel.CreateToolhelp32Snapshot.argtypes, kernel.CreateToolhelp32Snapshot.restype = [w.DWORD, w.DWORD], w.HANDLE
        for name in ("Process32FirstW", "Process32NextW"):
            getattr(kernel,name).argtypes, getattr(kernel,name).restype = [w.HANDLE,c.POINTER(Entry)], w.BOOL
        kernel.ProcessIdToSessionId.argtypes, kernel.ProcessIdToSessionId.restype = [w.DWORD,c.POINTER(w.DWORD)], w.BOOL
        kernel.CloseHandle.argtypes = [w.HANDLE]
        session = w.DWORD()
        if not kernel.ProcessIdToSessionId(os.getpid(),c.byref(session)):
            return dict(result, unavailable=f"current session lookup failed: {c.get_last_error()}")
        result["session_id"] = session.value
        snapshot = kernel.CreateToolhelp32Snapshot(0x2,0)  # TH32CS_SNAPPROCESS
        if snapshot == w.HANDLE(-1).value or not snapshot:
            return dict(result, unavailable=f"process snapshot failed: {c.get_last_error()}")
        try:
            entry = Entry()
            entry.size = c.sizeof(entry)
            candidates = set()
            session_errors = []
            present = kernel.Process32FirstW(snapshot,c.byref(entry))
            if not present:
                return dict(result, unavailable=f"process enumeration failed: {c.get_last_error()}")
            while present:
                if entry.name.lower() == "dwm.exe":
                    candidates.add(entry.pid)
                    other = w.DWORD()
                    matched=kernel.ProcessIdToSessionId(entry.pid,c.byref(other))
                    if matched and other.value == session.value:
                        return dict(result,pid=entry.pid)
                    if not matched and c.get_last_error():
                        session_errors.append(c.get_last_error())
                present = kernel.Process32NextW(snapshot,c.byref(entry))
        finally:
            kernel.CloseHandle(snapshot)
        if candidates:
            pid = wts_dwm_pid(session.value,candidates)
            if pid is not None:
                return dict(result,pid=pid,discovery="Toolhelp32 name plus WTS process session",
                            process_id_to_session_errors=sorted(set(session_errors)))
        return dict(result, unavailable="no DWM process name found in the current session")
    except (AttributeError,OSError) as error:
        return dict(result, unavailable=str(error))


def dwm_pid():
    return dwm_identity()["pid"]


def records(sample, previous, pids):
    """Convert one shared collection without storing unrelated PID identities."""
    base = {"backend": "windows-pdh", "sample_start_unix_ms": sample["unix_ms"],
            "sample_end_unix_ms": sample["unix_ms"],
            "interval_start_unix_ms": previous["unix_ms"] if previous else None,
            "interval_end_unix_ms": sample["unix_ms"],
            "qpc_before": sample.get("qpc_before"), "qpc_after": sample.get("qpc_after"),
            "previous_qpc_after": previous.get("qpc_after") if previous else None,
            "qpc_frequency": sample.get("qpc_frequency"), "errors": sample.get("errors", {})}
    engines, memory = defaultdict(dict), defaultdict(dict)
    totals = defaultdict(lambda: defaultdict(list))
    for row in sample.get("engines") or []:
        match = INSTANCE.fullmatch(row["instance"])
        if not match or match[4] is None:
            continue
        pid, luid, physical, engine, kind = match.groups()
        adapter, identity = f"{luid.lower()}_phys_{physical}", f"{engine}:{kind}"
        totals[adapter][identity].append(row["value"])
        for role, target in pids.items():
            if target == int(pid):
                engines[(role, adapter)][identity] = row["value"]
    for name in ("dedicated_mib", "shared_mib", "committed_mib"):
        for row in sample.get(name) or []:
            match = INSTANCE.fullmatch(row["instance"])
            if not match:
                continue
            pid, luid, physical, _, _ = match.groups()
            for role, target in pids.items():
                if target == int(pid):
                    memory[(role, f"{luid.lower()}_phys_{physical}")][name] = None if row["value"] is None else row["value"] / 1048576
    output = []
    for role, adapter in sorted(engines.keys() | memory.keys()):
        values = engines[(role, adapter)]
        busy = max(values.values()) if values and all(v is not None for v in values.values()) else None
        output.append(dict(base, scope="process", role=role, pid=pids[role], gpu_index=adapter,
                           busiest_engine_percent=busy, engines=values, **memory[(role, adapter)]))
    adapters = set(totals)
    device_memory = defaultdict(dict)
    for name in ("device_dedicated_mib", "device_shared_mib", "device_committed_mib"):
        for row in sample.get(name) or []:
            adapter = row["instance"].lower()
            adapters.add(adapter)
            device_memory[adapter][name.removeprefix("device_")] = None if row["value"] is None else row["value"] / 1048576
    for adapter in sorted(adapters):
        summed = {engine: sum(values) if all(v is not None for v in values) else None
                  for engine, values in totals[adapter].items()}
        busy = max(summed.values()) if summed and all(v is not None for v in summed.values()) else None
        output.append(dict(base, scope="device", gpu_index=adapter, busiest_engine_percent=busy,
                           engines=summed, **device_memory[adapter]))
    return output


class WindowsSampler:
    def __init__(self, output, pids, query_factory=PdhQuery):
        self.output, self.pids = Path(output), dict(pids)
        self.dwm = dwm_identity()
        self.pids.setdefault("desktop_compositor", self.dwm["pid"])
        self.stop, self.counts, self.errors = threading.Event(), defaultdict(int), []
        self.stream = (self.output / "gpu.jsonl").open("w", encoding="utf-8")
        self.metadata = {"requested": True, "backend": "windows-pdh", "status": "unavailable", "interval_ms": 1000,
                         "desktop_compositor_discovery": self.dwm,
                         "note": "App PID, shared desktop DWM PID and whole-adapter occupancy are separate. DWM/device costs are not app attribution. Missing counters are not zero; busiest-engine percentage is not power or execution time."}
        try:
            self.query = query_factory()
        except (OSError, AttributeError) as error:
            self.query = None
            self.metadata["reason"] = str(error)
            return
        self.worker = threading.Thread(target=self._run, name="windows-gpu-pdh", daemon=True)
        self.worker.start()

    def _run(self):
        previous = None
        try:
            while not self.stop.is_set():
                sample = self.query.collect()
                for name, detail in sample.get("errors", {}).items():
                    message = f"{name}: {detail}"
                    if message not in self.errors:
                        self.errors.append(message)
                for row in records(sample, previous, self.pids):
                    self.stream.write(json.dumps(row) + "\n")
                    if row.get("busiest_engine_percent") is not None and previous is not None:
                        self.counts[row.get("role", "device")] += 1
                self.stream.flush()
                previous = sample
                self.stop.wait(1)
        except Exception as error:
            self.errors.append(str(error))

    def set_app_pid(self, pid):
        self.pids = dict(self.pids, app=pid)

    def close(self):
        self.stop.set()
        if self.query:
            self.worker.join(5)
            if self.worker.is_alive():
                raise RuntimeError("PDH sampler did not stop; capture is invalid")
            self.query.close()
        self.stream.close()
        if self.counts.get("app"):
            self.metadata["status"] = "partial" if self.errors else "available"
        self.metadata.update(pids=self.pids, samples=dict(self.counts), errors=self.errors)
        (self.output / "gpu-capture.json").write_text(json.dumps(self.metadata, indent=2) + "\n", encoding="utf-8")
        return self.metadata
