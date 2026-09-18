# SPDX-License-Identifier: MPL-2.0
"""Network-free MCP clients and independent authenticated Unix host fixtures."""

import io
import json
import os
from pathlib import Path
import selectors
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

from runyte_context.client import Bridge, Failure, Connection, decode, encode, load_credential, FRAME_BYTES
from runyte_context.server import INSTRUCTIONS, Server, PROTOCOL
from runyte_context.tools import call, descriptors

PACKAGE = Path(__file__).resolve().parents[1]
REPO = PACKAGE.parents[1]
READS = {"terminal_read", "editor_context_read"}
ALL = READS | {"buffer_edit", "terminal_propose"}


class Host:
    def __init__(self, root, number, grants=None, detached=False):
        self.record = {"root": f"/projects/workspace-{number}", "workspace_id": f"{number:064x}",
            "host_incarnation": f"{number + 100:064x}", "endpoint": str(root / f"h{number}.sock"),
            "mode": "persistent", "pid": os.getpid(), "environment": "e" * 64,
            "label": "duplicate label", "transport_version": "runyte.context.v1"}
        self.grants = grants if grants is not None else {"a" * 64: ALL, "b" * 64: READS}
        self.detached = detached
        self.methods = []
        self.inputs = []
        self.snapshots = {}
        self.proposals = {}
        self.text = f"terminal {number}: untrusted text"
        self.buffer = "original界"
        self.revision = 1
        self.fail_next = None
        self.malformed_next = None
        self.slow = False
        self.hello_override = {}
        self.registered_override = {}
        self.stop = threading.Event()
        self.peers = []
        self.workers = []
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(self.record["endpoint"])
        os.chmod(self.record["endpoint"], 0o600)
        self.listener.listen(16)
        self.listener.settimeout(.1)
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    def accept(self):
        while not self.stop.is_set():
            try:
                peer, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            self.peers.append(peer)
            worker = threading.Thread(target=self.serve, args=(peer,), daemon=True)
            self.workers.append(worker)
            worker.start()

    def serve(self, peer):
        try:
            with peer, peer.makefile("rb") as source:
                auth = decode(source.readline(FRAME_BYTES + 1))
                credential = auth.get("credential")
                if self.slow:
                    self.stop.wait(.8)
                    return
                if credential not in self.grants:
                    peer.sendall(encode({"type": "registration_error", "code": "capability_denied", "message": "Denied"}))
                    return
                peer.sendall(encode({"type": "hello", "version": "runyte-1", "host_version": "0.3.0",
                                    "features": ["runyte.context.v1"], "capabilities": sorted(ALL),
                                    "limits": {"line_bytes": FRAME_BYTES}, "workspace": self.record,
                                    **self.hello_override}))
                registration = decode(source.readline(FRAME_BYTES + 1))
                self.methods.append(registration["type"])
                scopes = self.grants[credential] & set(registration["optional_capabilities"])
                peer.sendall(encode({"type": "registered", "runyte": ">=0.3.0, <0.4.0", "commands": [],
                                    "limits": {"line_bytes": FRAME_BYTES}, "features": ["runyte.context.v1"],
                                    "capabilities": sorted(scopes), **self.registered_override}))
                for raw in source:
                    request = decode(raw)
                    method, params = request["method"], request["params"]
                    self.methods.append(method)
                    response = {"type": "response", "id": request["id"]}
                    if credential not in self.grants:
                        response["error"] = {"code": "capability_denied", "message": "Revoked"}
                    elif self.fail_next == method:
                        self.fail_next = None
                        return
                    elif self.malformed_next is not None:
                        response.update(self.malformed_next)
                        self.malformed_next = None
                    else:
                        result = self.dispatch(method, params)
                        if "error" in result:
                            response.update(result)
                        else:
                            response["result"] = {**result, "workspace": self.record}
                    peer.sendall(encode(response))
        except (OSError, Failure, KeyError, ValueError):
            pass

    def dispatch(self, method, params):
        if method == "terminal.list":
            return {"terminals": [{"terminal": "t:1", "name": "agent", "revision": "r:1", "pane": "p:1"}], "next": None}
        if method == "buffer.list":
            return {"buffers": [{"buffer": "b:1", "revision": f"r:{self.revision}", "chars": len(self.buffer)}], "next": None}
        if method == "pane.context.list":
            return {"panes": [{"pane": "p:1", "buffer": "b:1", "focused": True}], "attached": not self.detached}
        if method == "pane.viewport.read":
            return ({"error": {"code": "no_frontend", "message": "Detached"}} if self.detached
                    else {"pane": "p:1", "rows": [{"text": self.buffer}], "review": False})
        if method == "terminal.read":
            return {"terminal": "t:1", "rows": [{"text": self.text}], "revision": "r:1", "truncation": {"rows": False}}
        if method == "terminal.snapshot.open":
            self.snapshots["s:1"] = self.text
            return {"snapshot": "s:1", "rows": 1, "revision": "r:1"}
        if method == "terminal.snapshot.read":
            return {"snapshot": params["snapshot"], "terminal": "t:1", "rows": [{"text": self.snapshots[params["snapshot"]]}], "next": None}
        if method == "terminal.snapshot.close":
            self.snapshots.pop(params["snapshot"])
            return {}
        if method == "buffer.read":
            return {"buffer": "b:1", "text": self.buffer[params["from"]:params["to"]], "revision": f"r:{self.revision}"}
        if method == "buffer.edit":
            if params["expected_revision"] != f"r:{self.revision}":
                return {"error": {"code": "stale", "message": "Stale revision"}}
            for change in reversed(params["changes"]):
                self.buffer = self.buffer[:change["from"]] + change["text"] + self.buffer[change["to"]:]
            self.revision += 1
            return {"buffer": "b:1", "revision": f"r:{self.revision}"}
        if method == "terminal.input.propose":
            self.proposals["i:1"] = {"text": params["text"], "state": "pending"}
            return {"proposal": "i:1", "state": "pending"}
        if method == "terminal.input.status":
            return {"proposal": params["proposal"], "state": self.proposals[params["proposal"]]["state"]}
        if method == "terminal.input.cancel":
            self.proposals[params["proposal"]]["state"] = "cancelled"
            return {"proposal": params["proposal"], "state": "cancelled"}
        return {"error": {"code": "unsupported", "message": "Fixture method is unsupported"}}

    def close(self):
        self.stop.set()
        self.listener.close()
        for peer in self.peers:
            try:
                peer.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(2)
        for worker in self.workers:
            worker.join(2)


