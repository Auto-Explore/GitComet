"""Measurement policy and run-level comparisons, independent of collectors."""
import math
import json
import random
import re
import statistics

POLICY = {"regression_ratio": 1.20, "pairs": 5, "sessions": 2,
          "input_p95_ms": 50, "input_p99_ms": 100, "stall_ms": 100,
          "severe_stall_ms": 1000}

# Initial investigation thresholds, not benchmark claims or shipping budgets.
# Debug assertions come from the running binary, not its filename/profile flag.
DEV_POLICY = {"draw_p95_ms": 50, "input_p95_ms": 100, "input_p99_ms": 250,
              "stall_ms": 250, "severe_stall_ms": 1000}


def build_mode(result):
    return "dev" if result.get("debug_assertions") else "release" if result.get("debug_assertions") is False else "unknown"


def exit_status(cases, strict=False):
    # Optional unavailable tools may be skipped locally; a strict selected
    # suite must not succeed with missing cases, even if it has no timings.
    return int(any(case["status"] == "failed" or
                   (strict and (case["status"] != "passed" or case.get("findings"))) for case in cases))


def perf_quality(report, returncode=0):
    lost = re.search(r"Total Lost Samples:\s*(\d+)", report)
    count = int(lost[1]) if lost else None
    return {"valid": returncode == 0 and count == 0, "lost_samples": count,
            "note": "Missing or lost profiler samples require a new capture; UI witness validity is separate."}


def percentile(values, fraction):
    values = sorted(values)
    return values[max(0, math.ceil(len(values) * fraction) - 1)] if values else None


def metrics(result):
    """Only comparable quantities, with units in the names. No invented zeros."""
    out = {}
    for key in ("milliseconds", "cpu_s", "bytes", "alloc_ops", "alloc_bytes", "retained_bytes"):
        if isinstance(result.get(key), (int, float)):
            out[key] = result[key]
    for phase, data in result.get("phases", {}).items():
        for metric in ("draw_ms", "submit_ms", "wake_ms", "dirty_to_draw_ms", "rss_kib", "pss_kib", "private_kib"):
            for quantile in ("p50", "p95", "p99", "max"):
                value = data.get(metric, {}).get(quantile)
                if value is not None:
                    out[f"{phase}.{metric}.{quantile}"] = value
        for metric in ("handler_ms", "dispatch_delay_ms", "input_to_draw_ms", "store_queue_ms", "task_queue_ms", "apply_ms"):
            for quantile in ("p50", "p95", "p99", "max"):
                value = data.get("inputs", {}).get(metric, {}).get(quantile)
                if value is not None:
                    out[f"{phase}.{metric}.{quantile}"] = value
        for metric in ("process_cpu_cores", "wakeups_per_second", "invalidations", "threads", "fds", "handles"):
            if data.get(metric) is not None:
                out[f"{phase}.{metric}"] = data[metric]
        for counter, value in data.get("work_counts", {}).items():
            out[f"{phase}.work.{counter}"] = value
        for counter, value in data.get("work_units", {}).items():
            out[f"{phase}.work_units.{counter}"] = value
    for counter, value in result.get("work_units", {}).items():
        out[f"total.work_units.{counter}"] = value
    for metric, value in (result.get("startup") or {}).items():
        if value is not None:
            out[f"startup.{metric}"] = value
    for phase in result.get("allocation_phases", []):
        for key in ("alloc_ops", "alloc_bytes", "net_alloc_bytes", "realloc_ops"):
            if phase.get(key) is not None:
                out[f"{phase['phase']}.{key}"] = phase[key]
    for operation in result.get("operation_results", []):
        for key in ("milliseconds", "cancel_ms"):
            if operation.get(key) is not None:
                out[f"operation.{operation['name']}.{key}"] = operation[key]
    return out


