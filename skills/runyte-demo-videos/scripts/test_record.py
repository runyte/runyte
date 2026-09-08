# SPDX-License-Identifier: MPL-2.0
"""Exercise recording timing, replay, encoding, and failed-run cleanup."""

import base64
import io
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from PIL import Image, ImageFont

from record import Encoder, RecordedCapture, frames, parser, recipe_from, record


FONT = ImageFont.load_default()


def options(directory):
    return parser().parse_args([
        "--binary", "/bin/false", "--cwd", directory, "--font", "/unused/font.ttf",
        "--steps", "/unused/steps.json", "--output", str(Path(directory) / "demo.mp4"),
        "--columns", "4", "--rows", "2", "--cell-width", "8", "--cell-height", "12",
        "--fps", "4", "--opening-hold", "0", "--closing-hold", "0",
    ])


def event(at, output):
    return [at, "output", base64.b64encode(output).decode("ascii")]


class ReplayTests(unittest.TestCase):
    def test_setup_trim_fixed_frame_timing_and_resize_padding(self):
        with tempfile.TemporaryDirectory() as directory:
            opts = options(directory)
            trace = [
                event(0, b"\x1b[41m\x1b[H    \r\n    "),
                event(1.5, b"\x1b[44m\x1b[H    \r\n    "),
                [1.75, "resize", [2, 1]],
            ]
            with patch("PIL.ImageFont.truetype", return_value=FONT):
                result = list(frames(opts, map(json.dumps, trace), 1, 2, (32, 24)))
            self.assertEqual(len(result), 4)
            self.assertEqual(result[0][1].getpixel((0, 0)), (205, 0, 0))
            self.assertIs(result[0][1], result[1][1])  # Static frame reuses its image.
            self.assertEqual(result[2][1].getpixel((0, 0)), (0, 0, 238))
            self.assertEqual(result[3][1].size, (32, 24))
            self.assertEqual(result[3][1].getpixel((0, 0)), (21, 38, 48))
            self.assertEqual(result[3][1].getpixel((8, 6)), (0, 0, 238))

    def test_split_output_replay_matches_unsplit_utf8_and_attributes(self):
        data = "\x1b[32mé\x1b[7mX".encode()
        with tempfile.TemporaryDirectory() as directory:
            opts = options(directory)
            with patch("PIL.ImageFont.truetype", return_value=FONT):
                whole = list(frames(opts, [json.dumps(event(0, data))], 0.5, 1, (32, 24)))
                chunks = [json.dumps(event(i / 100, bytes([byte]))) for i, byte in enumerate(data)]
                split = list(frames(opts, chunks, 0.5, 1, (32, 24)))
            self.assertEqual(whole[0][1].tobytes(), split[0][1].tobytes())

    def test_paced_typing_waits_between_keys_and_keeps_escape_separate(self):
        capture = RecordedCapture.__new__(RecordedCapture)
        capture.options = options("/tmp")
        events = []
        capture.write = lambda data: events.append(("key", data))
        capture.pump = lambda duration: events.append(("wait", duration))
        capture.play({"op": "command", "text": "open", "interval": 0.1, "wait": 1})
        self.assertEqual(events[:3], [("key", "\x1b"), ("wait", 0.1), ("key", ":")])
        self.assertEqual("".join(value for kind, value in events if kind == "key"), "\x1b:open\r")
        self.assertAlmostEqual(sum(value for kind, value in events if kind == "wait"), 1.5)

    def test_recipe_rejects_typos_invalid_markers_and_nonfinite_timing(self):
        for step in (
            {"op": "type", "text": "hello", "intervl": 0.2},
            {"op": "wait", "wait": float("nan")},
            {"op": "wait", "wait": -1},
            {"op": "expect", "text": ""},
            {"op": "resize", "columns": 0, "rows": 20},
            {"op": "checkpoint", "name": "bad", "required": "not an array"},
        ):
            with self.subTest(step=step), self.assertRaises(ValueError):
                recipe_from([step])

    def test_trace_limit_raises_instead_of_dropping_output(self):
        capture = RecordedCapture.__new__(RecordedCapture)
        capture.options = options("/tmp")
        capture.options.max_trace_mib = 0
        capture.recording = True
        capture.trace = io.StringIO()
        capture.trace_bytes = 0
        capture.stamp = lambda: 0
        with self.assertRaisesRegex(RuntimeError, "max-trace-mib"):
            capture.event("output", "data")
        self.assertEqual(capture.trace.getvalue(), "")


@unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "requires FFmpeg and FFprobe")
class EncoderTests(unittest.TestCase):
    def test_encoded_video_frame_count_duration_and_changing_content(self):
        with tempfile.TemporaryDirectory() as directory:
            opts = options(directory)
            destination = Path(opts.output)
            encoder = Encoder(opts, destination, (32, 24))
            try:
                for color in ("red", "red", "blue", "blue"):
                    encoder.write(Image.new("RGB", (32, 24), color))
                encoder.finish()
            finally:
                encoder.abort()
            info = json.loads(subprocess.check_output([
                "ffprobe", "-v", "error", "-count_frames", "-show_streams", "-of", "json", str(destination),
            ]))["streams"][0]
            self.assertEqual(info["codec_name"], "h264")
            self.assertEqual(info["pix_fmt"], "yuv420p")
            self.assertEqual(info["nb_read_frames"], "4")
            self.assertAlmostEqual(float(info["duration"]), 1)
            raw = subprocess.check_output([
                "ffmpeg", "-v", "error", "-i", str(destination), "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1",
            ])
            self.assertGreater(raw[0], 240)
            self.assertLess(raw[2], 10)
            last = 3 * 32 * 24 * 3
            self.assertLess(raw[last], 10)
            self.assertGreater(raw[last + 2], 240)
            self.assertIsNotNone(encoder.process.returncode)

    def test_early_child_exit_leaves_no_published_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            opts = options(directory)
            with patch("PIL.ImageFont.truetype", return_value=FONT):
                with self.assertRaisesRegex(RuntimeError, "Runyte exited"):
                    record(opts, recipe_from([{"op": "wait", "wait": 1}]))
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_existing_output_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            opts = options(directory)
            Path(opts.output).write_bytes(b"existing video")
            with self.assertRaisesRegex(ValueError, "already exists"):
                record(opts, recipe_from([{"op": "wait", "wait": 1}]))
            self.assertEqual(Path(opts.output).read_bytes(), b"existing video")

    def test_encoder_failure_is_reported_and_reaped(self):
        with tempfile.TemporaryDirectory() as directory:
            opts = options(directory)
            opts.ffmpeg = "/bin/false"
            encoder = Encoder(opts, Path(opts.output), (32, 24))
            try:
                with self.assertRaises(RuntimeError):
                    encoder.finish()
            finally:
                encoder.abort()
            self.assertIsNotNone(encoder.process.returncode)


if __name__ == "__main__":
    unittest.main()
