# SPDX-License-Identifier: MPL-2.0
"""Native local-media playlist backed by one host-managed mpv bridge."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import stat
import sys
import threading
import time

from application import Application, PluginError

EXTENSIONS = {'.wav', '.mp3', '.flac', '.ogg', '.opus', '.m4a', '.aac', '.mp4', '.mkv', '.webm', '.mov'}
LIMIT = 65536


def media_path(root, value):
    if (not isinstance(value, str) or not value or len(value) > 4096
            or '://' in value or any(not c.isprintable() for c in value)):
        raise PluginError('invalid_argument', 'Choose a bounded local media file')
    relative = Path(value)
    if relative.is_absolute() or '..' in relative.parts or relative.suffix.lower() not in EXTENSIONS:
        raise PluginError('unsupported', 'Choose a workspace-relative audio or video file; URLs and playlists are refused')
    try:
        path = (root / relative).resolve(strict=True)
        path.relative_to(root)
        if (not stat.S_ISREG(path.stat().st_mode) or path.suffix.lower() not in EXTENSIONS
                or len(str(path).encode()) > 4096):
            raise ValueError()
    except (OSError, ValueError):
        raise PluginError('invalid_argument', 'Media file must be an ordinary file inside this workspace') from None
    return str(path)


def command(name, description, argument=None, context='view', primary=False):
    result = {'name': name, 'description': description, 'context': context, 'primary': primary}
    if argument:
        result['arguments'] = [{'name': argument, 'type': 'string'}]
    return result


class MediaApplication:
    def __init__(self, app=None, workspace_root=None, backend_path=None, *, mpv='mpv', headless=False, timer_factory=threading.Timer):
        self.root = Path(workspace_root if workspace_root is not None else Path.cwd()).resolve()
        self.backend = Path(backend_path or Path(__file__).with_name('mpv_backend.py')).resolve()
        self.mpv, self.headless, self.timer_factory = mpv, headless, timer_factory
        self.app = app or Application('Local media', [
            command('open', 'Play a workspace-local media file', 'path', 'workspace'),
            command('add', 'Append a workspace-local media file', 'path', 'workspace'),
            command('show', 'Show the retained media playlist', context='workspace'),
            command('play', 'Play the selected item; resume it when paused', primary=True),
            command('pause', 'Pause playback and release its activity lease'),
            command('seek', 'Seek by a relative number of seconds', 'seconds'),
            command('next', 'Select the next item, preserving pause'),
            command('previous', 'Select the previous item, preserving pause'),
            command('stop-playback', 'Stop and close the managed media helper'),
        ], ['views', 'processes', 'activity', 'jobs'])
        self.lock = threading.RLock()
        self.view_lock = threading.Lock()
        self.session = None
        self.serial = self.generation = 0
        self.control_active = False
        self.control_done = threading.Event()
        self.control_done.set()
        self.view = self.revision = None
        self.state = self.empty_state()
        self.message = 'Open a workspace-local media file'
        self.publisher_active = self.publish_pending = False
        self.publisher_idle = threading.Event()
        self.publisher_idle.set()
        self.ended_leases = {}
        self.ended_jobs = {}
        self.pending_control = None
        self.app.handlers = {'open': lambda c: self.enqueue(c, 'load'), 'add': lambda c: self.enqueue(c, 'add'),
                             'show': self.show, **{name: (lambda c, name=name: self.enqueue(c, name))
                             for name in ('play', 'pause', 'seek', 'next', 'previous')},
                             'stop-playback': lambda c: self.enqueue(c, 'stop')}
        self.app.on_event = self.event

    @staticmethod
    def empty_state():
        return {'paused': True, 'playing': False, 'position': 0, 'duration': 0, 'playlist': [], 'current': None}

    def model(self):
        with self.lock:
            state, message = self.state, self.message
            rows = [{'id': self.row_id(item), 'text': Path(item['path']).name,
                     'role': 'heading' if index == state['current'] else 'ordinary'}
                    for index, item in enumerate(state['playlist'])]
            current = state['playlist'][state['current']]['path'] if state['current'] is not None else 'No current item'
            def clock(value):
                return 'unknown' if value is None else f'{value:.1f}'
            mode = ('Playing' if state['playing'] else 'Paused'
                    if state['paused'] and state['current'] is not None else 'Stopped')
            return {'title': 'Local media', 'purpose': 'dashboard', 'rows': rows,
                    'detail': {'text': f'{mode} · {current}\n{clock(state["position"])} / {clock(state["duration"])} seconds', 'role': 'ordinary'},
                    'status': {'text': message, 'role': 'muted'},
                    'actions': ['play', 'pause', 'seek', 'next', 'previous', 'stop-playback']}

    @staticmethod
    def row_id(item):
        return hashlib.sha256((str(item['id']) + '\0' + item['path']).encode()).hexdigest()

    def show(self, context):
        with self.view_lock:
            if self.view is None:
                result = self.app.request('view.create', model=self.model())
                self.view, self.revision = result['view'], result['revision']
            self.app.request('pane.show', invocation=context['invocation'], view=self.view)
        return {}

    def enqueue(self, context, operation):
        with self.lock:
            if self.control_active or self.session is not None and (self.session['closing'] or self.session['releasing']):
                raise PluginError('busy', 'A media control or cleanup is still pending')
            if operation not in ('load', 'add'):
                if self.view != context.get('view') or self.revision != context.get('model_revision'):
                    raise PluginError('stale', 'Media view changed; try again')
                if self.session is None:
                    raise PluginError('closed', 'Open a local media file first')
            params = {}
            if operation in ('load', 'add'):
                params['path'] = context.get('arguments', {}).get('path', '')
            elif operation == 'seek':
                try:
                    value = context.get('arguments', {}).get('seconds', '')
                    if not isinstance(value, str) or len(value) > 32:
                        raise ValueError()
                    seconds = float(value)
                    if not math.isfinite(seconds) or abs(seconds) > 86400:
                        raise ValueError()
                except (ValueError, TypeError):
                    raise PluginError('invalid_argument', 'Seek requires finite relative seconds within one day') from None
                params['seconds'] = seconds
            elif operation == 'play' and context.get('rows'):
                if len(context['rows']) != 1:
                    raise PluginError('invalid_argument', 'Choose one playlist item')
                matches = [index for index, item in enumerate(self.state['playlist'])
                           if self.row_id(item) == context['rows'][0]]
                if len(matches) != 1:
                    raise PluginError('stale', 'Selected playlist item changed')
                if matches[0] != self.state['current']:
                    params['index'] = matches[0]
            self.control_active = True
            intent = {'cancel': threading.Event(), 'job': None, 'session': None}
            self.pending_control = intent
            self.control_done.clear()
        try:
            if operation in ('load', 'add'):
                self.show(context)
            job = self.app.request('job.create', title='Media ' + operation, deadline_seconds=30)['job']
            with self.lock:
                intent['job'] = job
                if job in self.ended_jobs:
                    intent['cancel'].set()
            threading.Thread(target=self.control, args=(operation, params, job, intent), name='runyte-media-control', daemon=True).start()
            return {'job': job}
        except Exception:
            if intent['job'] is not None:
                try:
                    self.app.request('job.finish', job=intent['job'], state='failed')
                except PluginError:
                    pass
            with self.lock:
                self.control_active = False
                self.pending_control = None
                self.control_done.set()
            raise

    def new_session(self):
        with self.lock:
            if self.session is None:
                self.generation += 1
                self.session = {'generation': self.generation, 'process': None, 'subscription': None,
                                'started': threading.Event(), 'ready': threading.Event(), 'release_done': threading.Event(), 'closed': threading.Event(), 'cancel': threading.Event(),
                                'closing': False, 'releasing': False, 'lease': None, 'timer': None, 'duration': 600,
                                'offset': 0, 'partial': bytearray(), 'read_active': False,
                                'read_pending': False, 'resync': False, 'pending': None}
                self.session['release_done'].set()
            return self.session

    def acquire(self, session):
        self.check(session)
        with self.lock:
            if session['lease'] is not None:
                return
        result = self.app.acquire_activity('Local media playback', duration_seconds=600)
        with self.lock:
            session['lease'], session['duration'] = result['lease'], result['duration_seconds']
            if result['lease'] in self.ended_leases:
                session['cancel'].set()
        self.check(session)

    def check(self, session):
        if self.session is not session or session['cancel'].is_set():
            raise PluginError('cancelled', 'Media playback cancelled')

    def start(self, session):
        if session['process'] is not None:
            return
        try:
            self.check(session)
            args = [str(self.backend), '--mpv', self.mpv]
            if self.headless:
                args.append('--headless')
            info = self.app.start_process('Local media bridge', sys.executable, args)
            session['process'] = info['process']
            self.check(session)
            result = self.app.subscribe([{'kind': 'process', 'process': info['process']}],
                lambda event, sequence, data: self.observed(session, event, sequence, data))
            session['subscription'] = result['subscription']
            self.schedule_read(session)
            if not session['ready'].wait(5):
                raise PluginError('timeout', 'Media backend readiness timed out')
            self.check(session)
        finally:
            session['started'].set()

    def control(self, operation, params, job, intent):
        session = None
        state = 'failed'
        try:
            if intent['cancel'].is_set():
                raise PluginError('cancelled', 'Media command cancelled')
            if operation in ('load', 'add'):
                path = media_path(self.root, params.pop('path'))
                params['paths'] = [path]
                with self.lock:
                    paths = [item['path'] for item in self.state['playlist']] if operation == 'add' else []
                if len(paths) >= 64 or sum(len(item.encode()) for item in paths + [path]) > 16384:
                    raise PluginError('limit_exceeded', 'Playlist exceeds 64 files or 16 KiB of paths')
            with self.lock:
                if intent['cancel'].is_set():
                    raise PluginError('cancelled', 'Media command cancelled')
                session = self.new_session()
                intent['session'] = session
            if operation == 'stop':
                self.close(session)
                if not session['closed'].wait(8):
                    raise PluginError('timeout', 'Media helper cleanup is still pending')
            else:
                if operation in ('load', 'add', 'play', 'next', 'previous'):
                    self.acquire(session)
                self.start(session)
                self.rpc(session, operation, params)
            state = 'succeeded'
        except Exception:
            if intent['cancel'].is_set():
                state = 'cancelled'
            if session is not None:
                self.close(session)
            with self.lock:
                self.message = ('Media control failed; helper cleanup requested' if session is not None
                                else 'Media control refused; choose a valid local file or argument')
        finally:
            if session is not None:
                session['started'].set()
            try:
                self.app.request('job.finish', job=job, state=state)
            except PluginError as error:
                if state == 'succeeded' and error.code == 'cancelled':
                    try:
                        self.app.request('job.finish', job=job, state='cancelled')
                    except PluginError:
                        pass
            with self.lock:
                self.control_active = False
                self.pending_control = None
                self.control_done.set()
            self.schedule_publish()

    def rpc(self, session, operation, params):
        with self.lock:
            self.check(session)
            self.serial += 1
            pending = {'id': self.serial, 'event': threading.Event(), 'reply': None}
            session['pending'] = pending
        try:
            data = (json.dumps({'id': pending['id'], 'command': operation, **params}, allow_nan=False) + '\n').encode()
            if len(data) > LIMIT:
                raise PluginError('limit_exceeded', 'Media command exceeds helper limit')
            self.app.write_process(session['process'], data)
            if not pending['event'].wait(5):
                raise PluginError('timeout', 'Media helper acknowledgement timed out')
            self.check(session)
            if pending['reply'] is None or pending['reply'].get('ok') is not True:
                raise PluginError('unavailable', 'Media helper refused the control')
        finally:
            with self.lock:
                if session['pending'] is pending:
                    session['pending'] = None

    def observed(self, session, event, _sequence, _data):
        with self.lock:
            if self.session is not session or session['closing']:
                return
            if event == 'event.resync_required':
                session['resync'] = True
        self.schedule_read(session)

    def schedule_read(self, session):
        with self.lock:
            if self.session is not session or session['closing']:
                return
            session['read_pending'] = True
            if session['read_active']:
                return
            session['read_active'] = True
        threading.Thread(target=self.read, args=(session,), name='runyte-media-output', daemon=True).start()

    def read(self, session):
        try:
            while True:
                with self.lock:
                    if session['closing'] or self.session is not session:
                        return
                    session['read_pending'] = False
                    resync, session['resync'] = session['resync'], False
                if resync and session['subscription'] is not None:
                    self.app.resync(session['subscription'])
                process = session['process']
                info = self.app.request('process.get', process=process)
                if info['output_truncated'] or info['stdout']['start'] > session['offset']:
                    raise PluginError('stale', 'Media helper output was truncated')
                while session['offset'] < info['stdout']['end']:
                    chunk = self.app.read_process(process, 'stdout', session['offset'], LIMIT)
                    if not chunk['data']:
                        raise PluginError('unavailable', 'Media helper output stalled')
                    session['offset'] = chunk['next']
                    session['partial'].extend(chunk['data'])
                    while b'\n' in session['partial']:
                        line, _, remaining = session['partial'].partition(b'\n')
                        if len(line) > LIMIT:
                            raise PluginError('limit_exceeded', 'Media helper line exceeds limit')
                        session['partial'] = bytearray(remaining)
                        self.received(session, json.loads(line))
                    if len(session['partial']) > LIMIT:
                        raise PluginError('limit_exceeded', 'Media helper line exceeds limit')
                if info['state'] != 'running':
                    raise PluginError('closed', 'Media helper exited')
                with self.lock:
                    if not session['read_pending']:
                        return
        except Exception:
            self.close(session)
        finally:
            with self.lock:
                session['read_active'] = False
                again = session['read_pending'] and not session['closing'] and self.session is session
            if again:
                self.schedule_read(session)

    def validate_state(self, state):
        if not isinstance(state, dict) or type(state.get('paused')) is not bool or type(state.get('playing')) is not bool:
            raise ValueError('Invalid media state')
        playlist = state.get('playlist')
        if not isinstance(playlist, list) or len(playlist) > 64:
            raise ValueError('Invalid playlist')
        paths, ids = 0, set()
        for item in playlist:
            if not isinstance(item, dict) or not isinstance(item.get('id'), (str, int)) or isinstance(item.get('id'), bool):
                raise ValueError('Invalid playlist item')
            path = item.get('path')
            if not isinstance(path, str) or any(not c.isprintable() for c in path):
                raise ValueError('Invalid playlist path')
            Path(path).relative_to(self.root)
            if '..' in Path(path).parts or not Path(path).is_absolute() or Path(path).suffix.lower() not in EXTENSIONS:
                raise ValueError('Invalid playlist path')
            paths += len(path.encode())
            if len(path.encode()) > 4096 or len(str(item['id'])) > 128 or str(item['id']) in ids:
                raise ValueError('Invalid playlist identity')
            ids.add(str(item['id']))
        current = state.get('current')
        if paths > 16384 or current is not None and (type(current) is not int or not 0 <= current < len(playlist)):
            raise ValueError('Invalid current item')
        for key in ('position', 'duration'):
            value = state.get(key)
            if value is not None and (isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0):
                raise ValueError('Invalid playback time')
        if state['playing'] and (state['paused'] or current is None):
            raise ValueError('Invalid playback state')
        return state

    def received(self, session, message):
        with self.lock:
            if self.session is not session or session['closing']:
                return
            pending = session['pending']
            is_reply = pending is not None and message.get('id') == pending['id']
            if message.get('event') != 'state' and not is_reply:
                return
            if is_reply and message.get('ok') is True and 'state' not in message:
                raise ValueError('Missing acknowledged media state')
            if 'state' in message:
                candidate = self.validate_state(message['state'])
                if candidate['playing'] and session['lease'] is None:
                    raise ValueError('Unprotected playback')
                self.state = candidate
                session['ready'].set()
                self.message = 'Playback active' if self.state['playing'] else 'Playback paused or stopped'
            if is_reply:
                pending['reply'] = message
                pending['event'].set()
        self.schedule_publish()

    def schedule_publish(self):
        with self.lock:
            self.publish_pending = True
            if self.publisher_active:
                return
            self.publisher_active = True
            self.publisher_idle.clear()
        threading.Thread(target=self.publish, name='runyte-media-view', daemon=True).start()

    def publish(self):
        try:
            while True:
                with self.lock:
                    self.publish_pending = False
                    session = self.session
                    playing = self.state['playing']
                    may_release = not self.control_active and not playing
                if session is not None and not session['closing']:
                    if may_release:
                        self.release(session)
                    elif playing:
                        self.arm_renewal(session)
                with self.view_lock:
                    if self.view is not None:
                        model = self.model()
                        if model != getattr(self, 'published_model', None):
                            view = self.view
                            for attempt in range(3):
                                try:
                                    if self.view != view:
                                        break
                                    result = self.app.publish_model(view, self.revision, model)
                                    with self.lock:
                                        if self.view == view:
                                            self.revision, self.published_model = result['revision'], model
                                    break
                                except PluginError as error:
                                    if error.code != 'busy' or attempt == 2:
                                        break
                                    time.sleep(0.1)
                with self.lock:
                    if not self.publish_pending:
                        self.publisher_active = False
                        self.publisher_idle.set()
                        return
        except Exception:
            with self.lock:
                self.publisher_active = False
                self.publisher_idle.set()

    def arm_renewal(self, session):
        with self.lock:
            if (session['timer'] is not None or session['lease'] is None or session['closing']
                    or self.session is not session or not self.state['playing']):
                return
            timer = self.timer_factory(max(1, session['duration'] * 0.9), lambda: self.renew(session))
            timer.daemon = True
            session['timer'] = timer
        timer.start()

    def renew(self, session):
        with self.lock:
            if self.session is not session or session['closing'] or session['lease'] is None or not self.state['playing']:
                return
            lease = session['lease']
        try:
            result = self.app.renew_activity(lease, duration_seconds=600)
            with self.lock:
                if session['lease'] != lease or session['closing']:
                    return
                session['duration'] = result['duration_seconds']
                session['timer'] = None
            self.arm_renewal(session)
        except Exception:
            with self.lock:
                current = session['lease'] == lease
            if current:
                self.close(session)

    def release(self, session):
        with self.lock:
            if self.control_active and not session['closing'] or session['releasing']:
                return
            lease = session['lease']
            if lease is None:
                return
            session['releasing'] = True
            session['release_done'].clear()
            if session['timer'] is not None:
                session['timer'].cancel()
                session['timer'] = None
        try:
            self.app.release_activity(lease)
            with self.lock:
                if session['lease'] == lease:
                    session['lease'] = None
        except Exception:
            self.close(session)
            raise
        finally:
            with self.lock:
                session['releasing'] = False
                session['release_done'].set()

    def close(self, session):
        with self.lock:
            if session['closing']:
                return
            session['closing'] = True
            session['cancel'].set()
            session['ready'].set()
            if session['timer'] is not None:
                session['timer'].cancel()
                session['timer'] = None
            if session['pending'] is not None:
                session['pending']['event'].set()
        threading.Thread(target=self.cleanup, args=(session,), name='runyte-media-cleanup', daemon=True).start()

    def cleanup(self, session):
        try:
            if not session['started'].wait(10):
                raise PluginError('timeout', 'Media startup still owns cleanup')
            if session['process'] is not None:
                self.app.request('process.close', process=session['process'])
            if session['subscription'] is not None:
                self.app.unsubscribe(session['subscription'])
            if not session['release_done'].wait(10):
                raise PluginError('timeout', 'Activity release is still pending')
            self.release(session)
            if session['lease'] is not None:
                raise PluginError('unavailable', 'Activity release is still pending')
            with self.lock:
                if self.session is session:
                    self.session = None
                    self.state = self.empty_state()
                    self.message = 'Playback stopped; helper closed'
            session['closed'].set()
            self.schedule_publish()
        except Exception:
            # Keep ownership and the lease: its cancellation deadline lets the
            # host tear down the plugin and remaining managed child together.
            with self.lock:
                self.message = 'Media cleanup pending; stop the plugin if it does not settle'
            self.schedule_publish()

    def event(self, name, data):
        if name == 'view.closed':
            with self.lock:
                if data.get('view') == self.view:
                    self.view = self.revision = None
                    self.published_model = None
        elif (name == 'job.cancel_requested' or name == 'job.changed'
              and data.get('state') in ('cancelled', 'failed', 'outcome_unknown')):
            with self.lock:
                job = data.get('job')
                self.ended_jobs[job] = True
                while len(self.ended_jobs) > 16:
                    del self.ended_jobs[next(iter(self.ended_jobs))]
                intent = self.pending_control
                if intent is None or intent['job'] != job:
                    return
                intent['cancel'].set()
                session = intent['session']
            if session is not None:
                self.close(session)
        elif name == 'activity.cancel_requested':
            with self.lock:
                lease = data.get('lease')
                self.ended_leases[lease] = True
                while len(self.ended_leases) > 16:
                    del self.ended_leases[next(iter(self.ended_leases))]
                session = self.session
                if session is None or session['lease'] != lease:
                    return
            self.close(session)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mpv', default='mpv', help='Local mpv executable')
    parser.add_argument('--headless', action='store_true', help='Use null audio/video outputs for deterministic tests')
    options = parser.parse_args()
    MediaApplication(mpv=options.mpv, headless=options.headless).app.run()


if __name__ == '__main__':
    main()
