#!/usr/bin/env python3
"""Record five accepted release Criterion runs for the indexed-history budget report."""
import argparse
import json
from pathlib import Path
import platform

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runs', type=Path, nargs=5, required=True, help='Five archived release Criterion roots')
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--runner', required=True, help='Dedicated runner identity')
args = parser.parse_args()
runs = []
for root in args.runs:
    cases = {}
    for path in root.glob('indexed_history/**/new/estimates.json'):
        label = path.parent.parent.relative_to(root).as_posix()
        cases[label] = json.loads(path.read_text())['mean']['point_estimate']
    if not cases:
        parser.error(f'No indexed-history Criterion results in {root}')
    runs.append(cases)
if any(set(run) != set(runs[0]) for run in runs):
    parser.error('All five runs must contain the same cases')
args.output.write_text(json.dumps({'profile': 'release', 'runner': args.runner,
    'hardware': platform.uname()._asdict(), 'sources': [str(root.resolve()) for root in args.runs],
    'samples_ns': {label: [run[label] for run in runs] for label in runs[0]}}, indent=2))
