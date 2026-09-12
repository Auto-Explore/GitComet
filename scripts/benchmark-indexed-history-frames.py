#!/usr/bin/env python3
"""Pair production GPUI wheel/publication/draw probes; exclude compilation from samples."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--before', type=Path, required=True)
parser.add_argument('--after', type=Path, required=True)
parser.add_argument('--profile', choices=['test', 'release'], required=True)
parser.add_argument('--case', action='append', help='Frontier columns:graph design pixels:UI scale percent')
parser.add_argument('--pairs', type=int, default=5)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
if args.pairs < 1:
    parser.error('--pairs must be positive')
cases = [tuple(map(int, case.split(':'))) for case in args.case or ['5261:80:100']]
if any(len(case) != 3 or min(case) <= 0 for case in cases):
    parser.error('--case requires three positive integers')
args.output.mkdir(parents=True, exist_ok=True)
metadata = {'profile': args.profile, 'machine': platform.uname()._asdict(),
            'fixture': {'commits': 20000, 'displayed_rows': 38, 'frames_per_sample': 100},
            'binaries': {phase: {'path': str(getattr(args, phase).resolve()),
                                'sha256': hashlib.sha256(getattr(args, phase).read_bytes()).hexdigest()}
                         for phase in ['before', 'after']}}
if metadata['binaries']['before']['sha256'] == metadata['binaries']['after']['sha256']:
    parser.error('Before and after binaries are identical; use separate build target directories')
(args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2))
reports = []
for width, pixels, scale in cases:
    samples = {'before': [], 'after': []}
    label = f'{width}_columns_{pixels}_pixels_{scale}_percent'
    for pair in range(args.pairs):
        for phase in (['before', 'after'] if pair % 2 == 0 else ['after', 'before']):
            env = {**os.environ, 'GITCOMET_BENCH_GRAPH_WIDTH': str(width),
                   'GITCOMET_BENCH_GRAPH_PIXELS': str(pixels), 'GITCOMET_BENCH_UI_SCALE': str(scale),
                   'GITCOMET_PERF_CRITERION_ROOT': str((args.output / phase).resolve())}
            result = subprocess.run([str(getattr(args, phase).resolve()), 'indexed_history_real_frame_benchmark', '--ignored', '--nocapture'],
                                    env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=True)
            (args.output / f'{label}-{phase}-{pair}.log').write_text(result.stdout)
            draw = re.search(r'draw_p50_ms=([\d.]+) draw_p95_ms=([\d.]+) draw_p99_ms=([\d.]+) allocations_per_frame=([\d.]+) bytes_per_frame=([\d.]+)', result.stdout)
            total = re.search(r'publication \+ draw .* p50_ms=([\d.]+) p95_ms=([\d.]+) p99_ms=([\d.]+) paths_max=(\d+)', result.stdout)
            if not draw or not total:
                raise RuntimeError('Missing production frame benchmark output')
            sample = dict(zip(['draw_p50_ms', 'draw_p95_ms', 'draw_p99_ms', 'allocations_per_frame', 'bytes_per_frame'], map(float, draw.groups())))
            sample.update(dict(zip(['total_p50_ms', 'total_p95_ms', 'total_p99_ms'], map(float, total.groups()[:3]))))
            # Original production code has no path instrumentation.
            sample['emitted_paths_max'] = int(total[4]) if phase == 'after' else None
            samples[phase].append(sample)
            print(f'{label} pair={pair + 1} {phase}: {sample}', flush=True)
    reports.append({'case': label, 'samples': samples,
                    'medians': {phase: {metric: statistics.median(sample[metric] for sample in values)
                                        for metric in values[0] if values[0][metric] is not None}
                                for phase, values in samples.items()}})
    (args.output / 'results.json').write_text(json.dumps(reports, indent=2))
