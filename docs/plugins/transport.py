# SPDX-License-Identifier: MPL-2.0
"""Shared bounded remote workers and conservative mutation outcome errors."""
import threading
import time

from application import PluginError


class TransportError(PluginError):
    def __init__(self, code, message, *, outcome_unknown=False, settled=True):
        self.outcome_unknown = outcome_unknown
        self.settled = settled and not outcome_unknown
        super().__init__('outcome_unknown' if outcome_unknown else code, message)


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
    def _run(self, action, cancel, *, timeout=8.0):
        if not self._slots.acquire(blocking=False):
            _fail('busy', 'Two remote operations are still running')
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
                    operation.done.set()
                    self._slots.release()

        try:
            threading.Thread(target=worker, name='runyte-remote', daemon=True).start()
        except Exception:
            self._slots.release()
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

