#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Record timed Runyte PTY output, then render a silent MP4 on Linux."""

import base64
import importlib.util
import json
import math
import os
from pathlib import Path
import select
import shutil
import subprocess
import sys
import tempfile
import time

from PIL import Image, ImageDraw, ImageOps


DRIVER = Path(__file__).resolve().parents[2] / "runyte-screenshots/scripts/capture.py"
spec = importlib.util.spec_from_file_location("runyte_capture", DRIVER)
capture_driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture_driver)


def seconds(value, name, maximum=3600):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or not 0 <= value <= maximum:
        raise ValueError(f"{name} must be a finite number between 0 and {maximum}")
    return value


def recipe_from(value):
    if isinstance(value, list):
        value = {"setup": [], "actions": value}
    if not isinstance(value, dict) or set(value) - {"setup", "actions"}:
        raise ValueError("recipe must be an action array or an object with setup and actions")
    value = {"setup": value.get("setup", []), "actions": value.get("actions", [])}
    for group, steps in value.items():
        if not isinstance(steps, list):
            raise ValueError(f"{group} must be an array")
        for step in steps:
            if not isinstance(step, dict):
                raise ValueError("each action must be an object")
            op = step.get("op")
            fields = {
                "key": {"text"}, "type": {"text", "interval"},
                "command": {"text", "interval"}, "paste": {"text"},
                "expect": {"text", "timeout"}, "wait": set(),
                "screen": set(), "checkpoint": {"name", "required", "timeout"},
                "resize": {"columns", "rows"},
                "mouse": {"x", "y", "button", "release"},
            }
            if op not in fields or set(step) - fields[op] - {"op", "wait"}:
                raise ValueError(f"unknown action or fields: {step}")
            if op in ("key", "type", "command", "paste", "expect") and not isinstance(step.get("text"), str):
                raise ValueError(f"{op} requires text")
            if op in ("expect", "checkpoint"):
                markers = [step["text"]] if op == "expect" else step.get("required", [])
                if not isinstance(markers, list) or not all(isinstance(marker, str) and marker for marker in markers):
                    raise ValueError("expected markers must be nonempty strings")
            if op == "checkpoint" and not isinstance(step.get("name"), str):
                raise ValueError("checkpoint requires a name")
            if op in ("resize", "mouse"):
                for key in (("columns", "rows") if op == "resize" else ("x", "y")):
                    if type(step.get(key)) is not int or not 1 <= step[key] <= 1000:
                        raise ValueError(f"{key} must be an integer between 1 and 1000")
            if op == "mouse":
                if type(step.get("button", 0)) is not int or not 0 <= step.get("button", 0) <= 255:
                    raise ValueError("mouse button must be an integer between 0 and 255")
                if type(step.get("release", False)) is not bool:
                    raise ValueError("mouse release must be a boolean")
            for key in ("wait", "timeout", "interval"):
                if key in step:
                    seconds(step[key], key)
    if not value["actions"]:
        raise ValueError("recipe must contain at least one recorded action")
    return value


class RecordedCapture(capture_driver.Capture):
    def __init__(self, options, trace):
        self.trace = trace
        self.trace_bytes = 0
        self.origin = time.monotonic()
        self.recording = True
        super().__init__(options)

    def stamp(self):
        return time.monotonic() - self.origin

    def event(self, kind, value):
        if self.recording:
            line = json.dumps([self.stamp(), kind, value]) + "\n"
            self.trace_bytes += len(line)
            if self.trace_bytes > self.options.max_trace_mib * 1024 * 1024:
                raise RuntimeError("terminal recording exceeded --max-trace-mib")
            self.trace.write(line)

    def feed(self, data):
        self.event("output", base64.b64encode(data).decode("ascii"))
        super().feed(data)

    def resize(self, columns, rows):
        self.event("resize", [columns, rows])
        super().resize(columns, rows)

    def pump(self, duration=0.2):
        seconds(duration, "wait")
        remaining = self.options.max_duration - self.stamp()
        if remaining <= 0:
            raise RuntimeError("playback exceeded --max-duration (including setup)")
        super().pump(min(duration, remaining))
        if self.eof:
            raise RuntimeError("Runyte exited before the demo completed\n" + self.text())
        if duration > remaining:
            raise RuntimeError("playback exceeded --max-duration (including setup)")

    def paced_text(self, text, interval):
        for character in text:
            self.write("\r" if character == "\n" else character)
            self.pump(interval)

    def play(self, step):
        op = step["op"]
        if op == "type":
            self.paced_text(step["text"], step.get("interval", self.options.typing_interval))
        elif op == "command":
            self.write("\x1b")
            self.pump(0.1)
            self.write(":")
            self.paced_text(step["text"], step.get("interval", self.options.typing_interval))
            self.write("\r")
        elif op == "checkpoint":
            for marker in step.get("required", []):
                self.expect(marker, step.get("timeout", 10))
        else:
            return super().action(step)
        self.pump(step.get("wait", 0.2))

    def close(self):
        self.recording = False
        # Teardown must still work after a timeout, closed PTY, or trace limit.
        self.pump = lambda duration=0.2: capture_driver.Capture.pump(self, duration)
        super().close()


