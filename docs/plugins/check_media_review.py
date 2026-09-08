# SPDX-License-Identifier: MPL-2.0
"""Independent media lease, startup and retired-session boundary regressions."""
import copy
from pathlib import Path
import tempfile
import threading
import unittest

from application import PluginError
from check_media import Port, Timer
from media import MediaApplication


class MediaReviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='runyte-media-review-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'tone.wav').write_bytes(b'fixture data')
        self.port = Port()
        self.media = MediaApplication(self.port, self.root, timer_factory=Timer)
        self.addCleanup(self.cleanup)

    def cleanup(self):
        self.port.hook = None
        session = self.media.session
        if session is not None:
            self.media.close(session)
            session['closed'].wait(2)

    def context(self, **arguments):
        return {'view': self.media.view, 'model_revision': self.media.revision,
                'invocation': 'physical:review', 'rows': [], 'arguments': arguments}

    def settle(self):
        self.assertTrue(self.media.control_done.wait(3))
        self.assertTrue(self.media.publisher_idle.wait(3))

    def open(self):
        self.media.enqueue(self.context(path='tone.wav'), 'load')
        self.settle()
        self.assertTrue(self.media.state['playing'])
        return self.media.session

    def test_cancel_keeps_lease_until_actual_process_close_acknowledgement(self):
        session = self.open()
        lease = session['lease']
        entered, release = threading.Event(), threading.Event()

        def held_close(method, params, result):
            if method == 'process.close':
                entered.set()
                if not release.wait(3):
                    raise AssertionError('Close gate was not released')

        self.port.hook = held_close
        try:
            self.media.event('activity.cancel_requested', {'lease': lease})
            self.assertTrue(entered.wait(2))
            self.assertIs(self.media.session, session)
            self.assertEqual(session['lease'], lease)
            self.assertFalse(session['closed'].is_set())
            self.assertFalse(any(method == 'activity.release' for method, _ in self.port.calls))
            with self.assertRaises(PluginError) as raised:
                self.media.enqueue(self.context(path='tone.wav'), 'load')
            self.assertEqual(raised.exception.code, 'busy')
        finally:
            release.set()
        self.assertTrue(session['closed'].wait(3))
        self.assertIsNone(session['lease'])
        self.assertIsNone(self.media.session)

    def test_failed_lease_release_retains_token_through_cleanup_and_retries_same_token(self):
        session = self.open()
        lease = session['lease']
        entered, release = threading.Event(), threading.Event()
        releases = []

        def fail_then_close(method, params, result):
            if method == 'activity.release':
                releases.append(params['lease'])
                if len(releases) == 1:
                    raise PluginError('unavailable', 'Injected release failure')
            if method == 'process.close':
                entered.set()
                if not release.wait(3):
                    raise AssertionError('Close gate was not released')

        self.port.hook = fail_then_close
        try:
            self.media.enqueue(self.context(), 'pause')
            self.assertTrue(entered.wait(2))
            self.assertEqual(session['lease'], lease)
            self.assertFalse(session['closed'].is_set())
            self.assertEqual(releases, [lease])
        finally:
            release.set()
        self.assertTrue(session['closed'].wait(3))
        self.settle()
        self.assertEqual(releases, [lease, lease])
        self.assertIsNone(session['lease'])

    def test_denied_playback_lease_never_starts_a_helper_or_dispatches_play(self):
        def denied(method, params, result):
            if method == 'activity.acquire':
                raise PluginError('limit_exceeded', 'No playback lease available')

        self.port.hook = denied
        self.media.enqueue(self.context(path='tone.wav'), 'load')
        self.settle()
        session = self.media.session
        if session is not None:
            self.assertTrue(session['closed'].wait(2))
        self.assertFalse(self.port.commands)
        self.assertFalse(any(method in ('process.start', 'activity.release') for method, _ in self.port.calls))

    def test_old_state_cancel_and_timer_cannot_affect_reopened_session(self):
        previous = self.open()
        old_lease, old_timer = previous['lease'], previous['timer']
        self.media.enqueue(self.context(), 'stop')
        self.settle()
        self.assertTrue(previous['closed'].is_set())
        current = self.open()
        state = copy.deepcopy(self.media.state)
        before = len(self.port.calls)
        self.media.received(previous, {'event': 'state', 'state': MediaApplication.empty_state()})
        self.media.observed(previous, 'event.resync_required', 'old:sequence', {})
        self.media.event('activity.cancel_requested', {'lease': old_lease})
        old_timer.fire()
        self.assertIs(self.media.session, current)
        self.assertFalse(current['closing'])
        self.assertEqual(self.media.state, state)
        self.assertEqual(len(self.port.calls), before)

    def test_backend_initialization_wait_cancels_without_dispatching_an_early_command(self):
        subscribed = threading.Event()
        start = self.port.start_process

        def silent_start(*args):
            result = start(*args)
            self.port.output.clear()
            return result

        def noticed(method, params, result):
            if method == 'event.subscribe':
                subscribed.set()

        self.port.start_process = silent_start
        self.port.hook = noticed
        result = self.media.enqueue(self.context(path='tone.wav'), 'load')
        self.assertTrue(subscribed.wait(2))
        session = self.media.session
        self.assertIsNotNone(session)
        self.assertFalse(session['ready'].is_set())
        self.assertFalse(self.port.commands)
        self.assertIsNotNone(session['lease'])
        self.media.event('job.cancel_requested', result)
        self.assertTrue(session['closed'].wait(3))
        self.settle()
        self.assertFalse(self.port.commands)
        self.assertIsNone(session['lease'])


if __name__ == '__main__':
    unittest.main()
