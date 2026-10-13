"""macOS process CPU and GPU counters for the performance harness.

Standard library only (ctypes); no root access, no helper processes.

GPU: the AGX driver keeps an ``AppUsage`` list on every Metal client
(``AGXDeviceUserClient`` in the IORegistry, the same data ``ioreg`` prints)
whose ``accumulatedGPUTime`` is the client's GPU execution time in nanoseconds.
``IOUserClientCreator`` names the owning process ("pid 123, name"). Summing a
process's clients gives a cumulative per-process GPU time; Activity Monitor's
GPU Time column reads the same counter. A phase's delta divided by its wall
time is the process's GPU busy fraction. WindowServer is sampled the same way:
it composites every presented frame, but its counter also includes every other
application on screen, so it is context rather than attribution.
``AGXAccelerator``'s ``PerformanceStatistics`` holds whole-device utilization
percentages (driver point samples, all processes).

CPU: ``proc_pid_rusage(RUSAGE_INFO_V6)`` gives user/system time (Mach ticks),
package-idle and interrupt wakeups, instructions/cycles, the kernel's CPU
energy estimate (nJ, CPU only) and the physical footprint.

Missing counters are None, never zero.
"""
import ctypes as c
import functools
import platform
import re
import time

_SYSTEM = platform.system() == "Darwin"
_CF_UTF8 = 0x08000100
_CF_SINT64 = 4
_RUSAGE_INFO_V6 = 6
_PROC_PIDTASKINFO = 4

# struct rusage_info_v6 from <sys/resource.h>: uuid, then 64-bit counters.
_RUSAGE_FIELDS = (
    "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups", "pageins", "wired_size",
    "resident_size", "phys_footprint", "proc_start_abstime", "proc_exit_abstime",
    "child_user_time", "child_system_time", "child_pkg_idle_wkups", "child_interrupt_wkups",
    "child_pageins", "child_elapsed_abstime", "diskio_bytesread", "diskio_byteswritten",
    "cpu_time_qos_default", "cpu_time_qos_maintenance", "cpu_time_qos_background",
    "cpu_time_qos_utility", "cpu_time_qos_legacy", "cpu_time_qos_user_initiated",
    "cpu_time_qos_user_interactive", "billed_system_time", "serviced_system_time",
    "logical_writes", "lifetime_max_phys_footprint", "instructions", "cycles", "billed_energy",
    "serviced_energy", "interval_max_phys_footprint", "runnable_time", "flags", "user_ptime",
    "system_ptime", "pinstructions", "pcycles", "energy_nj", "penergy_nj",
    "secure_time_in_system", "secure_ptime_in_system", "neural_footprint",
    "lifetime_max_neural_footprint", "interval_max_neural_footprint", "conclave_footprint",
    "page_wait_time_mach", "page_cache_hits")


class _RusageInfo(c.Structure):
    _fields_ = [("uuid", c.c_uint8 * 16), *((name, c.c_uint64) for name in _RUSAGE_FIELDS),
                ("reserved", c.c_uint64 * 6)]


class _TaskInfo(c.Structure):
    # struct proc_taskinfo from <sys/proc_info.h>.
    _fields_ = [("virtual_size", c.c_uint64), ("resident_size", c.c_uint64),
                ("total_user", c.c_uint64), ("total_system", c.c_uint64),
                ("threads_user", c.c_uint64), ("threads_system", c.c_uint64),
                ("policy", c.c_int32), ("faults", c.c_int32), ("pageins", c.c_int32),
                ("cow_faults", c.c_int32), ("messages_sent", c.c_int32),
                ("messages_received", c.c_int32), ("syscalls_mach", c.c_int32),
                ("syscalls_unix", c.c_int32), ("csw", c.c_int32), ("threadnum", c.c_int32),
                ("numrunning", c.c_int32), ("priority", c.c_int32)]


class _Timebase(c.Structure):
    _fields_ = [("numer", c.c_uint32), ("denom", c.c_uint32)]


