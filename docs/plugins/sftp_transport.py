# SPDX-License-Identifier: MPL-2.0
"""Bounded SFTP operations with strict host authentication and honest outcomes.

Path containment assumes a trusted server namespace: SFTP has no descriptor-
relative open/rename. Use a server-side chroot for a security boundary. POSIX
rename is atomic replacement, not compare-and-swap or a durability guarantee.
"""
import errno
import hashlib
import json
import os
import posixpath
import stat
import threading
import time
import unicodedata
import uuid

import paramiko

from transport import BoundedTransport, TransportError

MAX_BYTES = 8 * 1024 * 1024
MAX_ENTRIES = 1024
MAX_METADATA = 4 * 1024 * 1024
MAX_PATH = 3800
BLOCK_BYTES = 128 * 1024
OPERATION_TIMEOUT = 8.0


def _fail(code, message):
    raise TransportError(code, message)


def _safe(value, maximum):
    try:
        return (isinstance(value, str) and 0 < len(value.encode('utf-8')) <= maximum
                and not any(unicodedata.category(c) == 'Cc' for c in value))
    except UnicodeError:
        return False


class SftpTransport(BoundedTransport):
    def __init__(self, config):
        allowed = {'alias', 'host', 'port', 'username', 'root', 'known_hosts',
                   'identity_files', 'allow_agent'}
        if not isinstance(config, dict) or set(config) - allowed:
            _fail('invalid_argument', 'Invalid SFTP connection configuration')
        for key, maximum in [('alias', 160), ('host', 253), ('username', 128),
                             ('root', MAX_PATH), ('known_hosts', 4096)]:
            if not _safe(config.get(key), maximum):
                _fail('invalid_argument', 'Missing or invalid SFTP connection setting')
        self.label, self.host = config['alias'], config['host']
        self.username, self.root = config['username'], config['root']
        self.port = config.get('port', 22)
        if (type(self.port) is not int or not 1 <= self.port <= 65535
                or any(c.isspace() or c in '/@' for c in self.host)):
            _fail('invalid_argument', 'Invalid SFTP endpoint')
        if (not self.root.startswith('/') or self.root.startswith('//')
                or '..' in self.root.split('/') or posixpath.normpath(self.root) != self.root):
            _fail('invalid_argument', 'SFTP root must be an absolute normalized directory')
        self.known_hosts = config['known_hosts']
        self.identity_files = config.get('identity_files', [])
        self.allow_agent = config.get('allow_agent', False)
        if (type(self.allow_agent) is not bool or not isinstance(self.identity_files, list)
                or len(self.identity_files) > 8
                or any(not _safe(p, 4096) or not os.path.isabs(p) for p in self.identity_files)
                or not os.path.isabs(self.known_hosts)
                or not (self.identity_files or self.allow_agent)):
            _fail('invalid_argument', 'SFTP requires explicit known hosts and keys or an SSH agent')
        self.identity_files = tuple(self.identity_files)
        identity = {'transport': 'sftp', 'host': self.host.lower(), 'port': self.port,
                    'username': self.username, 'root': self.root}
        self.connection_id = hashlib.sha256(json.dumps(
            identity, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        self._slots = threading.BoundedSemaphore(2)
        self._mutation_slot = threading.BoundedSemaphore(1)

    def _connect(self, operation):
        client = paramiko.SSHClient()
        with operation.lock:
            operation._check_locked()
            operation.client = client
        client.load_host_keys(self.known_hosts)
        client.set_missing_host_key_policy(paramiko.RejectPolicy())
        timeout = operation.remaining()
        client.connect(self.host, port=self.port, username=self.username,
                       key_filename=list(self.identity_files), allow_agent=self.allow_agent,
                       look_for_keys=False, timeout=timeout, banner_timeout=timeout,
                       auth_timeout=timeout, channel_timeout=timeout)
        operation.check()
        sftp = client.open_sftp()
        sftp.get_channel().settimeout(operation.remaining())
        return sftp

    @staticmethod
    def _error(error, promotion):
        if promotion:
            return TransportError('outcome_unknown',
                                  'Remote mutation may have completed; settlement is unknown',
                                  outcome_unknown=True, settled=False)
        if isinstance(error, TransportError):
            return error
        if isinstance(error, OSError) and error.errno == errno.ENOENT:
            return TransportError('not_found', 'Remote resource or configured key file was not found')
        # Paramiko errors may contain usernames, paths or authentication data.
        return TransportError('unavailable', 'SFTP operation failed; verify connection and trust settings')

    def _run(self, action, cancel, *, mutation=False):
        return super()._run(action, cancel, timeout=OPERATION_TIMEOUT, mutation=mutation)

    def _path(self, sftp, operation, path):
        if (not _safe(path, MAX_PATH) or '..' in path.split('/')
                or path.startswith('//')):
            _fail('invalid_argument', 'Invalid remote path')
        requested = posixpath.normpath(posixpath.join(self.root, path))
        self._contained(requested)
        operation.check()
        if sftp.normalize(self.root) != self.root:
            _fail('conflict', 'Configured SFTP root must use its canonical server path')
        operation.check()
        canonical = sftp.normalize(requested)
        self._contained(canonical)
        return canonical

    def _contained(self, path):
        if (not _safe(path, MAX_PATH) or not path.startswith('/')
                or path.startswith('//') or posixpath.normpath(path) != path
                or (self.root != '/' and path != self.root and not path.startswith(self.root + '/'))):
            _fail('invalid_argument', 'Remote path is outside the configured root')

    @staticmethod
    def _read(sftp, operation, path, sink=None):
        operation.check()
        attrs = sftp.stat(path)
        if attrs.st_mode is None or not stat.S_ISREG(attrs.st_mode):
            _fail('unsupported', 'Remote resource is not a regular file')
        if attrs.st_size is None or not 0 <= attrs.st_size <= MAX_BYTES:
            _fail('limit_exceeded', 'Remote file exceeds 8 MiB')
        content = bytearray()
        count = 0
        with sftp.open(path, 'rb', bufsize=0) as stream:
            operation.check()
            opened = stream.stat()
            if opened.st_mode is None or not stat.S_ISREG(opened.st_mode):
                _fail('conflict', 'Remote file changed while opening')
            if opened.st_size is None or not 0 <= opened.st_size <= MAX_BYTES:
                _fail('limit_exceeded', 'Remote file exceeds 8 MiB')
            while True:
                operation.check()
                chunk = stream.read(min(BLOCK_BYTES, MAX_BYTES + 1 - count))
                if not chunk:
                    break
                count += len(chunk)
                if count > MAX_BYTES:
                    _fail('limit_exceeded', 'Remote file exceeds 8 MiB')
                if sink is None:
                    content.extend(chunk)
                else:
                    sink(chunk)
            operation.check()
            final = stream.stat()
            if (count != opened.st_size or final.st_size != opened.st_size
                    or final.st_mtime != opened.st_mtime):
                _fail('conflict', 'Remote file changed while reading')
        return bytes(content) if sink is None else None

    def canonical(self, path, cancel=None):
        return self._run(lambda sftp, op: self._path(sftp, op, path), cancel)

    def observe(self, path, cancel=None):
        def action(sftp, operation):
            canonical = self._path(sftp, operation, path)
            return canonical, self._read(sftp, operation, canonical)
        return self._run(action, cancel)

    def read(self, path, cancel=None):
        return self.observe(path, cancel)[1]

    def browse(self, path, cancel=None):
        def action(sftp, operation):
            canonical = self._path(sftp, operation, path)
            entries, metadata = [], 0
            for entry in sftp.listdir_iter(canonical, read_aheads=1):
                operation.check()
                name = entry.filename
                if not _safe(name, MAX_PATH) or '/' in name or name in ('.', '..'):
                    _fail('invalid_argument', 'Server returned an invalid directory entry')
                metadata += len(name.encode('utf-8')) + 64
                if len(entries) >= MAX_ENTRIES or metadata > MAX_METADATA:
                    _fail('limit_exceeded', 'Remote directory exceeds listing limits')
                mode = entry.st_mode or 0
                kind = ('file' if stat.S_ISREG(mode) else 'directory' if stat.S_ISDIR(mode)
                        else 'symlink' if stat.S_ISLNK(mode) else 'other')
                entries.append({'name': name, 'kind': kind, 'size': entry.st_size or 0})
            return canonical, sorted(entries, key=lambda entry: entry['name'])
        return self._run(action, cancel)

    def list(self, path, cancel=None):
        return self.browse(path, cancel)[1]

    def _operation_path(self, sftp, operation, path, *, new):
        if not _safe(path, MAX_PATH) or '..' in path.split('/') or path.startswith('//'):
            _fail('invalid_argument', 'Invalid remote operation path')
        requested = posixpath.normpath(posixpath.join(self.root, path))
        self._contained(requested)
        parent = self._path(sftp, operation, posixpath.dirname(requested))
        operation.check()
        if not stat.S_ISDIR(sftp.stat(parent).st_mode or 0):
            _fail('unsupported', 'Remote operation parent is not a directory')
        canonical = posixpath.join(parent, posixpath.basename(requested))
        self._contained(canonical)
        if not new:
            operation.check()
            if stat.S_ISLNK(sftp.lstat(canonical).st_mode or 0):
                _fail('unsupported', 'Remote operations do not follow source symbolic links')
            if self._path(sftp, operation, canonical) != canonical:
                _fail('conflict', 'Remote source identity changed while resolving it')
        return canonical

    @staticmethod
    def _operation_absent(sftp, operation, path):
        operation.check()
        try:
            sftp.lstat(path)
        except OSError as error:
            if error.errno == errno.ENOENT:
                return
            raise
        _fail('conflict', 'Remote destination already exists')

    def _operation_state(self, sftp, operation, path):
        operation.check()
        attrs = sftp.lstat(path)
        mode = attrs.st_mode or 0
        if stat.S_ISREG(mode):
            kind = 'file'
            digest = hashlib.sha256(self._read(sftp, operation, path)).hexdigest()
        elif stat.S_ISDIR(mode):
            kind, digest = 'directory', None
            for _ in sftp.listdir_iter(path, read_aheads=1):
                operation.check()
                _fail('unsupported', 'Remote directory operations require an empty directory')
        else:
            _fail('unsupported', 'Remote operations require a regular file or empty directory')
        return kind, (mode, attrs.st_size, attrs.st_mtime, attrs.st_uid, attrs.st_gid, digest)

    @staticmethod
    def _operation_apply(sftp, prepared):
        if prepared.kind == 'mkdir':
            sftp.mkdir(prepared.source)
        elif prepared.kind == 'rename':
            # The normal SFTP request requires an absent destination. Never use
            # the overwrite extension for an ordinary namespace rename.
            sftp.rename(prepared.source, prepared.destination)
        elif prepared.entry_kind == 'directory':
            sftp.rmdir(prepared.source)
        else:
            sftp.remove(prepared.source)

    @staticmethod
    def _cleanup(sftp, operation, paths):
        for path in paths:
            try:
                remaining = operation.deadline - time.monotonic()
                if remaining <= 0:
                    return
                sftp.get_channel().settimeout(min(0.25, remaining))
                sftp.remove(path)
            except Exception:
                pass

    def _probe_replace(self, sftp, operation, directory):
        """Exercise the extension using only owned empty files, never the target.

        A failed probe cannot change document contents. Its rename deliberately
        does not enter the document promotion phase; even a lost probe reply is
        a definite refusal of the document write.
        """
        paths = [posixpath.join(directory, '.runyte-probe-' + uuid.uuid4().hex)
                 for _ in range(2)]
        created = []
        try:
            for path in paths:
                operation.check()
                with sftp.open(path, 'wx', bufsize=0):
                    created.append(path)
                    sftp.chmod(path, 0o600)
            operation.check()
            try:
                sftp.posix_rename(*paths)
            except OSError as error:
                unsupported = (error.errno in (errno.ENOSYS, errno.EOPNOTSUPP)
                               or str(error) == 'Operation unsupported')
                _fail('unsupported' if unsupported else 'unavailable',
                      'Server did not complete the atomic replacement capability probe')
        finally:
            self._cleanup(sftp, operation, created)
        operation.check()
        # Cleanup uses a short timeout; subsequent document IO receives the
        # remaining operation budget again.
        sftp.get_channel().settimeout(operation.remaining())

    def replace(self, path, data, expected_version, cancel=None):
        if (not isinstance(data, bytes) or len(data) > MAX_BYTES
                or not isinstance(expected_version, str) or len(expected_version) != 64
                or any(c not in '0123456789abcdef' for c in expected_version)):
            _fail('invalid_argument', 'Invalid bounded remote replacement')

        def action(sftp, operation):
            canonical = self._path(sftp, operation, path)
            operation.check()
            attrs = sftp.stat(canonical)
            if attrs.st_mode is None or not stat.S_ISREG(attrs.st_mode):
                _fail('unsupported', 'Remote resource is not a regular file')
            self._probe_replace(sftp, operation, posixpath.dirname(canonical))
            temporary = posixpath.join(posixpath.dirname(canonical), '.runyte-upload-' + uuid.uuid4().hex)
            created = False
            try:
                operation.check()
                with sftp.open(temporary, 'wx', bufsize=0) as stream:
                    created = True
                    # The initial empty exclusive file may inherit the server
                    # umask; make it private before sending document contents.
                    sftp.chmod(temporary, 0o600)
                    for offset in range(0, len(data), BLOCK_BYTES):
                        operation.check()
                        stream.write(data[offset:offset + BLOCK_BYTES])
                operation.check()
                sftp.chmod(temporary, stat.S_IMODE(attrs.st_mode) & 0o777)
                if self._path(sftp, operation, path) != canonical:
                    _fail('conflict', 'Remote path changed while preparing replacement')
                current = self._read(sftp, operation, canonical)
                if hashlib.sha256(current).hexdigest() != expected_version:
                    _fail('conflict', 'Remote file changed; inspect it before saving')
                del current
                operation.promote()
                sftp.posix_rename(temporary, canonical)
                return hashlib.sha256(data).hexdigest()
            finally:
                # No retry, destination unlink, or claim that closing a failed
                # rename settles it. A disconnected upload may leave its temp.
                if created and not operation.promotion_started:
                    self._cleanup(sftp, operation, [temporary])
        return self._run(action, cancel, mutation=True)
