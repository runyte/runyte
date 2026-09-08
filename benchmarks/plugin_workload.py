#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Bounded, explicit application workloads; checkpoint files are benchmark evidence."""
import argparse
from collections import deque
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'docs/plugins'))
from application import Application, PluginError

MODEL_BYTES = 4 * 1024 * 1024
ROWS = 10000


def encoded(model):
    return json.dumps(model, ensure_ascii=False, separators=(',', ':')).encode()


def large_model():
    model = {'title': 'BENCH_LARGE_READY', 'purpose': 'list',
             'rows': [{'id': f'row-{index}', 'text': f'Item {index:05} 猫 ' + 'x' * 320, 'role': 'ordinary'}
                      for index in range(ROWS)]}
    missing = MODEL_BYTES - len(encoded(model))
    if missing < 0:
        raise ValueError('Large fixture exceeds its target')
    width, extra = divmod(missing, ROWS)
    for index, row in enumerate(model['rows']):
        row['text'] += 'x' * (width + (index < extra))
    if len(encoded(model)) != MODEL_BYTES:
        raise ValueError('Large fixture must encode to exactly 4 MiB')
    return model


class Workload:
    def __init__(self, evidence, owner='workload'):
        self.evidence, self.owner = Path(evidence), owner
        self.lock = threading.RLock()
        self.serial = 0
        self.view = self.revision = self.model = None
        self.job = self.process = None
        self.cancel = threading.Event()
        self.done = threading.Event()
        self.done.set()
        self.publishes = 0
        self.publication_acks = deque(maxlen=64)
        self.helper_progress = deque(maxlen=32)
        self.ended_jobs = set()
        workload = self
        class ObservedApplication(Application):
            def _read(self):
                value = super()._read()
                if value.get('type') == 'registered':
                    workload.record('registered')
                return value
            def request(self, method, **params):
                result = super().request(method, **params)
                if method in ('view.publish', 'view.stage.commit'):
                    with workload.lock:
                        workload.publication_acks.append(time.monotonic_ns())
                return result
        commands = [{'name': name, 'description': description, 'context': 'workspace'} for name, description in (
            ('visible', 'Show the bounded benchmark view'), ('large', 'Show the exact 4 MiB benchmark list'),
            ('job', 'Start the finite detached benchmark job'), ('flood', 'Start the noisy managed helper'),
            ('publish', 'Publish maximum models during ordinary document edits'),
            ('checkpoint', 'Verify current benchmark workload admission'),
            ('probe', 'Respond from a quiet independent plugin'), ('end', 'Finish the benchmark workload'))]
        self.app = ObservedApplication('Benchmark ' + owner, commands, ['views', 'jobs', 'processes'])
        self.app.handlers = {'visible': lambda context: self.show(context, False),
                             'large': lambda context: self.show(context, True),
                             'job': lambda context: self.start(context, 'job'),
                             'flood': lambda context: self.start(context, 'flood'),
                             'publish': lambda context: self.start(context, 'publish'),
                             'checkpoint': self.checkpoint, 'probe': lambda _: self.record('probe'), 'end': self.end}
        self.app.on_event = self.event

    def record(self, event, **details):
        with self.lock:
            self.serial += 1
            if self.serial > 128:
                raise PluginError('limit_exceeded', 'Benchmark evidence exceeded its bound')
            value = {'owner': self.owner, 'event': event, 'at_ns': time.monotonic_ns(), **details}
            line = json.dumps(value, separators=(',', ':')) + '\n'
            if len(line.encode()) > 4096:
                raise PluginError('limit_exceeded', 'Benchmark checkpoint exceeded its bound')
            with self.evidence.open('a') as output:
                output.write(line)
        return {}

    def show(self, context, large):
        model = large_model() if large else {'title': 'BENCH_VISIBLE_READY', 'purpose': 'list',
                    'rows': [{'id': 'one', 'text': 'BENCH_VISIBLE_ROW 猫', 'role': 'ordinary'}]}
        result = self.app.request('view.create', model={'title': model['title'], 'purpose': 'list', 'rows': []})
        self.view, self.revision = result['view'], result['revision']
        result = self.app.publish_model(self.view, self.revision, model)
        self.revision, self.model = result['revision'], model
        self.app.request('pane.show', invocation=context['invocation'], view=self.view)
        self.record('view', view=self.view, revision=self.revision, rows=len(model['rows']), bytes=len(encoded(model)))
        return {}

    def start(self, context, kind):
        if not self.done.is_set():
            raise PluginError('busy', 'Benchmark work is already running')
        if kind == 'publish' and (self.model is None or len(self.model['rows']) != ROWS):
            raise PluginError('invalid_argument', 'Prepare the large model first')
        self.cancel.clear()
        self.done.clear()
        try:
            self.job = self.app.request('job.create', title='BENCH_ACTIVE_' + kind, deadline_seconds=90)['job']
        except Exception:
            self.done.set()
            raise
        if self.job in self.ended_jobs:
            self.cancel.set()
        def worker():
            status = 'succeeded'
            try:
                if self.cancel.is_set():
                    raise PluginError('cancelled', 'Benchmark cancelled before admission')
                if kind == 'flood':
                    self.process = self.app.start_process('Benchmark noisy helper', sys.executable,
                        [str(Path(__file__).resolve()), '--noise-helper', '--evidence', str(self.evidence)])['process']
                    deadline = time.monotonic() + 5
                    while self.app.request('process.get', process=self.process)['stdout']['end'] == 0:
                        if self.cancel.wait(0.01) or time.monotonic() >= deadline:
                            raise PluginError('unavailable', 'Noisy helper never produced output')
                self.record('active', kind=kind, job=self.job, process=self.process)
                deadline = time.monotonic() + 60
                if kind == 'publish':
                    while not self.cancel.is_set() and time.monotonic() < deadline:
                        self.model['rows'][0]['text'] = ('A' if self.publishes % 2 == 0 else 'B') + self.model['rows'][0]['text'][1:]
                        try:
                            result = self.app.publish_model(self.view, self.revision, self.model)
                            self.revision = result['revision']
                            self.publishes += 1
                        except PluginError as error:
                            if error.code != 'busy':
                                raise
                            self.cancel.wait(0.05)
                elif kind == 'flood':
                    while not self.cancel.is_set() and time.monotonic() < deadline:
                        info = self.app.request('process.get', process=self.process)
                        if info['state'] != 'running':
                            raise PluginError('unavailable', 'Noisy helper exited early')
                        with self.lock:
                            self.helper_progress.append([time.monotonic_ns(), info['stdout']['end']])
                        self.cancel.wait(0.2)
                else:
                    self.cancel.wait(60)
                if self.cancel.is_set():
                    status = 'cancelled'
            except Exception:
                status = 'failed'
            finally:
                try:
                    if self.process is not None:
                        self.app.request('process.close', process=self.process)
                        self.process = None
                    try:
                        self.app.request('job.finish', job=self.job, state=status)
                    except PluginError as error:
                        if error.code == 'cancelled':
                            self.app.request('job.finish', job=self.job, state='cancelled')
                        else:
                            raise
                    self.record('ended', status=status, publishes=self.publishes)
                finally:
                    self.done.set()
        try:
            threading.Thread(target=worker, name='runyte-benchmark', daemon=True).start()
        except Exception:
            self.app.request('job.finish', job=self.job, state='failed')
            self.done.set()
            raise
        return {'job': self.job}

    def checkpoint(self, _context):
        info = self.app.request('job.get', job=self.job) if self.job is not None else None
        process = self.app.request('process.get', process=self.process) if self.process is not None else None
        with self.lock:
            publication_acks, helper_progress = list(self.publication_acks), list(self.helper_progress)
        return self.record('checkpoint', publication_acks=publication_acks, helper_progress=helper_progress,
            job_state=info['state'] if info else None,
            process_state=process['state'] if process else None,
            helper_bytes=process['stdout']['end'] if process else None, publishes=self.publishes,
            rows=len(self.model['rows']) if self.model else 0,
            model_bytes=len(encoded(self.model)) if self.model else 0)

    def end(self, _context):
        self.cancel.set()
        if not self.done.wait(8):
            raise PluginError('timeout', 'Benchmark workload did not settle')
        return self.record('end_ack')

    def event(self, name, data):
        if name == 'job.cancel_requested':
            if len(self.ended_jobs) >= 16:
                self.ended_jobs.pop()
            self.ended_jobs.add(data['job'])
            if data['job'] == self.job:
                self.cancel.set()


def noise(evidence, leaf=False):
    child = None
    if not leaf:
        child = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '--noise-leaf'])
        with Path(evidence).open('a') as output:
            output.write(json.dumps({'owner': 'helper', 'event': 'descendants', 'at_ns': time.monotonic_ns(),
                                     'pids': [os.getpid(), child.pid]}) + '\n')
    chunk = b'benchmark-noise\n' * 4096
    try:
        while True:
            os.write(sys.stdout.fileno(), chunk)
            time.sleep(0.002)
    except (BrokenPipeError, KeyboardInterrupt):
        return
    finally:
        if child is not None:
            child.terminate()
            child.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path)
    parser.add_argument('--owner', default='workload')
    parser.add_argument('--noise-helper', action='store_true')
    parser.add_argument('--noise-leaf', action='store_true')
    args = parser.parse_args()
    if args.noise_helper or args.noise_leaf:
        noise(args.evidence, args.noise_leaf)
    elif args.evidence is None:
        parser.error('--evidence is required')
    else:
        Workload(args.evidence, args.owner).app.run()


if __name__ == '__main__':
    main()
