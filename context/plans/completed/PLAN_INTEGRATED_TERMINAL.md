# Runyte integrated terminal

Status: completed 2026-08-23

Created: 2026-08-20

## Decision

Integrated terminals are pane content owned by the editor, but they are not
buffers. A terminal session has a pseudoterminal, child process, emulator,
bounded scrollback, title, working-directory metadata, and input state. None of
those values fit the rope, transaction, saved-file, or undo model of a text
buffer.

In persistent mode the workspace host owns terminal sessions. An attached TUI
owns only physical terminal input and rendering. Disconnecting a client
therefore detaches the view without closing the pseudoterminal or signalling
its child.

The supported lifetime is the workspace-host process:

- terminal sessions survive TUI detach, disconnect, and reattachment;
- switching panes or buffers does not stop a child;
- closing a terminal or force-stopping its persistent session is explicit;
- host termination, logout, reboot, and machine failure end the session.

## Pane content

A pane addresses either a buffer or a terminal session. Editor integration
switches on that content boundary before attempting buffer-only operations.
Terminal state lives under `src/terminal/`, which alone handles PTYs, escape
sequences, emulation, scrollback, terminal modes, and presentation cells.

Terminal sessions have stable ids and user-facing names. Several panes may
show terminal content according to the editor's ordinary pane lifecycle, while
session ownership remains with the workspace host.

## Input and review

Terminal Insert mode forwards normalized input to the child. Terminal Normal
mode keeps the live screen visible while editor commands select panes or enter
review. Review captures a bounded text projection of the terminal so ordinary
movement, search, selection, and yank semantics can operate on stable content
without claiming that the live terminal grid is an editable document.

A buffer selection may be sent to a terminal as one bracketed paste. This
supports composing structured input in an ordinary editable buffer while the
receiving program continues to own its own input model.

Escape cannot universally leave terminal input because full-screen terminal
programs need it. The terminal keymap therefore has explicit mode-transition
commands documented in the user guide and keymap reference.

## Emulator and transport boundaries

The emulator supports the control sequences required by shells, line editors,
pagers, nested editors, fuzzy finders, Git TUIs, system monitors, and other
interactive terminal programs. It maintains primary and alternate screens,
cursor and style state, SGR mouse reporting, bracketed paste, default-color
queries, resize, damage, and bounded scrollback.

Persistent frontends receive bounded terminal cell snapshots and damage. The
host never sends raw escape sequences for the frontend to interpret, and the
editor layers above `src/terminal/` never parse them.

Input and output queues are bounded so a noisy or stalled child cannot grow
memory without limit or stop editor input. Exited sessions retain enough state
to report their exit and permit review until explicitly closed.

## Deliberate limits

- Integrated terminals are currently Unix-only; Windows requires a separately
  designed ConPTY backend.
- Scrollback is bounded and terminal resize does not reflow historical rows.
- Graphics protocols, OSC 52, palette mutation, and non-SGR mouse protocols
  are unsupported.
- Terminal persistence is process-lifetime persistence, not a replacement for
  machine- or login-lifetime supervision.

The complete tested compatibility boundary and commands used to reproduce it
are recorded in `context/reference/terminal-compatibility-v1.md`. Current user
behavior lives in `README.md`, `docs/user-guide.md`, and
`context/reference/helix-keymap-v1.md`.
