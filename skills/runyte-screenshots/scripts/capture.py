#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Drive a real Runyte PTY and render its visible cells. Unix only."""

import argparse
import codecs
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import sys
import tempfile
import termios
import time

import pyte
from PIL import Image, ImageColor, ImageDraw, ImageFont


ESCAPE = re.compile(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|P[^\x1b]*(?:\x1b\\)|(?![\[\]P])[ -/]*[0-~])")
ANSI = dict(zip(
    ("black", "red", "green", "brown", "blue", "magenta", "cyan", "white",
     "brightblack", "brightred", "brightgreen", "brightbrown", "brightblue",
     "brightmagenta", "brightcyan", "brightwhite"),
    ("000000", "cd0000", "00cd00", "cdcd00", "0000ee", "cd00cd", "00cdcd", "e5e5e5",
     "7f7f7f", "ff0000", "00ff00", "ffff00", "5c5cff", "ff00ff", "00ffff", "ffffff"),
))
STATE_KEYS = ("XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME",
              "XDG_STATE_HOME", "XDG_RUNTIME_DIR", "RUNYTE_ALL_HOSTS_DIR")


def isolated_env(root):
    env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor")
    for key in STATE_KEYS:
        directory = Path(root).resolve() / key.lower()
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        directory.chmod(0o700)
        env[key] = str(directory)
    for key in ("NO_COLOR", "RUNYTE_PARENT_CONTEXT"):
        env.pop(key, None)
    return env


