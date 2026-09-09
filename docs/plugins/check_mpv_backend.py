# SPDX-License-Identifier: MPL-2.0
"""Pure bounds and real local-WAV mpv integration; MPV optionally selects a binary."""
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import wave

from mpv_backend import BackendError, Bridge, MAX_LINE, MAX_OUTPUT, encoded, local_paths

MPV = os.environ.get('MPV') or shutil.which('mpv')
BACKEND = Path(__file__).with_name('mpv_backend.py')


class BridgeBoundsTests(unittest.TestCase):
    def bridge(self):
        left, right = socket.socketpair()
        read_input, write_input = os.pipe()
        read_output, write_output = os.pipe()
        bridge = Bridge(left, read_input, write_output)
        self.addCleanup(bridge.close)
        self.addCleanup(right.close)
        for descriptor in (read_input, write_input, read_output, write_output):
            self.addCleanup(os.close, descriptor)
        bridge.initializing, bridge.expected, bridge.deadline = False, None, None
        bridge.actions.clear()
        bridge.ipc_output.clear()
        bridge.values = {'pause': True, 'playlist': [], 'playlist-pos': -1, 'idle-active': True}
        return bridge

    def test_input_and_encoded_output_reject_oversized_or_nonfinite_messages(self):
        with self.assertRaises(BackendError):
            encoded({'large': 'x' * MAX_LINE})
        for raw in [b'{', b'{"number":NaN}', b'[' * 2000]:
            with self.assertRaises(BackendError):
                Bridge.parse(raw)
        with self.assertRaises(BackendError):
            local_paths(['https://example.invalid/media'])

    def test_complete_outbox_accounting_includes_partial_and_latest_frames(self):
        bridge = self.bridge()
        bridge.writing = b'x' * (MAX_OUTPUT - 8)
        bridge.writing_reply = True
        bridge.latest_state = b'pending'
        with self.assertRaises(BackendError):
            bridge.queue_reply({'id': 1, 'ok': True})
        bridge.writing, bridge.latest_state = b'partial', None
        for number in range(15):
            bridge.queue_reply({'id': number, 'ok': True})
        with self.assertRaises(BackendError):
            bridge.queue_reply({'id': 16, 'ok': True})

    def test_control_admission_discards_old_unstarted_state_and_bounds_busy_replies(self):
        bridge = self.bridge()
        bridge.latest_state = encoded({'event': 'state', 'state': bridge.state()})
        bridge.request({'id': 1, 'command': 'status'})
        self.assertIsNone(bridge.latest_state)
        self.assertEqual(bridge.pending, 1)
        bridge.request({'id': 2, 'command': 'pause'})
        self.assertEqual(json.loads(bridge.replies[-1])['id'], 2)
        self.assertFalse(json.loads(bridge.replies[-1])['ok'])
        self.assertEqual(bridge.pending, 1)

    def test_progress_coalesces_to_two_per_second_and_paused_state_has_no_timer(self):
        bridge = self.bridge()
        bridge.allowed_paths = {'/file'}
        bridge.values.update({'pause': False, 'playlist': [{'id': 1, 'filename': '/file'}],
                              'playlist-pos': 0, 'idle-active': False, 'time-pos': 1})
        bridge.dirty = True
        bridge.publish(10)
        first = bridge.latest_state
        bridge.values['time-pos'] = 2
        bridge.dirty = True
        bridge.publish(10.2)
        self.assertEqual(bridge.latest_state, first)
        self.assertTrue(bridge.dirty)
        bridge.publish(10.5)
        self.assertEqual(json.loads(bridge.latest_state)['state']['position'], 2)
        bridge.values['pause'] = True
        bridge.dirty = True
        bridge.publish(10.6)
        self.assertFalse(json.loads(bridge.latest_state)['state']['playing'])
        self.assertFalse(bridge.dirty)
        prior = bridge.latest_state
        bridge.publish(100)
        self.assertEqual(bridge.latest_state, prior)

    def test_automatic_playlist_transition_does_not_publish_unsettled_idle(self):
        bridge = self.bridge()
        bridge.allowed_paths = {'/one', '/two'}
        bridge.values.update({'pause': False, 'playlist': [{'id': 1, 'filename': '/one'},
                                                         {'id': 2, 'filename': '/two'}],
                              'playlist-pos': 0, 'idle-active': False})
        bridge.dirty = True
        bridge.publish(10)
        playing = bridge.latest_state
        bridge.end_events.append((1, 99, 'stop'))  # Replaced playlist's old leader.
        bridge.end_events.append((2, 1, 'eof'))
        bridge.values['playlist-pos'] = -1
        for idle in (False, True):
            bridge.values['idle-active'] = idle
            bridge.dirty = True
            bridge.publish(11)
            self.assertEqual(bridge.latest_state, playing, 'Intermediate EOF cannot release playback protection')
        bridge.values.update({'playlist-pos': 1, 'idle-active': False})
        bridge.publish(12)
        self.assertTrue(json.loads(bridge.latest_state)['state']['playing'])
        bridge.values.update({'playlist-pos': -1, 'idle-active': True})
        bridge.end_events.append((3, 2, 'eof'))
        bridge.dirty = True
        bridge.publish(13)
        self.assertFalse(json.loads(bridge.latest_state)['state']['playing'])

    def test_reply_ids_and_unexpected_player_playlist_are_fail_closed(self):
        bridge = self.bridge()
        for identity in [None, True, 0, 2**31, '1']:
            with self.assertRaises(BackendError):
                bridge.request({'id': identity, 'command': 'status'})
        with self.assertRaises(BackendError):
            bridge.player_message({'request_id': 100, 'error': 'success'})
        bridge.values['playlist'] = [{'id': 1, 'filename': 'https://example.invalid'}]
        with self.assertRaises(BackendError):
            bridge.state()

    def test_malformed_successful_player_state_never_claims_paused_or_empty(self):
        for name, invalid in [('pause', None), ('pause', 'yes'), ('idle-active', 0),
                              ('playlist-pos', True), ('playlist', None), ('playlist', False)]:
            bridge = self.bridge()
            bridge.values[name] = invalid
            with self.subTest(name=name, invalid=invalid), self.assertRaises(BackendError):
                bridge.state()


    def test_play_ack_waits_for_new_playback_instead_of_old_idle_snapshot(self):
        bridge = self.bridge()
        bridge.allowed_paths = {'/file'}
        bridge.values.update({'pause': False, 'playlist': [{'id': 1, 'filename': '/file'}]})
        bridge.pending, bridge.play_barrier, bridge.target_index = 7, True, 0
        bridge.deadline = time.monotonic() + 5
        bridge.advance()
        self.assertTrue(bridge.waiting_playback)
        self.assertFalse(bridge.replies)
        bridge.values['playlist-pos'] = 0
        bridge.player_message({'event': 'property-change', 'name': 'idle-active', 'data': False})
        while bridge.expected is not None:
            identity, name = bridge.expected
            bridge.player_message({'request_id': identity, 'error': 'success', 'data': bridge.values.get(name)})
        reply = json.loads(bridge.replies[-1])
        self.assertEqual(reply['id'], 7)
        self.assertTrue(reply['state']['playing'])
        self.assertIsNone(bridge.pending)

    def test_play_ack_does_not_accept_old_end_or_intermediate_playlist_end(self):
        bridge = self.bridge()
        bridge.allowed_paths = {'/one', '/two'}
        bridge.values.update({'pause': False, 'playlist': [{'id': 1, 'filename': '/one'},
                                                         {'id': 2, 'filename': '/two'}]})
        bridge.pending, bridge.play_barrier, bridge.target_index = 8, True, 0
        bridge.barrier_serial = 10
        bridge.end_events.append((9, 2, 'eof'))
        bridge.end_events.append((11, 1, 'eof'))
        bridge.advance()
        self.assertTrue(bridge.waiting_playback)
        self.assertFalse(bridge.replies)
        bridge.end_events.append((12, 2, 'eof'))
        bridge.advance()
        self.assertFalse(bridge.waiting_playback)
        self.assertFalse(json.loads(bridge.replies[-1])['state']['playing'])