@functools.cache
def _load():
    """Bind the native functions once; None when unavailable."""
    if not _SYSTEM:
        return None
    try:
        system = c.CDLL("/usr/lib/libSystem.B.dylib")
        iokit = c.CDLL("/System/Library/Frameworks/IOKit.framework/IOKit")
        cf = c.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
    except OSError:
        return None
    system.proc_pid_rusage.argtypes = [c.c_int, c.c_int, c.c_void_p]
    system.proc_pid_rusage.restype = c.c_int
    system.proc_pidinfo.argtypes = [c.c_int, c.c_int, c.c_uint64, c.c_void_p, c.c_int]
    system.proc_pidinfo.restype = c.c_int
    system.mach_timebase_info.argtypes = [c.POINTER(_Timebase)]
    iokit.IOServiceMatching.argtypes = [c.c_char_p]
    iokit.IOServiceMatching.restype = c.c_void_p
    iokit.IOServiceGetMatchingServices.argtypes = [c.c_uint32, c.c_void_p, c.POINTER(c.c_uint32)]
    iokit.IOServiceGetMatchingServices.restype = c.c_int
    iokit.IOIteratorNext.argtypes = [c.c_uint32]
    iokit.IOIteratorNext.restype = c.c_uint32
    iokit.IOObjectRelease.argtypes = [c.c_uint32]
    iokit.IORegistryEntryCreateCFProperty.argtypes = [c.c_uint32, c.c_void_p, c.c_void_p, c.c_uint32]
    iokit.IORegistryEntryCreateIterator.argtypes = [c.c_uint32, c.c_char_p, c.c_uint32, c.POINTER(c.c_uint32)]
    iokit.IORegistryEntryCreateIterator.restype = c.c_int
    iokit.IOObjectConformsTo.argtypes = [c.c_uint32, c.c_char_p]
    iokit.IOObjectConformsTo.restype = c.c_bool
    iokit.IORegistryEntryCreateCFProperty.restype = c.c_void_p
    cf.CFStringCreateWithCString.argtypes = [c.c_void_p, c.c_char_p, c.c_uint32]
    cf.CFStringCreateWithCString.restype = c.c_void_p
    cf.CFStringGetCString.argtypes = [c.c_void_p, c.c_char_p, c.c_long, c.c_uint32]
    cf.CFStringGetCString.restype = c.c_bool
    cf.CFGetTypeID.argtypes = [c.c_void_p]
    cf.CFGetTypeID.restype = c.c_ulong
    for name in ("CFArrayGetTypeID", "CFDictionaryGetTypeID", "CFNumberGetTypeID", "CFStringGetTypeID"):
        getattr(cf, name).restype = c.c_ulong
    cf.CFArrayGetCount.argtypes = [c.c_void_p]
    cf.CFArrayGetCount.restype = c.c_long
    cf.CFArrayGetValueAtIndex.argtypes = [c.c_void_p, c.c_long]
    cf.CFArrayGetValueAtIndex.restype = c.c_void_p
    cf.CFDictionaryGetValue.argtypes = [c.c_void_p, c.c_void_p]
    cf.CFDictionaryGetValue.restype = c.c_void_p
    cf.CFNumberGetValue.argtypes = [c.c_void_p, c.c_int, c.c_void_p]
    cf.CFNumberGetValue.restype = c.c_bool
    cf.CFRelease.argtypes = [c.c_void_p]
    timebase = _Timebase()
    system.mach_timebase_info(c.byref(timebase))
    keys = {name: cf.CFStringCreateWithCString(None, name.encode(), _CF_UTF8)
            for name in ("IOUserClientCreator", "AppUsage", "accumulatedGPUTime", "PerformanceStatistics",
                         "Device Utilization %", "Renderer Utilization %", "Tiler Utilization %",
                         "In use system memory", "Alloc system memory")}
    types = {name: getattr(cf, f"CF{name}GetTypeID")() for name in ("Array", "Dictionary", "Number", "String")}
    return {"system": system, "iokit": iokit, "cf": cf, "keys": keys, "types": types,
            "tick_ns": timebase.numer / timebase.denom}


def available():
    return _load() is not None


# ---------------------------------------------------------------- CPU

def process_sample(pid):
    """Cumulative counters for one process; None once it is gone."""
    libs = _load()
    if libs is None:
        return None
    info = _RusageInfo()
    if libs["system"].proc_pid_rusage(pid, _RUSAGE_INFO_V6, c.byref(info)) != 0:
        return None
    tick = libs["tick_ns"]
    task = _TaskInfo()
    size = libs["system"].proc_pidinfo(pid, _PROC_PIDTASKINFO, 0, c.byref(task), c.sizeof(task))
    has_task = size == c.sizeof(task)
    return {
        "unix_ms": time.time() * 1000, "pid": pid,
        "cpu_s": (info.user_time + info.system_time) * tick / 1e9,
        "user_cpu_s": info.user_time * tick / 1e9, "system_cpu_s": info.system_time * tick / 1e9,
        "children_cpu_s": (info.child_user_time + info.child_system_time) * tick / 1e9,
        # Wakeups from package idle plus interrupt wakeups: Activity Monitor's
        # "Idle Wake Ups" is the first of these.
        "idle_wakeups": info.pkg_idle_wkups, "interrupt_wakeups": info.interrupt_wkups,
        "instructions": info.instructions, "cycles": info.cycles,
        # The kernel's per-process CPU energy estimate; GPU work is not included.
        "cpu_energy_nj": info.energy_nj, "billed_energy": info.billed_energy,
        "rss_kib": info.resident_size / 1024, "phys_footprint_kib": info.phys_footprint / 1024,
        "threads": task.threadnum if has_task else None,
        "voluntary_switches": task.csw if has_task else None,
        "read_bytes": info.diskio_bytesread, "write_bytes": info.diskio_byteswritten,
    }


