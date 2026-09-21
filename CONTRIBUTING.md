# Contributing to Runyte

Feature requests, bug reports, and plugin development are welcome.

Suspected security vulnerabilities do not belong in any of the channels below.
Follow the [security policy](SECURITY.md) instead.

## Request a feature

Open an issue on [GitHub Issues](https://github.com/runyte/runyte/issues).
Describe the workflow you want to improve, what is difficult today, and how
the proposed feature would help. Include a concrete example and check for an
existing request before opening a new one.

## Report a bug

Open an issue on [GitHub Issues](https://github.com/runyte/runyte/issues).
Check the latest release or `main` first; only the most recent release receives
fixes.

A report is most useful when it includes:

- The output of `runyte --version`, the operating system, and the terminal
  emulator, including any multiplexer such as tmux.
- Whether the workspace was standalone or opened with `--persistent`.
- The exact keys pressed, what happened, and what you expected instead.
- The smallest file, configuration, or project that reproduces it.
- Relevant lines from the diagnostic log (`:log-open`) or from
  `:service-health` when a language server, Git, the clipboard, or a plugin is
  involved.

Logs and screenshots can contain file paths, project names, and buffer text.
Review them before attaching, and redact anything you would not publish.

Behavior that differs from Helix is not necessarily a bug. The
[keymap register](context/reference/helix-keymap-v1.md) records the deliberate
differences; a report that one of them is a poor choice is still welcome.

## Write a plugin

Plugins can be written in any language. Each one runs as an external process
and talks to Runyte over newline-delimited JSON, against a plugin contract that
is stable from Runyte 0.3.0.

- Start with the [installation and protocol guide](docs/plugins.md) and the
  [application authoring guide](docs/plugins/authoring.md).
- Build on the [examples](docs/plugins/todo/README.md), or on
  [ru-time](https://github.com/runyte/ru-time) as a complete plugin.
- Run the [conformance checks](docs/plugins/conformance.md) before
  distributing it.

Publish a plugin from its own repository, under a license of your choice, and
share it on [r/runyte](https://www.reddit.com/r/runyte/). If the contract is
missing something a plugin needs, open an issue describing the plugin and the
gap rather than working around it.

## Code changes

Read [`AGENTS.md`](AGENTS.md) before changing code. It describes the
architecture, where each responsibility lives, and the conventions every change
follows. A pull request should:

- Stay focused on one behavior, with tests at the boundary it changes.
- Keep line coverage at or above the floor in
  [`context/reference/test-coverage.md`](context/reference/test-coverage.md).
- Pass `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo test`.
- Update [`docs/user-guide.md`](docs/user-guide.md) when user-visible behavior
  changes.

Runyte is licensed under the [Mozilla Public License 2.0](LICENSE), and
contributions are accepted under the same license. Material taken from another
project needs a compatible license and an entry in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
