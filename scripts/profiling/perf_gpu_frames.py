"""GPU results use submission time; render elapsed is not scanout or power."""
from collections import Counter, defaultdict
import math


def finite(value):
    return isinstance(value, (float, int)) and math.isfinite(value) and value >= 0


def capability_payload(record):
    """Explicit nested payloads are authoritative; malformed data stays missing."""
    payload = record.get("detail", record)
    return payload if isinstance(payload, dict) else {}


def optional_labels(records, name):
    """Distinguish a reported null (no value) from legacy absent telemetry."""
    valid = [r[name] for r in records if isinstance(r.get(name), str) and r[name]]
    nulls = sum(name in r and r[name] is None for r in records)
    missing = sum(name not in r for r in records)
    malformed = len(records) - len(valid) - nulls - missing
    return {"counts": dict(Counter(valid)), "records": len(records),
            "reported": len(valid) + nulls, "null": nulls, "missing": missing,
            "malformed": malformed,
            "coverage": (len(valid) + nulls) / len(records) if records else None}


def summarize(records, begin_ms, end_ms, distribution):
    frames = sorted((r for r in records if r.get("event") == "gpu_frame"
                     and begin_ms <= r.get("at_ms", -1) < end_ms),
                    key=lambda r: (r["at_ms"], r.get("submission_id", 0)))
    duration = lambda r: r.get("elapsed_ns", r.get("gpu_duration_ns"))
    available = [r for r in frames if r.get("status") in ("complete", "available") and finite(duration(r))]
    capabilities = [capability_payload(r) for r in records if r.get("event") == "renderer_capabilities"]
    implemented = sorted(set.intersection(*(set(r.get("implemented_experiments", [])) for r in capabilities))) if capabilities else []
    pass_complete = all(r.get("per_pass_timestamps", False) for r in capabilities) if capabilities else False
    measured_passes = [r for r in available if r.get("pass_timing_complete", pass_complete)
                       and all(finite(p.get("elapsed_ns",p.get("duration_ns"))) for p in r.get("passes",[]))]
    names = sorted({p["name"] for r in measured_passes for p in r.get("passes", [])})
    passes = {name: [] for name in names}
    for frame in measured_passes:
        durations = defaultdict(float)
        for part in frame.get("passes", []):
            value = part.get("elapsed_ns", part.get("duration_ns"))
            if finite(value):
                durations[part["name"]] += value / 1e6
        for name in names:
            passes[name].append(durations[name])
    memory = defaultdict(list)
    for frame in frames:
        values = dict(frame.get("memory", {}))
        if "retained_bytes" in frame:
            values["retained_bytes"] = frame["retained_bytes"]
        for category, value in values.items():
            if finite(value):
                memory[category].append(value)
    submissions = [r for r in records if r.get("event") == "submission"
                   and begin_ms <= r.get("start_ms", r.get("at_ms", -1)) < end_ms]
    # Global IDs interleave windows. Compare exact membership, not ID gaps.
    key = lambda r: (r.get("window", r.get("window_id")), r.get("submission_id"))
    expected, recorded = {key(r) for r in submissions}, {key(r) for r in frames}
    counts = Counter(key(r) for r in frames)
    total = sum(duration(r) / 1e6 for r in available)
    seconds = (end_ms - begin_ms) / 1000
    def counter(name):
        values=[r[name] for r in available if isinstance(r.get(name),int) and not isinstance(r[name],bool)
                and 0 <= r[name] < 1 << 64]
        data=distribution(values)
        complete=bool(available) and len(values)==len(available)
        data.update(timed_frames=len(available),missing=len(available)-len(values),
                    coverage=len(values)/len(available) if available else None,
                    complete=complete,observed_sum=sum(values) if values else None,
                    sum=sum(values) if complete else None,
                    per_second=sum(values)/seconds if complete and seconds>0 else None)
        return data
    counters = sorted((r["at_ms"], r["gpu_records_dropped"]) for r in records
                      if r.get("event") == "interval" and "gpu_records_dropped" in r)
    counter_at = lambda at: next((value for time, value in reversed(counters) if time <= at), 0)
    return {
        "status": "available" if available else "unavailable",
        "statuses": dict(Counter(r.get("status", "unknown") for r in frames)),
        "recorded_frames": len(frames), "timed_frames": len(available),
        "all_submission_records": len(submissions),
        "cpu_submission_records": sum(r.get("event") == "submit" and begin_ms <= r.get("at_ms", -1) < end_ms for r in records),
        "coverage": len(available) / len(submissions) if submissions else None,
        "missing_submission_records": len(expected - recorded),
        "unexpected_submission_records": len(recorded - expected),
        "duplicate_submission_records": sum(count - 1 for count in counts.values()),
        "pass_timed_frames": len(measured_passes),
        "backend": sorted({r.get("backend", "unknown") for r in frames}),
        "implemented_experiments": implemented,
        "active_experiments": sorted({e for r in frames for e in r.get("experiments", [])}),
        "active_experiment_sets": dict(Counter(",".join(sorted(set(r.get("experiments",["unreported"])))) for r in frames)),
        "diagnostic_ablation_sets": dict(Counter(",".join(sorted(set(r.get("diagnostic_ablations",["unreported"])))) for r in frames)),
        "frame_ms": distribution(duration(r) / 1e6 for r in available),
        "timed_frame_ms_total": total,
        "render_elapsed_ms_per_second": total / seconds if seconds > 0 else None,
        "gpu_execution_union_ms_per_second": None,
        "execution_union_status": "requires ETW GPU execution intervals; render elapsed includes gaps",
        "recorded_frames_per_second": len(frames) / seconds if seconds > 0 else None,
        "damage_kinds": dict(Counter(r.get("damage_kind", "unknown") for r in frames)),
        "damage_fallback_reasons": optional_labels(frames, "damage_fallback_reason"),
        "surface_alpha_modes": optional_labels(capabilities, "surface_alpha_mode"),
        "pass_ms": {name: distribution(values) for name, values in passes.items()},
        "path_vertices": distribution(r.get("path_vertices") for r in frames),
        "pass_count": distribution(len(r.get("passes", [])) for r in measured_passes),
        "quad_draw_calls": counter("quad_draw_calls"),
        "quad_scissor_splits": counter("quad_scissor_splits"),
        "allocation_bytes": {name: {"initial": values[0], "peak": max(values), "settled": values[-1]}
                             for name, values in memory.items()},
        "windows": dict(Counter(r.get("window", str(r.get("window_id"))) for r in frames)),
        "query_samples_dropped": sum(r.get("query_samples_dropped", 0) for r in frames),
        "records_dropped": max(0, counter_at(end_ms) - counter_at(begin_ms)) if counters else None,
    }


def capability_errors(records, requested=(), backend=None, timings=True):
    capabilities = [capability_payload(r) for r in records if r.get("event") == "renderer_capabilities"]
    if not capabilities:
        return ["renderer capability record missing"]
    problems = []
    for capability in capabilities:
        malformed = []
        for name in ("implemented_experiments", "active_experiments"):
            value = capability.get(name)
            if not isinstance(value, list) or any(not isinstance(token, str) for token in value):
                malformed.append(f"renderer capability {name} missing or malformed")
        if malformed:
            problems += malformed
            continue
        if backend and capability.get("backend") != backend:
            problems.append(f"expected backend {backend}, observed {capability.get('backend')}")
        if not set(requested).issubset(capability.get("implemented_experiments", [])):
            problems.append("requested renderer experiment is not implemented")
        if set(capability.get("active_experiments", ["unreported"])) != set(requested):
            problems.append("actual active renderer experiments do not match requested flags")
        if timings and not (capability.get("gpu_timestamps") is True and capability.get("timing_enabled") is True):
            problems.append(f"GPU timestamps unavailable: {capability.get('unavailable_reason')}")
    return sorted(set(problems))
