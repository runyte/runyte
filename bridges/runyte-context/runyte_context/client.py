# SPDX-License-Identifier: MPL-2.0
"""Bounded Unix transport and explicit, incarnation-bound resource routing."""

import base64
import collections
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import secrets
import select
import selectors
import socket
import stat
import struct
import subprocess
import sys
import time

FRAME_BYTES = 2 * 1024 * 1024
FEATURE = "runyte.context.v1"
SCOPES = {"terminal_read", "editor_context_read", "buffer_edit", "terminal_propose"}
RESOURCE_KEYS = {"buffer", "terminal", "pane", "snapshot", "proposal"}
ERROR_CODES = {"invalid_argument", "unsupported", "capability_denied", "not_found", "closed", "stale",
               "conflict", "read_only", "busy", "limit_exceeded", "cancelled", "timeout", "unavailable",
               "internal", "outcome_unknown", "no_frontend", "context_changed"}


class Failure(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


class HostFailure(Failure):
    """An explicit matching host response, rather than uncertain transport loss."""


class NotSent(Failure):
    """Local admission failed before any request bytes were sent."""


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        key.encode("utf-8")
        if key in result:
            raise ValueError("duplicate member")
        result[key] = value
    return result


def decode(raw):
    if len(raw) > FRAME_BYTES:
        raise Failure("limit_exceeded", "JSON frame exceeds limit")
    try:
        value = json.loads(raw, object_pairs_hook=_pairs,
                           parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
        pending = [(value, 0)]
        nodes = 0
        while pending:
            item, depth = pending.pop()
            nodes += 1
            if depth > 24 or nodes > 65536:
                raise ValueError("structure limit")
            if isinstance(item, dict):
                pending.extend((child, depth + 1) for child in item.values())
            elif isinstance(item, list):
                pending.extend((child, depth + 1) for child in item)
            elif isinstance(item, str):
                item.encode("utf-8")
        return value
    except (ValueError, UnicodeError, RecursionError):
        raise Failure("invalid_argument", "Invalid or excessive JSON") from None


def encode(value):
    try:
        raw = json.dumps(value, ensure_ascii=False, separators=(",", ":"),
                         allow_nan=False).encode("utf-8") + b"\n"
    except (ValueError, UnicodeError, RecursionError):
        raise Failure("invalid_argument", "Invalid JSON value") from None
    if len(raw) > FRAME_BYTES:
        raise Failure("limit_exceeded", "JSON frame exceeds limit")
    return raw


def storage_root():
    override = os.environ.get("RUNYTE_CONTEXT_HOME")
    if override is not None:
        root = Path(override)
        if not root.is_absolute():
            raise Failure("unavailable", "Context storage override must be absolute")
        return root
    home = Path(pwd.getpwuid(os.geteuid()).pw_dir)
    return home / ("Library/Caches/runyte/context" if sys.platform == "darwin"
                   else ".cache/runyte/context")


def private_directory(root):
    """Pin components without following links; allow only macOS system aliases."""
    root = Path(root)
    if sys.platform == "darwin":
        for alias in ("/tmp", "/var", "/etc"):
            if root == Path(alias) or Path(alias) in root.parents:
                root = Path("/private") / str(root).lstrip("/")
                break
    if not root.is_absolute() or ".." in root.parts:
        raise Failure("unavailable", "Invalid private storage path")
    descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for component in root.parts[1:]:
            child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        info = os.fstat(descriptor)
        if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) & 0o077:
            raise Failure("unavailable", "Context directory must be owner-private")
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def load_credential(root, name):
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", name):
        raise Failure("invalid_argument", "Invalid bridge identity name")
    filename = "identity.json" if name == "agent" else f"identity-{name}.json"
    try:
        directory = private_directory(root)
        try:
            descriptor = os.open(filename, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                                 dir_fd=directory)
        finally:
            os.close(directory)
        with os.fdopen(descriptor, "rb") as source:
            info = os.fstat(source.fileno())
            if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                    or info.st_nlink != 1 or stat.S_IMODE(info.st_mode) & 0o077):
                raise Failure("unavailable", "Bridge identity must be an owned private file")
            raw = source.read(65537)
        if len(raw) > 65536:
            raise Failure("limit_exceeded", "Identity record exceeds limit")
        value = decode(raw)
        if (not isinstance(value, dict) or set(value) != {"name", "credential"}
                or value["name"] != name or not isinstance(value["credential"], str)
                or not re.fullmatch(r"[0-9a-f]{64}", value["credential"])):
            raise Failure("unavailable", "Invalid bridge identity record")
        return value["credential"]
    except OSError:
        raise Failure("unavailable", "Pair this identity in Runyte before connecting") from None


