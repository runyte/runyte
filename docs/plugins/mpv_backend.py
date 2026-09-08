# SPDX-License-Identifier: MPL-2.0
"""Bounded JSON-lines bridge to an owned mpv child; no timers while quiescent.

stdin: {id: positive integer, command: load/add, paths: [absolute local files]}
or play(index optional), pause, next, previous, seek(seconds relative), stop,
status. Replies contain id, ok, and state or a static error. Unsolicited state
has event='state'. This local-file controller is not a decoder sandbox.
"""
import argparse
from collections import deque
import json
import math
import os
from pathlib import Path
import selectors
import socket
import stat
import subprocess
import sys
import time
import unicodedata

MAX_LINE = 65536
MAX_PATH_BYTES = 16384
MAX_PLAYLIST = 64
MAX_OUTPUT = 256 * 1024
PROPERTIES = ('pause', 'time-pos', 'duration', 'playlist', 'playlist-pos', 'idle-active')


class BackendError(Exception):
    pass


def encoded(value):
    data = json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(',', ':')).encode() + b'\n'
    if len(data) > MAX_LINE:
        raise BackendError('Media message exceeds its limit')
    return data


def local_paths(paths):
    if not isinstance(paths, list) or not 1 <= len(paths) <= MAX_PLAYLIST:
        raise BackendError('Choose one to 64 local media files')
    result = []
    for value in paths:
        if (not isinstance(value, str) or not os.path.isabs(value)
                or any(unicodedata.category(c) == 'Cc' for c in value)):
            raise BackendError('Media paths must name absolute local files')
        try:
            if not 0 < len(value.encode()) <= 4096:
                raise ValueError()
            path = Path(value).resolve(strict=True)
            if not stat.S_ISREG(path.stat().st_mode):
                raise ValueError()
            result.append(str(path))
        except (OSError, RuntimeError, ValueError, UnicodeError):
            raise BackendError('Media path is not an available ordinary local file') from None
    if sum(len(path.encode()) for path in result) > MAX_PATH_BYTES:
        raise BackendError('Media playlist paths exceed their combined limit')
    return result


