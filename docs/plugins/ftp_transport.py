# SPDX-License-Identifier: MPL-2.0
"""Bounded explicit FTPS/FTP adapter; no third-party runtime dependencies.

FTPS verifies both control and data TLS. Plain FTP requires an explicit profile
choice. FTP cannot promise private staging permissions, atomic replacement,
compare-and-swap, or confinement against a hostile server namespace.
"""
import errno
import ftplib
import hashlib
import io
import json
import os
import posixpath
import socket
import ssl
import stat
import threading
import time
import unicodedata
import uuid

from transport import BoundedTransport, TransportError

MAX_BYTES = 8 * 1024 * 1024
MAX_ENTRIES = 1024
MAX_METADATA = 4 * 1024 * 1024
MAX_CONTROL = 64 * 1024
MAX_PATH = 3800
BLOCK_BYTES = 128 * 1024
OPERATION_TIMEOUT = 8.0


def _fail(code, message):
    raise TransportError(code, message) from None


def _safe(value, maximum):
    try:
        return (isinstance(value, str) and 0 < len(value.encode('utf-8')) <= maximum
                and not any(unicodedata.category(c) == 'Cc' for c in value))
    except UnicodeError:
        return False


class _BoundedControl:
    def getmultiline(self):
        first = self.getline()
        lines, size = [first], len(first.encode('utf-8'))
        if first[3:4] == '-':
            while True:
                line = self.getline()
                size += len(line.encode('utf-8')) + 1
                if size > MAX_CONTROL or len(lines) >= 128:
                    _fail('limit_exceeded', 'FTP control response exceeds its limit')
                lines.append(line)
                if line[:3] == first[:3] and line[3:4] != '-':
                    break
        return '\n'.join(lines)

    def ntransfercmd(self, cmd, rest=None):
        connection, size = super().ntransfercmd(cmd, rest)
        self._data_socket = connection
        return connection, size

    def close(self):
        # FTP.close closes its buffered reader first, which can wait for another
        # thread's pending read. Shutdown sockets first to unblock that reader.
        for connection in (getattr(self, '_data_socket', None), self.sock):
            if connection is not None:
                try:
                    connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
        super().close()


class _Ftp(_BoundedControl, ftplib.FTP):
    pass


class _Ftps(_BoundedControl, ftplib.FTP_TLS):
    pass


