# SPDX-License-Identifier: MPL-2.0
"""SDK output backpressure, reader progress, bounded queues and wire ordering."""
import json
import os
from pathlib import Path
import queue
import selectors
import subprocess
import sys
import threading
import time
import unittest
from unittest.mock import patch

from application import Application, LIMIT, PluginError, VERSION, _WireIO


class WriterTests(unittest.TestCase):
    def setUp(self):
        self.read_fd, self.write_fd = os.pipe()
        os.set_blocking(self.write_fd, False)
        self.app = Application('Writer checks', [], ['jobs'])
        self.app._io = _WireIO(self.app._disconnect, self.write_fd)
        self.threads = []

    def tearDown(self):
        self.app._disconnect()
        for thread in self.threads:
            thread.join(2)
            self.assertFalse(thread.is_alive())
        self.app._io.finish()
        for executor in (self.app._executor, self.app._resource_executor, self.app._control,
                         self.app._observations, self.app._validation_executor):
            executor.shutdown(wait=True, cancel_futures=True)
        if self.read_fd is not None:
            os.close(self.read_fd)
        os.close(self.write_fd)

    def fill(self):
        total = 0
        while True:
            try:
                total += os.write(self.write_fd, b'x' * 65536)
            except BlockingIOError:
                return total

    def thread(self, function):
        result = queue.Queue()
        def run():
            try:
                result.put(function())
            except Exception as error:
                result.put(error)
        thread = threading.Thread(target=run)
        self.threads.append(thread)
        thread.start()
        return result

    def test_full_pipe_keeps_inflight_message_and_byte_bounds_and_stops_promptly(self):
        self.fill()
        wire = self.app._io
        for _ in range(4):
            wire.enqueue(b'x' * LIMIT)
        with wire.condition:
            self.assertEqual(wire.bytes, wire.MAX_BYTES)
            self.assertEqual(len(wire.queue), 4)
        with self.assertRaises(PluginError) as caught:
            wire.enqueue(b'x')
        self.assertEqual(caught.exception.code, 'busy')
        before = time.monotonic()
        wire.close()
        wire.thread.join(1)
        self.assertFalse(wire.thread.is_alive())
        self.assertLess(time.monotonic() - before, 1)

    def test_message_count_includes_current_frame(self):
        self.fill()
        wire = self.app._io
        for _ in range(wire.MAX_MESSAGES):
            wire.enqueue(b'x')
        with self.assertRaises(PluginError):
            wire.enqueue(b'x')
        with wire.condition:
            self.assertEqual(len(wire.queue), wire.MAX_MESSAGES)
            self.assertEqual(wire.bytes, wire.MAX_MESSAGES)

    def test_stalled_output_deadline_closes_before_any_queued_mutation_can_be_sent(self):
        filler = self.fill()
        before = time.monotonic()
        result = self.thread(lambda: self.app._request('state.set', {'document': {}}, timeout=0.1))
        error = result.get(timeout=1)
        self.assertEqual(error.code, 'outcome_unknown')
        self.assertLess(time.monotonic() - before, 1)
        self.assertTrue(self.app._closed.is_set())
        self.assertFalse(self.app._pending)
        self.app._io.thread.join(1)
        os.set_blocking(self.read_fd, False)
        data = bytearray()
        while True:
            try:
                data.extend(os.read(self.read_fd, 65536))
            except BlockingIOError:
                break
        self.assertEqual(bytes(data), b'x' * filler)
        with self.assertRaises(PluginError):
            self.app.request('workspace.info')

    def test_reader_correlates_reply_then_cancels_and_exits_while_output_is_full(self):
        self.fill()
        input_read, input_write = os.pipe()
        self.addCleanup(lambda: os.close(input_write) if input_write is not None else None)
        stream = os.fdopen(input_read, 'rb', buffering=0)
        self.addCleanup(stream.close)
        cancelled = threading.Event()
        self.app.on_event = lambda name, _: cancelled.set() if name == 'job.cancel_requested' else None
        with patch.object(sys, 'stdin', stream):
            registered = threading.Event()
            original_read = self.app._read
            def read():
                value = original_read()
                if value.get('type') == 'registered':
                    registered.set()
                return value
            self.app._read = read
            runner = self.thread(self.app.run)
            # Deliberately supply the host handshake while registration output is
            # stalled; a full pipe must not make the reader acquire its writer lock.
            os.write(input_write, (json.dumps({'type': 'hello', 'version': VERSION}) + '\n'
                                  + json.dumps({'type': 'registered'}) + '\n').encode())
            self.assertTrue(registered.wait(1))
            request = self.thread(lambda: self.app._request('workspace.info', {}, timeout=2))
            with self.app._io.condition:
                self.assertTrue(self.app._io.condition.wait_for(
                    lambda: any(b'"p:1"' in entry for entry in self.app._io.queue), timeout=1))
            os.write(input_write, (json.dumps({'type': 'response', 'id': 'p:1', 'result': {}}) + '\n'
                + json.dumps({'type': 'event', 'event': 'job.cancel_requested', 'data': {'job': 'j:1'}}) + '\n').encode())
            self.assertEqual(request.get(timeout=1), {})
            self.assertTrue(cancelled.wait(1))
            os.close(input_write)
            input_write = None
            self.assertIsNone(runner.get(timeout=1))
            self.assertTrue(self.app._closed.is_set())

    def test_broken_output_wakes_a_reader_waiting_for_host_input(self):
        os.close(self.read_fd)
        self.read_fd = None
        input_read, input_write = os.pipe()
        self.addCleanup(lambda: os.close(input_write) if input_write is not None else None)
        stream = os.fdopen(input_read, 'rb', buffering=0)
        self.addCleanup(stream.close)
        with patch.object(sys, 'stdin', stream):
            reader = self.thread(self.app._read)
            self.app._write({'type': 'request', 'id': 'p:1'})
            self.assertIsInstance(reader.get(timeout=1), EOFError)
            self.assertTrue(self.app._closed.is_set())

    def test_full_output_refuses_new_requests_but_disconnects_on_lost_reliable_reply(self):
        self.fill()
        for _ in range(self.app._io.MAX_MESSAGES):
            self.app._io.enqueue(b'x')
        with self.assertRaises(PluginError) as caught:
            self.app.request('workspace.info')
        self.assertEqual(caught.exception.code, 'busy')
        self.assertFalse(self.app._closed.is_set())
        self.assertFalse(self.app._pending)
        with self.assertRaises(PluginError):
            self.app._write({'type': 'response', 'id': 'h:1', 'result': {'job': None}})
        self.assertTrue(self.app._closed.is_set())

    def test_reader_rejects_non_utf8_and_unterminated_full_frames(self):
        for data in (json.dumps({'type': 'hello'}).encode('utf-16') + b'\n', b'x' * LIMIT):
            with self.subTest(size=len(data)):
                input_read, input_write = os.pipe()
                stream = os.fdopen(input_read, 'rb', buffering=0)
                try:
                    with patch.object(sys, 'stdin', stream):
                        reader = self.thread(self.app._read)
                        with os.fdopen(input_write, 'wb', buffering=0) as output:
                            offset = 0
                            while offset < len(data):
                                offset += output.write(data[offset:])
                        self.assertIsInstance(reader.get(timeout=1), (UnicodeError, PluginError))
                finally:
                    stream.close()
                    self.app._input.clear()

    def test_close_wins_after_writable_selection_but_before_first_output_byte(self):
        selected, release = threading.Event(), threading.Event()
        original = selectors.DefaultSelector
        class GatedSelector:
            def __init__(self):
                self.inner = original()
            def __enter__(self):
                return self
            def __exit__(self, *_):
                self.inner.close()
            def register(self, *args):
                return self.inner.register(*args)
            def select(self):
                result = self.inner.select()
                selected.set()
                release.wait(2)
                return result
        try:
            with patch('application.selectors.DefaultSelector', GatedSelector):
                self.app._io.enqueue(b'never send this queued mutation')
                self.assertTrue(selected.wait(1))
                self.app._io.close()
                release.set()
                self.app._io.thread.join(1)
                self.assertFalse(self.app._io.thread.is_alive())
            os.set_blocking(self.read_fd, False)
            with self.assertRaises(BlockingIOError):
                os.read(self.read_fd, 4096)
        finally:
            release.set()

    def test_host_reported_timeout_does_not_disconnect_a_healthy_transport(self):
        self.app._write = lambda message: self.app._response({'type': 'response', 'id': message['id'],
                    'error': {'code': 'timeout', 'message': 'Operation deadline'}})
        with self.assertRaises(PluginError) as caught:
            self.app.request('workspace.info')
        self.assertEqual(caught.exception.code, 'timeout')
        self.assertFalse(self.app._closed.is_set())