class Bridge:
    def __init__(self, ipc, input_fd, output_fd):
        self.ipc, self.input, self.output = ipc, input_fd, output_fd
        self.selector = selectors.DefaultSelector()
        for descriptor in (ipc.fileno(), input_fd, output_fd):
            os.set_blocking(descriptor, False)
        self.selector.register(input_fd, selectors.EVENT_READ, 'input')
        self.selector.register(ipc, selectors.EVENT_READ, 'ipc')
        self.input_bytes, self.ipc_bytes, self.ipc_output = bytearray(), bytearray(), bytearray()
        self.replies, self.output_bytes = deque(), 0
        self.writing = b''
        self.writing_reply = False
        self.latest_state = None
        self.values = {}
        self.allowed_paths = set()
        self.pending = None
        self.pending_error = None
        self.play_barrier = False
        self.waiting_playback = False
        self.end_events = deque(maxlen=64)
        self.event_serial = 0
        self.barrier_serial = 0
        self.target_index = None
        self.serial = 0
        self.expected = None
        self.actions = deque()
        self.dirty, self.last_state = False, None
        self.last_emission = -float('inf')
        self.running = True
        self.initializing = True
        self.deadline = time.monotonic() + 5
        self.actions.extend((['observe_property', number, name], None)
                            for number, name in enumerate(PROPERTIES, 1))
        self.actions.extend((['get_property', name], name) for name in PROPERTIES)
        self.advance()

    def write_interest(self):
        wants = bool(self.writing or self.replies or self.latest_state)
        try:
            self.selector.get_key(self.output)
        except KeyError:
            if wants:
                self.selector.register(self.output, selectors.EVENT_WRITE, 'output')
        else:
            if not wants:
                self.selector.unregister(self.output)
        self.selector.modify(self.ipc, selectors.EVENT_READ |
                             (selectors.EVENT_WRITE if self.ipc_output else 0), 'ipc')

    def queue_reply(self, message):
        data = encoded(message)
        if (len(self.replies) + int(self.writing_reply) >= 16
                or self.output_bytes + len(self.writing) + len(self.latest_state or b'') + len(data) > MAX_OUTPUT):
            raise BackendError('Media controller stopped: output consumer is too slow')
        self.replies.append(data)
        self.output_bytes += len(data)
        self.write_interest()

    def state(self):
        playlist = self.values.get('playlist')
        if (type(self.values.get('pause')) is not bool
                or type(self.values.get('idle-active')) is not bool
                or type(self.values.get('playlist-pos')) is not int):
            raise BackendError('mpv playback state is unavailable')
        if not isinstance(playlist, list) or len(playlist) > MAX_PLAYLIST:
            raise BackendError('mpv playlist exceeds its limit')
        entries = []
        for entry in playlist:
            if not isinstance(entry, dict):
                raise BackendError('Invalid mpv playlist')
            path, identity = entry.get('filename'), entry.get('id')
            if (path not in self.allowed_paths or type(identity) is not int
                    or not 0 <= identity <= 2**63 - 1):
                raise BackendError('mpv playlist contains an unexpected file')
            entries.append({'id': identity, 'path': path})
        current = self.values.get('playlist-pos')
        if type(current) is not int or not 0 <= current < len(entries):
            current = None
        def number(name):
            value = self.values.get(name)
            return value if type(value) in (float, int) and math.isfinite(value) and value >= 0 else None
        paused = self.values['pause']
        return {'paused': paused, 'position': number('time-pos'), 'duration': number('duration'),
                'playlist': entries, 'current': current,
                'playing': current is not None and not paused and self.values.get('idle-active') is False}

    def publish(self, now):
        if not self.dirty or self.pending is not None or self.initializing:
            return
        state = self.state()
        if not state['paused'] and not state['playing'] and state['playlist']:
            last = state['playlist'][-1]['id']
            current_ids = {entry['id'] for entry in state['playlist']}
            settled = (self.values['idle-active'] and state['current'] is None
                       and any(serial > self.barrier_serial and (entry == last or reason == 'stop' and entry in current_ids)
                               for serial, entry, reason in self.end_events))
            if not settled:
                # Intermediate EOF/property updates may briefly lose the current
                # index while mpv is advancing. Keep playback protection until
                # a final end is established; wait for events without polling.
                return
        if state == self.last_state:
            self.dirty = False
            return
        if state['playing'] and now - self.last_emission < 0.5:
            return
        data = encoded({'event': 'state', 'state': state})
        if self.output_bytes + len(self.writing) + len(data) > MAX_OUTPUT:
            raise BackendError('Media controller stopped: output consumer is too slow')
        self.latest_state = data
        self.last_state, self.last_emission, self.dirty = state, now, False
        self.write_interest()

    def advance(self):
        if self.actions:
            command, property_name = self.actions.popleft()
            self.serial += 1
            self.expected = (self.serial, property_name)
            self.ipc_output.extend(encoded({'command': command, 'request_id': self.serial}))
            self.write_interest()
        else:
            self.expected = None
            state = self.state()
            if self.pending is not None and self.play_barrier and self.pending_error is None and not state['playing']:
                target = (state['playlist'][self.target_index]['id']
                          if self.target_index is not None and self.target_index < len(state['playlist']) else None)
                last = state['playlist'][-1]['id'] if state['playlist'] else None
                settled_end = (self.values['idle-active'] and state['current'] is None
                               and any(serial > self.barrier_serial and
                                       (entry == last or reason == 'stop' and entry == target)
                                       for serial, entry, reason in self.end_events))
                if not settled_end:
                    # mpv's load command only schedules playback. An old idle
                    # snapshot cannot authorize releasing the activity lease.
                    self.waiting_playback = True
                    return
            self.deadline = None
            self.play_barrier, self.waiting_playback = False, False
            if self.pending is not None:
                response = {'id': self.pending, 'ok': self.pending_error is None, 'state': state}
                if self.pending_error is not None:
                    response['error'] = self.pending_error
                self.queue_reply(response)
                self.pending, self.pending_error = None, None
            self.allowed_paths = {entry['path'] for entry in state['playlist']}
            self.initializing = False
            self.dirty = True

    def request(self, value):
        identity = value.get('id') if isinstance(value, dict) else None
        if type(identity) is not int or not 1 <= identity <= 2**31 - 1:
            raise BackendError('Invalid media request identifier')
        if self.pending is not None or self.initializing:
            self.queue_reply({'id': identity, 'ok': False, 'error': 'Media controller is busy'})
            return
        try:
            command = value.get('command')
            extra = {'paths'} if command in ('load', 'add') else {'seconds'} if command == 'seek' else {'index'} if command == 'play' else set()
            if set(value) - {'id', 'command'} - extra:
                raise BackendError('Invalid media request fields')
            actions = []
            state = self.state()
            if command in ('load', 'add'):
                paths = local_paths(value.get('paths'))
                old_paths = [item['path'] for item in state['playlist']] if command == 'add' else []
                prospective = old_paths + paths
                if len(prospective) > MAX_PLAYLIST or sum(len(path.encode()) for path in prospective) > MAX_PATH_BYTES:
                    raise BackendError('Media playlist exceeds its combined limit')
                # All possible acknowledged state messages must fit before the
                # first player mutation, including JSON path escaping and IDs.
                encoded({'id': identity, 'ok': True, 'state': {**state, 'playlist':
                         [{'id': 2**63 - 1, 'path': path} for path in prospective]}})
                self.allowed_paths = set(prospective) | {item["path"] for item in state["playlist"]}
                if command == 'load':
                    actions.extend([(['stop'], None), (['playlist-clear'], None)])
                actions.extend((['loadfile', path, 'replace' if command == 'load' and index == 0 else 'append'], None)
                               for index, path in enumerate(paths))
                if command == 'load':
                    actions.append((['set_property', 'pause', False], None))
            elif command == 'play':
                index = value.get('index', state['current'] if state['current'] is not None else 0)
                if type(index) is not int or not 0 <= index < len(state['playlist']):
                    raise BackendError('Choose a current playlist entry')
                if 'index' in value or state['current'] is None:
                    actions.append((['playlist-play-index', index], None))
                actions.append((['set_property', 'pause', False], None))
            elif command == 'pause':
                actions.append((['set_property', 'pause', True], None))
            elif command == 'seek':
                seconds = value.get('seconds')
                if type(seconds) not in (int, float) or not math.isfinite(seconds) or not -86400 <= seconds <= 86400:
                    raise BackendError('Seek must be a finite relative number of seconds')
                actions.append((['seek', seconds, 'relative+exact'], None))
            elif command in ('next', 'previous'):
                actions.append((['playlist-next' if command == 'next' else 'playlist-prev', 'weak'], None))
            elif command == 'stop':
                actions.append((['stop'], None))
            elif command != 'status':
                raise BackendError('Unknown media command')
            self.play_barrier = command in ('load', 'play') or command in ('next', 'previous') and not state['paused']
            self.barrier_serial = self.event_serial
            self.target_index = (0 if command == 'load' else value.get('index', state['current'] or 0)
                                 if command == 'play' else min(len(state['playlist']) - 1, (state['current'] or 0) + 1)
                                 if command == 'next' else max(0, (state['current'] or 0) - 1))
            self.pending = identity
            # An unstarted older snapshot must never follow the new command ack.
            # A partially written prior snapshot remains before that ack in FIFO.
            self.latest_state = None
            self.actions.extend(actions)
            self.actions.extend((['get_property', name], name) for name in PROPERTIES)
            self.deadline = time.monotonic() + 5
            self.advance()
        except BackendError as error:
            self.queue_reply({'id': identity, 'ok': False, 'error': str(error)})

    def player_message(self, value):
        if not isinstance(value, dict):
            raise BackendError('Invalid mpv response')
        if value.get('event') in ('end-file', 'file-loaded', 'playback-restart'):
            self.event_serial += 1
            if value.get('event') == 'end-file':
                self.end_events.append((self.event_serial, value.get('playlist_entry_id'), value.get('reason')))
            self.refresh_waiting_playback()
            return
        if value.get('event') == 'property-change':
            name = value.get('name')
            if name in PROPERTIES:
                self.values[name] = value.get('data')
                self.dirty = True
                self.refresh_waiting_playback()
            return
        if 'request_id' not in value:
            return
        if self.expected is None or value['request_id'] != self.expected[0]:
            raise BackendError('Unexpected mpv response identifier')
        name = self.expected[1]
        if name is not None:
            if value.get('error') != 'success' and name not in ('time-pos', 'duration'):
                raise BackendError('mpv state could not be established')
            self.values[name] = value.get('data') if value.get('error') == 'success' else None
        elif value.get('error') != 'success':
            if self.pending is None:
                raise BackendError('mpv could not initialize property observations')
            self.pending_error = 'mpv could not complete the media command'
            self.actions.clear()
            self.actions.extend((['get_property', name], name) for name in PROPERTIES)
        self.advance()

    def refresh_waiting_playback(self):
        if self.waiting_playback:
            self.waiting_playback = False
            self.actions.extend((['get_property', name], name) for name in PROPERTIES)
            self.advance()

    @staticmethod
    def parse(data):
        try:
            return json.loads(data, parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
        except (ValueError, UnicodeError, RecursionError):
            raise BackendError('Invalid media JSON message') from None

    def read(self, descriptor, buffer, handler):
        try:
            data = os.read(descriptor, 4096)
        except BlockingIOError:
            return
        if not data:
            self.running = False
            return
        buffer.extend(data)
        while b'\n' in buffer:
            offset = buffer.index(b'\n')
            if offset + 1 > MAX_LINE:
                raise BackendError('Media message exceeds its limit')
            line = bytes(buffer[:offset])
            del buffer[:offset + 1]
            handler(self.parse(line))
        if len(buffer) >= MAX_LINE:
            raise BackendError('Media message exceeds its limit')

    def write(self):
        if not self.writing:
            if self.replies:
                self.writing = self.replies.popleft()
                self.writing_reply = True
                self.output_bytes -= len(self.writing)
            elif self.latest_state:
                self.writing, self.latest_state = self.latest_state, None
                self.writing_reply = False
        if self.writing:
            try:
                self.writing = self.writing[os.write(self.output, self.writing):]
            except BlockingIOError:
                pass
        if not self.writing:
            self.writing_reply = False
        self.write_interest()

    def run(self):
        while self.running:
            now = time.monotonic()
            self.publish(now)
            timers = [self.deadline] if self.deadline is not None else []
            if self.dirty and not self.initializing and self.pending is None and self.state()['playing']:
                timers.append(max(now, self.last_emission + 0.5))
            timeout = max(0, min(timers) - now) if timers else None
            for key, mask in self.selector.select(timeout):
                if key.data == 'input':
                    self.read(self.input, self.input_bytes, self.request)
                elif key.data == 'output':
                    self.write()
                else:
                    if mask & selectors.EVENT_READ:
                        self.read(self.ipc.fileno(), self.ipc_bytes, self.player_message)
                    if mask & selectors.EVENT_WRITE and self.ipc_output:
                        try:
                            sent = self.ipc.send(self.ipc_output)
                            del self.ipc_output[:sent]
                        except BlockingIOError:
                            pass
                        self.write_interest()
            if self.deadline is not None and time.monotonic() >= self.deadline:
                raise BackendError('mpv command timed out; controller stopped')

    def close(self):
        self.selector.close()
        self.ipc.close()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mpv', default='mpv', help='mpv executable (no shell)')
    parser.add_argument('--headless', action='store_true', help='Use null audio/video outputs for tests')
    args = parser.parse_args(argv)
    parent, child = socket.socketpair()
    process, bridge = None, None
    try:
        flags = ['--no-config', '--load-scripts=no', '--autoload-files=no', '--access-references=no',
                 '--ytdl=no', '--terminal=no', '--input-terminal=no', '--input-default-bindings=no',
                 '--osc=no', '--input-vo-keyboard=no', '--input-cursor=no', '--input-media-keys=no',
                 '--drag-and-drop=no', '--idle=yes', '--pause=yes',
                 '--input-ipc-client=fd://' + str(child.fileno())]
        if args.headless:
            flags.extend(['--ao=null', '--vo=null'])
        process = subprocess.Popen([args.mpv, *flags], stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, pass_fds=(child.fileno(),))
        # Inherit the managed helper's process group. Host stop kills every
        # descendant, including during a blocked decoder or backend shutdown.
        child.close()
        bridge = Bridge(parent, sys.stdin.fileno(), sys.stdout.fileno())
        bridge.run()
        return 0
    except (BackendError, OSError, ValueError):
        # Never reflect mpv diagnostics, executable paths or arbitrary JSON.
        return 1
    finally:
        if bridge is not None:
            bridge.close()
        else:
            parent.close()
        child.close()
        if process is not None:
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                process.terminate()
                try:
                    process.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


if __name__ == '__main__':
    raise SystemExit(main())
