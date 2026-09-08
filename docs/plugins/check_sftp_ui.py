# SPDX-License-Identifier: MPL-2.0
"""SFTP application boundary tests; no network, credentials or third-party packages."""
import unittest

from application import PluginError
from sftp import MAX_ROWS, SftpApplication


class Port:
    def __init__(self):
        self.calls = []
        self.revision = 0
        self.failure = None

    def request(self, method, **params):
        self.calls.append((method, params))
        if self.failure == method:
            raise PluginError('busy', 'Injected host refusal')
        if method in ('view.create', 'view.publish'):
            self.revision += 1
            return {'view': 'v:1', 'revision': str(self.revision)}
        if method in ('ui.form', 'ui.confirm'):
            return {'surface': 'surface:1'}
        if method.startswith('resource.'):
            return {'job': 'j:1'}
        return {}


class Transport:
    root = '/root'
    label = 'Development 猫'
    connection_id = 'opaque-connection'

    def __init__(self):
        self.rows = [{'name': 'notes 猫.md', 'kind': 'file', 'size': 3},
                     {'name': 'folder', 'kind': 'directory', 'size': 0},
                     {'name': 'link', 'kind': 'symlink', 'size': 0}]
        self.paths = []

    def browse(self, path):
        self.paths.append(path)
        return (self.root if path == '.' else path), self.rows


class Provider:
    handlers = {}

    def __init__(self):
        self.events = []

    def requested_key(self, path):
        return 'opaque:' + path

    def on_event(self, name, data):
        self.events.append((name, data))


