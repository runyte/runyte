# SPDX-License-Identifier: MPL-2.0
"""Stable application authoring client. Python 3.10+, standard library only."""
import base64
import binascii
import concurrent.futures
from collections import deque
import json
import math
import os
import selectors
import sys
import threading
import time

VERSION = 'runyte-1'
LIMIT = 1024 * 1024
MODEL_LIMIT = 4 * 1024 * 1024
MODEL_CHUNK = 128 * 1024
STATE_LIMIT = LIMIT - 4096

class PluginError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code

class _HostDisconnected(PluginError):
    def __init__(self):
        super().__init__('unavailable', 'Host disconnected')


class _WireIO:
    """One bounded writer; a persistent close signal wakes both I/O selectors."""
    MAX_MESSAGES = 16
    MAX_BYTES = 4 * LIMIT

    def __init__(self, failed, output=None):
        self.failed = failed
        self.condition = threading.Condition()
        self.queue = deque()
        self.bytes = 0
        self.closed = False
        self.thread = None
        self.output = output
        self.wake_read, self.wake_write = os.pipe()
        os.set_blocking(self.wake_write, False)

    def enqueue(self, data):
        with self.condition:
            if self.closed:
                raise _HostDisconnected()
            if len(self.queue) >= self.MAX_MESSAGES or self.bytes + len(data) > self.MAX_BYTES:
                raise PluginError('busy', 'Application output queue full')
            if self.thread is None:
                if self.output is None:
                    self.output = sys.stdout.fileno()
                os.set_blocking(self.output, False)
                thread = threading.Thread(target=self._run, name='runyte-plugin-writer', daemon=True)
                thread.start()
                self.thread = thread
            # The first element remains charged throughout every partial write.
            self.queue.append(data)
            self.bytes += len(data)
            self.condition.notify()

    def _run(self):
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(self.output, selectors.EVENT_WRITE, 'output')
                selector.register(self.wake_read, selectors.EVENT_READ, 'closed')
                while True:
                    with self.condition:
                        self.condition.wait_for(lambda: self.closed or self.queue)
                        if self.closed:
                            return
                        data = self.queue[0]
                    offset = 0
                    while offset < len(data):
                        for key, _ in selector.select():
                            if key.data == 'closed':
                                return
                            with self.condition:
                                if self.closed:
                                    return
                                try:
                                    written = os.write(self.output, memoryview(data)[offset:])
                                except BlockingIOError:
                                    continue
                            if written == 0:
                                raise OSError('Output closed')
                            offset += written
                    with self.condition:
                        if self.closed:
                            return
                        self.queue.popleft()
                        self.bytes -= len(data)
        except (OSError, ValueError):
            self.failed()

    def close(self):
        with self.condition:
            if self.closed:
                return
            self.closed = True
            self.queue.clear()
            self.bytes = 0
            try:
                os.write(self.wake_write, b'x')
            except (BlockingIOError, OSError):
                pass
            self.condition.notify_all()

    def finish(self):
        self.close()
        if self.thread is not None:
            self.thread.join()
        with self.condition:
            if self.wake_read is not None:
                os.close(self.wake_read)
                os.close(self.wake_write)
                self.wake_read = self.wake_write = None

def _validate_state_document(document):
    # Keep only one iterator per nesting level; cycles hit the same depth bound.
    stack = [(iter((document,)), 1)]
    nodes = characters = 0
    while stack:
        iterator, depth = stack[-1]
        try:
            value = next(iterator)
        except StopIteration:
            stack.pop()
            continue
        nodes += 1
        if depth > 16 or nodes > 16384:
            raise PluginError('limit_exceeded', 'State document exceeds its structure limits')
        if value is None or isinstance(value, bool):
            continue
        if isinstance(value, int):
            if not -(1 << 63) <= value <= (1 << 64) - 1:
                raise PluginError('invalid_argument', 'State integer is outside the supported 64-bit range; use a string for larger identifiers')
        elif isinstance(value, float):
            if not math.isfinite(value):
                raise PluginError('invalid_argument', 'State must be finite JSON data')
        elif isinstance(value, str):
            characters += len(value)
        elif isinstance(value, (dict, list)):
            if len(value) > 1024:
                raise PluginError('limit_exceeded', 'State container exceeds 1024 entries')
            if isinstance(value, dict):
                for key in value:
                    if not isinstance(key, str):
                        raise PluginError('invalid_argument', 'State object keys must be strings')
                    characters += len(key)
                children = value.values()
            else:
                children = value
            stack.append((iter(children), depth + 1))
        else:
            raise PluginError('invalid_argument', 'State must be finite JSON data')
        # A cheap lower bound prevents huge strings from reaching the encoder;
        # the exact UTF-8/escaping budget is checked on the resulting bytes.
        if characters > STATE_LIMIT:
            raise PluginError('limit_exceeded', 'State document exceeds its byte limit')

