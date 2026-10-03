---
title: "Configuration reads can hang on special files"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: e4842fb
---

## Resolution

Commit `e4842fb` (`fix(config): reject special files before reading settings`).

`Config::read_source` now uses the descriptor-checked regular-file opener
shared with document loading. Configuration load/reload and both settings
persistence paths use it before reading. Unix opens are nonblocking before
checking the descriptor type, so a FIFO without a writer cannot stall startup
or the editor thread. Symlinks to ordinary files and missing configuration
behavior remain supported; errors leave the active configuration unchanged.

`special_file_config_reads_refuse_without_blocking` and its bounded subprocess
fixture `special_file_config_fixture` in `src/config/tests/special_files.rs`
passed. Coverage exercises actual config-reload dispatch, load/reload, setting
and indentation writes, FIFO aliases, device refusal, missing files and regular
symlinks. The parent reviewer checked all read call sites and failure ordering.

Known limitation: this rejects special filesystem objects; it does not make
regular-file access asynchronous or bound latency on a stalled filesystem.

## Report

Configuration reads use blocking file opens without checking the opened
object's type. On Unix, a configuration path replaced with a FIFO that has no
writer makes `:config-reload` wait indefinitely on the editor thread. The same
problem affects startup through `--config`, saving a setting such as a theme,
and saving an indentation override. A symlink to a FIFO behaves identically.
An unbounded character device can instead keep the read running indefinitely.

Load a regular configuration, replace that file externally with a named pipe,
and run `:config-reload` or save a setting to reproduce the stall. The editor
should reject the special file promptly and leave its working configuration
and the filesystem object intact. Existing regular-file symlinks and absent
configuration files must retain their current behavior.

Use the existing descriptor-checked regular-file opener for each configuration
read path. Regression tests must run potentially blocking operations in a
bounded subprocess so a regression cannot hang the entire test runner.
