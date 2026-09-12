#!/usr/bin/env python3
"""Run five paired production backend probes against unchanged repository snapshots."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess


def repository_metadata(path):
    def git(*args):
        return subprocess.check_output(['git', '-C', str(path), *args], text=True).strip()
    graph = Path(git('rev-parse', '--path-format=absolute', '--git-path', 'objects/info/commit-graph'))
    graph_files = [graph] if graph.is_file() else []
    chain = graph.parent / 'commit-graphs'
    if chain.is_dir():
        graph_files.extend(sorted(chain.iterdir()))
    return {'path': str(path), 'head': git('rev-parse', 'HEAD'), 'refs': git('show-ref'),
            'commit_graphs': {str(file): hashlib.sha256(file.read_bytes()).hexdigest()
                              for file in graph_files if file.is_file()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--repository', type=Path, action='append', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--profile', choices=['test', 'release'], required=True)
    args = parser.parse_args()
    binaries = {phase: {'path': str(getattr(args, phase).resolve()),
                        'sha256': hashlib.sha256(getattr(args, phase).read_bytes()).hexdigest()}
                for phase in ['before', 'after']}
    if binaries['before']['sha256'] == binaries['after']['sha256']:
        parser.error('Before and after binaries are identical; use separate build target directories')
    args.output.mkdir(parents=True, exist_ok=True)
    metadata = {'profile': args.profile, 'binaries': binaries, 'machine': platform.uname()._asdict(),
                'cpu': Path('/proc/cpuinfo').read_text() if Path('/proc/cpuinfo').exists() else platform.processor(),
                'repositories': [repository_metadata(path.resolve()) for path in args.repository]}
    (args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2))
    reports = []
    for repo_number, repo in enumerate(metadata['repositories']):
        samples = {'before': [], 'after': []}
        for pair in range(5):
            # Alternate execution order to avoid always assigning a warmer cache to after.
            for phase in (['before', 'after'] if pair % 2 == 0 else ['after', 'before']):
                if repository_metadata(Path(repo['path'])) != repo:
                    raise RuntimeError('Repository refs or commit-graph changed during paired run')
                executable = getattr(args, phase).resolve()
                result = subprocess.run([str(executable), 'indexed_history_large_repository_benchmark', '--ignored', '--nocapture'],
                                        env={**os.environ, 'GITCOMET_HISTORY_BENCH_REPO': repo['path']},
                                        text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=True)
                (args.output / f'repo-{repo_number}-{phase}-{pair}.log').write_text(result.stdout)
                match = re.search(r'index commits=(\d+) seconds=([\d.]+) estimated_mib=([\d.]+)', result.stdout)
                ranges = re.search(r'256-commit range reads p50_ms=([\d.]+) p95_ms=([\d.]+)', result.stdout)
                if not match or not ranges:
                    raise RuntimeError('Missing production benchmark output')
                if repository_metadata(Path(repo['path'])) != repo:
                    raise RuntimeError('Repository refs or commit-graph changed during paired run')
                samples[phase].append({'commits': int(match[1]), 'index_seconds': float(match[2]), 'retained_mib': float(match[3]),
                                       'range_p50_ms': float(ranges[1]), 'range_p95_ms': float(ranges[2])})
                for temperature in ['first_touch', 'warm']:
                    timed = re.search(rf'{temperature} range p50_ms=([\d.]+) p95_ms=([\d.]+) p99_ms=([\d.]+)', result.stdout)
                    if timed:
                        samples[phase][-1].update({f'{temperature}_{percentile}_ms': float(value)
                                                  for percentile, value in zip(['p50', 'p95', 'p99'], timed.groups())})
                print(f"{repo['path']} pair={pair + 1} {phase}: {samples[phase][-1]}", flush=True)
        reports.append({'path': repo['path'], 'samples': samples,
                        'medians': {phase: {metric: statistics.median(sample[metric] for sample in values)
                                            for metric in values[0]} for phase, values in samples.items()}})
    (args.output / 'results.json').write_text(json.dumps(reports, indent=2))


if __name__ == '__main__':
    main()