def discover(executable, hidden, timeout):
    argv = [executable, "--context-list", "--json"]
    if hidden:
        argv.append("--include-hidden")
    try:
        with subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL) as child:
            try:
                data = bytearray()
                deadline = time.monotonic() + timeout
                with selectors.DefaultSelector() as poll:
                    poll.register(child.stdout, selectors.EVENT_READ)
                    while True:
                        remaining = deadline - time.monotonic()
                        if remaining <= 0 or not poll.select(remaining):
                            raise Failure("timeout", "Workspace discovery timed out")
                        chunk = os.read(child.stdout.fileno(), 65536)
                        if not chunk:
                            break
                        data.extend(chunk)
                        if len(data) > FRAME_BYTES:
                            raise Failure("limit_exceeded", "Workspace discovery exceeds limit")
                if child.wait(timeout=max(.01, deadline - time.monotonic())) != 0:
                    raise Failure("unavailable", "Workspace discovery failed")
            except BaseException:
                child.kill()
                child.wait()
                raise
        value = decode(data)
    except (OSError, subprocess.TimeoutExpired):
        raise Failure("unavailable", "Cannot run workspace discovery") from None
    if (not isinstance(value, dict) or value.get("schema") != "runyte.context.discovery.v1"
            or not isinstance(value.get("workspaces"), list) or len(value["workspaces"]) > 512):
        raise Failure("unsupported", "Unsupported workspace discovery schema")
    return value


def workspace_key(record):
    data = json.dumps([record["host_incarnation"], record["endpoint"]], separators=(",", ":"))
    return "w:" + hashlib.sha256(data.encode()).hexdigest()


