# Contributing to Runyte

Runyte welcomes contributions from people who share its direction: a fast,
selection-first editor with consistent keybindings and closely integrated
tools. Bug fixes, performance improvements, tests, documentation, and
thoughtfully scoped features all help.

You do not need to start with a large feature. A focused improvement to
something you use is a good first contribution.

Suspected security vulnerabilities should follow the
[security policy](SECURITY.md).

## Before you start

Small fixes, documentation improvements, and focused performance or test
improvements can go straight to a pull request. An issue is not required
for every change.

For substantial features, architecture changes, or changes to editing
behavior and keybindings, open an issue first. Describe the workflow,
give a concrete example, and explain your proposed approach. This lets us
agree on scope and direction before you spend time implementing it.

Search existing issues and pull requests before starting. If you pick up an
existing issue, leave a comment describing your approach so others know
someone is working on it.

Runyte's selection-first model, keymap, and module boundaries are deliberate.
Contributions should build on those choices and keep the editor fast and
consistent. Integrations with separate services or applications are often
best implemented as plugins.

We review changes for both correctness and fit with the project. Discussing
larger changes early helps avoid work that we cannot accept.

## Request a feature

Open an issue on [GitHub Issues](https://github.com/runyte/runyte/issues).
Describe the workflow you want to improve, what is difficult today, and how
the proposed feature would help. Include a concrete example and check for an
existing request before opening a new one.

Runyte's selection-first model and its keymap are deliberate. A request that
works with them is easier to accept than one that adds a second way to do
something. For integrations with separate services or applications, consider
whether a plugin would fit the workflow.

## Report a bug

Open an issue on [GitHub Issues](https://github.com/runyte/runyte/issues).
Check the latest release or `main` first. Only the most recent release gets
fixes.

A report is most useful when it includes:

- The output of `runyte --version`, the operating system, and the terminal
  emulator, including any multiplexer such as tmux.
- Which mode Runyte ran in: `editor`, `ide`, or `mux`, as the status line
  shows.
- The exact keys pressed, what happened, and what you expected instead.
- The smallest file, configuration, or project that reproduces it.
- Relevant lines from the diagnostic log (`:log-open`) or from
  `:service-health` when a language server, Git, the clipboard, or a plugin is
  involved.

Logs and screenshots can contain file paths, project names, and buffer text.
Review them before attaching, and redact anything you would not publish.

Behavior that differs from Helix is not necessarily a bug. The [keymap
register](context/reference/helix-keymap-v1.md) records the deliberate
differences. If you think one of them is a poor choice, please tell us.

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
missing something a plugin needs, open an issue describing the plugin and what
is missing.

## Submit a pull request

Read [`AGENTS.md`](AGENTS.md) before changing code. Despite its name, it is
the development guide for everyone: it describes the architecture,
conventions, and required checks.

Fork the repository, create a branch from `main`, and keep your pull request
focused on one improvement. Draft pull requests are welcome when you want
feedback before finishing.

A pull request should:

- Explain the problem, what changes for the user, and why this approach fits.
  Link the relevant issue when one exists.
- Include regression tests for fixes and appropriate coverage for new behavior.
  Keep coverage at or above the
  [documented floor](context/reference/test-coverage.md).
- Update the relevant [user-guide chapters](docs/user-guide/README.md) when
  user-visible behavior changes.
- Describe how you validated the change and any remaining limitations.
  For performance improvements, include before-and-after measurements and
  enough detail to reproduce them.
- Pass `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  and `cargo test`. Run the additional desktop checks in `AGENTS.md` when
  the change can affect the desktop edition.

GitHub is the primary issue tracker. A fix does not require a duplicate
issue or resolution record under `context/`.

Runyte is licensed under the [Mozilla Public License 2.0](LICENSE), and
contributions are accepted under the same license. Material taken from another
project needs a compatible license and an entry in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
