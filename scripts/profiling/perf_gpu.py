"""Opt-in, external GPU telemetry. No renderer instrumentation or extra packages.

NVIDIA pmon's SM/memory busy percentages are interval averages; framebuffer
memory is a point sample in NVIDIA's reported MB. Device power/utilization is
whole-device data and must never be attributed to a process. Unsupported ('-'
or N/A) counters remain None. See https://docs.nvidia.com/deploy/nvidia-smi/.
"""
import csv
from datetime import datetime
import json
import math
from pathlib import Path
import platform
import shutil
import subprocess
import threading
import time

import perf_macos


DEVICE_FIELDS = ("timestamp", "index", "uuid", "name", "utilization.gpu", "utilization.memory",
                 "memory.used", "memory.total", "power.draw", "temperature.gpu",
                 "clocks.current.graphics", "clocks.current.memory", "pstate")
DEVICE_METRICS = {"utilization.gpu": "gpu_busy_percent", "utilization.memory": "memory_busy_percent",
                  "memory.used": "memory_used_mib", "memory.total": "memory_total_mib",
                  "power.draw": "power_w", "temperature.gpu": "temperature_c",
                  "clocks.current.graphics": "graphics_clock_mhz", "clocks.current.memory": "memory_clock_mhz"}
PROCESS_METRICS = {"sm": "sm_percent", "mem": "memory_busy_percent", "fb": "framebuffer_mb",
                   "enc": "encoder_percent", "dec": "decoder_percent"}
MACOS_INTERVAL_MS = 250
MACOS_DEVICE_METRICS = ("device_utilization_percent", "renderer_utilization_percent", "tiler_utilization_percent",
                        "in_use_system_memory_mib", "alloc_system_memory_mib")


def number(value):
    try:
        parsed = float(value)
        return parsed if math.isfinite(parsed) else None
    except (TypeError, ValueError):
        return None


class PmonParser:
    def __init__(self):
        self.header = []
        self.batches = {}

    def parse(self, line):
        if line.startswith("#"):
            fields = line.lstrip("#").lower().split()
            if {"gpu", "pid", "type", "date", "time"} <= set(fields):
                self.header = fields
            return None
        if not self.header or not line.strip():
            return None
        row = dict(zip(self.header, line.split(maxsplit=len(self.header) - 1)))
        stamp = datetime.strptime(row["date"] + " " + row["time"], "%Y%m%d %H:%M:%S").timestamp() * 1000
        gpu = str(int(row["gpu"]))
        batch = self.batches.get(gpu)
        if batch is None or batch[0] != stamp:
            # Track every GPU batch, including rows for unrelated processes, so
            # an absent target PID does not silently stretch its next interval.
            batch = (stamp, batch[0] if batch and stamp > batch[0] else None)
            self.batches[gpu] = batch
        if not row.get("pid", "").isdigit():
            return None
        return {"scope": "process", "gpu_index": gpu, "pid": int(row["pid"]),
                # Driver timestamps have only second precision. Use enclosing
                # bounds, not an invented millisecond-accurate observation time.
                "sample_start_unix_ms": stamp, "sample_end_unix_ms": stamp + 1000,
                "interval_start_unix_ms": batch[1], "interval_end_unix_ms": stamp + 1000,
                **{name: number(row.get(key)) for key, name in PROCESS_METRICS.items()}}


def parse_device(line):
    fields = next(csv.reader([line], skipinitialspace=True))
    if len(fields) != len(DEVICE_FIELDS):
        raise ValueError("unexpected device CSV columns")
    row = dict(zip(DEVICE_FIELDS, (s.strip() for s in fields)))
    stamp = datetime.strptime(row["timestamp"], "%Y/%m/%d %H:%M:%S.%f").timestamp() * 1000
    return {"scope": "device", "gpu_index": str(int(row["index"])), "uuid": row["uuid"], "name": row["name"],
            "pstate": row["pstate"],
            "sample_start_unix_ms": stamp, "sample_end_unix_ms": stamp + 1,
            # NVIDIA hardware utilization samples span up to one second.
            "interval_start_unix_ms": stamp - 1000, "interval_end_unix_ms": stamp + 1,
            **{name: number(row[key]) for key, name in DEVICE_METRICS.items()}}


