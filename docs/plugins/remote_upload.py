# SPDX-License-Identifier: MPL-2.0
"""Explicit upload of one immutable workspace disk-file snapshot, never buffer text."""
import json
import os
from pathlib import Path
import posixpath
import stat
import threading
import unicodedata

from application import PluginError
from remote_operations import Operations

MAX_BYTES = 8 * 1024 * 1024


def relative_path(value):
    value = Operations.path(value)
    if any(part in ('', '.', '..') for part in value.split('/')):
        raise PluginError('invalid_argument', 'Choose a workspace-relative disk file')
    return value


def snapshot_source(root, relative, cancel):
    """Read through no-follow directory descriptors, then verify identity and EOF."""
    relative = relative_path(relative)
    descriptors = []
    links = []
    def check():
        if cancel.is_set():
            raise PluginError('cancelled', 'Disk-file upload cancelled')
    def identity(info):
        return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
    try:
        check()
        flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK
        parent = os.open(root, flags | os.O_DIRECTORY)
        descriptors.append(parent)
        for part in relative.split('/')[:-1]:
            child = os.open(part, flags | os.O_DIRECTORY, dir_fd=parent)
            descriptors.append(child)
            links.append((parent, part, os.fstat(child)))
            parent = child
        name = relative.split('/')[-1]
        file = os.open(name, flags, dir_fd=parent)
        descriptors.append(file)
        before = os.fstat(file)
        if not stat.S_ISREG(before.st_mode):
            raise PluginError('unsupported', 'Upload source must be an ordinary disk file')
        if before.st_size > MAX_BYTES:
            raise PluginError('limit_exceeded', 'Disk-file upload exceeds 8 MiB')
        data = bytearray()
        while len(data) < before.st_size:
            check()
            chunk = os.read(file, min(65536, before.st_size - len(data)))
            if not chunk:
                raise PluginError('stale', 'Disk file changed while preparing the upload')
            data.extend(chunk)
        check()
        if os.read(file, 1) or identity(os.fstat(file)) != identity(before):
            raise PluginError('stale', 'Disk file changed while preparing the upload')
        if identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) != identity(before):
            raise PluginError('stale', 'Disk file changed while preparing the upload')
        for directory, part, captured in links:
            current = os.stat(part, dir_fd=directory, follow_symlinks=False)
            if (current.st_dev, current.st_ino) != (captured.st_dev, captured.st_ino):
                raise PluginError('stale', 'Disk source directory changed while preparing the upload')
        check()
        return bytes(data)
    except OSError:
        raise PluginError('unavailable', 'Cannot read the selected workspace disk file safely') from None
    finally:
        for descriptor in reversed(descriptors):
            os.close(descriptor)


