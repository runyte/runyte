# SPDX-License-Identifier: MPL-2.0
"""One bounded, explicitly confirmed remote namespace operation."""
import json
import posixpath
import threading
import unicodedata

from application import PluginError


class Operations:
    def __init__(self, app, transport, on_status=None):
        self.app, self.transport = app, transport
        self.on_status = on_status or (lambda phase: None)
        self.lock = threading.RLock()
        self.flow = None
        self.worker_active = False
        self.ended = {}

    def status(self, flow, phase):
        with self.lock:
            if self.flow is not flow or flow.get('status') == phase:
                return
            flow['status'] = phase
            try:
                self.on_status(phase)
            except Exception:
                pass

    @staticmethod
    def check(flow):
        if flow['cancel'].is_set():
            raise PluginError('cancelled', 'Remote operation cancelled')

    @staticmethod
    def path(value):
        try:
            valid = isinstance(value, str) and 0 < len(value.encode('utf-8')) <= 3800
        except UnicodeError:
            valid = False
        if (not valid or any(unicodedata.category(c) == 'Cc' for c in value)
                or value.startswith('/') or '..' in value.split('/') or value.endswith('/') or value in ('.', '/')):
            raise PluginError('invalid_argument', 'Choose a bounded remote path without parent traversal')
        return value

    def start(self, context, kind, source):
        if kind not in ('mkdir', 'rename', 'delete'):
            raise PluginError('invalid_argument', 'Unknown remote operation')
        if source.get('kind') not in ('file', 'directory') or (kind == 'mkdir' and source['kind'] != 'directory'):
            raise PluginError('unsupported', 'Choose an ordinary remote file or directory; links are refused')
        with self.lock:
            if self.flow is not None or self.worker_active:
                raise PluginError('busy', 'Finish or cancel the current remote operation first')
            flow = {'kind': kind, 'source': dict(source), 'cancel': threading.Event(),
                    'phase': 'prompt' if kind != 'delete' else 'starting', 'surface': None,
                    'surface_ready': threading.Event(), 'job': None, 'prepared': None,
                    'done': threading.Event(), 'closing': False}
            self.flow = flow
        if kind == 'delete':
            return self.launch(flow, prepare=True)
        try:
            surface = self.app.request('ui.prompt', invocation=context['invocation'],
                title='New remote directory' if kind == 'mkdir' else 'Rename remote entry',
                field={'id': 'destination', 'label': 'Destination relative to the remote directory',
                       'kind': 'text', 'required': True})['surface']
            self.capture_surface(flow, surface)
            return {}
        except Exception:
            self.finish(flow, 'failed')
            raise
        finally:
            flow['surface_ready'].set()

    def capture_surface(self, flow, surface):
        with self.lock:
            live = self.flow is flow and not flow['cancel'].is_set()
            if live:
                flow['surface'] = surface
        if not live:
            self.app.request('ui.dismiss', surface=surface)

    def submitted(self, context):
        with self.lock:
            flow = self.flow
            pending = flow is not None and flow['phase'] in ('prompt', 'confirming')
        if not pending:
            return None
        flow['surface_ready'].wait(10)
        with self.lock:
            if (self.flow is not flow or flow['phase'] not in ('prompt', 'confirming')
                    or context.get('surface') != flow['surface'] or flow['surface'] is None):
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
                raise PluginError('context_changed', 'Native confirmation requires a fresh invocation')
            if phase == 'prompt':
                destination = self.path(context.get('values', {}).get('destination', ''))
                base = flow['source']['path'] if flow['kind'] == 'mkdir' else posixpath.dirname(flow['source']['path'])
                flow['destination'] = posixpath.join(base, destination)
                return self.launch(flow, prepare=True)
            return self.launch(flow, prepare=False)
        except Exception:
            self.finish(flow, 'cancelled' if flow['cancel'].is_set() else 'failed')
            raise

    def launch(self, flow, *, prepare):
        try:
            self.check(flow)
            if prepare:
                flow['job'] = self.app.request('job.create', title='Prepare remote operation', deadline_seconds=60)['job']
            with self.lock:
                if flow['job'] in self.ended:
                    flow['cancel'].set()
                self.check(flow)
                if self.worker_active:
                    raise PluginError('busy', 'Previous remote worker is still settling')
                self.worker_active = True
                flow['phase'] = 'preparing' if prepare else 'applying'
                flow['done'].clear()
                self.status(flow, flow['phase'])
            try:
                threading.Thread(target=self.work, args=(flow, prepare), daemon=True).start()
            except Exception:
                with self.lock:
                    self.worker_active = False
                flow['done'].set()
                raise PluginError('unavailable', 'Remote operation worker could not start') from None
            return {'job': flow['job']}
        except Exception:
            self.finish(flow, 'cancelled' if flow['cancel'].is_set() else 'failed')
            raise

    def work(self, flow, prepare):
        try:
            self.check(flow)
            if prepare:
                source = flow.get('destination') if flow['kind'] == 'mkdir' else flow['source']['path']
                destination = flow.get('destination') if flow['kind'] == 'rename' else None
                prepared = self.transport.prepare_operation(flow['kind'], source, destination, flow['cancel'])
                self.check(flow)
                # Refuse unrenderable approval before announcing a ready plan.
                self.confirmation(prepared)
                with self.lock:
                    self.check(flow)
                    flow['prepared'] = prepared
                    flow['phase'] = 'ready'
                    self.status(flow, 'ready')
            else:
                result = self.transport.apply_operation(flow['prepared'], flow['cancel'])
                if result != 'applied':
                    raise PluginError('outcome_unknown', 'Remote mutation has no acknowledged outcome')
                # An authoritative success remains success even if cancellation
                # overtook delivery of its acknowledgement.
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

    def confirmation(self, prepared):
        labels = {'mkdir': 'Create directory', 'rename': 'Rename', 'delete': 'Permanently delete'}
        def quoted(value):
            return '"' + ''.join(json.dumps(c, ensure_ascii=not c.isprintable())[1:-1]
                                 for c in value) + '"'
        title = f'{quoted(self.transport.label)} · {labels[prepared.kind]}'
        message = quoted(prepared.source)
        if prepared.destination is not None:
            message += ' → ' + quoted(prepared.destination)
        message += '. ' + '; '.join(prepared.warnings)
        # Confirmation cannot abbreviate either the target or its guarantee.
        if any(len(value.encode('utf-8')) > 160 or any(unicodedata.category(c) == 'Cc' for c in value)
               for value in (title, message)):
            raise PluginError('limit_exceeded', 'Remote paths and warnings exceed the native confirmation limit')
        return title, message

    def confirm(self, context):
        with self.lock:
            flow = self.flow
            if flow is None or flow['phase'] != 'ready':
                raise PluginError('not_found', 'No remote operation is ready for confirmation')
            self.check(flow)
            if self.worker_active:
                raise PluginError('busy', 'Remote preparation is still settling')
            title, message = self.confirmation(flow['prepared'])
            flow['phase'] = 'confirming'
            flow['surface_ready'].clear()
            self.status(flow, 'confirming')
        try:
            surface = self.app.request('ui.confirm', invocation=context['invocation'],
                                       title=title, message=message)['surface']
            self.capture_surface(flow, surface)
            return {'job': flow['job']}
        except Exception:
            with self.lock:
                if self.flow is flow and not flow['cancel'].is_set():
                    flow['phase'] = 'ready'
                    self.status(flow, 'ready')
            raise
        finally:
            flow['surface_ready'].set()

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
                    # A host cancellation may precede a successful remote
                    # acknowledgement. Acknowledge that host cancellation with
                    # an honest terminal state instead of leaving it Cancelling.
                    state = 'outcome_unknown'
                    if error.code == 'cancelled':
                        try:
                            self.app.request('job.finish', job=job, state=state)
                        except PluginError:
                            pass
        with self.lock:
            self.status(flow, 'completed' if state == 'succeeded' else state)
            if self.flow is flow:
                self.flow = None

    def cancel(self, _context):
        with self.lock:
            flow = self.flow
            if flow is None:
                return {}
            flow['cancel'].set()
            active = self.worker_active or flow['phase'] == 'starting'
        if not active:
            self.finish(flow, 'cancelled')
        return {}

    def event(self, name, data):
        terminal = name == 'job.changed' and data.get('state') in ('succeeded', 'failed', 'cancelled', 'outcome_unknown')
        if name != 'job.cancel_requested' and not terminal:
            return
        with self.lock:
            job = data.get('job')
            self.ended[job] = True
            while len(self.ended) > 16:
                del self.ended[next(iter(self.ended))]
            flow = self.flow
            if flow is None or job != flow['job'] or flow['closing']:
                return
            flow['cancel'].set()
            if self.worker_active or flow['phase'] == 'starting':
                return
            # One cleanup worker, retained until host requests return. Never
            # block the SDK cancellation executor on a host round trip.
            self.worker_active = True
            flow['done'].clear()
        def cleanup():
            try:
                self.finish(flow, 'cancelled')
            finally:
                with self.lock:
                    self.worker_active = False
                flow['done'].set()
        threading.Thread(target=cleanup, daemon=True).start()
