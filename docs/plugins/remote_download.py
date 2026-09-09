# SPDX-License-Identifier: MPL-2.0
"""One bounded binary download, with host staging and native publication approval."""
import posixpath
import threading
import unicodedata

from application import PluginError

MAX_BYTES = 8 * 1024 * 1024


class Downloads:
    def __init__(self, app, transport, on_status=None):
        self.app, self.transport = app, transport
        self.lock = threading.RLock()
        self.flow = None
        self.ended = {}
        self.worker_active = False
        self.on_status = on_status or (lambda phase: None)
        self.last_status = None

    def status(self, phase):
        # Presentation failure cannot turn a safe transfer into a retry.
        with self.lock:
            if self.last_status == phase:
                return
            self.last_status = phase
            try:
                self.on_status(phase)
            except Exception:
                pass

    def prompt(self, context, entry):
        size = entry.get('size')
        if entry['kind'] != 'file':
            raise PluginError('unsupported', 'Download one ordinary remote file')
        if type(size) is not int or not 0 <= size <= MAX_BYTES:
            raise PluginError('limit_exceeded', 'Downloads are limited to 8 MiB; refresh remote metadata')
        with self.lock:
            if self.flow is not None or self.worker_active:
                raise PluginError('busy', 'Finish or cancel the current download first')
            flow = {'source': dict(entry), 'cancel': threading.Event(), 'phase': 'prompt',
                    'surface': None, 'job': None, 'staging': None, 'plan': None,
                    'surface_ready': threading.Event(), 'done': threading.Event()}
            self.flow = flow
        try:
            surface = self.app.request('ui.prompt', invocation=context['invocation'],
                title='Download destination', field={'id': 'destination',
                'label': 'New file path relative to the workspace', 'kind': 'text',
                'required': True})['surface']
            with self.lock:
                live = self.flow is flow and not flow['cancel'].is_set()
                if live:
                    flow['surface'] = surface
            if not live:
                self.app.request('ui.dismiss', surface=surface)
        except Exception:
            with self.lock:
                if self.flow is flow:
                    self.flow = None
            raise
        finally:
            flow['surface_ready'].set()

    @staticmethod
    def check(flow):
        if flow['cancel'].is_set():
            raise PluginError('cancelled', 'Download cancelled')

    def discard(self, flow, state='failed'):
        with self.lock:
            if flow['phase'] == 'closing':
                return
            flow['phase'] = 'closing'
        for method, field in [('filesystem.cancel', 'plan'), ('staging.close', 'staging')]:
            if flow[field] is not None:
                try:
                    self.app.request(method, **{field: flow[field]})
                except PluginError:
                    pass
        if flow['job'] is not None:
            try:
                self.app.request('job.finish', job=flow['job'], state=state)
            except PluginError:
                pass
        with self.lock:
            if self.flow is flow:
                self.flow = None
                self.status(state)

    def submitted(self, context):
        flow = self.flow
        if flow is not None:
            # A physical submission can overtake the prompt caller resuming.
            # This wait is on a command worker, never the cancellation executor.
            flow['surface_ready'].wait(10)
        with self.lock:
            flow = self.flow
            if flow is None or flow['phase'] != 'prompt' or context['surface'] != flow['surface']:
                return
            if not context['accepted']:
                self.flow = None
                return
            flow['phase'] = 'transferring'
        try:
            destination = context['values'].get('destination', '')
            try:
                valid_bytes = isinstance(destination, str) and len(destination.encode('utf-8')) <= 4096
            except UnicodeError:
                valid_bytes = False
            if (not isinstance(destination, str) or not destination or destination.startswith('/')
                    or not valid_bytes
                    or '..' in destination.split('/') or destination.endswith('/')
                    or any(unicodedata.category(c) == 'Cc' for c in destination)
                    or posixpath.normpath(destination) == '.'):
                raise PluginError('invalid_argument', 'Choose a new file path relative to the workspace')
            self.check(flow)
            flow['job'] = self.app.request('job.create', title='Download remote file', deadline_seconds=60)['job']
            with self.lock:
                if flow['job'] in self.ended:
                    flow['cancel'].set()
            self.check(flow)
            with self.lock:
                self.check(flow)
                self.worker_active = True
                self.status('transferring')
            try:
                threading.Thread(target=self.transfer, args=(flow, destination), daemon=True).start()
            except Exception:
                with self.lock:
                    self.worker_active = False
                flow['done'].set()
                raise PluginError('unavailable', 'Download worker could not start') from None
            return {'job': flow['job']}
        except Exception:
            self.discard(flow, 'cancelled' if flow['cancel'].is_set() else 'failed')
            raise

    def transfer(self, flow, destination):
        directory = None
        try:
            self.check(flow)
            stage = self.app.request('staging.create', job=flow['job'], bytes=flow['source']['size'])
            flow['staging'] = stage['staging']
            last_progress = -1
            def progress(count):
                nonlocal last_progress
                self.check(flow)
                value = min(90, count * 90 // max(1, flow['source']['size']))
                if value // 10 != last_progress:
                    last_progress = value // 10
                    self.app.request('job.update', job=flow['job'], progress=value)
            digest = self.transport.download(flow['source']['path'], stage['path'],
                flow['source']['size'], flow['cancel'], progress)
            self.check(flow)
            listing = self.app.request('filesystem.list', path=posixpath.dirname(destination) or '.',
                                       offset=0, limit=1, expected_revision=None)
            directory = listing['directory']
            self.check(flow)
            prepared = self.app.request('staging.prepare', staging=flow['staging'],
                directory=directory, expected_revision=listing['revision'],
                destination=destination, sha256=digest)
            flow['plan'] = prepared['plan']
            flow['staging'] = None
            self.app.request('filesystem.release', directory=directory)
            directory = None
            self.check(flow)
            with self.lock:
                flow['phase'] = 'prepared'
            self.check(flow)
            self.app.request('job.update', job=flow['job'], progress=100)
            with self.lock:
                if self.flow is flow and flow['phase'] == 'prepared':
                    self.status('ready')
        except Exception:
            self.discard(flow, 'cancelled' if flow['cancel'].is_set() else 'failed')
        finally:
            if directory is not None:
                try:
                    self.app.request('filesystem.release', directory=directory)
                except PluginError:
                    pass
            with self.lock:
                self.worker_active = False
            flow['done'].set()

    def apply(self, flow, invocation):
        self.check(flow)
        try:
            self.app.request('filesystem.apply', plan=flow['plan'], invocation=invocation)
        except PluginError as error:
            if error.code == 'context_changed':
                with self.lock:
                    flow['phase'] = 'prepared'
                if flow['cancel'].is_set():
                    self.discard(flow, 'cancelled')
                # The still-running job owns the unpresented plan. A new
                # confirm-download invocation can present native approval.
                return {'job': flow['job']}
            raise
        flow['plan'] = None
        self.app.request('job.finish', job=flow['job'], state='succeeded')
        with self.lock:
            if self.flow is flow:
                self.flow = None
            if self.flow is None:
                self.status('completed')
        return {'job': flow['job']}

    def confirm(self, context):
        with self.lock:
            flow = self.flow
            if flow is None or flow['phase'] != 'prepared':
                raise PluginError('not_found', 'No downloaded file is waiting for confirmation')
            flow['phase'] = 'confirming'
        try:
            return self.apply(flow, context['invocation'])
        except Exception:
            self.discard(flow, 'cancelled' if flow['cancel'].is_set() else 'failed')
            raise

    def cancel(self, _context):
        with self.lock:
            flow = self.flow
            if flow is None:
                return
            flow['cancel'].set()
            phase = flow['phase']
        if phase == 'prompt':
            try:
                if flow['surface'] is not None:
                    self.app.request('ui.dismiss', surface=flow['surface'])
            finally:
                self.discard(flow, 'cancelled')
        elif phase == 'prepared':
            self.discard(flow, 'cancelled')

    def event(self, name, data):
        terminal = name == 'job.changed' and data.get('state') in ('succeeded', 'failed', 'cancelled', 'outcome_unknown')
        if name == 'job.cancel_requested' or terminal:
            # Control events can overtake the job.create caller resuming after
            # its response. Retain bounded tombstones until it captures the ID.
            with self.lock:
                self.ended[data.get('job')] = True
                while len(self.ended) > 16:
                    del self.ended[next(iter(self.ended))]
        flow = self.flow
        if flow is None or data.get('job') != flow['job']:
            return
        if name == 'job.cancel_requested':
            flow['cancel'].set()
            with self.lock:
                prepared = flow['phase'] == 'prepared'
                if prepared:
                    flow['phase'] = 'cancelling'
            if prepared:
                # Only one retained plan exists. Its cleanup can wait on host
                # replies without blocking the dedicated cancellation worker.
                threading.Thread(target=self.discard, args=(flow, 'cancelled'), daemon=True).start()
        elif terminal:
            flow['cancel'].set()
            with self.lock:
                if self.flow is flow:
                    self.flow = None
                    self.status('completed' if data['state'] == 'succeeded' else
                                'cancelled' if data['state'] == 'cancelled' else 'failed')
