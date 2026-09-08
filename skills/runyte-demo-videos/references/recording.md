# Recording scripted demos

## Requirements and invocation

Use Linux, Python 3.10+, FFmpeg with `libx264`, a built Runyte executable, and
local monospace TTF/OTF fonts. Keep this skill beside `runyte-screenshots`;
`record.py` resolves the shared driver relative to its own real path, including
when this skill is discovered through a symlink.

```sh
python3 -m venv /tmp/runyte-video-venv
/tmp/runyte-video-venv/bin/pip install -r skills/runyte-demo-videos/scripts/requirements.txt
ffmpeg -hide_banner -encoders
```

Create `/tmp/demo-workspace`, initialize it with `git init`, and populate the
files needed for the chosen feature. Save a JSON recipe such as:

```json
{
  "setup": [
    {"op": "expect", "text": "Runyte"},
    {"op": "command", "text": "open README.md", "interval": 0},
    {"op": "expect", "text": "Demo notebook"}
  ],
  "actions": [
    {"op": "key", "text": "i"},
    {"op": "type", "text": "A small edit, shown live.\n", "interval": 0.07},
    {"op": "key", "text": "\u001b", "wait": 0.5},
    {"op": "checkpoint", "name": "Edited source", "required": ["A small edit, shown live."]},
    {"op": "command", "text": "render", "wait": 0.5},
    {"op": "checkpoint", "name": "Rendered page", "required": ["[rendered README.md]"]},
    {"op": "wait", "wait": 2},
    {"op": "key", "text": " n", "wait": 0.5},
    {"op": "checkpoint", "name": "Navigator", "required": ["Navigator"]}
  ]
}
```

This example expects `README.md` to contain `Demo notebook`. Replace the content,
commands, and assertions to fit the actual request. The recorder itself contains
no fixed scene or feature catalog. An action array without the object wrapper
also works; it has no hidden setup.

```sh
/tmp/runyte-video-venv/bin/python skills/runyte-demo-videos/scripts/record.py \
  --binary /absolute/path/to/runyte \
  --cwd /tmp/demo-workspace \
  --font /absolute/path/to/Monospace-Regular.ttf \
  --bold-font /absolute/path/to/Monospace-Bold.ttf \
  --italic-font /absolute/path/to/Monospace-Italic.ttf \
  --bold-italic-font /absolute/path/to/Monospace-BoldItalic.ttf \
  --theme ocean-dark \
  --steps /tmp/demo.json --output /tmp/demo.mp4 \
  --label 'Runyte version/source revision; feature demonstrated'
```

Record an already-built binary's actual provenance. A source label is supplied
by the caller, not independently verified by the recorder.

Video defaults to **200 columns × 50 rows**, suitable for a spacious terminal
demo on a 4K display. At the default 12×26-pixel cells this produces a
2400×1300-pixel video. Override with `--columns` and `--rows` as needed; terminal
geometry is independent of the monitor's pixel resolution.

## Actions and pacing

All actions accept an optional `wait` in seconds, defaulting to 0.2 seconds.
Output continues to be collected during all waits, including readiness checks
and per-character delays. JSON uses `\u001b` for Escape, `\u001c` for Ctrl-\,
`\u0017` for Ctrl-w, and `\r` for Enter. A newline in `type` is sent as Enter.

| Action | Fields | Purpose |
| --- | --- | --- |
| `key` | `text` | Raw keys or complete escape sequences, sent together. |
| `type` | `text`, optional `interval` | Type one Unicode code point per interval (default `--typing-interval`, 0.08s). Use for prose and search queries. |
| `command` | `text`, optional `interval` | Separate Escape by 100ms, open `:`, type the command at the configured pace, press Enter. Does not leave Terminal Insert for you. |
| `paste` | `text` | Bracketed paste into the active input. |
| `expect` | `text`, optional `timeout` | Wait for literal visible text, default timeout 10s, then apply the action's `wait`. |
| `wait` | `wait` | Hold the current interaction while processing live output. |
| `checkpoint` | `name`, optional `required`, `timeout` | Verify all literal markers in `required`; record this named step's times in metadata. Adds no overlay. |
| `resize` | `columns`, `rows` | Resize the live PTY and replay screen. |
| `mouse` | `x`, `y`, optional `button`, `release` | SGR mouse, coordinates from 1; left press is button 0, release is `true`, wheel up/down 64/65. |
| `screen` | none | A normal output-processing pause; final text is saved automatically. Use screenshot interactive mode for exploratory inspection. |

