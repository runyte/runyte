# Capture driver

Requires Linux or macOS, Python 3.10+, a built Runyte executable, and local
monospace TTF/OTF fonts. Paths below are placeholders except for the skill's
repository-relative paths. Resolve them for the current machine; the helper
does not depend on the old website scripts, a particular checkout, or fonts
in a particular home directory.

Install dependencies into a temporary virtual environment:

```sh
python3 -m venv /tmp/runyte-capture-venv
/tmp/runyte-capture-venv/bin/pip install -r skills/runyte-screenshots/scripts/requirements.txt
```

Create a temporary workspace containing a `README.md` for a simple first scene,
and run `git init /tmp/demo-workspace` so Runyte recognizes its project root.
Write an action array to `/tmp/scene.json`, for example:

```json
[
  {"op": "expect", "text": "Runyte"},
  {"op": "command", "text": "open README.md"},
  {"op": "command", "text": "render"},
  {"op": "save", "path": "/tmp/rendered-markdown.webp", "required": ["[rendered README.md]"]}
]
```

Run it from the repository root:

```sh
/tmp/runyte-capture-venv/bin/python skills/runyte-screenshots/scripts/capture.py \
  --binary /absolute/path/to/runyte \
  --cwd /tmp/demo-workspace \
  --font /absolute/path/to/Monospace-Regular.ttf \
  --bold-font /absolute/path/to/Monospace-Bold.ttf \
  --italic-font /absolute/path/to/Monospace-Italic.ttf \
  --bold-italic-font /absolute/path/to/Monospace-BoldItalic.ttf \
  --theme ocean-dark --label 'source revision and scene description' \
  --steps /tmp/scene.json
```

The default 160×58 cells at 12×26 pixels produce a 1920×1508 image. Use
`--columns`, `--rows`, `--font-size`, `--cell-width`, `--cell-height`, and
`--text-offset` to fit a different font or composition. Cell width should match
the font's monospace advance. Missing style fonts fall back to `--font` without
synthesizing bold or italic. The renderer draws colors, inverse cells, bold,
italic, underline, and strikethrough; wide glyphs and combining marks use the
decoded screen's cell positions. Inspect coverage of Nerd Font and Unicode
glyphs; there is no automatic fallback-font selection.

`--foreground` and `--background` specify the terminal's default colors and
OSC color-query replies. Runyte normally emits explicit theme colors. Named
ANSI colors use an xterm palette; truecolor and indexed colors are decoded by
pyte. Match terminal defaults to the intended scene when they are visible.
The hardware cursor is omitted by default; `--cursor` adds an outline at its
visible location, without claiming to reproduce its shape or blink phase.

`--config` supplies a complete scene configuration instead of the generated
theme/LSP-disabled config. Repeat `--arg` for Runyte arguments; use
`--arg=--persistent` for values starting with a dash. `--help` lists all options.

## Actions

JSON strings use actual JSON escapes, such as `\u001b`, not literal `\\x1b`.
Ordinary actions pump output for 0.2 seconds afterward; `wait` overrides this.
`save` captures the current screen after checking required text. Add `expect`
or `wait` actions beforehand if the scene needs further output to arrive.

| Operation | Fields | Behavior |
| --- | --- | --- |
| `key` | `text`, optional `wait` | Send UTF-8 text or raw control/escape sequences. |
| `command` | `text`, optional `wait` | Send Escape, pause 100 ms, then `:` + command + Enter. Use from an editor-controlled mode. |
| `paste` | `text`, optional `wait` | Send bracketed paste to the currently focused input. |
| `expect` | `text`, optional `timeout` | Wait up to 10 seconds by default for a literal substring in visible text. Fail with the screen if absent. |
| `wait` | `wait` | Process output for this many seconds, including terminal query replies. |
| `screen` | optional `wait` | Return visible text and whether the PTY has closed. |
| `resize` | `columns`, `rows`, optional `wait` | Resize the decoder and PTY; the kernel notifies the foreground process. |
| `mouse` | `x`, `y`, optional `button`, `release`, `wait` | Send SGR mouse coordinates, counted from 1. Default button 0 is left press; release uses `true`. Wheel up/down use buttons 64/65. |
| `save` | `path`, optional `required`, `timeout` | Write PNG or lossless WebP plus `.txt` and `.json` sidecars. `required` is an array of literal visible markers. |
| `quit` | none | Interactive mode only: close the capture process. |

