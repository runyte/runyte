## Diagnostics and logging

There are four places to look, and each answers a different question:

| Surface | Answers |
| --- | --- |
| Interaction line | What did the last command do? |
| `:notifications` | What happened in this workspace? (bounded, in memory) |
| `:service-health` | What state are optional services in right now? |
| **Diagnostic log** | What went wrong, even after Runyte is gone? |

The diagnostic log is a small local file that stays after Runyte exits. It is
not a second notification system and not an audit trail. A failure you can act
on still shows up in the usual places, even if nothing was written to the log.

### Reading the log

`:log-open` opens the log as a read-only `[log]` buffer. You can search, split
and scroll it as usual. It shows the log of the process that owns this
workspace.

- In mux mode that is the host's `host.log`. Client-side traces are not opened
  or merged.
- `:service-health` shows the owner role, the active level, the log path and
  any logger failure.

Each record is one line. It has an RFC 3339 timestamp, the level, the owning
role and PID, the short workspace ID, the subsystem, the message and any
`key=value` context.

```
2026-08-27T12:34:56.789+02:00 WARN  host[8123] ws=a1b2c3d4 lsp: language server stopped: exited with 1 language=rust
```

### When a window or terminal client disappears

A persistent session outlives whatever is attached to it. If the attached
window or terminal goes away without detaching, the host keeps running and
leaves one warning in that session's `:notifications`, stamped with the time.
You see it when you next attach.

- If the host knows why, the warning says so: a write to the client timed out,
  the client stopped reading, a message was invalid or truncated, or the
  connection reported an I/O error.
- Otherwise the warning says the cause is unknown. A connection that simply
  closes cannot be told apart from a crash, a killed process or a closed
  terminal, so Runyte does not guess.
- `:detach`, quitting, closing a window normally, switching to another session
  and control commands leave no warning.
- A warning stays with the session that lost its client. Other running sessions
  are not affected.
- The warning names only the cause. It never includes document text, terminal
  output or anything you typed. It lasts as long as the host does.

### Levels and startup controls

| Flags | Level |
| --- | --- |
| *(none)* | warning |
| `-v` | info |
| `-vv` | debug |
| `-vvv` and beyond | trace |

The default records warnings and errors. You can see the first failure without
having to reproduce it, and normal use writes very little.

**`--log PATH`** chooses an explicit file.

- If Runyte can't use the file, it refuses to start. Quietly writing somewhere
  else would be misleading.
- If another running Runyte process owns the path, Runyte waits up to two
  seconds for it to let go, then refuses. Choose another path, or let the first
  process exit.

**An unwritable default file** only affects logging. Editing continues, a
persistent host keeps serving, and the failure appears on stderr, as a
notification and in `:service-health`. If a file stops accepting writes later,
editing still continues. You get one warning in `:notifications`, and
`:service-health` keeps showing the failure.

**Persistent hosts** set verbosity and destination when they start.

- `--serve`, `--session-restart`, and the launch that creates a missing host
  pass them on.
- Attaching to a running host does not change its logger. If you pass `-v` or
  `--log` there, Runyte tells you the session kept its own configuration.
- Restart to change it:

```sh
runyte --session-restart -vv     # the only way to change a running host's logging
```

A normal restart refuses unsaved work. Save first, or use `--force` if you mean
it. You can't change the log level while Runyte is running.

### Who owns the log

The process that holds the editor state owns the log:

| Mode | Owner | File |
| --- | --- | --- |
| ide | the TUI process | `.runyte/standalone-<pid>.log` |
| mux | the host process | `.runyte/host.log` |
| editor | the TUI process, only with `--log PATH` | that path |

- Several ide-mode editors can open one workspace, so the file name includes
  the process ID. Two editors never share a file.
- A workspace has exactly one host, so the host log has one name. The host
  keeps logging while no TUI is attached.
- A client never writes to the host's log or sends records over the local
  protocol. Transport problems must be logged even when the transport is
  broken. The host records what it sees of clients: attachment, detachment,
  refusal, disconnection and rejected frames.
- Failures in a client's own input, rendering or terminal setup are printed to
  stderr after the terminal is restored. They are not in the host log.
- Logs live in the runtime state directory, normally `.runyte/`. They are never
  placed under a Git-tracked directory.

**File safety:**

- Linux and macOS create logs readable only by the owner (`0600`). Windows uses
  a protected ACL for the owning user.
- On Windows, private storage needs local NTFS. Network volumes, reparse points
  and hardlinked files are refused. Other filesystem types are not supported.
- Writes refuse symlinked parent directories, symlinked targets, hard-linked
  files, and special files such as FIFOs.
- Rotation keeps using the opened file and directory, even if their names
  change.
- A refused default log is reported and editing continues. An unusable explicit
  `--log` stops startup.

### What is recorded, and what never is

**Recorded:**

- process startup, version, role, workspace identity and orderly shutdown;
- session publication, idle retirement, signal termination and forced
  termination;
- client attachment, detachment, refusal and closure, a client that stops
  reading, and malformed or truncated frames;
- language servers becoming ready, restarting and stopping;
- terminal sessions that fail to start, and children that exit;
- Git failures already converted into typed results;
- background services whose channel closes;
- panics, with the thread, location, message, and a backtrace when enabled.

**Not yet recorded:** file-watcher lifecycle beyond its channel closing, and a
host restart as something different from a start followed by a stop.

**Never recorded:** routine keystrokes, rendered frames, successful commands,
buffer edits, and complete language-server requests or responses. No level ever
records buffer text, selections, clipboard contents, typed or pasted text,
terminal contents, credentials, environment-variable values, unrestricted
subprocess output, or full LSP JSON.

**Deliberately thinner records:**

- A failed Git operation records the refusal and exit status. It leaves out the
  arguments and Git's stderr, because a failing commit's arguments contain the
  message you just typed.
- A stopped language server records the language but not the reason. The reason
  can contain the end of the server's stderr.
- Both are available in full through `:notifications`, `:lsp-status`, and the
  interaction line.

Records **do** contain local paths and process metadata. They identify the
local operation that failed. Review a log before sharing it.

### Bounds

- The active file holds at most 4 MiB. One previous 4 MiB file sits beside it,
  ending in `.1`.
- The owning process rotates the log while running, and at startup if it
  inherits a full file. A long-lived or often-restarted host can't grow the log
  forever.
- Before opening its own default log, an ide-mode process cleans up. It keeps
  the four newest logs of exited ide-mode processes and removes older ones with
  their `.1` files. Logs of live processes are never removed.
- Runyte never waits for the disk. Records go through a bounded queue to one
  background writer. If the queue is full they are dropped, so input and
  rendering are not delayed. A later record says how many were lost.
- Shutdown and panic paths flush for a limited time and never wait forever.

<!-- navigation -->
[Contents](README.md) · [Previous: MCP server](agent-access-to-workspace-context.md) · [Next: Configuration](configuration.md)
