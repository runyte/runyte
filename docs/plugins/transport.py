# SPDX-License-Identifier: MPL-2.0
"""Shared bounded remote workers and conservative mutation outcome errors."""
from dataclasses import dataclass
import threading
import time
import hashlib
import os
import stat

from application import PluginError


class TransportError(PluginError):
    def __init__(self, code, message, *, outcome_unknown=False, settled=True):
        self.outcome_unknown = outcome_unknown
        self.settled = settled and not outcome_unknown
        super().__init__('outcome_unknown' if outcome_unknown else code, message)


@dataclass(frozen=True)
class PreparedOperation:
    connection_id: str
    kind: str
    source: str
    destination: str | None
    entry_kind: str
    fingerprint: tuple
    warnings: tuple[str, ...]


def _fail(code, message):
    raise TransportError(code, message)


class _Operation:
    def __init__(self, cancel, timeout):
        self.lock = threading.Lock()
        self.external = cancel
        self.deadline = time.monotonic() + timeout
        self.cancelled = False
        self.promotion_started = False
        self.done = threading.Event()
        self.client = None
        self.result = None
        self.error = None

    def _check_locked(self):
        if self.cancelled or (self.external is not None and self.external.is_set()):
            _fail('cancelled', 'Remote operation cancelled')
        if time.monotonic() >= self.deadline:
            _fail('timeout', 'Remote operation timed out')

    def check(self):
        with self.lock:
            self._check_locked()

    def remaining(self):
        self.check()
        return max(0.001, self.deadline - time.monotonic())

    def promote(self):
        # The caller commits cancellation under this same lock. A definite
        # cancellation can therefore never be followed by a late rename.
        with self.lock:
            self._check_locked()
            self.promotion_started = True

    def close(self):
        with self.lock:
            client = self.client
        if client is not None:
            # Local transport shutdown does not wait for a remote close reply.
            try:
                client.close()
            except Exception:
                pass


class BoundedTransport:
    def _prepare_operation(self, client, operation, kind, source, destination):
        if kind not in ('mkdir', 'rename', 'delete') or (kind == 'rename') != (destination is not None):
            _fail('invalid_argument', 'Invalid remote operation or destination')
        source = self._operation_path(client, operation, source, new=kind == 'mkdir')
        if source == self.root:
            _fail('invalid_argument', 'The configured remote root cannot be mutated')
        if kind == 'mkdir':
            self._operation_absent(client, operation, source)
            entry_kind, fingerprint = 'directory', ()
        else:
            entry_kind, fingerprint = self._operation_state(client, operation, source)
        if destination is not None:
            destination = self._operation_path(client, operation, destination, new=True)
            if source == destination:
                _fail('invalid_argument', 'Source and destination must differ')
            self._operation_absent(client, operation, destination)
        warning = ('Permanent deletion. Remote changes may race this check; no undo.'
                   if kind == 'delete' else 'Remote checks are best effort; no undo.'
                   if kind == 'mkdir' else 'Remote changes may race this check; no undo.')
        if kind == 'rename' and getattr(self, 'protocol', None) in ('ftp', 'ftps'):
            warning = 'FTP rename may overwrite a newly created target; no atomicity or undo.'
        return PreparedOperation(self.connection_id, kind, source, destination,
                                 entry_kind, fingerprint, (warning,))

    def prepare_operation(self, kind, source, destination=None, cancel=None):
        return self._run(lambda client, op: self._prepare_operation(
            client, op, kind, source, destination), cancel)

    def apply_operation(self, prepared, cancel=None):
        if not isinstance(prepared, PreparedOperation) or prepared.connection_id != self.connection_id:
            _fail('conflict', 'Operation belongs to a different remote connection')

        def action(client, operation):
            current = self._prepare_operation(client, operation, prepared.kind,
                                              prepared.source, prepared.destination)
            if current != prepared:
                _fail('conflict', 'Remote operation targets changed; prepare and confirm again')
            operation.promote()
            self._operation_apply(client, prepared)
            return 'applied'
        return self._run(action, cancel, mutation=True)

    def download(self, path, staging_path, expected_bytes, cancel=None, progress=None):
        """Stream raw bytes into an existing host staging file, never a destination."""
        if type(expected_bytes) is not int or not 0 <= expected_bytes <= 8 * 1024 * 1024:
            _fail('limit_exceeded', 'Download must fit the declared 8 MiB staging limit')

        def action(client, operation):
            canonical = self._path(client, operation, path)
            operation.check()
            descriptor = os.open(staging_path, os.O_WRONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
            try:
                attrs = os.fstat(descriptor)
                if (not stat.S_ISREG(attrs.st_mode) or attrs.st_size != 0
                        or attrs.st_uid != os.geteuid() or attrs.st_nlink != 1
                        or attrs.st_mode & 0o077):
                    _fail('conflict', 'Host staging file must be empty, private, owned and unlinked elsewhere')
                digest, count = hashlib.sha256(), 0

                def receive(data):
                    nonlocal count
                    operation.check()
                    if count + len(data) > expected_bytes:
                        _fail('conflict', 'Remote size changed; refresh the directory before downloading')
                    remaining = memoryview(data)
                    while remaining:
                        operation.check()
                        written = os.write(descriptor, remaining)
                        if not written:
                            _fail('unavailable', 'Staging write did not complete')
                        remaining = remaining[written:]
                    digest.update(data)
                    count += len(data)
                    if progress is not None:
                        progress(count)

                self._read(client, operation, canonical, sink=receive)
                operation.check()
                if count != expected_bytes:
                    _fail('conflict', 'Remote size changed; refresh the directory before downloading')
                return digest.hexdigest()
            finally:
                os.close(descriptor)
        return self._run(action, cancel)

    def _run(self, action, cancel, *, timeout=8.0, mutation=False):
        if not self._slots.acquire(blocking=False):
            _fail('busy', 'Two remote operations are still running')
        if mutation and not self._mutation_slot.acquire(blocking=False):
            self._slots.release()
            _fail('busy', 'Another remote mutation is still running')
        operation = _Operation(cancel, timeout)

        def worker():
            try:
                operation.result = action(self._connect(operation), operation)
            except Exception as error:
                with operation.lock:
                    operation.error = self._error(error, operation.promotion_started)
            finally:
                try:
                    operation.close()
                finally:
                    if mutation:
                        self._mutation_slot.release()
                    operation.done.set()
                    self._slots.release()

        try:
            threading.Thread(target=worker, name='runyte-remote', daemon=True).start()
        except Exception:
            self._slots.release()
            if mutation:
                self._mutation_slot.release()
            _fail('unavailable', 'Remote worker could not start')
        while not operation.done.wait(min(0.05, max(0.001, operation.deadline - time.monotonic()))):
            with operation.lock:
                try:
                    operation._check_locked()
                except TransportError as error:
                    operation.cancelled = True
                    failure = self._error(error, operation.promotion_started)
                else:
                    continue
            operation.close()
            raise failure
        if operation.error is not None:
            raise operation.error
        return operation.result