def findings(result, refresh_hz=60):
    issues = []
    mode = build_mode(result)
    policy = DEV_POLICY if mode == "dev" else POLICY
    if not result.get("valid", True):
        issues.append({"kind": "invalid_capture", "severity": "error", "evidence": result.get("problems", [])})
    for operation in result.get("unfinished_operations", []):
        if operation.get("cancel_pending_ms") is not None:
            issues.append({"kind": "cancellation_not_completed", "severity": "high", **operation})
    for operation in result.get("operation_results", []):
        if operation.get("overlapping_inputs") == 0:
            issues.append({"kind": "concurrency_not_exercised", "severity": "investigate",
                           "operation": operation["name"],
                           "note": "Operation completed without concurrent input; increase transport latency or payload before claiming responsiveness coverage."})
    for name, value in metrics(result).items():
        if result.get("allocation_tracking"):
            continue  # Diagnostic allocator timings are not shipping alerts.
        limit = None
        if name.endswith("draw_ms.p95") and not name.endswith(("input_to_draw_ms.p95", "dirty_to_draw_ms.p95")) and refresh_hz:
            limit = policy["draw_p95_ms"] if mode == "dev" else 1000 / refresh_hz
        elif name.endswith("input_to_draw_ms.p95"):
            limit = policy["input_p95_ms"]
        elif name.endswith("input_to_draw_ms.p99"):
            limit = policy["input_p99_ms"]
        elif name.endswith(("handler_ms.max", "wake_ms.max", "dispatch_delay_ms.max", "apply_ms.max", "draw_ms.max")):
            limit = policy["stall_ms"]
        if limit is not None and value > limit:
            issues.append({"kind": "target_exceeded", "metric": name, "value": value, "limit": limit,
                           "build_mode": mode,
                           "severity": "high" if value >= policy["severe_stall_ms"] else "investigate"})
    for metric, retention in (result.get("retention") or {}).items():
        growth = retention.get("growth_per_cycle")
        if growth is not None and growth > 0:
            issues.append({"kind": "retention_candidate", "severity": "investigate", "metric": metric,
                           "growth_per_cycle": growth,
                           "note": "Confirm at a second cycle count; RSS/PSS growth alone is not a heap leak."})
    for stall in ([] if result.get("allocation_tracking") else result.get("long_frames", [])):
        issues.append({"kind": "long_frame", "severity": "high", "build_mode": mode, **stall})
    return issues


def ranked_findings(manifest):
    """A review queue with reproducible evidence, without diagnosing from timing alone."""
    rows = []
    for case in manifest["cases"]:
        issues = case.get("findings", [])
        if case["status"] != "passed" and not any(i["kind"] == "invalid_capture" for i in issues):
            issues = [*issues, {"kind": "unusable_case", "severity": "error", "evidence": case.get("reason", case["status"])}]
        for issue in issues:
            result = case.get("result", {})
            phase = issue.get("metric", "").split(".")[0]
            data = result.get("phases", {}).get(phase, {})
            rows.append({**issue, "case": case["key"], "pair": case["pair"], "variant": case["variant"],
                         "build_mode": build_mode(result), "measurement_kind": case.get("measurement_kind"),
                         "artifacts": case.get("directory"),
                         "work_units": data.get("work_units", result.get("work_units", {})),
                         "thread_cpu_ms": data.get("thread_cpu_ms", {}),
                         "thread_runqueue_wait_ms": data.get("thread_runqueue_wait_ms", {})})
    rank = {"error": 0, "high": 1, "investigate": 2}
    return sorted(rows, key=lambda r: (rank[r["severity"]], -r.get("value", 0) / (r.get("limit") or 1), r["case"]))


def bootstrap_ratios(pairs, seed=7):
    ratios = [candidate / baseline for baseline, candidate in pairs if baseline > 0]
    if not ratios:
        return None
    rng = random.Random(seed)
    samples = [statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(4000)]
    return {"median": statistics.median(ratios), "ci95": [percentile(samples, .025), percentile(samples, .975)]}