Only `type` and `command` pace individual characters. Keep escape sequences
together in `key`; staggering their bytes changes how a terminal parses them.
For prose containing emoji/combining sequences, `paste` preserves the complete
input when code-point-by-code-point typing is inappropriate.

For scenes that need to inspect a real response before choosing the next keys,
import `record.py` and call `record(options, recipe, playback=controller)`.
The controller receives `(capture, play)`: inspect `capture.text()` or decoded
cells, then call `play(action)` for each actual action. The output recipe records
the actions executed, including dynamically chosen jump labels. Declare any
planned resize geometry in the input recipe so the canvas can be sized before
playback. Keep feature-specific decision logic in the scene controller.

`--opening-hold` (1s) and `--closing-hold` (2s) keep the starting and final states
visible. Set either to zero when intentionally unnecessary. `--fps` defaults to
15 (range 1–60). `--crf` defaults to 18; lower values increase H.264 quality and
file size. MP4 uses H.264, yuv420p, and faststart for broad browser playback.
There is no audio track or synthesized pointer/keystroke overlay.

The font and terminal options are shared with the
[screenshot driver](../../runyte-screenshots/references/driver.md), including
`--config`, `--arg`, `--state-dir`, cell sizes, colors, and `--cursor`.
For persistent-session demos, follow that driver's host setup/cleanup procedure.
The recorder closes its client; scene-owned persistent-session hosts need their
own `finally` cleanup. Keep fixture preparation outside the JSON; actions cannot
execute arbitrary setup scripts behind the scene.

## Timing and rendering design

The live phase answers terminal capability queries and records every PTY read
with a monotonic timestamp. Raw bytes are preserved, including UTF-8 and escape
sequences split across reads. Resize events are recorded in the same timeline.
It stores these events temporarily, bounded by `--max-trace-mib` (256 MiB), and
bounds the complete playback including setup by `--max-duration` (300s).
These can be adjusted for a longer/noisier demo. Errors abort the run rather
than silently dropping terminal output or shortening an unmet readiness check.

During offline rendering, replay setup output to reconstruct the initial state,
then sample at fixed frame timestamps. Static screens reuse a rendered image.
Video duration is the actual visible live duration rounded up to a frame;
this is a repeatable input recipe, not a guarantee of identical wall-clock
timing across asynchronous processes or machines. Changes faster than one frame
may not appear; increase the frame rate or slow the actions when needed.
Encoding speed cannot affect live playback.

MP4 dimensions stay fixed. The canvas fits the largest geometry anywhere in the
recipe, centers smaller terminal surfaces, and pads to even pixel dimensions
for yuv420p. It never stretches the font. The canvas is limited to 16 million
pixels to catch accidental enormous geometries before launching Runyte.
Resizing can expose an intermediate terminal state before Runyte's next repaint;
keep geometry fixed for demos that do not need to demonstrate resizing.

The encoder has bounded pipe-write/finalization waits and is reaped on failure.
Partial videos and raw traces are temporary. Only successfully encoded videos
are published to the requested destination. Existing output artifacts cause an
error unless `--overwrite` is supplied.

## Artifacts and review

A successful `demo.mp4` comes with:

- `demo.recording.json`: configuration, fonts, geometry, caller's source label, recipe,
  playback settings, frame count/duration, and actual action/checkpoint times.
- `demo.txt`: the final visible terminal text.
- `demo.preview.jpg`: up to six timestamped samples spanning the recording.

The JSON is a playback record, not a complete fixture archive. Preserve a small
fixture setup recipe and needed input files separately when delivering a reusable
demo. Review sidecars for personal paths, configuration, and sensitive contents.
Do not commit raw traces or rely on temporary demo directories persisting.

Verify the produced media:

```sh
ffprobe -v error -count_frames -select_streams v:0 \
  -show_entries stream=codec_name,width,height,r_frame_rate,nb_read_frames:format=duration \
  -of json /tmp/demo.mp4
ffmpeg -hide_banner -loglevel error -ss 3.5 -i /tmp/demo.mp4 -frames:v 1 /tmp/demo-transition.png
```

Use the actual checkpoint times to choose transition frames. Inspect these at
full size in addition to the contact sheet. Check legibility, colors, panes,
focus, intermediate typing, and the final state; watch the clip if playback is
available. A text marker alone cannot validate visual quality or the pacing.

Regression checks (encoding tests require local FFmpeg/FFprobe):

```sh
/tmp/runyte-video-venv/bin/python -m unittest discover -s skills/runyte-screenshots/scripts -p 'test_*.py'
/tmp/runyte-video-venv/bin/python -m unittest discover -s skills/runyte-demo-videos/scripts -p 'test_*.py'
```
