---
name: runyte-screenshots
description: Capture screenshots of Runyte features by driving the real editor in a pseudo-terminal and rendering its terminal output to PNG or WebP. Use for feature demonstrations, documentation images, and website galleries.
---

# Runyte screenshots

Use `scripts/capture.py` to launch a Runyte binary in a PTY, send input,
decode its escape sequences with pyte, and paint the resulting cells with
Pillow. These are captures of real application output; no editor API or desktop
display server is required. The helper contains no fixed scene or gallery.

## Prepare the scene

Locate the requested Runyte checkout and binary. Build it if necessary using
that checkout's instructions; record which version/revision was actually built.
Check its user guide and keymap reference for the feature's current commands.
Use that checkout's UI vocabulary for captions.

Create a temporary demo workspace with small, plausible files relevant to the
feature. Initialize it with `git init` so startup recognizes the project root;
otherwise handle the initial project-directory prompt before editor commands.
For Git views, commit the demo repository, then make the
changes to show. For LSP features, configure and run the appropriate server.
For integrated terminal features, run the real program. Keep these preparations
in a scene-specific script or recipe, separate from the reusable capture helper.

Default to an isolated demo workspace. The helper isolates XDG directories and
the persistent-session registry, passes an explicit config, and removes inherited
Runyte parent context. This is not an OS sandbox: programs can still access the
filesystem and network, and some use home-directory configuration. Use synthetic
content and avoid exposing account details in images or snapshot sidecars.
Disable LSP in scenes that do not need it; do not disable the feature being shown.

Read [references/driver.md](references/driver.md) for installation, invocation,
the action format, and interactive operation. Choose a monospace font with the
glyphs used by the scene. Pass real bold/italic variants when the scene uses them.
JetBrainsMono Nerd Font is a useful choice for Runyte's icons.

## Drive and verify

Use a JSON action sequence for predictable scenes, or `--interactive` to send
one JSON action per line and inspect the evolving screen. Both modes support raw
keys, editor commands, paste, resize, mouse input, waiting for visible text, and
multiple captures. For custom orchestration, import `Capture` from the helper
and use it as a context manager with the same action dictionaries.

Wait for feature-specific visible markers before saving. A delay alone cannot
prove that a search, syntax parse, language server, or terminal program is ready.
Inspect the saved text and open the image using the available image-viewing tool.
Check the intended feature, focus, layout, colors, glyphs, and absence of loading
states or unexpected errors. Adjust the scene and recapture when needed.

`command` sends Escape separately before opening `:` so Crossterm does not
interpret it as an Alt chord. In Terminal Insert, first send `Ctrl-\` (`\u001c`)
to return control to Runyte; Escape alone goes to the child program.

For persistent-session demonstrations, use `--state-dir` to share one temporary
runtime environment between capture clients and scene-owned hosts. Read the
persistent-session notes in the driver reference. Stop only the hosts created
for the scene before removing their runtime directory.

## Deliver

Save requested images to the requested project destination. Keep temporary
captures and debugging sidecars outside tracked development context. Preserve
a scene recipe and minimal sanitized fixtures in the project when repeatability
is requested; never leave its only copy under `/tmp` or `.runyte/`.

Record the source version, theme, cell dimensions, and font with durable images.
The helper writes text and metadata sidecars; `--label` can add a source revision
or scene description. Review these before committing them. Report that the image
was captured from terminal output and rendered, rather than a desktop screenshot.

The decoder is a practical terminal-text renderer, not a complete terminal
emulator. It does not implement terminal graphics, every extended text attribute,
or font fallback. If a requested feature depends on unsupported behavior, identify
that limitation and extend/test the helper or use an actual terminal capture;
do not fabricate missing UI in the image.
