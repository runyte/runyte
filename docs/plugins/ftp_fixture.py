# SPDX-License-Identifier: MPL-2.0
"""Real loopback FTP/FTPS fixture with temporary credentials and TLS trust."""
import datetime
import ipaddress
import logging
import os
from pathlib import Path
import tempfile
import threading

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID
from pyftpdlib.authorizers import DummyAuthorizer
from pyftpdlib.handlers import FTPHandler, TLS_FTPHandler
from pyftpdlib.ioloop import IOLoop
from pyftpdlib.servers import FTPServer


def certificate_files(base, *, valid_hostname=True):
    """Generate a private fixture CA and its localhost leaf, never executable."""
    now = datetime.datetime.now(datetime.timezone.utc)
    ca_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    ca_name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'Runyte fixture CA')])
    ca = (x509.CertificateBuilder().subject_name(ca_name).issuer_name(ca_name)
          .public_key(ca_key.public_key()).serial_number(x509.random_serial_number())
          .not_valid_before(now - datetime.timedelta(days=1))
          .not_valid_after(now + datetime.timedelta(days=2))
          .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
          .add_extension(x509.SubjectKeyIdentifier.from_public_key(ca_key.public_key()), critical=False)
          .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(ca_key.public_key()), critical=False)
          .add_extension(x509.KeyUsage(True, False, False, False, False, True, True, False, False), critical=True)
          .sign(ca_key, hashes.SHA256()))
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'localhost')])
    names = [x509.DNSName('localhost'), x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]
    if not valid_hostname:
        names = [x509.DNSName('wrong.invalid')]
    leaf = (x509.CertificateBuilder().subject_name(name).issuer_name(ca_name)
            .public_key(key.public_key()).serial_number(x509.random_serial_number())
            .not_valid_before(now - datetime.timedelta(days=1))
            .not_valid_after(now + datetime.timedelta(days=2))
            .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
            .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(ca_key.public_key()), critical=False)
            .add_extension(x509.KeyUsage(True, False, True, False, False, False, False, False, False), critical=True)
            .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
            .add_extension(x509.SubjectAlternativeName(names), critical=False)
            .sign(ca_key, hashes.SHA256()))
    ca_path, cert_path, key_path = base / 'ca.pem', base / 'server.pem', base / 'server-key.pem'
    ca_path.write_bytes(ca.public_bytes(serialization.Encoding.PEM))
    cert_path.write_bytes(leaf.public_bytes(serialization.Encoding.PEM))
    key_path.write_bytes(key.private_bytes(serialization.Encoding.PEM,
                         serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
    key_path.chmod(0o600)
    return ca_path, cert_path, key_path


class FtpFixture:
    def __init__(self, *, tls=True, valid_hostname=True, reject_protection=False,
                 reject_rename=False):
        self.temporary = tempfile.TemporaryDirectory(prefix='runyte-ftp-fixture-')
        self.base = Path(self.temporary.name).resolve()
        self.root = self.base / 'remote'
        self.root.mkdir()
        self.password = 'temporary-fixture-password'
        self.password_file = self.base / 'password'
        self.password_file.write_text(self.password + '\n')
        self.password_file.chmod(0o600)
        self.ca_file, cert_file, key_file = certificate_files(self.base, valid_hostname=valid_hostname)
        self.operations = []
        self.on_write_close = self.before_rename = None
        self.drop_rename_reply = False
        self.rename_entered, self.rename_done = threading.Event(), threading.Event()
        self.rename_release = threading.Event()
        self.rename_release.set()
        self.read_started = threading.Event()
        self.two_reads_started = threading.Event()
        self.read_count = 0
        self.read_release = threading.Event()
        self.read_release.set()
        self._closed = threading.Event()
        fixture = self
        authorizer = DummyAuthorizer()
        authorizer.add_user('fixture', self.password, str(self.root), perm='elradfmwMT')

        class Handler(TLS_FTPHandler if tls else FTPHandler):
            auth_failed_timeout = 0
            def ftp_PROT(self, line):
                fixture.record('PROT', line)
                if reject_protection:
                    self.respond('534 Private data protection refused')
                else:
                    super().ftp_PROT(line)

            def ftp_RETR(self, path):
                fixture.record('RETR', path)
                fixture.read_count += 1
                if fixture.read_count >= 2:
                    fixture.two_reads_started.set()
                fixture.read_started.set()
                if not fixture.read_release.is_set():
                    def resume():
                        if self._closed:
                            return
                        if fixture.read_release.is_set():
                            super(Handler, self).ftp_RETR(path)
                        else:
                            self.ioloop.call_later(0.01, resume)
                    self.ioloop.call_later(0.01, resume)
                    return
                return super().ftp_RETR(path)

            def ftp_MLSD(self, path):
                fixture.record('MLSD', path)
                return super().ftp_MLSD(path)

            def ftp_DELE(self, path):
                fixture.record('DELE', path)
                return super().ftp_DELE(path)

            def ftp_RNFR(self, path):
                fixture.record('RNFR', path)
                return super().ftp_RNFR(path)

            def ftp_RNTO(self, path):
                fixture.record('RNTO', path)
                source, self._rnfr = self._rnfr, None
                if source is None:
                    self.respond('503 RNFR required')
                    return
                fixture.rename_entered.set()
                if reject_rename:
                    self.respond('550 Rename refused')
                    return
                if fixture.before_rename:
                    fixture.before_rename(Path(source), Path(path))
                os.replace(source, path)
                fixture.rename_done.set()
                if fixture.drop_rename_reply:
                    self.close()
                elif fixture.rename_release.is_set():
                    self.respond('250 Renaming ok')
                else:
                    def reply():
                        if self._closed:
                            return
                        if fixture.rename_release.is_set():
                            self.respond('250 Renaming ok')
                        else:
                            self.ioloop.call_later(0.01, reply)
                    self.ioloop.call_later(0.01, reply)

            def on_file_received(self, path):
                fixture.record('STOR', path)
                if fixture.on_write_close:
                    fixture.on_write_close(Path(path))

        Handler.authorizer = authorizer
        if tls:
            Handler.certfile, Handler.keyfile = str(cert_file), str(key_file)
            Handler.tls_control_required = Handler.tls_data_required = True
        logging.getLogger('pyftpdlib').setLevel(logging.CRITICAL)
        self.ioloop = IOLoop()
        self.server = FTPServer(('127.0.0.1', 0), Handler, ioloop=self.ioloop)
        logging.getLogger('pyftpdlib').setLevel(logging.CRITICAL)
        self.server.max_cons, self.server.max_cons_per_ip = 16, 16
        self.port = self.server.socket.getsockname()[1]
        self.config = {'transport': 'ftps' if tls else 'ftp', 'alias': 'Fixture',
                       'host': '127.0.0.1', 'port': self.port, 'username': 'fixture',
                       'root': '/', 'password_file': str(self.password_file)}
        if tls:
            self.config['ca_file'] = str(self.ca_file)
        self.thread = threading.Thread(target=self._serve, daemon=True)
        self.thread.start()

    def record(self, *operation):
        if len(self.operations) < 4096:
            self.operations.append(operation)

    def _serve(self):
        try:
            while not self._closed.is_set():
                self.ioloop.loop(timeout=0.01, blocking=False)
        finally:
            self.server.close_all()
            self.ioloop.close()

    def close(self):
        self._closed.set()
        self.read_release.set()
        self.rename_release.set()
        self.thread.join(timeout=3)
        if self.thread.is_alive():
            raise RuntimeError('FTP fixture did not stop')
        self.temporary.cleanup()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