Examples of input actions:

```json
{"op": "key", "text": "\u001c"}
{"op": "key", "text": "\u0017v"}
{"op": "key", "text": " f"}
{"op": "key", "text": "\t"}
{"op": "key", "text": "task\r"}
{"op": "key", "text": "\u001b[B"}
```

These encode Ctrl-\, Ctrl-w then v, Space then f, Tab, typing `task` followed by
Enter, and Down. Confirm bindings in the target checkout. To send standalone
Escape followed by ordinary text, use separate actions with a pause; combine
Escape with following bytes only for an intentional escape sequence or Alt key.

## Interactive operation and custom scenes

Replace `--steps ...` with `--interactive`. Start with piped stdin, keep the
process alive using the execution tool's session handle, and send one JSON action
per line. The driver returns `{"ready": true}` and then one JSON result per
action, including errors that allow the scene to be adjusted. It continues to
drain PTY output between actions. Send `{"op":"quit"}` or close stdin to finish.
Batch failures instead exit nonzero. Both modes clean up the direct Runyte child
on exit, with a bounded SIGTERM/SIGKILL fallback.

For an agent-driven scene, inspect `screen` output, send the next actions, and
use `save` for visual previews. This accommodates arbitrary feature workflows
without baking a feature catalog into the helper.

For more elaborate setup, load the script using `importlib.util` and use
`options = module.parser().parse_args([...])` followed by
`with module.Capture(options) as capture:`. The options still require a mode
flag (use `--interactive`), but the stdin loop is only used by `main()`.
Call `capture.action({...})`, `capture.expect(...)`, and `capture.text()`.
Use `capture.env` for scene-owned helper processes when they need the same
runtime scope. Ordinary fixture preparation can use Python filesystem calls or
subprocess argument vectors in a separate recipe.

## Persistent sessions and real external programs

Use temporary project roots and `--state-dir /tmp/scene-runtime` for a
persistent-session scene. In a Python recipe, call `isolated_env(state_dir)`
and use that environment for each `runyte -c CONFIG --serve` host, the lifecycle
CLI calls, and capture clients. Pass the same explicit configuration to hosts
and clients. Consult the target checkout's user guide for session naming,
attachment, and switching commands. Own each host subprocess and stop each demo
workspace explicitly with the documented `--session-stop ... --force` command
in a `finally` block; wait for those hosts before deleting their runtime state.
Closing a capture client alone does not stop a persistent-session host or its
terminal processes. Never use a global stop command for scene cleanup.

External programs launched inside a terminal are real. A screenshot does not
require sending a prompt to a remote agent or granting extra permissions. Set up
the requested visible state using the existing authorization and required local
dependencies. Disable unrelated personal shell startup customizations when they
would affect the scene. If access to a required program/server is unavailable,
report that instead of drawing a substitute interface.

## Artifacts and validation

Each `save` writes the image, visible `.txt`, and a `.json` containing launch
arguments, workspace, configuration, geometry, fonts, and the caller's label.
Same-stem sidecars are replaced on another save. Use a dedicated artifact
directory. Paths/configuration can contain private data; inspect and sanitize
anything intended for tracking or publication. Sidecars are evidence of what
was captured, not a complete replay recipe or automatic verification of a
source revision.

Open each final image. Text assertions do not detect clipped glyphs, stale
highlighting, wrong focus, font substitutions, or a poorly composed layout.
Use a feature-specific marker, not only the word `Runyte`, for final captures.

The helper's regression checks run with:

```sh
/tmp/runyte-capture-venv/bin/python -m unittest discover \
  -s skills/runyte-screenshots/scripts -p 'test_*.py'
```