class Replay(capture_driver.Capture):
    def __init__(self, options):
        self.init_screen(options)

    def write(self, _data):
        # Terminal query replies were sent to the real process during capture.
        pass

    def apply(self, event):
        if event[1] == "output":
            self.feed(base64.b64decode(event[2]))
        elif event[1] == "resize":
            self.screen.resize(columns=event[2][0], lines=event[2][1])


def frames(options, trace, start, end, canvas_size):
    """Sample the replay at fixed timestamps; setup initializes the first frame."""
    replay = Replay(options)
    events = (json.loads(line) for line in trace)
    event = next(events, None)
    count = max(1, math.ceil((end - start) * options.fps))
    image = None
    for index in range(count):
        at = start + index / options.fps
        changed = False
        while event is not None and event[0] <= at:
            replay.apply(event)
            changed = True
            event = next(events, None)
        if changed or image is None:
            surface = replay.render_image()
            image = Image.new("RGB", canvas_size, options.background)
            image.paste(surface, ((canvas_size[0] - surface.width) // 2,
                                  (canvas_size[1] - surface.height) // 2))
        yield index, image


class Encoder:
    """Bounded writes/finalization; partial output stays in a temporary directory."""
    def __init__(self, options, destination, size):
        self.log = tempfile.TemporaryFile()
        self.process = None
        try:
            self.process = subprocess.Popen([
                options.ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-y",
                "-f", "rawvideo", "-pixel_format", "rgb24", "-video_size", f"{size[0]}x{size[1]}",
                "-framerate", str(options.fps), "-i", "pipe:0", "-an",
                "-c:v", "libx264", "-preset", "fast", "-crf", str(options.crf),
                "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(destination),
            ], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=self.log)
            os.set_blocking(self.process.stdin.fileno(), False)
        except BaseException:
            self.abort()
            raise

    def error(self):
        self.log.seek(0)
        return self.log.read(8192).decode(errors="replace")

    def write(self, image):
        remaining = memoryview(image.tobytes())
        until = time.monotonic() + 30
        fd = self.process.stdin.fileno()
        while remaining:
            if self.process.poll() is not None:
                raise RuntimeError("FFmpeg exited: " + self.error())
            timeout = until - time.monotonic()
            if timeout <= 0:
                raise RuntimeError("FFmpeg accepted no complete frame within 30 seconds")
            if select.select([], [fd], [], min(timeout, 0.1))[1]:
                try:
                    remaining = remaining[os.write(fd, remaining):]
                except BlockingIOError:
                    pass

    def finish(self):
        self.process.stdin.close()
        if self.process.wait(timeout=60):
            raise RuntimeError("FFmpeg failed: " + self.error())

    def abort(self):
        if self.process is not None:
            if self.process.stdin and not self.process.stdin.closed:
                self.process.stdin.close()
            if self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait()
        self.log.close()


def record(options, recipe, *, playback=None):
    """Run a recipe, or an adaptive playback(capture, play) scene controller."""
    destination = Path(options.output).resolve()
    if destination.suffix.lower() != ".mp4":
        raise ValueError("output must end in .mp4")
    outputs = [destination, destination.with_suffix(".recording.json"), destination.with_suffix(".txt"),
               destination.with_suffix(".preview.jpg")]
    if not options.overwrite and any(path.exists() for path in outputs):
        raise ValueError("an output artifact already exists; choose a new name or use --overwrite")
    if not shutil.which(options.ffmpeg):
        raise ValueError("FFmpeg not found; install FFmpeg with the libx264 encoder")
    columns, rows = options.columns, options.rows
    for step in recipe["setup"] + recipe["actions"]:
        if step["op"] == "resize":
            columns, rows = max(columns, step["columns"]), max(rows, step["rows"])
    # H.264 yuv420p requires even pixel dimensions. Never stretch the font.
    size = tuple(value + value % 2 for value in (columns * options.cell_width, rows * options.cell_height))
    if size[0] * size[1] > 16_000_000:
        raise ValueError("video canvas exceeds 16 million pixels")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".runyte-video-", dir=destination.parent) as scratch:
        scratch = Path(scratch)
        trace_path = scratch / "events.jsonl"
        timeline = []
        with trace_path.open("w") as trace:
            with RecordedCapture(options, trace) as capture:
                capture.pump(0.3)
                for step in recipe["setup"]:
                    capture.play(step)
                start = capture.stamp()
                print("Setup ready; recording demo actions.", flush=True)
                capture.pump(options.opening_hold)
                executed = []

                def play(step):
                    recipe_from([step])
                    if step["op"] == "resize" and (step["columns"] > columns or step["rows"] > rows):
                        raise ValueError("adaptive resize exceeds the recipe's declared canvas")
                    began = capture.stamp() - start
                    capture.play(step)
                    timeline.append({"step": len(executed), "start": began, "end": capture.stamp() - start,
                                     **({"name": step["name"]} if step["op"] == "checkpoint" else {})})
                    executed.append(dict(step))

                if playback is None:
                    for step in recipe["actions"]:
                        play(step)
                else:
                    playback(capture, play)
                recipe = {"setup": recipe["setup"], "actions": executed}
                capture.pump(options.closing_hold)
                end = capture.stamp()
                metadata = capture.metadata()
                final_text = capture.text()
        count = max(1, math.ceil((end - start) * options.fps))
        print(f"Encoding {count} frames ({count / options.fps:.1f}s) at {options.fps} fps.", flush=True)
        preview = Image.new("RGB", (960, 3 * 294), "#202020")
        preview_indices = sorted(set(round(i * (count - 1) / 5) for i in range(6)))
        encoder = Encoder(options, scratch / "video.mp4", size)
        try:
            with trace_path.open() as trace:
                for index, image in frames(options, trace, start, end, size):
                    encoder.write(image)
                    if index in preview_indices:
                        slot = preview_indices.index(index)
                        x, y = (slot % 2) * 480, (slot // 2) * 294
                        tile = ImageOps.contain(image, (480, 270))
                        preview.paste(tile, (x + (480 - tile.width) // 2, y))
                        ImageDraw.Draw(preview).text((x + 8, y + 274), f"{index / options.fps:.2f}s", fill="white")
                    if index and index % (options.fps * 10) == 0:
                        print(f"Encoded {index}/{count} frames.", flush=True)
            encoder.finish()
        finally:
            encoder.abort()
        metadata.update({
            "format": "runyte-demo-video-v1", "fps": options.fps, "frames": count,
            "duration": count / options.fps, "live_duration": end - start,
            "initial_columns": options.columns, "initial_rows": options.rows,
            "canvas_pixels": size, "codec": "h264", "pixel_format": "yuv420p", "crf": options.crf,
            "typing_interval": options.typing_interval, "opening_hold": options.opening_hold,
            "closing_hold": options.closing_hold, "recipe": recipe, "timeline": timeline,
        })
        (scratch / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
        (scratch / "final.txt").write_text(final_text + "\n")
        preview.save(scratch / "preview.jpg", quality=92)
        for source, target in zip(("video.mp4", "metadata.json", "final.txt", "preview.jpg"), outputs):
            if options.overwrite:
                os.replace(scratch / source, target)
            else:
                # Same filesystem; fail if another writer claimed the name meanwhile.
                os.link(scratch / source, target)
        return {"video": str(destination), "preview": str(outputs[-1]), "duration": count / options.fps}


def parser():
    result = capture_driver.parser(modes=False)
    result.set_defaults(columns=200, rows=50)
    result.description = __doc__
    result.add_argument("--steps", required=True, help="JSON action array or setup/actions object")
    result.add_argument("--output", required=True, help="Destination MP4")
    result.add_argument("--fps", type=int, default=15)
    result.add_argument("--crf", type=int, default=18, help="H.264 quality; lower is better")
    result.add_argument("--typing-interval", type=float, default=0.08)
    result.add_argument("--opening-hold", type=float, default=1)
    result.add_argument("--closing-hold", type=float, default=2)
    result.add_argument("--max-duration", type=float, default=300, help="Maximum live seconds, including setup")
    result.add_argument("--max-trace-mib", type=int, default=256)
    result.add_argument("--ffmpeg", default="ffmpeg")
    result.add_argument("--overwrite", action="store_true")
    return result


def main():
    if not sys.platform.startswith("linux"):
        raise SystemExit("This recorder currently supports Linux only.")
    options = parser().parse_args()
    for field in ("columns", "rows", "font_size", "cell_width", "cell_height"):
        if not 1 <= getattr(options, field) <= 1000:
            raise ValueError(f"{field} must be between 1 and 1000")
    if not 1 <= options.fps <= 60 or not 0 <= options.crf <= 51 or options.max_trace_mib <= 0:
        raise ValueError("fps must be 1–60, crf 0–51, and max-trace-mib positive")
    for field in ("typing_interval", "opening_hold", "closing_hold", "max_duration"):
        seconds(getattr(options, field), field)
    recipe = recipe_from(json.loads(Path(options.steps).read_text()))
    print(json.dumps(record(options, recipe)), flush=True)


if __name__ == "__main__":
    main()
