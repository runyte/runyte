# SPDX-License-Identifier: MPL-2.0
"""Bounded transport-neutral epoch 2 text provider used by remote examples."""
from collections import OrderedDict
from dataclasses import dataclass, field
import hashlib
import threading
import time
import unicodedata

from application import PluginError

MAX_BYTES = 8 * 1024 * 1024
CHUNK_BYTES = 128 * 1024
MAX_READS = MAX_UPLOADS = 2
HISTORY = 128
# Explicit editor grammar names; remote identities never become local paths.
SYNTAX = {'rs': 'rust', 'py': 'python', 'swift': 'swift', 'c': 'c', 'h': 'c',
          'cpp': 'cpp', 'hpp': 'cpp', 'cc': 'cpp', 'js': 'javascript', 'mjs': 'javascript',
          'ts': 'typescript', 'tsx': 'tsx', 'html': 'html', 'css': 'css', 'go': 'go',
          'sh': 'bash', 'bash': 'bash', 'java': 'java', 'kt': 'kotlin', 'sql': 'sql',
          'lua': 'lua', 'cs': 'c-sharp', 'zig': 'zig', 'cmake': 'cmake', 'proto': 'proto',
          'ini': 'ini', 'json': 'json', 'toml': 'toml', 'yaml': 'yaml', 'yml': 'yaml',
          'md': 'markdown'}


def fail(code, message):
    raise PluginError(code, message)


def token(value, limit=256):
    if not isinstance(value, str) or not value or any(unicodedata.category(c) == 'Cc' for c in value):
        return False
    try:
        return len(value.encode('utf-8')) <= limit
    except UnicodeEncodeError:
        return False


def version(data):
    return hashlib.sha256(data).hexdigest()


def valid_version(value):
    return isinstance(value, str) and len(value) == 64 and all(c in '0123456789abcdef' for c in value)


@dataclass
class Read:
    created: float
    key: str = ''
    data: bytes | None = None
    version: str = ''
    cancelled: threading.Event = field(default_factory=threading.Event)


@dataclass
class Upload:
    token: str
    job: str
    path: str
    version: str
    size: int
    created: float
    data: bytearray = field(default_factory=bytearray)
    cancelled: threading.Event = field(default_factory=threading.Event)
    committing: bool = False


