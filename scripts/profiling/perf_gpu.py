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


DEVICE_FIELDS = ("timestamp", "index", "uuid", "name", "utilization.gpu", "utilization.memory",
                 "memory.used", "memory.total", "power.draw", "temperature.gpu",
                 "clocks.current.graphics", "clocks.current.memory", "pstate")
DEVICE_METRICS = {"utilization.gpu": "gpu_busy_percent", "utilization.memory": "memory_busy_percent",
                  "memory.used": "memory_used_mib", "memory.total": "memory_total_mib",
                  "power.draw": "power_w", "temperature.gpu": "temperature_c",
                  "clocks.current.graphics": "graphics_clock_mhz", "clocks.current.memory": "memory_clock_mhz"}
PROCESS_METRICS = {"sm": "sm_percent", "mem": "memory_busy_percent", "fb": "framebuffer_mb",
                   "enc": "encoder_percent", "dec": "decoder_percent"}


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


class Sampler:
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
        binary = shutil.which("nvidia-smi")
        if platform.system() != "Linux" or not binary:
            self.metadata["reason"] = "This collector requires Linux and nvidia-smi; other GPU/OS collectors are not implemented."
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


def summarize_phase(records, begin_unix_ms, end_unix_ms, distribution):
    """Exclude boundary-straddling utilization buckets and unprimed first samples.

    Short phases can legitimately have no utilization samples. Keep per-device
    counters and coverage counts; never sum percentages across different GPUs.
    """
    result = {"processes": {}, "devices": {}}
    groups = {}
    for row in records:
        key = (row["scope"], row.get("role"), row["gpu_index"])
        groups.setdefault(key, []).append(row)
    for (scope, role, gpu), rows in groups.items():
        metrics = PROCESS_METRICS.values() if scope == "process" else DEVICE_METRICS.values()
        values = {}
        for metric in metrics:
            interval = metric.endswith("percent")
            prefix = "interval" if interval else "sample"
            inside = [r for r in rows if r.get(prefix + "_start_unix_ms") is not None
                      and begin_unix_ms <= r[prefix + "_start_unix_ms"]
                      and r[prefix + "_end_unix_ms"] <= end_unix_ms]
            data = distribution(r.get(metric) for r in inside)
            data["missing"] = sum(r.get(metric) is None for r in inside)
            values[metric] = data
        identity = {key: rows[-1].get(key) for key in ("uuid", "name") if key in rows[-1]}
        group = result["processes"].setdefault(role, {}) if scope == "process" else result["devices"]
        group[gpu] = {**identity, **values}
    return result
