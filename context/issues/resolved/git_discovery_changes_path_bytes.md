---
title: "Git repository discovery changes valid pathname bytes"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: cb59e84
---

## Resolution

Commit `cb59e84` (`fix(git): preserve repository discovery path bytes`).

`GitCliProvider::discover_with_marker_probe` now removes exactly one output
newline and decodes the remaining path with the platform's filesystem-path
representation. The former UTF-8 conversion and `trim_end` removed meaningful
trailing pathname characters, could address a different directory, and refused
valid non-UTF-8 Unix roots. Required empty responses still fail. Other Git
text parsing is unaffected.

`discovery_preserves_trailing_whitespace_and_non_utf8_path_bytes` in
`tests/git_provider.rs` uses real temporary repositories whose roots end with
a space, tab, newline or non-UTF-8 byte. It checks working, Git and common
metadata directories. The test failed before the change; all four discovery
tests passed after it. The parent reviewer checked all three discovery path
responses and the existing lossless path decoder.

Known limitation: this session validated the regression on Linux. Native
Windows and macOS checks remain necessary in their normal CI environments.

## Report

Repository discovery changes valid filesystem paths before using them.

`GitCliProvider::discover_with_marker_probe` decodes each `git rev-parse`
path response as UTF-8 and calls `trim_end`. A repository whose root ends in
a space, tab, or newline is consequently addressed under a different path
when discovery asks for its Git directories. On Unix, a root containing
non-UTF-8 bytes is refused even though Git and the editor can address it.

Initialize a temporary repository, rename its directory to a name ending in
a space, and call `discover` on that directory. Discovery fails when the
shortened directory is absent and can inspect the wrong repository if the
shortened directory exists. The same mismatch applies to trailing whitespace
in an explicitly separate Git directory.

Discovery should remove only Git's one output newline and decode filesystem
paths with the platform's lossless path representation. Empty required path
responses must still fail. Other textual Git output retains its existing
parsing rules.
