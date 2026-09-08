#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Serial plugin startup, descendant CPU, and demonstrated input-to-frame evidence.

Build and retain both binaries before running. --applications includes native
views, a detached finite job, and active maximum-model/helper workloads. Every
sample is retained; an incomplete sample fails the run rather than becoming zero.
"""
import argparse
from contextlib import contextmanager
import hashlib
import json
import math
import os
from pathlib import Path
import pty
import select
import signal
import statistics
import subprocess
import sys
import tempfile
import time

import fixtures
import ptybench
import startup

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
DOCUMENT = 'local function scan_0'
TIMEOUT = 40


class Terminal(startup.Terminal):
    """Keep pyte's incremental UTF-8 decoder; read cells, avoiding display's wide-glyph bug."""
    def lines(self):
        return [''.join(self.screen.buffer[row][column].data for column in range(self.screen.columns))
                for row in range(self.screen.lines)]

    def position(self, text):
        if (2026 << 5) in self.screen.mode:
            return None
        for row in range(self.screen.lines):
            line, columns = '', []
            for column in range(self.screen.columns):
                data = self.screen.buffer[row][column].data
                line += data
                columns.extend([column] * len(data))
            index = line.find(text)
            if index >= 0:
                return row, columns[index]
        return None

    def query(self, value):
        return ((2026 << 5) not in self.screen.mode
                and self.lines()[-1].rstrip() == ':' + value)

    def at(self, position, text):
        if (2026 << 5) in self.screen.mode:
            return False
        row, column = position
        line = ''.join(self.screen.buffer[row][index].data for index in range(column, self.screen.columns))
        return line.startswith(text)


def environment(root):
    env = dict(os.environ)
    for key in ('RUNYTE_PARENT_CONTEXT', 'RUNYTE_BENCH_EVENTS'):
        env.pop(key, None)
    for key, name in [('HOME', 'home'), ('XDG_CONFIG_HOME', 'config'), ('XDG_CACHE_HOME', 'cache'),
                      ('XDG_STATE_HOME', 'state'), ('XDG_DATA_HOME', 'data'), ('XDG_RUNTIME_DIR', 'runtime'),
                      ('RUNYTE_ALL_HOSTS_DIR', 'hosts')]:
        path = root / name
        path.mkdir(mode=0o700)
        env[key] = str(path)
    env['TERM'] = 'xterm-256color'
    return env


def spawn(argv, env, cwd):
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(cwd)
        os.environ.clear()
        os.environ.update(env)
        try:
            os.execv(argv[0], argv)
        except OSError:
            os._exit(127)
    ptybench._configure(fd)
    return pid, fd


def evidence(path):
    if not path.exists():
        return []
    data = path.read_bytes()
    if len(data) > 512 * 1024:
        raise ValueError('Workload checkpoint log exceeded its bound')
    # Writers append one bounded record. Ignore only an unfinished final record.
    return [json.loads(line) for line in data.split(b'\n')[:-1] if line]


def newest(path, event, owner='workload', after=0):
    return next((item for item in reversed(evidence(path))
                 if item['event'] == event and item['owner'] == owner and item['at_ns'] > after), None)


def binary_info(path):
    digest = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(block)
    return {'path': str(path), 'bytes': path.stat().st_size, 'sha256': digest.hexdigest()}


def summary(values, expected):
    if expected < 1 or len(values) != expected or any(not isinstance(v, (int, float)) or isinstance(v, bool)
                                      or not math.isfinite(v) or v < 0 for v in values):
        raise ValueError(f'Incomplete or invalid sample set ({len(values)}/{expected})')
    ordered = sorted(values)
    return {'count': len(values), 'median': statistics.median(values), 'min': min(values),
            'max': max(values), 'p95': ordered[math.ceil(len(values) * .95) - 1]}


def cpu_snapshot(roots):
    if not Path('/proc').is_dir():
        raise RuntimeError('Descendant CPU measurement currently requires Linux /proc')
    # Keep root own/reaped-child accounting from ptybench; roots must not overlap.
    roots = set(roots)
    live = set()
    def visit(pid):
        if pid in live:
            return
        Path(f'/proc/{pid}/stat').read_text()
        live.add(pid)
        try:
            tasks = list(Path(f'/proc/{pid}/task').iterdir())
        except FileNotFoundError:
            return
        for task in tasks:
            try:
                children = (task / 'children').read_text().split()
            except FileNotFoundError:
                continue
            for child in children:
                try:
                    visit(int(child))
                except FileNotFoundError:
                    pass  # Reaped child's ticks remain charged to its parent.
    for pid in roots:
        visit(pid)
    identities = {str(pid): Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[-1].split()[19] for pid in roots}
    total = sum(ptybench._cpu_ticks(pid) for pid in roots)
    return {'ticks': total, 'pids': sorted(live), 'roots': sorted(roots), 'identities': identities}


def process_identity(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[-1].split()
    return int(fields[1]), fields[19]


def process_children(pid):
    children = set()
    for task in Path(f'/proc/{pid}/task').iterdir():
        try:
            children.update(int(value) for value in (task / 'children').read_text().split())
        except FileNotFoundError:
            continue
        if len(children) > 128:
            raise ValueError('Cleanup descendant count exceeded its bound')
    return children


class OwnedProcesses:
    """Pin verified process identities before cleanup; never signal numeric PIDs."""
    def __init__(self):
        self.fds = {}

    def capture(self, pid, validate=None, parent=None):
        if pid in self.fds:
            return
        if len(self.fds) >= 128:
            raise ValueError('Cleanup process count exceeded its bound')
        try:
            before = process_identity(pid)
            fd = os.pidfd_open(pid)
        except (FileNotFoundError, ProcessLookupError):
            return
        try:
            after = process_identity(pid)
            if before != after or select.select([fd], [], [], 0)[0]:
                return
            if parent is not None:
                if after[0] != parent or select.select([self.fds[parent]], [], [], 0)[0]:
                    return
            if validate is not None:
                validate(pid)
            self.fds[pid] = fd
            fd = None
            try:
                children = process_children(pid)
            except FileNotFoundError:
                children = ()
            for child in children:
                self.capture(child, parent=pid)
        except (FileNotFoundError, ProcessLookupError):
            pass
        finally:
            if fd is not None:
                os.close(fd)

    def wait(self, seconds=2):
        pending = set(self.fds.values())
        deadline = time.monotonic() + seconds
        while pending:
            ready = select.select(list(pending), [], [], max(0, deadline - time.monotonic()))[0]
            pending.difference_update(ready)
            if pending and time.monotonic() >= deadline:
                return False
        return True

    def terminate(self):
        # Descendants are retained after their parents. Stop them first so an
        # intact parent still has the opportunity to reap before its own exit.
        for fd in reversed(list(self.fds.values())):
            try:
                signal.pidfd_send_signal(fd, signal.SIGKILL)
            except ProcessLookupError:
                pass
        return self.wait()

    def close(self):
        for fd in self.fds.values():
            os.close(fd)
        self.fds.clear()


class Session:
    def __init__(self, binary, root, fixture, enabled=False, persistent=False, quiet=False, epoch1=False):
        self.binary, self.root = binary, root
        self.env = environment(root)
        self.project = root / 'project'
        self.project.mkdir()
        subprocess.run(['git', 'init', '-q'], cwd=self.project, env=self.env, check=True, capture_output=True)
        self.fixture = fixtures.ensure(self.project, [fixture])[fixture]
        self.original = self.fixture.read_bytes()
        self.evidence = root / 'evidence.jsonl'
        config_dir = Path(self.env['XDG_CONFIG_HOME']) / 'runyte'
        config_dir.mkdir()
        config = 'lsp:\n  enable: false\nworkspace:\n  session_strip: hidden\n'
        if epoch1:
            config += ('plugins:\n  - id: case\n    enabled: true\n'
                       f'    executable: {json.dumps(sys.executable)}\n'
                       f'    args: [{json.dumps(str(REPO / "docs/plugins/uppercase.py"))}]\n')
        if enabled:
            config += 'plugins:\n'
            for owner in (['workload', 'quiet'] if quiet else ['workload']):
                args = [str(HERE / 'plugin_workload.py'), '--evidence', str(self.evidence), '--owner', owner]
                config += (f'  - id: {owner}\n    enabled: true\n    api: runyte-experimental-2\n'
                           '    capabilities: [views, jobs, processes]\n'
                           f'    executable: {json.dumps(sys.executable)}\n    args: {json.dumps(args)}\n')
        (config_dir / 'config.yaml').write_text(config)
        self.enabled, self.quiet, self.persistent = enabled, quiet, persistent
        self.epoch1, self.epoch1_proven = epoch1, False
        self.terminal = Terminal()
        self.bytes = self.read_chunks = 0
        self.first_byte = None
        self.reaped = False
        self.closed_fd = False
        self.origin_ns = time.monotonic_ns()
        self.origin = time.perf_counter()
        argv = [str(binary), '-a', str(self.project)] if persistent else [str(binary), fixture]
        self.pid, self.fd = spawn(argv, self.env, str(self.project))

    def pump(self, timeout=.005):
        if not select.select([self.fd], [], [], max(0, timeout))[0]:
            return
        data = os.read(self.fd, 65536)
        if not data:
            raise EOFError('Editor closed before the sample completed')
        if self.first_byte is None:
            self.first_byte = (time.perf_counter() - self.origin) * 1000
        self.bytes += len(data)
        self.read_chunks += 1
        reply = self.terminal.feed(data)
        if reply:
            os.write(self.fd, reply)

    def until(self, predicate, description, seconds=TIMEOUT):
        deadline = time.perf_counter() + seconds
        while not predicate():
            if time.perf_counter() >= deadline:
                raise TimeoutError(description + '; terminal edges=' + repr(self.terminal.lines()[-4:]))
            self.pump()

    def drain(self, seconds):
        end = time.perf_counter() + seconds
        while time.perf_counter() < end:
            self.pump(min(.02, end - time.perf_counter()))

    def command(self, value, *, known_normal=False):
        if not known_normal:
            os.write(self.fd, b'\x1b')
            self.drain(.08)
        os.write(self.fd, b':' + value.encode())
        self.until(lambda: self.terminal.query(value), 'Displayed native command prompt ' + value)
        os.write(self.fd, b'\r')

    def checkpoint(self, event='checkpoint', owner='workload'):
        after = time.monotonic_ns()
        self.command(f'plugin.{owner}.' + ('probe' if event == 'probe' else 'checkpoint'))
        self.until(lambda: newest(self.evidence, event, owner, after) is not None, 'Host-admitted workload checkpoint')
        return newest(self.evidence, event, owner, after)

    def document(self):
        if self.persistent:
            self.until(lambda: self.terminal.contains('[about]'), 'Persistent attachment')
            self.command('open ' + self.fixture.name)
        self.until(lambda: self.terminal.contains(DOCUMENT), 'First complete document frame')
        return (time.perf_counter() - self.origin) * 1000

    def prove_epoch1(self):
        # Legacy invocations capture the whole document and cannot demonstrate
        # readiness on maximum fixtures. Probe a tiny file outside timed input,
        # then undo and restore the measured document before idle settlement.
        text = 'legacy_registration_probe'
        (self.project / 'epoch1-proof.txt').write_text(text + '\n')
        self.command('open epoch1-proof.txt')
        self.until(lambda: self.terminal.contains(text), 'Epoch 1 proof document')
        position = self.terminal.position(text)
        os.write(self.fd, b':plugin.case.')
        self.until(lambda: self.terminal.contains('plugin.case.uppercase'),
                   'Accepted epoch 1 command metadata')
        os.write(self.fd, b'uppercase')
        self.until(lambda: self.terminal.query('plugin.case.uppercase'), 'Epoch 1 command prompt')
        os.write(self.fd, b'\r')
        self.until(lambda: self.terminal.at(position, 'L' + text[1:]), 'Actual epoch 1 replacement')
        os.write(self.fd, b'u')
        self.until(lambda: self.terminal.at(position, text), 'Epoch 1 single-step undo')
        self.return_document()
        self.epoch1_proven = True

    def plugin_ready(self):
        self.until(lambda: newest(self.evidence, 'registered') is not None, 'Accepted plugin registration')
        item = newest(self.evidence, 'registered')
        if self.quiet:
            self.until(lambda: newest(self.evidence, 'registered', 'quiet') is not None, 'Quiet owner registration')
        return (item['at_ns'] - self.origin_ns) / 1e6

    def prepare_workload(self, kind):
        self.document()
        if self.enabled:
            self.plugin_ready()
        elif self.epoch1:
            self.prove_epoch1()
        if kind in ('visible', 'large', 'publish'):
            self.command('plugin.workload.' + ('large' if kind in ('large', 'publish') else 'visible'))
            self.until(lambda: newest(self.evidence, 'view') is not None, 'Accepted and presented native view')
            item = newest(self.evidence, 'view')
            expected = 10000 if kind != 'visible' else 1
            if item['rows'] != expected or kind != 'visible' and item['bytes'] != 4 * 1024 * 1024:
                raise ValueError('Maximum model workload was not admitted exactly')
            self.until(lambda: self.terminal.contains('BENCH_LARGE_READY' if kind != 'visible' else 'BENCH_VISIBLE_ROW'),
                       'Actual rendered native workload')
        if kind in ('job', 'flood', 'publish'):
            self.command('plugin.workload.' + kind)
            self.until(lambda: newest(self.evidence, 'active') is not None, 'Live finite workload')
            if kind == 'flood':
                self.until(lambda: newest(self.evidence, 'descendants', 'helper') is not None,
                           'Owned helper and noisy descendant')
        checkpoint = self.checkpoint() if self.enabled else None
        if kind in ('job', 'flood', 'publish') and checkpoint['job_state'] != 'running':
            raise ValueError('Background job was not running')
        return checkpoint

    def return_document(self):
        self.command('open ' + self.fixture.name)
        self.until(lambda: self.terminal.contains(DOCUMENT), 'Ordinary document restored')

    def detach(self):
        self.command('detach')
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.reaped = True
                if status != 0:
                    raise RuntimeError('Attachment failed while detaching')
                os.close(self.fd)
                self.closed_fd = True
                return
            try:
                self.pump(.01)
            except (EOFError, OSError):
                pass
        raise TimeoutError('Attachment did not detach')

    def reattach(self):
        if not self.persistent or not self.reaped or not self.closed_fd:
            raise ValueError('Reattachment requires a settled detached frontend')
        self.terminal = Terminal()
        self.pid, self.fd = spawn([str(self.binary), '-a', str(self.project)], self.env, str(self.project))
        self.reaped = self.closed_fd = False
        self.until(lambda: self.terminal.contains(DOCUMENT), 'Same persistent document reattached')

    def host_pid(self):
        endpoints = list(self.root.rglob('endpoint.json'))
        records = [json.loads(path.read_text()) for path in endpoints]
        if any(bytes(record['project_root_bytes']) != os.fsencode(self.project.resolve()) for record in records):
            raise ValueError('Endpoint does not belong to the isolated workspace')
        pids = {record['pid'] for record in records}
        if len(pids) != 1:
            raise ValueError('Expected exactly one isolated persistent host')
        return pids.pop()

    def validate_host(self, pid):
        if (Path(f'/proc/{pid}/cwd').resolve() != self.project.resolve()
                or Path(f'/proc/{pid}/exe').resolve() != self.binary.resolve()):
            raise ValueError('Cleanup target is not the measured host in its private workspace')

    def close(self):
        owned = OwnedProcesses()
        failures = []
        try:
            try:
                if self.persistent:
                    owned.capture(self.host_pid(), validate=self.validate_host)
                elif not self.reaped:
                    owned.capture(self.pid, validate=self.validate_host)
            except Exception as error:
                failures.append('Process capture: ' + str(error))
            try:
                self.shutdown()
            except Exception as error:
                failures.append('Ordinary shutdown: ' + str(error))
            if not owned.wait():
                failures.append('Owned process survived ordinary shutdown')
            if failures:
                if not owned.terminate():
                    failures.append('Owned process did not exit after exact-identity termination')
                raise RuntimeError('; '.join(failures))
        finally:
            owned.close()
            if not self.reaped:
                ptybench._reap(self.pid)
                self.reaped = True
            if not self.closed_fd:
                os.close(self.fd)
                self.closed_fd = True

    def shutdown(self):
        if self.persistent:
            result = subprocess.run([str(self.binary), '--session-stop', '--force', str(self.project)],
                env=self.env, cwd=self.project, capture_output=True, timeout=15)
            if result.returncode:
                raise RuntimeError('Isolated persistent host did not stop')
        elif not self.reaped:
            try:
                if self.enabled:
                    self.command('plugin.workload.end')
                    self.until(lambda: newest(self.evidence, 'end_ack') is not None, 'Actual workload cleanup')
                self.command('q!')
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    pid, status = os.waitpid(self.pid, os.WNOHANG)
                    if pid:
                        self.reaped = True
                        if status != 0:
                            raise RuntimeError('Editor quit failed')
                        break
                    try:
                        self.pump(.01)
                    except (EOFError, OSError):
                        pass
                if not self.reaped:
                    raise TimeoutError('Editor did not quit after workload cleanup')
            finally:
                ptybench._reap(self.pid)
        if not self.reaped:
            ptybench._reap(self.pid)
            self.reaped = True
        if not self.closed_fd:
            os.close(self.fd)
            self.closed_fd = True


@contextmanager
def session(binary, fixture, enabled=False, persistent=False, quiet=False, epoch1=False):
    with tempfile.TemporaryDirectory(prefix='runyte-plugin-bench-') as directory:
        value = Session(binary, Path(directory), fixture, enabled, persistent, quiet, epoch1)
        try:
            yield value
        except BaseException as error:
            try:
                value.close()
            except BaseException as cleanup:
                # Keep the original failed observation and separately report
                # cleanup, rather than replacing it with a later quit timeout.
                error.cleanup_failure = f'{type(cleanup).__name__}: {cleanup}'
            raise
        else:
            value.close()


def startup_sample(binary, fixture, enabled, epoch1=False):
    with session(binary, fixture, enabled, epoch1=epoch1) as editor:
        first_document = editor.document()
        position = editor.terminal.position(DOCUMENT)
        os.write(editor.fd, b'i ')
        editor.until(lambda: editor.terminal.at(position, ' ' + DOCUMENT), 'Demonstrated edited document')
        ready = (time.perf_counter() - editor.origin) * 1000
        registered = editor.plugin_ready() if enabled else None
        if epoch1:
            editor.prove_epoch1()
        return {'complete': True, 'first_byte_ms': editor.first_byte, 'first_document_ms': first_document,
                'ready_to_edit_ms': ready, 'plugin_registered_ms': registered,
                'epoch1_proven': editor.epoch1_proven}


def startup_view_sample(binary, fixture):
    # Separate processes keep first usable plugin work distinct from editor input readiness.
    with session(binary, fixture, enabled=True) as editor:
        first_document = editor.document()
        registered = editor.plugin_ready()
        editor.command('plugin.workload.visible', known_normal=True)
        editor.until(lambda: editor.terminal.contains('BENCH_VISIBLE_ROW'), 'First usable rendered plugin view')
        usable = (time.perf_counter() - editor.origin) * 1000
        editor.until(lambda: newest(editor.evidence, 'view') is not None, 'Accepted first plugin presentation')
        return {'complete': True, 'first_byte_ms': editor.first_byte, 'first_document_ms': first_document,
                'plugin_registered_ms': registered, 'first_usable_view_ms': usable,
                'view_admission': newest(editor.evidence, 'view')}


def phase_progress(kind, phase, checkpoint):
    start, end = phase['start_ns'], phase['end_ns']
    if end - start < 1_000_000_000:
        raise ValueError('Input phase must span at least one second')
    key = 'publication_acks' if kind == 'publish' else 'helper_progress'
    points = [point for point in checkpoint[key]
              if start <= (point if kind == 'publish' else point[0]) <= end]
    if len(points) < 2 or kind == 'flood' and points[-1][1] <= points[0][1]:
        raise ValueError('Background workload did not progress during the timed input phase')
    return points


def latency_samples(editor, count):
    editor.return_document()
    position = editor.terminal.position(DOCUMENT)
    os.write(editor.fd, b'i')
    editor.drain(.08)
    values = []
    start_ns = time.monotonic_ns()
    for number in range(1, count + 1):
        # Deterministic jitter is outside the timed interval and spans publication cycles.
        editor.drain(0.06 + (number % 4) * 0.007)
        started = time.perf_counter()
        os.write(editor.fd, b'x')
        marker = 'x' * number + DOCUMENT
        editor.until(lambda: editor.terminal.at(position, marker), 'Exact completed edit frame', seconds=10)
        values.append((time.perf_counter() - started) * 1000)
    phase = {'start_ns': start_ns, 'end_ns': time.monotonic_ns()}
    editor.command('write! verified.txt')
    target = editor.project / 'verified.txt'
    editor.until(lambda: target.exists() and target.read_bytes() == b'x' * count + editor.original,
                 'Complete saved-file verification of input samples')
    return values, phase


def workload_sample(binary, kind, window, latency_count):
    persistent = kind == 'detached-job'
    actual = 'job' if persistent else kind
    enabled = kind not in ('disabled', 'epoch1-quiescent')
    with session(binary, 'medium.lua', enabled, persistent, quiet=kind in ('flood', 'publish'),
                 epoch1=kind == 'epoch1-quiescent') as editor:
        before_checkpoint = editor.prepare_workload(actual)
        editor.drain(2.5)
        if persistent:
            editor.detach()
        roots = {editor.host_pid()} if persistent else {editor.pid}
        before = cpu_snapshot(roots)
        if kind == 'flood':
            required = newest(editor.evidence, 'descendants', 'helper')['pids']
            if not set(required) <= set(before['pids']):
                raise ValueError('Noisy helper descendants are missing from CPU accounting')
        start_bytes, start_chunks = editor.bytes, editor.read_chunks
        begin = time.perf_counter()
        if persistent:
            time.sleep(window)
        else:
            editor.drain(window)
        elapsed = time.perf_counter() - begin
        window_bytes, window_chunks = editor.bytes - start_bytes, editor.read_chunks - start_chunks
        after = cpu_snapshot(roots)
        ticks = after['ticks'] - before['ticks']
        if ticks < 0 or elapsed < window or before['identities'] != after['identities']:
            raise ValueError('Incomplete descendant CPU window')
        values = []
        probe_ms = None
        phase = progress = None
        after_checkpoint = None
        if kind in ('flood', 'publish'):
            values, phase = latency_samples(editor, latency_count)
            started = time.perf_counter()
            editor.checkpoint('probe', 'quiet')
            probe_ms = (time.perf_counter() - started) * 1000
        if enabled and not persistent:
            after_checkpoint = editor.checkpoint()
            if kind in ('flood', 'publish'):
                if after_checkpoint['job_state'] != 'running':
                    raise ValueError('Flood ended before input measurement completed')
                progress = phase_progress(kind, phase, after_checkpoint)
                metric = 'helper_bytes' if kind == 'flood' else 'publishes'
                if after_checkpoint[metric] <= before_checkpoint[metric]:
                    raise ValueError('Background workload made no progress during measurement')
        elif persistent:
            if newest(editor.evidence, 'ended') is not None:
                raise ValueError('Detached finite job ended before the idle window completed')
            # Absence of a completion log cannot prove a live job: a crashed
            # plugin might never write one. Query the actual same host afterward.
            editor.reattach()
            if {editor.host_pid()} != roots:
                raise ValueError('Persistent host changed during detached measurement')
            after_checkpoint = editor.checkpoint()
            if after_checkpoint['job_state'] != 'running':
                raise ValueError('Detached job did not survive the measured window')
        return {'complete': True, 'window_seconds': elapsed,
                'cpu_percent': ticks / os.sysconf('SC_CLK_TCK') / elapsed * 100,
                'cpu_before': before, 'cpu_after': after,
                'screen_bytes': None if persistent else window_bytes,
                'pty_read_chunks': None if persistent else window_chunks,
                'epoch1_proven': editor.epoch1_proven,
                'frontend': 'detached' if persistent else 'attached',
                'latency_ms': values, 'latency_summary': summary(values, latency_count) if values else None,
                'input_phase': phase, 'input_phase_progress': progress,
                'quiet_probe_ms': probe_ms, 'before_checkpoint': before_checkpoint,
                'after_checkpoint': after_checkpoint}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, default=REPO / 'target/release/runyte')
    parser.add_argument('--runs', type=int, default=10)
    parser.add_argument('--idle-runs', type=int, default=3)
    parser.add_argument('--window', type=float, default=10)
    parser.add_argument('--latency-samples', type=int, default=40)
    parser.add_argument('--applications', action='store_true')
    parser.add_argument('--fixtures', default='short.txt,medium.lua,long.lua')
    parser.add_argument('--json', type=Path, required=True)
    args = parser.parse_args()
    if args.runs < 10 or args.idle_runs < 3 or not math.isfinite(args.window) or args.window < 10 or not 20 <= args.latency_samples <= 60:
        parser.error('Require >=10 startups, >=3 idle windows of >=10 seconds, and 20–60 latency samples')
    before, after = args.before.resolve(), args.after.resolve()
    for binary in (before, after):
        if not binary.is_file():
            parser.error(f'Missing retained binary: {binary}')
    names = args.fixtures.split(',')
    for name in names:
        fixtures.split(name)
    artifact = {'complete': False, 'binaries': {'before': binary_info(before), 'after': binary_info(after)},
                'parameters': {'startup_runs': args.runs, 'idle_runs': args.idle_runs,
                               'idle_seconds': args.window, 'latency_samples': args.latency_samples},
                'contracts': {'startup': 'Process start to completed document frame and demonstrated first edit; registration receipt is separate. Epoch1 command/undo proof runs after captured startup times and before idle settlement.',
                              'screen_activity': 'screen_bytes counts observed PTY output bytes; pty_read_chunks counts harness reads, not editor write syscalls.',
                              'first_usable_view': 'Separate fresh process; start to first completed native plugin view frame after explicit command and acknowledged presentation; no Escape delay in known Normal mode.',
                              'input_latency': 'Each key write to exact edited marker in a completed synchronized frame; deterministic 60–81 ms spacing excluded from timing; >=2 workload progress points inside the >=1 second phase.'},
                'startup': {}, 'workloads': {}}
    def save():
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(artifact, indent=2) + '\n')
    save()
    try:
        for label, binary, enabled in [('base-disabled', before, False), ('branch-disabled', after, False),
                                        ('branch-quiescent', after, True), ('branch-epoch1-quiescent', after, False)]:
            artifact['startup'][label] = {}
            for name in names:
                samples = []
                artifact['startup'][label][name] = {'samples': samples}
                for _ in range(args.runs):
                    samples.append(startup_sample(binary, name, enabled, epoch1=label == 'branch-epoch1-quiescent'))
                    save()
                metrics = ['first_byte_ms', 'first_document_ms', 'ready_to_edit_ms']
                if enabled:
                    metrics.append('plugin_registered_ms')
                artifact['startup'][label][name]['summary'] = {key: summary([v[key] for v in samples], args.runs) for key in metrics}
                print(label, name, json.dumps(artifact['startup'][label][name]['summary']), flush=True)
        artifact['startup']['branch-first-usable-view'] = {}
        for name in names:
            samples = []
            artifact['startup']['branch-first-usable-view'][name] = {'samples': samples}
            for _ in range(args.runs):
                samples.append(startup_view_sample(after, name))
                save()
            artifact['startup']['branch-first-usable-view'][name]['summary'] = {
                key: summary([sample[key] for sample in samples], args.runs)
                for key in ('first_byte_ms', 'first_document_ms', 'plugin_registered_ms', 'first_usable_view_ms')}
            print('branch-first-usable-view', name, json.dumps(artifact['startup']['branch-first-usable-view'][name]['summary']), flush=True)
        cases = [('base-disabled', before, 'disabled'), ('branch-disabled', after, 'disabled'),
                 ('branch-quiescent', after, 'quiescent'),
                 ('branch-epoch1-quiescent', after, 'epoch1-quiescent')]
        if args.applications:
            cases += [(kind, after, kind) for kind in ('visible', 'large', 'detached-job', 'flood', 'publish')]
        for label, binary, kind in cases:
            samples = []
            artifact['workloads'][label] = {'samples': samples}
            for _ in range(args.idle_runs):
                samples.append(workload_sample(binary, kind, args.window, args.latency_samples))
                save()
            metrics = ['cpu_percent', 'window_seconds']
            if kind != 'detached-job':
                metrics.extend(['screen_bytes', 'pty_read_chunks'])
            artifact['workloads'][label]['summary'] = {key: summary([v[key] for v in samples], args.idle_runs) for key in metrics}
            if kind in ('flood', 'publish'):
                artifact['workloads'][label]['summary']['latency_ms'] = summary(
                    [value for sample in samples for value in sample['latency_ms']], args.idle_runs * args.latency_samples)
                artifact['workloads'][label]['summary']['quiet_probe_ms'] = summary(
                    [sample['quiet_probe_ms'] for sample in samples], args.idle_runs)
            print(label, json.dumps(artifact['workloads'][label]['summary']), flush=True)
        artifact['complete'] = True
    except Exception as error:
        artifact['failure'] = {'type': type(error).__name__, 'message': str(error)}
        if getattr(error, 'cleanup_failure', None):
            artifact['failure']['cleanup'] = error.cleanup_failure
        raise
    finally:
        save()


if __name__ == '__main__':
    main()
