# SPDX-License-Identifier: MPL-2.0
import tempfile
import unittest
from pathlib import Path

import plugin_host


class HostArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.binary = self.root / "original"
        self.binary.write_bytes(b"fixture bytes; never executed")
        self.artifact = self.root / "artifact"
        self.output = self.root / "target/debug/runyte"
        self.identity = dict(source="abc", system="Linux", machine="x86_64",
                             version="0.4.1", build="default-debug")
        plugin_host.pack(self.artifact, self.binary, self.identity)

    def test_round_trip_restores_bytes_and_executable_mode(self):
        plugin_host.restore(self.artifact, self.output, self.identity)
        self.assertEqual(self.output.read_bytes(), self.binary.read_bytes())
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o755)

    def test_wrong_source_platform_version_or_build_fails_before_install(self):
        for key in self.identity:
            with self.subTest(key=key), self.assertRaises(ValueError):
                plugin_host.restore(self.artifact, self.output,
                                    {**self.identity, key: "different"})
            self.assertFalse(self.output.exists())

    def test_corrupt_binary_fails_before_install(self):
        (self.artifact / "runyte").write_bytes(b"corrupt")
        with self.assertRaises(ValueError):
            plugin_host.restore(self.artifact, self.output, self.identity)
        self.assertFalse(self.output.exists())

    def test_missing_manifest_fails_closed(self):
        (self.artifact / "manifest.json").unlink()
        with self.assertRaises(FileNotFoundError):
            plugin_host.restore(self.artifact, self.output, self.identity)
        self.assertFalse(self.output.exists())