class Uploads(Operations):
    def __init__(self, app, transport, on_status=None, workspace_root=None):
        super().__init__(app, transport, on_status)
        self.workspace_root = Path(workspace_root if workspace_root is not None else Path.cwd()).resolve()

    def start(self, context, directory):
        with self.lock:
            if self.flow is not None or self.worker_active:
                message = ('Upload outcome unknown; inspect the remote target and explicitly restart the plugin'
                           if self.flow is not None and self.flow['phase'] == 'outcome_unknown'
                           else 'Finish or cancel the current disk-file upload first')
                raise PluginError('busy', message)
            flow = {'directory': directory, 'cancel': threading.Event(), 'phase': 'prompt',
                    'surface': None, 'surface_ready': threading.Event(), 'job': None,
                    'prepared': None, 'data': None, 'done': threading.Event(), 'closing': False,
                    'progress': -1}
            self.flow = flow
        try:
            surface = self.app.request('ui.form', invocation=context['invocation'],
                title='Upload disk file', fields=[
                    {'id': 'source', 'label': 'Workspace-relative disk file (saved bytes only)',
                     'kind': 'text', 'required': True},
                    {'id': 'destination', 'label': 'Destination relative to the remote directory',
                     'kind': 'text', 'required': True}])['surface']
            self.capture_surface(flow, surface)
            return {}
        except Exception:
            self.finish(flow, 'failed')
            raise
        finally:
            flow['surface_ready'].set()

    def submitted(self, context):
        with self.lock:
            flow = self.flow
            pending = flow is not None and flow['phase'] in ('prompt', 'confirming')
        if not pending:
            return None
        flow['surface_ready'].wait(10)
        with self.lock:
            if (self.flow is not flow or flow['phase'] not in ('prompt', 'confirming')
                    or flow['surface'] is None or context.get('surface') != flow['surface']):
                return None
            phase = flow['phase']
            flow['surface'] = None
            flow['phase'] = 'starting'
        if context.get('accepted') is not True:
            self.finish(flow, 'cancelled')
            return {}
        try:
            self.check(flow)
            if not context.get('invocation'):
                raise PluginError('context_changed', 'Upload confirmation requires a fresh invocation')
            if phase == 'prompt':
                values = context.get('values', {})
                flow['source'] = relative_path(values.get('source', ''))
                flow['destination'] = posixpath.join(flow['directory'], self.path(values.get('destination', '')))
            return self.launch(flow, prepare=phase == 'prompt')
        except Exception:
            self.finish(flow, 'cancelled' if flow['cancel'].is_set() else 'failed')
            raise

    def launch(self, flow, *, prepare):
        try:
            self.check(flow)
            if prepare:
                flow['job'] = self.app.request('job.create', title='Upload disk file; then confirm-upload',
                                               deadline_seconds=60)['job']
            with self.lock:
                if flow['job'] in self.ended:
                    flow['cancel'].set()
                self.check(flow)
                if self.worker_active:
                    raise PluginError('busy', 'Previous upload worker is still settling')
                self.worker_active = True
                flow['phase'] = 'preparing' if prepare else 'applying'
                flow['done'].clear()
                self.status(flow, flow['phase'])
            try:
                threading.Thread(target=self.work, args=(flow, prepare),
                                 name='runyte-upload', daemon=True).start()
            except Exception:
                with self.lock:
                    self.worker_active = False
                flow['done'].set()
                raise PluginError('unavailable', 'Upload worker could not start') from None
            return {'job': flow['job']}
        except Exception:
            self.finish(flow, 'cancelled' if flow['cancel'].is_set() else 'failed')
            raise

    def progress(self, flow, value):
        # At most eleven host updates, and 100 belongs exclusively to the
        # acknowledged promotion below, never a transport streaming callback.
        if not isinstance(value, int) or isinstance(value, bool):
            return
        value = max(0, min(90, value // 10 * 10))
        with self.lock:
            if self.flow is not flow or flow['closing'] or flow['cancel'].is_set() or value <= flow['progress']:
                return
            flow['progress'] = value
        try:
            self.app.request('job.update', job=flow['job'], progress=value)
        except PluginError:
            flow['cancel'].set()

    def work(self, flow, prepare):
        try:
            self.check(flow)
            if prepare:
                data = snapshot_source(self.workspace_root, flow['source'], flow['cancel'])
                self.check(flow)
                prepared = self.transport.prepare_upload(flow['destination'], data, flow['cancel'])
                self.check(flow)
                self.confirmation_for(flow, prepared)
                with self.lock:
                    self.check(flow)
                    flow['prepared'], flow['data'] = prepared, data
                    flow['phase'] = 'ready'
                    self.status(flow, 'ready')
            else:
                result = self.transport.upload(flow['prepared'], flow['data'], flow['cancel'],
                                               lambda value: self.progress(flow, value))
                if result != 'applied':
                    raise PluginError('outcome_unknown', 'Upload has no acknowledged outcome')
                try:
                    self.app.request('job.update', job=flow['job'], progress=100)
                except PluginError:
                    pass
                self.finish(flow, 'succeeded')
        except Exception as error:
            unknown = (not prepare and (getattr(error, 'outcome_unknown', False)
                       or getattr(error, 'settled', True) is False
                       or getattr(error, 'code', None) == 'outcome_unknown'
                       or not isinstance(error, PluginError)))
            self.finish(flow, 'outcome_unknown' if unknown else
                        'cancelled' if flow['cancel'].is_set() else 'failed')
        finally:
            if prepare and flow['cancel'].is_set():
                self.finish(flow, 'cancelled')
            with self.lock:
                self.worker_active = False
            flow['done'].set()

    def confirmation_for(self, flow, prepared):
        def quoted(value):
            return '"' + ''.join(json.dumps(c, ensure_ascii=not c.isprintable())[1:-1]
                                 for c in value) + '"'
        title = f'Upload remote file · {quoted(self.transport.label)}'
        message = (f'{quoted(flow["source"])} → {quoted(prepared.destination)} '
                   f'({prepared.bytes} disk bytes). ' + '; '.join(prepared.warnings))
        try:
            fits = all(0 < len(value.encode('utf-8')) <= 160
                       and not any(unicodedata.category(c) == 'Cc' for c in value)
                       for value in (title, message))
        except UnicodeError:
            fits = False
        if not fits:
            raise PluginError('limit_exceeded', 'Exact upload paths and warnings exceed native confirmation limits')
        return title, message

    def confirmation(self, prepared):
        return self.confirmation_for(self.flow, prepared)

    def finish(self, flow, state):
        with self.lock:
            if flow['closing']:
                return
            flow['closing'] = True
            surface, job = flow['surface'], flow['job']
            flow['surface'] = None
        if surface is not None:
            try:
                self.app.request('ui.dismiss', surface=surface)
            except PluginError:
                pass
        if job is not None:
            try:
                self.app.request('job.finish', job=job, state=state)
            except PluginError as error:
                if state == 'succeeded':
                    state = 'outcome_unknown'
                    if error.code == 'cancelled':
                        try:
                            self.app.request('job.finish', job=job, state=state)
                        except PluginError:
                            pass
        with self.lock:
            self.status(flow, 'completed' if state == 'succeeded' else state)
            flow['phase'] = state
            if self.flow is flow and state != 'outcome_unknown':
                self.flow = None
                flow['data'] = None

    def cancel(self, context):
        with self.lock:
            if self.flow is not None and self.flow['phase'] == 'outcome_unknown':
                raise PluginError('outcome_unknown', 'Inspect the remote target and explicitly restart the plugin before another upload')
        return super().cancel(context)