class MCPClient:
    def __init__(self, root, name):
        self.process = subprocess.Popen([sys.executable, "-m", "runyte_context", "--identity", name,
                                        "--runyte", str(root / "runyte"), "--timeout", ".3"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=root, bufsize=0,
            env={**os.environ, "PYTHONPATH": str(PACKAGE), "RUNYTE_CONTEXT_HOME": str(root),
                 "RUNYTE_CONTEXT_TEST_INVENTORY": str(root / "inventory.json"),
                 "XDG_CONFIG_HOME": str(root / "config")})
        self.counter = 0
        self.notifications = []
        self.rpc("initialize", {"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": name, "version": "1"}})
        self.process.stdin.write(encode({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        self.process.stdin.flush()

    def rpc(self, method, params):
        self.counter += 1
        self.process.stdin.write(encode({"jsonrpc": "2.0", "id": self.counter, "method": method, "params": params}))
        self.process.stdin.flush()
        with selectors.DefaultSelector() as poll:
            poll.register(self.process.stdout, selectors.EVENT_READ)
            while True:
                if not poll.select(5):
                    raise AssertionError("MCP response timeout")
                response = decode(self.process.stdout.readline(FRAME_BYTES + 1))
                if "id" in response:
                    self.assert_id(response)
                    return response
                self.notifications.append(response)

    def assert_id(self, response):
        if response["id"] != self.counter:
            raise AssertionError("Mismatched MCP ID")

    def tool(self, name, **arguments):
        return self.rpc("tools/call", {"name": name, "arguments": arguments})["result"]

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.process.stdout.close()
        self.process.stderr.close()


class BridgeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="rybridge-", dir="/tmp")
        self.root = Path(self.temp.name).resolve()
        for name, credential in (("codex", "a" * 64), ("claude", "b" * 64)):
            path = self.root / f"identity-{name}.json"
            path.write_text(json.dumps({"name": name, "credential": credential}))
            path.chmod(0o600)
        self.hosts = [Host(self.root, 1), Host(self.root, 2, detached=True)]
        self.bridges = []
        self.clients = []

    def tearDown(self):
        for client in self.clients:
            client.close()
        for bridge in self.bridges:
            bridge.close()
        for host in self.hosts:
            host.close()
        self.temp.cleanup()

    def inventory(self, hidden=False):
        return {"schema": "runyte.context.discovery.v1", "truncated": False,
                "workspaces": [host.record for host in self.hosts]}

    def bridge(self, name="codex"):
        bridge = Bridge(name=name, root=self.root, timeout=.2, discovery=self.inventory)
        self.bridges.append(bridge)
        return bridge

    def targets(self, bridge):
        result, offset = [], 0
        while True:
            page = bridge.list_workspaces(offset=offset)
            result.extend(item["workspace"] for item in page["workspaces"])
            offset = page["next"]
            if offset is None:
                return result

    def terminal(self, bridge, workspace):
        return call(bridge, "list_terminals", {"workspace": workspace})["data"]["terminals"][0]["terminal"]

    def test_explicit_cross_workspace_routing_and_two_independent_readers(self):
        first, second = self.bridge(), self.bridge("claude")
        workspaces = self.targets(first)
        self.targets(second)
        self.assertNotIn("edit_buffer", {x["name"] for x in descriptors(second)})
        self.assertIn("edit_buffer", {x["name"] for x in descriptors(first)})
        handles = [self.terminal(first, workspace) for workspace in workspaces]
        other = self.terminal(second, workspaces[1])
        result = call(second, "read_terminal", {"workspace": workspaces[1], "terminal": other})
        self.assertEqual(result["data"]["rows"][0]["text"], self.hosts[1].text)
        self.assertEqual(result["source_content"], "untrusted")
        self.assertEqual(result["provenance"]["host_incarnation"], self.hosts[1].record["host_incarnation"])
        for workspace, handle in ((workspaces[1], handles[0]), (workspaces[1], other)):
            with self.assertRaises(Failure) as error:
                call(first, "read_terminal", {"workspace": workspace, "terminal": handle})
            self.assertEqual(error.exception.code, "stale")
        with self.assertRaises(Failure):
            call(first, "read_terminal", {"terminal": handles[0]})
        # Discovery fetched only negotiation metadata, never another terminal's content.
        self.assertNotIn("terminal.read", self.hosts[0].methods)

    def test_detached_reads_and_snapshot_paging_remain_explicit(self):
        bridge = self.bridge()
        workspace = self.targets(bridge)[1]
        terminal = self.terminal(bridge, workspace)
        snapshot = call(bridge, "open_terminal_snapshot", {"workspace": workspace, "terminal": terminal})["data"]["snapshot"]
        original = self.hosts[1].text
        self.hosts[1].text = "later"
        page = call(bridge, "read_terminal_snapshot", {"workspace": workspace, "snapshot": snapshot})
        self.assertEqual(page["data"]["rows"][0]["text"], original)
        pane = call(bridge, "list_panes", {"workspace": workspace})["data"]["panes"][0]["pane"]
        with self.assertRaises(Failure) as error:
            call(bridge, "read_pane", {"workspace": workspace, "pane": pane})
        self.assertEqual(error.exception.code, "no_frontend")
        call(bridge, "close_terminal_snapshot", {"workspace": workspace, "snapshot": snapshot})
        self.assertFalse(self.hosts[1].snapshots)

    def test_revocation_denies_without_retry_or_returning_cached_content(self):
        bridge = self.bridge()
        workspace = self.targets(bridge)[0]
        terminal = self.terminal(bridge, workspace)
        self.hosts[0].grants.pop("a" * 64)
        with self.assertRaises(Failure) as error:
            call(bridge, "read_terminal", {"workspace": workspace, "terminal": terminal})
        self.assertEqual(error.exception.code, "capability_denied")
        self.assertNotIn(workspace, bridge.connections)
        self.assertEqual(self.hosts[0].methods.count("terminal.read"), 1)

    def test_native_registration_failure_is_reported_as_denied(self):
        self.hosts[0].grants.pop("a" * 64)
        bridge = self.bridge()
        listed = bridge.list_workspaces()["workspaces"]
        self.assertFalse(listed[0]["readable"])
        self.assertEqual(listed[0]["unavailable_reason"], "capability_denied")
        self.assertTrue(listed[1]["readable"])

    def test_unsupported_release_and_malformed_negotiation_fail_closed(self):
        bridge = self.bridge()
        for value in ({"host_version": "0.4.0"}, {"host_version": None}, {"limits": {"line_bytes": FRAME_BYTES + 1}}):
            self.hosts[0].hello_override = value
            self.assertFalse(bridge.list_workspaces()["workspaces"][0]["readable"])
        self.hosts[0].hello_override = {}
        for value in ({"runyte": ">=0.4.0, <0.5.0"}, {"commands": [{}]},
                      {"capabilities": ["buffer_edit"]}, {"features": ["runyte.context.v1", "unexpected"]}):
            self.hosts[0].registered_override = value
            self.assertFalse(bridge.list_workspaces()["workspaces"][0]["readable"])

    def test_authoritative_stale_edit_error_keeps_connection_and_handles(self):
        bridge = self.bridge()
        workspace = self.targets(bridge)[0]
        buffer = call(bridge, "list_buffers", {"workspace": workspace})["data"]["buffers"][0]
        with self.assertRaises(Failure) as error:
            call(bridge, "edit_buffer", {"workspace": workspace, "buffer": buffer["buffer"],
                "expected_revision": "r:0", "changes": [{"from": 0, "to": 0, "text": "x"}]})
        self.assertEqual(error.exception.code, "stale")
        self.assertIn(workspace, bridge.connections)
        self.assertEqual(call(bridge, "read_buffer", {"workspace": workspace, "buffer": buffer["buffer"],
            "expected_revision": buffer["revision"], "from": 0, "to": 8})["data"]["text"], "original")

    def test_closed_revoked_connection_is_not_advertised_as_readable(self):
        bridge = self.bridge()
        workspace = self.targets(bridge)[0]
        self.terminal(bridge, workspace)
        self.hosts[0].grants.pop("a" * 64)
        for peer in self.hosts[0].peers:
            try:
                peer.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        listed = bridge.list_workspaces()["workspaces"]
        self.assertFalse(listed[0]["readable"])
        self.assertEqual(listed[0]["unavailable_reason"], "capability_denied")
        self.assertIn("b" * 64, self.hosts[0].grants)

    def test_malformed_mutation_acknowledgment_is_uncertain(self):
        bridge = self.bridge()
        workspace = self.targets(bridge)[0]
        for malformed in ({"result": {}, "error": {"code": "invalid_argument", "message": "x"}},
                          {"error": []}, {"error": {"code": "stale", "message": 7}}):
            terminal = self.terminal(bridge, workspace)
            self.hosts[0].malformed_next = malformed
            with self.assertRaises(Failure) as error:
                call(bridge, "propose_terminal_text", {"workspace": workspace, "terminal": terminal, "text": "x"})
            self.assertEqual(error.exception.code, "outcome_unknown")
        self.assertEqual(self.hosts[0].methods.count("terminal.input.propose"), 3)

    def test_multiline_edit_and_native_proposal_never_offer_submission(self):
        bridge = self.bridge()
        workspace = self.targets(bridge)[0]
        buffer = call(bridge, "list_buffers", {"workspace": workspace})["data"]["buffers"][0]
        call(bridge, "edit_buffer", {"workspace": workspace, "buffer": buffer["buffer"],
            "expected_revision": buffer["revision"], "changes": [{"from": 0, "to": 0, "text": "new\nlines\n"}]})
        self.assertTrue(self.hosts[0].buffer.startswith("new\nlines\n"))
        terminal = self.terminal(bridge, workspace)
        proposal = call(bridge, "propose_terminal_text", {"workspace": workspace, "terminal": terminal,
            "text": "echo hello", "reason": "Prepare a greeting"})["data"]
        self.assertEqual(proposal["state"], "pending")
        self.assertFalse(self.hosts[0].inputs)
        status = call(bridge, "terminal_proposal_status", {"workspace": workspace, "proposal": proposal["proposal"]})
        self.assertEqual(status["data"]["state"], "pending")
        for text in ("echo x\n", "echo x\r", "\x1b[201~", "\x85", "\u2028", "\u2029", "\t", "\x7f"):
            with self.assertRaises(Failure):
                call(bridge, "propose_terminal_text", {"workspace": workspace, "terminal": terminal, "text": text})
        with self.assertRaises(Failure):
            call(bridge, "propose_terminal_text", {"workspace": workspace, "terminal": terminal, "text": "x", "submit": True})
        for reason in ("x" * 257, "\nreason", "\x85"):
            with self.assertRaises(Failure):
                call(bridge, "propose_terminal_text", {"workspace": workspace, "terminal": terminal, "text": "x", "reason": reason})
        call(bridge, "cancel_terminal_proposal", {"workspace": workspace, "proposal": proposal["proposal"]})
        self.assertEqual(self.hosts[0].proposals["i:1"]["state"], "cancelled")
        self.assertEqual(self.hosts[0].methods.count("terminal.input.propose"), 1)

    def test_uncertain_mutation_is_not_replayed_and_stale_handles_cannot_reconnect(self):
        bridge = self.bridge()
        workspace = self.targets(bridge)[0]
        terminal = self.terminal(bridge, workspace)
        self.hosts[0].fail_next = "terminal.input.propose"
        with self.assertRaises(Failure) as error:
            call(bridge, "propose_terminal_text", {"workspace": workspace, "terminal": terminal, "text": "literal"})
        self.assertEqual(error.exception.code, "outcome_unknown")
        with self.assertRaises(Failure) as error:
            call(bridge, "read_terminal", {"workspace": workspace, "terminal": terminal})
        self.assertEqual(error.exception.code, "stale")
        self.assertEqual(self.hosts[0].methods.count("terminal.input.propose"), 1)

    def test_exhausted_request_budget_is_known_not_sent(self):
        bridge = self.bridge()
        workspace = self.targets(bridge)[0]
        terminal = self.terminal(bridge, workspace)
        bridge.connections[workspace].counter = 1024
        with self.assertRaises(Failure) as error:
            call(bridge, "propose_terminal_text", {"workspace": workspace, "terminal": terminal, "text": "x"})
        self.assertEqual(error.exception.code, "stale")
        self.assertNotIn("terminal.input.propose", self.hosts[0].methods)

    def test_one_unavailable_host_does_not_hide_healthy_results(self):
        self.hosts[0].slow = True
        bridge = self.bridge()
        started = time.monotonic()
        result = bridge.list_workspaces()
        self.assertLess(time.monotonic() - started, .7)
        self.assertFalse(result["workspaces"][0]["readable"])
        self.assertTrue(result["workspaces"][1]["readable"])

    def test_discovery_admits_no_more_than_available_parallel_probes(self):
        self.hosts.extend(Host(self.root, number) for number in range(3, 12))
        bridge = self.bridge()
        workspaces = self.targets(bridge)
        for workspace in workspaces[:7]:
            self.terminal(bridge, workspace)
        page = bridge.list_workspaces(offset=7, limit=64)
        self.assertEqual(len(page["workspaces"]), 1)
        self.assertEqual(page["next"], 8)

    def test_eight_connection_lru_and_incarnation_changes_invalidate_handles(self):
        self.hosts.extend(Host(self.root, number) for number in range(3, 10))
        bridge = self.bridge()
        workspaces = self.targets(bridge)
        terminal = self.terminal(bridge, workspaces[0])
        for workspace in workspaces[1:]:
            self.terminal(bridge, workspace)
        self.assertEqual(len(bridge.connections), 8)
        with self.assertRaises(Failure) as error:
            call(bridge, "read_terminal", {"workspace": workspaces[0], "terminal": terminal})
        self.assertEqual(error.exception.code, "stale")
        self.hosts[1].record = {**self.hosts[1].record, "host_incarnation": "f" * 64}
        bridge.list_workspaces()
        self.assertNotIn(workspaces[1], bridge.records)

    def test_private_credential_files_are_never_followed_or_logged(self):
        self.assertEqual(load_credential(self.root, "codex"), "a" * 64)
        path = self.root / "identity-codex.json"
        path.chmod(0o644)
        with self.assertRaises(Failure):
            load_credential(self.root, "codex")
        path.chmod(0o600)
        os.link(path, self.root / "hardlink")
        with self.assertRaises(Failure):
            load_credential(self.root, "codex")
        (self.root / "hardlink").unlink()
        (self.root / "identity-link.json").symlink_to(path)
        with self.assertRaises(Failure):
            load_credential(self.root, "link")
        with self.assertRaises(Failure):
            load_credential(self.root, "../codex")

    def test_two_real_mcp_stdio_clients_route_to_two_hosts(self):
        (self.root / "inventory.json").write_text(json.dumps(self.inventory()))
        (self.root / "runyte").symlink_to(REPO / "src/fixtures/stand-in")
        (self.root / "runyte.behavior").write_text('cat "$RUNYTE_CONTEXT_TEST_INVENTORY"\n')
        first, second = MCPClient(self.root, "codex"), MCPClient(self.root, "claude")
        self.clients.extend((first, second))
        initial = first.rpc("tools/list", {})["result"]["tools"]
        self.assertNotIn("edit_buffer", {tool["name"] for tool in initial})
        first_inventory = first.tool("list_workspaces")["structuredContent"]
        second_inventory = second.tool("list_workspaces")["structuredContent"]
        self.assertEqual(len(first_inventory["workspaces"]), 2)
        self.assertEqual(len(second_inventory["workspaces"]), 2)
        for client, workspace in ((first, first_inventory["workspaces"][0]["workspace"]),
                                  (second, second_inventory["workspaces"][1]["workspace"])):
            terminal = client.tool("list_terminals", workspace=workspace)["structuredContent"]["data"]["terminals"][0]["terminal"]
            response = client.tool("read_terminal", workspace=workspace, terminal=terminal)
            self.assertFalse(response["isError"])
            self.assertEqual(response["structuredContent"]["source_content"], "untrusted")
        self.assertTrue(any(n["method"] == "notifications/tools/list_changed" for n in first.notifications))

    def test_strict_json_lifecycle_and_bounded_stdio(self):
        for raw in (b'{"id":1,"id":2}', b'{"a":NaN}', b'[' * 30 + b']' * 30):
            with self.assertRaises(Failure):
                decode(raw)
        server = Server(self.bridge())
        bad = server.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
        self.assertIn("error", bad[0])
        for request_id in ("x" * 129, 2**53):
            bad = server.handle({"jsonrpc": "2.0", "id": request_id, "method": "ping"})
            self.assertIsNone(bad[0]["id"])
        output = io.BytesIO()
        server.run(io.BytesIO(b"x" * (FRAME_BYTES + 1)), output)
        self.assertEqual(output.getvalue(), b"")

    def test_initialize_routes_the_common_same_project_workflows(self):
        server = Server(self.bridge())
        response = server.handle({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": PROTOCOL},
        })[0]["result"]
        self.assertEqual(response["instructions"], INSTRUCTIONS)
        self.assertLessEqual(len(INSTRUCTIONS), 512)
        self.assertIn("root matches the client's current project", INSTRUCTIONS)
        for route in (
            "Buffer read: list_buffers then read_buffer",
            "Terminal read: list_terminals then read_terminal",
            "Buffer write: list_buffers then edit_buffer",
            "Terminal write: list_terminals then propose_terminal_text then terminal_proposal_status",
        ):
            self.assertIn(route, INSTRUCTIONS)


if __name__ == "__main__":
    unittest.main()
