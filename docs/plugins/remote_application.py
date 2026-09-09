# SPDX-License-Identifier: MPL-2.0
"""Shared bounded browser/editor UI for transport-neutral remote providers."""
import hashlib
import json
import posixpath
import re
import threading
from contextlib import contextmanager

from application import Application, PluginError
from remote_download import Downloads
from remote_operations import Operations
from remote_upload import Uploads
from remote_status import DownloadStatus, OperationStatus, UploadStatus, STATUS_HEADROOM

MAX_ROWS = 1024
MAX_MODEL_BYTES = 900 * 1024


def command(name, description, argument=None, context='view', primary=False):
    value = {'name': name, 'description': description, 'context': context,
             'primary': primary}
    if argument:
        value['arguments'] = [{'name': argument, 'type': 'string'}]
    return value


def display(value):
    """Escape controls in server labels without changing their resource identity."""
    return ''.join(character if character.isprintable() else
                   f'\\u{ord(character):04x}' for character in value)


def title(value):
    encoded = display(value).encode('utf-8')
    return encoded[:157].decode('utf-8', errors='ignore') + ('…' if len(encoded) > 157 else '')


@contextmanager
def available(lock):
    if not lock.acquire(blocking=False):
        raise PluginError('busy', 'Another remote command is still running; try again')
    try:
        yield
    finally:
        lock.release()