class Player:
    def __init__(self):
        self.process = subprocess.Popen([sys.executable, str(BACKEND), '--mpv', MPV, '--headless'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            start_new_session=True)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.buffer, self.messages, self.serial = bytearray(), [], 0
        try:
            self.until(lambda message: message.get('event') == 'state')
            children = Path(f'/proc/{self.process.pid}/task/{self.process.pid}/children')
            child_output = (children.read_text() if children.exists() else subprocess.check_output(
                ['pgrep', '-P', str(self.process.pid)], text=True, timeout=2))
            self.child_pids = [int(value) for value in child_output.split()]
            if not self.child_pids or len(self.child_pids) > 4:
                raise AssertionError('Expected a bounded owned mpv child')
        except Exception:
            self.close()
            raise

    def pump(self, timeout=0.1):
        if self.selector.select(timeout):
            chunk = os.read(self.process.stdout.fileno(), 65536)
            if not chunk:
                raise AssertionError('mpv backend exited unexpectedly')
            self.buffer.extend(chunk)
            while b'\n' in self.buffer:
                line, _, remaining = self.buffer.partition(b'\n')
                self.buffer = bytearray(remaining)
                self.messages.append(json.loads(line))

    def until(self, predicate, timeout=5):
        deadline = time.monotonic() + timeout
        while True:
            for index, message in enumerate(self.messages):
                if predicate(message):
                    del self.messages[index]
                    return message
            if time.monotonic() >= deadline:
                raise AssertionError('Expected mpv state did not arrive: ' + repr(self.messages[-4:]))
            self.pump(min(0.1, deadline - time.monotonic()))

    def request(self, command, **params):
        self.serial += 1
        identity = self.serial
        self.process.stdin.write(encoded({'id': identity, 'command': command, **params}))
        self.process.stdin.flush()
        return self.until(lambda message: message.get('id') == identity)

    def drain(self, seconds=0.2):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.pump(min(0.05, deadline - time.monotonic()))
        self.messages.clear()

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=4)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=2)
                raise
        self.selector.close()
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            if not stream.closed:
                stream.close()


