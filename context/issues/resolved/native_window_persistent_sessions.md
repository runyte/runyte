---
title: "Native windows cannot attach to persistent sessions"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 8683956
---

## Resolution

Commit `8683956` (`Attach native windows to persistent sessions`) reuses the
existing `run_workspace_switcher` and `run_attached` loop through an
`AttachedSurface` adapter and native input events. GPUI still owns the main
thread, while the attachment loop runs on its existing worker. Session history,
single-client admission, switching, protected-state quit refusal, and fallback
to previous running sessions consequently retain the terminal client's host
semantics. The persistent window close button sends detach; standalone close
continues to request `:qa`.

`App::native_media` previously described an entire process, while native media
capture depended on direct access to that process's editor state. Protocol 74
adds the per-attachment media capability, owned pane media metadata, bounded
navigation/page-count requests, and media action responses. The host retains
read-only projections and their selected PDF page; decoding, Poppler work,
zoom, and pixel/text selection remain in the window. Terminal clients display
the unsupported-media placeholder, retain media title markers even when the
path is clipped, and can return to the source directory. Opening binaries
uses the current attachment's capability, including when a terminal-only build
started the host.

Native input and media requests carry attachment generations so queued input
from an earlier session cannot supply a painted-frame witness in a new one.
Window resize events remain valid across generations: discarding one before
the first destination paint would otherwise strand the host at the previous
geometry. Media actions also carry their required frame and wait in a bounded
queue until that frame or a newer replacement reaches GPUI. The transport's
priority semantic lane can overtake visual frames, so an action must not be
lost merely because the window still displays the preceding PDF page buffer.
Paint acknowledgements do not synthesize physical approval input.

`finish_attached_detach` used to cancel host-owned parent-context waits even
though their calling processes remained live inside retained terminal sessions.
It now completes only waits belonging to that interactive attachment and leaves
parent-context waits pending. Disconnect cancellation for outside-shell control
clients is unchanged. Native-feature builds must be the `runyte` on `PATH` when
mixing frontends and integrated `runyte --wait`, because the private protocol
version is checked at attachment.

Regression coverage:

- `native_media_capability_and_projection_survive_frontend_handoff` in
  `tests/local_protocol.rs` covers admission, PDF metadata/navigation/page
  buffers, stale media back requests, projection retention, the terminal
  placeholder, terminal navigation, and external-program prompting.
- `integrated_parent_wait_survives_explicit_frontend_handoff` in
  `tests/local_protocol.rs` covers colon detach and the window-close request,
  both attachment directions, unsaved prompt text, and successful completion
  back into the original live terminal. Existing save-route tests share its
  fixture.
- `media_protocol_bounds_and_page_changes_require_complete_frames` in
  `tests/local_protocol.rs` checks lossless frame conversion, page-count and
  navigation bounds, and full-frame publication when media pages change.
- `attachment_handoff_keeps_window_resize_but_drops_old_document_input` and
  `media_actions_wait_for_their_visual_frame_and_do_not_cross_attachments` in
  `src/native_frontend/tests/input.rs` cover the asynchronous bridge races.
- `tests/native_window.py --mux` exercises a real window's editing, media
  controls and selection, session switching, quit fallback, close-button
  detach, terminal-client handoff, retained unsaved text, and live terminal
  children. The same harness without `--mux` checks standalone close refusal.

Linux validation passed formatting, default and native all-target Clippy, all
4,641 ordinary tests (54 ignored), 29 native adapter tests (one optional Poppler
test ignored), and both isolated X11 acceptance modes. Canonical default-feature
coverage measured 92.04% lines, above the unchanged 89% floor. Astra High found
the two asynchronous bridge races above; its follow-up review was clean.

Known limitation: The native window remains unavailable on Windows. This change
was validated on Linux; native macOS runtime validation remains outstanding.

## Report

### Observed behavior

The experimental native window (`--window`, built with `--features native`)
runs only in standalone ide and editor modes. `src/main.rs` refuses any other
launch mode with:

```text
--window currently supports standalone ide and editor modes
```

The README records the same limit: persistent-session window attachments are
not implemented. Closing the window ends its editor state and terminal
sessions, and the window cannot discover, attach to, or switch between the
persistent sessions that terminal clients use.

Two consequences follow:

- A persistent session created from a terminal client cannot be continued in
  the window, and the window cannot create a session that outlives it.
- `Ctrl-g` in Claude Code or Codex running in an integrated terminal of the
  window is refused, because standalone instances reject parent-context
  `runyte --wait` requests from their own terminals.

### Expected behavior

Persistent sessions are shared between the terminal client and the native
window. Either frontend can create a session, attach to it, detach from it,
and switch between running sessions, and a session stays live after either
frontend exits until it is stopped explicitly.

A typical sequence:

1. `runyte --window --mux` creates or attaches to the workspace's session.
2. `:detach` in the window closes the window; the host keeps every pane,
   buffer, unsaved edit, and terminal session.
3. `runyte --mux` in a terminal attaches to the same session.
4. `:detach` in the terminal, then `runyte --window --mux` attaches the window
   again.
