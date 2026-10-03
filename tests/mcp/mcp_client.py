# SPDX-License-Identifier: MPL-2.0
"""Bounded test-only stdio client for the Runyte binary, on Unix and Windows."""
import json
import queue
import subprocess
import threading
import time

PROTOCOL = '2025-06-18'
FRAME_BYTES = 1024 * 1024


class RealMCPClient:
    def __init__(self, binary, root, env, identity):
        self.process = subprocess.Popen(
            [str(binary), 'mcp', '--identity', identity, '--timeout', '2'],
            cwd=root, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self.counter = 0
        self.responses = queue.Queue(maxsize=64)
        self.stopped = threading.Event()
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()
        try:
            reply = self.rpc('initialize', {'protocolVersion': PROTOCOL, 'capabilities': {},
                                           'clientInfo': {'name': identity, 'version': 'acceptance'}})
            assert reply['result']['protocolVersion'] == PROTOCOL, reply
            self._send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        except BaseException:
            self.close()
            raise

    def _read(self):
        try:
            while not self.stopped.is_set():
                line = self.process.stdout.readline(FRAME_BYTES + 1)
                if not line:
                    raise AssertionError('Runyte MCP exited before its response')
                if len(line) > FRAME_BYTES or not line.endswith(b'\n'):
                    raise AssertionError('Oversized or incomplete MCP response')
                self.responses.put(json.loads(line), timeout=1)
        except Exception as error:
            if not self.stopped.is_set():
                try:
                    self.responses.put(error, timeout=1)
                except queue.Full:
                    pass

    def _send(self, value):
        self.process.stdin.write(json.dumps(value, ensure_ascii=False).encode() + b'\n')
        self.process.stdin.flush()

    def rpc(self, method, params, *, seconds=10):
        self.counter += 1
        self._send({'jsonrpc': '2.0', 'id': self.counter, 'method': method, 'params': params})
        deadline = time.monotonic() + seconds
        for _ in range(65):
            try:
                reply = self.responses.get(timeout=max(0, deadline - time.monotonic()))
            except queue.Empty as error:
                raise AssertionError('MCP response deadline exceeded') from error
            if isinstance(reply, Exception):
                raise reply
            if 'id' in reply:
                assert reply['id'] == self.counter, reply
                assert 'error' not in reply, reply
                return reply
        raise AssertionError('Too many MCP notifications')

    def tool(self, name, **arguments):
        return self.rpc('tools/call', {'name': name, 'arguments': arguments})['result']

    def data(self, name, *, response_seconds=10, **arguments):
        result = self.rpc('tools/call', {'name': name, 'arguments': arguments},
                          seconds=response_seconds)['result']
        if result.get('isError'):
            raise AssertionError(f'MCP tool {name} failed: {result}')
        value = result['structuredContent']
        return value if name in ('list_workspaces', 'find_resources') else value['data']

    def close(self):
        if self.stopped.is_set():
            return
        self.stopped.set()
        self.process.stdin.close()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        self.reader.join(timeout=5)
        assert not self.reader.is_alive(), 'MCP reader did not stop'
        self.process.stdout.close()
