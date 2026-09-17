# SPDX-License-Identifier: MPL-2.0
"""Scoped Runyte context client. Python 3.10+, Unix, standard library only.

This module is independently vendorable. It never reconnects or retries a
request: delivery uncertainty must not duplicate edits or approved insertion.
"""
import json
import os
import re
import socket
import stat
import struct
import threading
import time
import sys

VERSION = 'runyte-1'
FEATURE = 'runyte.context.v1'
RUNYTE_RANGE = '>=0.3.0, <0.4.0'
FRAME_BYTES = 2 * 1024 * 1024
SCOPES = frozenset(('terminal_read', 'editor_context_read', 'buffer_edit', 'terminal_propose'))
ERROR_CODES = frozenset(('invalid_argument', 'unsupported', 'capability_denied', 'not_found', 'closed',
                        'stale', 'conflict', 'read_only', 'busy', 'limit_exceeded', 'cancelled',
                        'timeout', 'unavailable', 'internal', 'outcome_unknown', 'no_frontend', 'context_changed'))
IDENTIFIER = re.compile(r'[A-Za-z0-9:_.-]{1,256}\Z')
CREDENTIAL = re.compile(r'[0-9a-f]{64}\Z')
METHODS = {
    'buffer.list': ('editor_context_read', 'offset limit', ''),
    'buffer.read': ('editor_context_read', 'buffer expected_revision from to', ''),
    'buffer.edit': ('buffer_edit', 'buffer expected_revision changes', ''),
    'buffer.snapshot.open': ('editor_context_read', 'buffer expected_revision', ''),
    'buffer.snapshot.read': ('editor_context_read', 'snapshot from to', ''),
    'buffer.snapshot.close': ('editor_context_read', 'snapshot', ''),
    'selection.get': ('editor_context_read', 'pane', ''),
    'pane.context.list': ('editor_context_read', '', ''),
    'pane.viewport.read': ('editor_context_read', 'pane max_rows max_bytes max_cells', 'expected_revision'),
    'terminal.list': ('terminal_read', 'offset limit', ''),
    'terminal.read': ('terminal_read', 'terminal region max_rows max_bytes max_cells', 'expected_revision'),
    'terminal.snapshot.open': ('terminal_read', 'terminal region max_rows max_bytes max_cells', 'expected_revision'),
    'terminal.snapshot.read': ('terminal_read', 'snapshot offset limit', ''),
    'terminal.snapshot.close': ('terminal_read', 'snapshot', ''),
    'terminal.input.propose': ('terminal_propose', 'terminal text', 'reason'),
    'terminal.input.status': ('terminal_propose', 'proposal', ''),
    'terminal.input.cancel': ('terminal_propose', 'proposal', ''),
}
MUTATIONS = frozenset(('buffer.edit', 'terminal.input.propose', 'terminal.input.cancel'))


class ContextError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


def _fail(message):
    raise ContextError('invalid_argument', message)


def _host_error(error):
    if (not isinstance(error, dict) or not isinstance(error.get('code'), str)
            or error['code'] not in ERROR_CODES or not isinstance(error.get('message'), str)
            or len(error['message']) > 1024):
        raise ValueError('Invalid context error')
    raise ContextError(error['code'], error['message'])


def _scopes(values):
    if not isinstance(values, (list, tuple, set, frozenset)) or len(values) > 4:
        _fail('Invalid context scopes')
    if any(not isinstance(scope, str) or scope not in SCOPES for scope in values):
        _fail('Unknown context scope')
    if len(set(values)) != len(values):
        _fail('Duplicate context scope')
    return frozenset(values)


def _dependencies(values):
    if ('buffer_edit' in values and 'editor_context_read' not in values
            or 'terminal_propose' in values and 'terminal_read' not in values):
        _fail('Write scopes require corresponding read scopes')