5. `Space Space`, `Space 1`–`Space 9`, `Shift-Left` / `Shift-Right`, and
   `Ctrl-w a` switch sessions from either frontend.

Single-client attachment stays as it is: the host refuses a second interactive
attachment, so the frontends take turns rather than viewing a session at the
same time.

#### Quitting and detaching in the window

- `:detach` closes the window without stopping the session, as it disconnects
  a terminal client.
- `:quit` from the last pane and `:quit-all` behave as in the terminal client:
  they stop a clean session and attach the same window to the previously
  visited running session, closing the window only when no running session can
  take it. Unsaved buffers and live terminal children still refuse the stop.
- In a persistent session the window's close button requests `:detach`, not
  `:qa`. Closing a terminal emulator with an attached terminal client already
  leaves the host running, and `:qa` refuses whenever a terminal child is
  live, which would make the close button usually report an error. In
  standalone `--window` the close button keeps requesting `:qa`.

#### Media panes

- In the window, image and PDF panes in a persistent session render and behave
  as they do in standalone `--window`: zoom, pan, selection, PDF page
  navigation, and the PDF page buffer.
- A terminal client renders a media pane's body as:

  ```text
  MEDIA UNSUPPORTED IN THE TERMINAL MODE
  ```

  The `[pdf]` / `[image]` title remains. Host-side navigation such as Escape to
  the source directory and `Space e` keeps working from the placeholder.
- Opening a binary file (`:open`, the explorer, the directory tree) while a
  terminal client is attached keeps today's behavior: supported media types
  get the external-program prompt, as in a terminal build. While the window is
  attached, they open as read-only media projections in the pane.
- Media panes created while the window was attached survive a switch to a
  terminal client and render normally again when the window reattaches.

#### `Ctrl-g` and pending external edits

In a persistent session the window supports the existing parent-context flow:
`Ctrl-g` in Claude Code or Codex in an integrated terminal opens the prompt as
a buffer in the same session, covering the origin terminal until the edit
completes.

A pending edit of this kind must survive a frontend handoff. Today
`finish_attached_detach` in `src/main.rs` calls
`cancel_parent_waits("outer TUI detached before the external edit
completed")`, so pressing `Ctrl-g`, running `:detach` in one frontend, and
attaching the other cancels the edit and returns a nonzero result to the
agent. This is inconsistent with the neighbouring cases:

- Losing the client connection (the `ServerEvent::Disconnected` path) clears
  the attachment without cancelling parent-context waits.
- Switching sessions keeps the request pending in its host, as documented in
  `docs/user-guide.md`.

The waiting `runyte --wait` process runs inside the host's own terminal
session and does not depend on any client, so explicit detach should keep
parent-context waits pending as well. That includes detach requested by the
window's close button. Waits owned by control clients (`runyte --wait` from an
outside shell) keep their current cancellation on disconnect.

### Constraints

- The attached terminal client already renders only from host frames
  (`run_attached` with `ui::render_host_frame`), and the window already renders
  Ratatui cells through `native_frontend::Surface::Native` and feeds input
  through `native_frontend::Events::Native`. A window client is expected to
  reuse the attached-client loop with those surfaces rather than add a second
  attach implementation. GPUI owns the main thread, so the attach loop runs on
  the worker thread `native_frontend::launch` starts.
- Media state currently crosses the bridge by reading live editor state
  in-process: `native_frontend::capture_media` reads `pane.terminal`,
  `buffer.media_path`, `shows_pdf_pages()`, and `app.media_requests`, and
  `native_frontend::update_media` calls `leave_native_media`,
  `navigate_native_media`, and `update_native_media_pages`. In a persistent
  session that state lives in the host, so it needs private protocol
  additions in `src/protocol/`: per-pane media path and page in frames, and
  client-to-host media navigation and page-count requests. These are wire
  changes that require a `protocol::VERSION` bump.
- Image decoding and Poppler rasterization stay in the window client. The
  transport is local, so media paths resolve on the same machine.
- `App::native_media` is currently fixed per process. In a persistent host it
  follows the attached client, declared at the handshake alongside the existing
  `directory_handoff` capability, so the host decides between media projection
  and the external-program prompt per attachment.
- The handshake refuses mismatched protocol versions. The terminal client, the
  window, and the `runyte --wait` that integrated terminals resolve through
  `PATH` (bare `runyte` in `EDITOR` / `VISUAL` gains only `--wait`) must
  therefore come from the same build. The host-side media code is not behind
  the `native` feature, so a session started by either frontend can hold media
  panes. The user guide should state the expectation that the
  native-feature build is the `runyte` on `PATH` when both frontends are used.
- Windows remains out of scope for the native window.

### Reproduction

```sh
cargo build --features native
target/debug/runyte --window --mux
```

Startup fails with `--window currently supports standalone ide and editor
modes`.

For the detach cancellation, in a terminal client attached with
`runyte --mux`:

1. In an integrated terminal, start Claude Code or Codex with
   `EDITOR='runyte --wait'` and press `Ctrl-g`.
2. Run `:detach` while the prompt buffer is open.
3. Reattach with `runyte --mux`.

The agent has already received a cancelled edit; the buffer is kept but the
agent no longer waits for it.
