## Diagnostics and logging

Four surfaces answer four different questions:

| Surface | Answers |
| --- | --- |
| Interaction line | What did the last command do? |
| `:notifications` | What happened in this workspace? (bounded, in memory) |
| `:service-health` | What state are optional services in right now? |
| **Diagnostic log** | What went wrong, even after Runyte is gone? |

The diagnostic log is a small local file that outlives the process. It is not
a second notification system and not an audit trail: an actionable failure
still reaches you through the ordinary surfaces whether or not a record was
written.

### Reading the log

`:log-open` opens the log of the process that owns this workspace as a
read-only `[log]` buffer, searchable, splittable, and scrollable as usual.

- In mux mode that is the host's `host.log`. No client-side trace is
  opened or merged.
- `:service-health` names the owner role, active level, resolved path, and any
  logger failure.

Each record is one line: an RFC 3339 timestamp, the level, the owning role and
PID, the abbreviated workspace ID, the subsystem, the message, and any
structured `key=value` context.

```
2026-08-27T12:34:56.789+02:00 WARN  host[8123] ws=a1b2c3d4 lsp: language server stopped: exited with 1 language=rust
```

### Levels and startup controls

| Flags | Level |
| --- | --- |
| *(none)* | warning |
| `-v` | info |
| `-vv` | debug |
| `-vvv` and beyond | trace |

The default records warnings and errors, so a first unexpected failure does
not need reproducing, while routine operation writes no high-volume trace.

**`--log PATH`** chooses an explicit file.

- Failing to use it is a startup error, because silently writing elsewhere
  would make the capture misleading.
- A path already owned by another running Runyte process is refused after a
  two-second handover window. Choose another path, or let the first process
  exit.

**An unwritable default file** only degrades logging: editing continues, a
persistent host still serves, and the failure appears on stderr, as a
notification, and in `:service-health`. If a file stops accepting writes after
startup, editing continues and one warning is kept in `:notifications`, with
the standing failure in `:service-health`.

**Persistent hosts** fix verbosity and destination at host startup.

- `--serve`, `--session-restart`, and the launch that creates a missing host
  pass them on.
- Attaching to a running host does not change its logger. `-v` or `--log`
  there reports that the session kept its own configuration.
- Restart to change it:

```sh
runyte --session-restart -vv     # the only way to change a running host's logging
```

A normal restart refuses unsaved work: save first, or decide explicitly on
`--force`. There is no runtime log-level command and no protocol message for
logging.

### Who owns the log

Log ownership follows editor-state ownership:

| Mode | Owner | File |
| --- | --- | --- |
| ide | the TUI process | `.runyte/standalone-<pid>.log` |
| mux | the host process | `.runyte/host.log` |
| editor | the TUI process, only with `--log PATH` | that path |

- An ide-mode name includes the process ID because several ide-mode editors
  may open one workspace; two can never write or rotate the same file.
- A host has one name because exactly one host serves a workspace, and it
  keeps logging while no TUI is attached.
- A client never appends to a host's log or forwards records over the local
  protocol: transport diagnostics must not depend on the transport working.
  The host records what it sees of clients — attachment, detachment, refusal,
  disconnection, and rejected frames.
- Failures confined to a client's own input, rendering, or terminal setup are
  printed to stderr after the terminal is restored, and are not in the host
  log.
- Logs live under the runtime state boundary, normally `.runyte/`, and never
  under a Git-tracked directory.

**File safety:**

- Linux and macOS create logs owner-only (`0600`). Windows uses a protected
  ACL granting access to the owning user.
- Private storage needs local NTFS on Windows; network volumes, reparse
  points, and hardlinked files are refused. Other filesystem types are
  unsupported.
- Writes reject symlinked parent directories, symlinked targets, hard-linked
  files, and special files such as FIFOs.
- Rotation uses the opened file and directory even if their names change.
- A refused default log is reported without blocking editing. An unusable
  explicit `--log` fails startup.

### What is recorded, and what never is

**Recorded:**

- process startup, version, role, workspace identity, and orderly shutdown;
- session publication, idle retirement, signal termination, and forced
  termination;
- client attachment, detachment, refusal, closure, a client that stops
  reading, and malformed or truncated frames;
- language servers becoming ready, restarting, and stopping;
- terminal sessions that fail to start, and children that exit;
- Git failures already converted into typed results;
- background services whose channel closes;
- panics, with the thread, location, message, and a backtrace when enabled.

**Not yet recorded,** though surrounding events are: file-watcher lifecycle
beyond its channel closing, and a host restart as distinct from a start
followed by a stop.

**Never recorded:** routine keystrokes, rendered frames, successful commands,
buffer edits, and complete language-server request or response bodies. No
level ever records buffer text, selections, clipboard contents, typed or
pasted text, terminal contents, credentials, environment-variable values,
unrestricted subprocess output, or full LSP JSON.

**Deliberately thinner records:**

- A failed Git operation records the refusal and exit status, but not the
  argument vector or Git's stderr, because a failing commit's arguments hold
  the message just typed.
- A stopped language server records the language, but not the composed
  reason, because a server closed by its own process carries its stderr tail
  there.
- Both are available in full through `:notifications`, `:lsp-status`, and the
  interaction line.

Records **do** contain local paths and process metadata, because those
identify a failing local operation. Review a log before sharing it.

### Bounds

- The active file keeps at most 4 MiB, with one previous 4 MiB file beside it
  ending in `.1`.
- The owning process rotates, both while running and at startup when it
  inherits a full file, so a long-lived or often-restarted host cannot grow
  without bound.
- Before opening its own default log, an ide-mode process keeps the four
  newest logs of exited ide-mode processes and removes older ones with their
  `.1` files. Logs of live processes are never pruned.
- Producers never wait for disk. Records go through a bounded queue to one
  background writer and are dropped rather than delaying input, rendering,
  protocol handling, or service events; a later record says how many were
  lost.
- Shutdown and panic paths flush within a bounded budget and never wait
  indefinitely.

<!-- navigation -->
[Contents](README.md) · [Previous: Agent access to workspace context](agent-access-to-workspace-context.md) · [Next: Configuration](configuration.md)