class NvidiaSampler:
    """Two persistent nvidia-smi processes, drained off the app/sampling thread.

    Captures only named PIDs and device totals. Unrelated process details are
    parsed for timing boundaries but never stored. Failure is an explicit
    unavailable/partial result and does not prevent a normal UI capture.
    """
    def __init__(self, output, pids):
        self.output = Path(output)
        self.pids = dict(pids)
        self.lock = threading.Lock()
        self.workers = []
        self.counts = {"process": 0, "device": 0}
        self.roles = {"app": 0, "compositor": 0}
        self.errors = {}
        self.stream = (self.output / "gpu.jsonl").open("w", encoding="utf-8")
        self.metadata = {"requested": True, "backend": "nvidia-smi", "interval_ms": 1000,
                         "process_timestamp_precision_ms": 1000,
                         "status": "unavailable", "commands": {}, "pids": self.pids,
                         "note": "Process counters aggregate all windows. Device counters include other applications. Missing values are not zero."}
        if platform.system() == "Darwin":
            self._start_macos()
            return
        binary = shutil.which("nvidia-smi")
        if platform.system() != "Linux" or not binary:
            self.metadata["reason"] = "This collector requires Linux and nvidia-smi or macOS IOKit; other GPU/OS collectors are not implemented."
            return
        commands = {
            "process": [binary, "pmon", "-s", "um", "-d", "1", "-o", "DT"],
            "device": [binary, "--query-gpu=" + ",".join(DEVICE_FIELDS), "--format=csv,noheader,nounits", "-l", "1"],
        }
        for scope, command in commands.items():
            self.metadata["commands"][scope] = command
            stderr = (self.output / f"gpu-{scope}.stderr.log").open("w")
            try:
                process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                           stderr=stderr, text=True, encoding="utf-8", errors="replace")
            except OSError as error:
                stderr.close()
                self.metadata.setdefault("start_errors", {})[scope] = str(error)
                continue
            thread = threading.Thread(target=self._read, args=(scope, process), daemon=True,
                                      name=f"gpu-{scope}")
            self.workers.append((scope, process, thread, stderr))
            thread.start()

    def _start_macos(self):
        """In-process IOKit reads of cumulative per-process GPU nanoseconds
        (see perf_macos). WindowServer is the compositor role; its time
        includes every other application drawing on screen."""
        if not perf_macos.available():
            self.metadata["reason"] = "IOKit/CoreFoundation could not be loaded."
            return
        if self.pids.get("compositor") is None:
            self.pids["compositor"] = perf_macos.window_server_pid()
        self.metadata.update(backend="iokit-agx", interval_ms=MACOS_INTERVAL_MS, process_timestamp_precision_ms=None,
                             note="Process GPU time is the AGX driver's accumulated execution time of the process's "
                                  "Metal clients (all windows). The compositor is WindowServer, which also composites "
                                  "other applications. Device utilization is a whole-GPU driver sample. Missing values are not zero.")
        self.ioreport = perf_macos.IOReportSampler()
        self.metadata["soc_energy"] = "ioreport" if self.ioreport.ok else "unavailable"
        self.stop = threading.Event()
        thread = threading.Thread(target=self._sample_macos, daemon=True, name="gpu-macos")
        self.workers.append(("iokit", None, thread, None))
        thread.start()

    def _sample_macos(self):
        previous, previous_ms = self.ioreport.sample(), time.time() * 1000
        while not self.stop.is_set():
            started = time.time() * 1000
            pids = {role: pid for role, pid in self.pids.items() if pid is not None}
            try:
                times = perf_macos.gpu_times_ns(set(pids.values()))
                device = perf_macos.device_statistics()
            except (OSError, ValueError):
                self._error("process")
                times, device = None, None
            stamp = time.time() * 1000
            lines = []
            for role, pid in pids.items():
                if times is not None and pid in times:
                    lines.append({"scope": "process", "gpu_index": "0", "pid": pid, "role": role,
                                  "sample_unix_ms": stamp, "query_start_unix_ms": started,
                                  "gpu_time_ns": times[pid]})
            if device is not None:
                lines.append({"scope": "device", "gpu_index": "0", "sample_unix_ms": stamp,
                              "sample_start_unix_ms": stamp, "sample_end_unix_ms": stamp, **device})
            current = self.ioreport.sample()
            soc = self.ioreport.delta(previous, current)
            if soc is not None:
                lines.append({"scope": "soc", "gpu_index": "0", "interval_start_unix_ms": previous_ms,
                              "interval_end_unix_ms": stamp, **soc})
            self.ioreport.release(previous)
            previous, previous_ms = current, stamp
            with self.lock:
                for record in lines:
                    record["received_unix_ms"] = stamp
                    self.stream.write(json.dumps(record) + "\n")
                    if record["scope"] != "soc":
                        self.counts[record["scope"]] += 1
                    if record["scope"] == "process":
                        self.roles[record["role"]] = self.roles.get(record["role"], 0) + 1
                    elif record["scope"] == "soc":
                        self.counts["soc"] = self.counts.get("soc", 0) + 1
                self.stream.flush()
            self.stop.wait(MACOS_INTERVAL_MS / 1000)

    def set_app_pid(self, pid):
        # Reassignment also handles a profiler wrapper handing off to its child.
        self.pids = dict(self.pids, app=pid)

    def _error(self, scope):
        with self.lock:
            self.errors[scope] = self.errors.get(scope, 0) + 1

    def _read(self, scope, process):
        parser = PmonParser()
        try:
            for line in process.stdout:
                try:
                    record = parser.parse(line) if scope == "process" else parse_device(line)
                    if record is None:
                        continue
                    if scope == "process":
                        role = next((role for role, pid in self.pids.items() if pid == record["pid"]), None)
                        if role is None:
                            continue
                        record["role"] = role
                    record["received_unix_ms"] = time.time() * 1000
                    with self.lock:
                        self.stream.write(json.dumps(record) + "\n")
                        self.stream.flush()
                        self.counts[scope] += 1
                        if scope == "process":
                            self.roles[role] += 1
                except (ValueError, KeyError, IndexError, OverflowError):
                    self._error(scope)
        except OSError:
            self._error(scope)

    def close(self):
        exited_early = {}
        if getattr(self, "stop", None) is not None:
            self.stop.set()
            for _, _, thread, _ in self.workers:
                thread.join(timeout=5)
            self.workers = []
        for scope, process, _, _ in self.workers:
            code = process.poll()
            if code is None:
                process.terminate()
            else:
                exited_early[scope] = code
        for _, process, thread, stderr in self.workers:
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=2)
            thread.join(timeout=2)
            process.stdout.close()
            stderr.close()
        self.stream.close()
        if all(self.counts.values()) and self.roles["app"] and not (self.errors or exited_early or self.metadata.get("start_errors")):
            self.metadata["status"] = "available"
        elif any(self.counts.values()):
            self.metadata["status"] = "partial"
        self.metadata.update(pids=self.pids, samples=self.counts, process_samples=self.roles, parse_errors=self.errors,
                             exited_early=exited_early)
        (self.output / "gpu-capture.json").write_text(json.dumps(self.metadata, indent=2) + "\n")
        return self.metadata


