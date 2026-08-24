---
title: "--cwd-file is an exposed implementation detail"
status: resolved
reported: 2026-08-18
resolved: 2026-08-18
legacy_commit: 99b0e95
---

## Resolution

Commit `99b0e95` (`Stop documenting --cwd-file as an option anyone should
pass by hand`) removed the `--cwd-file PATH` line from the `OPTIONS:` block
in `print_help` (`src/main.rs`). The flag's parsing in `src/launch.rs` and
its handling throughout `src/main.rs` are unchanged: the documented
`runyte()` shell function in `README.md` still passes it on every
invocation, and `--workspace-list`, `--workspace-stop`, and the other
workspace-management modes still accept and ignore it so the same wrapper
can invoke them.

Deleting the `OPTIONS:` line only satisfied half of the report: the flag
was gone, but nothing in `--help` said `:quit-here` existed or how to turn
it on. `print_help` now carries a short paragraph past the `TARGETS:`
section, in the register of the existing "Inside the editor press Space+?
for the complete key reference." trailing line: ":quit-here moves the shell
to the editor's directory on exit; it requires the runyte() shell function
documented in README.md." It names the feature and the shell function
without naming the flag, leaving `OPTIONS:` and `WORKSPACES:` themselves
untouched.

`App::request_quit_here` in `src/app.rs` refused with `":qh requires the
runyte() shell wrapper (--cwd-file); see README.md"`, naming the flag it
happens to use rather than the integration that is actually missing. It now
reads `":qh requires the runyte() shell function from README.md"`, naming
the shell function and where to find it instead.

`README.md` keeps documenting `--cwd-file` in the "Change the shell
directory on exit" section, since the shell function it prints there
contains it. The one sentence claiming an explicit `--cwd-file` was an
alternative to the wrapper — "Without the wrapper or an explicit
`--cwd-file`, `:quit-here` refuses to exit..." — was cut down to "Without
the wrapper, `:quit-here` refuses to exit...", since that alternative is no
longer part of the documented surface.

Tests:

- `editor_help_does_not_document_the_shell_cwd_handoff_option` in
  `tests/release_packaging.rs` asserts `runyte --help` no longer contains
  `--cwd-file`, and that it still mentions `:quit-here`, `runyte()`, and
  `README.md`.
- `cwd_file_option_still_works_though_undocumented` in
  `tests/release_packaging.rs` invokes `runyte --cwd-file PATH
  --workspace-list` in an isolated `XDG_RUNTIME_DIR`/`XDG_CACHE_HOME` and
  asserts it still succeeds, confirming the flag remains fully functional
  while hidden.
- `cwd_file_option_preserves_its_path` in `src/main.rs` (pre-existing)
  continues to cover that `LaunchArguments` still parses both the
  space-separated and `=`-joined spellings.
- `quit_here_refuses_to_degrade_to_plain_quit_without_a_shell_handoff` in
  `src/app.rs` was updated to assert the refusal names `runyte()` and
  `README.md` and no longer mentions `--cwd-file`.
- `quit_here_reports_its_directory_to_a_handoff_capable_client` in
  `tests/persistent_host.rs` was updated to match the new wording in the
  refusal it exercises over the local protocol.

## Report

`--cwd-file PATH` was listed in `runyte --help` under `OPTIONS:`, described
as "Write the directory selected by :quit-here for a shell wrapper". It was
not an option anyone should pass by hand. It exists so that the documented
Bash and Zsh `runyte()` function can learn where `:quit-here` decided the
shell should go, and passing it manually gives a temporary file that
nothing reads.

The user-facing feature is `:quit-here` (`:qh`), and the choice a user
makes is between `:quit`, which leaves the shell where it was, and
`:quit-here`, which moves it. Whether that choice is available at all
depends on one thing: whether the shell function from `README.md` has been
added to the shell configuration. That is what `--help` and the editor
should talk about.

So `--cwd-file` should stop appearing in `runyte --help`. It stays a
working option, because the shell function passes it, but it becomes an
internal detail of the shell integration rather than part of the documented
surface.

`:quit-here` already refuses to run without the handoff, but its refusal
named the flag:

```
:qh requires the runyte() shell wrapper (--cwd-file); see README.md
```

With the flag hidden, that message should name the shell function and
where to find it, and not the flag it happens to use.

`README.md` keeps documenting `--cwd-file`, since the shell function it
prints contains it and someone reading that function should be able to
find out what the option does. The sentence saying the option can also be
passed explicitly is the part that stops being true in spirit.
