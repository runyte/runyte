# SPDX-License-Identifier: MPL-2.0
"""Shared remote UI regressions with explicit FTP/FTPS guarantees; no network."""
import subprocess
import sys
import unittest
from pathlib import Path

import check_sftp_ui as shared
from application import PluginError
from ftp import FtpApplication


class FtpUiTests(shared.SftpUiTests):
    def setUp(self):
        super().setUp()
        self.transport.protocol = 'ftps'
        self.ui = FtpApplication(self.transport, self.provider, 'ftp', self.port)

    def test_browse_registers_weak_provider_and_opens_selected_file_with_invocation(self):
        context = self.browse()
        self.assertEqual(self.port.calls[0], ('provider.register', {
            'name': 'remote', 'conditional_write': False, 'atomic_replace': False}))
        self.assertEqual(self.port.calls[-1], ('pane.show', {'invocation': 'h:1', 'view': 'v:1'}))
        context['rows'] = [next(key for key, row in self.ui.entries.items() if row['kind'] == 'file')]
        self.assertEqual(self.ui.enter(context), {'job': 'j:1'})
        self.assertEqual(self.port.calls[-1], ('resource.open', {
            'plugin': 'ftp', 'provider': 'remote', 'key': 'opaque:/root/notes 猫.md',
            'invocation': 'h:2'}))
        self.assertEqual(sum(method == 'provider.register' for method, _ in self.port.calls), 1)

    def test_transport_label_and_guarantees_are_explicit_in_model_and_commands(self):
        for protocol, label in [('ftps', 'FTPS'), ('ftp', 'FTP (unencrypted)')]:
            with self.subTest(protocol=protocol):
                self.transport.protocol = protocol
                self.transport.alias = 'Development 猫'
                self.transport.label = label + ' · ' + self.transport.alias
                self.port.calls.clear()
                self.ui = FtpApplication(self.transport, self.provider, 'other-ftp', self.port)
                self.browse()
                model = next(params['model'] for method, params in self.port.calls if method == 'view.create')
                self.assertTrue(model['title'].startswith(label + ' · '))
                self.assertEqual(model['title'].count(label), 1)
                self.assertFalse(self.ui.atomic_replace)
                ui = FtpApplication(self.transport, self.provider)
                self.assertEqual(ui.app.name, label + ' files')
                for name in ('browse', 'open'):
                    command = next(command for command in ui.app.commands if command['name'] == name)
                    self.assertIn(label, command['description'])
                self.assertNotIn('save', ui.handlers)
                for name, scope in [('upload', 'view'), ('confirm-upload', 'workspace'),
                                    ('cancel-upload', 'workspace')]:
                    registered = next(command for command in ui.app.commands if command['name'] == name)
                    self.assertEqual(registered['context'], scope)
                self.ui.open(self.context)
                self.assertEqual(self.port.calls[-1][1]['plugin'], 'other-ftp')

    def test_unsupported_transport_and_invalid_plugin_identity_are_refused(self):
        self.transport.protocol = 'sftp'
        with self.assertRaises(PluginError):
            FtpApplication(self.transport, self.provider, app=self.port)
        self.transport.protocol = 'ftps'
        with self.assertRaises(PluginError):
            FtpApplication(self.transport, self.provider, 'bad.plugin', self.port)

    def test_ftp_ui_import_and_help_need_only_standard_library(self):
        directory = str(Path(__file__).resolve().parent)
        script = """import importlib.abc
import sys
sys.path.insert(0, sys.argv[1])
class NoSsh(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if fullname.split('.')[0] in ('paramiko', 'sftp', 'sftp_transport'):
            raise AssertionError('FTP tried to import SSH support')
sys.meta_path.insert(0, NoSsh())
import ftp
import ftp_transport
assert 'paramiko' not in sys.modules
assert 'sftp_transport' not in sys.modules
assert 'sftp' not in sys.modules
"""
        result = subprocess.run([sys.executable, '-S', '-c', script, directory],
                                capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        result = subprocess.run([sys.executable, '-S', str(Path(directory) / 'ftp.py'), '--help'],
                                capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertIn(b'--config', result.stdout)
        self.assertIn(b'--plugin-id', result.stdout)


if __name__ == '__main__':
    unittest.main()