def compare(manifests, allocations=False):
    """Match actual pairs; never pool frames or silently accept missing cases."""
    pairs, errors, skipped, definitions, contexts = {}, [], [], {}, {}
    excluded_metrics = []
    if not manifests:
        errors.append("No runs supplied")
    first_environment = manifests[0]["comparison_environment"] if manifests else None
    first_binaries = None
    for manifest in manifests:
        identities = {name: data["sha256"] for name, data in manifest.get("binaries", {}).items()}
        if first_binaries is None:
            first_binaries = identities
        if identities != first_binaries or manifest["comparison_environment"] != first_environment:
            errors.append(f"Changed binaries or environment between sessions: {manifest['session']}")
        for case in manifest["cases"]:
            if case["status"] != "passed":
                skipped.append({"case": case["key"], "status": case["status"], "reason": case.get("reason")})
                errors.append(f"Unusable pair member: {manifest['session']} {case['key']} {case['variant']} ({case['status']})")
                continue
            if not case["result"].get("valid", True):
                errors.append(f"Invalid capture labelled passed: {case['key']}")
                continue
            definition = definitions.setdefault(case["key"], case["workload"])
            if definition != case["workload"]:
                errors.append(f"Changed workload between pairs: {case['key']}")
            context = (case["result"].get("graphics"), case["result"].get("debug_assertions"), case["result"].get("probe_interval_ms"))
            if contexts.setdefault(case["key"], context) != context:
                errors.append(f"Changed renderer, build mode or probe interval between pairs: {case['key']}")
            identity = (manifest["session"], case["pair"], case["key"])
            group = pairs.setdefault(identity, {})
            variant = case["variant"]
            if variant in group:
                errors.append(f"Duplicate pair member: {identity} {variant}")
            group[variant] = (manifest, case)
    measured = {}
    for identity, variants in pairs.items():
        if set(variants) != {"baseline", "candidate"}:
            errors.append(f"Incomplete pair: {identity}")
            continue
        (bm, b), (cm, c) = variants["baseline"], variants["candidate"]
        if any(x.get("measurement_kind") == "validation" for x in (b, c)):
            errors.append(f"Functional validation is not a timing sample: {identity}")
            continue
        if bm["comparison_environment"] != cm["comparison_environment"] or b["workload"] != c["workload"]:
            errors.append(f"Incomparable environment or workload: {identity}")
            continue
        if b["result"].get("graphics") != c["result"].get("graphics") or b["result"].get("debug_assertions") != c["result"].get("debug_assertions"):
            errors.append(f"Different application renderer or build mode: {identity}")
            continue
        if not allocations and (b.get("measurement_kind") == "diagnostic" or c.get("measurement_kind") == "diagnostic"):
            errors.append(f"Diagnostic timing is not comparable: {identity}")
            continue
        if allocations and not all(x["result"].get("allocation_tracking") for x in (b, c)):
            errors.append(f"Allocation comparison requires two allocation captures: {identity}")
            continue
        for phase in set(b["result"].get("phases", {})) | set(c["result"].get("phases", {})):
            bi = b["result"].get("phases", {}).get(phase, {}).get("inputs", {})
            ci = c["result"].get("phases", {}).get(phase, {}).get("inputs", {})
            if bi.get("count") != ci.get("count") or abs(bi.get("witnessed", 0) - ci.get("witnessed", 0)) > .05 * max(bi.get("witnessed", 0), ci.get("witnessed", 0), 1):
                errors.append(f"Different input completion rates: {identity} {phase}")
                break
        else:
            baseline, candidate = metrics(b["result"]), metrics(c["result"])
            if allocations:
                baseline = {k: v for k, v in baseline.items() if k.endswith((".alloc_ops", ".alloc_bytes", ".realloc_ops"))}
                candidate = {k: v for k, v in candidate.items() if k.endswith((".alloc_ops", ".alloc_bytes", ".realloc_ops"))}
            # Stage counters genuinely absent from a complete trace are zero;
            # unavailable sampled metrics must remain unavailable.
            for metric in baseline.keys() | candidate.keys():
                if ".work." in metric:
                    baseline.setdefault(metric, 0)
                    candidate.setdefault(metric, 0)
            # An input need not dispatch store/worker work. A complete trace
            # with zero such stages has no queue/apply latency, not missing
            # telemetry and not a zero-duration sample. Keep its work counters
            # comparable, and explicitly exclude the inapplicable distribution.
            for metric in sorted(baseline.keys() ^ candidate.keys()):
                parts = metric.rsplit(".", 2)
                if len(parts) != 3 or parts[1] not in ("store_queue_ms", "task_queue_ms", "apply_ms"):
                    continue
                phase, stage, _ = parts
                counts = [case["result"].get("phases", {}).get(phase, {}).get("inputs", {}).get(stage, {}).get("count")
                          for case in (b, c)]
                if None not in counts and 0 in counts:
                    baseline.pop(metric, None)
                    candidate.pop(metric, None)
                    excluded_metrics.append({"session": identity[0], "pair": identity[1], "case": identity[2],
                                             "metric": metric, "stage_samples": counts,
                                             "reason": "No applicable stage samples on one side; see work counters."})
            if baseline.keys() != candidate.keys():
                errors.append(f"Different available metrics: {identity}")
            for metric in baseline.keys() & candidate.keys():
                measured.setdefault((identity[2], metric), []).append((identity[0], baseline[metric], candidate[metric]))
    aa = bool(first_binaries and first_binaries.get("baseline")
              and first_binaries.get("baseline") == first_binaries.get("candidate")
              and first_binaries.get("baseline-backend") == first_binaries.get("candidate-backend"))
    rows = []
    for (case, metric), samples in sorted(measured.items()):
        estimate = bootstrap_ratios([(b, c) for _, b, c in samples])
        ratio_samples = [(s, b, c) for s, b, c in samples if b > 0]
        sufficient = len(ratio_samples) >= POLICY["pairs"] and len({s for s, _, _ in ratio_samples}) >= POLICY["sessions"]
        worse = estimate is not None and estimate["median"] >= POLICY["regression_ratio"] and estimate["ci95"][0] > 1
        rows.append({"case": case, "metric": metric, "pairs": len(samples), "sessions": len({s for s, _, _ in samples}),
                     "ratio_pairs": len(ratio_samples),
                     "baseline": statistics.median(b for _, b, _ in samples),
                     "candidate": statistics.median(c for _, _, c in samples), "ratio": estimate,
                     "verdict": "ratio_unavailable" if estimate is None else ("noise_alert" if aa else "regression") if worse and sufficient
                     else "needs_confirmation" if worse else "no_detected_regression"})
    if not rows:
        errors.append("No complete comparable measurements")
    return {"version": 1, "policy": POLICY, "measurement_kind": "allocation_counts" if allocations else "timing",
            "experiment": "A/A" if aa else "A/B",
            "valid": not errors, "errors": errors,
            "excluded": skipped, "excluded_metrics": excluded_metrics, "comparisons": rows}


