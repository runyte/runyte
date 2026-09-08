# SPDX-License-Identifier: MPL-2.0
"""Bounded local SSH/SFTP fixture. All keys and filesystem data are temporary."""

import errno
import os
from pathlib import Path
import socket
import struct
import tempfile
import threading

import paramiko
from paramiko.sftp import CMD_INIT, CMD_VERSION, _VERSION


class _Authentication(paramiko.ServerInterface):
    def __init__(self, fixture):
        self.fixture = fixture

    def check_auth_publickey(self, username, key):
        if (self.fixture.accept_client and username == 'fixture'
                and key == self.fixture.client_key):
            return paramiko.AUTH_SUCCESSFUL
        return paramiko.AUTH_FAILED

    def get_allowed_auths(self, username):
        return 'publickey'

    def check_channel_request(self, kind, channel_id):
        if kind == 'session':
            return paramiko.OPEN_SUCCEEDED
        return paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED


class _Server(paramiko.SFTPServer):
    def _send_server_version(self):
        message_type, data = self._read_packet()
        if message_type != CMD_INIT:
            raise paramiko.SFTPError('Expected SFTP INIT')
        version = struct.unpack('>I', data[:4])[0]
        message = paramiko.Message()
        message.add_int(_VERSION)
        if self.server.fixture.posix_rename:
            message.add('posix-rename@openssh.com', '1')
        self._send_packet(CMD_VERSION, message)
        return version


class _Handle(paramiko.SFTPHandle):
    def __init__(self, fixture, path, flags, stream):
        super().__init__(flags)
        self.fixture, self.path = fixture, path
        self.writable = (flags & os.O_ACCMODE) != os.O_RDONLY
        if self.writable:
            self.writefile = stream
        if (flags & os.O_ACCMODE) != os.O_WRONLY:
            self.readfile = stream

    def stat(self):
        stream = getattr(self, 'readfile', None) or self.writefile
        try:
            return paramiko.SFTPAttributes.from_stat(os.fstat(stream.fileno()))
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def read(self, offset, length):
        with self.fixture._lock:
            self.fixture.read_count += 1
            if self.fixture.read_count >= 2:
                self.fixture.two_reads_started.set()
        self.fixture.read_started.set()
        if not self.fixture.read_release.wait(15):
            return paramiko.SFTP_FAILURE
        return super().read(offset, length)

    def close(self):
        result = super().close()
        if (self.writable and self.path.name.startswith('.runyte-upload-')
                and self.fixture.on_write_close is not None):
            self.fixture.on_write_close(self.path)
        return result


class _Filesystem(paramiko.SFTPServerInterface):
    def __init__(self, server, fixture):
        super().__init__(server)
        self.fixture = fixture

    def _path(self, path, follow=True):
        candidate = Path(path)
        if not candidate.is_absolute():
            candidate = self.fixture.root / candidate
        checked = candidate.resolve() if follow else candidate.parent.resolve() / candidate.name
        if not checked.is_relative_to(self.fixture.root):
            raise PermissionError(errno.EACCES, 'Outside fixture root')
        return checked

    def canonicalize(self, path):
        candidate = Path(path)
        if not candidate.is_absolute():
            candidate = self.fixture.root / candidate
        # REALPATH must expose outside-root aliases so the client can reject
        # them itself. File operations still enforce the fixture boundary.
        self.fixture.record('realpath', path)
        return str(candidate.resolve())

    def stat(self, path):
        self.fixture.record('stat', path)
        try:
            return paramiko.SFTPAttributes.from_stat(self._path(path).stat())
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def lstat(self, path):
        try:
            return paramiko.SFTPAttributes.from_stat(self._path(path, False).lstat())
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def list_folder(self, path):
        self.fixture.record('list', path)
        try:
            result = []
            for child in sorted(self._path(path).iterdir()):
                attributes = paramiko.SFTPAttributes.from_stat(child.lstat())
                attributes.filename = child.name
                result.append(attributes)
                if len(result) >= 1100:
                    break
            return result
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def open(self, path, flags, attributes):
        self.fixture.record('open', path, flags)
        try:
            candidate = self._path(path)
            descriptor = os.open(candidate, flags, 0o600)
            access = flags & os.O_ACCMODE
            mode = 'rb' if access == os.O_RDONLY else ('wb' if access == os.O_WRONLY else 'r+b')
            stream = os.fdopen(descriptor, mode, buffering=0)
            return _Handle(self.fixture, candidate, flags, stream)
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def remove(self, path):
        self.fixture.record('remove', path)
        try:
            self.fixture.namespace_begin('delete', path)
            self._path(path, False).unlink()
            self.fixture.namespace_finish('delete', path)
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def chattr(self, path, attributes):
        self.fixture.record('chattr', path)
        try:
            paramiko.SFTPServer.set_file_attr(str(self._path(path)), attributes)
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def rename(self, oldpath, newpath):
        self.fixture.record('rename', oldpath, newpath)
        if not self.fixture.is_namespace(oldpath):
            return paramiko.SFTP_OP_UNSUPPORTED
        try:
            self.fixture.namespace_begin('rename', oldpath)
            source, destination = self._path(oldpath, False), self._path(newpath, False)
            if destination.exists() or destination.is_symlink():
                return paramiko.SFTP_FAILURE
            source.rename(destination)
            self.fixture.namespace_finish('rename', oldpath)
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def mkdir(self, path, attributes):
        self.fixture.record('mkdir', path)
        try:
            self.fixture.namespace_begin('mkdir', path)
            self._path(path, False).mkdir(mode=attributes.st_mode or 0o700)
            self.fixture.namespace_finish('mkdir', path)
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def rmdir(self, path):
        self.fixture.record('rmdir', path)
        try:
            self.fixture.namespace_begin('delete', path)
            self._path(path, False).rmdir()
            self.fixture.namespace_finish('delete', path)
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def posix_rename(self, oldpath, newpath):
        self.fixture.record('posix_rename', oldpath, newpath)
        promotion = Path(oldpath).name.startswith('.runyte-upload-')
        if promotion:
            self.fixture.rename_entered.set()
        if not self.fixture.posix_rename:
            return paramiko.SFTP_OP_UNSUPPORTED
        try:
            source, destination = self._path(oldpath), self._path(newpath)
            if promotion and self.fixture.before_rename is not None:
                self.fixture.before_rename(source, destination)
            os.replace(source, destination)
            if promotion:
                self.fixture.rename_done.set()
            if promotion and self.fixture.drop_rename_reply:
                self.fixture.disconnect_clients()
            elif promotion:
                self.fixture.rename_release.wait(15)
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)