class SftpUiTests(unittest.TestCase):
    def setUp(self):
        self.port, self.transport, self.provider = Port(), Transport(), Provider()
        self.ui = SftpApplication(self.transport, self.provider, 'sftp', self.port)
        self.context = {'arguments': {'path': '.'}, 'invocation': 'h:1'}

    def browse(self):
        self.ui.browse(self.context)
        return {'view': self.ui.view, 'model_revision': self.ui.revision,
                'rows': [], 'invocation': 'h:2'}

    def test_browse_registers_weak_provider_and_opens_selected_file_with_invocation(self):
        context = self.browse()
        self.assertEqual(self.port.calls[0], ('provider.register', {
            'name': 'remote', 'conditional_write': False, 'atomic_replace': True}))
        self.assertEqual(self.port.calls[-1], ('pane.show', {'invocation': 'h:1', 'view': 'v:1'}))
        context['rows'] = [next(key for key, row in self.ui.entries.items() if row['kind'] == 'file')]
        self.assertEqual(self.ui.enter(context), {'job': 'j:1'})
        self.assertEqual(self.port.calls[-1], ('resource.open', {
            'plugin': 'sftp', 'provider': 'remote', 'key': 'opaque:/root/notes 猫.md',
            'invocation': 'h:2'}))
        self.assertEqual(sum(method == 'provider.register' for method, _ in self.port.calls), 1)

    def test_refresh_retains_stable_rows_and_rejects_old_actions(self):
        context = self.browse()
        old = dict(self.ui.entries)
        self.transport.rows.reverse()
        self.ui.refresh(context)
        self.assertEqual(old, self.ui.entries)
        self.assertEqual(self.port.calls[-1][1]['expected_revision'], '1')
        with self.assertRaises(PluginError) as raised:
            self.ui.enter(context)
        self.assertEqual(raised.exception.code, 'stale')
        context['model_revision'] = self.ui.revision
        context['rows'] = [next(key for key, row in self.ui.entries.items() if row['kind'] == 'symlink')]
        with self.assertRaises(PluginError) as raised:
            self.ui.enter(context)
        self.assertEqual(raised.exception.code, 'unsupported')
        context['rows'] = list(self.ui.entries)
        with self.assertRaises(PluginError):
            self.ui.enter(context)

    def test_navigation_is_remote_and_root_parent_stays_at_root(self):
        context = self.browse()
        self.ui.parent(context)
        self.assertEqual(self.transport.paths[-1], '/root')
        context['model_revision'] = self.ui.revision
        context['rows'] = [next(key for key, row in self.ui.entries.items() if row['kind'] == 'directory')]
        self.ui.enter(context)
        self.assertEqual(self.transport.paths[-1], '/root/folder')
        self.assertEqual(self.port.calls[-1][0], 'view.publish')

    def test_failed_and_oversized_publication_preserves_previous_model(self):
        context = self.browse()
        previous = self.ui.view, self.ui.revision, self.ui.path, dict(self.ui.entries)
        for rows in ([{'name': f'n{i}', 'kind': 'file'} for i in range(MAX_ROWS + 1)],
                     [{'name': 'x' * (901 * 1024), 'kind': 'file'}],
                     [{'name': '../escape', 'kind': 'file'}],
                     [{'name': 'dup', 'kind': 'file'}] * 2):
            self.transport.rows = rows
            with self.assertRaises(PluginError):
                self.ui.refresh(context)
            self.assertEqual((self.ui.view, self.ui.revision, self.ui.path, self.ui.entries), previous)
        self.transport.rows = []
        self.port.failure = 'view.publish'
        with self.assertRaises(PluginError):
            self.ui.refresh(context)
        self.assertEqual((self.ui.view, self.ui.revision, self.ui.path, self.ui.entries), previous)

    def test_controls_are_escaped_in_presentation_only_and_title_is_bounded(self):
        self.transport.rows = [{'name': 'odd\nname', 'kind': 'file'}]
        self.transport.label = '猫' * 100
        self.browse()
        model = next(params['model'] for method, params in self.port.calls if method == 'view.create')
        self.assertLessEqual(len(model['title'].encode('utf-8')), 160)
        self.assertEqual(model['rows'][0]['text'], 'odd\\u000aname')
        self.assertEqual(next(iter(self.ui.entries.values()))['path'], '/root/odd\nname')

    def test_busy_commands_do_not_wait_and_close_forwards_provider_events(self):
        context = self.browse()
        self.ui.lock.acquire()
        try:
            with self.assertRaises(PluginError) as raised:
                self.ui.refresh(context)
            self.assertEqual(raised.exception.code, 'busy')
        finally:
            self.ui.lock.release()
        self.ui.event('resource.released', {'job': 'j:1'})
        self.ui.event('view.closed', {'view': 'unrelated'})
        self.assertEqual(self.ui.view, 'v:1')
        self.ui.event('view.closed', {'view': 'v:1'})
        self.assertIsNone(self.ui.view)
        self.assertEqual(self.ui.entries, {})
        self.assertEqual(self.provider.events[0], ('resource.released', {'job': 'j:1'}))

    def test_upload_uses_captured_current_directory_and_routes_only_its_surface(self):
        context = self.browse()
        context['rows'] = []
        self.assertEqual(self.ui.upload(context), {})
        self.assertEqual(self.ui.uploads.flow['directory'], '/root')
        method, params = self.port.calls[-1]
        self.assertEqual((method, params['title']), ('ui.form', 'Upload disk file'))
        self.assertEqual([field['id'] for field in params['fields']], ['source', 'destination'])
        self.assertIn('saved bytes only', params['fields'][0]['label'])
        self.assertIsNone(self.ui.submitted({'surface': 'other', 'accepted': False}))
        self.assertEqual(self.ui.submitted({'surface': 'surface:1', 'accepted': False}), {})
        self.assertIsNone(self.ui.uploads.flow)
        self.assertTrue(self.ui.upload_status.idle.wait(2))
        with self.assertRaises(PluginError) as raised:
            self.ui.upload(context)
        self.assertEqual(raised.exception.code, 'stale')
        self.assertFalse(any(method == 'job.create' for method, _ in self.port.calls))
        self.assertTrue({'upload', 'confirm-upload', 'cancel-upload'} <= self.ui.handlers.keys())

    def test_inspect_rebind_and_direct_open_return_issued_jobs(self):
        context = {'buffer': 'b:1', 'buffer_revision': 'r:2', 'invocation': 'h:3'}
        self.assertEqual(self.ui.inspect(context), {'job': 'j:1'})
        self.assertEqual(self.port.calls[-1], ('resource.inspect', {
            'buffer': 'b:1', 'expected_revision': 'r:2', 'invocation': 'h:3'}))
        self.assertEqual(self.ui.rebind(context), {'job': 'j:1'})
        self.assertEqual(self.port.calls[-1], ('resource.rebind', {
            'buffer': 'b:1', 'expected_revision': 'r:2'}))
        self.assertEqual(self.ui.open(self.context), {'job': 'j:1'})
        self.assertNotIn('save', self.ui.handlers)


if __name__ == '__main__':
    unittest.main()