# ---------------------------------------------------------------- GPU

def _cf_string(libs, ref):
    if not ref or libs["cf"].CFGetTypeID(ref) != libs["types"]["String"]:
        return None
    buffer = c.create_string_buffer(512)
    return buffer.value.decode("utf-8", "replace") if libs["cf"].CFStringGetCString(ref, buffer, 512, _CF_UTF8) else None


def _cf_int(libs, ref):
    if not ref or libs["cf"].CFGetTypeID(ref) != libs["types"]["Number"]:
        return None
    value = c.c_int64()
    return value.value if libs["cf"].CFNumberGetValue(ref, _CF_SINT64, c.byref(value)) else None


def _iterate(libs, iterator):
    """Yields the iterator's entries, releasing each after use, then the iterator."""
    iokit = libs["iokit"]
    try:
        while True:
            entry = iokit.IOIteratorNext(iterator)
            if not entry:
                break
            try:
                yield entry
            finally:
                iokit.IOObjectRelease(entry)
    finally:
        iokit.IOObjectRelease(iterator)


def _services(libs, class_name):
    """Registered services of one class (the GPU accelerators)."""
    matching = libs["iokit"].IOServiceMatching(class_name.encode())
    iterator = c.c_uint32()
    # IOServiceGetMatchingServices consumes the matching dictionary.
    if not matching or libs["iokit"].IOServiceGetMatchingServices(0, matching, c.byref(iterator)) != 0:
        return
    yield from _iterate(libs, iterator.value)


def _gpu_clients(libs):
    """Metal clients: user clients are not registered services, so walk each
    accelerator's service-plane subtree as ioreg does."""
    for accelerator in _services(libs, "AGXAccelerator"):
        iterator = c.c_uint32()
        # kIORegistryIterateRecursively = 1
        if libs["iokit"].IORegistryEntryCreateIterator(accelerator, b"IOService", 1, c.byref(iterator)) != 0:
            continue
        for entry in _iterate(libs, iterator.value):
            if libs["iokit"].IOObjectConformsTo(entry, b"AGXDeviceUserClient"):
                yield entry


def _property(libs, entry, key):
    return libs["iokit"].IORegistryEntryCreateCFProperty(entry, libs["keys"][key], None, 0)


_CREATOR = re.compile(r"pid (\d+)")


def gpu_times_ns(pids=None):
    """{pid: cumulative GPU nanoseconds} summed over each process's Metal
    clients. With `pids`, other processes are skipped before reading usage."""
    libs = _load()
    if libs is None:
        return None
    cf, types = libs["cf"], libs["types"]
    totals = {}
    for entry in _gpu_clients(libs):
        creator_ref = _property(libs, entry, "IOUserClientCreator")
        if not creator_ref:
            continue
        try:
            match = _CREATOR.match(_cf_string(libs, creator_ref) or "")
        finally:
            cf.CFRelease(creator_ref)
        if not match:
            continue
        pid = int(match.group(1))
        if pids is not None and pid not in pids:
            continue
        usage = _property(libs, entry, "AppUsage")
        if not usage:
            totals.setdefault(pid, 0)
            continue
        try:
            if cf.CFGetTypeID(usage) != types["Array"]:
                continue
            total = totals.setdefault(pid, 0)
            for index in range(cf.CFArrayGetCount(usage)):
                item = cf.CFArrayGetValueAtIndex(usage, index)
                if not item or cf.CFGetTypeID(item) != types["Dictionary"]:
                    continue
                value = _cf_int(libs, cf.CFDictionaryGetValue(item, libs["keys"]["accumulatedGPUTime"]))
                if value is not None:
                    total += value
            totals[pid] = total
        finally:
            cf.CFRelease(usage)
    return totals


