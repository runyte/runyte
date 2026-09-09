# SPDX-License-Identifier: MPL-2.0
"""Native controller for a deterministic host-managed echo helper."""
import concurrent.futures
from pathlib import Path
import sys
import threading
import time
from application import Application, PluginError

app = Application('Helper', [
    {'name': 'open', 'description': 'Open the managed helper', 'context': 'workspace'},
    {'name': 'send', 'description': 'Send a line to the helper', 'context': 'view'},
    {'name': 'flood', 'description': 'Generate bounded retained output', 'context': 'view'},
    {'name': 'eof', 'description': 'Close helper input', 'context': 'view'},
    {'name': 'close', 'description': 'Stop and reap the helper', 'context': 'view'},
], ['views', 'interaction', 'processes'])
lock = threading.RLock()
worker = concurrent.futures.ThreadPoolExecutor(max_workers=1)
view = revision = process = subscription = pending_input = None
refresh_pending = refresh_running = False
last_text = 'Starting helper…'


def safe_text(data):
    # Output is data. Never interpret escape sequences or inject control cells.
    return ''.join(c if c == '\n' or c.isprintable() else f'\\u{ord(c):04x}'
                   for c in data.decode('utf-8', errors='replace'))


def model(text, state):
    return {'title': 'Managed helper', 'purpose': 'dashboard', 'rows': [],
            'detail': {'text': text or '(No output)', 'role': 'ordinary'},
            'status': {'text': f'{state} · Tab offers Send, Flood, EOF and Close', 'role': 'muted'},
            'actions': ['send', 'flood', 'eof', 'close']}


def schedule_refresh():
    global refresh_pending, refresh_running
    refresh_pending = True
    if not refresh_running:
        refresh_running = True
        worker.submit(refresh)


def observed(event, sequence, data):
    with lock:
        if event == 'event.resync_required':
            app.resync(data['subscription'])
        elif process is not None:
            schedule_refresh()


def refresh():
    global refresh_pending, refresh_running, revision, last_text
    while True:
        # Group only pending changes; no sleeping worker or timer when idle.
        time.sleep(0.12)
        with lock:
            if not refresh_pending or view is None or process is None:
                refresh_pending = refresh_running = False
                return
            refresh_pending = False
            try:
                info = app.request('process.get', process=process)
                bounds = info['stdout']
                offset = max(bounds['start'], bounds['end'] - 4096)
                output = app.read_process(process, 'stdout', offset, 4096)
                text = safe_text(output['data'])
                state = info['state'] + (' · final output truncated' if info['output_truncated'] else '')
                candidate = model(text, state)
                # A source event may arrive after a foreground publication.
                # Retry only a pacing refusal, never an uncertain mutation.
                for attempt in range(3):
                    try:
                        result = app.publish_model(view, revision, candidate)
                        revision, last_text = result['revision'], text
                        break
                    except PluginError as error:
                        if error.code != 'busy' or attempt == 2:
                            raise
                        time.sleep(0.12)
            except PluginError as error:
                # Eviction between get and read is an ordinary stale cursor.
                if error.code == 'stale':
                    refresh_pending = True
            if not refresh_pending:
                refresh_running = False
                return


def open_view(context):
    global view, revision, process, subscription
    with lock:
        if process is None:
            info = app.start_process('Echo helper', sys.executable,
                                     [str(Path(__file__).with_name('echo_helper.py'))])
            process = info['process']
            subscription = app.subscribe([{'kind': 'process', 'process': process}], observed)['subscription']
        if view is None:
            result = app.request('view.create', model=model(last_text, 'running'))
            view, revision = result['view'], result['revision']
        app.request('pane.show', invocation=context['invocation'], view=view)
        schedule_refresh()


def require_process(context):
    if view != context['view'] or revision != context['model_revision']:
        raise PluginError('stale', 'Helper view changed; try again')
    if process is None:
        raise PluginError('closed', 'Helper is closed; run Open to start again')


def send(context):
    global pending_input
    with lock:
        require_process(context)
        result = app.request('ui.prompt', invocation=context['invocation'], title='Send to helper',
                             field={'id': 'line', 'label': 'Line', 'kind': 'text', 'maximum_length': 1024})
        pending_input = (result['surface'], process)


def submitted(context):
    global pending_input
    with lock:
        if pending_input is None or context['surface'] != pending_input[0]:
            return
        _, target = pending_input
        pending_input = None
        if context['accepted'] and target == process:
            app.write_process(target, (context['values']['line'] + '\n').encode('utf-8'))


def write(context, data, eof=False):
    with lock:
        require_process(context)
        app.write_process(process, data, eof=eof)


def close(context):
    global process, subscription, revision
    with lock:
        require_process(context)
        app.request('process.close', process=process)
        process = None
        if subscription is not None:
            app.unsubscribe(subscription)
            subscription = None
        # A foreground action is useful even if the last output update just ran.
        for attempt in range(3):
            try:
                result = app.publish_model(view, revision, model(last_text, 'closed'))
                revision = result['revision']
                break
            except PluginError as error:
                if error.code != 'busy' or attempt == 2:
                    raise
                time.sleep(0.12)


def event(name, data):
    global view, revision, pending_input, refresh_pending
    with lock:
        if name == 'view.closed' and data['view'] == view:
            view = revision = pending_input = None
            refresh_pending = False


app.handlers = {'open': open_view, 'send': send, 'flood': lambda context: write(context, b'flood\n'),
                'eof': lambda context: write(context, b'', True), 'close': close}
app.on_input, app.on_event = submitted, event
if __name__ == '__main__':
    try:
        app.run()
    finally:
        with lock:
            refresh_pending = False
        worker.shutdown(wait=True, cancel_futures=True)
