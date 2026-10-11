"""Causal row paint -> exact scene revision -> all-present submission joins."""
from collections import Counter
import json
from pathlib import Path


def fields(record):
    return dict(record, **record.get("detail", {}))


def sequence(record):
    return record.get("sequence", record.get("input_seq", record.get("op")))


def summarize(records, begin_ms, end_ms, distribution, replay=()):
    rows = [fields(r) for r in records]
    targets = {sequence(r): r for r in rows if r.get("event") == "hover_target"
               and begin_ms <= r.get("at_ms", -1) < end_ms}
    requested = {sequence(r) for r in rows if r.get("event") == "scenario_input"
                 and begin_ms <= r.get("at_ms", -1) < end_ms}
    requested = requested or set(targets)
    paints = {sequence(r): r for r in rows if r.get("event") == "hover_paint"}
    superseded = {sequence(r) for r in rows if r.get("event") == "hover_superseded"}
    arrivals = {sequence(r): r for r in rows if r.get("event") == "native_input_arrival"}
    unchanged = {sequence(r) for r in rows if r.get("event") in ("hover_unchanged", "hover_same_target_pending")}
    submissions = sorted((r for r in rows if r.get("event") == "submission"), key=lambda r: r["at_ms"])
    start = next((r for r in rows if r.get("event") == "start"), {})
    anchor_before = start.get("qpc_before", start.get("qpc"))
    anchor, frequency = start.get("qpc_after", anchor_before), start.get("qpc_frequency")
    sent = {sequence(r): r for r in replay if r.get("success")}
    latencies, arrival, dispatch, problems, samples = [], [], [], [], []
    for row in rows:
        if row.get("event") == "hover_superseded" and row.get("reason") == "native_event_coalesced" and sequence(row) in requested & set(arrivals):
            problems.append(f"hover {sequence(row)}: native input arrived without a row target")
    for seq, target in targets.items():
        if seq in superseded or seq in unchanged:
            continue
        paint = paints.get(seq)
        if not paint:
            problems.append(f"hover {seq}: no paint or superseded witness")
            continue
        if paint.get("row_id") != target.get("row_id") or not paint.get("hovered"):
            problems.append(f"hover {seq}: painted row does not match target")
            continue
        if paint.get("previous_row_id") is not None and not paint.get("previous_cleared"):
            problems.append(f"hover {seq}: previous hover not cleared")
            continue
        submit = next((r for r in submissions if r.get("window") == paint.get("window")
                       and r.get("scene_revision") == paint.get("scene_revision")
                       and r["at_ms"] >= paint["at_ms"]), None)
        if not submit:
            problems.append(f"hover {seq}: painted scene has no submission")
            continue
        native = sent.get(seq)
        input_at = arrivals.get(seq, target)["at_ms"]
        latency = submit["at_ms"] - input_at
        arrival.append(latency)
        # Use the later probe anchor and earlier SendInput bound: latency is
        # conservatively high within the retained anchor uncertainty.
        if native and anchor and frequency == native.get("qpc_frequency"):
            injected_at = (native["qpc_before"] - anchor) * 1000 / frequency
            latency = submit["at_ms"] - injected_at
            if latency < 0:
                problems.append(f"hover {seq}: negative native dispatch latency")
                continue
            dispatch.append(latency)
        latencies.append(latency)
        samples.append({"sequence": seq, "row_id": paint.get("row_id"),
                        "scene_revision": paint.get("scene_revision"),
                        "submission_id": submit.get("submission_id"), "latency_ms": latency})
    accounted = {r["sequence"] for r in samples} | superseded | unchanged
    acknowledged = {sequence(r) for r in replay if r.get("success")}
    return {"requested": len(requested), "targets": len(targets), "painted_and_submitted": len(samples),
            "superseded": len(requested & superseded), "unchanged": len(requested & (unchanged - superseded)),
            "native_acknowledged": len(requested & acknowledged),
            "unaccounted": len(requested - accounted),
            "final_witness": any(r.get("event") == "hover_final_witness" and begin_ms <= r.get("at_ms", -1) < end_ms for r in rows),
            "handler_arrival_to_submit_ms": distribution(arrival),
            "native_dispatch_to_submit_ms": distribution(dispatch),
            "immediate_feedback_ms": distribution(latencies),
            "latency_origin": "SendInput QPC" if len(dispatch) == len(samples) and samples else "app native handler arrival",
            "qpc_anchor_uncertainty_ms": (anchor - anchor_before) * 1000 / frequency if anchor and anchor_before and frequency else None,
            "display_latency_ms": None, "display_validation": "pending",
            "problems": problems, "samples": samples}


def load_replay(directory):
    path = Path(directory) / "native-input.jsonl"
    return [json.loads(line)["ack"] for line in path.read_text(encoding="utf-8").splitlines()] if path.exists() else []


def submission_summary(records, begin_ms, end_ms, distribution):
    rows = [r for r in records if r.get("event") == "submission"
            and begin_ms <= r.get("start_ms", r.get("at_ms", -1)) < end_ms]
    seconds = (end_ms - begin_ms) / 1000
    return {"count": len(rows), "per_second": len(rows) / seconds if seconds > 0 else None,
            "reused_scene": sum(bool(r.get("reused_scene")) for r in rows),
            "reasons": dict(Counter(r.get("reason", "unknown") for r in rows)),
            "cpu_ms": distribution(r.get("duration_ms") for r in rows)}