def device_statistics():
    """Whole-GPU driver statistics (all processes); None when unavailable."""
    libs = _load()
    if libs is None:
        return None
    cf, types = libs["cf"], libs["types"]
    for entry in _services(libs, "AGXAccelerator"):
        stats = _property(libs, entry, "PerformanceStatistics")
        if not stats:
            continue
        try:
            if cf.CFGetTypeID(stats) != types["Dictionary"]:
                continue
            value = lambda key: _cf_int(libs, cf.CFDictionaryGetValue(stats, libs["keys"][key]))  # noqa: E731
            in_use, allocated = value("In use system memory"), value("Alloc system memory")
            return {"device_utilization_percent": value("Device Utilization %"),
                    "renderer_utilization_percent": value("Renderer Utilization %"),
                    "tiler_utilization_percent": value("Tiler Utilization %"),
                    "in_use_system_memory_mib": in_use / 2**20 if in_use is not None else None,
                    "alloc_system_memory_mib": allocated / 2**20 if allocated is not None else None}
        finally:
            cf.CFRelease(stats)
    return None


class IOReportSampler:
    """Whole-SoC GPU energy and GPU performance-state residency from IOReport
    (the private framework behind powermetrics), readable without root.

    `delta()` between two `sample()`s returns the GPU energy in nanojoules and
    the time spent in each GPU performance state (OFF, P1 = lowest clock, ...)
    in 24 MHz ticks. Energy covers every process using the GPU, WindowServer
    included; the P-states explain changes in per-process GPU time that come
    from clock changes rather than from work.
    """

    def __init__(self):
        self.ok = False
        if not _SYSTEM:
            return
        try:
            self.cf = c.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
            self.ior = c.CDLL("/usr/lib/libIOReport.dylib")
        except OSError:
            return
        cf, ior, ptr = self.cf, self.ior, c.c_void_p
        cf.CFStringCreateWithCString.argtypes = [ptr, c.c_char_p, c.c_uint32]
        cf.CFStringCreateWithCString.restype = ptr
        cf.CFStringGetCString.argtypes = [ptr, c.c_char_p, c.c_long, c.c_uint32]
        cf.CFStringGetCString.restype = c.c_bool
        cf.CFDictionaryGetValue.argtypes = [ptr, ptr]
        cf.CFDictionaryGetValue.restype = ptr
        cf.CFArrayGetCount.argtypes = [ptr]
        cf.CFArrayGetCount.restype = c.c_long
        cf.CFArrayGetValueAtIndex.argtypes = [ptr, c.c_long]
        cf.CFArrayGetValueAtIndex.restype = ptr
        cf.CFRelease.argtypes = [ptr]
        cf.CFDictionaryCreateMutableCopy.argtypes = [ptr, c.c_long, ptr]
        cf.CFDictionaryCreateMutableCopy.restype = ptr
        ior.IOReportCopyChannelsInGroup.argtypes = [ptr, ptr, c.c_uint64, c.c_uint64, c.c_uint64]
        ior.IOReportCopyChannelsInGroup.restype = ptr
        ior.IOReportMergeChannels.argtypes = [ptr, ptr, ptr]
        ior.IOReportCreateSubscription.argtypes = [ptr, ptr, c.POINTER(ptr), c.c_uint64, ptr]
        ior.IOReportCreateSubscription.restype = ptr
        ior.IOReportCreateSamples.argtypes = [ptr, ptr, ptr]
        ior.IOReportCreateSamples.restype = ptr
        ior.IOReportCreateSamplesDelta.argtypes = [ptr, ptr, ptr]
        ior.IOReportCreateSamplesDelta.restype = ptr
        for name in ("IOReportChannelGetGroup", "IOReportChannelGetSubGroup",
                     "IOReportChannelGetChannelName", "IOReportChannelGetUnitLabel"):
            getattr(ior, name).argtypes = [ptr]
            getattr(ior, name).restype = ptr
        ior.IOReportChannelGetFormat.argtypes = [ptr]
        ior.IOReportChannelGetFormat.restype = c.c_uint8
        ior.IOReportSimpleGetIntegerValue.argtypes = [ptr, c.c_int32]
        ior.IOReportSimpleGetIntegerValue.restype = c.c_int64
        ior.IOReportStateGetCount.argtypes = [ptr]
        ior.IOReportStateGetCount.restype = c.c_int32
        ior.IOReportStateGetNameForIndex.argtypes = [ptr, c.c_int32]
        ior.IOReportStateGetNameForIndex.restype = ptr
        ior.IOReportStateGetResidency.argtypes = [ptr, c.c_int32]
        ior.IOReportStateGetResidency.restype = c.c_int64
        self.channels_key = self._string("IOReportChannels")
        channels = ior.IOReportCopyChannelsInGroup(self._string("Energy Model"), None, 0, 0, 0)
        states = ior.IOReportCopyChannelsInGroup(self._string("GPU Stats"),
                                                 self._string("GPU Performance States"), 0, 0, 0)
        if not channels or not states:
            return
        ior.IOReportMergeChannels(channels, states, None)
        desired = cf.CFDictionaryCreateMutableCopy(None, 0, channels)
        self.subscribed = ptr()
        self.subscription = ior.IOReportCreateSubscription(None, desired, c.byref(self.subscribed), 0, None)
        self.ok = bool(self.subscription)

    def _string(self, text):
        return self.cf.CFStringCreateWithCString(None, text.encode(), _CF_UTF8)

    def _text(self, ref):
        if not ref:
            return None
        buffer = c.create_string_buffer(256)
        return buffer.value.decode() if self.cf.CFStringGetCString(ref, buffer, 256, _CF_UTF8) else None

    def sample(self):
        return self.ior.IOReportCreateSamples(self.subscription, self.subscribed, None) if self.ok else None

    def release(self, sample):
        if sample:
            self.cf.CFRelease(sample)

    def delta(self, before, after):
        """{"gpu_energy_nj": int|None, "gpu_states": {name: ticks}}"""
        if not before or not after:
            return None
        delta = self.ior.IOReportCreateSamplesDelta(before, after, None)
        if not delta:
            return None
        result = {"gpu_energy_nj": None, "gpu_states": {}}
        scale = {"nJ": 1, "uJ": 1_000, "mJ": 1_000_000}
        try:
            channels = self.cf.CFDictionaryGetValue(delta, self.channels_key)
            for index in range(self.cf.CFArrayGetCount(channels) if channels else 0):
                channel = self.cf.CFArrayGetValueAtIndex(channels, index)
                name = self._text(self.ior.IOReportChannelGetChannelName(channel))
                if self.ior.IOReportChannelGetFormat(channel) == 2:
                    if name == "GPUPH":
                        for state in range(self.ior.IOReportStateGetCount(channel)):
                            label = self._text(self.ior.IOReportStateGetNameForIndex(channel, state))
                            result["gpu_states"][label] = self.ior.IOReportStateGetResidency(channel, state)
                elif name == "GPU Energy":
                    unit = self._text(self.ior.IOReportChannelGetUnitLabel(channel)) or "nJ"
                    value = self.ior.IOReportSimpleGetIntegerValue(channel, 0)
                    result["gpu_energy_nj"] = value * scale.get(unit.strip(), 1)
        finally:
            self.cf.CFRelease(delta)
        return result