def load(directory):
    path = Path(directory) / "gpu.jsonl"
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines()]


def Sampler(output, pids):
    if platform.system() == "Windows":
        from perf_gpu_windows import WindowsSampler
        return WindowsSampler(output, pids)
    return NvidiaSampler(output, pids)


def summarize_phase(records, begin_unix_ms, end_unix_ms, distribution):
    """Exclude boundary-straddling utilization buckets and unprimed first samples.

    Short phases can legitimately have no utilization samples. Keep per-device
    counters and coverage counts; never sum percentages across different GPUs.
    """
    if any("gpu_time_ns" in row for row in records):
        return summarize_macos_phase(records, begin_unix_ms, end_unix_ms, distribution)
    result = {"processes": {}, "devices": {}}
    groups = {}
    for row in records:
        key = (row["scope"], row.get("role"), row["gpu_index"])
        groups.setdefault(key, []).append(row)
    for (scope, role, gpu), rows in groups.items():
        windows = any(r.get("backend") == "windows-pdh" for r in rows)
        metrics = ("busiest_engine_percent", "dedicated_mib", "shared_mib", "committed_mib") if windows else (PROCESS_METRICS.values() if scope == "process" else DEVICE_METRICS.values())
        values = {}
        for metric in metrics:
            interval = metric.endswith("percent")
            prefix = "interval" if interval else "sample"
            inside = [r for r in rows if r.get(prefix + "_start_unix_ms") is not None
                      and begin_unix_ms <= r[prefix + "_start_unix_ms"]
                      and r[prefix + "_end_unix_ms"] <= end_unix_ms]
            data = distribution(r.get(metric) for r in inside)
            data["missing"] = sum(r.get(metric) is None for r in inside)
            if interval:
                available = [r for r in inside if r.get(metric) is not None]
                duration = sum(r["interval_end_unix_ms"] - r["interval_start_unix_ms"] for r in available)
                data["covered_ms"] = duration
                data["duration_weighted_mean"] = sum(r[metric] * (r["interval_end_unix_ms"] - r["interval_start_unix_ms"]) for r in available) / duration if duration else None
            values[metric] = data
        identity = {key: rows[-1].get(key) for key in ("uuid", "name") if key in rows[-1]}
        if windows:
            engine_rows = [r for r in rows if r.get("interval_start_unix_ms") is not None
                           and begin_unix_ms <= r["interval_start_unix_ms"]
                           and r["interval_end_unix_ms"] <= end_unix_ms]
            # PDH also emits process-memory rows for adapters with no engines
            # used by this process. Preserve their memory, but do not mistake
            # their missing occupancy for a failed active rendering adapter.
            identity["engine_instances_observed"] = len({name for r in engine_rows for name in r.get("engines",{})})
        group = result["processes"].setdefault(role, {}) if scope == "process" else result["devices"]
        group[gpu] = {**identity, **values}
    return result


