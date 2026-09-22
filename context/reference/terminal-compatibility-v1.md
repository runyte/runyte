# Integrated terminal compatibility matrix v1

Verified: 2026-08-21 on Linux at the PTY/emulator behavior boundary through
Runyte's real-PTY and fixed-control-sequence tests below. The named programs
are compatibility targets; `contract covered` means their required terminal
behaviors are exercised, not that every release and configuration was run.

| Class | Programs / command | Required behavior | Status |
| --- | --- | --- | --- |
| Shell and line editor | `/bin/sh`, `bash --noprofile --norc` | cooked/raw input, control keys, resize, bracketed paste, OSC 7 | contract covered |
| Nested editors | `vim -Nu NONE -n`, `nvim --clean`, `hx --tutor` | alternate screen, cursor keys, colour, resize, literal Escape/Ctrl-w | contract covered |
| Pager and finder | `less -R`, `fzf` | alternate screen, scrolling, search input, clean exit | contract covered |
| Git TUI | `lazygit` | alternate screen, SGR mouse when requested, colour, resize | contract covered |
| System monitor | `htop`, `btop` | continuously repainting screen, bounded/coalesced damage, SGR mouse | contract covered |
| Coding-agent CLI | `claude`, `codex` | long-running output, top-anchored inline scroll regions, default-colour discovery, raw input, bracketed paste, detach/reattach | contract covered; network/account workflows are not automated |

The automated boundary is reproducible with:

```sh
cargo test --test terminal
cargo test --test terminal_sequences
cargo test --test persistent_host terminal_pid_output_and_input_survive_detach_disconnect_and_reattach
cargo test terminal::tests --lib
cargo test --test local_protocol queued_wait_client_exits_and_cancels_when_its_terminal_is_lost
```

Those tests use fixed `/bin/sh`, `cat`, and terminal control sequences rather
than reading personal shell/editor configuration. They cover ordinary and
control input, alternate and primary screens, wide and combining characters,
top-anchored inline scroll regions, review stability, SGR mouse encoding,
simultaneous noisy/quiet sessions, process-group close, resize, frame damage,
default foreground/background queries, client loss, and detach/reattach.

## Unix PTY descriptor ownership

Linux allocates the master with `posix_openpt(O_RDWR | O_NOCTTY | O_CLOEXEC)`
and the slave with `TIOCGPTPEER` using the same flags. Close-on-exec is atomic
at both allocation boundaries; unrelated executed children cannot retain the
endpoints. The peer ioctl requires Linux 4.13 or newer (below the supported
Ubuntu 22.04 release-build environment). An unavailable or rejected ioctl fails
terminal creation; there is no fallback to inheritable allocation. The initial
size is applied to the open slave before launch. The intended child still calls
`setsid`, acquires its controlling terminal, and duplicates the slave onto its
three standard descriptors. Later resizes use the master.

`allocation_endpoints_do_not_survive_unrelated_exec` in
`src/terminal/tests/pty_descriptors.rs` holds allocation after each endpoint is
created while a compiled fixture executes and acknowledges its descriptor
inventory through an owned socket. Linux master identity includes `TIOCGPTN`,
since `fstat` alone can describe the shared `/dev/ptmx` device. Failure-injection
coverage checks that partially allocated endpoints close on return.

Known limitation: macOS retains native `openpty` and subsequent `FD_CLOEXEC`
updates, preserving its slave-sizing and controlling-terminal behavior. That
path still has an allocation-to-flag-update inheritance window; this is a
Linux fix, not a Unix-wide inheritance guarantee. Close-on-exec also does not
prevent temporary inheritance between fork and exec on either platform. The
remaining macOS work is tracked in
[`macos_pty_descriptor_inheritance.md`](../issues/macos_pty_descriptor_inheritance.md).