# Version comparison deliberately implements the documented bounded interval
# language, not a third-party package manager's range conventions.
_MAX_COMPONENT = (1 << 64) - 1


def _version(value):
    if not isinstance(value, str) or not 1 <= len(value.encode('utf-8')) <= 256:
        raise ValueError('Invalid version length')
    version, plus, build = value.partition('+')
    core, dash, prerelease = version.partition('-')
    for suffix, numeric, present in ((build, False, plus), (prerelease, True, dash)):
        if present:
            for part in suffix.split('.'):
                if not part or any(not (c.isascii() and (c.isalnum() or c == '-')) for c in part):
                    raise ValueError('Invalid version identifier')
                if numeric and part.isdigit() and len(part) > 1 and part[0] == '0':
                    raise ValueError('Leading zero in prerelease identifier')
    parts = core.split('.')
    if len(parts) != 3 or any(not p or not p.isascii() or not p.isdigit() or (len(p) > 1 and p[0] == '0') for p in parts):
        raise ValueError('Version needs three numeric components')
    values = tuple(map(int, parts))
    if any(v > _MAX_COMPONENT for v in values):
        raise ValueError('Version component overflow')
    return values, prerelease if dash else None


class ReleaseRange:
    """One bounded final-release interval, or one exact release/prerelease."""
    def __init__(self, value):
        if not isinstance(value, str) or not 1 <= len(value.encode('utf-8')) <= 256:
            raise ValueError('Range must contain 1–256 bytes')
        if '+' in value or any(c.isspace() and c not in ' \t' for c in value):
            raise ValueError('Invalid range whitespace or build metadata')
        value = value.strip(' \t')
        self.prerelease = None
        if value.startswith('='):
            exact = value[1:].strip(' \t')
            core, self.prerelease = _version(exact)
            self.first = self.last = self._ordinal(core)
            self.normalized = '=' + exact
            return
        terms = value.split(',')
        if len(terms) != 2:
            raise ValueError('Range needs one lower and one upper bound, or =version')
        lower = upper = None
        for term in terms:
            term = term.strip(' \t')
            operator = next((op for op in ('>=', '<=', '>', '<') if term.startswith(op)), None)
            if operator is None:
                raise ValueError('Unsupported range comparator')
            spelling = term[len(operator):].strip(' \t')
            core, pre = _version(spelling)
            if pre is not None:
                raise ValueError('Prereleases require an exact =version range')
            ordinal = self._ordinal(core)
            if operator.startswith('>'):
                if lower is not None:
                    raise ValueError('Duplicate lower bound')
                lower = (ordinal + (operator == '>'), operator + spelling)
            else:
                if upper is not None:
                    raise ValueError('Duplicate upper bound')
                upper = (ordinal - (operator == '<'), operator + spelling)
        if lower is None or upper is None or lower[0] > upper[0]:
            raise ValueError('Empty or incomplete range')
        self.first, self.last = lower[0], upper[0]
        self.normalized = lower[1] + ', ' + upper[1]

    @staticmethod
    def _ordinal(core):
        return (core[0] << 128) | (core[1] << 64) | core[2]

    def contains(self, version):
        core, pre = _version(version)
        return pre == self.prerelease and self.first <= self._ordinal(core) <= self.last

    def is_subset_of(self, authored):
        return self.prerelease == authored.prerelease and self.first >= authored.first and self.last <= authored.last


