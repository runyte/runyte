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
```

Those tests use fixed `/bin/sh`, `cat`, and terminal control sequences rather
than reading personal shell/editor configuration. They cover ordinary and
control input, alternate and primary screens, wide and combining characters,
top-anchored inline scroll regions, review stability, SGR mouse encoding,
simultaneous noisy/quiet sessions, process-group close, resize, frame damage,
default foreground/background queries, client loss, and detach/reattach.

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
