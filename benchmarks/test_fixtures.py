# SPDX-License-Identifier: MPL-2.0

"""Regression coverage for generated benchmark fixtures."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import fixtures


class LuaFixtureTests(unittest.TestCase):
    def test_large_lua_is_deterministic_complete_source(self) -> None:
        source = fixtures._lua_source(fixtures.LUA_LINES)

        self.assertEqual(source, fixtures._lua_source(fixtures.LUA_LINES))
        self.assertEqual(source.count("\n"), fixtures.LUA_LINES)
        self.assertTrue(source.endswith("\n\n"))
        self.assertEqual(source.count("local function "), source.count("\nend\n"))

    def test_large_lua_avoids_every_editors_injection_triggers(self) -> None:
        source = fixtures._lua_source(fixtures.LUA_LINES)

        for trigger in (
            "--",
            "[[",
            "]]",
            "cdef(",
            "exec_lua(",
            "vim.",
        ):
            with self.subTest(trigger=trigger):
                self.assertNotIn(trigger, source)

    def test_ensure_generates_large_lua(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = fixtures.ensure(Path(directory), ("large.lua",))["large.lua"]

            self.assertEqual(path.read_text(), fixtures._lua_source(fixtures.LUA_LINES))


if __name__ == "__main__":
    unittest.main()