# Stable v1 minimum capacities and deadline allowances. Extra host inventory
# keys are informational; the client never expands its own fixed wire bounds.
_LIMIT_FLOORS = {'line_bytes': 1048576,
 'commands': 64,
 'requests': 16,
 'finite_jobs': 4,
 'control_queue_messages': 32,
 'control_queue_bytes': 4194304,
 'control_deadline_seconds': 10,
 'job_deadline_seconds': 3600,
 'cancellation_seconds': 2,
 'process_handles': 4,
 'process_io_bytes': 65536,
 'process_output_bytes': 1048576,
 'process_write_seconds': 5,
 'activity_leases': 2,
 'activity_seconds': 600,
 'settings_bytes': 65536,
 'state_document_bytes': 1044480,
 'state_requests': 1}
_RESOURCE_FLOORS = {'plugin_instances': 8,
 'views': 16,
 'model_bytes': 4194304,
 'model_rows': 10000,
 'model_projection_bytes': 4194304,
 'model_stages': 2,
 'model_snapshots': 2,
 'model_chunk_bytes': 131072,
 'model_idle_seconds': 30,
 'text_chunk_bytes': 262144,
 'transaction_changes': 1024,
 'transaction_text_bytes': 524288,
 'text_snapshots': 2,
 'text_snapshot_bytes': 16777216,
 'text_snapshot_idle_seconds': 30,
 'retained_payload_bytes': 50331648,
 'host_retained_payload_bytes': 167772160,
 'subscriptions': 32,
 'watched_sources': 256,
 'observation_state_bytes': 4096,
 'input_surfaces': 1,
 'input_fields': 16}


def _validate_limits(value):
    if not isinstance(value, dict):
        raise ValueError('Invalid host limits')
    def validate_fields(fields, floors):
        for name, minimum in floors.items():
            number = fields.get(name)
            if type(number) is not int or not minimum <= number <= _MAX_COMPONENT:
                raise ValueError('Missing, invalid or insufficient host limit')
    validate_fields(value, _LIMIT_FLOORS)
    if 'resources' in value:
        resources = value['resources']
        if not isinstance(resources, dict):
            raise ValueError('Invalid host resource inventory')
        validate_fields(resources, _RESOURCE_FLOORS)
    return value


def _names(values, *, features=False):
    if not isinstance(values, (list, tuple)) or len(values) > 32 or len(set(values)) != len(values):
        raise ValueError('Invalid negotiation set')
    limit = 64 if features else 48
    for name in values:
        if not isinstance(name, str) or not 1 <= len(name) <= limit:
            raise ValueError('Invalid negotiation name')
        if any(not (c.isascii() and (c.isalnum() or c in ('-._' if features else '-'))) for c in name):
            raise ValueError('Invalid negotiation name')
        if not features and name != name.lower():
            raise ValueError('Invalid capability name')
    return frozenset(values)