class FtpTransport(BoundedTransport):
    def __init__(self, config):
        allowed = {'transport', 'alias', 'host', 'port', 'username', 'root',
                   'password_file', 'ca_file'}
        if not isinstance(config, dict) or set(config) - allowed:
            _fail('invalid_argument', 'Invalid FTP connection configuration')
        for key, maximum in [('alias', 160), ('host', 253), ('username', 128),
                             ('root', MAX_PATH), ('password_file', 4096)]:
            if not _safe(config.get(key), maximum):
                _fail('invalid_argument', 'Missing or invalid FTP connection setting')
        self.protocol = config.get('transport', 'ftps')
        self.alias, self.host = config['alias'], config['host']
        self.username, self.root = config['username'], config['root']
        self.password_file = config['password_file']
        self.ca_file, self.port = config.get('ca_file'), config.get('port', 21)
        if (self.protocol not in ('ftps', 'ftp') or type(self.port) is not int
                or not 1 <= self.port <= 65535
                or any(c.isspace() or c in '/@' for c in self.host)):
            _fail('invalid_argument', 'Invalid FTP endpoint or transport')
        self.label = ('FTPS' if self.protocol == 'ftps' else 'FTP (unencrypted)') + ' · ' + self.alias
        if (not self.root.startswith('/') or self.root.startswith('//')
                or '..' in self.root.split('/') or posixpath.normpath(self.root) != self.root):
            _fail('invalid_argument', 'FTP root must be an absolute normalized directory')
        if (not os.path.isabs(self.password_file)
                or (self.ca_file is not None and
                    (not _safe(self.ca_file, 4096) or not os.path.isabs(self.ca_file)))
                or (self.protocol == 'ftp' and self.ca_file is not None)):
            _fail('invalid_argument', 'Invalid FTP password or CA file setting')
        identity = {'transport': self.protocol, 'host': self.host.lower(), 'port': self.port,
                    'username': self.username, 'root': self.root}
        self.connection_id = hashlib.sha256(json.dumps(
            identity, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        self._slots = threading.BoundedSemaphore(2)
        self._mutation_slot = threading.BoundedSemaphore(1)

    def _password(self):
        flags = os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC
        descriptor = os.open(self.password_file, flags)
        try:
            attrs = os.fstat(descriptor)
            if (not stat.S_ISREG(attrs.st_mode) or attrs.st_uid != os.geteuid()
                    or attrs.st_mode & 0o077 or not 0 < attrs.st_size <= 4096):
                _fail('invalid_argument', 'Password file must be private, owned, regular and bounded')
            with os.fdopen(descriptor, 'rb', closefd=False) as stream:
                data = stream.read(4097)
            if len(data) > 4096:
                _fail('limit_exceeded', 'Password file exceeds its limit')
            if data.endswith(b'\r\n'):
                data = data[:-2]
            elif data.endswith(b'\n'):
                data = data[:-1]
            try:
                password = data.decode('utf-8')
            except UnicodeError:
                password = None
            del data
            if password is None:
                _fail('invalid_argument', 'Password file must contain a UTF-8 line')
            if not _safe(password, 4096):
                del password
                _fail('invalid_argument', 'Password file must contain one nonempty line without controls')
            return password
        finally:
            os.close(descriptor)

    def _connect(self, operation):
        if self.protocol == 'ftps':
            context = ssl.create_default_context(cafile=self.ca_file)
            client = _Ftps(context=context, timeout=operation.remaining(), encoding='utf-8')
        else:
            client = _Ftp(timeout=operation.remaining(), encoding='utf-8')
        with operation.lock:
            operation._check_locked()
            operation.client = client
        password = self._password()
        try:
            client.connect(self.host, self.port, timeout=operation.remaining())
            self._tick(client, operation)
            # FTP_TLS.login authenticates TLS before USER/PASS; failures never
            # retry through an unencrypted connection.
            client.login(self.username, password)
        finally:
            del password
        if self.protocol == 'ftps':
            self._tick(client, operation)
            client.prot_p()
        self._tick(client, operation)
        return client

    @staticmethod
    def _error(error, promotion):
        if promotion:
            return TransportError('outcome_unknown',
                                  'Remote mutation may have completed; settlement is unknown',
                                  outcome_unknown=True, settled=False)
        if isinstance(error, TransportError):
            return error
        if isinstance(error, ftplib.error_perm) and str(error)[:3] in ('500', '501', '502', '504'):
            return TransportError('unsupported', 'Server does not support the required FTP operation')
        if isinstance(error, OSError) and error.errno == errno.ENOENT:
            return TransportError('not_found', 'Configured credential or trust file was not found')
        return TransportError('unavailable', 'FTP operation failed; verify connection, credentials and trust settings')

    def _run(self, action, cancel, *, mutation=False):
        return super()._run(action, cancel, timeout=OPERATION_TIMEOUT, mutation=mutation)

    @staticmethod
    def _tick(client, operation):
        client.timeout = operation.remaining()
        if client.sock is not None:
            client.sock.settimeout(client.timeout)

    def _contained(self, path):
        if (not _safe(path, MAX_PATH) or not path.startswith('/') or path.startswith('//')
                or posixpath.normpath(path) != path
                or (self.root != '/' and path != self.root and not path.startswith(self.root + '/'))):
            _fail('invalid_argument', 'Remote path is outside the configured root')

    def _path(self, client, operation, path, *, directory=False):
        if not _safe(path, MAX_PATH) or '..' in path.split('/') or path.startswith('//'):
            _fail('invalid_argument', 'Invalid remote path')
        requested = posixpath.normpath(posixpath.join(self.root, path))
        self._contained(requested)
        self._tick(client, operation)
        client.cwd(self.root)
        if client.pwd() != self.root:
            _fail('conflict', 'Configured FTP root must use its canonical server path')
        self._tick(client, operation)
        client.cwd(requested if directory else posixpath.dirname(requested))
        parent = client.pwd()
        self._contained(parent)
        canonical = parent if directory else posixpath.join(parent, posixpath.basename(requested))
        self._contained(canonical)
        return canonical

    @staticmethod
    def _facts(line):
        facts, separator, name = line.partition(' ')
        if not separator or not facts.endswith(';') or not name:
            _fail('unsupported', 'Server returned invalid machine-readable FTP metadata')
        result = {}
        for fact in facts[:-1].split(';'):
            key, equals, value = fact.partition('=')
            if not equals or not key or key.lower() in result:
                _fail('unsupported', 'Server returned invalid machine-readable FTP metadata')
            result[key.lower()] = value
        return name, result

    def _metadata(self, client, operation, path):
        self._tick(client, operation)
        response = client.sendcmd('MLST ' + path)
        records = [line[1:] for line in response.split('\n')[1:-1] if line.startswith(' ')]
        if len(records) != 1:
            _fail('unsupported', 'Server must provide one MLST resource record')
        name, facts = self._facts(records[0])
        if name not in (path, posixpath.basename(path)):
            _fail('conflict', 'FTP metadata names a different resource')
        return facts

    def _stat(self, client, operation, path):
        facts = self._metadata(client, operation, path)
        if facts.get('type', '').lower() != 'file':
            _fail('unsupported', 'Remote resource is not a regular file')
        raw_size = facts.get('size', '')
        if not raw_size.isascii() or not raw_size.isdigit() or len(raw_size) > 20:
            _fail('unsupported', 'Server must provide a bounded regular-file size')
        size = int(raw_size)
        if size > MAX_BYTES:
            _fail('limit_exceeded', 'Remote file exceeds 8 MiB')
        return size, facts.get('modify'), facts.get('unique')

    def _read(self, client, operation, path, sink=None):
        before = self._stat(client, operation, path)
        content = bytearray()
        count = 0

        def received(data):
            nonlocal count
            operation.check()
            count += len(data)
            if count > MAX_BYTES:
                _fail('limit_exceeded', 'Remote file exceeds 8 MiB')
            if sink is None:
                content.extend(data)
            else:
                sink(data)

        self._tick(client, operation)
        client.retrbinary('RETR ' + path, received, blocksize=BLOCK_BYTES)
        after = self._stat(client, operation, path)
        if before != after or count != before[0]:
            _fail('conflict', 'Remote file changed while reading')
        return bytes(content) if sink is None else None

    def canonical(self, path, cancel=None):
        return self._run(lambda client, op: self._path(client, op, path), cancel)

    def observe(self, path, cancel=None):
        def action(client, operation):
            canonical = self._path(client, operation, path)
            return canonical, self._read(client, operation, canonical)
        return self._run(action, cancel)

    def read(self, path, cancel=None):
        return self.observe(path, cancel)[1]

    def _listing(self, client, operation, canonical):
        entries, metadata, count = [], 0, 0

        def received(line):
            nonlocal metadata, count
            operation.check()
            count += 1
            metadata += len(line.encode('utf-8')) + 1
            if count > MAX_ENTRIES + 2 or metadata > MAX_METADATA:
                _fail('limit_exceeded', 'Remote directory exceeds listing limits')
            name, facts = self._facts(line)
            kind = facts.get('type', '').lower()
            if kind in ('cdir', 'pdir'):
                return
            if (not _safe(name, MAX_PATH) or '/' in name or name in ('.', '..')):
                _fail('invalid_argument', 'Server returned an invalid directory entry')
            if len(entries) >= MAX_ENTRIES:
                _fail('limit_exceeded', 'Remote directory exceeds 1024 entries')
            kind = ('directory' if kind == 'dir' else 'file' if kind == 'file'
                    else 'symlink' if 'slink' in kind else 'other')
            size = facts.get('size', '0')
            if not size.isascii() or not size.isdigit() or len(size) > 20:
                _fail('unsupported', 'Server returned an invalid directory entry size')
            entries.append({'name': name, 'kind': kind, 'size': int(size)})

        self._tick(client, operation)
        # ftplib.mlsd stores the complete listing before yielding records.
        # retrlines invokes a bounded callback as each MLSD line arrives.
        client.retrlines('MLSD ' + canonical, received)
        return sorted(entries, key=lambda entry: entry['name'])

    def browse(self, path, cancel=None):
        def action(client, operation):
            canonical = self._path(client, operation, path, directory=True)
            return canonical, self._listing(client, operation, canonical)
        return self._run(action, cancel)

    def list(self, path, cancel=None):
        return self.browse(path, cancel)[1]

    def _operation_path(self, client, operation, path, *, new):
        # PWD resolves the containing directory. MLST is the server's type
        # authority; FTP cannot identify links a server reports as normal files.
        return self._path(client, operation, path)

    def _operation_absent(self, client, operation, path):
        # A 550 response alone could mean permission denied rather than absence.
        # Require a successful bounded parent listing to establish absence.
        entries = self._listing(client, operation, posixpath.dirname(path))
        if any(entry['name'] == posixpath.basename(path) for entry in entries):
            _fail('conflict', 'Remote destination already exists')

    def _operation_state(self, client, operation, path):
        facts = self._metadata(client, operation, path)
        kind = facts.get('type', '').lower()
        if kind == 'file':
            digest = hashlib.sha256(self._read(client, operation, path)).hexdigest()
        elif kind == 'dir':
            kind, digest = 'directory', None
            if self._listing(client, operation, path):
                _fail('unsupported', 'Remote directory operations require an empty directory')
        else:
            _fail('unsupported', 'Remote operations require a regular file or empty directory')
        return kind, (facts.get('size'), facts.get('modify'), facts.get('unique'), digest)

    @staticmethod
    def _operation_apply(client, prepared):
        if prepared.kind == 'mkdir':
            client.mkd(prepared.source)
        elif prepared.kind == 'rename':
            # FTP has no portable no-replace RNTO. The prepared warning names
            # the concurrent-destination overwrite race explicitly.
            client.rename(prepared.source, prepared.destination)
        elif prepared.entry_kind == 'directory':
            client.rmd(prepared.source)
        else:
            client.delete(prepared.source)

    @staticmethod
    def _cleanup(client, operation, directory, staged):
        try:
            remaining = operation.deadline - time.monotonic()
            if remaining <= 0 or client.sock is None:
                return
            client.timeout = min(0.25, remaining)
            client.sock.settimeout(client.timeout)
            try:
                client.delete(staged)
            except ftplib.error_perm:
                pass
            client.rmd(directory)
        except Exception:
            pass

    def _upload_state(self, client, operation, path):
        canonical = self._operation_path(client, operation, path, new=True)
        if canonical == self.root:
            _fail('invalid_argument', 'The configured remote root cannot be replaced')
        try:
            facts = self._metadata(client, operation, canonical)
        except ftplib.error_perm as error:
            if str(error)[:3] != '550':
                raise
            # 550 is ambiguous (absence, permissions or another failure).
            # Only a complete, successful bounded listing can prove absence.
            entries = self._listing(client, operation, posixpath.dirname(canonical))
            if any(entry['name'] == posixpath.basename(canonical) for entry in entries):
                _fail('unavailable', 'Existing FTP destination metadata is unavailable')
            return canonical, None
        if facts.get('type', '').lower() != 'file':
            _fail('unsupported', 'Upload destination must be an ordinary file or absent')
        digest = hashlib.sha256()
        self._read(client, operation, canonical, sink=digest.update)
        return canonical, digest.hexdigest()

    def _upload(self, client, operation, canonical, data, expected_version, progress=None):
        self._check_upload_target(client, operation, canonical, expected_version)
        directory = posixpath.join(posixpath.dirname(canonical), '.runyte-upload-' + uuid.uuid4().hex)
        staged = posixpath.join(directory, 'document')
        created, sent = False, 0

        def uploaded(block):
            nonlocal sent
            operation.check()
            sent += len(block)
            self._upload_progress(progress, sent, len(data))

        try:
            self._tick(client, operation)
            client.mkd(directory)
            created = True
            self._tick(client, operation)
            client.storbinary('STOR ' + staged, io.BytesIO(data), blocksize=BLOCK_BYTES,
                              callback=uploaded)
            self._check_upload_target(client, operation, canonical, expected_version)
            self._tick(client, operation)
            operation.promote()
            client.rename(staged, canonical)
            if progress is not None:
                progress(100)
        finally:
            if created:
                self._cleanup(client, operation, directory, staged)

    def replace(self, path, data, expected_version, cancel=None):
        if (not isinstance(data, bytes) or len(data) > MAX_BYTES
                or not isinstance(expected_version, str) or len(expected_version) != 64
                or any(c not in '0123456789abcdef' for c in expected_version)):
            _fail('invalid_argument', 'Invalid bounded remote replacement')

        def action(client, operation):
            canonical = self._path(client, operation, path)
            self._upload(client, operation, canonical, data, expected_version)
            return hashlib.sha256(data).hexdigest()
        return self._run(action, cancel, mutation=True)
