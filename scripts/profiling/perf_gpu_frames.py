"""Renderer timestamps, kept separate from CPU submission and device utilization.

Frames belong to the phase in which they were submitted, even when asynchronous
readback arrives later. Unknown or dropped measurements never become zero cost.
Allocation counters are renderer estimates, not driver residency measurements.
"""
from collections import Counter, defaultdict
import math


def summarize(records, begin_ms, end_ms, distribution):
    frames = sorted((r for r in records if r.get("event") == "gpu_frame"
                     and begin_ms <= r["at_ms"] < end_ms),
                    key=lambda r: (r["at_ms"], r.get("submission_id", 0)))
    available = [r for r in frames if r.get("status") == "available"
                 and isinstance(r.get("gpu_duration_ns"), (int, float))
                 and math.isfinite(r["gpu_duration_ns"]) and r["gpu_duration_ns"] >= 0]
    # Only complete pass instrumentation can establish that an absent pass cost
    # zero. Metal's command-buffer-only fallback cannot establish that.
    measured_passes = [r for r in available if r.get("pass_timing_complete", False)]
    names = sorted({p["name"] for r in measured_passes for p in r.get("passes", [])})
    passes = {name: [] for name in names}
    for frame in measured_passes:
        durations = defaultdict(float)
        for part in frame.get("passes", []):
            durations[part["name"]] += part["duration_ns"] / 1e6
        for name in names:
            passes[name].append(durations[name])
    memory = defaultdict(list)
    windows = Counter()
    for frame in frames:
        windows[frame.get("window", str(frame.get("window_id")))] += 1
        for category, value in frame.get("memory", {}).items():
            if value is not None:
                memory[category].append(value)
    counters = sorted((r["at_ms"], r["gpu_records_dropped"]) for r in records
                      if r.get("event") == "interval" and "gpu_records_dropped" in r)
    counter_at = lambda at: next((value for time, value in reversed(counters) if time <= at), 0)
    submits = sum(1 for r in records if r.get("event") == "submit"
                  and begin_ms <= r["at_ms"] < end_ms)
    ids = defaultdict(set)
    for frame in frames:
        if "renderer_id" in frame and "submission_id" in frame:
            ids[frame["renderer_id"]].add(frame["submission_id"])
    missing = sum(max(values) - min(values) + 1 - len(values) for values in ids.values() if values)
    return {
        "missing_submission_records": missing,
        "status": "available" if available else "unavailable",
        "recorded_frames": len(frames), "cpu_submission_records": submits,
        "timed_frames": len(available),
        "coverage": len(available) / len(frames) if frames else None,
        "pass_timed_frames": len(measured_passes),
        "statuses": dict(Counter(r.get("status", "unknown") for r in frames)),
        "backend": sorted({r.get("backend", "unknown") for r in frames}),
        "implemented_experiments": sorted(set.intersection(*(set(r.get("implemented_experiments", [])) for r in frames))) if frames else [],
        "frame_ms": distribution(r["gpu_duration_ns"] / 1e6 for r in available),
        "timed_frame_ms_total": sum(r["gpu_duration_ns"] / 1e6 for r in available),
        "recorded_frames_per_second": len(frames) * 1000 / (end_ms - begin_ms) if end_ms > begin_ms else None,
        "pass_ms": {name: distribution(values) for name, values in passes.items()},
        "path_vertices": distribution(r.get("path_vertices") for r in frames),
        "pass_count": distribution(len(r.get("passes", [])) for r in measured_passes),
        "cache_hits": sum(r.get("cache_hits", 0) for r in frames),
        "cache_misses": sum(r.get("cache_misses", 0) for r in frames),
        "allocation_bytes": {name: {"initial": values[0], "peak": max(values), "settled": values[-1]}
                             for name, values in memory.items()},
        "windows": dict(windows),
        "query_samples_dropped": sum(r.get("query_samples_dropped", 0) for r in frames),
        "records_dropped": max(0, counter_at(end_ms) - counter_at(begin_ms)) if counters else None,
    }