def _names(values):
    if (not isinstance(values, list) or len(values) > 32
            or any(not isinstance(name, str) or not re.fullmatch(r'[A-Za-z0-9_.-]{1,64}', name) for name in values)
            or len(set(values)) != len(values)):
        raise ContextError('unsupported', 'Invalid context negotiation names')
    return frozenset(values)


def _integer(value, minimum=0, maximum=(1 << 64) - 1):
    return type(value) is int and minimum <= value <= maximum


def _single_line(text):
    return isinstance(text, str) and not any(ord(c) < 32 or 127 <= ord(c) <= 159
                                            or c in '\u2028\u2029' for c in text)


def _utf8_len(text):
    try:
        return len(text.encode('utf-8'))
    except UnicodeError:
        _fail('Context text must contain valid Unicode scalars')


def _peer_uid(connection):
    if hasattr(socket, 'SO_PEERCRED'):
        return struct.unpack('3i', connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize('3i')))[1]
    if hasattr(connection, 'getpeereid'):
        return connection.getpeereid()[0]
    if sys.platform == 'darwin':
        # Python builds without socket.getpeereid still expose the native
        # operation in libc. uid_t/gid_t are unsigned int on supported macOS.
        import ctypes
        library = ctypes.CDLL(None, use_errno=True)
        operation = library.getpeereid
        operation.argtypes = [ctypes.c_int, ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint)]
        operation.restype = ctypes.c_int
        uid, gid = ctypes.c_uint(), ctypes.c_uint()
        if operation(connection.fileno(), ctypes.byref(uid), ctypes.byref(gid)) != 0:
            raise OSError(ctypes.get_errno(), 'Context peer ownership unavailable')
        return uid.value
    raise ContextError('unsupported', 'Context peer ownership is unavailable on this platform')


def validate_request(method, params):
    """Validate the closed profile before issuing any socket bytes."""
    if not isinstance(method, str) or method not in METHODS:
        _fail('Method is not in the context profile')
    scope, required, optional = METHODS[method]
    required, optional = set(required.split()), set(optional.split())
    if not isinstance(params, dict) or not required <= params.keys() or params.keys() - required - optional:
        _fail('Missing or unknown context parameter')
    for key in ('buffer', 'pane', 'terminal', 'snapshot', 'proposal', 'expected_revision'):
        if key not in params or key == 'expected_revision' and key in optional and params[key] is None:
            continue
        if not isinstance(params[key], str) or not IDENTIFIER.fullmatch(params[key]):
            _fail('Invalid context identifier')
    for key in ('offset', 'from', 'to'):
        if key in params and not _integer(params[key]):
            _fail('Invalid context offset')
    if 'limit' in params:
        maximum = 1000 if method == 'terminal.snapshot.read' else 256
        if not _integer(params['limit'], 1, maximum) or params['offset'] + params['limit'] >= 1 << 64:
            _fail('Invalid context page')
    if 'from' in params and (params['from'] > params['to'] or params['to'] - params['from'] > 262144):
        _fail('Invalid scalar range')
    for key, maximum in (('max_rows', 1000), ('max_bytes', 262144), ('max_cells', 262144)):
        if key in params and not _integer(params[key], 1, maximum):
            _fail('Invalid context read bound')
    if 'region' in params and params['region'] not in ('screen', 'tail'):
        _fail('Invalid terminal region')
    if method == 'buffer.edit':
        changes = params['changes']
        if not isinstance(changes, list) or not 1 <= len(changes) <= 1024:
            _fail('Invalid buffer changes')
        total, previous = 0, None
        for change in changes:
            if not isinstance(change, dict) or set(change) != {'from', 'to', 'text'}:
                _fail('Invalid buffer change')
            start, end, text = change['from'], change['to'], change['text']
            if not _integer(start) or not _integer(end) or start > end or not isinstance(text, str):
                _fail('Invalid buffer change range')
            if previous is not None and (previous[1] > start or previous[0] == start):
                _fail('Overlapping buffer changes')
            previous = (start, end)
            total += _utf8_len(text)
        if total > 524288:
            _fail('Buffer changes exceed replacement limit')
    if method == 'terminal.input.propose':
        text, reason = params['text'], params.get('reason')
        if not _single_line(text) or not 1 <= _utf8_len(text) <= 4096:
            _fail('Proposal must be one bounded line without controls')
        if reason is not None and (not _single_line(reason) or len(reason) > 256 or _utf8_len(reason) > 1024):
            _fail('Proposal reason exceeds limits')
    return scope