class Application:
    def __init__(self, name, commands, capabilities, *, runyte, optional_capabilities=(), required_features=(), optional_features=(), settings_schema=None):
        self.name, self.commands, self.capabilities = name, commands, capabilities
        self.runyte = ReleaseRange(runyte)
        self.optional_capabilities = list(optional_capabilities)
        self.required_features = list(required_features)
        self.optional_features = list(optional_features)
        required = _names(capabilities)
        optional = _names(self.optional_capabilities)
        features = _names(self.required_features, features=True)
        optional_features = _names(self.optional_features, features=True)
        if required & optional or len(required | optional) > 32 or features & optional_features or len(features | optional_features) > 32:
            raise ValueError('Overlapping or excessive negotiation sets')
        self.host_version = None
        self.granted_capabilities = frozenset()
        self.features = frozenset()
        self.limits = {}
        self.settings_schema = settings_schema
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
        self._io = None
        self._input = bytearray()
        self._executor = concurrent.futures.ThreadPoolExecutor(max_workers=4)

    def _write(self, message):
        data = (json.dumps(message, ensure_ascii=False, separators=(',', ':')) + '\n').encode('utf-8')
        if len(data) > LIMIT:
            raise PluginError('limit_exceeded', 'Encoded message exceeds limit')
        with self._lock:
            if self._closed.is_set():
                raise _HostDisconnected()
            try:
                self._wire_io().enqueue(data)
            except (PluginError, OSError, RuntimeError):
                # Requests can fail before admission. A lost reliable reply or
                # registration cannot leave a seemingly healthy connection.
                if message.get('type') != 'request':
                    self._disconnect()
                raise

    def _wire_io(self):
        with self._lock:
            if self._io is None:
                self._io = _WireIO(self._disconnect)
            return self._io

    def _disconnect(self):
        self._closed.set()
        with self._lock:
            for future in self._pending.values():
                if not future.done():
                    future.set_exception(_HostDisconnected())
            self._pending.clear()
            self._accept.clear()
            self._subscriptions.clear()
            if self._io is not None:
                self._io.close()

    def request(self, method, **params):
        # Leave delivery headroom beyond the host's authoritative state deadline.
        timeout = 12 if method in ('state.get', 'state.set', 'state.delete') else 10
        return self._request(method, params, timeout=timeout)

    def start_process(self, label, executable, args=(), *, cwd=None, capture_stderr=False):
        """Start one host-managed argument vector; stdout stays outside the protocol."""
        params = {'label': label, 'executable': executable, 'args': list(args),
                  'capture_stderr': capture_stderr}
        if cwd is not None:
            params['cwd'] = cwd
        return self.request('process.start', **params)

    def publish_notification(self, severity, title, body=''):
        """Publish bounded owner-labelled feedback without taking focus."""
        return self.request('notification.publish', severity=severity, title=title, body=body)

    def acquire_activity(self, title, duration_seconds=600):
        """Protect deliberate continuing work; renewal is an explicit owner decision."""
        return self.request('activity.acquire', title=title, duration_seconds=duration_seconds)

    def renew_activity(self, lease, duration_seconds=600):
        """Renew a still-active lease once, without replaying or reviving expired work."""
        return self.request('activity.renew', lease=lease, duration_seconds=duration_seconds)

    def get_activity(self, lease):
        return self.request('activity.get', lease=lease)

    def release_activity(self, lease):
        """Acknowledge that continuing work and its cleanup have finished."""
        return self.request('activity.release', lease=lease)

    def cancel_activity(self, lease):
        """Request cooperative cleanup; release acknowledges its completion."""
        return self.request('activity.cancel', lease=lease)

    def get_settings(self):
        """Read this owner's immutable configured settings after registration."""
        return self.request('settings.get')['settings']

    def get_state(self):
        """Read nonsecret workspace state and its opaque content revision."""
        return self.request('state.get')

    def set_state(self, expected_revision, version, data):
        """Replace one versioned document by content-CAS; never retry or migrate."""
        if isinstance(version, bool) or not isinstance(version, int) or not 0 <= version <= 0xffffffff:
            raise PluginError('invalid_argument', 'State version must be an unsigned 32-bit integer')
        document = {'version': version, 'data': data}
        try:
            _validate_state_document(document)
            encoded = json.dumps(document, ensure_ascii=False, allow_nan=False,
                                 separators=(',', ':')).encode('utf-8')
            if len(encoded) > STATE_LIMIT:
                raise PluginError('limit_exceeded', 'State document exceeds its byte limit')
            # Request admission may wait on another writer; retain the exact validated value.
            document = json.loads(encoded)
            _validate_state_document(document)
        except (TypeError, ValueError, OverflowError, UnicodeError, RuntimeError) as error:
            raise PluginError('invalid_argument', 'State must be finite JSON data') from error
        return self.request('state.set', expected_revision=expected_revision, document=document)

    def delete_state(self, expected_revision):
        """Delete only the observed revision; inspect conflicts or unknown outcomes."""
        return self.request('state.delete', expected_revision=expected_revision)

    def open_terminal(self, invocation, label, executable, args=(), *, cwd=None):
        """Hand a native terminal session to the user with exact argument boundaries."""
        params = {'invocation': invocation, 'label': label, 'executable': executable,
                  'args': list(args)}
        if cwd is not None:
            params['cwd'] = cwd
        return self.request('terminal.open', **params)

    def open_url(self, invocation, url):
        """Request one foreground system-browser handoff; never replay a failure."""
        return self.request('external.open', invocation=invocation,
                            target={'kind': 'url', 'url': url})

    def open_file_externally(self, invocation, path):
        """Hand an existing workspace file to its system handler."""
        return self.request('external.open', invocation=invocation,
                            target={'kind': 'file', 'path': path})

    def read_process(self, process, stream, offset, limit=65536):
        """Read retained output once. The returned data field contains decoded bytes."""
        if not 1 <= limit <= 65536:
            raise PluginError('invalid_argument', 'Helper read limit must be 1–65536 bytes')
        result = self.request('process.read', process=process, stream=stream, offset=offset, limit=limit)
        encoded = result['data']
        if not isinstance(encoded, str) or len(encoded) > ((limit + 2) // 3) * 4:
            raise PluginError('invalid_argument', 'Invalid helper output encoding')
        try:
            data = base64.b64decode(encoded, validate=True)
        except (binascii.Error, ValueError) as error:
            raise PluginError('invalid_argument', 'Invalid helper output encoding') from error
        if (len(data) > limit or result['process'] != process or result['stream'] != stream
                or result['offset'] != offset or result['next'] != offset + len(data)):
            raise PluginError('invalid_argument', 'Invalid helper output chunk')
        return {**result, 'data': data}

    def write_process(self, process, data, *, eof=False):
        """Acknowledge bytes written to the pipe, without claiming helper consumption."""
        if not isinstance(data, (bytes, bytearray, memoryview)):
            raise PluginError('invalid_argument', 'Helper input must be bytes')
        size = data.nbytes if isinstance(data, memoryview) else len(data)
        if size > 65536:
            raise PluginError('limit_exceeded', 'Helper input exceeds 65536 bytes')
        result = self.request('process.write', process=process,
                              data=base64.b64encode(bytes(data)).decode('ascii'), eof=eof)
        if (result['process'] != process or result['written'] != size
                or eof and not result['stdin_closed']):
            raise PluginError('outcome_unknown', 'Helper input acknowledgement was incomplete')
        return result

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

    def _request(self, method, params, accept=None, *, timeout=10):
        deadline = time.monotonic() + timeout
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
                try:
                    self._write({'type': 'request', 'id': request_id, 'method': method, 'params': params})
                except OSError as error:
                    if method in ('state.set', 'state.delete'):
                        raise PluginError('outcome_unknown', 'State mutation outcome is unknown; read state before retrying') from error
                    raise
            try:
                return future.result(timeout=max(0, deadline - time.monotonic()))
            except _HostDisconnected as error:
                if method in ('state.set', 'state.delete'):
                    raise PluginError('outcome_unknown', 'State mutation outcome is unknown; read state before retrying') from error
                raise
            except concurrent.futures.TimeoutError as error:
                # A queued or partial frame must not become a surprise mutation
                # after its caller times out. Retire this real wire connection;
                # host-reported timeout errors do not enter this branch.
                if self._io is not None:
                    self._disconnect()
                if method in ('state.set', 'state.delete'):
                    raise PluginError('outcome_unknown', 'State mutation outcome is unknown; read state before retrying') from error
                raise PluginError('timeout', 'Control request timed out') from error
        finally:
            with self._lock:
                if 'request_id' in locals():
                    self._pending.pop(request_id, None)
                    self._accept.pop(request_id, None)
            self._slots.release()

    def _read(self):
        wire = self._wire_io()
        with selectors.DefaultSelector() as selector:
            descriptor = sys.stdin.fileno()
            selector.register(descriptor, selectors.EVENT_READ, 'input')
            selector.register(wire.wake_read, selectors.EVENT_READ, 'closed')
            while True:
                if self._closed.is_set():
                    raise EOFError()
                newline = self._input.find(b'\n')
                if newline >= 0:
                    data = bytes(self._input[:newline + 1])
                    del self._input[:newline + 1]
                    return json.loads(data.decode('utf-8'))
                if len(self._input) >= LIMIT:
                    raise PluginError('limit_exceeded', 'Invalid host frame')
                for key, _ in selector.select():
                    if key.data == 'closed':
                        raise EOFError()
                    data = os.read(descriptor, min(65536, LIMIT - len(self._input)))
                    if not data:
                        if self._input:
                            raise PluginError('limit_exceeded', 'Invalid host frame')
                        raise EOFError()
                    self._input.extend(data)

    def _submit(self, message):
        if message.get('event', '').startswith('event.'):
            with self._lock:
                callback = self._subscriptions.get(message['data']['subscription'], self.on_observation)
                self._queue_observation(callback, message['event'], message['sequence'], message['data'])
            return
        # Cancellation must still run while command handlers await host replies.
        control = message.get('event') in ('job.cancel_requested', 'activity.cancel_requested',
                                         'resource.released', 'ui.validation_cancelled')
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
            if not isinstance(hello, dict) or hello.get('type') != 'hello' or hello.get('version') != VERSION:
                raise PluginError('unsupported_protocol', 'Application requires runyte-1')
            try:
                host_version = hello['host_version']
                if not self.runyte.contains(host_version):
                    raise PluginError('unsupported_release', f'Host {host_version} is outside {self.runyte.normalized}')
                _validate_limits(hello['limits'])
                supported = _names(hello['capabilities'])
                supported_features = _names(hello['features'], features=True)
                if not set(self.capabilities) <= supported:
                    raise PluginError('unsupported_capability', 'Required host capability is unavailable')
                if not set(self.required_features) <= supported_features:
                    raise PluginError('unsupported_feature', 'Required host feature is unavailable')
                registration = {'type': 'register', 'version': VERSION, 'runyte': self.runyte.normalized,
                                'name': self.name, 'commands': self.commands,
                                'required_capabilities': self.capabilities,
                                'optional_capabilities': self.optional_capabilities,
                                'required_features': self.required_features, 'optional_features': self.optional_features}
                if self.settings_schema is not None:
                    registration['settings_schema'] = self.settings_schema
                self._write(registration)
                registered = self._read()
                if not isinstance(registered, dict):
                    raise ValueError('Invalid registration envelope')
                if registered.get('type') == 'registration_error':
                    raise PluginError(registered.get('code', 'invalid_registration'), registered.get('message', 'Registration refused'))
                if registered.get('type') != 'registered':
                    raise PluginError('unavailable', 'Registration refused')
                granted = _names(registered['capabilities'])
                selected = _names(registered['features'], features=True)
                effective = ReleaseRange(registered['runyte'])
                if (not set(self.capabilities) <= granted or not granted <= supported
                        or not granted <= set(self.capabilities) | set(self.optional_capabilities)
                        or not set(self.required_features) <= selected or not selected <= supported_features
                        or not selected <= set(self.required_features) | set(self.optional_features)
                        or not effective.is_subset_of(self.runyte) or not effective.contains(host_version)):
                    raise PluginError('invalid_registration', 'Invalid negotiated registration')
                limits = _validate_limits(registered['limits'])
                self.host_version = host_version
                self.granted_capabilities, self.features = granted, selected
                self.limits = limits
            except (KeyError, TypeError, ValueError) as error:
                raise PluginError('invalid_registration', 'Invalid host handshake') from error
            while True:
                message = self._read()
                if message['type'] == 'response':
                    self._response(message)
                elif message['type'] in ('request', 'event'):
                    self._submit(message)
        except EOFError:
            pass
        finally:
            self._disconnect()
            if self._io is not None:
                self._io.finish()
            self._resource_executor.shutdown(wait=True, cancel_futures=True)
            self._validation_executor.shutdown(wait=True, cancel_futures=True)
            self._control.shutdown(wait=True, cancel_futures=True)
            self._executor.shutdown(wait=True, cancel_futures=True)
            self._observations.shutdown(wait=True, cancel_futures=True)