class SftpFixture:
    """Actual SSH server with deterministic staging/promotion failure controls."""
    def __init__(self, *, posix_rename=True, accept_client=True):
        self.temporary = tempfile.TemporaryDirectory(prefix='runyte-sftp-fixture-')
        self.base = Path(self.temporary.name).resolve()
        self.root = self.base / 'remote'
        self.root.mkdir()
        self.posix_rename, self.accept_client = posix_rename, accept_client
        self.host_key = paramiko.RSAKey.generate(2048)
        self.client_key = paramiko.RSAKey.generate(2048)
        self.identity = self.base / 'identity'
        self.client_key.write_private_key_file(str(self.identity))
        self.identity.chmod(0o600)
        self.known_hosts = self.base / 'known_hosts'
        self.operations = []
        self.errors = []
        self.on_write_close = self.before_rename = None
        self.before_namespace = self.after_namespace = None
        self.namespace_entered, self.namespace_done = threading.Event(), threading.Event()
        self.namespace_release = threading.Event()
        self.namespace_release.set()
        self.drop_namespace_reply = False
        self.drop_rename_reply = False
        self.rename_entered = threading.Event()
        self.rename_done = threading.Event()
        self.rename_release = threading.Event()
        self.rename_release.set()
        self.read_started = threading.Event()
        self.two_reads_started = threading.Event()
        self.read_count = 0
        self.read_release = threading.Event()
        self.read_release.set()
        self._lock = threading.Lock()
        self._closed = threading.Event()
        self._transports = []
        self._threads = []
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(8)
        self.listener.settimeout(0.1)
        self.port = self.listener.getsockname()[1]
        self.known_hosts.write_text(
            f'[127.0.0.1]:{self.port} {self.host_key.get_name()} {self.host_key.get_base64()}\n')
        self.config = {'alias': 'Fixture', 'host': '127.0.0.1', 'port': self.port,
                       'username': 'fixture', 'root': str(self.root),
                       'known_hosts': str(self.known_hosts),
                       'identity_files': [str(self.identity)], 'allow_agent': False}
        self._acceptor = threading.Thread(target=self._accept, daemon=True)
        self._acceptor.start()

    def record(self, *operation):
        with self._lock:
            if len(self.operations) < 4096:
                self.operations.append(operation)

    @staticmethod
    def is_namespace(path):
        return not any(part.startswith(('.runyte-upload-', '.runyte-probe-')) for part in Path(path).parts)

    def namespace_begin(self, kind, path):
        if self.is_namespace(path):
            self.namespace_entered.set()
            if self.before_namespace:
                self.before_namespace(kind, Path(path))

    def namespace_finish(self, kind, path):
        if self.is_namespace(path):
            self.namespace_done.set()
            if self.after_namespace:
                self.after_namespace(kind, Path(path))
            if self.drop_namespace_reply:
                self.disconnect_clients()
            else:
                self.namespace_release.wait(15)

    def _accept(self):
        while not self._closed.is_set():
            try:
                client, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            with self._lock:
                active = sum(transport.is_active() for transport in self._transports)
            if active >= 8 or len(self._threads) >= 128:
                client.close()
                continue
            worker = threading.Thread(target=self._serve, args=(client,), daemon=True)
            self._threads.append(worker)
            worker.start()

    def _serve(self, client):
        transport = paramiko.Transport(client)
        with self._lock:
            self._transports.append(transport)
        try:
            transport.add_server_key(self.host_key)
            transport.set_subsystem_handler('sftp', _Server, _Filesystem, fixture=self)
            transport.start_server(server=_Authentication(self))
            while transport.is_active() and not self._closed.wait(0.05):
                pass
        except (EOFError, OSError, paramiko.SSHException) as error:
            if not self._closed.is_set():
                self.errors.append(type(error).__name__)
        finally:
            transport.close()

    def disconnect_clients(self):
        with self._lock:
            transports = list(self._transports)
        for transport in transports:
            transport.close()

    def close(self):
        self._closed.set()
        self.rename_release.set()
        self.namespace_release.set()
        self.read_release.set()
        self.listener.close()
        self.disconnect_clients()
        self._acceptor.join(timeout=2)
        for worker in self._threads:
            worker.join(timeout=2)
        self.temporary.cleanup()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
