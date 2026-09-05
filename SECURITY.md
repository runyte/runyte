# Security policy

## Reporting a vulnerability

Report privately through GitHub, from
[the Security tab](https://github.com/runyte/runyte/security/advisories/new)
of this repository. That opens a thread only the maintainer can read, keeps
attachments and reproduction steps out of public view, and becomes the
published advisory once a fix ships.

Please do not open a public issue, a pull request, or a discussion for a
suspected vulnerability. A public report is the disclosure.

A useful report describes what an attacker controls, what they gain, and how
to reproduce it. A failing test, a repository or file that triggers the
behavior, or a recording is worth more than a description of the code path.

## Supported versions

Runyte is before 1.0 and is maintained by one person. Only the most recent
release receives fixes; there are no backports to earlier versions. Report
against the latest release or against `main`.

## What to expect

Acknowledgement within seven days, and an assessment of severity and likely
timeline within thirty. No patch deadline is promised: some fixes are small,
and some wait on a design decision that a narrow patch would only paper over.
The advisory thread stays open and is updated either way.

Disclosure is coordinated. The advisory is published when a fix is released,
or earlier if the issue is already public or is being exploited. Reporters are
credited in the advisory unless they ask not to be.

## Scope

Runyte is a local terminal editor. The boundary that matters is what content
belonging to someone else can do to the person who opens it. In scope:

- A file, directory, or Git repository whose contents lead to code execution,
  to reads or writes outside the workspace, or to data leaving the machine,
  when it is opened, previewed, or listed.
- Escape sequences, from a file being viewed or from a terminal session's
  child process, that escape the emulator's own state and reach the host
  terminal or the editor.
- Filesystem plans that apply outside the set of paths that was confirmed, or
  that follow a path to a target the confirmation did not name.
- Arguments reaching `git` as options rather than as operands, or any path by
  which repository content chooses what is executed.
- The persistent session host: another local account attaching to a session,
  reading its buffers, or reaching its transport and lock files.
- Escapes from the workspace cache used for pasted images.

Out of scope:

- Anything that presupposes an attacker who can already execute code as the
  user. They can edit the configuration, the keymap, and the binary itself;
  Runyte does not defend against its own operator.
- Behavior the user configured, including commands bound to keys and language
  servers named in a configuration file.
- Resource exhaustion from pathological but honest input, such as a file large
  enough to make the editor slow. That is a bug worth an issue, not an
  advisory, unless a small input causes disproportionate cost.
- Vulnerabilities in dependencies with no demonstrated path through Runyte.
  Report those upstream; mentioning them here as well is welcome.

Some weaknesses in scope are already known and recorded as limitations. Report
one anyway rather than assuming it has been seen: the reply will say so, and a
concrete reproduction can change how a known problem is prioritized.
