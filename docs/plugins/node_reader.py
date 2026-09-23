# SPDX-License-Identifier: MPL-2.0
"""Bounded message-aware reads for the real Node conformance fixture."""
from collections import deque
import json
import time


def stderr_snapshot(read):
    """One nonblocking read, never a wait for the child or stderr EOF."""
    try:
        return read(1024)
    except BlockingIOError:
        return b''
    except OSError as error:
        return f'<stderr unavailable: errno={error.errno}>'.encode('ascii')


class ResponseReader:
    def __init__(self, wait, read, validate, diagnostics, limit, clock=time.monotonic):
        self.wait = wait
        self.read_chunk = read
        self.validate = validate
        self.diagnostics = diagnostics
        self.limit = limit
        self.clock = clock
        self.buffer = bytearray()
        self.messages = deque()

    def failure(self, reason, phase, started):
        status, stderr = self.diagnostics()
        return AssertionError(
            f'{reason}; phase={phase}; elapsed={self.clock() - started:.3f}s; '
            f'child_status={status}; partial_bytes={len(self.buffer)}; '
            f'queued_messages={len(self.messages)}; '
            f'partial_prefix={bytes(self.buffer[:256])!r}; stderr_prefix={stderr[:1024]!r}')

    def read(self, deadline, phase='response'):
        started = self.clock()
        # A preceding os.read may have returned several complete messages.
        # Never ask the descriptor for new readiness before consuming them.
        while not self.messages:
            remaining = max(0, deadline - self.clock())
            if not self.wait(remaining):
                raise self.failure('Node response timed out', phase, started)
            data = self.read_chunk(self.limit + 1)
            if not data:
                raise self.failure('Node exited before a response', phase, started)
            self.buffer.extend(data)
            if len(self.buffer) > 2 * self.limit:
                raise AssertionError('Node response buffer exceeds the fixture bound')
            while b'\n' in self.buffer:
                line, _, remaining = self.buffer.partition(b'\n')
                self.buffer = bytearray(remaining)
                if len(line) + 1 > self.limit:
                    raise AssertionError('Node response frame exceeds the protocol bound')
                value = json.loads(line)
                self.validate(value)
                self.messages.append(value)
        return self.messages.popleft()