def summarize_macos_phase(records, begin_unix_ms, end_unix_ms, distribution):
    """Cumulative GPU nanoseconds per role between the first and last samples
    inside the phase, as busy milliseconds per second of that span. A phase
    shorter than two samples has no rate."""
    result = {"processes": {}, "devices": {}}
    roles = {}
    for row in records:
        if row["scope"] == "process" and begin_unix_ms <= row["sample_unix_ms"] <= end_unix_ms:
            roles.setdefault(row["role"], []).append(row)
    for role, rows in roles.items():
        rows.sort(key=lambda r: r["sample_unix_ms"])
        first, last = rows[0], rows[-1]
        span_s = (last["sample_unix_ms"] - first["sample_unix_ms"]) / 1000
        # A client created or destroyed mid-phase can move the sum backwards.
        delta_ns = last["gpu_time_ns"] - first["gpu_time_ns"]
        valid = len(rows) >= 2 and span_s > 0 and delta_ns >= 0
        result["processes"].setdefault(role, {})[first["gpu_index"]] = {
            "pid": last["pid"], "samples": len(rows), "span_s": span_s,
            "gpu_ms": delta_ns / 1e6 if valid else None,
            "gpu_ms_per_s": delta_ns / 1e6 / span_s if valid else None,
            "gpu_busy_percent": delta_ns / 1e7 / span_s if valid else None}
    # IOReport intervals fully inside the phase: GPU energy and clock states.
    soc = [row for row in records if row["scope"] == "soc"
           and begin_unix_ms <= row["interval_start_unix_ms"] and row["interval_end_unix_ms"] <= end_unix_ms]
    if soc:
        span_s = sum(row["interval_end_unix_ms"] - row["interval_start_unix_ms"] for row in soc) / 1000
        energy = [row["gpu_energy_nj"] for row in soc if row.get("gpu_energy_nj") is not None]
        states = {}
        for row in soc:
            for name, ticks in row.get("gpu_states", {}).items():
                states[name] = states.get(name, 0) + ticks
        total = sum(states.values())
        active = {name: ticks for name, ticks in states.items() if name not in ("OFF", "IDLE", "DOWN")}
        active_total = sum(active.values())
        result["soc"] = {
            "intervals": len(soc), "span_s": span_s,
            "gpu_energy_mj": sum(energy) / 1e6 if energy else None,
            "gpu_power_mw": sum(energy) / 1e6 / span_s if energy and span_s else None,
            "gpu_active_fraction": active_total / total if total else None,
            "gpu_state_fractions": {name: ticks / total for name, ticks in states.items()} if total else {},
            # Mean P-state index while active (P1 = 1): higher means higher clocks.
            "gpu_active_mean_pstate": sum(int(name[1:]) * ticks for name, ticks in active.items()
                                          if name[1:].isdigit()) / active_total if active_total else None,
        }
    devices = [row for row in records if row["scope"] == "device"
               and begin_unix_ms <= row["sample_unix_ms"] <= end_unix_ms]
    if devices:
        result["devices"]["0"] = {metric: distribution(row.get(metric) for row in devices)
                                  for metric in MACOS_DEVICE_METRICS}
    return result
