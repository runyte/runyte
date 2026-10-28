#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Offline Codex CLI wake acceptance test (tested with 0.164.0, Unix only).

Run: python3 tests/mcp/codex_queue_probe.py [PATH_TO_RUNYTE]
The optional binary exercises the real host -> MCP adapter -> Codex queue path.
Uses a temporary CODEX_HOME/XDG_CONFIG_HOME, fixture-owned daemon, and localhost
fake Responses model. No real API/authentication or personal daemon is used.
Verifies idle wake and queueing while command approval remains unanswered.
The fixture writes data only; all executables come from the installed Codex CLI.
"""

import argparse
import contextlib
import http.server
import json
import os
from pathlib import Path
import queue
import signal
import socket
import struct
import subprocess
import tempfile
import threading
import time


class FakeModel(http.server.BaseHTTPRequestHandler):
    requests = 0
    payloads = []

    def log_message(self, *_args):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        FakeModel.payloads.append(json.loads(body))
        FakeModel.requests += 1
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        item = {
            "type": "function_call", "id": "fc_fixture", "call_id": "call_fixture",
            "name": "exec_command",
            "arguments": json.dumps({
                "cmd": "true", "sandbox_permissions": "require_escalated",
                "justification": "Offline fixture approval: leave unanswered",
            }),
        }
        response = {"id": "r_fixture", "status": "in_progress", "output": []}
        events = [
            {"type": "response.created", "response": response},
            {"type": "response.output_item.added", "output_index": 0, "item": item},
            {"type": "response.output_item.done", "output_index": 0, "item": item},
            {"type": "response.completed", "response": {
                **response, "status": "completed", "output": [item],
                "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2},
            }},
        ]
        for event in events:
            self.wfile.write(("data: " + json.dumps(event) + "\n\n").encode())
            self.wfile.flush()


class Client:
    """Minimal local WebSocket JSON-RPC client, with bounded reads and deadlines."""

    def __init__(self, endpoint):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(10)
        self.socket.connect(endpoint)
        self.socket.sendall(
            b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n"
            b"Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
            b"Sec-WebSocket-Version: 13\r\n\r\n"
        )
        header = bytearray()
        while not header.endswith(b"\r\n\r\n"):
            header += self.exact(1)
            assert len(header) < 8192, "excessive handshake"
        assert b"101 Switching" in header, header
        self.events = queue.Queue()
        self.next_id = 0
        self.thread = threading.Thread(target=self.receive, daemon=True)
        self.thread.start()

    def exact(self, size):
        data = bytearray()
        while len(data) < size:
            part = self.socket.recv(size - len(data))
            if not part:
                raise EOFError("fixture socket closed")
            data += part
        return bytes(data)

    def receive(self):
        try:
            while True:
                header = self.exact(2)
                size = header[1] & 127
                if size == 126:
                    size = struct.unpack("!H", self.exact(2))[0]
                elif size == 127:
                    size = struct.unpack("!Q", self.exact(8))[0]
                assert size <= 16 * 1024 * 1024, "excessive server frame"
                body = self.exact(size)
                opcode = header[0] & 15
                if opcode == 1:
                    self.events.put(json.loads(body))
                elif opcode == 8:
                    return
        except (EOFError, OSError, AssertionError, ValueError) as error:
            self.events.put(error)

    def send(self, value):
        body = json.dumps(value).encode()
        size = len(body)
        assert size < 65536
        header = b"\x81" + (bytes([128 + size]) if size < 126 else b"\xfe" + struct.pack("!H", size))
        mask = b"1234"
        self.socket.sendall(header + mask + bytes(byte ^ mask[index % 4] for index, byte in enumerate(body)))

    def until(self, predicate):
        for _ in range(1000):
            event = self.events.get(timeout=15)
            if isinstance(event, BaseException):
                raise event
            if predicate(event):
                return event
        raise AssertionError("too many events without expected response")

    def request(self, method, params):
        self.next_id += 1
        request_id = self.next_id
        self.send({"id": request_id, "method": method, "params": params})
        reply = self.until(lambda event: event.get("id") == request_id and "method" not in event)
        assert "error" not in reply, reply
        return reply["result"]

    def close(self):
        self.socket.close()



def product_probe(binary, env, client, thread):
    # Imports remain optional: the standalone Codex protocol test uses only the
    # standard library; the product path uses the existing headless persistent-host fixture.
    from durable_fixture import DurableFixture
    from mcp_client import PROTOCOL, RealMCPClient

    class Recipient(RealMCPClient):
        def __init__(self, case):
            arguments = [str(binary), "mcp", "--identity", "codex", "--timeout", "2",
                         "--channel-notifications", "codex", "--codex-thread", thread]
            recipient_env = dict(case.env)
            recipient_env["CODEX_HOME"] = env["CODEX_HOME"]
            for name in ("OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_THREAD_ID", "CODEX_SESSION_ID"):
                recipient_env.pop(name, None)
            self.process = subprocess.Popen(arguments, cwd=case.root, env=recipient_env,
                                            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                            stderr=subprocess.DEVNULL)
            self.counter = 0
            self.responses = queue.Queue(maxsize=64)
            self.stopped = threading.Event()
            self.reader = threading.Thread(target=self._read, daemon=True)
            self.reader.start()
            try:
                self.rpc("initialize", {"protocolVersion": PROTOCOL, "capabilities": {},
                                       "clientInfo": {"name": "codex-offline-fixture", "version": "1"}})
                self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})
            except BaseException:
                self.close()
                raise

    case = DurableFixture(binary)
    try:
        case.add_identity("codex", scopes=("agent_channel", "channel_nudge"))
        case.add_identity("claude", scopes=("agent_channel",))
        case.start()
        receiver = Recipient(case)
        case.clients.append(receiver)
        sender = RealMCPClient(binary, case.root, case.env, "claude")
        case.clients.append(sender)
        workspace = str(case.project)
        joined = receiver.tool("join_channel", workspace=workspace,
                               provider="codex", task="offline product wake")
        assert not joined.get("isError"), joined
        envelope = joined["structuredContent"]
        handle = envelope["workspace"]
        recipient = envelope["data"]["participant"]
        assert recipient["terminal"] is None
        sender.data("join_channel", workspace=workspace, provider="claude", task="offline sender")
        receiver.data("read_messages", workspace=workspace)

        def post(text):
            result = sender.data("post_message", workspace=workspace, text=text,
                                 nudge=[recipient["participant"]])
            assert result["nudges"][0]["state"] == "queued", result

        post("PRIVATE_FIRST_BODY_MUST_STAY_IN_CHANNEL")
        client.until(lambda event: event.get("method") == "turn/started")
        client.until(lambda event: event.get("method") == "item/commandExecution/requestApproval")
        assert FakeModel.requests == 1
        payload = FakeModel.payloads[0]
        user_text = [part.get("text", "") for item in payload["input"]
                     if item.get("role") == "user" for part in item.get("content", [])]
        expected_handle = "Workspace handle: " + json.dumps(handle)
        wake = next(text for text in user_text if expected_handle in text)
        assert "Use read_messages" in wake
        assert "PRIVATE_FIRST_BODY_MUST_STAY_IN_CHANNEL" not in json.dumps(payload)
        read = receiver.data("read_messages", workspace=handle)
        assert any(message["text"] == "PRIVATE_FIRST_BODY_MUST_STAY_IN_CHANNEL"
                   for message in read["messages"])
        post("PRIVATE_SECOND_BODY_MUST_STAY_IN_CHANNEL")
        deadline = time.monotonic() + 5
        while True:
            queued = client.request("thread/queue/list", {"threadId": thread})["data"]
            if queued:
                break
            assert time.monotonic() < deadline, "product wake never reached Codex daemon"
            time.sleep(.02)
        assert len(queued) == 1, queued
        reminder = queued[0]["input"][0]["text"]
        assert expected_handle in reminder
        assert "PRIVATE_SECOND_BODY_MUST_STAY_IN_CHANNEL" not in reminder
        assert "waitingOnApproval" in client.request("thread/read", {"threadId": thread})["thread"]["status"]["activeFlags"]
        assert FakeModel.requests == 1, "product wake bypassed command approval"
    finally:
        case.close()


def run(binary=None):
    FakeModel.requests = 0
    FakeModel.payloads = []
    with tempfile.TemporaryDirectory(prefix="runyte-codex-wake-") as directory:
        root = Path(directory)
        env = os.environ.copy()
        env["CODEX_HOME"] = directory
        env["XDG_CONFIG_HOME"] = str(root / "config")
        for name in ("OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_THREAD_ID", "CODEX_SESSION_ID"):
            env.pop(name, None)
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakeModel)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        (root / "config.toml").write_text(
            'analytics.enabled=false\n[features]\ncode_mode_host=false\n'
            '[model_providers.probe]\nname="offline fixture"\n'
            f'base_url="http://127.0.0.1:{server.server_port}"\n'
            'wire_api="responses"\nrequires_openai_auth=false\n'
            'request_max_retries=0\nstream_max_retries=0\n'
        )

        def command(*arguments, timeout=15):
            # A daemon can inherit output descriptors after its launcher exits.
            # Files keep launcher completion independent of pipe EOF ownership.
            with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as error:
                result = subprocess.run(["codex", *arguments], env=env, cwd=root,
                                        stdout=output, stderr=error, timeout=timeout, check=True)
                output.seek(0)
                error.seek(0)
                return subprocess.CompletedProcess(result.args, result.returncode,
                                                   output.read().decode(), error.read().decode())

        daemon = None
        client = None
        try:
            daemon = json.loads(command("app-server", "daemon", "start", timeout=30).stdout)
            assert daemon["status"] == "started", daemon
            client = Client(daemon["socketPath"])
            version = client.request("initialize", {
                "clientInfo": {"name": "runyte_offline_wake_fixture", "version": "1"},
                "capabilities": {"experimentalApi": True},
            })["userAgent"]
            client.send({"method": "initialized"})
            thread = client.request("thread/start", {
                "cwd": directory, "model": "sandbox-no-model", "modelProvider": "probe",
                "approvalPolicy": "on-request", "sandbox": "read-only",
            })["thread"]["id"]
            if binary is not None:
                product_probe(binary, env, client, thread)
                print(f"PASS {version}: actual Runyte host/MCP wake starts idle Codex and preserves pending approval with exact workspace handle")
            else:
                command("queue", "--thread", thread, "--message", "Offline fixture idle wake")
                client.until(lambda event: event.get("method") == "turn/started")
                client.until(lambda event: event.get("method") == "item/commandExecution/requestApproval")
                assert FakeModel.requests == 1
                command("queue", "--thread", thread, "--message", "Offline fixture wake while approval pending")
                queued = client.request("thread/queue/list", {"threadId": thread})["data"]
                assert len(queued) == 1, queued
                assert queued[0]["input"][0]["text"] == "Offline fixture wake while approval pending"
                status = client.request("thread/read", {"threadId": thread})["thread"]["status"]
                assert "waitingOnApproval" in status["activeFlags"], status
                assert FakeModel.requests == 1, "pending approval was bypassed"
                print(f"PASS {version}: idle wake starts a model turn; pending approval preserves queued message")
        finally:
            if client:
                client.close()
            if daemon:
                # Vendor shutdown may wait on an unanswered approval. Kill only
                # the PID returned by this fixture's isolated daemon start.
                try:
                    command("app-server", "daemon", "stop", timeout=3)
                except (subprocess.TimeoutExpired, subprocess.CalledProcessError):
                    with contextlib.suppress(ProcessLookupError):
                        os.kill(daemon["pid"], signal.SIGKILL)
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runyte", nargs="?", type=lambda value: Path(value).resolve(strict=True),
                        help="optional prebuilt Runyte binary for host/MCP/Codex end-to-end coverage")
    run(parser.parse_args().runyte)
