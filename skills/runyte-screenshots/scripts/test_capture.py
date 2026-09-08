# SPDX-License-Identifier: MPL-2.0
"""Behavior checks for stream reconstruction and screenshot artifacts."""

import codecs
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import pyte
from PIL import Image, ImageFont

from capture import Capture, isolated_env, parser


class CaptureTests(unittest.TestCase):
    def screen(self):
        capture = Capture.__new__(Capture)
        capture.options = parser().parse_args([
            "--binary", "/demo/runyte", "--cwd", "/demo/workspace",
            "--font", "demo.ttf", "--interactive",
        ])
        capture.screen = pyte.Screen(12, 4)
        capture.stream = pyte.Stream(capture.screen)
        capture.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        capture.pending = ""
        capture.eof = False
        capture.replies = []
        capture.write = capture.replies.append
        return capture

    def test_fragmented_queries_and_utf8_preserve_screen(self):
        capture = self.screen()
        data = '界e\u0301\x1b[2;3H\x1b[6n\x1b[?u\x1b]11;?\x1b\\\x1b[38;2;18;52;86mX'.encode()
        for byte in data:
            capture.feed(bytes([byte]))
        self.assertEqual(capture.screen.buffer[0][0].data, "界")
        self.assertEqual(capture.screen.buffer[0][2].data, "é")
        self.assertEqual(capture.screen.buffer[1][2].data, "X")
        self.assertEqual(capture.screen.buffer[1][2].fg, "123456")
        self.assertEqual(capture.replies, [
            "\x1b[2;3R", "\x1b[?0u", "\x1b]11;rgb:1515/2626/3030\x1b\\",
        ])

    def test_missing_marker_fails_after_child_exit(self):
        capture = self.screen()
        capture.eof = True
        with self.assertRaisesRegex(RuntimeError, "did not contain"):
            capture.expect("never appeared")
        with self.assertRaisesRegex(RuntimeError, "dead PTY"):
            capture.save("unused.png")

    def test_images_match_cell_colors_and_have_sidecars(self):
        capture = self.screen()
        capture.fonts = dict.fromkeys(
            ("font", "bold_font", "italic_font", "bold_italic_font"), ImageFont.load_default()
        )
        capture.feed(b"\x1b[48;2;18;52;86m \x1b[38;2;101;67;33;7m ")
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "config.yaml"
            config.write_text("theme: ocean-dark\n")
            capture.argv = ["/demo/runyte", "-c", str(config)]
            for extension in ("png", "webp"):
                destination = Path(directory) / ("shot." + extension)
                capture.save(destination)
                with Image.open(destination) as image:
                    self.assertEqual(image.size, (144, 104))
                    self.assertEqual(image.getpixel((0, 0)), (18, 52, 86))
                    self.assertEqual(image.getpixel((12, 0)), (101, 67, 33))
                self.assertEqual(destination.with_suffix(".txt").read_text(), capture.text() + "\n")
                self.assertEqual(json.loads(destination.with_suffix(".json").read_text())["columns"], 12)

    def test_isolated_environment_does_not_inherit_attachment(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.dict("os.environ", {"RUNYTE_PARENT_CONTEXT": "private", "NO_COLOR": "1"}):
                env = isolated_env(directory)
            self.assertNotIn("RUNYTE_PARENT_CONTEXT", env)
            self.assertNotIn("NO_COLOR", env)
            root = Path(env["RUNYTE_ALL_HOSTS_DIR"])
            self.assertTrue(root.is_relative_to(directory))
            self.assertEqual(root.stat().st_mode & 0o777, 0o700)


if __name__ == "__main__":
    unittest.main()
