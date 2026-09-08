# SPDX-License-Identifier: MPL-2.0
"""Pure benchmark boundary checks; no editor, helper or timed workload is launched."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from jsonschema import Draft202012Validator

import plugins
import plugin_workload
from application import Application, PluginError


def shutdown(app):
    for name in ('_executor', '_resource_executor', '_control', '_observations', '_validation_executor'):
        getattr(app, name).shutdown(wait=True, cancel_futures=True)


class TerminalTests(unittest.TestCase):
    def test_split_utf8_and_wide_glyph_use_physical_columns(self):
        terminal = plugins.Terminal()
        data = '猫 edit'.encode()
        terminal.feed(data[:1])
        self.assertFalse(terminal.contains('猫'))
        terminal.feed(data[1:2])
        terminal.feed(data[2:])
        self.assertEqual(terminal.position('edit'), (0, 3))
        self.assertTrue(terminal.at((0, 3), 'edit'))
        terminal.feed(b'\x1b[?2026h\x1b[1;4Hchanged')
        self.assertFalse(terminal.contains('changed'))
        self.assertFalse(terminal.at((0, 3), 'changed'))
        terminal.feed(b'\x1b[?2026l')
        self.assertTrue(terminal.at((0, 3), 'changed'))

    def test_command_waits_for_exact_prompt_and_completed_frame(self):
        terminal = plugins.Terminal()
        bottom = f'\x1b[{terminal.screen.lines};1H\x1b[2K'.encode()
        terminal.feed(bottom + b':plugin.workload.large (Completed)')
        self.assertFalse(terminal.query('plugin.workload.large'))
        terminal.feed(b'\x1b[?2026h' + bottom + b':plugin.workload.large')
        self.assertFalse(terminal.query('plugin.workload.large'))
        terminal.feed(b'\x1b[?2026l')
        self.assertTrue(terminal.query('plugin.workload.large'))


class EvidenceTests(unittest.TestCase):
    def test_samples_require_exact_count_and_finite_values(self):
        result = plugins.summary(list(range(1, 21)), 20)
        self.assertEqual(result, {'count': 20, 'median': 10.5, 'min': 1, 'max': 20, 'p95': 19})
        for values, expected in (([], 0), ([1], 2), ([None], 1), ([float('nan')], 1), ([True], 1), ([-1], 1)):
            with self.subTest(values=values), self.assertRaises(ValueError):
                plugins.summary(values, expected)

    def test_isolated_environment_removes_parent_context_without_mutating_parent(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {
                'RUNYTE_PARENT_CONTEXT': 'parent', 'RUNYTE_BENCH_EVENTS': '/outside'}):
            root = Path(directory)
            env = plugins.environment(root)
            self.assertNotIn('RUNYTE_PARENT_CONTEXT', env)
            self.assertNotIn('RUNYTE_BENCH_EVENTS', env)
            self.assertEqual(os.environ['RUNYTE_PARENT_CONTEXT'], 'parent')
            for key in ('HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_STATE_HOME',
                        'XDG_DATA_HOME', 'XDG_RUNTIME_DIR', 'RUNYTE_ALL_HOSTS_DIR'):
                path = Path(env[key])
                self.assertEqual(path.parent, root)
                self.assertEqual(path.stat().st_mode & 0o777, 0o700)

    def test_checkpoint_reader_never_accepts_partial_or_wrong_owner_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'evidence.jsonl'
            first = {'owner': 'workload', 'event': 'active', 'at_ns': 10}
            second = {'owner': 'quiet', 'event': 'active', 'at_ns': 20}
            path.write_text(json.dumps(first) + '\n' + json.dumps(second) + '\n{"event":')
            self.assertEqual(plugins.newest(path, 'active'), first)
            self.assertIsNone(plugins.newest(path, 'active', after=10))
            path.write_text('{broken}\n')
            with self.assertRaises(json.JSONDecodeError):
                plugins.evidence(path)


class WorkloadTests(unittest.TestCase):
    def test_publication_history_records_commit_ack_not_later_stage_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            workload = plugin_workload.Workload(Path(directory) / 'evidence')
            try:
                with patch.object(Application, 'request', return_value={'revision': 'm:2'}), \
                     patch.object(plugin_workload.time, 'monotonic_ns', return_value=42):
                    workload.app.request('view.stage.commit', stage='s:1')
                    workload.app.request('view.stage.close', stage='s:1')
                self.assertEqual(list(workload.publication_acks), [42])
                with patch.object(Application, 'request', side_effect=PluginError('busy', 'Paced')):
                    with self.assertRaises(PluginError):
                        workload.app.request('view.stage.commit', stage='s:2')
                self.assertEqual(list(workload.publication_acks), [42])
            finally:
                shutdown(workload.app)

    def test_maximum_model_matches_canonical_required_fields_and_projection_bound(self):
        model = plugin_workload.large_model()
        self.assertEqual(len(plugin_workload.encoded(model)), 4 * 1024 * 1024)
        self.assertEqual(len(model['rows']), 10000)
        self.assertEqual(len({row['id'] for row in model['rows']}), 10000)
        self.assertTrue(all(set(row) == {'id', 'text', 'role'} and row['role'] == 'ordinary'
                            for row in model['rows']))
        # A plain list projects exactly one text line per row. No header or block.
        self.assertLess(len(('\n'.join(row['text'] for row in model['rows']) + '\n').encode()),
                        4 * 1024 * 1024)
        schema = json.loads((plugins.REPO / 'docs/plugins/runyte-experimental-2.schema.json').read_text())
        Draft202012Validator({'$defs': schema['$defs'], '$ref': '#/$defs/model'}).validate(model)

    def test_real_registration_metadata_and_checkpoint_order(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'evidence.jsonl'
            workload = plugin_workload.Workload(path)
            messages = iter([{'type': 'hello', 'version': 'runyte-experimental-2'}, {'type': 'registered'}])
            output = []
            def read(_app):
                try:
                    return next(messages)
                except StopIteration:
                    raise EOFError
            with patch.object(Application, '_read', read), patch.object(Application, '_write', lambda _, message: output.append(message)):
                workload.app.run()
            schema = json.loads((plugins.REPO / 'docs/plugins/runyte-experimental-2.schema.json').read_text())
            Draft202012Validator(schema).validate(output[0])
            self.assertEqual(plugins.newest(path, 'registered')['owner'], 'workload')
            self.assertNotIn('stop', [command['name'] for command in output[0]['commands']])

    def test_view_evidence_only_follows_successful_publication_and_presentation(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'evidence.jsonl'
            workload = plugin_workload.Workload(path)
            calls = []
            def request(method, **params):
                calls.append(method)
                self.assertIsNone(plugins.newest(path, 'view'))
                if method == 'view.create':
                    return {'view': 'v:1', 'revision': 'm:1'}
                raise PluginError('context_changed', 'Refused presentation')
            workload.app.request = request
            workload.app.publish_model = lambda *args: {'revision': 'm:2'}
            try:
                with self.assertRaises(PluginError):
                    workload.show({'invocation': 'i:1'}, False)
                self.assertEqual(calls, ['view.create', 'pane.show'])
                self.assertIsNone(plugins.newest(path, 'view'))
            finally:
                shutdown(workload.app)

    def test_failed_job_admission_releases_workload_slot(self):
        with tempfile.TemporaryDirectory() as directory:
            workload = plugin_workload.Workload(Path(directory) / 'evidence')
            workload.app.request = lambda *args, **kwargs: (_ for _ in ()).throw(PluginError('busy', 'Full'))
            try:
                with self.assertRaises(PluginError):
                    workload.start({}, 'job')
                self.assertTrue(workload.done.is_set())
            finally:
                shutdown(workload.app)


class WindowTests(unittest.TestCase):
    def test_legacy_epoch1_case_retains_the_original_uppercase_example(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(plugins, 'spawn', return_value=(123, 4)), \
             patch.object(plugins.subprocess, 'run'):
            editor = plugins.Session(Path('/unused'), Path(directory), 'short.txt', epoch1=True)
            config = (Path(editor.env['XDG_CONFIG_HOME']) / 'runyte/config.yaml').read_text()
            self.assertIn('docs/plugins/uppercase.py', config)
            self.assertNotIn('runyte-experimental-2', config)
            self.assertNotIn('plugin_workload.py', config)

    def test_progress_outside_input_phase_is_not_latency_evidence(self):
        phase = {'start_ns': 2_000_000_000, 'end_ns': 4_000_000_000}
        for kind, checkpoint in (
                ('publish', {'publication_acks': [1, 5_000_000_000]}),
                ('publish', {'publication_acks': [2_100_000_000]}),
                ('flood', {'helper_progress': [[2_100_000_000, 10], [3_000_000_000, 10]]})):
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                plugins.phase_progress(kind, phase, checkpoint)
        self.assertEqual(plugins.phase_progress('publish', phase,
                         {'publication_acks': [1, 2_100_000_000, 3_000_000_000]}),
                         [2_100_000_000, 3_000_000_000])
        self.assertEqual(len(plugins.phase_progress('flood', phase,
                         {'helper_progress': [[2_100_000_000, 10], [3_000_000_000, 20]]})), 2)

    def test_first_usable_view_requires_render_and_admission_in_a_separate_process(self):
        class Terminal:
            rendered = False
            def contains(self, text):
                return self.rendered and text == 'BENCH_VISIBLE_ROW'
        class Editor:
            terminal = Terminal()
            first_byte = 2
            origin = 0
            evidence = Path('/not-read')
            commands = []
            def document(self):
                return 3
            def plugin_ready(self):
                return 4
            def command(self, value, **kwargs):
                self.commands.append((value, kwargs))
            def until(self, predicate, description):
                if description == 'First usable rendered plugin view':
                    assert not predicate()
                    self.terminal.rendered = True
                assert predicate()
        editor = Editor()
        @contextmanager
        def fake_session(*args, **kwargs):
            self.assertTrue(kwargs['enabled'])
            yield editor
        with patch.object(plugins, 'session', fake_session), patch.object(plugins, 'newest', return_value={'event': 'view'}), \
             patch.object(plugins.time, 'perf_counter', return_value=.005):
            result = plugins.startup_view_sample(Path('/unused'), 'short.txt')
        self.assertEqual(editor.commands, [('plugin.workload.visible', {'known_normal': True})])
        self.assertEqual(result['first_usable_view_ms'], 5)
        self.assertEqual(result['plugin_registered_ms'], 4)
        self.assertNotIn('ready_to_edit_ms', result)

    def test_detached_window_requires_authoritative_live_job_after_reattachment(self):
        class Editor:
            epoch1_proven = False
            bytes = read_chunks = 0
            now = 0
            evidence = Path('/unused')
            reattached = False
            state = 'running'
            def prepare_workload(self, kind):
                self.asserted_kind = kind
                return {'job_state': 'running'}
            def drain(self, duration):
                self.now += duration
            def detach(self):
                pass
            def host_pid(self):
                return 71
            def reattach(self):
                self.reattached = True
            def checkpoint(self):
                assert self.reattached
                return {'job_state': self.state}
        for state in ('running', 'succeeded', 'failed'):
            editor = Editor()
            editor.state = state
            @contextmanager
            def fake_session(*args, **kwargs):
                yield editor
            snapshots = [{'ticks': 10, 'identities': {'71': 'same'}, 'pids': [71]},
                         {'ticks': 10, 'identities': {'71': 'same'}, 'pids': [71]}]
            with self.subTest(state=state), patch.object(plugins, 'session', fake_session), \
                 patch.object(plugins, 'cpu_snapshot', side_effect=snapshots), \
                 patch.object(plugins.time, 'perf_counter', lambda: editor.now), \
                 patch.object(plugins.time, 'sleep', side_effect=lambda duration: setattr(editor, 'now', editor.now + duration)), \
                 patch.object(plugins, 'newest', return_value=None):
                if state == 'running':
                    result = plugins.workload_sample(Path('/unused'), 'detached-job', 10, 20)
                    self.assertEqual(result['after_checkpoint'], {'job_state': 'running'})
                    self.assertIsNone(result['screen_bytes'])
                    self.assertIsNone(result['pty_read_chunks'])
                else:
                    with self.assertRaisesRegex(ValueError, 'did not survive'):
                        plugins.workload_sample(Path('/unused'), 'detached-job', 10, 20)
                self.assertTrue(editor.reattached)

    def test_epoch1_proof_cannot_pass_without_actual_result_and_undo(self):
        class Terminal:
            def contains(self, _text):
                return True
            def position(self, _text):
                return (0, 0)
            def at(self, _position, _text):
                return True
            def query(self, _value):
                return True
        with tempfile.TemporaryDirectory() as directory:
            editor = object.__new__(plugins.Session)
            editor.project = Path(directory)
            editor.fd = 99
            editor.terminal = Terminal()
            editor.epoch1_proven = False
            editor.command = lambda _: None
            editor.return_document = lambda: None
            observations = []
            def until(predicate, description):
                observations.append(description)
                if description == 'Actual epoch 1 replacement':
                    raise TimeoutError('No result')
                assert predicate()
            editor.until = until
            with patch.object(plugins.os, 'write'):
                with self.assertRaises(TimeoutError):
                    editor.prove_epoch1()
            self.assertFalse(editor.epoch1_proven)
            self.assertNotIn('Epoch 1 single-step undo', observations)
            editor.until = lambda predicate, description: observations.append(description) if predicate() else None
            with patch.object(plugins.os, 'write'):
                editor.prove_epoch1()
            self.assertTrue(editor.epoch1_proven)
            self.assertIn('Epoch 1 single-step undo', observations)

    def test_latency_and_checkpoint_frames_do_not_pollute_idle_screen_totals(self):
        class Editor:
            pid = 1
            epoch1_proven = False
            bytes = read_chunks = 0
            now = 0
            def prepare_workload(self, _kind):
                return {'job_state': 'running', 'publishes': 1}
            def drain(self, duration):
                self.now += duration
                self.bytes += 10
                self.read_chunks += 1
            def checkpoint(self, *args):
                self.bytes += 1000
                self.read_chunks += 100
                return {'job_state': 'running', 'publishes': 2, 'publication_acks': [100, 1_100_000_000]}
        editor = Editor()
        @contextmanager
        def fake_session(*args, **kwargs):
            yield editor
        def latency(*args):
            editor.bytes += 2000
            editor.read_chunks += 200
            return [1, 2], {'start_ns': 0, 'end_ns': 1_200_000_000}
        snapshots = [{'ticks': 10, 'identities': {'1': 'old'}, 'pids': [1]},
                     {'ticks': 12, 'identities': {'1': 'old'}, 'pids': [1]}]
        with patch.object(plugins, 'session', fake_session), patch.object(plugins, 'cpu_snapshot', side_effect=snapshots), \
             patch.object(plugins.time, 'perf_counter', lambda: editor.now), patch.object(plugins, 'latency_samples', latency):
            result = plugins.workload_sample(Path('/unused'), 'publish', 10, 2)
        self.assertEqual(result['screen_bytes'], 10)
        self.assertEqual(result['pty_read_chunks'], 1)
        self.assertEqual(result['latency_ms'], [1, 2])
        self.assertTrue(result['complete'])


if __name__ == '__main__':
    unittest.main()