class Connection:
    def __init__(self, record, credential, name, timeout, root):
        self.record = record
        self.token = secrets.token_hex(16)
        self.scopes = set()
        self.counter = 0
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(timeout)
        self.timeout = timeout
        self.frame_bytes = FRAME_BYTES
        self.handshake_deadline = time.monotonic() + timeout
        self.pending = bytearray()
        try:
            endpoint = Path(record["endpoint"])
            # Discovery is a hint. Verify the endpoint before sending a credential.
            if endpoint.parent.resolve() != Path(root).resolve():
                raise Failure("unavailable", "Endpoint is outside private context storage")
            directory = private_directory(endpoint.parent)
            try:
                info = os.stat(endpoint.name, dir_fd=directory, follow_symlinks=False)
                if (not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.geteuid()
                        or stat.S_IMODE(info.st_mode) & 0o077):
                    raise Failure("unavailable", "Context endpoint must be owner-private")
                self.sock.connect(str(endpoint))
                after = os.stat(endpoint.name, dir_fd=directory, follow_symlinks=False)
                if (info.st_ino, info.st_dev) != (after.st_ino, after.st_dev):
                    raise Failure("stale", "Context endpoint changed during connection")
            finally:
                os.close(directory)
            if sys.platform.startswith("linux"):
                _, uid, _ = struct.unpack("3i", self.sock.getsockopt(socket.SOL_SOCKET,
                                                                  socket.SO_PEERCRED, 12))
                if uid != os.geteuid():
                    raise Failure("unavailable", "Context peer has a different owner")
            elif sys.platform == "darwin":
                # LOCAL_PEERCRED: xucred { unsigned version; uid_t uid; ... }.
                peer = self.sock.getsockopt(0, 1, 84)
                version, uid = struct.unpack_from("II", peer)
                if version != 0 or uid != os.geteuid():
                    raise Failure("unavailable", "Context peer has a different owner")
            else:
                raise Failure("unsupported", "Context bridge supports Linux and macOS")
            hello = self.exchange({"type": "authenticate", "credential": credential})
            self.check_error(hello)
            if (hello.get("type") != "hello" or hello.get("version") != "runyte-1"
                    or not isinstance(hello.get("features"), list)
                    or not isinstance(hello.get("workspace"), dict)
                    or FEATURE not in hello.get("features", [])
                    or any(hello.get("workspace", {}).get(key) != record.get(key)
                           for key in ("host_incarnation", "endpoint", "workspace_id", "root"))):
                raise Failure("stale", "Context host identity or protocol changed")
            version = hello.get("host_version")
            if not isinstance(version, str) or not re.fullmatch(r"0\.3\.(0|[1-9][0-9]*)(?:\+[A-Za-z0-9.-]+)?", version):
                raise Failure("unsupported", "Host release is outside the supported stable range")
            self.set_limits(hello.get("limits"))
            registered = self.exchange({"type": "register", "version": "runyte-1",
                "runyte": ">=0.3.0, <0.4.0", "name": f"Runyte MCP ({name})", "commands": [],
                "required_features": [FEATURE], "optional_features": [],
                "required_capabilities": [], "optional_capabilities": sorted(SCOPES)})
            self.check_error(registered)
            if (registered.get("type") != "registered"
                    or not isinstance(registered.get("features"), list)
                    or registered["features"] != [FEATURE]
                    or registered.get("runyte") != ">=0.3.0, <0.4.0"
                    or registered.get("commands") != []
                    or not isinstance(registered.get("capabilities"), list)
                    or not all(isinstance(scope, str) for scope in registered["capabilities"])):
                raise Failure("unsupported", "Context profile negotiation failed")
            self.set_limits(registered.get("limits"))
            self.scopes = set(registered.get("capabilities", []))
            if (not self.scopes <= SCOPES
                    or len(self.scopes) != len(registered["capabilities"])
                    or ("buffer_edit" in self.scopes and "editor_context_read" not in self.scopes)
                    or ("terminal_propose" in self.scopes and "terminal_read" not in self.scopes)):
                raise Failure("unsupported", "Unknown granted context scope")
            self.handshake_deadline = None
        except BaseException:
            self.close()
            raise

    def close(self):
        self.sock.close()

    def set_limits(self, limits):
        if (not isinstance(limits, dict) or type(limits.get("line_bytes")) is not int
                or not 256 <= limits["line_bytes"] <= FRAME_BYTES):
            raise Failure("unsupported", "Invalid context frame limit")
        self.frame_bytes = min(self.frame_bytes, limits["line_bytes"])

    def alive(self):
        # There are no unsolicited host messages in this negotiated profile.
        # Readability while idle therefore means EOF or an invalid extra reply.
        try:
            return not self.pending and not select.select([self.sock], [], [], 0)[0]
        except (OSError, ValueError):
            return False

    @staticmethod
    def check_error(value):
        if "error" in value or value.get("type") == "registration_error":
            error = value.get("error", value)
            if (not isinstance(error, dict) or not isinstance(error.get("code"), str)
                    or error["code"] not in ERROR_CODES or not isinstance(error.get("message"), str)):
                raise Failure("unavailable", "Malformed context error")
            raise HostFailure(str(error.get("code", "unavailable")),
                          str(error.get("message", "Context request failed")))

    def exchange(self, value):
        deadline = self.handshake_deadline or (time.monotonic() + self.timeout)
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError()
        self.sock.settimeout(remaining)
        try:
            packet = encode(value)
        except Failure as error:
            raise NotSent(error.code, error.message) from None
        if len(packet) > self.frame_bytes:
            raise NotSent("limit_exceeded", "Request exceeds negotiated frame limit")
        self.sock.sendall(packet)
        while b"\n" not in self.pending:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError()
            self.sock.settimeout(remaining)
            chunk = self.sock.recv(min(65536, self.frame_bytes + 1 - len(self.pending)))
            if not chunk:
                raise ConnectionError()
            self.pending.extend(chunk)
            if len(self.pending) > self.frame_bytes:
                raise Failure("limit_exceeded", "Context reply exceeds limit")
        raw, _, rest = self.pending.partition(b"\n")
        self.pending = bytearray(rest)
        value = decode(raw)
        if not isinstance(value, dict):
            raise Failure("unavailable", "Invalid context reply")
        return value

    def request(self, method, params):
        if self.counter >= 1024:
            raise NotSent("stale", "Connection request budget exhausted; rediscover resources")
        self.counter += 1
        request_id = f"m:{self.counter}"
        # Never retry this exchange, including when delivery is uncertain.
        value = self.exchange({"type": "request", "id": request_id,
                               "method": method, "params": params})
        if value.get("type") != "response" or value.get("id") != request_id:
            raise Failure("unavailable", "Mismatched context response")
        if ("result" in value) == ("error" in value):
            raise Failure("unavailable", "Context response must have exactly one outcome")
        self.check_error(value)
        result = value.get("result")
        if not isinstance(result, dict):
            raise Failure("unavailable", "Invalid context result")
        return result