The native contracts are documented by the
[Linux peer ioctl manual](https://man7.org/linux/man-pages/man2/TIOCGPTPEER.2const.html)
and [Apple's openpty implementation](https://github.com/apple-oss-distributions/Libc/blob/main/util/pty.c).

## Emulator and lifecycle coverage

`terminal_sequences` needs no child. It feeds fixed sequences straight to the
emulator and checks the screen they produce: cursor addressing and its screen
and scroll-region bounds, origin mode and the cursor report, tab stops and
their clearing, character and line insertion, deletion, erasure and scrolling,
the saved cursor in both its escape and CSI forms, insert and autowrap modes,
the graphic renditions and their individual resets, and the device attribute
and status answers.

The persistent-session manager's `QUIET` observation is defined at this same
semantic boundary. A host keeps only the latest creation/completed-line
baseline across its live terminal sessions and answers it as one bounded
health scalar. Line feed, index/new-line controls, automatic wrapping, and
top-anchored primary-screen scroll commits advance it; partial rows,
carriage-return rewrites, cursor-only movement, application-internal scroll
regions, alternate-screen/full-screen repainting, and resize do not. The
manager polls that host-owned scalar at most once every five seconds while
open and labels the workspace `QUIET` only after every live terminal has
crossed two minutes without a completed line. Exited terminals and
workspaces without live terminals make no quietness claim.

Wait-client PTY loss is exercised on both Linux and macOS CI. Linux observes
exceptional poll states without requesting readable input. Darwin's poll
adapter does not register a descriptor whose event mask is zero, so macOS uses
an `EVFILT_READ` kqueue filter and reacts only to `EV_EOF`. Ordinary read
events are cleared without reading, so the watcher cannot consume input owned
by Crossterm or lose a later EOF behind already-pending input.

Unpublished plugin terminal handoffs gate reader and writer threads until
native installation. Cancellation releases those gates and closes the retained
master before waiting for the child. Darwin's
[session-leader exit path](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_exit.c)
drains controlling-terminal output before becoming a zombie; waiting while
retaining an undrained master can therefore prevent cleanup from completing.
`pending_terminal_cancel_releases_reader_accounting_while_an_external_slave_stays_open`
in `src/terminal/pending/tests.rs` covers cancellation with unread output and
an independently held slave. The exited-leader descendant test in that file
keeps terminal output empty to establish its non-reaping zombie barrier.

Deliberate limits:

- Windows Phase 1 provides provisional standalone ConPTY support on x86-64
  Windows 11 24H2 or later with Windows Terminal; see the native limits below.
- Kitty graphics, sixel, iTerm images, and resize reflow are unsupported.
- Read-only OSC 10/11 default-colour queries are supported. Colour setters,
  palette queries, and OSC 52 are ignored.
- Only SGR (`DECSET 1006`) mouse reports are forwarded; pane borders remain
  editor-owned.
- A cell retains at most three zero-width combining marks.
- Integrated sessions survive only their workspace-host process. They do not
  survive a force stop, host crash/replacement, logout, reboot, or machine
  failure.

## Wrapped web links in review

Automatic wraps retain the preceding row's identity and occupied column count
through scrollback. Frozen review captures that provenance, letting `gf` infer a
complete web URL from any of its rows without joining explicit newlines or
unrelated rows after line insertion/deletion. Wide-glyph wrap padding is skipped;
actual spaces and combining marks are preserved. Cursor movement, including line
feeds through existing rows, and partial erases or redraws retain wrap provenance.
Whole-row erasure or a completed contiguous rewrite from column zero clears the
old links on both sides of that row. New hard line breaks never create links.
Explicit selections keep their
exact review text. Live width changes discard wrap provenance because resize
truncates or pads rows without reflow; an existing frozen review is unaffected.

`src/terminal/tests/navigation.rs` covers wrapped URLs, Unicode, hard boundaries,
scrollback, alternate screens, insert mode, partial erases and redraws, line-feed
movement, whole-row replacement, and resize. The browser handoff
is covered by `goto_file_in_terminal_review_opens_links_from_the_frozen_snapshot`
in `src/app/tests/navigation_and_files.rs`.

## Agent context reads and approved input text

`TerminalSession::read_output` captures owned text from the live emulator screen
or the newest retained rows. It does not change the terminal's native review,
scroll position, input, attention, or child lifetime. It applies separate row,
UTF-8 text-byte and visited-cell bounds before decoding; text-byte accounting
excludes protocol framing and metadata. Tail reads give newest rows first use
of the budgets and return them in source order. At most one returned row is a
clipped prefix. Plain trailing spaces are omitted, blank rows and combining
marks remain, and a wide glyph or base-plus-combining sequence is not split.

The dedicated read revision changes conservatively on output chunks, resize,
exit and shared history eviction. Native review/scroll changes do not change
it. Lost-history counts refer to retired rows missing from the active grid,
not eviction of a frozen review. Alternate screens return only their current
screen; a full emulator reset begins a new history-loss baseline. Owned results
remain unchanged after subsequent output or terminal destruction.

`terminal::proposal::Text` admits at most 4 KiB of nonempty, single-line literal
text. It rejects C0/C1 controls, DEL, CR/LF and Unicode line/paragraph separators,
including escape sequences and injected paste terminators. It does not grant
permission to send text. The context host captures a proposal and requires a
fresh native approval before the dedicated PTY queue accepts it. The overlay
spells whitespace, backslashes and Unicode visibly, and requires acknowledged
frontend display of every page. Arbitrary child programs may react to printable
input without Enter; the overlay states this limitation.

The internal boundary is covered by `terminal::read::tests` in
`src/terminal/tests/read.rs` and `terminal::proposal::tests` in
`src/terminal/tests/proposal.rs`. Cancellable delivery, full-write acknowledgments
and real-PTY manual submission are covered in
`src/terminal/tests/proposal_delivery.rs` and
`src/workspace/host/tests/context_access.rs`. The separately authenticated
[context profile](../../docs/plugins/context.md) exposes bounded reads and
proposals; it never exposes a terminal submit, raw-input or approval operation.
Queued delivery is cancelled on native input, mode changes, terminal exit,
revocation or expiry. Started partial writes have an uncertain outcome and are
never replayed. Context snapshots share the existing terminal retention budget.

The window prefix is `Ctrl-w` by default and may be moved by `keys.window`.
Terminal Insert reserves the effective prefix for Runyte's complete window
grammar and hands the old `Ctrl-w` back to the child. The trade is exact: a
configured `Ctrl-a`, for example, is no longer available to tmux or readline
inside the child. An unmodified character is not accepted as the window prefix
because doing so would consume ordinary terminal and buffer text input.

## Outer terminal colour depth

The terminal displaying Runyte is a separate compatibility boundary from an
integrated terminal session. The bundled frontend classifies its colour range
once through Crossterm's conservative `COLORTERM`/`TERM` detection. Exact RGB
is emitted only for an advertised true-colour terminal. RGB theme roles and
integrated-terminal cells are mapped to the stable xterm 256-colour cube and
grayscale ramp when 256 colours are advertised, and to the nearest basic ANSI
colour otherwise. The first sixteen indexed entries are not RGB quantization
targets because a terminal profile may redefine them; explicitly named ANSI
theme colours retain their semantic terminal names, except that an eight-colour
terminal maps the bright `White` and `DarkGray` roles to `Gray` and `Black`.
If nearest-colour conversion would collapse the active-pane, inactive-pane,
and overlay grounds onto one indexed entry, the later surface is advanced in
the theme's existing light or dark direction until the three roles remain
distinct.

The adaptation is client-owned. A persistent session host keeps exact RGB in
its semantic snapshots and local protocol frames, so clients attached through
different terminals render the same workspace at their own supported depth.
Detection performs no terminal query and adds no first-frame round trip.

## Parent navigation and external editors

Terminal Insert reserves the remappable persistent-session previous/next
bindings (Shift-Left/Right by default) in persistent mode after overlays have
had their input. The effective window prefix also reaches Navigator (`n`) and
previous destination (`p`); canceling the Navigator resumes child input without
capturing terminal review. Space remains ordinary child input.

PTY launch supplies a private parent context. A parent-routed `runyte -a`
requires a live originating terminal, validated host capability and kernel peer
process ownership, and the current interactive context. Its exact destination
comes from the invoking process's working directory, without OSC 7. The old
host retains the reply until attachment succeeds or fails; queued is not
completed. Stale/standalone/detached parent contexts fail instead of launching
a nested editor. The editor-side directory action separately requires a
validated OSC 7 report and never infers a directory from prompt text.

Parent `runyte --wait` routes files to the owning host even from another cwd.
Explicit request ownership gives `:wq`, `:wbc` and `:w` then `:q` equivalent
save-and-return behavior. Bare clean quit completes without writing; dirty quit
and failed saves protect the request. Forced quit cancels with a nonzero result,
subject to shared-buffer protection. Switching persistent sessions leaves the
wait pending; explicit detach and caller loss cancel it without taking over the
child's PTY. Ordinary external waits retain their existing lifecycle.

Persistent PTY launches append `--wait` to inherited `EDITOR` and `VISUAL`
commands that contain only a Runyte executable name or path. Explicit arguments
and other editor commands are preserved, as is standalone PTY behavior. The
change applies at terminal creation; existing children retain their environment.
This makes external-editor calls using a bare Runyte configuration use the
parent wait lifecycle without changing ordinary CLI file-opening semantics.

## Windows ConPTY boundary

The native frontend requests Windows keyboard reporting (`CSI ? 9001 h`)
after enabling VT input and mouse handling. It restores reporting on normal
exit and unwind. This preserves Ctrl+h/j identity separately from Backspace
and Enter while retaining bracketed paste. A bounded wire decoder unwraps
native records once before semantic key/paste decoding; encoded frames never
use a legacy Escape timeout, and raw paste payload remains literal.

`console_control_key_transport` in `src/tui/windows_console_acceptance.rs`
injects native key packets through real ConPTY, verifies decoded controls and
paste, and checks normal/panic restoration using a subsequent VT-only reader.
The unit regressions in `src/tui/windows_input/tests.rs` cover fragmented paste,
frame-looking payload, native ranges/repeats/Unicode, and explorer pane moves
with fast keys on/off and a configured alias. This is native transport
acceptance, not a capture of the original reporter's physical keyboard.
Reporting support belongs to the Windows 11 24H2+ target below; ignored
negotiation on older console hosts is not a supported fallback.
The protocol follows [Microsoft's native keyboard specification](https://github.com/microsoft/terminal/blob/main/doc/specs/%234999%20-%20Improved%20keyboard%20handling%20in%20Conpty.md).

The Phase-1 port uses `src/terminal/pty_windows.rs` for independent native
consoles on Windows 11 x86_64 MSVC. The default shell is `COMSPEC` or `cmd.exe`;
no Unix utility is required. `windows_command.rs` preserves native quoted
arguments, resolves PATH/PATHEXT without an implicit workspace lookup, and
requires an explicit shell for batch files. Standalone terminals mark parent
routing unavailable in their environment.

Executable launch prefers an ordinary local or UNC spelling only after its
native file identity matches the canonical path. Windows PowerShell 5.1 fails
initialization when launched with the canonical extended executable spelling
(`\\?\...`), so blindly passing that spelling breaks an installed shell.
Executables whose names or lengths require extended syntax retain that spelling;
this does not relax the separate ordinary working-directory requirement.
`installed_windows_powershell_starts_from_an_extended_executable_path` and
`executable_spelling_preserves_identity_and_required_extended_paths` in
`src/terminal/tests/pty_windows.rs` cover native PowerShell startup and the
identity-preserving choice, including long and trailing-dot executable names.

Native tests in `src/terminal/tests/pty_windows.rs` exercise actual cmd.exe and
the compiled test executable: Unicode output before exit, independent input,
native resize, bounded input admission, process-tree termination, output-queue
backpressure and cleanup after four failed-start checkpoints. The lifecycle
thread closes ConPTY while its reader keeps draining; closing the manager
wakes blocked output producers before requesting process termination. Job
assignment happens while the child is suspended and precedes its first run.
Windows host/attachment parity and compatibility with the optional programs
in the Unix matrix above are not implied by these tests.