class RemoteProvider:
    """No timer or background polling; all retained tables have fixed limits.

    The transport supplies observe/browse/replace operations. A failed promotion
    is permanently unknown in this process unless an authoritative transport
    completion proof is added; connection closure is deliberately not such proof.
    """
    def __init__(self, transport, name='remote', clock=time.monotonic):
        self.transport, self.name, self.clock = transport, name, clock
        self.lock = threading.RLock()
        self.reads = {}
        self.uploads = {}
        self.settled = OrderedDict()
        self.serial = 0
        self.handlers = {
            'resource.stat': self.stat, 'resource.read': self.read,
            'resource.reconcile': self.reconcile,
            'resource.write.begin': self.begin, 'resource.write.chunk': self.chunk,
            'resource.write.commit': self.commit, 'resource.write.abort': self.abort,
        }

    def requested_key(self, path):
        if not token(path, 3800):
            fail('invalid_argument', 'Remote path is empty, too long or contains control characters')
        return self.transport.connection_id + ':' + path

    def path(self, context):
        key = context.get('key')
        prefix = self.transport.connection_id + ':'
        if context.get('provider') != self.name or not token(key, 4096) or not key.startswith(prefix):
            fail('conflict', 'Resource belongs to a different remote connection')
        return key[len(prefix):]

    def job(self, context):
        job = context.get('job')
        if not token(job, 200):
            fail('invalid_argument', 'Invalid resource job')
        return job

    def remember(self, job, state):
        self.settled[job] = state
        self.settled.move_to_end(job)
        while len(self.settled) > HISTORY:
            self.settled.popitem(last=False)

    def on_event(self, name, data):
        if name != 'resource.released':
            return
        job = data.get('job')
        if not token(job, 200):
            return
        with self.lock:
            read = self.reads.pop(job, None)
            if read is not None:
                read.cancelled.set()
            # The control/event worker can overtake a queued stat handler.
            if job not in self.uploads and self.settled.get(job) in (None, 'read_released'):
                self.remember(job, 'read_released')

    def expire(self):
        now = self.clock()
        for job, read in list(self.reads.items()):
            if now - read.created > 60:
                read.cancelled.set()
                del self.reads[job]
                self.remember(job, 'read_released')
        for job, upload in list(self.uploads.items()):
            if now - upload.created > 60 and not upload.committing:
                upload.cancelled.set()
                del self.uploads[job]
                self.remember(job, 'aborted')

    def stat(self, context):
        job, path = self.job(context), self.path(context)
        with self.lock:
            self.expire()
            if job in self.reads or job in self.uploads or job in self.settled:
                fail('conflict', 'Resource job was already used')
            if len(self.reads) >= MAX_READS:
                fail('busy', 'Remote read slots are full')
            read = Read(self.clock())
            self.reads[job] = read
        try:
            canonical, data = self.transport.observe(path, read.cancelled)
            if not isinstance(data, bytes) or len(data) > MAX_BYTES:
                fail('limit_exceeded', 'Remote document exceeds 8 MiB')
            try:
                data.decode('utf-8')
            except UnicodeDecodeError:
                fail('unsupported', 'Remote resource is not UTF-8 text; use a download workflow')
            if b'\0' in data:
                fail('unsupported', 'Remote resource is binary; use a download workflow')
            key = self.requested_key(canonical)
            # Bound presentation independently from opaque identity; no endpoint
            # credentials or opaque version are rendered in the document label.
            name = canonical.rsplit('/', 1)[-1] or '/'
            label = f'{self.transport.label} · {name}'
            while len(label.encode('utf-8')) > 160:
                label = label[:-1]
            metadata = {'key': key, 'label': label, 'syntax_hint': SYNTAX.get(name.rsplit('.', 1)[-1].lower()),
                        'version': version(data), 'encoding': 'utf-8', 'bytes': len(data)}
            with self.lock:
                if self.reads.get(job) is not read:
                    fail('cancelled', 'Remote read expired')
                read.key, read.data, read.version = key, data, metadata['version']
            return {'kind': 'stat', 'value': metadata}
        except BaseException:
            with self.lock:
                if self.reads.get(job) is read:
                    del self.reads[job]
            raise

    def read(self, context):
        job = self.job(context)
        self.path(context)
        with self.lock:
            self.expire()
            read = self.reads.get(job)
            if read is None or read.data is None:
                fail('stale', 'Remote snapshot expired; open the resource again')
            if context.get('key') != read.key or context.get('version') != read.version:
                fail('stale', 'Remote snapshot identity or version changed')
            offset, limit = context.get('offset'), context.get('limit')
            if type(offset) is not int or type(limit) is not int or not 0 <= offset <= len(read.data) or not 1 <= limit <= CHUNK_BYTES:
                fail('invalid_argument', 'Invalid remote read bounds')
            end = min(len(read.data), offset + limit)
            # UTF-8 chunks end at a complete scalar. Invalid starting offsets
            # never silently shift the requested byte coordinate.
            while end < len(read.data) and end > offset and read.data[end] & 0xC0 == 0x80:
                end -= 1
            if end == offset and offset < len(read.data):
                fail('invalid_argument', 'Read limit cannot hold the next UTF-8 scalar')
            try:
                text = read.data[offset:end].decode('utf-8')
            except UnicodeDecodeError:
                fail('invalid_argument', 'Read offset splits a UTF-8 scalar')
            eof = end == len(read.data)
            result = {'kind': 'read', 'value': {'version': read.version, 'offset': offset, 'text': text, 'eof': eof}}
            if eof:
                del self.reads[job]
                self.remember(job, 'read_released')
            return result

    def begin(self, context):
        job, path = self.job(context), self.path(context)
        size = context.get('bytes')
        if context.get('mode') != 'confirmed_best_effort':
            fail('unsupported', 'This transport requires a host-confirmed best-effort write')
        if context.get('encoding') != 'utf-8' or type(size) is not int or not 0 <= size <= MAX_BYTES or not valid_version(context.get('expected_version')):
            fail('invalid_argument', 'Invalid remote upload declaration')
        with self.lock:
            self.expire()
            if job in self.uploads or job in self.settled or job in self.reads:
                fail('conflict', 'Resource job was already used or cancelled')
            if len(self.uploads) >= MAX_UPLOADS:
                fail('busy', 'Remote upload slots are full')
            self.serial += 1
            issued = f'upload:{self.serial}:{job}'
            upload = Upload(issued, job, path, context['expected_version'], size, self.clock())
            self.uploads[job] = upload
            return {'kind': 'write_started', 'value': {'upload': issued}}

    def upload(self, context):
        job = self.job(context)
        upload = self.uploads.get(job)
        # The complete issued token is retained independently of job naming.
        supplied = context.get('upload')
        if upload is None or not token(supplied) or supplied != upload.token:
            fail('not_found', 'Unknown remote upload')
        return upload

    def chunk(self, context):
        text = context.get('text')
        if not isinstance(text, str) or '\0' in text:
            fail('invalid_argument', 'Upload chunk must be UTF-8 text without NUL')
        try:
            data = text.encode('utf-8')
        except UnicodeEncodeError:
            fail('invalid_argument', 'Upload chunk contains an invalid Unicode scalar')
        with self.lock:
            self.expire()
            upload = self.upload(context)
            if upload.committing or upload.cancelled.is_set():
                fail('busy', 'Remote upload is settling')
            if type(context.get('offset')) is not int or context['offset'] != len(upload.data) or len(data) > CHUNK_BYTES or len(upload.data) + len(data) > upload.size:
                fail('invalid_argument', 'Upload chunk offset or size is invalid')
            upload.data.extend(data)
            return {'kind': 'write_chunk', 'value': {'offset': len(upload.data)}}

    def commit(self, context):
        with self.lock:
            self.expire()
            upload = self.upload(context)
            if upload.committing or upload.cancelled.is_set():
                fail('busy', 'Remote upload is settling')
            if context.get('mode') != 'confirmed_best_effort' or context.get('expected_version') != upload.version or len(upload.data) != upload.size:
                fail('conflict', 'Upload mode, version or size differs from its declaration')
            upload.committing = True
        state = 'unknown'
        try:
            data = bytes(upload.data)
            receipt = self.transport.replace(upload.path, data, upload.version, upload.cancelled)
            if receipt != version(data):
                fail('outcome_unknown', 'Remote commit returned an invalid receipt')
            state = 'committed'
            return {'kind': 'write_committed', 'value': {'version': receipt}}
        except PluginError as error:
            if error.code != 'outcome_unknown' and getattr(error, 'settled', False):
                state = 'rejected'
                return {'kind': 'write_rejected', 'value': {'error': {'code': error.code, 'message': str(error)}}}
            return {'kind': 'write_rejected', 'value': {'error': {'code': 'outcome_unknown', 'message': 'Remote replacement outcome is unknown; do not retry automatically'}}}
        except Exception:
            return {'kind': 'write_rejected', 'value': {'error': {'code': 'outcome_unknown', 'message': 'Remote replacement outcome is unknown; do not retry automatically'}}}
        finally:
            with self.lock:
                self.uploads.pop(upload.job, None)
                self.remember(upload.job, state)

    def abort(self, context):
        job = self.job(context)
        with self.lock:
            upload = self.uploads.get(job)
            if upload is not None:
                supplied = context.get('upload')
                if supplied is not None:
                    self.upload(context)
                upload.cancelled.set()
                if upload.committing:
                    fail('outcome_unknown', 'Remote replacement may already have started')
                del self.uploads[job]
            if self.settled.get(job) in ('unknown', 'committed'):
                fail('outcome_unknown', 'Remote replacement may already have committed')
            # An unseen/forgotten job can be tombstoned against a late Begin,
            # but that cannot certify any mutation from before this process.
            state = 'aborted' if upload is not None else self.settled.get(job, 'unseen_abort')
            self.remember(job, state)
            self.reads.pop(job, None)
            return {'kind': 'write_aborted', 'value': {}}

    def reconcile(self, context):
        previous = context.get('previous_write')
        if not token(previous):
            fail('invalid_argument', 'Invalid prior write reference')
        with self.lock:
            self.expire()
            if previous in self.uploads or self.settled.get(previous) not in ('committed', 'rejected', 'aborted'):
                fail('outcome_unknown', 'No authoritative proof that the previous remote write settled')
        result = self.stat(context)
        return {'kind': 'reconciled', 'value': {'metadata': result['value'], 'previous_write': previous}}
