---
title: "Subprocess tests load personal Runyte configuration"
status: resolved
reported: 2026-09-17
resolved: 2026-09-17
commit: 69eb95c
---

## Resolution

Commit `69eb95c` (`Isolate subprocess tests from personal configuration`) fixes
child-process environment construction in the test fixtures. The diagnostic-log
helper isolated runtime and cache directories but left `XDG_CONFIG_HOME`
inherited, allowing personal plugin settings to alter host startup and shutdown.
It now supplies a fixture-owned configuration directory. Explicit `--config`
arguments still select the test's authored configuration.

The audit found the same omission in bulk-workspace CLI/host builders, a local
protocol waiter, the release-packaging cwd-file probe and the standalone terminal
parent-navigation/editor child. Each now sets a temporary configuration root.
Bulk host startup reuses the CLI builder so its isolation cannot drift separately.
Already-isolated fixtures and configuration-free help probes need no override.
The repository convention now explicitly requires configuration isolation for
subprocess tests; no process-wide environment mutation or personal-config edit
is used.

Two regression tests launch selected existing fixture tests in child runners
whose inherited configuration is deliberately invalid. They demonstrate that
fixture defaults override that environment, while an explicit invalid `--config`
still produces the expected YAML error. The original log-rotation test now passes
under the normal environment without an outer configuration workaround.

Validation: With both follow-up fixes present, `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings` and `cargo test --locked`
passed (3,654 passed, 34 ignored). The canonical
`cargo llvm-cov --locked --workspace --summary-only --fail-under-lines 89`
run also passed and measured 91.83% line coverage on
`x86_64-unknown-linux-gnu`, above the unchanged 89% floor. Process/socket
checks ran outside the sandbox without an outer `XDG_CONFIG_HOME` override.
Native macOS coverage remains a CI check.

Regression coverage:

- `diagnostic_fixture_ignores_parent_config_and_honors_explicit_config` and
  `rotation_bounds_the_host_log_across_a_restart` in `tests/diagnostic_log.rs`.
- `bulk_fixtures_ignore_parent_configuration_for_hosts_and_cli` and
  `stop_all_then_clean_manages_the_complete_workspace_inventory` in
  `tests/workspace_bulk.rs`.
- `standalone_terminal_parent_requests_refuse_without_nesting_and_return_to_shell`
  in `tests/terminal.rs`, plus the existing local-protocol and release-packaging
  suites, exercise the other corrected child launch paths.

## Report

`tests/diagnostic_log.rs::rotation_bounds_the_host_log_across_a_restart` can fail
its expectation of `HostResponse::ShuttingDown` when the developer's default
configuration includes the `dbviewer` plugin. With the source tree unchanged,
the reported comparison was:

| Configuration | Result |
| --- | --- |
| Default configuration containing `dbviewer` | Failed |
| The same configuration without `dbviewer` | Passed |
| Empty temporary `XDG_CONFIG_HOME` | Passed |

The fixture's `bundled_runyte` helper isolates runtime and cache storage but
does not override `XDG_CONFIG_HOME`. Its subprocess consequently loads personal
settings and may start configured plugins. This makes results depend on the
developer's environment and violates the repository's test-isolation boundary.
The previous plugin-input fix needed a temporary empty configuration for its
coverage run; that workaround does not make the subprocess fixtures isolated.

Expected behavior is for every test-launched editor or persistent-session host
that loads configuration to use a temporary fixture-owned configuration root.
Tests that exercise particular settings must pass their own explicit config.
Parent process environment must not be mutated because tests run concurrently.
Audit the other direct CLI, host and editor-child launch paths for the same gap.

Regression coverage should run the fixture under an intentionally invalid or
plugin-bearing temporary parent configuration and prove that the editor still
uses fixture defaults. It should also show that explicit fixture configurations
continue to take effect. No regression should read or modify personal settings,
start personal plugins, or write runtime state into tracked repository context.
