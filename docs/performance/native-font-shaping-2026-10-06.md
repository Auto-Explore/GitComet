# Native font shaping in development builds

The repository-picker survey on the real GitComet and Pro checkouts exposed
158–204 ms first-use frames, even with cached picker rows and bounded path
measurements. A diagnostic main-thread profile found 206 of 391 leaf samples
in font-table reads or glyph substitution, and one in the NVIDIA driver, during
the first path-filter interval. The capture lost ten samples overall; it
identifies a dependency family rather than precise inclusive call costs.

Like the existing GPUI/layout package overrides, the development profile now
optimises `harfrust`, `read-fonts` and `font-types` at level 2. Application code
remains at level 0, and release settings are unchanged. Pro repeats these
settings because Cargo does not inherit profiles from its upstream dependency.

Sequential native Vulkan runs with private Mutter sessions and profiles,
40 synthetic recent repositories with long hidden paths, and read-only working
checkouts observed:

| Repository | First picker open before/after | First path filter before/after | Repeated path filter after |
| --- | ---: | ---: | ---: |
| GitComet | 158.27 / 60.86 ms | 204.16 / 68.17 ms | 5.75 ms |
| Pro | 163.68 / 63.34 ms | 202.34 / 63.00 ms | 5.32 ms |

Every picker query/paste/backspace witness passes. Native terminal output now
measures draw p95 11.99/11.40 ms and 59.99/59.83 frames/s on GitComet/Pro.
History selection still has development frames above 16 ms (p95 20.45/21.32 ms),
so this is not a claim that every input meets a 60 Hz frame budget. First font
use still costs tens of milliseconds. OS caches were not flushed, GPU caches
were warm, and the repositories were being edited during the wider survey.

[Raw picker observations and profile evidence](native-font-shaping-2026-10-06.json)
retain the input distributions, repository heads and binary hashes. Frame times
measure CPU draw/submission, not display completion. These are development
build measurements on one Linux GPU; they establish no release or other-platform
latency guarantee. No experimental renderer is enabled.
