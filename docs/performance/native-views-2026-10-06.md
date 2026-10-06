# Repository view performance survey, 6 October 2026

Read-only native Linux runs on the GitComet and GitComet Pro working repositories,
using the isolated Mutter/Wayland profiling harness. Baseline is b493481d;
candidate adds the terminal and straight history connector fixes. Both binaries
use the same development profile and GPUI revision. Hardware is Ryzen 9 5950X,
NVIDIA GTX 1080, Vulkan, 60 Hz. Full summaries, binary hashes, repository heads,
phase durations, CPU and memory samples are in [the JSON](native-views-2026-10-06.json).
These are working repositories, not a frozen corpus. Background edits, system
load and driver caches differed; this exploratory survey is not a release
promotion gate or a controlled estimate of each change's effect.

| View / phase | Repository | Baseline draw p95 | Candidate draw p95 |
| --- | --- | ---: | ---: |
| History scrolling | Pro | 6.87 ms | 5.42 ms |
| History selection | Pro | 20.96 ms | 19.97 ms |
| Terminal scrolling with output | Pro | 99.80 ms | 6.10 ms |
| Terminal after output | Pro | 86.05 ms | 3.02 ms |
| Diff repeated search | Pro | 62.82 ms | 10.77 ms |
| Settings opening | Pro | unmeasured | 29.27 ms |
| History selection | GitComet | 21.26 ms | 18.52 ms |

Terminal scrolling returned to approximately 60 frames/s from 13. Painting and
scrollbar measurement now try the parser lock without waiting, retain a coherent
last snapshot when the PTY is busy, and schedule at most one retry. Row processing
uses Alacritty's ordered display cells to avoid scanning the full viewport once
per row. A regression test paints and reads the scrollbar while deliberately
holding the parser lock, then verifies recovery.

Straight history connectors use existing quad primitives; curved connectors
keep paths. This reduces unnecessary path tessellation without changing layout.
The diff results are recorded observations, not attributed to these changes;
none of this patch modifies diff search.

## Remaining measurements

GitComet History and terminal scenarios each hit a first-use 1.2–1.4 s draw stall
and fail the harness's one-second validity guard. Their ordinary p95 values are
not evidence that the stall is resolved. A sampled native draw stack contains
NVIDIA shader compilation (`_nv002nvvm`); cached driver state and current GPU
experiments require separate validation. No experiment is enabled by this patch.

The baseline GitComet idle run consumed 1.70 process CPU cores and drew 20 frames/s.
That repository has many linked worktrees and concurrent filesystem activity.
The existing `perf/linked_worktrees` PR addresses repeated status work and stale
commit requests; its integration and performance must be checked separately.

Startup, idle, history selection/scroll, diff search, heavy terminal output and
settings are covered here. Pro-only Explore, Coverage, Agents and Forge layouts
are measured in the downstream repository with its native render harness.
Provider network latency and native macOS/Windows behavior are not measured.

Reproduce without changing the real application profile:

```sh
python3 scripts/profiling/live-ui.py run --binary /path/to/gitcomet \
  --repository /path/to/repository --scenario terminal-output --output /tmp/terminal-survey
python3 scripts/profiling/live-ui.py run --binary /path/to/gitcomet \
  --repository /path/to/repository --scenario settings --output /tmp/settings-survey
```
