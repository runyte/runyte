# SPDX-License-Identifier: MPL-2.0
"""Isolated real-host SQLite trial; no editor input or personal runtime state."""
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time

from mcp_client import RealMCPClient


def private_json(path, value):
    with open(path, 'x', opener=lambda name, flags: os.open(name, flags, 0o600)) as target:
        json.dump(value, target)


class DurableFixture:
    def __init__(self, binary, root=None):
        self.binary = Path(binary).resolve(strict=True)
        self.temporary = tempfile.TemporaryDirectory(prefix='ry-sql-', dir='/tmp') if root is None else None
        self.root = Path(self.temporary.name if root is None else root).resolve()
        self.root.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.env = dict(os.environ)
        for key in ('RUNYTE_PARENT_CONTEXT', 'RUNYTE_BENCH_EVENTS', 'RUNYTE_INPUT_TRACE'):
            self.env.pop(key, None)
        for key, name in [('HOME', 'home'), ('XDG_CONFIG_HOME', 'config'), ('XDG_CACHE_HOME', 'cache'),
                          ('XDG_RUNTIME_DIR', 'runtime'), ('XDG_STATE_HOME', 'state'), ('XDG_DATA_HOME', 'data'),
                          ('RUNYTE_ALL_HOSTS_DIR', 'hosts'), ('RUNYTE_CONTEXT_HOME', 'ctx')]:
            path = self.root / name
            path.mkdir(mode=0o700)
            self.env[key] = str(path)
        self.store = self.root / 'sqlite'
        self.env['RUNYTE_CHANNEL_HOME'] = str(self.store)
        self.project = self.root / 'project'
        self.project.mkdir()
        subprocess.run([str(self.binary), '--init', str(self.project)], env=self.env,
                       cwd=self.project, check=True, capture_output=True)
        self.config = self.root / 'config' / 'config.yaml'
        self.config.write_text('lsp:\n  enable: false\n')
        self.profiles = self.root / 'profiles'
        self.profiles.mkdir(mode=0o700)
        self.host = None
        self.clients = []
        self.log = open(self.root / 'host.log', 'wb')
        self.add_identity('trial')

    def add_identity(self, name, permissions=('read',)):
        secret = secrets.token_hex(32)
        identity = hashlib.sha256(secret.encode()).hexdigest()
        private_json(self.root / 'ctx' / ('identity-' + name + '.json'), {'name': name, 'credential': secret})
        path = os.fsencode(self.project.resolve())
        key = hashlib.sha256(path + b'\0' + identity.encode()).hexdigest()
        private_json(self.root / 'ctx' / ('grant-' + key + '.json'), {'root': list(path), 'identity': identity, 'permissions': list(permissions)})

    def profile(self, name):
        path = self.profiles / (name + '.json')
        if not path.exists():
            private_json(path, {'workspace': hashlib.sha256(os.fsencode(self.project.resolve())).hexdigest()[:32],
                                'store': hashlib.sha256(str(self.store).encode()).hexdigest(), 'secret': secrets.token_hex(32)})
        return path

    def client(self, name, identity='trial'):
        client = RealMCPClient(self.binary, self.project, self.env, identity, channel_profile=self.profile(name))
        self.clients.append(client)
        return client

    def start(self):
        self.host = subprocess.Popen([str(self.binary), '--serve', '--project-root', str(self.project), '--config', str(self.config)],
                                     env=self.env, cwd=self.project, stdin=subprocess.DEVNULL, stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + 15
        probe = RealMCPClient(self.binary, self.project, self.env, 'trial')
        try:
            while time.monotonic() < deadline:
                assert self.host.poll() is None, 'fixture host exited'
                if probe.data('list_workspaces')['workspaces']:
                    return
                time.sleep(.05)
            raise AssertionError('host readiness deadline exceeded')
        finally:
            probe.close()

    def stop(self):
        if self.host is not None:
            self.host.terminate()
            try:
                self.host.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.host.kill()
                self.host.wait(timeout=5)
            self.host = None

    def close(self):
        for client in self.clients:
            client.close()
        self.stop()
        self.log.close()
        if self.temporary:
            self.temporary.cleanup()


def call(client, action, **args):
    return client.data('durable_channel', action=action, **args)


def post(client, channel, text, directory):
    args = dict(channel=channel, text=text, nonce=secrets.token_hex(32), issued_ms=time.time_ns() // 1_000_000,
                directory=str(directory))
    return call(client, 'post', **args), args