def _unique(pairs):
    value = {}
    for key, item in pairs:
        key.encode('utf-8')
        if key in value:
            raise ValueError('Duplicate context field')
        value[key] = item
    return value


def _bounded(value, depth=0, budget=None):
    if budget is None:
        budget = [32768]
    budget[0] -= 1
    if depth > 16 or budget[0] < 0:
        raise ValueError('Context JSON structure exceeds limits')
    if isinstance(value, dict):
        if len(value) > 4096:
            raise ValueError('Context object exceeds limits')
        for child in value.values():
            _bounded(child, depth + 1, budget)
    elif isinstance(value, list):
        if len(value) > 4096:
            raise ValueError('Context array exceeds limits')
        for child in value:
            _bounded(child, depth + 1, budget)
    elif isinstance(value, float) or type(value) is int and not -(1 << 63) <= value < 1 << 64:
        raise ValueError('Context numeric value exceeds limits')
    elif isinstance(value, str):
        value.encode('utf-8')


class ContextClient:
    def __init__(self, endpoint, credential, *, name='Runyte context bridge',
                 required_scopes=('terminal_read',), optional_scopes=('editor_context_read',),
                 timeout=2.0, expected_incarnation=None):
        self._socket = None
        self._buffer = bytearray()
        self._lock = threading.Lock()
        self._next = 0
        self._limit = FRAME_BYTES
        self.capabilities = frozenset()
        self.workspace = None
        if not isinstance(credential, str) or not CREDENTIAL.fullmatch(credential):
            _fail('Invalid private context credential')
        if not _single_line(name) or not 1 <= _utf8_len(name) <= 128:
            _fail('Invalid reader name')
        required, optional = _scopes(required_scopes), _scopes(optional_scopes)
        if required & optional:
            _fail('Context scope lists overlap')
        _dependencies(required | optional)
        if not isinstance(timeout, (int, float)) or isinstance(timeout, bool) or not 0 < timeout <= 30:
            _fail('Context timeout must be between zero and thirty seconds')
        self.timeout = timeout
        try:
            metadata = os.lstat(endpoint)
            if not stat.S_ISSOCK(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077:
                raise ContextError('capability_denied', 'Context endpoint is not an owner-private socket')
            self._socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            deadline = time.monotonic() + timeout
            self._socket.settimeout(timeout)
            self._socket.connect(os.fspath(endpoint))
            if _peer_uid(self._socket) != os.geteuid():
                raise ContextError('capability_denied', 'Context endpoint owner changed')
            self._send({'type': 'authenticate', 'credential': credential}, deadline)
            hello = self._receive(deadline)
            if hello.get('type') == 'registration_error':
                _host_error(hello)
            if hello.get('type') != 'hello' or hello.get('version') != VERSION or FEATURE not in _names(hello.get('features')):
                raise ContextError('unsupported', 'Host does not support the context profile')
            if not required <= _names(hello.get('capabilities')):
                raise ContextError('capability_denied', 'Required context scopes are unavailable')
            if not re.fullmatch(r'0\.3\.(0|[1-9][0-9]*)(?:\+[A-Za-z0-9.-]+)?', hello.get('host_version', '')):
                raise ContextError('unsupported', 'Host release is outside the supported stable range')
            self._set_limits(hello.get('limits'))
            self.workspace = hello.get('workspace')
            if expected_incarnation is not None and (not isinstance(self.workspace, dict)
                    or self.workspace.get('host_incarnation') != expected_incarnation):
                raise ContextError('stale', 'Host incarnation changed')
            self._send({'type': 'register', 'version': VERSION, 'runyte': RUNYTE_RANGE,
                        'name': name, 'commands': [], 'required_features': [FEATURE],
                        'optional_features': [], 'required_capabilities': sorted(required),
                        'optional_capabilities': sorted(optional)}, deadline)
            registered = self._receive(deadline)
            if registered.get('type') == 'registration_error':
                _host_error(registered)
            if (registered.get('type') != 'registered'
                    or _names(registered.get('features')) != {FEATURE}
                    or registered.get('commands') != []
                    or registered.get('runyte') != RUNYTE_RANGE):
                raise ContextError('capability_denied', 'Context registration refused')
            capabilities = _scopes(registered.get('capabilities'))
            if not required <= capabilities or not capabilities <= required | optional:
                raise ContextError('capability_denied', 'Host returned inconsistent context scopes')
            _dependencies(capabilities)
            self._set_limits(registered.get('limits'))
            self.capabilities = capabilities
        except (OSError, EOFError, ValueError, TypeError, RecursionError, ContextError) as error:
            self.close()
            if isinstance(error, ContextError):
                raise
            raise ContextError('unavailable', 'Context connection could not be established') from None

    def _set_limits(self, limits):
        if not isinstance(limits, dict) or not _integer(limits.get('line_bytes'), 256, FRAME_BYTES):
            raise ContextError('unsupported', 'Invalid context frame limit')
        self._limit = min(self._limit, limits['line_bytes'])

    def _remaining(self, deadline):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('Context deadline elapsed')
        self._socket.settimeout(remaining)

    def _send(self, value, deadline):
        data = json.dumps(value, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode('utf-8') + b'\n'
        if len(data) > self._limit:
            raise ContextError('limit_exceeded', 'Context request exceeds frame limit')
        self._remaining(deadline)
        self._socket.sendall(data)

    def _receive(self, deadline):
        while b'\n' not in self._buffer:
            if len(self._buffer) >= self._limit:
                raise ValueError('Context reply exceeds frame limit')
            self._remaining(deadline)
            data = self._socket.recv(min(65536, self._limit - len(self._buffer)))
            if not data:
                raise EOFError('Context host disconnected')
            self._buffer.extend(data)
        index = self._buffer.index(b'\n')
        raw = bytes(self._buffer[:index])
        del self._buffer[:index + 1]
        value = json.loads(raw, object_pairs_hook=_unique, parse_constant=lambda _: (_ for _ in ()).throw(ValueError('Invalid JSON constant')))
        if not isinstance(value, dict):
            raise ValueError('Invalid context envelope')
        _bounded(value)
        return value

    def request(self, method, params):
        scope = validate_request(method, params)
        if scope not in self.capabilities:
            raise ContextError('capability_denied', 'Context scope was not granted')
        with self._lock:
            if self._socket is None:
                raise ContextError('unavailable', 'Context connection is closed')
            if self._next >= 1024:
                raise ContextError('limit_exceeded', 'Context request lifetime exhausted; reconnect explicitly')
            self._next += 1
            request_id = f'c:{self._next}'
            try:
                deadline = time.monotonic() + self.timeout
                self._send({'type': 'request', 'id': request_id, 'method': method, 'params': params}, deadline)
                reply = self._receive(deadline)
                if reply.get('type') != 'response' or reply.get('id') != request_id or ('result' in reply) == ('error' in reply):
                    raise ValueError('Mismatched context response')
                if 'error' in reply:
                    _host_error(reply['error'])
                if not isinstance(reply['result'], dict):
                    raise ValueError('Invalid context result')
                return reply['result']
            except (OSError, EOFError, ValueError, RecursionError, TypeError):
                self.close()
                code = 'outcome_unknown' if method in MUTATIONS else 'unavailable'
                raise ContextError(code, 'Context response was not confirmed; request was not retried') from None

    def close(self):
        if self._socket is not None:
            self._socket.close()
            self._socket = None
        self._buffer.clear()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