class ActualWireTests(unittest.TestCase):
    def test_real_jobs_client_writes_valid_fifo_frames_and_settles_eof(self):
        child = subprocess.Popen([sys.executable, str(Path(__file__).with_name('jobs.py'))],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        def send(value):
            child.stdin.write((json.dumps(value) + '\n').encode())
        def read():
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(2), 'SDK output stalled')
            return json.loads(child.stdout.readline(LIMIT + 1))
        try:
            send({'type': 'hello', 'version': VERSION})
            self.assertEqual(read()['type'], 'register')
            send({'type': 'registered'})
            send({'type': 'request', 'id': 'h:1', 'method': 'command.invoke', 'params': {'command': 'start'}})
            create = read()
            self.assertEqual(create['id'], 'p:1')
            send({'type': 'response', 'id': create['id'], 'result': {'job': 'j:1'}})
            send({'type': 'event', 'event': 'job.cancel_requested', 'data': {'job': 'j:1'}})
            replies = [read(), read()]
            finish = next(value for value in replies if value.get('method') == 'job.finish')
            self.assertEqual(finish['id'], 'p:2')
            self.assertEqual(finish['params']['state'], 'cancelled')
            self.assertTrue(any(value['id'] == 'h:1' for value in replies))
            send({'type': 'response', 'id': finish['id'], 'result': {}})
            child.stdin.close()
            self.assertEqual(child.wait(timeout=2), 0)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=2)
            for stream in (child.stdin, child.stdout, child.stderr):
                stream.close()


if __name__ == '__main__':
    unittest.main(verbosity=2)
