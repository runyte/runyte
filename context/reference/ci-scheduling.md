<!-- SPDX-License-Identifier: MPL-2.0 -->
# CI scheduling

| Event | Checks |
| --- | --- |
| PR targeting main, dev or exp | Full CI and Desktop edition |
| Push to main | Full CI and Desktop edition on the exact pushed commit |
| Push to dev or exp | Development checks: Linux formatting, Clippy and ordinary tests; ordinary macOS tests |
| Nightly | Full CI on dev at 02:17 UTC; Desktop edition on dev at 02:47 UTC |
| Manual CI or Desktop edition dispatch | Full selected workflow on the selected ref |
| Manual Development checks dispatch | Slim checks on the selected ref, with development-only check names |

The legacy `feat/windows-support` push still runs full CI. Full checks retain
Windows acceptance, both 10× lifecycle stress jobs, performance budgets, plugin
contracts and frozen compatibility, installers, release/MSRV builds, desktop
packaging and GUI acceptance. Main push checks are required by the release
procedure; PR results cannot replace them.

Development check names differ from the required PR check names. A successful
slim run must not stand in for a full merge check. Both ordinary and instrumented
core tests select the root `runyte` package; desktop tests remain separate.
On full runs, Linux and macOS coverage execute the core tests once inside the
existing required Gates and Tests jobs, retaining the canonical measurement,
89% floor, summary and HTML reports. Coverage and ordinary-test caches have
separate keys; LLVM instrumentation remains in cargo-llvm-cov's own target tree.

Scheduled workflows are loaded from the default branch by GitHub. Each resolves
dev to one SHA before its jobs begin, and every repository checkout in that run
uses that SHA for the source under test. Jobs that use repository CI helpers
separately check out `.github` from `github.workflow_sha` into an ignored
`.ci-tools` directory. This lets the current workflow test dev even before dev
adopts its new helper files, without rewriting the tested source or its own
packaging tests. Direct dev/exp pushes switch to slim checks when those branches
adopt the workflow changes.

Nightly check names have a `Nightly` prefix: GitHub associates
scheduled runs with the default-branch SHA, and a dev result must not substitute
for a required check on main. The two nightly workflows can resolve different dev commits if
dev advances between them. Their concurrency groups are separate from main push
runs, so a nightly run cannot cancel validation of a release commit. Schedules
become active only after these workflow definitions reach the default branch.

Plugin conformance builds and tests an uninstrumented, default-feature debug
host once per OS. The frozen-plugin and advisory ru-time jobs consume only that
executable from the same workflow run and OS artifact, after conformance passes.
The manifest verifies source revision, version, OS/architecture, build kind and
SHA-256 before installing the binary. No target directory, instrumented object,
release binary or cross-run artifact is shared. Artifacts expire after one day;
a later rerun of consumers alone may require rerunning conformance to regenerate
them. Producer reruns replace the same-run artifact. Consumers keep independent
plugin checkouts and assertions, and do not restore Rust compilation caches.

This dependency removes six redundant host builds but makes consumers wait for
both conformance matrix entries. Assess elapsed time together with queue time
and runner minutes: fewer builds do not necessarily shorten a run whose critical
path is Native Windows. Windows build/test concurrency remains unchanged pending
measurements of memory, process limits and fixture isolation.