def window_server_pid():
    """The compositor's PID, from its Metal client registration."""
    libs = _load()
    if libs is None:
        return None
    for entry in _gpu_clients(libs):
        creator_ref = _property(libs, entry, "IOUserClientCreator")
        if not creator_ref:
            continue
        try:
            text = _cf_string(libs, creator_ref) or ""
        finally:
            libs["cf"].CFRelease(creator_ref)
        if text.endswith(", WindowServer"):
            match = _CREATOR.match(text)
            if match:
                return int(match.group(1))
    return None


def screen_locked():
    """True when the console session is locked: GUI captures then draw nothing
    and still finish. None when the state cannot be read."""
    import plistlib
    import subprocess
    try:
        out = subprocess.run(["ioreg", "-n", "Root", "-d1", "-a"], capture_output=True, timeout=5, check=True).stdout
        root = plistlib.loads(out)
    except (OSError, subprocess.SubprocessError, plistlib.InvalidFileException, ValueError):
        return None
    users = root.get("IOConsoleUsers") or []
    # The key is absent, not false, while unlocked.
    return any(user.get("CGSSessionScreenIsLocked") for user in users)


if __name__ == "__main__":
    import json
    import sys
    target = [int(arg) for arg in sys.argv[1:]] or None
    server = window_server_pid()
    started = time.perf_counter()
    times = gpu_times_ns(set(target + [server]) if target else None)
    elapsed = time.perf_counter() - started
    print(json.dumps({"window_server_pid": server, "gpu_ns": times, "device": device_statistics(),
                      "query_ms": elapsed * 1000,
                      "process": [process_sample(pid) for pid in (target or [])],
                      "screen_locked": screen_locked()}, indent=2))