def render(manifest):
    lines = [f"GitComet performance: {manifest['session']}",
             "Timing targets are alerts; dev and release use separate thresholds. Failed witnesses are validation failures.",
             "See findings.json for ranked evidence and capture directories."]
    for case in manifest["cases"]:
        lines.append(f"{case['status']:8} {case['variant']:9} pair={case['pair']} {case['key']} ({build_mode(case.get('result', {}))})")
        if case.get("reason"):
            lines.append(f"  {case['reason']}")
        result = case.get("result", {})
        if result.get("startup"):
            startup = result["startup"]
            lines.append(f"  startup: first draw={startup.get('spawn_to_first_draw_ms')} ms; "
                         f"first page={startup.get('spawn_to_ready_ms')} ms; "
                         f"indexed rows ready={startup.get('spawn_to_indexed_ready_ms')} ms")
        if result.get("milliseconds") is not None:
            lines.append(f"  operation: {result['milliseconds']:.2f} ms; verified bytes: {result.get('bytes', 'unavailable')}")
        for phase, data in result.get("phases", {}).items():
            draw = data.get("draw_ms", {}).get("p95")
            latency = data.get("inputs", {}).get("input_to_draw_ms", {}).get("p95")
            lines.append(f"  {phase}: draw p95={draw} ms; witnessed input-to-draw p95={latency} ms; frames={data.get('frames')}")
            if data.get("work_units"):
                lines.append(f"    work: {json.dumps({k: v for k, v in data['work_units'].items() if v}, sort_keys=True)}")
        for operation in result.get("operation_results", []):
            lines.append(f"  {operation['name']}: {operation['milliseconds']:.2f} ms; overlapping inputs={operation['overlapping_inputs']}; cancellation={operation.get('cancel_ms')} ms")
        for issue in case.get("findings", []):
            detail = {key: value for key, value in issue.items() if key not in ("severity", "kind")}
            lines.append(f"  {issue['severity']}: {issue['kind']} {json.dumps(detail, sort_keys=True)}")
    return "\n".join(lines) + "\n"
