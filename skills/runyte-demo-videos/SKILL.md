---
name: runyte-demo-videos
description: Design and record scripted videos of real Runyte features on Linux from a user's demo description. Prepare demo fixtures, play timed editor actions in a PTY, and produce a reviewed MP4 with a repeatable action recipe.
---

# Runyte demo videos

Turn the requested demonstration into a small scene and an action sequence,
then use `scripts/record.py` to record a real Runyte process. This is a silent
video rendered from terminal output; no desktop, display server, editor API,
or screen recorder is needed. Linux and FFmpeg with `libx264` are required.

The recorder shares its PTY, decoder, and font renderer with the adjacent
`runyte-screenshots` skill. Keep both skill directories together under `skills/`.
Read [references/recording.md](references/recording.md) for installation,
the recipe format, timing behavior, and validation commands.

## Design the demo

Translate the user's description into visible steps: establish the starting
state, perform the feature's actions at readable speed, and hold the result.
Choose concise, plausible example content that makes each action understandable.
Use the target checkout's guide and keymap reference to confirm commands and
its UI vocabulary when naming the steps. Do not assume Helix spellings.

Make reasonable choices for pacing, theme, and geometry when unspecified.
Use the default 15 fps and 200 columns × 50 rows unless the user requests another
terminal size. Match cell dimensions to the chosen
monospace font and supply actual bold/italic variants. Prefer short sequences
with visible pauses over racing through every related command.

Create isolated temporary project fixtures and initialize a Git repository so
Runyte recognizes the root. Commit a baseline when demonstrating Git changes.
Set up real language servers or terminal programs when they are part of the
feature. Default configuration disables LSP; provide `--config` when needed.
Use the [screenshot skill's](../runyte-screenshots/SKILL.md) isolation and persistent-session guidance for those
scenes. Runtime isolation is not an OS sandbox; commands still have their normal
filesystem/network access, and some programs read home-directory configuration.

Write the recipe as JSON with `setup` and `actions`. Use setup for opening files,
arranging panes, and waiting for readiness before recording starts. The actions
are what the viewer sees. Add feature-specific `expect` or `checkpoint` markers
to prove the requested state appeared, and reading pauses after meaningful changes.
Use `type` for visible typing, `key` for shortcuts, and `command` for commands
typed into `:`. In Terminal Insert, send `Ctrl-\` (`\u001c`) before an editor
command; Escape belongs to the child program there.

## Record and review

Run the recorder with the prepared workspace, built binary, fonts, recipe, and
requested output path. Live playback records timestamped terminal output while
answering terminal queries. Video encoding happens afterward, so slow font
rendering cannot change the typing pace or editor response time. Setup output
initializes the first frame but is excluded from the visible video.

Inspect the preview contact sheet, final text, and timestamped action/checkpoint
metadata. Use FFprobe to verify codec, dimensions, duration, and frame count;
extract frames around important transitions and inspect them with an image tool.
When a video viewer is available, watch the whole clip to judge pacing. A contact
sheet alone does not establish that every intermediate frame is correct.

When the same text appears in multiple panes, verify the intended destination
pane. A whole-screen text match does not prove that a copy, paste, or focus
change succeeded.

If the demo misses a state, clips text, shows loading/errors unexpectedly, or
moves too quickly, adjust fixtures or actions and record again. Use a new output
name or `--overwrite` intentionally. Failed playback or encoding exits nonzero
and does not publish its temporary recording as a completed demo.

## Deliver

Return the MP4 and keep the action recipe with a minimal fixture setup recipe
where the user can reuse them. The output JSON embeds actions and timing, but
does not recreate demo files, installed programs, or persistent-session hosts.
Preserve those inputs alongside a reusable demo when requested; do not rely on
temporary paths or `.runyte/` images surviving.

Record which binary/source revision was used with `--label`, plus the scene's
theme and font choices. Inspect paths, configuration, terminal contents, and
sidecars before tracking or publishing them. The recorder discards raw PTY traces
after encoding. Keep verification artifacts out of tracked development context.

The shared renderer has the screenshot skill's terminal-text limitations:
no terminal graphics, complete extended-attribute emulation, or automatic font
fallback. Mouse events reach Runyte, but the video does not draw a desktop mouse
pointer. Hardware cursor outlines are optional; real editor selection/caret
cells are captured as drawn. Do not invent a missing interface or imply that
the video came from a desktop capture.
