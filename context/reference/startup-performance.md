# Startup, quit and idle performance

Recorded measurements of demonstrated editing readiness, complete file
loading, and initial syntax parsing, alongside Neovim and Helix. Earlier
result sets also record first document output, quitting, and idle cost. This
is a register of measurements, not of budgets: `tests/performance.rs` holds
the assertions that fail in CI, and nothing here is enforced automatically.

The harness is `benchmarks/`, and `benchmarks/README.md` documents what each
fixture isolates and how the measurement is taken. Runyte's modes and editions
are compared with `benchmarks/editions.py`. The readiness comparison with other
editors uses `benchmarks/startup.py`; its file-loaded and syntax-ready
columns require the probes documented there. The older first-output, quit,
and idle harness remains available:

```sh
cargo build --release
benchmarks/run.py
```

## 2026-11-28 — Full key-program condition set (stage 5e)

[PR #164](https://github.com/runyte/runyte/pull/164) adds selection, register,
protection and buffer-type conditions with buffer-type-aware mode-flow validation. Ten alternating fresh-process samples after
warm-up compare the stage 5d probes on x86-64 and stage 5d head `2dd889e0` on ARM
with implementation `d86efad0` (rebased as `fdb9de03`). Later commits update
documentation, display/diagnostic strings and tests only. The x86 current probes
include those wording changes at `7ad0066b`.
[Raw samples](../../benchmarks/results/keymap-conditions-2026-11-28.json) include
local binary hashes, all reload phases and ARM preset/readiness samples.
The [ARM run](https://github.com/runyte/runyte/actions/runs/46125980240) checks out
the revisions above.

| Platform/profile | Baseline changed ms | 5e changed ms | Baseline unchanged ms | 5e unchanged ms |
| --- | ---: | ---: | ---: | ---: |
| x86_64 debug | 1020.383 | 1017.711 | 270.678 | 271.216 |
| x86_64 release | 94.390 | 97.030 | 25.826 | 26.453 |
| aarch64 debug | 2923.895 | 2925.602 | 777.872 | 776.406 |
| aarch64 release | 154.751 | 153.506 | 41.220 | 40.765 |

Both release reload comparisons meet the 25%/20 ms gate. The local idle release
guard measured 97.411 ms changed and 26.337 ms unchanged, below its 350/100 ms
ceilings. Installed-binding and zero repeated flow-work assertions pass.

| ARM release readiness | Dev ms | 5e ms | Change |
| --- | ---: | ---: | ---: |
| short.txt | 43.532 | 43.808 | +0.63% |
| short.lua | 44.704 | 44.896 | +0.43% |

Readiness uses ten alternating pairs per fixture; both remain within 5%.
ARM debug preset initialization: 123.301 → 123.955 ms median.
ARM release preset initialization: 8.787 → 8.767 ms median.

## 2026-11-28 — Per-step count, register and extension (stage 5d)

[PR #163](https://github.com/runyte/runyte/pull/163) adds per-step options and
count-aware mode-flow validation. Ten alternating fresh-process samples after
warm-up compare the stage 5c probes on x86-64 and merged dev `7a9d45ae` on ARM
with implementation `30243dd6`. Later commits update documentation and test
expectations only. [Raw samples](../../benchmarks/results/keymap-steps-2026-11-28.json)
include local binary hashes, all reload phases and ARM preset/readiness samples.
The [ARM run](https://github.com/runyte/runyte/actions/runs/46120176196) checks out
the revisions above; its summary text retains labels from an older diagnostic.

| Platform/profile | Baseline changed ms | 5d changed ms | Baseline unchanged ms | 5d unchanged ms |
| --- | ---: | ---: | ---: | ---: |
| x86_64 debug | 1033.355 | 1036.259 | 274.315 | 278.515 |
| x86_64 release | 95.334 | 95.356 | 26.016 | 25.952 |
| aarch64 debug | 2906.099 | 2907.969 | 771.261 | 772.369 |
| aarch64 release | 154.769 | 154.199 | 41.208 | 40.954 |

Both release reload comparisons meet the 25%/20 ms gate. The local idle release
guard measured 100.764 ms changed and 26.967 ms unchanged, below its 350/100 ms
ceilings. Installed-binding and zero repeated flow-work assertions pass.

| ARM release readiness | Dev ms | 5d ms | Change |
| --- | ---: | ---: | ---: |
| short.txt | 41.801 | 42.094 | +0.70% |
| short.lua | 42.545 | 42.480 | -0.15% |

Readiness uses ten alternating pairs per fixture; both remain within 5%.
ARM debug preset initialization: 123.739 → 123.172 ms median.
ARM release preset initialization: 8.967 → 8.869 ms median.

## 2026-11-28 — Supplied arguments and literal insertion (stage 5c)

[PR #162](https://github.com/runyte/runyte/pull/162) adds supplied argument
validation and literal insertion. The same live native-preview fixture probe
compares dev `d564c471` with the stage 5c implementation `16afc260` in ten
alternating fresh-process samples after warm-up. Later corrections affect
discovery display and test fixtures, not the measured reload or startup paths.
[Raw samples](../../benchmarks/results/keymap-arguments-2026-11-28.json) retain
all observations and local probe hashes. The
[ARM run](https://github.com/runyte/runyte/actions/runs/46116398298) also measures
cold compilation, preset initialization and stock release readiness.

| Platform/profile | Dev changed ms | 5c changed ms | Dev unchanged ms | 5c unchanged ms |
| --- | ---: | ---: | ---: | ---: |
| x86_64 debug | 1001.612 | 1024.226 | 987.940 | 270.491 |
| x86_64 release | 91.193 | 93.828 | 90.555 | 25.587 |
| aarch64 debug | 2860.936 | 2912.407 | 2858.077 | 774.359 |
| aarch64 release | 150.381 | 153.629 | 148.848 | 40.780 |

Both release reload comparisons meet the 25%/20 ms gate. The local release
guard measured 97.361 ms changed and 26.276 ms unchanged, below its 350/100 ms
ceilings; installed-binding and zero repeated flow-work assertions also pass.

| ARM release readiness | Dev ms | 5c ms | Change |
| --- | ---: | ---: | ---: |
| short.txt | 40.870 | 40.888 | +0.04% |
| short.lua | 42.135 | 42.402 | +0.63% |

Readiness uses ten alternating pairs per fixture; both remain within 5%.
ARM debug preset initialization: 123.390 → 123.532 ms median.
ARM release preset initialization: 8.789 → 8.797 ms median.

## 2026-11-28 — configured mode-flow reload regression

PR [#161](https://github.com/runyte/runyte/pull/161) initially took 6.76 s to
reload the native-preview fixture on ARM in debug, exceeding its 5 s window
acceptance deadline. Repeating full mode-flow analysis accounted for 3.88 s.
[The original comparison](https://github.com/runyte/runyte/actions/runs/46111889474)
measured the failing production revision with timing instrumentation.

The fix caches raw compiled keymaps across unchanged key/mode/buffer-type
configuration, retains diagnostics, and still checks live plugin collisions.
Changed bindings reuse immutable admission inventories and preset reachability
only while every effective preset transition survives. Removed transitions
fall back to full analysis; stable transfers are memoized within the existing
work bound. Both pane-key variants have independent preset caches.

Production revisions are dev `d564c471` and fixed `01e02205`. The
`config_reload` probe uses ten alternating fresh-process samples after a
discarded warm-up per binary/profile. It measures cold configured compilation,
then starts the editor with the fixture, adds F6, reloads, and reloads unchanged.
The empty-key case remains empty. Debug and release use the same profile on
each side. Tables show median milliseconds;
[raw samples](../../benchmarks/results/keymap-reload-2026-11-28.json) retain every
sample. [ARM run](https://github.com/runyte/runyte/actions/runs/46114048517) pairs
both revisions on one `ubuntu-24.04-arm` runner.

| Platform/profile | Case | Dev reload | Fixed reload | Dev unchanged | Fixed unchanged |
| --- | --- | ---: | ---: | ---: | ---: |
| x86_64 debug | empty | 290.974 | 290.869 | 265.525 | 265.128 |
| x86_64 debug | preview | 986.374 | 1013.589 | 989.842 | 269.106 |
| x86_64 release | empty | 26.573 | 28.182 | 24.875 | 25.942 |
| x86_64 release | preview | 91.822 | 96.060 | 91.216 | 26.247 |
| aarch64 debug | empty | 836.616 | 836.282 | 756.838 | 756.307 |
| aarch64 debug | preview | 2860.926 | 2901.426 | 2859.351 | 771.397 |
| aarch64 release | empty | 44.174 | 45.367 | 40.680 | 40.910 |
| aarch64 release | preview | 150.266 | 154.027 | 148.538 | 40.840 |

| Platform/profile | Case | Dev compile | Fixed compile | Fixed flow | Fixed other work |
| --- | --- | ---: | ---: | ---: | ---: |
| x86_64 debug | empty | 201.111 | 160.744 | 0.000 | 160.744 |
| x86_64 debug | preview | 358.076 | 722.070 | 404.117 | 317.145 |
| x86_64 release | empty | 27.639 | 17.062 | 0.000 | 17.062 |
| x86_64 release | preview | 32.010 | 65.338 | 35.959 | 29.333 |
| aarch64 debug | empty | 575.660 | 455.422 | 0.000 | 455.422 |
| aarch64 debug | preview | 1037.717 | 2054.777 | 1138.334 | 916.891 |
| aarch64 release | empty | 32.738 | 25.336 | 0.000 | 25.336 |
| aarch64 release | preview | 53.141 | 112.722 | 65.832 | 46.890 |

Cold configured compilation remains more expensive than dev because it
initializes the new flow analysis. The release reload gate is separate:
changed and unchanged preview reloads must remain within 25% or 20 ms of dev,
whichever allowance is larger. Both pass. Every fixed unchanged-reload sample
performs zero mode-flow analysis. The release regression guard also verifies
installed binding changes and holds changed/unchanged reloads to 350/100 ms.

Stock release editing readiness uses the existing 120×40 PTY harness,
isolated home/XDG paths, and plain-text/Lua fixtures. Both revisions alternate.
An initial ten-pair x86 Lua result was +5.94% amid broad variation, so both
x86 fixtures received forty additional pairs; all fifty pairs are retained.
ARM uses ten pairs per fixture. Both platforms remain within the 5% readiness
gate. These are terminal-edition readiness measurements; native repaint is
covered separately by the passing
[ARM window acceptance](https://github.com/runyte/runyte/actions/runs/46112216358).

| Platform | Fixture | Dev readiness | Fixed readiness | Change |
| --- | --- | ---: | ---: | ---: |
| x86_64 | short.txt | 32.548 | 33.043 | +1.52% |
| x86_64 | short.lua | 32.486 | 32.311 | -0.54% |
| aarch64 | short.txt | 41.974 | 41.718 | -0.61% |
| aarch64 | short.lua | 43.181 | 42.608 | -1.33% |

## 2026-11-28 — Configured reload guard calibration

The release reload guard in [#161](https://github.com/runyte/runyte/pull/161)
uses 350 ms for changed keys and 100 ms for unchanged source, through the
existing `budget()` profile scaling. It catches multi-second regressions;
the separate dev-relative acceptance gate measures smaller performance changes.
The original 100 ms changed-key ceiling was below ordinary runner cost:
[CI run 46112216275](https://github.com/runyte/runyte/actions/runs/46112216275)
measured five-sample medians of 160.315 ms on Linux and 161.287 ms on macOS.

Paired release measurements against dev `d564c471` give a changed-key median
of 91.822 ms locally on x86 and 149.941 ms on ARM CI. The fixed implementation
(`01e02205`) measured 96.060 ms and 153.305 ms respectively. The 350 ms ceiling
provides roughly twice the observed runner cost in headroom. Unchanged reloads
measured 26.247 ms locally and 40.807 ms on ARM, so their 100 ms ceiling retains
similar headroom. The test also checks that changed bindings are installed and
that unchanged source performs no mode-flow validation.
ARM reload measurements came from
[run 46112471194](https://github.com/runyte/runyte/actions/runs/46112471194);
its later readiness phase failed because the Python `pyte` dependency was
missing, so that run supplies no readiness result.

The local release readiness comparison retained all 50 alternating pairs for
each fixture: plain text changed from 32.548 to 33.043 ms (+1.52%), and Lua
from 32.486 to 32.311 ms (−0.54%). Both meet the 5% startup acceptance gate.
The completed ARM readiness results are recorded in the preceding comparison.

## 2026-11-28 — ARM debug public mode-flow validation

Stage 5b of [#132](https://github.com/runyte/runyte/issues/132) was measured
against stage 5a with one discarded warm-up per binary and ten alternating
fresh-process debug-profile `preset_startup` samples on one
`ubuntu-24.04-arm` runner.
[Run 46107774562](https://github.com/runyte/runyte/actions/runs/46107774562)
compares `d564c471` with `4815f4c2`. Later test-only fixture corrections do not
change the measured production paths.

| Phase | Stage 5a ms | Stage 5b ms |
| --- | ---: | ---: |
| Typed YAML parsing | 18.550 (18.338–18.674) | 18.689 (18.401–18.808) |
| Programs and metadata | 7.346 (7.305–7.390) | 7.444 (7.407–7.500) |
| Construction, lookup index and role validation | 97.519 (97.203–97.816) | 97.340 (97.164–98.047) |
| Total keymap initialization | 123.337 (123.202–123.583) | 123.503 (123.362–123.981) |

Cells are median (min–max). Total median increases by 0.166 ms (0.13%).
The shipped preset retains its role-validation fast path; configured mode-flow
validation is not exercised by this preset-only probe. These component
measurements do not establish end-to-end desktop readiness, configured-keymap
loading costs, or release-profile costs. Raw samples are attached to the linked run.

## 2026-11-28 — ARM debug shared typed action syntax

Stage 5a of [#132](https://github.com/runyte/runyte/issues/132) was measured
against stage 4e with one discarded warm-up per binary and ten alternating
fresh-process debug-profile `preset_startup` samples on one
`ubuntu-24.04-arm` runner.
[Run 46100371965](https://github.com/runyte/runyte/actions/runs/46100371965)
compares `6a16a798` with `3dbc2185`. Later test-only scope corrections do not
change the measured production paths.

| Phase | Stage 4e ms | Stage 5a ms |
| --- | ---: | ---: |
| Typed YAML parsing | 17.787 (17.663–17.938) | 18.413 (18.204–18.581) |
| Programs and metadata | 7.347 (7.319–7.382) | 7.285 (7.244–7.324) |
| Construction, lookup index and role validation | 97.545 (97.200–98.025) | 97.310 (96.937–98.126) |
| Total keymap initialization | 122.659 (122.457–123.161) | 122.972 (122.728–123.757) |

Cells are median (min–max). Typed parsing increases by 0.626 ms (3.52%);
total median increases by 0.313 ms (0.26%). The preset remains directly typed,
without a generic YAML value conversion. These component measurements do not
establish end-to-end desktop readiness or release-profile costs. Raw samples
are attached to the linked run.

## 2026-11-28 — ARM debug subject-first action names

Stage 4e of [#132](https://github.com/runyte/runyte/issues/132) was measured
against stage 4d with one discarded warm-up per binary and ten alternating
fresh-process debug-profile `preset_startup` samples on one
`ubuntu-24.04-arm` runner.
[Run 46097657028](https://github.com/runyte/runyte/actions/runs/46097657028)
compares `afb5ad1a` with `22792eba`. Later fixture and rendered-hint assertion
updates do not change the measured production paths.

| Phase | Stage 4d ms | Stage 4e ms |
| --- | ---: | ---: |
| Typed YAML parsing | 17.917 (17.788–18.049) | 17.961 (17.745–18.280) |
| Programs and metadata | 7.379 (7.346–7.433) | 7.369 (7.295–7.657) |
| Construction, lookup index and role validation | 98.101 (97.560–98.689) | 97.281 (97.100–97.697) |
| Total keymap initialization | 123.349 (122.900–123.946) | 122.692 (122.387–122.998) |

Cells are median (min–max). Total median decreases by 0.657 ms (0.53%);
this small difference does not establish a performance improvement.
These component measurements do not establish end-to-end desktop readiness
or release-profile costs. Raw samples are attached to the linked run.

## 2026-11-28 — ARM debug directory/tree and clipboard actions

Stage 4d of [#132](https://github.com/runyte/runyte/issues/132) was measured
against stage 4c2 with one discarded warm-up per binary and ten alternating
fresh-process debug-profile `preset_startup` samples on one
`ubuntu-24.04-arm` runner.
[Run 46094450429](https://github.com/runyte/runyte/actions/runs/46094450429)
compares `76127923` with `ca751f5b`. Subsequent parent CRLF fixes affect test
source parsing only and do not change either measured production path.

| Phase | Stage 4c2 ms | Stage 4d ms |
| --- | ---: | ---: |
| Typed YAML parsing | 17.813 (17.612–18.084) | 17.926 (17.666–18.088) |
| Programs and metadata | 7.379 (7.328–7.421) | 7.420 (7.381–7.498) |
| Construction, lookup index and role validation | 97.729 (97.525–97.899) | 98.152 (97.636–98.493) |
| Total keymap initialization | 122.905 (122.801–123.025) | 123.387 (123.172–124.022) |

Cells are median (min–max). Total median increases by 0.482 ms (0.39%).
These component measurements do not establish end-to-end desktop readiness
or release-profile costs. Raw samples are attached to the linked run.

## 2026-11-28 — ARM debug remaining terminal meanings

Stage 4c2 of [#132](https://github.com/runyte/runyte/issues/132) was measured
against stage 4c with the same debug-profile `preset_startup` protocol: one
discarded warm-up per binary and ten alternating fresh-process samples on one
`ubuntu-24.04-arm` runner.
[Run 46091408418](https://github.com/runyte/runyte/actions/runs/46091408418)
compares `6a5b66b7` with `d47e0d1f`. The latter has the same source tree as
rebased `4c24cd63`; the blank-row yank fix moved to the parent before rebasing.

| Phase | Stage 4c ms | Stage 4c2 ms |
| --- | ---: | ---: |
| Typed YAML parsing | 17.735 (17.536–17.818) | 17.833 (17.635–18.057) |
| Programs and metadata | 7.334 (7.308–7.364) | 7.340 (7.331–7.359) |
| Construction, lookup index and role validation | 97.223 (96.776–97.547) | 97.543 (97.070–97.807) |
| Total keymap initialization | 122.279 (121.934–122.657) | 122.701 (122.375–122.969) |

Cells are median (min–max). Total median increases by 0.422 ms (0.34%).
These component measurements do not establish end-to-end desktop readiness
or release-profile costs. Raw samples are attached to the linked run.

## 2026-11-28 — ARM debug terminal programs and role validation

Stage 4c of [#132](https://github.com/runyte/runyte/issues/132) was measured
against its approved stage 4b parent with the same debug-profile
`preset_startup` protocol: one discarded warm-up per binary and ten alternating
fresh-process samples on one `ubuntu-24.04-arm` runner.
[Run 46088600328](https://github.com/runyte/runyte/actions/runs/46088600328)
compares `a0ad296e` with stage 4c `b197b36d`.

| Phase | Stage 4b ms | Stage 4c ms |
| --- | ---: | ---: |
| Typed YAML parsing | 17.514 (17.232–17.591) | 17.631 (17.510–17.801) |
| Programs and metadata | 7.148 (7.139–7.191) | 7.331 (7.268–7.371) |
| Construction, lookup index and role validation | 89.532 (89.453–90.499) | 97.421 (96.438–97.680) |
| Total keymap initialization | 114.214 (114.136–115.045) | 122.365 (121.449–122.634) |

Cells are median (min–max). Total median increases by 8.151 ms (7.14%).
This includes new load-time role-target validation and the changed default
programs. These component measurements do not establish end-to-end desktop
readiness or release-profile costs. Raw samples are attached to the linked run.

The first implementation repeatedly resolved effective tables during validation.
[Run 46087335770](https://github.com/runyte/runyte/actions/runs/46087335770)
measured 114.286 ms baseline versus 205.818 ms at `c31dff94` (+80.09%).
Validation now reuses the execution index's empty-prefix binding list and
extracts mode-entry targets once per binding before resolving them in each
reachable buffer-type/mode context. Overrides, removals and conditional branches
retain their validation semantics. No disk cache is introduced.

## 2026-11-28 — ARM debug whole-line selection extent

Stage 4b of [#132](https://github.com/runyte/runyte/issues/132) was measured
against its approved stage 4a parent using the same debug-profile
`preset_startup` protocol below: one discarded warm-up per binary and ten
alternating fresh-process samples on one `ubuntu-24.04-arm` runner.
[Run 46083639209](https://github.com/runyte/runyte/actions/runs/46083639209)
compares `1c8cd16b` with stage 4b `9ebe499e`.

| Phase | Stage 4a ms | Stage 4b ms |
| --- | ---: | ---: |
| Typed YAML parsing | 17.378 (17.226–17.485) | 17.419 (17.238–17.850) |
| Programs and metadata | 7.184 (7.162–7.217) | 7.141 (7.111–7.155) |
| Construction and lookup index | 89.379 (89.120–89.545) | 89.685 (89.264–90.114) |
| Total keymap initialization | 113.943 (113.647–114.145) | 114.224 (113.856–114.604) |

Cells are median (min–max). Total median increases by 0.281 ms (0.25%).
These component measurements do not establish end-to-end desktop readiness
or release-profile costs. Raw samples are attached to the linked run.

## 2026-11-28 — ARM debug named-mode initialization

The repeated ARM native-preview acceptance failure on
[#153](https://github.com/runyte/runyte/pull/153) prompted a same-runner
comparison with merged #152 (`5766b8f3`). These are isolated
`preset_startup` measurements, not end-to-end editor readiness. Both binaries
use `cargo build --locked --features startup-timing --example preset_startup`
on GitHub's `ubuntu-24.04-arm` runner. Each comparison discards one warm-up per
binary, then alternates ten fresh-process samples per binary.

Before the optimization, [run 46079777228](https://github.com/runyte/runyte/actions/runs/46079777228)
measured 119.718 ms baseline versus 160.124 ms at `5a1bed90`: +40.406 ms
(+33.75%). Construction/indexing accounted for +36.977 ms. Empty named
children repeatedly resolved the same inheritance chain before sharing their
lookup index.

The construction-local cache in `f4613782` skips nodes with neither bindings
nor tombstones when resolving an effective list. Nodes with either still walk
the complete raw inheritance chain; descendant tombstones can expose an
ancestor's longer sequence. Named identity and input policy remain separate.
No disk cache is introduced.

[Run 46081041091](https://github.com/runyte/runyte/actions/runs/46081041091)
measured the optimized version against the same baseline:

| Phase | #152 baseline ms | Optimized #153 ms |
| --- | ---: | ---: |
| Typed YAML parsing | 15.385 (15.243–15.481) | 17.328 (17.118–17.478) |
| Programs and metadata | 5.953 (5.911–5.993) | 7.153 (7.135–7.182) |
| Construction and lookup index | 97.650 (96.576–98.132) | 89.420 (89.120–89.701) |
| Total keymap initialization | 119.034 (117.750–119.404) | 113.935 (113.689–114.130) |

Cells are median (min–max). Total median improves by 5.099 ms (4.28%).
Debug-profile timings do not establish release or desktop readiness costs.
The preview fixture now waits for the expected host state and its acknowledged
painted frame instead of assuming a fixed pause is sufficient.

## 2026-11-27 — named binding modes (stage 4a)

Stage 4a of [#132](https://github.com/runyte/runyte/issues/132), based on
`5766b8f3` (merged #152). Same Linux host and stock release build command as
below. The readiness comparison uses `benchmarks/startup.py`'s `measure`
function, alternating the pre-stage-4a binary and current binary each round,
with ten measured fresh processes after one discarded warm-up per fixture and
binary. Home/XDG paths and the 120×40 PTY follow the standard harness.

| Implementation | Plain text readiness ms | Lua readiness ms |
| --- | ---: | ---: |
| Pre-stage-4a local release | 32.3 (30.3–36.2) | 32.1 (30.6–39.5) |
| Named binding modes | 31.8 (30.8–37.6) | 32.6 (31.7–38.7) |

Cells are median (min–max). Median changes are −1.8% and +1.5%, within the
5% readiness budget. Baseline SHA-256:
`12a8fe89289b6e8a977fc82312d96785f658c71e272d4834552afce1855799bb`;
current binary SHA-256:
`f99e6629eb933c69d7016487ddcfbd12b471e2bd7462abde8772129fea06a44c`.
These measurements cover the terminal edition on Linux, not desktop, macOS or
Windows.

The initial named-mode implementation measured 16.765 ms median cold keymap
initialization, including 13.151 ms in construction/indexing. It built the
legacy index before replacing it with the named index, and built identical
inherited lookup tables separately. Construction now starts with the named
graph and shares immutable indexes when their ordered effective binding
indices are identical. Named identities and unbound-key policies remain
separate. Inheritance leaf suppression uses bounded prefix-set lookups.
No disk cache is introduced.

Final ten-process `preset_startup` phase sample, median (min–max), ms:

| Phase | Time |
| --- | ---: |
| Typed YAML parsing | 2.752 (1.812–3.199) |
| Programs and metadata | 0.800 (0.559–1.018) |
| Construction and lookup index | 7.835 (6.326–9.718) |
| Total keymap initialization | 11.419 (8.701–13.303) |

## 2026-11-27 — embedded Runyte semantic preset

Stage 3 of [#132](https://github.com/runyte/runyte/issues/132), based on
`90dba021` (merged #151). Release builds with `cargo build --release --locked
--bin runyte`, AMD Ryzen AI 9 365, Linux 7.3.4-arch1-1 x86_64.
`benchmarks/startup.py --only runyte --runs 10 --fixtures short.txt,short.lua`
uses isolated home/XDG paths, a 120×40 PTY and one discarded warm-up per cell.
These are stock terminal-edition readiness measurements; no desktop or
instrumented file-loaded/syntax-ready result is claimed.

| Implementation | Plain text readiness ms | Lua readiness ms |
| --- | ---: | ---: |
| Rust tables (#151) | 30.5 (28.0–33.5) | 31.4 (28.2–33.4) |
| Initial YAML Value loader | 42.8 (33.3–151.4) | 37.4 (30.1–165.5) |
| Optimized typed YAML loader | 31.2 (29.9–36.6) | 30.7 (29.3–38.8) |

Cells are median (min–max), ten fresh processes. Baseline SHA-256:
`9cc144fcf10c204954a3f9d5388201292f4661152dbd3c3e927bfe635bb36e6a`;
optimized binary SHA-256:
`11b91e5116da047c03c9f6323ec4a1410cfb9bf198aa6846eaddfcbf4bb313bc`.
The initial loader had large outliers. Optimized median readiness changes are
+2.3% and −2.2%, within decision 39's deciding 5% readiness budget.

The initial uninstrumented keymap initialization measured 17.31 ms. Phase
profiling exposed repeated command/descriptor scans, YAML Value conversion,
and redundant exhaustive cross-scope validation. The optimized loader uses
indexed immutable command metadata, direct typed shape visitors, shared
paired-mode programs and omitted redundant YAML metadata. Full cross-scope
validation remains in every-preset tests; runtime schema and program checks
remain. The pre-existing index implementation is unchanged.

The retained `preset_startup` probe (see `benchmarks/README.md`) reports this
final ten-process sample, median (min–max), ms:

| Phase | Time |
| --- | ---: |
| Typed YAML parsing | 2.718 (1.814–3.102) |
| Programs and metadata | 0.769 (0.551–1.238) |
| YAML + programs per sample | 3.490 (2.365–3.987) |
| Existing lookup index | 7.143 (5.424–8.683) |
| Total keymap initialization | 10.635 (8.981–12.349) |

An earlier optimized ten-process sample measured 2.97 ms for YAML + programs;
CPU variation is visible in the unchanged index as well. The isolated preset
cost still exceeds the 2 ms target. Following the profiling amendment, no disk
cache is introduced, by decision: isolated YAML + programs is approximately
3 ms, above the 2 ms proxy, but readiness stays within 5%. The serial release
performance job checks a five-cold-process median against a generous 15 ms
parse + program ceiling (excluding the existing index). These
measurements do not establish equivalent timing on macOS or Windows.

## 2026-11-11 — login shell environment in the macOS launcher

[#104](https://github.com/runyte/runyte/issues/104): `Runyte.app` now asks the
account's login shell for its environment before it starts the editor, so a
Finder launch's window appears only after that shell has run its startup files.
The cost is the person's own startup files, so it is bounded by a 3-second
deadline rather than by Runyte; nothing else in startup changed.

Measured on an Apple M2, macOS 26.6.2, zsh 5.9 with one account's ordinary
dotfiles, using an unoptimized `runyte-app-launcher` started directly with a
launchd-like environment (`HOME`, `USER`, `LOGNAME`, `TMPDIR`,
`PATH=/usr/bin:/bin:/usr/sbin:/sbin`) and an `exec` target that exits at once.
Twenty launches after one discarded launch, from start to the target's exit:

| `SHELL` | Capture | Median ms (min–max) |
| --- | --- | ---: |
| `/bin/zsh` | runs `zsh -l -i -c` | 25.5 (24.6–26.8) |
| `/bin/tcsh` | skipped as unsupported | 9.5 (9.1–10.0) |

The capture added a 16 ms median here, against a window that appears 39–45 ms
into a launch in the 2026-11-09 set (measured on Linux, so not directly
comparable). Startup files that load a version manager or prompt framework can
take hundreds of milliseconds; that time is now spent before the window rather
than missing from terminals. Fixture-shell tests check that a hanging startup
file is cut off at the deadline. No editor readiness benchmark was rerun.

## 2026-11-09 — Runyte 0.4.4, editions and modes

Release `v0.4.4`, commit `50644e3`, built with
`cargo build --release --locked -p runyte -p runyte-desktop` and Rust 1.97.1.
Machine: AMD Ryzen AI 9 365 with Radeon 880M, 10 cores / 20 logical CPUs,
Linux 7.2.14-arch1-1, ext4, Mesa 26.2.6 (RADV), KWin 6.8.2. Python 3.14.9,
pyte 0.8.2 and wcwidth 0.8.3 from `benchmarks/requirements.txt`. The harness is
`benchmarks/editions.py`, at this measurement uncommitted on top of that
commit; [its guide](../../benchmarks/README.md#editions-and-modes) defines each
column, the desktop display and the pixel comparison.

| Binary | Bytes | SHA-256 |
| --- | ---: | --- |
| `runyte` (terminal) | 61,024,200 | `dfb4daf6bd449cd248319441e55918728691c5f3d140516adc2f09e5f5cc9f21` |
| `runyte-desktop` | 111,781,752 | `1f3f6982d73bccad56aff814ed5570deb4a5bc46eaa0ab6c17c2de9f24510009` |

Isolated home and XDG storage, LSP disabled, syntax enabled, 120×40 cells
(a 1080×800 window for the desktop edition, drawn on the Radeon GPU in an
isolated virtual KWin compositor). Each cell is **median (min–max),
milliseconds**, from ten warm-cache launches after one discarded launch; cell
order rotates each round. Every round started with at most 1.0 logical CPU
busy across the machine (101 rounds, 122 seconds of waiting in total); no round
timed out. The [samples](../../benchmarks/results/editions-2026-11-09.json)
retain all 640 launches, 30 idle windows and those waits. None failed.

### Ready to edit

| Fixture | Editor | Ide | Mux, cold host | Mux, reattach |
| --- | ---: | ---: | ---: | ---: |
| **Terminal edition** | | | | |
| `short.txt` | 29.7 (22.7–34.4) | 31.2 (24.8–41.6) | 95.0 (91.7–107.0) | 21.4 (16.9–30.9) |
| `medium.txt` | 32.5 (21.0–39.6) | 34.4 (23.8–59.3) | 103.7 (90.0–134.9) | 23.7 (19.1–25.1) |
| `long.txt` | 39.4 (30.3–46.2) | 43.5 (30.7–52.3) | 101.2 (88.7–110.2) | 23.7 (18.5–32.4) |
| `huge.txt` | 90.1 (84.1–137.1) | 97.1 (85.7–157.6) | 166.9 (137.5–204.9) | 20.6 (16.3–23.4) |
| `short.lua` | 33.6 (27.1–39.4) | 37.7 (29.2–44.7) | 105.8 (95.5–116.3) | 32.9 (29.6–55.0) |
| `medium.lua` | 31.4 (28.6–36.1) | 31.8 (27.8–38.9) | 105.6 (76.5–140.2) | 30.2 (23.7–38.1) |
| `long.lua` | 33.2 (27.9–55.1) | 40.3 (29.7–56.9) | 75.6 (64.5–104.5) | 25.6 (18.6–31.2) |
| `huge.lua` | 103.9 (86.4–131.3) | 97.5 (84.2–120.5) | 117.2 (109.8–201.1) | 19.6 (17.5–42.3) |
| **Desktop edition** | | | | |
| `short.txt` | 83.7 (69.1–95.8) | 85.5 (72.6–91.9) | 109.1 (99.2–117.2) | 80.6 (77.1–87.5) |
| `medium.txt` | 80.2 (63.5–93.5) | 83.9 (76.6–102.0) | 113.7 (102.6–117.8) | 83.8 (74.6–90.9) |
| `long.txt` | 83.8 (65.2–92.7) | 85.8 (78.5–94.0) | 128.7 (114.9–133.1) | 81.5 (70.4–87.2) |
| `huge.txt` | 127.5 (116.8–165.1) | 136.7 (112.6–179.9) | 179.8 (153.7–217.7) | 75.4 (69.4–81.8) |
| `short.lua` | 88.5 (77.2–95.9) | 85.4 (76.1–97.8) | 114.0 (102.2–117.0) | 84.2 (79.5–90.2) |
| `medium.lua` | 85.4 (80.2–94.8) | 86.2 (76.2–96.6) | 112.7 (102.2–114.4) | 78.0 (74.6–89.6) |
| `long.lua` | 82.7 (67.5–96.7) | 75.9 (68.4–104.6) | 91.7 (80.4–124.7) | 77.7 (74.6–84.2) |
| `huge.lua` | 118.5 (111.5–134.8) | 136.3 (109.9–144.6) | 161.2 (131.2–177.5) | 71.5 (65.4–74.7) |

Editor and ide readiness ranges overlap in every row of both editions. As in
the 2026-10-04 set, ide's additional services start after the first editable
frame, so the difference between the modes is background work, not time to the
first edit. A cold mux launch includes starting the host and the `:open`
command, because `--mux` takes a workspace rather than a file. Reattaching does
not grow with document size, since the host already holds the document.

The desktop edition's medians are 36–55 ms above the terminal edition for
editor and ide up to 50,000 lines (15–40 ms at 500,000 lines, where document
loading dominates), and 48–60 ms above it for reattach. The window itself
appears at a median of 39–45 ms. Its edit lands 14–17 ms after the document is
visible in every cell (medians), which is consistent with waiting for the next
presented frame; the harness cannot separate that from input handling.
Document-visible timings for every cell are in the samples.

Values are higher than the 0.3.5 table below, whose kernel and filesystem
differ. The 2026-10-04 set is closer, on ext4 with the same CPU, and its medians
are within about 9 ms of these with overlapping ranges. A different build and a
different rotation of cells mean that comparison neither establishes nor rules
out a regression.

### Quit or detach

Timed from the final key of `:q!`, or of `:detach` in mux, to process exit,
after the verified save.

| Fixture | Editor | Ide | Mux, cold host | Mux, reattach |
| --- | ---: | ---: | ---: | ---: |
| **Terminal edition** | | | | |
| `short.txt` | 4.9 (3.5–5.8) | 4.6 (4.2–5.9) | 4.1 (3.0–5.5) | 4.1 (2.7–5.2) |
| `long.txt` | 5.6 (4.2–6.5) | 5.9 (4.5–6.6) | 4.8 (4.1–6.2) | 4.3 (3.2–5.7) |
| `huge.txt` | 8.1 (7.4–9.6) | 8.6 (7.3–9.6) | 5.5 (4.3–6.5) | 5.1 (4.5–6.1) |
| `long.lua` | 8.7 (6.5–10.0) | 9.0 (6.5–10.1) | 5.8 (4.1–6.7) | 5.2 (4.5–7.7) |
| `huge.lua` | 16.7 (14.9–20.8) | 17.4 (14.9–21.2) | 4.8 (4.2–5.2) | 4.3 (3.7–5.1) |
| **Desktop edition** | | | | |
| `short.txt` | 11.1 (9.6–12.3) | 11.9 (8.7–14.2) | 13.7 (12.1–14.7) | 12.5 (9.7–13.8) |
| `long.txt` | 12.7 (10.6–15.5) | 12.5 (11.3–13.4) | 13.3 (11.7–15.8) | 13.7 (11.7–14.7) |
| `huge.txt` | 13.3 (11.7–16.0) | 14.8 (13.3–16.9) | 12.8 (12.0–14.8) | 12.3 (11.0–14.4) |
| `long.lua` | 15.0 (13.5–16.8) | 14.2 (11.8–16.1) | 13.3 (12.3–15.6) | 13.5 (9.7–14.7) |
| `huge.lua` | 22.2 (19.6–25.2) | 23.9 (20.0–26.8) | 13.1 (10.6–14.9) | 12.0 (10.8–13.4) |

The omitted `medium` and `short.lua` rows are within these ranges and in the
samples. Detaching leaves the document with the host, so mux does not pay the
cost of dropping it that grows with `huge.lua` in the other modes.

All 160 desktop mux detaches in this set exited. The same `:detach`
intermittently failed outside it, leaving the window open with `Broken pipe
(os error 32)` under a "Runyte could not start" heading: 5 of 60 launches on the
quiet machine and 17 of 60 while builds were running, when detaching without
first saving to another path. The cause has not been established; it is
tracked in [#108](https://github.com/runyte/runyte/issues/108). The harness
records such a sample as `detach_failed` rather than dropping it.

### Idle

`medium.lua`, five independent 16-second windows after 2.5 seconds of settling.
CPU is the client process tree plus any persistent host, excluding the
compositor and Xwayland. One 10 ms scheduler tick is about 0.06% of such a
window.

| Edition | Mode | CPU %, median (range) | Screen writes or repaints |
| --- | --- | ---: | ---: |
| Terminal | editor | 0.00 (0.00–0.06) | 0 in every window |
| Terminal | ide | 0.06 (0.00–0.12) | 0 in every window |
| Terminal | mux | 0.00 (0.00–0.12) | 0 in every window |
| Desktop | editor | 0.06 (0.00–0.12) | 0 in every window |
| Desktop | ide | 0.06 (0.00–0.06) | 0 in every window |
| Desktop | mux | 0.12 (0.00–0.12) | 0 in every window |

These are at or near the measurement floor in every mode and edition. Neovim
and Helix were not rerun for this set. These results describe this Linux machine
only and do not measure macOS, Windows, a physical display's latency, or a
Wayland window.

## 2026-11-08 — resize preview transition

Keyboard resize previews reuse the existing 80 ms presentation tick for a
160 ms entrance fade. The tick is enabled only while that finite transition
or an existing long-running action remains active. Any resize control ends
the fade immediately; completed previews add no idle timer. A regression test
advances the transition clock and verifies that animation demand ends. No
startup benchmark was run; no startup latency change is claimed.

## 2026-11-06 — exact home workspace

[#73](https://github.com/runyte/runyte/issues/73) captures canonical home
identities alongside startup safety roots. CLI initialization and App startup
reuse that capture; scanner policies retain the home-workspace decision, with
no home lookup during traversal or drawing. The startup echo reads the selected
state path. Home project scans reuse the existing 20,000-entry contained scan
budget. No controlled startup benchmark ran; no latency change is claimed.

## 2026-11-06 — out-of-tree state foundation

[#72](https://github.com/runyte/runyte/issues/72) adds explicit out-of-tree
placement. Ordinary in-tree placement does not invoke the per-user resolver;
a closure-based regression test asserts this boundary. Environment
fingerprinting hashes captured selector bytes without filesystem operations.

Reservation validation additionally compares in-tree state against the new
per-user state root, including canonical existing ancestors to detect aliases.
The [approved cost boundary](https://github.com/runyte/runyte/issues/72#issuecomment-6683697808)
permits bounded safety checks once at startup: user roots and canonical
reservations are captured once, and the selected project/location admission
is reused through CLI, application and host startup. App scopes and consumes
that admission even if construction fails before storage validation; a later
startup using a cloned configuration captures fresh safety information. This
applies to the new state-root checks, not preexisting workspace
discovery/layout checks. Regression tests count root resolution and
canonical normalization and verify admission reuse and fresh alias checks.

These checks add bounded metadata/canonicalization work without creating
state directories. No controlled startup benchmark ran; the latency effect
has not been quantified. Creation remains deferred until the workspace needs
state. The home-workspace selection policy is separate work in #73.

## 2026-11-06 — graphical launch preparation

Marked graphical launches resolve their relative file arguments once before
first-target workspace discovery and reuse the captured startup safety values.
Graphical mux sends one bounded initial target request over the existing native
attachment; its existing complete-frame barrier acknowledges readiness. No
polling timer or terminal discovery work is added. Native failure presentation
retains a bounded diagnostic until close instead of exiting the window.
Readiness and idle benchmarks have not been rerun for this change; no measured
performance improvement is claimed.

## 2026-10-25 — commented defaults and larger configured keymaps

Follow-up to [issue #31](https://github.com/runyte/runyte/issues/31): export
keeps explicit assignments active and writes built-in defaults as comments.
A default-only export therefore has no active `keys` section and skips
configured-keymap compilation on load. The assignment limit is 65,536;
this is an admission ceiling, not a promise of fast startup at that size.

`cargo bench --bench config_export` measures component costs in-process in a
release build. Each value is the median of three samples after one warm-up.
The compilation measurement retains both fast-pane-key variants together.
Export includes prospective validation and file replacement. Load reads the
exported files but does not compile keys. Files live in temporary directories.
These are not complete editor-readiness measurements.

| Explicit assignments | Load (ms) | Compile both variants (ms) | Export (ms) |
| ---: | ---: | ---: | ---: |
| 0 | 0.945 | skipped | 16.716 |
| 1 | 0.910 | 31.705 | 49.006 |
| 1,400 | 3.809 | 138.424 | 212.787 |
| 65,536 | 57.965 | 7,616.074 | 9,846.959 |

Rejection of 65,536 conflicting assignments across both variants took
11,326.158 ms. Diagnostic lookup indexes the first violation per assignment
instead of scanning all violations for each rejected rule.

Measured on Linux x86-64, AMD Ryzen AI 9 365, Rust 1.97.1. Other builds were
active on the machine; treat these values as an observed capacity check, not
an isolated baseline or a comparison with the earlier measurements below.
At the ceiling, compilation and export take seconds. Ordinary custom keymaps
still pay the existing configured-compilation cost; `:write-config` is not a
startup cache.

## 2026-10-18 — configuration directories and written keymaps

Configuration loading now lists `themes.d/` and `keybinds.d/` beside the
configuration file on every launch, and `:write-config` writes the whole
effective keymap into `keybinds.d/`, so a written configuration has 720
`keys.bind` entries for startup to compile. The same work applies `bind`
rules in one pass over the bindings instead of one pass per rule.

These are not readiness measurements and `startup.py` was not rerun. They
time the two functions on the startup path in-process: the configured keymap
compilation, `compile_configured_keymaps`, which builds both
`editor.fast_pane_keys` variants, and `Config::load`. Each was measured from a
temporary ignored test in a `cargo test --release` build, not committed. A
measurement is one warm-up and 21 samples. Each cell is the **median of the
three rounds' medians (range of those medians), milliseconds**.

- **Inputs.** A one-binding section (`bind: normal: F5: save`), and the
  written keymap, which is what `:write-config` produces for a configuration
  with no `keys` of its own, merged across its four mode files. For loading,
  `config.yaml` written for the default configuration on its own, and the
  same file with its four `keybinds.d/` files. No theme files, because the
  default configuration has no custom theme.
- **Revisions.** The baseline is `main` at `e5ed45f`, with the `keys.bind`
  limit raised from 256 to 1,024 for the measurement only; unpatched, it
  rejects the written keymap whole. The change is branch `config-split-files`
  at `f7bd812`.
- **Machine.** AMD Ryzen 9 3900X, 12 cores / 24 logical CPUs, Linux 6.18.56,
  ZFS, Rust 1.97.1.

| Work | Baseline | Change |
| --- | ---: | ---: |
| Compile one binding | 32.7 (31.9–32.7) | 34.1 (34.0–34.1) |
| Compile the written keymap | 83.6 (82.8–84.1) | 39.9 (39.7–40.0) |
| Load `config.yaml` alone | 1.03 (1.00–1.16) | 1.02 (1.00–1.02) |
| Load with `keybinds.d/` | — | 1.92 (1.91–1.95) |

Most of the cost of a configured keymap does not depend on its size: any
`keys` section costs about 34 ms here, against nothing for a configuration
without one, which reuses the built-in keymap. The written keymap adds about
6 ms to that, where the per-rule passes added about 51 ms. The fixed part is
the alias, namespace, and validation work that compares bindings with each
other, for both variants. It predates this change. The one-binding case is
about 1.4 ms slower after it; the rounds do not overlap, so it is probably the
per-compile terminal-key set and rule grouping rather than noise.

Listing two missing directories made no measurable difference to loading.
Four written key files add about 0.9 ms of parsing.

For scale only, since the machines differ: `short.txt` readiness in the
2026-10-04 set below was 29 ms. Configured keymap compilation is the same
order as that whole figure, so a configuration with any `keys` section,
written or not, is likely to show in a readiness rerun.

## 2026-10-09 — bundled Hayro PDF helper versus Poppler

Linux x86-64, AMD Ryzen AI 9 365, Rust 1.97.1 release build with repository
LTO settings, Hayro 0.8.0 and Poppler 26.08.0. Reproduce with:

```sh
cargo build --release --features native
python3 tests/native_pdf.py --binary target/release/runyte --compare
```

Three-request medians in milliseconds, warm filesystem caches. Every request
starts a fresh process and reparses the PDF, so these include helper startup;
no parsed-document cache is reused. Hayro base requests include raster and text
extraction and RGBA pipe transfer. Poppler base requests invoke `pdfinfo`,
`pdftoppm` and `pdftotext`; their totals include PNG encoding and file I/O but
exclude Runyte's subsequent PNG decoding. Detail requests render a 600×600 crop
at origin (200,200) of a 3200×2400 virtual page, without text extraction.
These are backend request timings, not native-window first-paint measurements.

| Authored fixture | Base Hayro / Poppler | Detail Hayro / Poppler | RGB mean absolute error (0–255) |
| --- | ---: | ---: | ---: |
| Standard Helvetica text | 11.6 / 71.4 | 4.5 / 19.4 | 0.850 |
| Two columns | 12.1 / 76.6 | 5.2 / 36.6 | 0.734 |
| Vector shapes and text | 10.4 / 69.1 | 6.6 / 25.8 | 0.120 |
| Empty-password RC4 PDF | 10.2 / 71.0 | 5.7 / 19.9 | 0.663 |
| CropBox, 0° | 9.6 / 76.7 | 6.0 / 22.0 | 1.397 |
| CropBox, 90° | 8.7 / 76.9 | 5.8 / 30.3 | 1.500 |
| CropBox, 180° | 10.1 / 76.6 | 6.3 / 24.5 | 1.273 |
| CropBox, 270° | 9.7 / 72.8 | 5.9 / 24.1 | 1.385 |
| RGB image scan | 11.1 / 65.6 | 6.5 / 30.5 | 0.000 |
| Form XObject | 10.0 / 75.6 | 6.9 / 21.8 | 1.375 |

RGB comparison samples every sixteenth pixel at matching physical dimensions;
the short axis is explicitly scaled because the backends round base dimensions
differently. All fixtures produce the same word multiset. The two-column case
has deliberately different reading order: Hayro follows authored content order
(left column, then right); Poppler groups both columns by horizontal rows.
Glyph/antialiasing differences remain even when words and vector positions match.
The mixed-backend Rust regression also verifies text-box overlap and at least
95% near-matching crop pixels across all four rotations.

This small, Runyte-authored corpus supports the default switch for ordinary
pages; it does not establish accuracy or performance for a broad PDF corpus.
JBIG2/CCITT/JPEG 2000 scans, CJK embedded fonts, complex interactive forms, real
papers, and large slide decks still need broader comparison. No third-party
PDFs are committed. Automatic Poppler fallback remains available for failures
and reported unsupported content, but cannot detect every silent visual or
reading-order difference. Nonembedded recognized Latin names may use Hayro's
standard substitutes. macOS actual-helper acceptance is wired into native CI;
these timings were measured only on Linux.

## 2026-10-08 — animated native media idle

`tests/native_window.py --animations`, debug native build, isolated X11/Xvfb
1280×960 with the lavapipe software Vulkan driver on Linux. The test verifies
two GIF frame colors, selection/copy pause, resume, and finite-loop completion.
Over two-second samples after 500 ms settling, the native main thread recorded
0 context switches per second in each of four states: selected animation,
hidden window, preview left, and finite animation finished. These are main-thread
measurements, not whole-process idle figures. No animation timer is armed for
ordinary still images or an editor with no visible animated media. Wayland and
macOS visibility gating was source-reviewed but not measured on those desktops.

## 2026-10-04 — plain sessions

Plain sessions have since become editor mode (`runyte --editor`, or `runed`);
the services it leaves out are the same, so the comparison still describes it.

A development build of the plain-session change on top of commit `ed5a1cc`,
built with `cargo build --release` and Rust 1.97.1. Machine: AMD Ryzen AI 9
365, 10 cores / 20 logical CPUs, Linux 7.2.8-arch1-2, ext4. Python with pyte
0.8.2 and wcwidth 0.8.3 from `benchmarks/requirements.txt`.

The comparison uses `measure` from `benchmarks/startup.py` with the same
isolated environment and fixtures, inside the fixtures' Git repository. The
only difference between the columns is `--plain`, so both launch from the same
directory and differ only in whether a workspace is adopted. Each cell is
**median (min–max), milliseconds** to the verified first edit, from ten
measured launches after one discarded warm-up, alternating which variant runs
first each round. LSP is disabled in both, as in every configuration here.

| Fixture | Workspace | Plain |
| --- | ---: | ---: |
| `short.txt` | 29.2 (24.6–31.7) | 28.4 (25.0–32.6) |
| `long.txt` | 40.1 (32.6–46.0) | 39.3 (36.2–41.3) |
| `short.lua` | 31.7 (27.4–45.4) | 27.8 (24.5–36.9) |
| `long.lua` | 31.4 (29.5–35.9) | 30.7 (25.7–32.5) |

Readiness is within run-to-run spread. This is expected: every service a
plain session leaves out — Git, the language-server manager attachment,
plugins, the context endpoint, and the session catalog — already starts after
the first editable frame, so leaving it out saves background work and child
processes rather than time to the first edit. Absolute values are higher than
the 0.3.5 table because this run did not follow its procedure (different
kernel, filesystem, and background load) and should only be compared within
its own rows.

## 2026-09-30 — Runyte 0.3.5

Release `v0.3.5`, commit `b937570`, built with `cargo build --release --locked`
and Rust 1.97.1. Machine: AMD Ryzen AI 9 365, 10 cores / 20 logical CPUs,
Linux 7.2.5-200.fc44.x86_64, btrfs. Neovim 0.12.5 and Helix 25.07.1
(`a05c151b`) are the installed packaged binaries. Python 3.14.7, pyte 0.8.2,
and wcwidth 0.8.3 run the harnesses from the same commit.

The build finished before measurement. The main harnesses ran sequentially;
background system activity was not controlled. Standalone measurements ran
inside the execution sandbox. Persistent measurements required execution
outside it so the temporary hosts could start and bind local sockets.
All configurations disable LSP and isolate editor settings and storage.
PTY geometry is 120×40. These results describe this Linux machine only.

### Readiness to edit

Each cell is **median (min–max), milliseconds**, from ten measured warm-cache
launches after one discarded warm-up per editor/fixture. Editor order rotates.
The harness inserts one leading space as soon as the first document line is
visible, waits for the edited line in a completed frame, then verifies a save
of the entire file. All 240 measured readiness samples passed; no successful
slow sample was removed. Initial syntax parsing may still be running.
The fixtures contain 500, 5,000, 50,000, and 500,000 lines; text and Lua pairs are
byte-identical, differing only by extension.

| Fixture | Neovim | Helix | Runyte |
| --- | ---: | ---: | ---: |
| `short.txt` | 20.9 (18.2–26.3) | 32.2 (26.7–38.1) | 18.8 (16.5–22.2) |
| `medium.txt` | 20.9 (17.5–25.8) | 32.7 (29.6–34.1) | 20.4 (18.4–24.5) |
| `long.txt` | 22.6 (19.8–26.9) | 33.0 (28.7–37.5) | 26.9 (22.7–32.7) |
| `huge.txt` | 38.6 (34.4–46.3) | 45.9 (38.5–48.9) | 88.0 (80.9–113.7) |
| `short.lua` | 42.3 (38.2–63.6) | 39.1 (32.9–88.1) | 20.6 (18.2–30.5) |
| `medium.lua` | 31.7 (26.0–33.3) | 63.7 (49.9–71.9) | 21.7 (17.8–35.5) |
| `long.lua` | 31.3 (29.0–33.5) | 349.0 (285.7–410.0) | 27.9 (24.1–32.5) |
| `huge.lua` | 45.7 (43.5–53.9) | 548.8 (542.9–550.9) | 102.5 (82.3–114.8) |

Runyte has the lowest median in five of eight rows; Neovim has the lowest
median for 50,000-line text and both 500,000-line fixtures. Several ranges
overlap. These results measure one visible edit near the start of a file, including the harness's decoding and I/O,
and do not establish a general editor ranking.

The [startup samples](../../benchmarks/results/startup-2026-09-30.json)
retain the original 180 readiness measurements and 60 supplementary Neovim
internal measurements. Runyte and Helix internal milestones were not measured
in this refresh; no probe builds were used. The separately collected
[500,000-line samples](../../benchmarks/results/startup-2026-09-30-huge.json)
add 60 readiness and 20 Neovim internal measurements with the same binaries
and settings later on the same date. Each new file is 17,498,766 bytes. The
expanded fixture generator is based on `b937570`; the artifact records hashes
of all measurement scripts because the fixture addition was uncommitted.
Large-file handling uses each editor's defaults; readiness does not establish
equal parsing or highlighting progress. See the
[startup methodology](../../benchmarks/README.md#startup-milestones).

### Persistent attachment

Each timing cell is **median (min–max), milliseconds**, from three samples.
Cold start ends at the first About-pane output of the last newly started host;
warm attach ends at a content marker in the restored one-line document.
These are output timings, distinct from demonstrated editing readiness.
Each attachment settles for 32 seconds before a 16-second idle observation.
CPU is a percentage of one logical CPU, summed across the attached TUI and
all benchmark hosts; it excludes terminal child processes. The noisy terminal
in another session produces a line every 20 ms.

| Scenario | Cold start | Warm attach | Editor CPU %, median (range) | Screen writes, each sample |
| --- | ---: | ---: | ---: | --- |
| One session | 34.23 (32.82–35.37) | 6.12 (6.00–7.07) | 0.00 (0.00–0.12) | 0, 0, 0 |
| Three sessions | 33.26 (33.13–33.80) | 6.80 (6.37–7.08) | 0.12 (0.00–0.19) | 0, 0, 0 |
| Three sessions, strip hidden | 34.04 (33.20–34.47) | 6.27 (6.26–6.57) | 0.12 (0.12–0.19) | 0, 0, 0 |
| Three sessions, another terminal producing output | 33.43 (33.15–33.63) | 6.70 (6.08–6.82) | 0.25 (0.25–0.37) | 0, 0, 0 |

The [persistent-session samples](../../benchmarks/results/session-navigation-2026-09-30.json)
retain all timings, CPU, output counts, observation durations, and process counts.

### Quit and idle

Quitting immediately after the first large Lua document frame takes
**4.9 (2.8–7.6) ms**, from ten samples after one discarded warm-up.
The interval starts when `:q` plus Enter is sent and ends at successful exit.
The stock binary does not report parser completion, so this measurement makes
no claim that a parse was still active. The
[early-quit samples](../../benchmarks/results/early-quit-2026-09-30.json)
retain all observations.

After terminal output settles, `run.py` measures quit from the final force-quit
keystroke to successful exit. Ten samples per fixture give **4 ms** for
50,000-line text and **22 ms** for 50,000-line Lua. Five independent ten-second
idle windows with 5,000-line Lua open in a Git repository give **0.00% CPU**
median (0.00–0.10% of one logical CPU) and **zero screen writes in every window**.
The [settled-quit and idle report](../../benchmarks/results/quit-idle-2026-09-30.md)
retains the harness output; this older harness retains aggregates rather than
individual samples.

### Reproduction and binary provenance

```sh
cargo build --release --locked
benchmarks/.work/venv/bin/python benchmarks/startup.py --runs 10 --json /tmp/startup.json
benchmarks/.work/venv/bin/python benchmarks/early_syntax_quit.py --runs 10 --json /tmp/early-quit.json
benchmarks/.work/venv/bin/python benchmarks/run.py --only runyte --runs 10 --idle-runs 5 --fixtures long.txt,long.lua
benchmarks/.work/venv/bin/python benchmarks/session_navigation.py --runs 3 --json /tmp/session-navigation.json
```

Use the pinned Python environment described in [the harness guide](../../benchmarks/README.md).
Run timing workloads sequentially after compilation and tests finish.

| Binary | SHA-256 |
| --- | --- |
| neovim | `31b1f7b2bbf9d790596e4f9a76f3b7f09c01cf501c314ffa68d365a7fc6a03b0` |
| helix | `3f31b5db36dec738e153fc027edb280063616df8823598df2da56c80c71542e5` |
| runyte | `c623c669dc5fbc4460f505657f261a604958670eb6491fb4d2d4b575a40e2a9e` |

## 2026-09-28 — large comparison edit-to-frame cost

The ignored `large_diff_edit_to_frame_latency` test in
`src/app/tests/diff_latency.rs` measures a transaction followed by one
`prepare_view` call in a release build. The fixture has 50,000 lines (about
2.5 MB); a leading-space difference makes the Git base and the two compared
buffers unequal. Each entry is the median of 40 alternating insert/delete
edits at line 25,000, after the initial alignment has settled. The before
column is `5e139bc` with the same measurement test temporarily added; the
after column is the asynchronous comparison implementation. Values exclude
terminal drawing and the later exact-result publication.

| Comparison | Edit | Before edit / frame / total | After edit / frame / total |
| --- | --- | ---: | ---: |
| Git gutter | Character | 2.304 / 935.097 / 937.542 µs | 3.337 / 4.088 / 7.465 µs |
| Git gutter | Newline | 2.245 / 939.697 / 941.861 µs | 3.356 / 4.068 / 7.404 µs |
| Side by side | Character | 3.256 / 1096.174 / 1099.391 µs | 4.659 / 7.454 / 12.144 µs |
| Side by side | Newline | 3.176 / 1094.771 / 1098.148 µs | 5.120 / 7.094 / 12.114 µs |

The separately ignored `large_diff_worker_cost` test in `src/app/diff_work.rs`
measures only exact comparison computation, with 10 warmups and 50 samples.
Its release medians were 3.213 ms for the Git gutter and 3.990 ms for the
paired view. The first prepared frame uses pending marks and provisional row
geometry; after the worker result publishes, both views use the same exact
alignment. These timings establish the first-frame reduction, not end-to-end
input latency or the delay until exact colors and alignment appear.

The ignored `large_diff_history_to_frame_latency` test uses the same 50,000-line
paired view and measures 40 undo/redo cycles after five warmups. It compares
`f2c810b`, which found changed rows by scanning the whole buffer on each history
step, with transaction replay. Each cell gives undo / redo median elapsed time
through the prepared frame; the frame-only medians are listed separately.
"Already dirty" means an earlier edit remains after undo, avoiding the
separate saved-text equality check in `Buffer::update_dirty`.

| State | Edit | Before total | After total | After frame only |
| --- | --- | ---: | ---: | ---: |
| Already dirty | Character | 21.325 / 21.327 ms | 7.886 / 6.943 µs | 3.587 / 3.737 µs |
| Already dirty | Newline | 20.630 / 20.542 ms | 8.216 / 8.386 µs | 3.797 / 4.108 µs |
| Clean after undo | Character | 31.398 / 21.141 ms | 9.476 ms / 5.090 µs | 3.267 / 2.525 µs |
| Clean after undo | Newline | 31.076 / 20.864 ms | 9.478 ms / 5.631 µs | 3.366 / 2.686 µs |

Undoing back to saved text still spends about 9.5 ms checking whether the
50,000-line buffer is clean; that cost is outside comparison geometry. The
already-dirty rows isolate the geometry improvement. As above, these are
in-process transaction-to-prepared-frame measurements rather than terminal
input-to-drawn-frame latency.

## 2026-09-28 — persistent cursor publications

Release builds of the full-frame protocol at `83e0ade` and the editor-row-delta
implementation were compared with the same local-host fixture on Linux. Each
case opened a 120-line plain-text document with isolated temporary workspace
and configuration storage, waited for startup publications, then sent
`h`, `l`, `h`, `l`, `j`, `k` through the attached client. The 120×40 case had one
visible pane; the 240×80 case had two. The table reports medians of the six
cursor publications. Wire size is the serialized host-to-client JSON response,
excluding its one-byte newline delimiter.

| Geometry | Full-frame bytes | Row-delta bytes | Full-frame input to draw | Row-delta input to draw |
| --- | ---: | ---: | ---: | ---: |
| 120×40, one pane | 18,777 | 1,309 (93.0% less) | 0.770 ms | 0.501 ms |
| 240×80, two panes | 68,672 | 1,348 (98.0% less) | 2.901 ms | 2.741 ms |

The respective full-frame and delta byte ranges were 18,775–18,904 and
1,182–1,627 at 120×40, and 68,619–68,848 and 1,171–1,616 at 240×80. Timing
ranges were 0.623–1.605 and 0.484–0.823 ms at 120×40, and 2.570–3.101 and
1.942–3.273 ms at 240×80. The timing ranges overlap, so these six-sample
results establish the wire reduction but only suggest a latency difference.
The timer starts before the input send and ends after JSON receive, frame
reconstruction, snapshot conversion, and rendering into Ratatui's
`TestBackend`. It does not measure a real terminal write, PTY flush, or
rendered acknowledgement, nor compare with standalone mode.

## 2026-09-19 — complete plugin value documents

Observed the coordinated database-viewer implementation on Linux x86-64 using
uninstrumented debug builds of Runyte 0.3.0 and ru-dbviewer. This is a native
acceptance observation from `ru-dbviewer/tests/native.py`, not a release startup
benchmark or a comparison with the release-build measurements below. No startup
ordering changed. Background load/format/publication and ordinary document input
are the measured paths.

A captured 4,944,933-byte JSON value became one 5,256,945-byte, 48,005-line
read-only document. The harness observed completion in 0.801 seconds and a
command prompt response in 0.100 seconds while loading was still in progress.
The harness includes a 0.3-second send drain and 0.1-second polling; these are
observed elapsed times, not isolated worker latency or a sub-millisecond input
measurement. Complete formatted copying, exact raw copying, tail search and
persistent detach/reattach passed in the same fixture.

Host-only Linux RSS/high-water marks were 50,052 KiB before loading, 72,392 KiB
with the full document open, and 144,888 KiB after copying formatted and raw
values into two additional editable documents. These observations exclude the
plugin process and do not assert a combined memory peak. Back at the beginning
of the full read-only document, after two seconds of settling, a one-second
sample recorded 0 ms host CPU at Linux process-counter resolution. A prior
sample taken while an editable copy was still settling is excluded. Non-Linux
or unavailable process counters are reported as unavailable.

The native cancellation test covers responsive input and the atomic
commit/cancel race. `tests/full_values.py` separately holds a staged commit to
prove cancellation wins before publication and leaves the preview intact. No
idle database polling or new periodic worker is introduced. These measurements
do not claim macOS, ARM64, or release-build performance.

## 2026-09-18 — scoped agent context access

Measured the completed agent-context implementation based on `0999b82`, built
with `cargo build --release --locked`, after builds, tests and coverage finished.
Machine: Linux x86_64, `7.1.13-200.fc44.x86_64`, AMD Ryzen AI 9 365,
20 logical CPUs, approximately 27.2 GiB RAM, Rust 1.97.1 and Python 3.14.7.
The binary is 53,124,576 bytes, SHA-256
`2b36a473b78b4bc14bc1182bb13e00f33e01559ec2d480658b0e8afe2d4128ff`.

`benchmarks/context_access.py --runs 3 --window 10 --latency-count 40`
rotates three cases through fresh standalone processes with isolated runtime
storage and configuration, disabled LSP, `short.lua`, and a 120×40 PTY. The
[raw artifact](../../benchmarks/results/context-access-2026-09-18.json) retains
all nine complete samples, 360 native-input latencies and timestamped reader
revision evidence. Failed harness setup/cleanup attempts were diagnosed and
fixed before this complete matrix; no successful sample in the matrix is dropped.

Startup requires a displayed document, an acknowledged native edit and a later
full-file verification of that edit. Readers and noisy output start after this
startup interval. Each idle CPU window lasts ten seconds after warmup; latency
measures input through rendered acknowledgment with a later full-file check.
The noisy case runs one hidden terminal producing roughly 50 wide lines/second
and two independent context readers polling at up to 20 Hz each. Both readers
must observe changing revisions inside each CPU and input interval. The fixture
stops its child process and verifies shutdown after measurement.

| Case | Startup median (range), ms | Editor CPU median (range), % | Input median / p95 / max, ms | Screen bytes per idle window |
| --- | ---: | ---: | ---: | --- |
| Access disabled | 12.11 (10.73–12.17) | 0.00 (0.00–0.10) | 1.96 / 2.75 / 3.66 | 0, 0, 0 |
| Enabled, no readers | 13.61 (9.86–14.67) | 0.00 (0.00–0.10) | 2.19 / 3.10 / 3.75 | 0, 0, 0 |
| Noisy terminal, two readers | 11.45 (10.94–11.65) | 7.00 (6.80–7.20) | 1.62 / 2.22 / 3.20 | 8,816; 8,778; 8,759 |

CPU percentages use one core as 100%. The noisy process-tree CPU range is
13.80–13.90%; it includes the shell and its children. This measures combined
terminal activity and context reads, not the isolated cost of reading. Quiet
enabled access produces no screen traffic and shows no measurable median idle
CPU increase at this sampling resolution. All 360 steady-state typing latencies
stay below 4 ms. Three startup samples per case do not establish statistical
equivalence, a historical regression comparison, or a macOS result.

## 2026-09-09 — complete Linux application workload matrix

Application implementation `f67289c`, measured with harness `104b31f` against
the retained `c7c18bd` release binary. Both builds use
`cargo build --release --locked`. Machine: Linux x86_64,
`7.1.13-200.fc44.x86_64`, AMD Ryzen AI 9 365, 20 logical CPUs,
approximately 27.2 GiB RAM, Rust 1.97.1 and Python 3.14. The measurement ran
serially after builds, tests and coverage finished, on September 9 in
Europe/Warsaw (September 8 UTC).

The [raw artifact](../../benchmarks/results/plugin-applications-2026-09-09.json)
retains all 150 startup samples, 27 workload windows, 240 input latencies,
workload progress checkpoints, binary sizes and SHA-256 identities. Only binary
paths were replaced with repository-relative labels. Binary size increased from
45,335,520 to 48,735,392 bytes (7.50%). The branch SHA-256 is
`4951842e178fc1d591e8e270a7fd258162f6fde1506a7014b85beae28b96e3a1`;
the base is `724984cd14f457584ec0162926dfa0ea74f60bbce10ab7c9f1ef31dedbdbc2ec`.

Every sample uses a fresh temporary Git workspace, isolated home/XDG storage,
disabled LSP, enabled syntax and a 120×40 PTY. Ten samples per startup cell
measure completed document output and a demonstrated first edit separately.
First-byte timing remains diagnostic and is available in the raw artifact.
Epoch 1 readiness is proved by invoking the checked-in uppercase example and
undoing its result outside the captured startup interval. Epoch 2 registration
must be acknowledged by the host. A separate fresh-process series measures the
first rendered, admitted native view following an explicit command.

Values below are median (minimum–maximum), in milliseconds.

| First document output | Base, disabled | Branch, disabled | Epoch 1, quiescent | Epoch 2, quiescent |
| --- | ---: | ---: | ---: | ---: |
| `short.txt` | 7.77 (6.89–10.64) | 8.23 (7.00–9.47) | 8.85 (7.64–10.07) | 8.85 (8.10–9.90) |
| `medium.lua` | 8.34 (7.13–9.87) | 8.90 (7.66–9.90) | 8.97 (7.89–10.55) | 8.72 (6.37–10.32) |
| `long.lua` | 15.00 (12.96–18.02) | 15.13 (13.44–17.63) | 14.59 (12.87–18.33) | 15.99 (13.45–21.55) |

| Demonstrated first edit | Base, disabled | Branch, disabled | Epoch 1, quiescent | Epoch 2, quiescent |
| --- | ---: | ---: | ---: | ---: |
| `short.txt` | 11.51 (10.70–13.98) | 12.57 (11.33–15.73) | 13.36 (11.47–16.90) | 13.09 (11.97–17.56) |
| `medium.lua` | 13.00 (11.40–17.69) | 12.50 (11.33–25.77) | 13.25 (11.96–33.54) | 13.10 (12.06–17.71) |
| `long.lua` | 18.84 (16.85–20.82) | 20.07 (17.26–23.51) | 18.88 (17.93–23.60) | 19.60 (18.02–25.35) |

| Separate native-view startup | Acknowledged registration | First usable view |
| --- | ---: | ---: |
| `short.txt` | 47.95 (44.33–56.99) | 65.96 (61.48–75.41) |
| `medium.lua` | 51.01 (44.82–59.44) | 75.46 (67.20–86.95) |
| `long.lua` | 57.64 (51.44–69.41) | 77.57 (69.03–92.30) |

Disabled first-document medians differ by at most 0.57 ms; demonstrated-edit
medians differ by −0.50 to +1.22 ms, with overlapping ranges. These samples show
no material disabled-startup regression on this machine, without claiming
statistical equivalence or turning the 16 ms sustained-input target into a
startup limit.

Each workload has three ten-second windows after 2.5 seconds of settlement.
CPU includes the editor/host and its descendants, including reaped-child CPU;
100% means one logical CPU. One 10 ms scheduler tick across a ten-second window
is approximately 0.10%, the measurement floor. Output counts observed PTY bytes
and read chunks, not editor write syscalls. A detached job has no frontend, so
its output measurement is unavailable. Reattachment and an authoritative job
query prove the same host still owns the running job after each window.

| Workload | CPU %, median (range) | PTY bytes, median (range) |
| --- | ---: | ---: |
| Base, disabled | 0.00 (0.00–0.00) | 0 (0–0) |
| Branch, disabled | 0.00 (0.00–0.00) | 0 (0–0) |
| Epoch 1, quiescent | 0.10 (0.00–0.10) | 0 (0–0) |
| Epoch 2, quiescent | 0.10 (0.00–0.10) | 0 (0–0) |
| Visible native view | 0.00 (0.00–0.10) | 0 (0–0) |
| 10,000-row, 4 MiB model | 0.00 (0.00–0.10) | 0 (0–0) |
| Detached finite job | 0.00 (0.00–0.00) | unavailable |
| Noisy helper and descendant | 7.00 (6.60–7.30) | 0 (0–0) |
| Repeated maximum-model publication | 60.30 (59.60–60.50) | 2,542 (2,542–2,573) |

The active publisher changes visible content, so its output is expected;
quiescent workloads introduce no output. Publication CPU includes Python model
encoding and all host work and is not an idle cost. Exact canonical model size
and row count are checked after host admission before measurement.

Each active-load case also measures 40 individual edits in each of three
windows. Timing runs from key write through the exact edited marker in a
completed synchronized terminal frame, including harness decoding. Untimed
60–81 ms spacing spans multiple publication cycles. Full saved-document bytes
must equal the original plus all 40 inserted characters. Bounded histories
prove helper output or accepted model commits during the input phase: every
publication phase contains 24 acknowledged commits and every helper phase
contains 15 increasing output checkpoints.

| Input under load, 120 samples each | Median | Range | p95 |
| --- | ---: | ---: | ---: |
| Noisy helper and descendant | 1.64 ms | 1.22–2.60 ms | 2.16 ms |
| Repeated maximum-model publication | 1.64 ms | 1.12–4.08 ms | 2.36 ms |

Both p95 values are below the recorded-machine target of 16 ms. An independent
quiet plugin also answered beside each busy owner. Its end-to-end command
probe took 94.11 ms (94.02–94.74) beside helper output and 95.71 ms
(93.79–96.45) beside publication; these include the harness's fixed 80 ms Escape
guard and command-prompt rendering and are not the per-keystroke metric.

All normal cleanup checks passed, including owned helper/descendant exit.
This completes the Linux release workload matrix; it does not supply macOS
test or coverage evidence. The application plan remains active until its
required supported-platform CI passes. These results do not measure physical
display/audio latency, remote network performance or service-account behavior.

## 2026-09-08 — application API development

Measured on Linux `x86_64-unknown-linux-gnu`, kernel `7.1.13-200.fc44.x86_64`,
AMD Ryzen AI 9 365 (20 logical CPUs, approximately 27.2 GiB RAM), with Rust
1.97.1. The retained base is `c7c18bd`; the measured branch is `e86fe54` plus
the in-progress application implementation. Both were built with
`cargo build --release --locked`. Builds, tests and coverage finished before
the serial measurement. Binary size changed from 45,335,520 to
45,837,312 bytes (1.11%). The retained binary SHA-256 values are:

- Base: `724984cd14f457584ec0162926dfa0ea74f60bbce10ab7c9f1ef31dedbdbc2ec`.
- Branch: `f98af0b21b937bdd56d2135623589a163c02a71381601f7f8ae6055f84a9935d`.

`benchmarks/plugins.py --applications` uses ten launches per startup cell in a
120×40 PTY, isolated home/XDG storage, disabled LSP and enabled syntax. All 120
launches emitted document text, settled and quit successfully. The enabled
cases run the checked-in epoch 1 uppercase example and the epoch 2 jobs example
through the same Python interpreter, with no command/job started during startup.
Values are medians; this harness does not retain startup ranges.

| First document output | Base, disabled | Branch, disabled | Epoch 1, quiescent | Epoch 2, quiescent |
| --- | ---: | ---: | ---: | ---: |
| `short.txt` | 6.1 ms | 5.9 ms | 5.9 ms | 6.3 ms |
| `medium.lua` | 6.2 ms | 6.8 ms | 6.2 ms | 7.0 ms |
| `long.lua` | 12.1 ms | 11.6 ms | 11.9 ms | 12.6 ms |

| Quit after settlement | Base, disabled | Branch, disabled | Epoch 1, quiescent | Epoch 2, quiescent |
| --- | ---: | ---: | ---: | ---: |
| `short.txt` | 3.3 ms | 3.3 ms | 3.8 ms | 3.0 ms |
| `medium.lua` | 5.8 ms | 6.8 ms | 6.8 ms | 5.9 ms |
| `long.lua` | 23.4 ms | 23.6 ms | 22.2 ms | 22.4 ms |

Idle uses three independent ten-second windows after 2.5 seconds of settlement.
CPU includes editor descendants. The native-view case opens `tasks.py` through
the command palette, requires rendered task text, and settles for another 2.5
seconds before measurement. All fifteen idle windows completed. Values are
median (minimum–maximum).

| Idle case | CPU | Screen writes |
| --- | ---: | ---: |
| Base, disabled | 0.10% (0.00–0.10) | 0 (0–0) |
| Branch, disabled | 0.00% (0.00–0.00) | 0 (0–0) |
| Epoch 1, quiescent | 0.00% (0.00–0.10) | 0 (0–0) |
| Epoch 2, quiescent | 0.00% (0.00–0.00) | 0 (0–0) |
| Native task-list view | 0.00% (0.00–0.00) | 0 (0–0) |

These are representative first-document-output and idle measurements. They do
not establish full editing/plugin readiness, input-latency p95, statistical
startup equivalence, large-view/flood performance, detached job cost or the
provider/helper workload matrix. Native macOS evidence was outstanding at this
measurement; the [completed application plan](../plans/completed/PLAN_PLUGIN_APPLICATIONS.md)
records the later full workload measurements and native-platform CI acceptance.

The initial real-view check exposed an inherited palette bug: plugin commands
were listed but Enter resolved only built-in names. The corrected implementation
and a rendered-content setup guard are included in this measurement; failed
setups cannot produce zero-cost idle results.

Reproduce after building and retaining both releases, with the benchmark Python
requirements installed:

```sh
python3 benchmarks/plugins.py --before /path/to/base/runyte --applications
```

## 2026-09-07 — experimental process plugins

Measured on Linux `x86_64-unknown-linux-gnu`, AMD Ryzen AI 9 365 (20 logical
CPUs), with Rust 1.97.1. A retained release binary from `ef1bf58` was compared
with the plugin implementation on that base, built with `cargo build --release
--locked`. Builds, tests and coverage finished before measurement. No dependency
was added. The stripped binary grew from 45,093,088 to 45,335,520 bytes (0.54%).

The comparison uses the existing `ptybench.median_startup` and `median_idle`
functions, now exposed for repetition by `benchmarks/plugins.py`. Each startup
cell is the median of ten launches in a 120×40 PTY; every sample emitted the
document marker, reached the quiet guard and quit successfully. Home and XDG
storage were isolated, LSP was disabled, syntax stayed enabled, and the example
case explicitly enabled `docs/plugins/uppercase.py` through the Python interpreter
running the harness. These are representative fixtures, not a complete matrix.

| First document output | Base, plugins absent | Branch, plugins disabled | Branch, example enabled |
| --- | ---: | ---: | ---: |
| `short.txt` (500 lines) | 6.2 ms | 5.5 ms | 6.2 ms |
| `medium.lua` (5,000 lines) | 6.6 ms | 6.7 ms | 6.3 ms |
| `long.lua` (50,000 lines) | 11.1 ms | 11.5 ms | 11.8 ms |

| Quit after settlement | Base, plugins absent | Branch, plugins disabled | Branch, example enabled |
| --- | ---: | ---: | ---: |
| `short.txt` | 2.9 ms | 2.9 ms | 3.6 ms |
| `medium.lua` | 7.0 ms | 6.5 ms | 6.8 ms |
| `long.lua` | 22.7 ms | 22.5 ms | 22.6 ms |

Idle used three independent ten-second windows on `medium.lua`, after the
harness's 2.5-second settlement delay. CPU includes the editor and descendants;
all windows completed. Values below are median (minimum–maximum).

| Idle measurement | Base, plugins absent | Branch, plugins disabled | Branch, example enabled |
| --- | ---: | ---: | ---: |
| CPU | 0.00% (0.00–0.00) | 0.00% (0.00–0.10) | 0.00% (0.00–0.00) |
| Screen writes | 0 (0–0) | 0 (0–0) | 0 (0–0) |

The nonzero disabled-plugin window remains visible rather than being erased by
its zero median. These small samples do not establish a statistically precise
startup difference. They show similar first-document-output medians and no idle
screen writes. First document output does not measure full syntax completion,
input readiness or plugin registration readiness. Plugins start with the optional
host services after the first standalone frame. When disabled they start no task,
process, channel or timer; enabled workers wait on pipes/channels, and buffer
observations share existing host turns rather than adding polling. Work performed
by arbitrary enabled plugins is outside these example measurements.

Reproduce with a retained base release binary and the current release:

```sh
python3 benchmarks/plugins.py --before /path/to/base/runyte
```

## Reading these numbers

Comparisons of Runyte against its own later result sets are the intended use.
Fixtures are generated from a fixed seed, so they do not change when Runyte's
source does. Differences between result sets on the same machine still need to
be judged against ordinary run-to-run variation; a few milliseconds are not by
themselves evidence of a code improvement.

The fixture matrix is one document at 500, 5,000 and 50,000 lines, written twice
per size as `.txt` and as byte-identical `.lua`. The `.txt` fixtures carry no
language for any editor. Every editor measured here has one tree-sitter Lua
grammar active for `.lua`. In the first-output measurements retained below,
document content can be emitted before parsing or highlighting completes.
The difference within a same-size pair shows
how language treatment delays that output event, not the complete cost of
parsing the document.

Parser availability was checked for the historical result sets rather than
inferred: Neovim reported an active tree-sitter highlighter with `filetype=lua`
and no regular-expression syntax,
Helix reported its Lua parser and highlight queries present, and Runyte's release
build contains its statically linked Lua grammar and queries. That check makes
the fixture configuration explicit; it does not turn first content emitted into
a parser-completion benchmark. The new internal probes observe successful
parse completion explicitly; stock-binary readiness requires a rendered and
subsequently verified edit. Readiness and internal milestones come from
separate launches and may complete in different orders across editors. Their
medians cannot be subtracted to isolate a stage's cost.

Absolute values are machine-specific and are not comparable between result sets
taken on different hardware.

The default startup path does not compile a configured keymap: it clones one
`Arc` from the already initialized built-in variant. When a `keys` section is
present, startup compiles and validates both `editor.fast_pane_keys` variants
once so a later settings preview only selects an already diagnosed map. This
bounded, configured-only work has not been added to the readiness
measurements; the 2026-10-18 set times it in-process.

The file monitor's two-second reconciliation reads the directory of every open
explorer, because a listing has no cheaper baseline to compare first. The read
happens on the monitor thread and forwards nothing when the listing is
unchanged, so it does not wake the editor. The fixtures below open a document
rather than an explorer, so this work is not in the measurements either.

## Mouse selection autoscroll

Document selection drags at a pane's top or bottom edge schedule one visual-row
scroll every 60 ms in standalone and attached persistent modes. Each tick
prepares the current geometry and resolves the held pointer against the newly
scrolled rows. The deadline exists only during an edge drag; moving inward,
releasing the button, keyboard input, or detachment cancels it. Late ticks
advance once without catching up in a burst. No startup work or idle polling
is added. Startup and idle measurements have not been repeated for this change.

## Git discovery recovery

Failed repository discovery can be retried explicitly through `:git-refresh`.
It uses the existing asynchronous Git service, adds no timer or automatic
retry, and allows only one discovery attempt at a time. Each of the three
discovery reads now uses the existing 30-second local-read deadline and output
bound. Startup and idle measurements below have not been rerun for this change.

## LSP permission startup change

Workspace startup now performs a bounded read of one exact-root permission
record before attaching language services. The manager starts denied and
cannot launch a server until the owning host grants permission. An undecided
workspace with LSP enabled opens a choice overlay. No polling timer is added.
The measurements below predate this change and have not been rerun; readiness
benchmarks now explicitly disable LSP in their isolated configuration while
keeping syntax highlighting enabled. This applies to both `startup.py` and
`run.py` through their shared setup, including instrumented launches. The
first-open choice overlay therefore cannot intercept measurement keystrokes.

## 2026-09-07 — rendered table wrapping

Rendered Markdown pages retain table cell ranges when generated. Column
measurements are shared across a table, including cached tab-width-dependent
measurements. Per-row geometry is reused across navigation and redraws at the
same pane width. Each rendered buffer retains at most 16 table layouts and
2 MiB of their geometry payload; larger layouts are computed without retention.
Viewport snapshots visit visible cell fragments rather than each complete
logical row. No startup scan or polling timer is added.

The shared segment type now also identifies structured table rows. Ordinary
soft-wrap retention remains limited to 8 MiB of segment payload, with its
segment-count ceiling derived from the type's size instead of a fixed count.
Cloned buffers retain immutable table metadata but discard row geometry.

The existing release-mode
`moving_through_wrapped_json_stays_responsive_at_every_depth` check passed on
Linux x86-64 at base `c655304` plus this change. Its 3,277,786-byte JSON fixture
measured slowest movement-plus-snapshot times of 0.58 ms at the beginning,
0.53 ms halfway through, and 0.51 ms near the end, all below its 16 ms budget.
The check ran serially after the correctness and coverage suites completed;
these timings measure ordinary soft wrapping, not table layout construction.

Geometry and interaction coverage lives in `src/table_layout/tests/mod.rs`
and `src/app/tests/markdown_tables.rs`. Startup and idle measurements have not
been repeated for this change.

## 2026-09-07 — soft-wrapped single-line navigation

Measured on AMD Ryzen AI 9 365, Linux x86-64, Rust 1.97.1, with the default
release profile. The baseline is `c56eb96`; the comparison adds cached wrap
geometry, direct rope seeks in snapshot rendering, and contiguous visible
highlight queries. Syntax parsing completes before timing. These are semantic
movement plus snapshot measurements at 120×40, excluding terminal output and
initial layout construction.

An initial probe on the existing 1,693,791-byte `minified_json` fixture averaged
six alternating down/up commands at each location:

| Cursor location | Before | After |
| --- | ---: | ---: |
| Beginning | 16.88 ms | 0.53 ms |
| Halfway through | 85.53 ms | 0.48 ms |
| 2,000 characters before the end | Not measured | 0.49 ms |

The baseline stopped at the middle location when its 50 ms assertion failed.
The end-of-line value is therefore an after-only observation.

Previously, each projected movement recomputed the whole logical line's wrap
boundaries several times. Each visible row then traversed its hidden prefix
through a character iterator. The prefix traversal repeated for every screen
row, making scrolling progressively slower toward the end. Adjacent wrapped
rows also restarted the syntax query separately.

`wrap::Cache` retains at most 16 layouts and 262,144 segments per buffer
(8 MiB of segment payload on a 64-bit target). Its key includes the text
revision, logical row, pane width, and tab width. Clones drop derived geometry;
edits, undo, redo, and text replacement invalidate it through text revisions.
Layouts larger than the retention budget are computed without caching.
The 64,000,000-byte soft-wrap refusal remains in place for first-layout and
resize costs. Startup and idle measurements have not been rerun.

Regression coverage is
`moving_through_wrapped_json_stays_responsive_at_every_depth` in
`tests/performance.rs`: a fixture exceeding 3 MB, beginning/middle/end positions,
48 downward and 48 upward movements per position, with highlighting and cursor
movement asserted and the slowest movement plus snapshot limited to 16 ms.
The final 3,277,786-byte fixture measured a slowest movement of 0.60 ms at the
beginning, 0.54 ms halfway through, and 0.54 ms at 10,000 characters before the
end, over 96 movements per location. All 19 release performance tests passed
serially with `cargo test --release --locked --test performance -- --ignored
--test-threads=1 --nocapture`.

Run the test serially in release mode with the other performance gates. Unicode
coordinates, tab stops, layout eviction, edits and undo are covered in
`src/wrap/tests/cache.rs`; deep Unicode JSON text and highlight preservation
across resizing are covered in `src/snapshot/tests/long_lines.rs`.

## 2026-09-06 — persistent session navigation

Release build from base `9750cd0` plus the session-navigation implementation,
Rust 1.97.1, AMD Ryzen AI 9 365 (20 logical CPUs), Linux 7.1.13. The full binary
hash and individual samples are in
[`session-navigation-2026-09-06.json`](../../benchmarks/results/session-navigation-2026-09-06.json).
Builds, tests, and coverage had finished before measurement.

The new `benchmarks/session_navigation.py` uses a 120×40 PTY, temporary
configuration and workspace roots, disabled LSP, and three samples per scenario.
Cold startup ends at the initial About pane's first output from the last newly
started host. Warm attachment ends at a unique content token in a restored
one-line text document, beyond the styled caret character. These measure visible
output, not demonstrated editing readiness or completed syntax parsing, and are
not directly comparable to the standalone readiness table below.

The attached TUI settles for 32 seconds, covering discovery and the following
attention observation, before a 16-second idle window. CPU sums each editor
process's own user and system time: the attached TUI plus one or three hosts.
It excludes terminal children and is expressed as a percentage of one logical
CPU. Screen writes count PTY reads containing output, with byte totals retained
in the sample file. Zero reads establishes zero output independently of read
chunk boundaries.

| Scenario | Cold start, median | Warm attach, median | Editor CPU, median | Screen writes, each sample |
| --- | ---: | ---: | ---: | --- |
| One host, automatic strip | 32.59 ms | 5.22 ms | 0.00% | 0, 0, 0 |
| Three hosts, automatic strip | 33.32 ms | 6.13 ms | 0.06% | 0, 0, 0 |
| Three hosts, hidden strip | 33.48 ms | 6.37 ms | 0.06% | 0, 0, 0 |
| Three hosts, remote noisy terminal | 33.20 ms | 6.55 ms | 0.25% | 0, 0, 0 |

The noisy case writes a line every 20 ms in another host's terminal. Its host
still consumes and emulates that output, explaining editor CPU above the quiet
cases, while the attached editor emits no unchanged frames. Automatic strip
discovery runs asynchronously every 15 seconds without Git; scalar requests are
bounded and coalesced. Hidden/zen presentation skips attention-only reads, and
detached hosts do not run strip observation. The hidden and quiet medians are
within the timer tick's resolution; these three-sample runs establish a baseline,
not a measured improvement over earlier code. Native macOS was not measured.

The subsequent reattachment fix also requests one asynchronous observation on
each successful frontend attachment, bypassing the retained 15-second deadline.
An in-flight scan from before that attachment is replaced after it completes;
the cached rows remain until the new result arrives. The idle interval and
detached-host behavior are unchanged. The measurements above predate this fix.

## 2026-09-07 — document readiness before initial syntax

Runyte 0.2.0 at `aee17b0` was measured before and after the asynchronous
initial-syntax change. Both runs used the same AMD Ryzen AI 9 365 Linux host,
120×40 PTY, isolated home/XDG configuration, warm-cache fixtures, and ten
measured launches after one discarded warm-up per cell. Editor order rotated.
Builds and tests finished before measurements; desktop applications remained
running. Both comparisons ran in the same execution sandbox.

Neovim was 0.12.5 and Helix was 25.07.1 (`a05c151b`). Rust builds used
1.97.1; the harness used Python 3.14. Runyte readiness used the ordinary release
build. Internal Runyte milestones used a separate disposable build of the
same source with `startup-timing` and the updated observation patch. The patch
observes successful initial parsing on the worker, independently of when a
frame is shown or a result is applied. No current Helix probe build was used.

Every cell below is **median (min–max), milliseconds**. All 180 readiness
samples in each comparison passed whole-file save verification; no successful
slow sample was discarded. The before run also contains 60 Neovim internal
samples; the after run contains 120 Neovim/Runyte internal samples.

### Ready to edit — stock binaries

| Fixture | Runyte before | Runyte after | Neovim after | Helix after |
| --- | ---: | ---: | ---: | ---: |
| `short.txt` | 15.8 (13.2–16.7) | 15.3 (13.7–16.0) | 20.9 (16.8–22.5) | 29.6 (27.9–32.6) |
| `medium.txt` | 17.7 (14.9–25.9) | 16.6 (13.8–18.8) | 20.8 (18.0–25.1) | 30.5 (27.0–34.0) |
| `long.txt` | 24.4 (21.0–27.2) | 23.4 (19.5–27.3) | 23.5 (20.1–26.6) | 34.5 (30.2–47.0) |
| `short.lua` | 21.7 (18.6–23.9) | 17.4 (14.3–25.5) | 38.8 (37.4–44.2) | 38.5 (31.9–40.9) |
| `medium.lua` | 34.0 (30.8–43.6) | 17.6 (15.3–21.7) | 29.3 (27.5–30.8) | 52.7 (49.2–66.7) |
| `long.lua` | 160.9 (153.6–170.4) | 22.3 (20.0–30.0) | 29.7 (26.6–32.3) | 297.6 (289.2–314.6) |

### File loaded and syntax ready — instrumented, after change

| Fixture | Runyte file loaded | Runyte syntax ready | Neovim syntax ready |
| --- | ---: | ---: | ---: |
| `short.lua` | 5.7 (5.1–8.2) | 11.8 (10.7–14.5) | 20.3 (15.8–28.4) |
| `medium.lua` | 6.7 (5.5–8.9) | 26.0 (22.8–30.3) | 33.0 (28.6–42.4) |
| `long.lua` | 11.0 (10.3–16.0) | 156.9 (148.1–180.6) | 163.0 (157.9–182.4) |

The large Lua fixture is ready to edit in 22.3 ms instead of 160.9 ms,
about seven times sooner, and below Neovim's 29.7 ms median in this run.
Runyte's initial parse still completes around 156.9 ms. These measurements
show editing becoming available earlier; parsing itself takes a similar time.
The large plain-text medians are close before and after (24.4 and 23.4 ms),
and the after result is similar to Neovim's 23.5 ms. Overlapping ranges and
ordinary run-to-run variation prevent universal editor rankings. Readiness
and internal milestones come from separate launches and their medians must
not be subtracted to isolate stage costs.

### Quit during initial parsing

`early_syntax_quit.py` decodes the first large Lua document frame and sends
`:q` immediately from Normal mode. Ten measured samples follow one discarded
warm-up, with the same isolated configuration and binaries as above. The
interval starts when the command is sent and ends at successful process exit.
Stock Runyte takes 2.7 ms (2.5–4.7); the instrumented build takes 3.3 ms
(2.6–6.4). None of the ten instrumented launches reported syntax completion
before exit. The [individual early-quit samples](../../benchmarks/results/startup-2026-09-07-async-syntax-early-quit.csv)
retain those observations; the stock binary has no parser-completion probe.

### Settled quit and idle

The existing `run.py` harness, run afterward with ten quit samples per fixture,
measured settled-document quit at 4 ms for `long.txt` and 22 ms for `long.lua`.
The same Lua quit measurement on the pre-change binary was also 22 ms.
These settled results include ordinary cleanup of an accepted document tree;
they do not isolate its destructor cost. Three independent ten-second idle
windows with `medium.lua` open in a Git repository measured 0.00% CPU median
(range 0.00–0.10% of one logical CPU) and zero screen writes in every sample.

### Samples and provenance

The [before samples](../../benchmarks/results/startup-2026-09-07-async-syntax-before.csv)
and [after samples](../../benchmarks/results/startup-2026-09-07-async-syntax-after.csv)
retain every measured value, including plain-text internal milestones.
The [preliminary samples](../../benchmarks/results/startup-2026-09-07-async-syntax-first-pass.csv)
predate the second review's worker-side tree-disposal correction and are
retained separately; their large Lua readiness median was 23.1 ms. That stock
binary's SHA-256 was
`db558ce7051160f4121688b8c7f0434e923a08954dd95737ff6010c5a360445d`, and its
probe's was `8bb12a579eb01ae8f1f9d27dcb554be88a42ca26440e9cad5c606b1b7bf6e1cb`.
The tables above and binary hashes below describe the final reviewed build.

| Binary | SHA-256 |
| --- | --- |
| Runyte before | `b10630314605080e2021192a95ec4740918155d49b0391fafb2fadfd8d7dc160` |
| Runyte after | `f8ebe84e611f4ade1bd4c15834d1fb191dfdffab39de6781823248108225d20d` |
| Runyte after probe | `d85747f556e7bc0d9a3620c667d4ee735a9e77c5341f3634f2d2008eb7649a89` |
| Neovim | `31b1f7b2bbf9d790596e4f9a76f3b7f09c01cf501c314ffa68d365a7fc6a03b0` |
| Helix | `3f31b5db36dec738e153fc027edb280063616df8823598df2da56c80c71542e5` |

## 2026-09-05 — readiness, loading, and syntax

Machine: AMD Ryzen AI 9 365, 10 cores / 20 hardware threads, approximately
27 GiB RAM visible to the OS, Linux 7.1, btrfs. Measurements ran outside the
filesystem/process sandbox after compilation and tests finished. Ordinary
desktop applications remained running.

- Neovim 0.12.4, packaged executable.
- Helix 25.07.1 (`a05c151b`), packaged executable for readiness.
- Runyte 0.1.10 from `8c0bcba`, default release build for readiness.
- Internal Helix and Runyte observations use disposable release builds from
  those source revisions with Rust 1.97.1 and the supplied milestone patches.
  Runyte additionally enables `startup-timing`. Helix uses the installed
  `/usr/lib64/helix/runtime` for both builds.
- Python 3.12.12, pyte 0.8.2, wcwidth 0.8.3. No Lua language server installed.

Each cell reports **median (min–max) in milliseconds**, from ten measured
launches following one discarded warm-up. Editor order rotates each round;
home and XDG storage are isolated; the PTY is 120×40. These are warm-cache
measurements. All 360 measured launches completed, and every readiness sample
passed whole-file save verification. No slow successful samples were removed.
The [individual samples](../../benchmarks/results/startup-2026-09-05.csv) retain
the measurements behind every cell; empty syntax fields for plain text mean
not applicable.

Readiness means inserting **one leading space** and observing the first
document line shift by one cell at its recorded position. After timing ends,
saving to a temporary file must reproduce the whole original document with
exactly that extra space. The insertion keeps Lua valid and introduces no
injected grammar. Readiness uses ordinary binaries; the internal measurements
use separate, unedited launches. The
[methodology and probe setup](../../benchmarks/README.md#startup-milestones)
define the precise observation sites and their overhead.

### Ready to edit — stock binaries

| Fixture | Neovim | Helix | Runyte |
| --- | ---: | ---: | ---: |
| `short.txt` | 22.5 (19.7–24.0) | 29.7 (26.7–33.4) | 15.1 (13.2–31.1) |
| `medium.txt` | 23.3 (20.2–24.1) | 31.6 (27.6–44.2) | 15.5 (13.6–17.2) |
| `long.txt` | 25.3 (22.1–37.7) | 31.1 (28.6–34.4) | 23.0 (20.0–25.1) |
| `short.lua` | 41.0 (39.1–43.1) | 34.7 (31.6–53.5) | 21.2 (17.9–27.6) |
| `medium.lua` | 32.0 (28.6–49.4) | 53.1 (49.3–59.0) | 32.7 (30.8–36.6) |
| `long.lua` | 32.2 (31.7–35.9) | 295.7 (290.9–303.3) | 157.5 (154.8–188.4) |

### File loaded — instrumented

| Fixture | Neovim | Helix | Runyte |
| --- | ---: | ---: | ---: |
| `short.txt` | 11.2 (10.2–12.2) | 15.7 (12.4–18.0) | 5.3 (4.1–6.3) |
| `medium.txt` | 10.9 (9.8–11.7) | 15.5 (12.6–18.5) | 6.3 (4.6–7.2) |
| `long.txt` | 14.1 (12.2–14.6) | 18.0 (15.4–19.6) | 12.8 (10.9–14.7) |
| `short.lua` | 11.7 (10.4–12.5) | 14.7 (12.3–18.2) | 5.1 (4.1–6.3) |
| `medium.lua` | 12.1 (9.3–12.5) | 16.7 (13.3–18.7) | 6.7 (5.3–7.5) |
| `long.lua` | 12.8 (9.6–13.9) | 17.1 (12.4–19.3) | 11.9 (9.9–15.3) |

### Syntax ready — instrumented

| Fixture | Neovim | Helix | Runyte |
| --- | ---: | ---: | ---: |
| `short.txt` | not applicable | not applicable | not applicable |
| `medium.txt` | not applicable | not applicable | not applicable |
| `long.txt` | not applicable | not applicable | not applicable |
| `short.lua` | 18.9 (17.1–22.6) | 18.3 (15.8–22.2) | 8.9 (7.4–11.0) |
| `medium.lua` | 32.8 (29.7–39.9) | 36.8 (32.6–39.8) | 22.8 (19.9–26.6) |
| `long.lua` | 162.7 (160.7–168.7) | 206.8 (199.0–224.4) | 153.1 (144.6–175.1) |

### Interpretation

Runyte has the lowest readiness median on all three plain-text fixtures and
the small Lua fixture. Neovim and Runyte are close on the medium Lua fixture:
32.0 and 32.7 ms, with overlapping ranges. Neovim is substantially earlier on
the large Lua fixture: 32.2 ms against Runyte's 157.5 ms and Helix's 295.7 ms.

The internal observations explain why reading the entire file and being ready
to edit are different questions. All three load the large Lua document into
their native text buffers well before its syntax is ready. Neovim demonstrates
a displayed edit before its separate initial-parse measurement completes.
Runyte's startup ordering prepares syntax before exposing document text.
Readiness also includes processing the insertion and displaying the result;
it is not a timestamp for the end of initial parsing.

The columns do not measure equal background progress, but the readiness
comparison requires the same completed user operation in each editor. It
covers one edit near the start of these fixtures, not general editing latency,
every language, or physical terminal rendering. Internal values include
instrumentation overhead and may reflect compiler and packaging differences.
Because readiness and internal values come from separate launches, subtracting
their medians does not isolate parsing, drawing, or input-processing cost.
Quit and idle figures below remain the earlier dated measurements.

### Provenance

The measurement harness is identified by content because these scripts were
not yet committed when the run was taken:

| File | SHA-256 |
| --- | --- |
| `benchmarks/startup.py` | `3f421fd66508c2d8bbc6d5357b3c9025e0176401d10d99349b06ab4ed0abab9a` |
| `benchmarks/ptybench.py` | `cfb688cdeb6e513ce2b1eb5692669f29ad345ad4acfe039034f583d2a095f037` |
| `benchmarks/neovim_milestones.lua` | `821cbffab00dd7936522e3f8c65eb7f1b7d7187901a0680d3833206f00232c68` |
| `benchmarks/milestone_probe.rs` | `9469f335e4d8d1442066c22657f663e4d338b35b6a330ca45dcaf5de56211157` |

The Rust observation sites are retained in
[`runyte-milestones.patch`](../../benchmarks/runyte-milestones.patch) and
[`helix-milestones.patch`](../../benchmarks/helix-milestones.patch).

| Editor | Stock executable SHA-256 | Probe executable SHA-256 |
| --- | --- | --- |
| neovim | `a13c5dd869d4219852604bfcbe6b9895b78fc03ce162ad45da23c1d280f4b411` | same executable with Lua probe |
| helix | `3f31b5db36dec738e153fc027edb280063616df8823598df2da56c80c71542e5` | `3eb3aea66edc6e4ab09e86315beb8f92c16a9c884956d7ffac70f382000c9ddd` |
| runyte | `c3090c35dd7a81507424ca191876e6e5149bba9b059dd087b25a047e2121bba8` | `49af48cf70b3a8f4ae571e1d4d10ab7fdd080060fb5880ecaeac9375b528a364` |


## 2026-08-31

Startup and quit harness `18a6bb9`; idle harness `e44e2cf`. Startup and quit are
medians of 10 runs, 120x40 pty, isolated home and XDG storage. Idle is the median
and range of five independent ten-second windows, each with a fresh process.
The Runyte release build contains the first-frame and idle changes recorded by
the resolved `first_frame_and_benchmark_breadth` issue.

- neovim: `NVIM v0.12.4`
- helix: `helix 25.07.1 (a05c151b)`
- runyte: `runyte 0.1.6`

Machine: AMD Ryzen AI 9 365, 20 threads, 27 GB, Linux 7.1, btrfs.

Parser availability was rechecked inside the isolated environment. Neovim
reported an active tree-sitter highlighter and `filetype=lua`; Helix reported
its Lua parser and highlight queries present; Runyte contains the statically
linked Lua grammar and queries.

### Startup: first document content emitted

The comparative value is the time from immediately before process launch until
a shared token from the first document line is emitted in the raw terminal
stream. It is a common output event, not proof that the terminal has presented
the whole screen, input is accepted, highlighting is complete, or background
work has finished. A sample counts only if the process subsequently reaches
terminal-output quiet.

| Fixture | Size | neovim | helix | runyte |
| --- | --- | ---: | ---: | ---: |
| `short.txt` | 17 kB | 19 ms | 17 ms | 6 ms |
| `medium.txt` | 171 kB | 21 ms | 21 ms | 8 ms |
| `long.txt` | 1.7 MB | 23 ms | 21 ms | 15 ms |
| `short.lua` | 17 kB | 33 ms | 27 ms | 14 ms |
| `medium.lua` | 171 kB | 27 ms | 48 ms | 28 ms |
| `long.lua` | 1.7 MB | 28 ms | 215 ms | 176 ms |

#### First terminal byte (diagnostic only)

The first byte may be an invisible capability query, terminal setup, a loading
presentation, or document drawing. It is not a readiness metric and is not used
for the comparison above.

| Fixture | Size | neovim | helix | runyte |
| --- | --- | ---: | ---: | ---: |
| `short.txt` | 17 kB | 6 ms | 16 ms | 5 ms |
| `medium.txt` | 171 kB | 7 ms | 20 ms | 5 ms |
| `long.txt` | 1.7 MB | 7 ms | 21 ms | 5 ms |
| `short.lua` | 17 kB | 7 ms | 26 ms | 5 ms |
| `medium.lua` | 171 kB | 7 ms | 47 ms | 4 ms |
| `long.lua` | 1.7 MB | 7 ms | 214 ms | 4 ms |

### Quit

Time from the final force-quit keystroke until the process exits. The harness's
staggered-key delay is excluded.

| Fixture | Size | neovim | helix | runyte |
| --- | --- | ---: | ---: | ---: |
| `short.txt` | 17 kB | 2 ms | 4 ms | 4 ms |
| `medium.txt` | 171 kB | 3 ms | 4 ms | 4 ms |
| `long.txt` | 1.7 MB | 3 ms | 4 ms | 5 ms |
| `short.lua` | 17 kB | 2 ms | 4 ms | 5 ms |
| `medium.lua` | 171 kB | 2 ms | 7 ms | 8 ms |
| `long.lua` | 1.7 MB | 6 ms | 22 ms | 28 ms |

### Idle cost, `medium.lua` open in a Git repository

| Editor | Idle CPU median (range) | Screen writes median (range) |
| --- | ---: | ---: |
| neovim | 0.00 % (0.00–0.00) | 0 (0–0) |
| helix | 0.00 % (0.00–0.00) | 0 (0–0) |
| runyte | 0.00 % (0.00–0.10) | 0 (0–0) |

### Interpretation

Runyte's raw first-byte diagnostic is 4–5 ms, but the harness cannot identify
whether that byte is terminal setup or part of `Opening workspace…`. Runyte
separately draws that stable startup presentation, then replaces it with one
complete highlighted editor frame. No document text is exposed in an
unhighlighted or reflowing intermediate state.

Runyte emitted the shared document token first on all three plain-text fixtures
and `short.lua`. Neovim emitted it 1 ms before Runyte on `medium.lua`, which is
within ordinary run variation, and much earlier on `long.lua`: 28 ms versus 176
ms. That is an output-order result, not evidence that Neovim completed the Lua
parse at 28 ms. The older quiet heuristic observed its later drawing at 175 ms,
while Runyte deliberately withholds document text until its highlighted frame
is complete. No result here supports a claim that one editor is globally
fastest or that the loading presentation improved Runyte's parsing speed.

The opt-in startup trace measured one `long.lua` run as 1.9 ms to terminal
entry, 10.7 ms to the opened buffer, 192.0 ms to completed syntax, and 194.4 ms
to the editor frame. The trace now also distinguishes the startup presentation
from that editor frame. Its buffer-open and syntax milestones surround the
operations they name; previously both were recorded only after the combined
operation returned.

Quit is deliberately retained even where Runyte does not lead. Neovim exits
first in all six rows. This category measures
orderly process teardown after the same unchanged document, not a save or
persistence workflow.

Runyte's idle median is 0.00% with zero writes in every window. One window
rounded to 0.10%. The remaining scheduled work is bounded and named: a
one-second host maintenance wake checks Git fallback/retry, monitor
registration, session-list activity and logging health, while the file monitor
keeps a two-second metadata reconciliation for lost native filesystem events.
Signal delivery and monitor deadlines themselves are event-driven rather than
25 ms polling loops. A termination signal received during synchronous startup
restores the saved terminal state and exits directly, so a blocked file open
cannot strand the terminal in raw mode while waiting for the event loop.

## 2026-08-29

Harness `41cb0bf`. Median of 10 runs, 120x40 pty, empty config. The benchmark was
run outside a filesystem sandbox so each editor could use its ordinary writable
cache, state and local-socket paths; the empty configuration still excluded
personal settings and plugins.

- neovim: `NVIM v0.12.4`
- helix: `helix 25.07.1 (a05c151b)`
- runyte: `runyte 0.1.4`, release build from `41cb0bf`

Machine: AMD Ryzen AI 9 365, 20 threads, 27 GB, Linux 7.1, btrfs.

### Startup

This historical table recorded first terminal byte / drawing quiet under the
then-current heuristic. The first-byte values are diagnostic and must not be
used to rank editors; the labels are retained to preserve the original result.

| Fixture | Size | neovim first / ready | helix first / ready | runyte first / ready |
| --- | --- | ---: | ---: | ---: |
| `short.txt` | 17 kB | 6 / 18 ms | 17 / 18 ms | 5 / 6 ms |
| `medium.txt` | 171 kB | 6 / 17 ms | 19 / 20 ms | 6 / 7 ms |
| `long.txt` | 1.7 MB | 6 / 22 ms | 22 / 23 ms | 16 / 17 ms |
| `short.lua` | 17 kB | 6 / 30 ms | 22 / 23 ms | 10 / 12 ms |
| `medium.lua` | 171 kB | 6 / 46 ms | 48 / 50 ms | 28 / 29 ms |
| `long.lua` | 1.7 MB | 6 / 175 ms | 214 / 215 ms | 150 / 152 ms |

### Idle cost, `medium.lua` open in a Git repository, 10 s

| Editor | Idle CPU | Screen writes |
| --- | ---: | ---: |
| neovim | 0.00 % | 0 |
| helix | 0.00 % | 0 |
| runyte | 0.30 % | 0 |

### Interpretation

Runyte reached drawing quiet first in every row. On documents without a
language, its settled time grew from 6 ms at 17 kB to 17 ms at 1.7 MB. The other
editors reached 18–23 ms across the same `.txt` range.

At the time, subtracting each `.txt` settled time from its same-size `.lua` time
was reported as language cost:

| Size | neovim | helix | runyte |
| --- | ---: | ---: | ---: |
| short | 12 ms | 5 ms | 6 ms |
| medium | 29 ms | 30 ms | 22 ms |
| long | 153 ms | 192 ms | 135 ms |

At 1.7 MB, Runyte reached drawing quiet at 152 ms, against 175 ms for Neovim and
215 ms for Helix. These differences remain historical observations of the old
quiet heuristic. They do not prove parser completion or isolate full parser
cost, which is why the current harness no longer labels them readiness or
language cost.

All three editors produced zero screen writes during the unchanged ten-second
idle window. Runyte's single idle sample used 0.30% CPU; because idle is one
window rather than a median, that point estimate should be compared with a
repeated later run before treating a small difference as a regression.
