#!/usr/bin/env python3
"""Alternating native comparisons for repository sharing and Linux THP.

Uses the production multi-window driver and isolated profiles/compositors. No
allocation instrumentation or GPU experiments are enabled in these latency runs.
"""
import argparse
import json
from pathlib import Path
import shutil
import statistics

import perf_metadata
import perf_workloads

multi = perf_workloads.module("shared_reads_multi", "multi-window.py")
live = multi.live


def comparison(records, pairs):
    rows = []
    cases = dict.fromkeys(record["case"] for record in records)
    for case in cases:
        selected = [record for record in records if record["case"] == case]
        grouped = {(record["pair"], record["variant"]): record for record in selected}
        complete = (len(selected) == 2 * pairs
                    and set(grouped) == {(pair, variant) for pair in range(pairs) for variant in ("baseline", "candidate")}
                    and all(record["result"]["valid"]
                            and not record["result"].get("allocation_tracking", False)
                            and not record["result"].get("debug_assertions", False)
                            and not record["result"].get("gpu_timings_requested", False)
                            for record in selected))
        phases = set.union(*(set(record["result"]["phases"]) for record in selected))
        metrics = []
        for phase in sorted(phases):
            if phase not in ("select", "hover", "scroll", "transfer", "baseline_select", "select_while_loading", "after_load_select") and not phase.startswith("focus_"):
                continue
            # Pointer motion has no state-publication witness in the native
            # driver. Gate its draw time; keyboard/scroll phases also have
            # witnessed input-to-draw latency and must supply both metrics.
            paths = ("draw_ms.p95",) if phase == "hover" else ("draw_ms.p95", "inputs.input_to_draw_ms.p95")
            for path in paths:
                values = {variant: [] for variant in ("baseline", "candidate")}
                for record in selected:
                    value = record["result"]["phases"].get(phase, {})
                    for part in path.split("."):
                        value = value.get(part) if isinstance(value, dict) else None
                    if value is not None:
                        values[record["variant"]].append(value)
                base, candidate = (statistics.median(values[v]) if values[v] else None for v in ("baseline", "candidate"))
                enough = all(len(value) == pairs for value in values.values())
                limit = max(base * .05, 1.) if base is not None else None
                metrics.append(dict(phase=phase, metric=path, baseline_ms=base,
                                    candidate_ms=candidate, allowed_increase_ms=limit,
                                    passes=enough and candidate - base <= limit, raw=values))
        memory = {}
        work = {}
        for variant in ("baseline", "candidate"):
            samples = [(record["result"]["phases"].get("settled") or record["result"]["phases"].get("refresh", {})).get("end_sample") or {}
                       for record in selected if record["variant"] == variant]
            memory[variant] = {key: statistics.median(sample[key] for sample in samples if sample.get(key) is not None)
                               for key in ("pss_kib", "rss_kib", "fds")
                               if any(sample.get(key) is not None for sample in samples)}
            counts = [record["result"].get("work_units", {}) for record in selected if record["variant"] == variant]
            work[variant] = {key: statistics.median(count[key] for count in counts) if counts and all(key in count for count in counts) else None
                             for key in ("log_topology_build", "history_index_build", "range_object_read")}
        rows.append(dict(case=case, complete=complete, metrics=metrics, memory=memory, work=work,
                         passes_latency=complete and bool(metrics) and all(row["passes"] for row in metrics)))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--linked", type=Path, nargs="+", required=True)
    parser.add_argument("--unrelated", type=Path, nargs="+", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pairs", type=int, default=5)
    parser.add_argument("--inputs", type=int, default=120)
    parser.add_argument("--windows", type=int, nargs="+", default=[1, 2, 4])
    parser.add_argument("--idle-seconds", type=int, default=3)
    parser.add_argument("--display", choices=["headless", "desktop"], default="headless")
    parser.add_argument("--thp", action="store_true", help="same candidate binary, default allocator versus MIMALLOC_ALLOW_THP=0")
    parser.add_argument("--transfers", nargs="+", choices=["fetch", "lfs-fetch", "annex-get"],
                        help="measure verified transfers with concurrent history inputs using the largest requested window count")
    parser.add_argument("--background-repository", type=Path,
                        help="load a large second repository while interacting with the foreground window")
    args = parser.parse_args()
    if min(args.pairs, args.inputs, args.idle_seconds, *args.windows) < 1:
        parser.error("counts must be positive")
    if args.transfers and args.background_repository:
        parser.error("transfer and background loading scenarios are separate")
    if not args.transfers and not args.background_repository and min(len(args.linked), len(args.unrelated)) < max(args.windows) - 1:
        parser.error("provide a distinct linked and unrelated repository for each additional window")
    args.output.mkdir(parents=True, exist_ok=False)
    binaries = {}
    for variant, source in (("baseline", args.candidate if args.thp else args.baseline), ("candidate", args.candidate)):
        target = args.output / variant / source.name
        target.parent.mkdir()
        shutil.copy2(source, target)
        binaries[variant] = target.resolve()
    repositories = dict(same=[args.repository], linked=[args.repository, *args.linked],
                        unrelated=[args.repository, *args.unrelated])
    repositories = {key: [path.resolve(strict=True) for path in paths] for key, paths in repositories.items()}
    manifest = dict(source=perf_metadata.source_info(), pairs=args.pairs, inputs=args.inputs,
                    comparison="default-versus-thp-off" if args.thp else "baseline-versus-candidate",
                    binaries={key: dict(path=str(path), sha256=perf_metadata.sha256_file(path)) for key, path in binaries.items()},
                    records=[])
    if not args.thp and manifest["binaries"]["baseline"]["sha256"] == manifest["binaries"]["candidate"]["sha256"]:
        parser.error("baseline and candidate must be different binaries")
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    for pair in range(args.pairs):
        cases = [(kind, count) for kind in repositories for count in args.windows] if not args.transfers else [(operation, max(args.windows)) for operation in args.transfers]
        if args.background_repository:
            cases = [("background", 2)]
        if pair % 2:
            cases.reverse()
        for kind, count in cases:
            case = f"{kind}-{count}"
            for variant in (("baseline", "candidate") if pair % 2 == 0 else ("candidate", "baseline")):
                directory = args.output / f"pair-{pair}" / case / variant
                print(f"pair={pair + 1}/{args.pairs} case={case} variant={variant}", flush=True)
                env = {"MIMALLOC_ALLOW_THP": "0"} if args.thp and variant == "candidate" else {}
                if args.background_repository:
                    result = live.run_once(binaries[variant], args.repository, case, directory, 300,
                        display=args.display, ping_ms=1000, extra_env=env,
                        scenario_steps=multi.background(args.background_repository.resolve(strict=True), args.inputs))
                    result["background_load_overlap"] = multi.background_overlap(directory)
                    if not result["background_load_overlap"]["overlapping_inputs"]:
                        result["valid"] = False
                        result["problems"].append("large history loading did not overlap foreground input")
                elif args.transfers:
                    with perf_workloads.transfer(directory.parent / f"{variant}-fixture", args.repository,
                            kind, live, shape="small", bandwidth_mib=8) as fixture:
                        scenario = perf_workloads.transfer_scenario(fixture)
                        opening = [multi.ready()]
                        for index in range(1, count):
                            opening += multi.new_repo(f"window-{index}", fixture["repo"])
                        scenario["steps"][:0] = [*opening, {"do": "switch_window", "name": "primary"},
                                                {"do": "expect_windows", "count": count}]
                        result = live.run_once(binaries[variant], fixture["repo"], case, directory, 300,
                            display=args.display, ping_ms=1000, extra_env={**fixture["env"], **env},
                            scenario_steps=scenario)
                        if result["valid"]:
                            result["content_verification"] = fixture["verify"]()
                            if not any(operation["overlapping_inputs"] for operation in result["operation_results"]):
                                result["valid"] = False
                                result["problems"].append("transfer completed without concurrent inputs")
                else:
                    result = live.run_once(binaries[variant], repositories[kind][0], case, directory, 300,
                        display=args.display, ping_ms=1000, extra_env=env,
                        scenario_steps=multi.matrix(repositories[kind], count, args.idle_seconds * 1000, args.inputs))
                try:
                    multi.verify_isolation(directory)
                except ValueError as error:
                    result["valid"] = False
                    result["problems"].append(str(error))
                result["shared_history_memory"] = [record["detail"] for record in live.load_records(directory)
                                                   if record["event"] == "scenario_history_memory"]
                manifest["records"].append(dict(pair=pair, case=case, variant=variant, result=result,
                                                directory=str(directory.resolve())))
                (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
                (args.output / "comparison.json").write_text(json.dumps(comparison(manifest["records"], args.pairs), indent=2) + "\n")
                if not result["valid"]:
                    print(json.dumps(result["problems"]), flush=True)
                    return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
