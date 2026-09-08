# SPDX-License-Identifier: MPL-2.0
"""Small epoch 2 authoring client. Python 3.10+, standard library only."""
import concurrent.futures
import json
import sys
import threading

VERSION = 'runyte-experimental-2'
LIMIT = 1024 * 1024
MODEL_LIMIT = 4 * 1024 * 1024
MODEL_CHUNK = 128 * 1024

class PluginError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code

class Application:
    def __init__(self, name, commands, capabilities):
        self.name, self.commands, self.capabilities = name, commands, capabilities
        self.handlers = {}
        self.resource_handlers = {}
        self.on_event = lambda event, data: None
        self.on_observation = lambda event, sequence, data: self.on_event(event, data)
        self.on_input = lambda context: None
        self.on_validate = lambda context: {field: 'unavailable' for field in context['fields']}
        self._lock = threading.RLock()
        self._pending = {}
        self._accept = {}
        self._subscriptions = {}
        self._observation_slots = threading.BoundedSemaphore(32)
        self._observations = concurrent.futures.ThreadPoolExecutor(max_workers=1)
        self._serial = 0
        self._slots = threading.BoundedSemaphore(16)
        self._dispatch = threading.BoundedSemaphore(16)
        self._resource_slots = threading.BoundedSemaphore(4)
        self._resource_executor = concurrent.futures.ThreadPoolExecutor(max_workers=2)
        self._validation_slots = threading.BoundedSemaphore(2)
        self._validation_executor = concurrent.futures.ThreadPoolExecutor(max_workers=1)
        self._control_slots = threading.BoundedSemaphore(16)
        self._control = concurrent.futures.ThreadPoolExecutor(max_workers=1)
        self._closed = threading.Event()
        self._executor = concurrent.futures.ThreadPoolExecutor(max_workers=4)

    def _write(self, message):
        data = (json.dumps(message, ensure_ascii=False, separators=(',', ':')) + '\n').encode('utf-8')
        if len(data) > LIMIT:
            raise PluginError('limit_exceeded', 'Encoded message exceeds limit')
        with self._lock:
            sys.stdout.buffer.write(data)
            sys.stdout.buffer.flush()

    def request(self, method, **params):
        return self._request(method, params)

    def publish_model(self, view, expected_revision, model, *, expected_query_revision=None):
        """Atomically publish a model, staging large JSON without exposing partial rows."""
        return self._model_update(view, expected_revision, 'model', model, expected_query_revision)

    def patch_view(self, view, expected_revision, operations, header=None, *, expected_query_revision=None):
        patch = {'operations': operations}
        if header is not None:
            patch['header'] = header
        return self._model_update(view, expected_revision, 'patch', patch, expected_query_revision)

    def set_query(self, view, expected_revision, text, *, expected_query_revision=None):
        """Begin an explicit query; matching publication settles its pending state."""
        params = {'view': view, 'expected_revision': expected_revision, 'text': text}
        if expected_query_revision is not None:
            params['expected_query_revision'] = expected_query_revision
        return self.request('view.query.set', **params)

    def _model_update(self, view, expected_revision, kind, value, expected_query_revision=None):
        data = json.dumps(value, ensure_ascii=False, allow_nan=False,
                          separators=(',', ':')).encode('utf-8')
        if len(data) > MODEL_LIMIT:
            raise PluginError('limit_exceeded', 'Encoded model update exceeds limit')
        preconditions = {'expected_revision': expected_revision}
        if expected_query_revision is not None:
            preconditions['expected_query_revision'] = expected_query_revision
        if len(data) <= LIMIT - 8192:
            params = {'model': value} if kind == 'model' else value
            return self.request('view.publish' if kind == 'model' else 'view.patch',
                                view=view, **preconditions, **params)
        stage = self.request('view.stage.open', view=view, **preconditions,
                             kind=kind, bytes=len(data))['stage']
        try:
            offset = 0
            while offset < len(data):
                end = min(offset + MODEL_CHUNK, len(data))
                while end < len(data) and data[end] & 0xc0 == 0x80:
                    end -= 1
                result = self.request('view.stage.write', stage=stage, offset=offset,
                                      text=data[offset:end].decode('utf-8'))
                if result['offset'] != end:
                    raise PluginError('invalid_argument', 'Staging offset mismatch')
                offset = end
            return self.request('view.stage.commit', stage=stage)
        finally:
            try:
                self.request('view.stage.close', stage=stage)
            except PluginError:
                pass

    def get_model(self, view):
        """Read one immutable model revision, including models larger than a frame."""
        result = self.request('view.get', view=view)
        if 'model' in result:
            return result
        snapshot = self.request('view.snapshot.open', view=view,
                                expected_revision=result['revision'])
        handle = snapshot['snapshot']
        try:
            if not 0 <= snapshot['bytes'] <= MODEL_LIMIT:
                raise PluginError('limit_exceeded', 'Model snapshot exceeds limit')
            data = bytearray()
            while True:
                chunk = self.request('view.snapshot.read', snapshot=handle,
                                     offset=len(data), limit=MODEL_CHUNK)
                encoded = chunk['text'].encode('utf-8')
                if (chunk['offset'] != len(data) or len(encoded) > MODEL_CHUNK
                        or len(data) + len(encoded) > snapshot['bytes']
                        or not encoded and not chunk['eof']):
                    raise PluginError('invalid_argument', 'Invalid model snapshot chunk')
                data.extend(encoded)
                if chunk['eof']:
                    if len(data) != snapshot['bytes']:
                        raise PluginError('invalid_argument', 'Incomplete model snapshot')
                    return {'view': view, 'revision': snapshot['revision'],
                            'model': json.loads(data),
                            **({'query': result['query']} if 'query' in result else {})}
        finally:
            try:
                self.request('view.snapshot.close', snapshot=handle)
            except PluginError:
                pass

    def subscribe(self, sources, callback):
        """Deliver baseline and ordered updates as callback(event, sequence, data).

        Baselines use the SDK-only event name event.baseline. Callbacks must
        return promptly; the bounded observation lane is independent of commands.
        """
        def accept(result):
            self._subscriptions[result['subscription']] = callback
            self._queue_observation(callback, 'event.baseline', result['sequence'], result)
        return self._request('event.subscribe', {'sources': sources}, accept)

    def resync(self, subscription):
        def accept(result):
            callback = self._subscriptions.get(subscription)
            if callback is not None:
                self._queue_observation(callback, 'event.baseline', result['sequence'], result)
        return self._request('event.resync', {'subscription': subscription}, accept)

    def unsubscribe(self, subscription):
        return self._request('event.unsubscribe', {'subscription': subscription},
                             lambda _: self._subscriptions.pop(subscription, None))

    def _queue_observation(self, callback, event, sequence, data):
        if not self._observation_slots.acquire(blocking=False):
            raise PluginError('busy', 'Application observation queue full')
        def deliver():
            try:
                callback(event, sequence, data)
            finally:
                self._observation_slots.release()
        self._observations.submit(deliver)

    def _response(self, message):
        with self._lock:
            future = self._pending.pop(message['id'], None)
            accept = self._accept.pop(message['id'], None)
            if future is not None:
                if 'error' in message:
                    future.set_exception(PluginError(**message['error']))
                else:
                    try:
                        if accept is not None:
                            accept(message['result'])
                    except Exception as error:
                        future.set_exception(error)
                        raise
                    future.set_result(message['result'])

    def _request(self, method, params, accept=None):
        if not self._slots.acquire(blocking=False):
            raise PluginError('busy', 'Too many outstanding requests')
        future = concurrent.futures.Future()
        try:
            with self._lock:
                if self._closed.is_set():
                    raise PluginError('unavailable', 'Host disconnected')
                self._serial += 1
                request_id = f'p:{self._serial}'
                self._pending[request_id] = future
                if accept is not None:
                    self._accept[request_id] = accept
                self._write({'type': 'request', 'id': request_id, 'method': method, 'params': params})
            try:
                return future.result(timeout=10)
            except concurrent.futures.TimeoutError as error:
                raise PluginError('timeout', 'Control request timed out') from error
        finally:
            with self._lock:
                if 'request_id' in locals():
                    self._pending.pop(request_id, None)
                    self._accept.pop(request_id, None)
            self._slots.release()

    def _read(self):
        data = sys.stdin.buffer.readline(LIMIT + 1)
        if not data:
            raise EOFError()
        if len(data) > LIMIT or not data.endswith(b'\n'):
            raise PluginError('limit_exceeded', 'Invalid host frame')
        return json.loads(data)

    def _submit(self, message):
        if message.get('event', '').startswith('event.'):
            with self._lock:
                callback = self._subscriptions.get(message['data']['subscription'], self.on_observation)
                self._queue_observation(callback, message['event'], message['sequence'], message['data'])
            return
        # Cancellation must still run while command handlers await host replies.
        control = message.get('event') in ('job.cancel_requested', 'resource.released', 'ui.validation_cancelled')
        resource = message.get('method', '').startswith('resource.')
        validation = message.get('method') == 'ui.validate'
        slots = (self._validation_slots if validation else self._resource_slots if resource
                 else self._control_slots if control else self._dispatch)
        executor = (self._validation_executor if validation else self._resource_executor if resource
                    else self._control if control else self._executor)
        if not slots.acquire(blocking=False):
            raise PluginError('busy', 'Application dispatch queue full')
        executor.submit(self._handle, message, slots)

    def _handle(self, message, slots):
        try:
            if message['type'] == 'event':
                self.on_event(message['event'], message['data'])
                return
            params = {**message['params'], 'invocation': message['id']}
            try:
                method = message.get('method', 'command.invoke')
                if method.startswith('resource.'):
                    handler = self.resource_handlers.get(method)
                    if handler is None:
                        raise PluginError('unsupported', 'Resource method is unsupported')
                elif method == 'ui.validate':
                    handler = self.on_validate
                else:
                    handler = self.on_input if method == 'ui.submit' else self.handlers[params['command']]
                if method == 'ui.validate':
                    statuses = handler(params)
                    if (not isinstance(statuses, dict) or set(statuses) != set(params['fields'])
                            or any(status not in ('valid', 'invalid', 'unavailable') for status in statuses.values())):
                        raise PluginError('invalid_argument', 'Invalid field validation result')
                    result = {'kind': 'validation', 'surface': params['surface'],
                              'revision': params['revision'], 'fields': [
                                  {'field': field, 'status': statuses[field]} for field in params['fields']]}
                else:
                    result = handler(params) or {'job': None}
                self._write({'type': 'response', 'id': message['id'], 'result': result})
            except PluginError as error:
                detail = 'Application validation failed' if method == 'ui.validate' else str(error)
                self._write({'type': 'response', 'id': message['id'], 'error': {'code': error.code, 'message': detail}})
            except Exception:
                # Do not put exception details, paths or credentials on the wire.
                self._write({'type': 'response', 'id': message['id'], 'error': {'code': 'internal', 'message': 'Application command failed'}})
        finally:
            slots.release()

    def run(self):
        try:
            hello = self._read()
            if hello.get('type') != 'hello' or hello.get('version') != VERSION:
                raise PluginError('unsupported', 'Application requires epoch 2')
            self._write({'type': 'register', 'version': VERSION, 'name': self.name,
                         'commands': self.commands, 'required_capabilities': self.capabilities,
                         'optional_capabilities': []})
            if self._read().get('type') != 'registered':
                raise PluginError('unavailable', 'Registration refused')
            while True:
                message = self._read()
                if message['type'] == 'response':
                    self._response(message)
                elif message['type'] in ('request', 'event'):
                    self._submit(message)
        except EOFError:
            pass
        finally:
            self._closed.set()
            with self._lock:
                for future in self._pending.values():
                    future.set_exception(PluginError('unavailable', 'Host disconnected'))
                self._pending.clear()
                self._accept.clear()
                self._subscriptions.clear()
            self._resource_executor.shutdown(wait=True, cancel_futures=True)
            self._validation_executor.shutdown(wait=True, cancel_futures=True)
            self._control.shutdown(wait=True, cancel_futures=True)
            self._executor.shutdown(wait=True, cancel_futures=True)
            self._observations.shutdown(wait=True, cancel_futures=True)
