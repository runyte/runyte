# SPDX-License-Identifier: MPL-2.0

import unittest
from pathlib import Path
from unittest import mock

import editions


class CompositorEnvironment(unittest.TestCase):
    def test_failed_window_spawn_cleans_its_temporary_directory(self):
        display = mock.Mock()
        display.environment.return_value = {}
        with mock.patch.object(editions.subprocess, 'Popen', side_effect=FileNotFoundError('missing binary')) as launch:
            with self.assertRaisesRegex(RuntimeError, 'missing binary'):
                editions.DesktopSession(display, '/missing', {}, Path('/fixture/main.txt'), 'ide')
        log = launch.call_args.kwargs['stdout']
        self.assertTrue(log.closed)
        self.assertFalse(Path(log.name).parent.exists())

    def test_editor_cannot_reintroduce_ambient_wayland_after_environment_merge(self):
        display = editions.Display.__new__(editions.Display)
        display.number, display.runtime = 42, '/fixture-runtime'
        env = display.environment({'WAYLAND_DISPLAY': 'inherited', 'XAUTHORITY': '/outside-auth',
                                   'XDG_SESSION_TYPE': 'wayland', 'DISPLAY': ':99'})
        self.assertEqual(env['DISPLAY'], ':42')
        for name in ('WAYLAND_DISPLAY', 'XDG_SESSION_TYPE', 'XAUTHORITY'):
            self.assertNotIn(name, env)

    def test_compositor_storage_is_isolated_and_ambient_bus_removed(self):
        with mock.patch.dict(editions.os.environ, {
            "HOME": "/outside-home", "XDG_CONFIG_HOME": "/outside-config",
            "DBUS_SESSION_BUS_ADDRESS": "unix:path=/outside-bus",
            "DISPLAY": ":99", "WAYLAND_DISPLAY": "outside-wayland",
            "XAUTHORITY": "/outside-auth",
        }), mock.patch.object(editions.subprocess, "Popen") as launch, \
                mock.patch.object(editions.Display, "wait_for_socket", side_effect=RuntimeError("stop")):
            with self.assertRaisesRegex(RuntimeError, "stop"):
                editions.Display()
        env = launch.call_args.kwargs["env"]
        root = Path(env["XDG_RUNTIME_DIR"])
        for name in ("HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME"):
            self.assertTrue(Path(env[name]).is_relative_to(root), name)
        for name in ("DBUS_SESSION_BUS_ADDRESS", "DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY"):
            self.assertNotIn(name, env)
        self.assertFalse(root.exists())


def pixels(width, height, ink):
    """32-bit pixels with green-channel ink at the given (x, y) positions."""
    data = bytearray(b"\x10\x18\x18\xff" * (width * height))
    for x, y in ink:
        data[(y * width + x) * 4 + 1] = 0xd0
    return bytes(data)


class LaunchArguments(unittest.TestCase):
    def test_file_modes_name_the_document(self):
        self.assertEqual(editions.launch_arguments("terminal", "editor", "a.lua"), ["--editor", "a.lua"])
        self.assertEqual(editions.launch_arguments("desktop", "ide", "a.lua"), ["--window", "--ide", "a.lua"])

    def test_mux_takes_a_workspace_rather_than_a_file(self):
        self.assertEqual(editions.launch_arguments("terminal", "mux-cold", "a.lua"), ["--mux"])
        self.assertEqual(editions.launch_arguments("desktop", "mux-warm", "a.lua"), ["--window", "--mux"])


class ShapeComparison(unittest.TestCase):
    def test_a_recolored_frame_is_the_same_text(self):
        ink = [(x, 3) for x in range(40)]
        plain = pixels(60, 8, ink)
        recolored = bytearray(plain)
        for x, y in ink:
            recolored[(y * 60 + x) * 4 + 1] = 0x90
        self.assertTrue(editions.same_shape(editions.mask(plain), editions.mask(bytes(recolored))))

    def test_shifted_text_differs(self):
        glyphs = [(x, 3) for x in range(0, 60, 4)]
        moved = [(x + 2, 3) for x in range(0, 56, 4)]
        self.assertFalse(editions.same_shape(editions.mask(pixels(60, 8, glyphs)),
                                             editions.mask(pixels(60, 8, moved))))

    def test_a_blank_frame_matches_nothing(self):
        blank = editions.mask(pixels(10, 2, []))
        self.assertFalse(editions.same_shape(blank, blank))


class EditedBand(unittest.TestCase):
    def test_band_starts_past_the_caret_on_the_changed_row(self):
        width, height = editions.WINDOW_SIZE
        before = pixels(width, height, [(x, 45) for x in range(90, 300, 3)])
        after = pixels(width, height, [(x + 9, 45) for x in range(90, 300, 3)])
        left, top, band_width, band_height = editions.edited_band(before, after, width * 4, height)
        self.assertEqual((top, band_height), (40, editions.CELL_HEIGHT))
        self.assertEqual(left, 90 + editions.CARET_CELLS * editions.CELL_WIDTH)
        self.assertLessEqual(band_width, editions.BAND_CELLS * editions.CELL_WIDTH)

    def test_unchanged_frames_are_rejected(self):
        width, height = editions.WINDOW_SIZE
        frame = pixels(width, height, [])
        with self.assertRaises(ValueError):
            editions.edited_band(frame, frame, width * 4, height)


class Report(unittest.TestCase):
    def test_incomplete_cells_say_so(self):
        self.assertEqual(editions.summary([1.0], 2), "incomplete (1/2)")

    def test_failed_detaches_are_reported_not_dropped(self):
        samples = [{"quit": 4.0}, {"quit": 6.0}, {"detach_failed": True}]
        self.assertEqual(editions.quit_cell(samples, 3), "5.0 (4.0–6.0); 1/3 detaches failed")

    def test_only_failures_still_report(self):
        self.assertEqual(editions.quit_cell([{"detach_failed": True}], 1), "1/1 detaches failed")


if __name__ == "__main__":
    unittest.main()