@unittest.skipUnless(MPV, 'Install mpv or set MPV to a local executable')
class RealMpvTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='runyte-mpv-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.player = Player()
        self.addCleanup(self.player.close)

    def media(self, name='- 猫.wav', seconds=4):
        path = self.root / name
        with wave.open(str(path), 'wb') as media:
            media.setnchannels(1)
            media.setsampwidth(2)
            media.setframerate(8000)
            media.writeframes(b'\0\0' * int(8000 * seconds))
        return str(path)

    def test_real_load_pause_seek_resume_and_explicit_status(self):
        path = self.media()
        result = self.player.request('load', paths=[path])
        self.assertTrue(result['ok'], result)
        state = self.player.until(lambda item: item.get('event') == 'state' and item['state']['playing'])['state']
        self.assertEqual(state['playlist'][0]['path'], path)
        paused = self.player.request('pause')
        self.assertTrue(paused['ok'])
        self.assertTrue(paused['state']['paused'])
        self.assertFalse(paused['state']['playing'])
        self.assertTrue(self.player.request('seek', seconds=1)['ok'])
        self.assertTrue(self.player.request('play')['ok'])
        current = self.player.request('status')
        self.assertTrue(current['ok'])
        self.assertEqual(current['state']['current'], 0)
        self.assertTrue(self.player.request('stop')['ok'])

    def test_real_add_preserves_pause_and_selected_playlist_navigation(self):
        first, second = self.media('one.wav'), self.media('two.wav')
        self.assertTrue(self.player.request('load', paths=[first])['ok'])
        self.assertTrue(self.player.request('pause')['ok'])
        added = self.player.request('add', paths=[second])
        self.assertTrue(added['ok'], added)
        self.assertTrue(added['state']['paused'])
        self.assertEqual([entry['path'] for entry in added['state']['playlist']], [first, second])
        selected = self.player.request('play', index=1)
        self.assertTrue(selected['ok'], selected)
        self.assertEqual(selected['state']['current'], 1)
        previous = self.player.request('previous')
        self.assertTrue(previous['ok'], previous)
        self.assertEqual(previous['state']['current'], 0)
        self.assertFalse(self.player.request('play', index=64)['ok'])

    def test_real_paused_and_finished_playback_have_no_periodic_output(self):
        path = self.media(seconds=0.3)
        self.assertTrue(self.player.request('load', paths=[path])['ok'])
        self.player.until(lambda item: item.get('event') == 'state'
                          and item['state']['playlist'] and not item['state']['playing'], timeout=5)
        self.player.drain(0.2)
        self.assertFalse(self.player.selector.select(0.7))
        self.assertTrue(self.player.request('play', index=0)['ok'])
        self.assertTrue(self.player.request('pause')['ok'])
        self.player.drain(0.2)
        self.assertFalse(self.player.selector.select(0.7))

    def test_real_automatic_two_track_transition_reports_idle_only_after_final_end(self):
        first, second = self.media('short-one.wav', 0.7), self.media('short-two.wav', 0.7)
        result = self.player.request('load', paths=[first, second])
        self.assertTrue(result['ok'], result)
        observed = [result['state']]
        final = self.player.until(lambda message: message.get('event') == 'state'
            and not message['state']['playing'] and message['state']['current'] is None, timeout=5)
        observed.extend(message['state'] for message in self.player.messages if message.get('event') == 'state')
        observed.append(final['state'])
        self.assertTrue(any(state['playing'] and state['current'] == 1 for state in observed), observed)
        self.assertTrue(all(state['playing'] for state in observed[:-1]), observed)
        self.player.drain(0.2)
        self.assertFalse(self.player.selector.select(0.6))

    def test_real_rejects_nonlocal_inputs_and_extra_commands_without_player_side_effects(self):
        path = self.media()
        for paths in [['https://example.invalid/x'], ['relative.wav'], [str(self.root)], [path] * 65]:
            self.assertFalse(self.player.request('load', paths=paths)['ok'])
        self.assertFalse(self.player.request('run', args=['anything'])['ok'])
        self.assertFalse(self.player.request('pause', extra=True)['ok'])
        self.assertEqual(self.player.request('status')['state']['playlist'], [])

    def test_real_parent_eof_closes_ipc_and_reaps_owned_same_group_child(self):
        self.assertTrue(self.player.child_pids)
        for pid in self.player.child_pids:
            self.assertEqual(os.getpgid(pid), os.getpgid(self.player.process.pid))
        self.player.process.stdin.close()
        self.assertEqual(self.player.process.wait(timeout=4), 0)
        for pid in self.player.child_pids:
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)

    def test_real_slow_stdout_consumer_cannot_pin_parent_eof_cleanup(self):
        path = self.media('a' * 190 + '.wav')
        self.player.serial += 1
        self.player.process.stdin.write(encoded({'id': self.player.serial,
            'command': 'load', 'paths': [path] * 64}))
        self.player.process.stdin.flush()
        # A ~16 KiB response fills a small pipe. Intentionally never drain it;
        # stdin EOF must remain independently selectable during backpressure.
        time.sleep(0.2)
        self.player.process.stdin.close()
        self.assertEqual(self.player.process.wait(timeout=4), 0)
        for pid in self.player.child_pids:
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)

    def test_real_oversized_input_stops_backend_and_reaps_child(self):
        self.player.process.stdin.write(b'x' * MAX_LINE)
        self.player.process.stdin.flush()
        self.assertEqual(self.player.process.wait(timeout=4), 1)
        for pid in self.player.child_pids:
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)


if __name__ == '__main__':
    unittest.main(verbosity=2)
