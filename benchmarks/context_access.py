#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Serial context-access startup, idle, and native-input measurements on Linux.

Build the release binary first. Uses fixture-owned grants and two bounded readers;
never changes a person's permissions. Every complete sample is retained.
"""
import argparse
from collections import deque
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import secrets
import select
import shlex
import statistics
import sys
import tempfile
import threading
import time

import fixtures
import plugins
import ptybench

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / 'docs' / 'plugins'))
from context_client import ContextClient

SCOPES = ['terminal_read', 'editor_context_read']
CASES = ('disabled', 'enabled_zero_readers', 'noisy_terminal_two_readers')
# Keep the physical command prompt visible; printf generates the wide payload.
NOISE = 'while [ ! -e s ];do printf "noise-%0512d\\n" 0;sleep .02;done'


def private_json(path, value):
    with open(path, 'x', opener=lambda name, flags: os.open(name, flags, 0o600)) as file:
        json.dump(value, file, separators=(',', ':'))


def seed_grant(storage, project):
    storage.mkdir(mode=0o700)
    credential = secrets.token_hex(32)
    fingerprint = hashlib.sha256(credential.encode()).hexdigest()
    root = os.fsencode(project.resolve())
    key = hashlib.sha256(root + b'\0' + fingerprint.encode()).hexdigest()
    private_json(storage / 'identity.json', {'name': 'agent', 'credential': credential})
    private_json(storage / ('grant-' + key + '.json'), {
        'root': list(root), 'identity': fingerprint, 'scopes': SCOPES})
    return credential


class Session(plugins.Session):
    def __init__(self, binary, root, enabled):
        self.binary, self.root = binary, root
        self.env = plugins.environment(root)
        self.env.pop('RUNYTE_INPUT_TRACE', None)
        self.storage = root / 'ctx'
        self.env['RUNYTE_CONTEXT_HOME'] = str(self.storage)
        self.project = root / 'project'
        self.project.mkdir()
        # No Git repository is needed: --project-root pins the isolated root.
        self.fixture = fixtures.ensure(self.project, ['short.lua'])['short.lua']
        self.original = self.fixture.read_bytes()
        self.credential = seed_grant(self.storage, self.project) if enabled else None
        self.noise_started = False
        config_dir = Path(self.env['XDG_CONFIG_HOME']) / 'runyte'
        config_dir.mkdir()
        config = config_dir / 'config.yaml'
        config.write_text('lsp:\n  enable: false\nworkspace:\n  session_strip: hidden\n')
        self.enabled = self.persistent = self.quiet = self.uppercase = self.uppercase_proven = False
        self.terminal = plugins.Terminal()
        self.bytes = self.read_chunks = 0
        self.first_byte = None
        self.reaped = self.closed_fd = False
        self.origin_ns = time.monotonic_ns()
        self.origin = time.perf_counter()
        argv = [str(binary), '--standalone', '--project-root', str(self.project),
                '--config', str(config), str(self.fixture)]
        self.pid, self.fd = plugins.spawn(argv, self.env, str(self.project))

    def shutdown(self):
        if self.reaped:
            return
        if self.noise_started:
            children = plugins.OwnedProcesses()
            try:
                # Pin descendants before the sentinel ends the shell, so PID reuse
                # cannot be confused with its successful exit during cleanup.
                children.capture(self.pid, validate=self.validate_host)
                descendants = [fd for pid, fd in children.fds.items() if pid != self.pid]
                (self.project / 's').touch()  # Data only; never an executable fixture.
                self.until(lambda: all(select.select([fd], [], [], 0)[0] for fd in descendants),
                           'Noisy terminal child exit', seconds=5)
                self.drain(.1)  # Apply the PTY exit event before native quit.
            finally:
                children.close()
        # q! returns a covered document to its retained terminal. qa! closes the
        # editor once the child has exited; the terminal's retained rows are safe.
        self.command('qa!')
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.reaped = True
                if status:
                    raise RuntimeError('Context benchmark editor quit failed')
                return
            try:
                self.pump(.01)
            except (EOFError, OSError):
                pass
        raise TimeoutError('Context benchmark editor did not quit after terminal exit')

    def registration(self):
        records = list(self.storage.glob('host-*.json'))
        if len(records) != 1:
            return None
        record = json.loads(records[0].read_text())
        if record['pid'] != self.pid or Path(record['root']) != self.project.resolve():
            raise ValueError('Context endpoint does not belong to benchmark fixture')
        return record


@contextmanager
def session(binary, enabled):
    # Short paths also fit macOS Unix socket limits; measurement CPU needs Linux.
    with tempfile.TemporaryDirectory(prefix='ryctxb-', dir='/tmp') as directory:
        editor = Session(binary, Path(directory).resolve(), enabled)
        try:
            yield editor
        finally:
            editor.close()


class Readers:
    def __init__(self, registration, credential):
        self.stop = threading.Event()
        self.lock = threading.Lock()
        self.counts = [0, 0]
        self.revisions = [set(), set()]
        self.evidence = [deque(maxlen=2048), deque(maxlen=2048)]
        self.errors = []
        self.threads = [threading.Thread(target=self.run, args=(index, registration, credential), daemon=True)
                        for index in range(2)]
        for thread in self.threads:
            thread.start()

    def run(self, index, registration, credential):
        try:
            with ContextClient(registration['endpoint'], credential,
                               expected_incarnation=registration['host_incarnation'],
                               required_scopes=('terminal_read',), optional_scopes=()) as client:
                terminals = client.request('terminal.list', {'offset': 0, 'limit': 10})['terminals']
                if len(terminals) != 1:
                    raise ValueError('Expected one fixture-owned noisy terminal')
                terminal = terminals[0]['terminal']
                while not self.stop.is_set():
                    result = client.request('terminal.read', {'terminal': terminal, 'region': 'tail',
                        'max_rows': 50, 'max_bytes': 32768, 'max_cells': 32768})
                    if 'noise-' not in json.dumps(result):
                        raise ValueError('Context read did not contain noisy terminal evidence')
                    with self.lock:
                        self.counts[index] += 1
                        self.revisions[index].add(result.get('revision'))
                        self.evidence[index].append((time.monotonic_ns(), result.get('revision')))
                    self.stop.wait(.05)
        except Exception as error:
            # No credentials, root paths, terminal contents or unrestricted errors.
            with self.lock:
                self.errors.append(type(error).__name__)

    def snapshot(self):
        with self.lock:
            if self.errors:
                raise RuntimeError('Context reader failed: ' + ', '.join(self.errors))
            return {'reads': list(self.counts), 'distinct_revisions': [len(values) for values in self.revisions]}

    def phase_progress(self, start_ns, end_ns):
        with self.lock:
            return phase_progress(self.evidence, start_ns, end_ns)

    def close(self):
        self.stop.set()
        for thread in self.threads:
            thread.join(3)
        if any(thread.is_alive() for thread in self.threads):
            raise RuntimeError('Context reader exceeded shutdown deadline')
        self.snapshot()


def phase_progress(evidence, start_ns, end_ns):
    result = []
    for points in evidence:
        selected = [(stamp, revision) for stamp, revision in points if start_ns <= stamp <= end_ns]
        if len(selected) < 2 or len({revision for _, revision in selected}) < 2:
            raise ValueError('Each context reader must observe changing output within the measured phase')
        result.append(selected)
    return result


def cpu(pid):
    # Own editor CPU is kept separate from its noisy PTY descendants.
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return {'editor': int(fields[11]) + int(fields[12]), 'tree': ptybench._cpu_ticks(pid),
            'identity': fields[19]}


def sample(binary, case, window, latency_count):
    enabled = case != 'disabled'
    with session(binary, enabled) as editor:
        first_document_ms = editor.document()
        position = editor.terminal.position(plugins.DOCUMENT)
        started = time.perf_counter()
        os.write(editor.fd, b'i ')
        editor.until(lambda: editor.terminal.at(position, ' ' + plugins.DOCUMENT), 'Startup edit acknowledgment')
        ready_ms = (time.perf_counter() - editor.origin) * 1000
        first_input_ms = (time.perf_counter() - started) * 1000
        editor.command('write! ready.lua')
        ready = editor.project / 'ready.lua'
        editor.until(lambda: ready.exists() and ready.read_bytes() == b' ' + editor.original, 'Whole startup edit verification')
        editor.command('open ' + editor.fixture.name)
        editor.until(lambda: editor.terminal.contains(plugins.DOCUMENT), 'Unmodified measured document')
        if enabled:
            editor.until(lambda: editor.registration() is not None, 'Context registration')
        elif editor.storage.exists():
            raise ValueError('Disabled context access created storage')
        readers = None
        try:
            if case == 'noisy_terminal_two_readers':
                editor.noise_started = True
                editor.command('terminal /bin/sh -c ' + shlex.quote(NOISE))
                editor.until(lambda: editor.terminal.contains('noise-' + '0' * 20), 'Noisy terminal output')
                os.write(editor.fd, b'\x1c')  # Leave PTY input before the native open command.
                editor.drain(.1)
                editor.return_document()
                readers = Readers(editor.registration(), editor.credential)
                editor.until(lambda: min(readers.snapshot()['reads']) >= 2, 'Both context readers active')
            editor.drain(2.5)
            before = cpu(editor.pid)
            read_before = readers.snapshot() if readers else None
            byte_before, chunks_before = editor.bytes, editor.read_chunks
            window_start_ns = time.monotonic_ns()
            started = time.perf_counter()
            editor.drain(window)
            elapsed = time.perf_counter() - started
            window_end_ns = time.monotonic_ns()
            after = cpu(editor.pid)
            read_after = readers.snapshot() if readers else None
            if (before['identity'] != after['identity'] or elapsed < window
                    or after['editor'] < before['editor'] or after['tree'] < before['tree']):
                raise ValueError('Incomplete CPU observation')
            hertz = os.sysconf('SC_CLK_TCK')
            idle = {'seconds': elapsed,
                    'editor_cpu_percent': (after['editor'] - before['editor']) / hertz / elapsed * 100,
                    'tree_cpu_percent': (after['tree'] - before['tree']) / hertz / elapsed * 100,
                    'screen_bytes': editor.bytes - byte_before, 'pty_read_chunks': editor.read_chunks - chunks_before,
                    'reader_before': read_before, 'reader_after': read_after,
                    'phase_ns': {'start_ns': window_start_ns, 'end_ns': window_end_ns},
                    'reader_progress': readers.phase_progress(window_start_ns, window_end_ns) if readers else None}
            if readers and any(after <= before for after, before in zip(read_after['reads'], read_before['reads'])):
                raise ValueError('Concurrent reads did not progress during CPU observation')
            read_input_before = readers.snapshot() if readers else None
            latencies, input_phase = plugins.latency_samples(editor, latency_count)
            read_input_after = readers.snapshot() if readers else None
            if readers and (any(after <= before for after, before in zip(read_input_after['reads'], read_input_before['reads']))
                            or min(read_input_after['distinct_revisions']) < 2):
                raise ValueError('Noisy context workload did not progress during native input')
            return {'complete': True, 'first_document_ms': first_document_ms,
                    'demonstrated_startup_ms': ready_ms, 'first_input_ms': first_input_ms,
                    'idle': idle, 'latency_ms': latencies, 'latency_summary': plugins.summary(latencies, latency_count),
                    'input_readers_before': read_input_before, 'input_readers_after': read_input_after,
                    'input_phase_ns': input_phase,
                    'input_reader_progress': readers.phase_progress(input_phase['start_ns'], input_phase['end_ns']) if readers else None}
        finally:
            if readers:
                readers.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=REPO / 'target/release/runyte')
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--window', type=float, default=10)
    parser.add_argument('--latency-count', type=int, default=40)
    parser.add_argument('--json', type=Path, required=True)
    options = parser.parse_args()
    if options.runs < 3 or not 10 <= options.window <= 30 or not 10 <= options.latency_count <= 80:
        parser.error('Require at least 3 runs, 10..30-second windows, and 10..80 latency samples')
    if not Path('/proc').is_dir():
        parser.error('CPU measurements currently require Linux /proc')
    binary = options.binary.resolve(strict=True)
    info = plugins.binary_info(binary)
    info.pop('path')
    result = {'schema': 'runyte.context.benchmark.v1', 'binary': info,
              'runs': options.runs, 'window_seconds': options.window,
              'geometry': {'columns': ptybench.COLUMNS, 'rows': ptybench.ROWS},
              'samples': {case: [] for case in CASES}, 'complete': False}
    def save():
        options.json.write_text(json.dumps(result, indent=2) + '\n')
    try:
        # Rotate case order; don't discard successful slow samples.
        for iteration in range(options.runs):
            for case in CASES[iteration % 3:] + CASES[:iteration % 3]:
                print(f'{case} sample {iteration + 1}/{options.runs}', flush=True)
                result['samples'][case].append(sample(binary, case, options.window, options.latency_count))
                save()
        result['complete'] = True
        result['summary'] = {case: {
            'startup_median_ms': statistics.median(row['demonstrated_startup_ms'] for row in rows),
            'idle_editor_cpu_median_percent': statistics.median(row['idle']['editor_cpu_percent'] for row in rows),
            'screen_bytes': [row['idle']['screen_bytes'] for row in rows],
            'latency': plugins.summary([value for row in rows for value in row['latency_ms']], options.runs * options.latency_count),
        } for case, rows in result['samples'].items()}
        save()
    except Exception as error:
        result['failure'] = type(error).__name__
        save()
        raise SystemExit('Benchmark incomplete: ' + type(error).__name__) from None


if __name__ == '__main__':
    main()