class RemoteApplication:
    def __init__(self, transport, provider, plugin_id, app=None, *, protocol_label, atomic_replace, workspace_root=None):
        if not re.fullmatch(r'[a-z][a-z0-9_-]{0,63}', plugin_id):
            raise PluginError('invalid_argument', 'Invalid configured plugin ID')
        self.transport, self.provider, self.plugin_id = transport, provider, plugin_id
        self.protocol_label, self.atomic_replace = protocol_label, atomic_replace
        self.app = app or Application(f'{protocol_label} files', [
            command('browse', f'Browse an {protocol_label} directory', 'path', context='workspace'),
            command('enter', 'Open the selected remote file or directory', primary=True),
            command('parent', 'Browse the remote parent directory'),
            command('refresh', 'Refresh the remote directory'),
            command('open', f'Open an {protocol_label} text document', 'path', context='workspace'),
            command('inspect', 'Compare current remote text with local edits', context='buffer'),
            command('rebind', 'Reconcile the remote document explicitly', context='buffer'),
            command('download', 'Download the selected remote file through native confirmation'),
            command('confirm-download', 'Confirm a completed download', context='workspace'),
            command('cancel-download', 'Cancel the current download', context='workspace'),
            command('upload', 'Prepare an upload of a workspace disk file'),
            command('confirm-upload', 'Confirm the prepared disk-file upload', context='workspace'),
            command('cancel-upload', 'Cancel the pending disk-file upload', context='workspace'),
            command('mkdir', 'Prepare a remote directory creation'),
            command('rename', 'Prepare a selected remote file or empty directory rename'),
            command('delete', 'Prepare permanent deletion of a remote file or empty directory'),
            command('confirm-operation', 'Review the prepared remote operation', context='workspace'),
            command('cancel-operation', 'Cancel the pending remote operation', context='workspace'),
        ], ['views', 'providers', 'documents', 'jobs', 'filesystem', 'interaction'])
        self.lock = threading.Lock()
        self.registration_lock = threading.Lock()
        self.registered = False
        self.view = self.revision = self.path = None
        self.entries = {}
        self.model = None
        self.download_status = DownloadStatus(self)
        self.operation_status = OperationStatus(self)
        self.upload_status = UploadStatus(self)
        self.downloads = Downloads(self.app, transport, on_status=self.download_status)
        self.operations = Operations(self.app, transport, on_status=self.operation_status)
        self.uploads = Uploads(self.app, transport, on_status=self.upload_status, workspace_root=workspace_root)
        self.handlers = {'browse': self.browse, 'enter': self.enter,
                         'parent': self.parent, 'refresh': self.refresh,
                         'open': self.open, 'inspect': self.inspect, 'rebind': self.rebind,
                         'download': self.download, 'confirm-download': self.downloads.confirm,
                         'cancel-download': self.downloads.cancel,
                         'upload': self.upload, 'confirm-upload': self.uploads.confirm,
                         'cancel-upload': self.uploads.cancel,
                         'mkdir': self.mkdir, 'rename': self.rename, 'delete': self.delete,
                         'confirm-operation': self.operations.confirm,
                         'cancel-operation': self.operations.cancel}
        self.app.handlers = self.handlers
        self.app.resource_handlers = provider.handlers
        self.app.on_event = self.event
        self.app.on_input = self.submitted

    def register(self):
        with available(self.registration_lock):
            if not self.registered:
                self.app.request('provider.register', name='remote',
                                 conditional_write=False, atomic_replace=self.atomic_replace)
                self.registered = True

    def current(self, context):
        if (self.view is None or context.get('view') != self.view
                or context.get('model_revision') != self.revision):
            raise PluginError('stale', 'Remote directory view changed; try again')

    def listing(self, destination):
        canonical, rows = self.transport.browse(destination)
        if len(rows) > MAX_ROWS:
            raise PluginError('limit_exceeded', 'Remote directory exceeds 1024 entries')
        label = getattr(self.transport, 'alias', self.transport.label)
        model = {'title': title(f'{self.protocol_label} · {label} · {canonical}'),
                 'purpose': 'list', 'rows': []}
        entries = {}
        for row in rows:
            name = row['name']
            if not isinstance(name, str) or name in ('', '.', '..') or '/' in name or '\0' in name:
                raise PluginError('invalid_argument', 'Invalid remote directory entry')
            path = posixpath.join(canonical, name)
            row_id = hashlib.sha256((self.transport.connection_id + '\0' + path).encode('utf-8')).hexdigest()
            if row_id in entries:
                raise PluginError('invalid_argument', 'Duplicate remote directory entry')
            kind = row['kind']
            suffix = '/' if kind == 'directory' else ' [symlink]' if kind == 'symlink' else ''
            model['rows'].append({'id': row_id, 'text': display(name) + suffix,
                                  'role': 'heading' if kind == 'directory' else
                                          'ordinary' if kind == 'file' else 'muted'})
            entries[row_id] = {'path': path, 'kind': kind, 'size': row.get('size')}
        if len(json.dumps(model, ensure_ascii=False).encode('utf-8')) > MAX_MODEL_BYTES - 3 * STATUS_HEADROOM:
            raise PluginError('limit_exceeded', 'Remote directory model exceeds message budget')
        presented = self.status_model(base=model)
        if self.view is None:
            result = self.app.request('view.create', model=presented)
        else:
            result = self.app.request('view.publish', view=self.view,
                                      expected_revision=self.revision, model=presented)
        self.view, self.revision = result['view'], result['revision']
        self.path, self.entries, self.model = canonical, entries, model

    def browse(self, context):
        with available(self.lock):
            self.register()
            self.listing(context['arguments']['path'])
            self.app.request('pane.show', invocation=context['invocation'], view=self.view)

    def open_path(self, context, path):
        self.register()
        result = self.app.request('resource.open', plugin=self.plugin_id, provider='remote',
                                  key=self.provider.requested_key(path),
                                  invocation=context['invocation'])
        return {'job': result['job']}

    def open(self, context):
        return self.open_path(context, context['arguments']['path'])

    def enter(self, context):
        with available(self.lock):
            self.current(context)
            rows = context.get('rows', [])
            if len(rows) != 1 or rows[0] not in self.entries:
                raise PluginError('invalid_argument', 'Select exactly one remote entry')
            entry = self.entries[rows[0]]
            if entry['kind'] == 'directory':
                self.listing(entry['path'])
            elif entry['kind'] == 'file':
                return self.open_path(context, entry['path'])
            else:
                raise PluginError('unsupported', 'Open ordinary files and directories; links are not followed')

    def parent(self, context):
        with available(self.lock):
            self.current(context)
            self.listing(self.path if self.path == self.transport.root else posixpath.dirname(self.path))

    def refresh(self, context):
        with available(self.lock):
            self.current(context)
            self.listing(self.path)

    def inspect(self, context):
        self.register()
        result = self.app.request('resource.inspect', buffer=context['buffer'],
                                  expected_revision=context['buffer_revision'],
                                  invocation=context['invocation'])
        return {'job': result['job']}

    def rebind(self, context):
        self.register()
        result = self.app.request('resource.rebind', buffer=context['buffer'],
                                  expected_revision=context['buffer_revision'])
        return {'job': result['job']}

    def download(self, context):
        with available(self.lock):
            self.current(context)
            rows = context.get('rows', [])
            if len(rows) != 1 or rows[0] not in self.entries:
                raise PluginError('invalid_argument', 'Select exactly one remote file')
            self.downloads.prompt(context, self.entries[rows[0]])

    def event(self, name, data):
        self.downloads.event(name, data)
        self.operations.event(name, data)
        self.uploads.event(name, data)
        self.provider.on_event(name, data)
        # View closures may arrive while a command waits for a host response.
        # Defer the invalidation until that bounded command has released its lock.
        if name == 'view.closed':
            with self.lock:
                if data['view'] == self.view:
                    self.view = self.revision = self.path = None
                    self.entries = {}
                    self.model = None

    def status_model(self, override=None, base=None):
        model = self.model if base is None else base
        for status in (self.download_status, self.operation_status, self.upload_status):
            phase = override[1] if override is not None and override[0] is status else None
            model = status.model(model, phase)
        return model

    def submitted(self, context):
        result = self.uploads.submitted(context)
        if result is not None:
            return result
        result = self.operations.submitted(context)
        return self.downloads.submitted(context) if result is None else result

    def upload(self, context):
        with available(self.lock):
            self.current(context)
            return self.uploads.start(context, self.path)

    def mkdir(self, context):
        with available(self.lock):
            self.current(context)
            return self.operations.start(context, 'mkdir', {'path': self.path, 'kind': 'directory', 'size': 0})

    def selected_operation(self, context, kind):
        with available(self.lock):
            self.current(context)
            rows = context.get('rows', [])
            if len(rows) != 1 or rows[0] not in self.entries:
                raise PluginError('invalid_argument', 'Select exactly one remote entry')
            return self.operations.start(context, kind, self.entries[rows[0]])

    def rename(self, context):
        return self.selected_operation(context, 'rename')

    def delete(self, context):
        return self.selected_operation(context, 'delete')
