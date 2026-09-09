# SPDX-License-Identifier: MPL-2.0
"""Native media controller with an in-memory managed process and public host calls."""
import copy
import json
from pathlib import Path
from unittest.mock import patch
import tempfile
import threading
import unittest

from application import PluginError
from media import MediaApplication, media_path


class Timer:
    def __init__(self, interval, callback):
        self.interval, self.callback = interval, callback
        self.started = self.cancelled = False
    def start(self):
        self.started = True
    def cancel(self):
        self.cancelled = True
    def fire(self):
        if not self.cancelled:
            self.callback()


class Port:
    def __init__(self):
        self.calls = []
        self.output = bytearray()
        self.state = MediaApplication.empty_state()
        self.callback = None
        self.process = 'process:1'
        self.revision = 0
        self.serial = 0
        self.info_state = 'running'
        self.hook = None
        self.command_hook = None
        self.commands = []
        self.lock = threading.RLock()

    def request(self, method, **params):
        with self.lock:
            self.calls.append((method, params))
            self.serial += 1
            if method == 'job.create':
                result = {'job': f'job:{self.serial}'}
            elif method in ('view.create', 'view.publish'):
                self.revision += 1
                result = {'view': 'view:1', 'revision': f'm:{self.revision}'}
            elif method == 'process.get':
                result = {'process': self.process, 'state': self.info_state, 'stdout': {'start': 0, 'end': len(self.output)},
                          'output_truncated': False}
            else:
                result = {}
        if self.hook:
            self.hook(method, params, result)
        if method == 'process.close':
            self.info_state = 'exited'
        return result

    def start_process(self, label, executable, args):
        self.request('process.start', label=label, executable=executable, args=args)
        self.info_state = 'running'
        self.output.clear()
        self.emit({'event': 'state', 'state': self.state})
        return {'process': self.process}

    def subscribe(self, sources, callback):
        self.request('event.subscribe', sources=sources)
        self.callback = callback
        callback('event.baseline', 'e:1', {})
        return {'subscription': 'subscription:1'}

    def unsubscribe(self, subscription):
        self.request('event.unsubscribe', subscription=subscription)
        self.callback = None

    def resync(self, subscription):
        return self.request('event.resync', subscription=subscription)

    def acquire_activity(self, title, duration_seconds=600):
        result = {'lease': f'lease:{self.serial + 1}', 'duration_seconds': duration_seconds}
        self.request('activity.acquire', title=title, result=result)
        return result

    def release_activity(self, lease):
        return self.request('activity.release', lease=lease)

    def renew_activity(self, lease, duration_seconds=600):
        self.request('activity.renew', lease=lease)
        return {'lease': lease, 'duration_seconds': duration_seconds}

    def read_process(self, process, stream, offset, limit):
        self.request('process.read', process=process, stream=stream, offset=offset, limit=limit)
        data = bytes(self.output[offset:offset + limit])
        return {'data': data, 'next': offset + len(data), 'eof': False}

    def publish_model(self, view, revision, model):
        return self.request('view.publish', view=view, expected_revision=revision, model=model)

    def write_process(self, process, data):
        self.request('process.write', process=process, data=data)
        message = json.loads(data)
        self.commands.append(message)
        if self.command_hook and self.command_hook(message):
            return
        command = message['command']
        if command == 'load':
            self.state['playlist'] = [{'id': str(i), 'path': path} for i, path in enumerate(message['paths'])]
            self.state.update(current=0, paused=False, playing=True)
        elif command == 'add':
            for path in message['paths']:
                self.state['playlist'].append({'id': str(len(self.state['playlist'])), 'path': path})
        elif command == 'play':
            self.state.update(current=message.get('index', self.state['current'] or 0), paused=False, playing=True)
        elif command == 'pause':
            self.state.update(paused=True, playing=False)
        elif command == 'seek':
            self.state['position'] = max(0, self.state['position'] + message['seconds'])
        self.emit({'id': message['id'], 'ok': True, 'state': self.state})

    def emit(self, message):
        with self.lock:
            self.output.extend((json.dumps(message) + '\n').encode())
            callback = self.callback
        if callback:
            callback('event.changed', 'e:2', {})


class MediaTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'tone.wav').write_bytes(b'fixture data never executed')
        self.port = Port()
        self.media = MediaApplication(self.port, self.root, timer_factory=Timer)
        self.addCleanup(self.cleanup)

    def cleanup(self):
        session = self.media.session
        if session:
            self.media.close(session)
            session['closed'].wait(2)

    def context(self, **arguments):
        return {'view': self.media.view, 'model_revision': self.media.revision,
                'invocation': 'fresh:1', 'rows': [], 'arguments': arguments}

    def settle(self):
        self.assertTrue(self.media.control_done.wait(2), 'Media control did not finish')
        self.assertTrue(self.media.publisher_idle.wait(2), 'Media publisher did not finish')

    def open(self):
        self.media.enqueue(self.context(path='tone.wav'), 'load')
        self.settle()
        self.assertTrue(self.media.state['playing'])
        return self.media.session

    def test_sdk_registration_matches_wire_and_avoids_reserved_plugin_stop(self):
        from application import VERSION
        from jsonschema import Draft202012Validator
        app = MediaApplication(workspace_root=self.root).app
        inputs = iter([{'type': 'hello', 'version': VERSION}, {'type': 'registered'}])
        def read():
            try:
                return next(inputs)
            except StopIteration:
                raise EOFError() from None
        frames = []
        app._read, app._write = read, frames.append
        app.run()
        schema = json.loads(Path(__file__).with_name('runyte-experimental-2.schema.json').read_text())
        Draft202012Validator({'$defs': schema['$defs'], '$ref': '#/$defs/register'}).validate(frames[0])
        names = [entry['name'] for entry in frames[0]['commands']]
        self.assertNotIn('stop', names)
        self.assertIn('stop-playback', names)
        self.assertEqual(set(names), set(app.handlers))

    def test_exact_managed_argv_and_authoritative_playback_have_preexisting_lease(self):
        session = self.open()
        methods = [method for method, _ in self.port.calls]
        self.assertLess(methods.index('activity.acquire'), methods.index('process.start'))
        self.assertLess(methods.index('activity.acquire'), methods.index('process.write'))
        start = next(params for method, params in self.port.calls if method == 'process.start')
        self.assertEqual(start['args'][0], str(Path(__file__).with_name('mpv_backend.py')))
        self.assertEqual(self.port.commands[-1]['paths'], [str(self.root / 'tone.wav')])
        self.assertIsNotNone(session['lease'])
        self.assertEqual(session['timer'].interval, 540)
        self.assertIn('Playing', self.media.model()['detail']['text'])

    def test_pause_releases_lease_and_timer_but_keeps_single_backend_quiet(self):
        session = self.open()
        timer = session['timer']
        self.media.enqueue(self.context(), 'pause')
        self.settle()
        self.assertIsNone(session['lease'])
        self.assertTrue(timer.cancelled)
        self.assertIsNone(session['timer'])
        self.assertEqual(sum(method == 'process.close' for method, _ in self.port.calls), 0)
        before = len(self.port.calls)
        timer.fire()
        self.assertEqual(len(self.port.calls), before)
        self.media.enqueue(self.context(), 'play')
        self.settle()
        self.assertEqual(sum(method == 'process.start' for method, _ in self.port.calls), 1)
        self.assertEqual(sum(method == 'activity.acquire' for method, _ in self.port.calls), 2)

    def test_delayed_load_does_not_release_lease_on_initial_paused_state(self):
        entered = threading.Event()
        def hold(message):
            entered.set()
            return True
        self.port.command_hook = hold
        self.media.enqueue(self.context(path='tone.wav'), 'load')
        self.assertTrue(entered.wait(2))
        self.assertTrue(self.media.publisher_idle.wait(2))
        session = self.media.session
        self.assertIsNotNone(session['lease'])
        self.assertFalse(any(method == 'activity.release' for method, _ in self.port.calls))
        self.media.event('activity.cancel_requested', {'lease': session['lease']})
        self.assertTrue(session['closed'].wait(2))
        self.settle()
        methods = [method for method, _ in self.port.calls]
        self.assertLess(methods.index('process.close'), methods.index('activity.release'))

    def test_selected_stable_row_and_stale_model_actions_are_fenced(self):
        self.open()
        (self.root / 'other.mp3').write_bytes(b'other fixture')
        self.media.enqueue(self.context(path='other.mp3'), 'add')
        self.settle()
        context = self.context()
        context['rows'] = [self.media.row_id(self.media.state['playlist'][1])]
        self.media.enqueue(context, 'play')
        self.settle()
        self.assertEqual(self.port.commands[-1]['index'], 1)
        with self.assertRaises(PluginError):
            self.media.enqueue(context, 'pause')
        bad = self.context()
        bad['rows'] = ['retired-id']
        with self.assertRaises(PluginError):
            self.media.enqueue(bad, 'play')

    def test_current_paused_row_resumes_without_resetting_its_position(self):
        self.open()
        self.media.enqueue(self.context(seconds='7.5'), 'seek')
        self.settle()
        self.media.enqueue(self.context(), 'pause')
        self.settle()
        context = self.context()
        context['rows'] = [self.media.row_id(self.media.state['playlist'][0])]
        self.media.enqueue(context, 'play')
        self.settle()
        self.assertNotIn('index', self.port.commands[-1])
        self.assertEqual(self.media.state['position'], 7.5)
        self.assertTrue(self.media.state['playing'])

    def test_slow_view_publication_cannot_block_close_or_activity_cancellation(self):
        session = self.open()
        entered, resume, view_closed = threading.Event(), threading.Event(), threading.Event()
        def hook(method, params, result):
            if method == 'view.publish':
                entered.set()
                resume.wait(2)
        self.port.hook = hook
        self.port.state['position'] = 1
        self.port.emit({'event': 'state', 'state': self.port.state})
        self.assertTrue(entered.wait(2))
        def close_view():
            self.media.event('view.closed', {'view': self.media.view})
            view_closed.set()
        thread = threading.Thread(target=close_view)
        thread.start()
        try:
            self.assertTrue(view_closed.wait(1))
            self.media.event('activity.cancel_requested', {'lease': session['lease']})
            self.assertTrue(session['closed'].wait(2))
        finally:
            resume.set()
            thread.join(2)
        self.assertTrue(self.media.publisher_idle.wait(2))
        self.assertIsNone(self.media.view)

    def test_failed_control_worker_start_finishes_reserved_job(self):
        start = threading.Thread.start
        def refusing(thread):
            if thread.name == 'runyte-media-control':
                raise RuntimeError('Injected thread start failure')
            return start(thread)
        with patch('threading.Thread.start', refusing), self.assertRaises(RuntimeError):
            self.media.enqueue(self.context(path='tone.wav'), 'load')
        self.assertTrue(self.media.control_done.is_set())
        self.assertFalse(self.media.control_active)
        self.assertEqual([p['state'] for m, p in self.port.calls if m == 'job.finish'], ['failed'])
        self.assertIsNone(self.media.session)

    def test_seek_bounds_and_local_media_containment_refuse_before_helper_io(self):
        for path in ('https://example.test/a.mp3', '../tone.wav', '/tmp/tone.wav', 'script.py', 'playlist.m3u'):
            with self.subTest(path=path), self.assertRaises(PluginError):
                media_path(self.root, path)
        self.open()
        before = len(self.port.commands)
        for value in ('nan', 'inf', '86401', '1' * 33):
            with self.assertRaises(PluginError):
                self.media.enqueue(self.context(seconds=value), 'seek')
        self.assertEqual(len(self.port.commands), before)
        self.media.enqueue(self.context(seconds='2.5'), 'seek')
        self.settle()
        self.assertEqual(self.media.state['position'], 2.5)

    def test_early_job_and_activity_cancellation_never_start_backend(self):
        for target in ('job.create', 'activity.acquire'):
            with self.subTest(target=target):
                def hook(method, params, result):
                    if method == target:
                        if method == 'job.create':
                            self.media.event('job.cancel_requested', result)
                        else:
                            self.media.event('activity.cancel_requested', params['result'])
                self.port.hook = hook
                self.media.enqueue(self.context(path='tone.wav'), 'load')
                self.settle()
                session = self.media.session
                if session:
                    self.assertTrue(session['closed'].wait(2))
                self.assertFalse(any(method == 'process.start' for method, _ in self.port.calls))

    def test_natural_end_releases_protection_and_resync_reads_without_restarting(self):
        session = self.open()
        self.port.state.update(playing=False, paused=False, current=None)
        self.port.emit({'event': 'state', 'state': self.port.state})
        self.media.observed(session, 'event.resync_required', 'e:3', {})
        # Wait for the actual release, without injecting periodic controller IO.
        released = threading.Event()
        original = self.port.hook
        self.port.hook = lambda method, params, result: released.set() if method == 'activity.release' else None
        self.media.schedule_read(session)
        if session['lease'] is not None:
            self.assertTrue(released.wait(2))
        self.assertIsNone(session['timer'])
        self.assertEqual(sum(method == 'process.start' for method, _ in self.port.calls), 1)
        self.port.hook = original

    def test_malformed_or_evicted_helper_output_closes_before_releasing_lease(self):
        session = self.open()
        self.port.emit({'event': 'state', 'state': {**self.port.state, 'position': float('nan')}})
        self.assertTrue(session['closed'].wait(2))
        methods = [method for method, _ in self.port.calls]
        self.assertLess(methods.index('process.close'), methods.index('activity.release'))


if __name__ == '__main__':
    unittest.main()
