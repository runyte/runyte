# SPDX-License-Identifier: MPL-2.0
"""Small epoch 2 authoring client. Python 3.10+, standard library only."""
import concurrent.futures
import json
import sys
import threading

VERSION = 'runyte-experimental-2'
LIMIT = 1024 * 1024

class PluginError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code

class Application:
    def __init__(self, name, commands, capabilities):
        self.name, self.commands, self.capabilities = name, commands, capabilities
        self.handlers = {}
        self.on_event = lambda event, data: None
        self.on_input = lambda context: None
        self._lock = threading.RLock()
        self._pending = {}
        self._serial = 0
        self._slots = threading.BoundedSemaphore(16)
        self._dispatch = threading.BoundedSemaphore(16)
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
                self._write({'type': 'request', 'id': request_id, 'method': method, 'params': params})
            try:
                return future.result(timeout=10)
            except concurrent.futures.TimeoutError as error:
                raise PluginError('timeout', 'Control request timed out') from error
        finally:
            with self._lock:
                if 'request_id' in locals():
                    self._pending.pop(request_id, None)
            self._slots.release()

    def _read(self):
        data = sys.stdin.buffer.readline(LIMIT + 1)
        if not data:
            raise EOFError()
        if len(data) > LIMIT or not data.endswith(b'\n'):
            raise PluginError('limit_exceeded', 'Invalid host frame')
        return json.loads(data)

    def _submit(self, message):
        # Cancellation must still run while command handlers await host replies.
        control = message.get('event') == 'job.cancel_requested'
        slots = self._control_slots if control else self._dispatch
        executor = self._control if control else self._executor
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
                handler = self.on_input if message.get('method') == 'ui.submit' else self.handlers[params['command']]
                result = handler(params) or {'job': None}
                self._write({'type': 'response', 'id': message['id'], 'result': result})
            except PluginError as error:
                self._write({'type': 'response', 'id': message['id'], 'error': {'code': error.code, 'message': str(error)}})
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
                    with self._lock:
                        future = self._pending.pop(message['id'], None)
                    if future is not None:
                        if 'error' in message:
                            future.set_exception(PluginError(**message['error']))
                        else:
                            future.set_result(message['result'])
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
            self._control.shutdown(wait=True, cancel_futures=True)
            self._executor.shutdown(wait=True, cancel_futures=True)
