# SPDX-License-Identifier: MPL-2.0
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import editions
import interaction_latency as interaction


def frame(ink):
    width, height = editions.WINDOW_SIZE
    pixels = bytearray(b'\x10\x18\x18\xff' * (width * height))
    for x, y in ink:
        pixels[(y * width + x) * 4 + 1] = 0xd0
    return bytes(pixels)


class Calibration(unittest.TestCase):
    def test_pdf_proof_ignores_white_pixels_outside_companion_pane(self):
        width, height = editions.WINDOW_SIZE
        pixels = bytearray(b'\xff\xff\xff\xff' * (width * height))
        self.assertGreater(interaction.white_page_pixels((pixels, width * 4)), 10000)
        for y in range(height):
            pixels[y * width * 4 + width // 2 * 4:(y + 1) * width * 4] = (
                b'\x10\x18\x18\xff' * (width // 2))
        self.assertEqual(interaction.white_page_pixels((pixels, width * 4)), 0)

    def test_cursor_only_change_is_rejected(self):
        before = frame([(40, 25)])
        after = frame([(49, 25)])
        with self.assertRaises(ValueError):
            interaction.calibration_band(before, after, editions.WINDOW_SIZE[0] * 4)

    def test_status_and_side_pane_do_not_select_band(self):
        before = frame([(x, 25) for x in range(40, 250, 11)])
        after = frame([(x + 9, 25) for x in range(40, 250, 11)]
                      + [(x, y) for x in range(600, 1000) for y in range(40)]
                      + [(x, 780) for x in range(1000)])
        band = interaction.calibration_band(before, after, editions.WINDOW_SIZE[0] * 4)
        self.assertEqual(band[1], 20)
        self.assertGreaterEqual(band[0], 40 + 3 * editions.CELL_WIDTH)
        self.assertLess(band[0] + band[2], editions.WINDOW_SIZE[0] // 2)
        self.assertFalse(editions.same_shape(editions.mask(editions.region(before, band)),
                                            editions.mask(editions.region(after, band))))


class Isolation(unittest.TestCase):
    def test_all_storage_is_fixture_owned_and_ambient_trace_removed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.dict(os.environ, {'RUNYTE_INPUT_TRACE': '/must/not/write'}):
                env = interaction.isolated_environment(root)
            self.assertNotIn('RUNYTE_INPUT_TRACE', env)
            for name in ('HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_STATE_HOME',
                         'XDG_DATA_HOME', 'XDG_RUNTIME_DIR', 'RUNYTE_TEST_STATE_HOME',
                         'RUNYTE_TEST_ACCOUNT_HOME', 'RUNYTE_TEST_LOCAL_APP_DATA',
                         'RUNYTE_ALL_HOSTS_DIR', 'RUNYTE_CONTEXT_HOME', 'RUNYTE_TEST_SOCKET_HOME'):
                self.assertTrue(Path(env[name]).is_relative_to(root), name)


if __name__ == '__main__':
    unittest.main()