class Capture:
    def __init__(self, options):
        self.init_screen(options)
        self.temporary = tempfile.TemporaryDirectory(prefix="runyte-capture-")
        self.env = isolated_env(options.state_dir or self.temporary.name)
        config = options.config
        if not config:
            config = Path(self.temporary.name) / "config.yaml"
            config.write_text("theme: " + json.dumps(options.theme) + "\nlsp:\n  enable: false\n")
        self.argv = [str(Path(options.binary).resolve()), "-c", str(Path(config).resolve()), *options.arg]
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            try:
                self._size(0, options.columns, options.rows)
                os.chdir(options.cwd)
                os.execve(self.argv[0], self.argv, self.env)
            except BaseException as error:
                os.write(2, f"capture launch failed: {error}\n".encode())
                os._exit(127)

    def init_screen(self, options):
        """Initialize the decoder/renderer independently of a live PTY."""
        self.options = options
        self.fonts = {}
        for style in ("font", "bold_font", "italic_font", "bold_italic_font"):
            path = getattr(options, style) or options.font
            self.fonts[style] = ImageFont.truetype(path, options.font_size)
        self.screen = pyte.Screen(options.columns, options.rows)
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.pending = ""
        self.eof = False

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    @staticmethod
    def _size(fd, columns, rows):
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))

    def text(self):
        return "\n".join(self.screen.display)

    def resize(self, columns, rows):
        if not (1 <= columns <= 1000 and 1 <= rows <= 1000):
            raise ValueError("geometry must be between 1 and 1000 cells")
        self.screen.resize(lines=rows, columns=columns)
        self._size(self.fd, columns, rows)

    def write(self, data):
        data = memoryview(data.encode() if isinstance(data, str) else data)
        while data:
            count = os.write(self.fd, data)
            data = data[count:]

    def _reply(self, sequence):
        # Parse complete sequences so queries split across PTY reads still work.
        replies = {
            "\x1b[c": "\x1b[?62;1;2;6;9;15;22c",
            "\x1b[0c": "\x1b[?62;1;2;6;9;15;22c",
            "\x1b[>c": "\x1b[>0;276;0c",
            "\x1b[>0c": "\x1b[>0;276;0c",
            "\x1b[?u": "\x1b[?0u",
            "\x1b[5n": "\x1b[0n",
            "\x1b[6n": f"\x1b[{self.screen.cursor.y + 1};{self.screen.cursor.x + 1}R",
            "\x1b[>0q": "\x1bP>|xterm(390)\x1b\\",
        }
        if sequence in replies:
            return replies[sequence]
        mode = re.fullmatch(r"\x1b\[\?(\d+)\$p", sequence)
        if mode:
            return f"\x1b[?{mode[1]};2$y"
        color = re.fullmatch(r"\x1b\](10|11);\?(?:\x07|\x1b\\)", sequence)
        if color:
            value = self.options.foreground if color[1] == "10" else self.options.background
            rgb = "/".join(f"{channel * 257:04x}" for channel in ImageColor.getrgb(value))
            return f"\x1b]{color[1]};rgb:{rgb}\x1b\\"
        if sequence.startswith("\x1bP$q"):
            return "\x1bP0$r\x1b\\"
        return ""

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
            index = self.pending.find("\x1b")
            if index == -1:
                self.stream.feed(self.pending)
                self.pending = ""
            elif index:
                self.stream.feed(self.pending[:index])
                self.pending = self.pending[index:]
            else:
                match = ESCAPE.match(self.pending)
                if not match:
                    if len(self.pending) > 65536:
                        raise ValueError("unterminated or unsupported terminal escape sequence")
                    break
                sequence = match[0]
                self.pending = self.pending[len(sequence):]
                reply = self._reply(sequence)
                if reply:
                    self.write(reply)
                elif not sequence.startswith("\x1bP"):
                    self.stream.feed(sequence)

    def pump(self, seconds=0.2):
        if seconds < 0:
            raise ValueError("wait must be non-negative")
        until = time.monotonic() + seconds
        while not self.eof and time.monotonic() < until:
            if not select.select([self.fd], [], [], min(0.05, max(0, until - time.monotonic())))[0]:
                continue
            try:
                data = os.read(self.fd, 65536)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                data = b""
            if not data:
                self.eof = True
                break
            self.feed(data)

    def expect(self, text, timeout=10):
        until = time.monotonic() + timeout
        while text not in self.text():
            if self.eof or time.monotonic() >= until:
                raise RuntimeError(f"screen did not contain {text!r}\n{self.text()}")
            self.pump(0.05)

    def action(self, step):
        op = step["op"]
        if op == "key":
            self.write(step["text"])
        elif op == "command":
            self.write("\x1b")
            self.pump(0.1)
            self.write(":" + step["text"] + "\r")
        elif op == "paste":
            self.write("\x1b[200~" + step["text"] + "\x1b[201~")
        elif op == "resize":
            columns, rows = int(step["columns"]), int(step["rows"])
            self.resize(columns, rows)
        elif op == "mouse":
            ending = "m" if step.get("release") else "M"
            self.write(f"\x1b[<{int(step.get('button', 0))};{int(step['x'])};{int(step['y'])}{ending}")
        elif op == "expect":
            self.expect(step["text"], step.get("timeout", 10))
        elif op == "save":
            for marker in step.get("required", []):
                self.expect(marker, step.get("timeout", 10))
            return self.save(step["path"])
        elif op not in ("wait", "screen"):
            raise ValueError(f"unknown action: {op}")
        self.pump(step.get("wait", 0.2))
        return {"screen": self.text(), "eof": self.eof}

    def render_image(self):
        """Render the current screen without writing screenshot artifacts."""
        options = self.options
        width, height = options.cell_width, options.cell_height
        image = Image.new("RGB", (self.screen.columns * width, self.screen.lines * height))
        draw = ImageDraw.Draw(image)

        def rgb(value, default):
            return default if value == "default" else "#" + ANSI.get(value, value)

        # Paint all backgrounds first, so wide/italic glyphs survive the next cell.
        for y in range(self.screen.lines):
            for x in range(self.screen.columns):
                cell = self.screen.buffer[y][x]
                fg, bg = rgb(cell.fg, options.foreground), rgb(cell.bg, options.background)
                if cell.reverse:
                    fg, bg = bg, fg
                draw.rectangle((x * width, y * height, (x + 1) * width - 1,
                                (y + 1) * height - 1), fill=bg)
        for y in range(self.screen.lines):
            for x in range(self.screen.columns):
                cell = self.screen.buffer[y][x]
                fg = rgb(cell.bg, options.background) if cell.reverse else rgb(cell.fg, options.foreground)
                style = ("bold_italic_font" if cell.bold else "italic_font") if cell.italics else ("bold_font" if cell.bold else "font")
                px, py = x * width, y * height
                if cell.data.strip():
                    draw.text((px, py + options.text_offset), cell.data, font=self.fonts[style], fill=fg)
                if cell.underscore:
                    draw.line((px, py + height - 3, px + width - 1, py + height - 3), fill=fg)
                if cell.strikethrough:
                    draw.line((px, py + height // 2, px + width - 1, py + height // 2), fill=fg)
        cursor = self.screen.cursor
        if options.cursor and not cursor.hidden:
            px, py = cursor.x * width, cursor.y * height
            draw.rectangle((px, py, px + width - 1, py + height - 1), outline=options.foreground)
        return image

    def metadata(self):
        options = self.options
        return {
            "argv": self.argv, "cwd": str(Path(options.cwd).resolve()),
            "columns": self.screen.columns, "rows": self.screen.lines,
            "pixels": (self.screen.columns * options.cell_width, self.screen.lines * options.cell_height),
            "theme": options.theme if not options.config else "see config",
            "config": Path(self.argv[2]).read_text(),
            "fonts": {key: getattr(options, key) for key in self.fonts},
            "font_size": options.font_size, "cell_width": options.cell_width,
            "cell_height": options.cell_height, "text_offset": options.text_offset,
            "foreground": options.foreground, "background": options.background,
            "label": options.label, "cursor": "outline" if options.cursor else "omitted",
        }

    def save(self, destination):
        if self.eof:
            raise RuntimeError("Runyte exited; refusing to capture a dead PTY")
        path = Path(destination).resolve()
        if path.suffix.lower() not in (".png", ".webp"):
            raise ValueError("capture destination must end in .png or .webp")
        image = self.render_image()
        path.parent.mkdir(parents=True, exist_ok=True)
        image.save(path, **({"lossless": True, "method": 6} if path.suffix.lower() == ".webp" else {}))
        path.with_suffix(".txt").write_text(self.text() + "\n")
        path.with_suffix(".json").write_text(json.dumps(self.metadata(), indent=2) + "\n")
        return {"image": str(path), "text": str(path.with_suffix('.txt'))}

    def close(self):
        # Retain ownership until waitpid reaps the child; never signal a reused PID.
        try:
            reaped, _ = os.waitpid(self.pid, os.WNOHANG)
            if not reaped:
                os.kill(self.pid, signal.SIGTERM)
                until = time.monotonic() + 2
                while time.monotonic() < until:
                    self.pump(0.05)
                    reaped, _ = os.waitpid(self.pid, os.WNOHANG)
                    if reaped:
                        break
                if not reaped:
                    os.kill(self.pid, signal.SIGKILL)
                    os.waitpid(self.pid, 0)
        except (ChildProcessError, ProcessLookupError):
            pass
        finally:
            os.close(self.fd)
            self.temporary.cleanup()


def parser(*, modes=True):
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--binary", required=True, help="Path to a built Runyte executable")
    result.add_argument("--cwd", required=True, help="Demo workspace directory")
    result.add_argument("--font", required=True, help="Monospace TTF/OTF file")
    for flag in ("bold-font", "italic-font", "bold-italic-font", "config", "state-dir"):
        result.add_argument("--" + flag)
    result.add_argument("--theme", default="ocean-dark")
    result.add_argument("--arg", action="append", default=[], help="Runyte argument; use --arg=--flag")
    for flag, default in (("columns", 160), ("rows", 58), ("font-size", 20),
                          ("cell-width", 12), ("cell-height", 26)):
        result.add_argument("--" + flag, type=int, default=default)
    result.add_argument("--text-offset", type=int, default=0)
    result.add_argument("--foreground", default="#c0cbd3")
    result.add_argument("--background", default="#152630")
    result.add_argument("--cursor", action="store_true", help="Draw visible terminal cursor as an outline")
    result.add_argument("--label", default="", help="Source revision or scene provenance")
    if modes:
        mode = result.add_mutually_exclusive_group(required=True)
        mode.add_argument("--steps", help="JSON file containing an array of actions")
        mode.add_argument("--interactive", action="store_true", help="Read JSON actions, one per stdin line")
    return result


def main():
    options = parser().parse_args()
    for field in ("columns", "rows", "font_size", "cell_width", "cell_height"):
        if not 1 <= getattr(options, field) <= 1000:
            raise ValueError(f"{field} must be between 1 and 1000")
    with Capture(options) as capture:
        capture.pump(0.3)
        if options.steps:
            for step in json.loads(Path(options.steps).read_text()):
                result = capture.action(step)
                if step["op"] in ("screen", "save"):
                    print(json.dumps(result), flush=True)
        else:
            print(json.dumps({"ready": True}), flush=True)
            # Read bytes directly: buffered readline can hide queued lines from select.
            pending = b""
            while True:
                capture.pump(0.05)
                if not select.select([sys.stdin], [], [], 0.05)[0]:
                    continue
                data = os.read(sys.stdin.fileno(), 65536)
                if not data:
                    if pending.strip():
                        print(json.dumps(capture.action(json.loads(pending))), flush=True)
                    break
                pending += data
                while b"\n" in pending:
                    line, pending = pending.split(b"\n", 1)
                    if not line.strip():
                        continue
                    try:
                        step = json.loads(line)
                        if step["op"] == "quit":
                            return
                        print(json.dumps(capture.action(step)), flush=True)
                    except (ValueError, KeyError, RuntimeError, OSError) as error:
                        print(json.dumps({"error": str(error)}), flush=True)


if __name__ == "__main__":
    main()