class Bridge:
    def __init__(self, executable="runyte", name="agent", timeout=2.0, root=None,
                 discovery=None):
        self.executable, self.name, self.timeout = executable, name, timeout
        self.root = Path(root) if root is not None else storage_root()
        self.discovery = discovery or (lambda hidden: discover(executable, hidden, 5.0))
        self.records = {}
        self.known_scopes = {}
        self.connections = collections.OrderedDict()

    def close(self):
        for connection in self.connections.values():
            connection.close()
        self.connections.clear()

    def granted_scopes(self):
        return set().union(*self.known_scopes.values()) if self.known_scopes else set()

    def _connect(self, record):
        try:
            return Connection(record, load_credential(self.root, self.name), self.name,
                              self.timeout, self.root)
        except OSError:
            raise Failure("unavailable", "Context host is unavailable or timed out") from None

    def list_workspaces(self, include_hidden=False, offset=0, limit=16):
        inventory = self.discovery(include_hidden)
        candidates = inventory["workspaces"]
        records = {}
        for record in candidates:
            if (not isinstance(record, dict) or not isinstance(record.get("endpoint"), str)
                    or not isinstance(record.get("root"), str)
                    or not re.fullmatch(r"[0-9a-f]{64}", str(record.get("host_incarnation", "")))):
                continue
            records[workspace_key(record)] = record
        for key in list(self.connections):
            if key not in records or not self.connections[key].alive():
                self.connections.pop(key).close()
                self.known_scopes.pop(key, None)
        self.records = records
        self.known_scopes = {key: scopes for key, scopes in self.known_scopes.items() if key in records}
        available = 8 - len(self.connections)
        page = []
        probes = 0
        for key, record in list(records.items())[offset:offset + limit]:
            if key not in self.connections and available:
                if probes == available:
                    break
                probes += 1
            page.append((key, record))
        output = {}

        def probe(item):
            key, record = item
            try:
                connection = self._connect(record)
                try:
                    return key, connection.scopes, None
                finally:
                    connection.close()
            except Failure as error:
                return key, set(), error.code

        pending = []
        for key, record in page:
            if key in self.connections:
                # An existing authenticated connection remains authoritative;
                # any later access is still rechecked by the host's live grant.
                output[key] = (self.connections[key].scopes, None)
            else:
                pending.append((key, record))
        if available:
            with concurrent.futures.ThreadPoolExecutor(max_workers=available) as pool:
                for key, scopes, error in pool.map(probe, pending):
                    output[key] = (scopes, error)
        else:
            for key, _ in pending:
                output[key] = (set(), "connection_limit")
        workspaces = []
        for key, record in page:
            scopes, error = output[key]
            self.known_scopes[key] = scopes
            workspaces.append({"workspace": key, "label": record.get("label", record["root"]),
                "root": record["root"], "host_incarnation": record["host_incarnation"],
                "mode": record.get("mode"), "readable": bool(scopes & {"terminal_read", "editor_context_read"}),
                "scopes": sorted(scopes), "unavailable_reason": error})
        return {"workspaces": workspaces, "next": offset + len(page) if offset + len(page) < len(records) else None,
                "truncated": bool(inventory.get("truncated", False)), "content_read": False}

    def prime_scopes(self):
        """Learn grants that already exist, so a client's first tool list includes them.

        Some clients fetch tools once and ignore list_changed. This runs the same
        bounded discovery as list_workspaces but keeps only the scopes: targets
        still have to be discovered explicitly before any call can use them.
        """
        try:
            self.list_workspaces()
        except Failure:
            pass  # Fall back to read tools; list_workspaces reports the reason.
        self.records = {}

    def connection(self, workspace, resource=False):
        if workspace not in self.records:
            raise Failure("not_found", "Discover this explicit workspace first")
        if workspace in self.connections:
            value = self.connections.pop(workspace)
            self.connections[workspace] = value
            return value
        if resource:
            raise Failure("stale", "Connection changed; rediscover resource handles")
        if len(self.connections) == 8:
            _, evicted = self.connections.popitem(last=False)
            evicted.close()
        value = self._connect(self.records[workspace])
        self.connections[workspace] = value
        self.known_scopes[workspace] = value.scopes
        return value

    def _wrap(self, workspace, connection, value):
        if isinstance(value, list):
            return [self._wrap(workspace, connection, child) for child in value]
        if not isinstance(value, dict):
            return value
        result = {}
        for key, child in value.items():
            if key in RESOURCE_KEYS and isinstance(child, str):
                payload = [workspace, connection.token, key, child]
                result[key] = "h:" + base64.urlsafe_b64encode(encode(payload).strip()).decode()
            else:
                result[key] = self._wrap(workspace, connection, child)
        return result

    def request(self, workspace, method, scope, params):
        resources = {key: value for key, value in params.items() if key in RESOURCE_KEYS}
        connection = self.connection(workspace, bool(resources))
        if scope not in connection.scopes:
            raise Failure("capability_denied", "This workspace has not granted the required scope")
        params = dict(params)
        for kind, handle in resources.items():
            try:
                if not handle.startswith("h:") or len(handle) > 2048:
                    raise ValueError()
                decoded = decode(base64.b64decode(handle[2:], altchars=b"-_", validate=True))
                owner, token, actual_kind, native = decoded
                if (owner != workspace or token != connection.token or actual_kind != kind
                        or not isinstance(native, str)):
                    raise ValueError()
                params[kind] = native
            except (ValueError, TypeError, Failure):
                raise Failure("stale", "Resource does not belong to this workspace connection") from None
        mutating = method in {"buffer.edit", "buffer.append", "terminal.input.propose", "terminal.input.cancel"}
        try:
            result = connection.request(method, params)
        except (OSError, Failure) as error:
            # Protocol/IO failures cannot establish whether a mutation arrived.
            uncertain = not isinstance(error, (HostFailure, NotSent))
            if uncertain or error.code == "capability_denied" or (isinstance(error, NotSent) and error.code == "stale"):
                self.connections.pop(workspace, None)
                self.known_scopes.pop(workspace, None)
                connection.close()
            if mutating and uncertain:
                raise Failure("outcome_unknown", "Delivery is uncertain; do not retry this mutation automatically") from None
            if isinstance(error, OSError):
                raise Failure("unavailable", "Context connection closed or timed out; rediscover resources") from None
            raise
        return {"provenance": {"workspace": workspace, "host_incarnation": connection.record["host_incarnation"],
                               "connection": connection.token},
                "source_content": "untrusted", "data": self._wrap(workspace, connection, result)}
