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

Wait-client PTY loss is exercised on both Linux and macOS CI. Linux observes
exceptional poll states without requesting readable input. Darwin's poll
adapter does not register a descriptor whose event mask is zero, so macOS uses
an `EVFILT_READ` kqueue filter and reacts only to `EV_EOF`. Ordinary read
events are cleared without reading, so the watcher cannot consume input owned
by Crossterm or lose a later EOF behind already-pending input.

Deliberate limits:

- Windows remains unsupported until a separately approved ConPTY backend.
- Kitty graphics, sixel, iTerm images, and resize reflow are unsupported.
- Read-only OSC 10/11 default-colour queries are supported. Colour setters,
  palette queries, and OSC 52 are ignored.
- Only SGR (`DECSET 1006`) mouse reports are forwarded; pane borders remain
  editor-owned.
- A cell retains at most three zero-width combining marks.
- Integrated sessions survive only their workspace-host process. They do not
  survive a force stop, host crash/replacement, logout, reboot, or machine
  failure.

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
