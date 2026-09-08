# SPDX-License-Identifier: MPL-2.0
"""Explicit asynchronous filtering and viewport/action observations; no network."""
import concurrent.futures
import threading
import time
from application import Application, PluginError

app = Application('Catalog', [
    {'name': 'open', 'description': 'Open the searchable catalog', 'context': 'workspace'},
    {'name': 'filter', 'description': 'Enter a catalog query', 'context': 'view'},
    {'name': 'refresh', 'description': 'Retry the current catalog query', 'context': 'view'},
    {'name': 'inspect', 'description': 'Inspect the selected record', 'context': 'view', 'primary': True},
    {'name': 'observations', 'description': 'Show the last viewport and accepted action', 'context': 'view'},
], ['views', 'interaction'])
lock = threading.RLock()
worker = concurrent.futures.ThreadPoolExecutor(max_workers=1)
records = tuple({'id': f'record-{i:04}', 'text': f'Record {i:04} · Topic {i % 37:02} · é猫',
                 'role': 'ordinary'} for i in range(5000))
view = revision = query = current = subscription = None
pending_input = None
pending_search = None
search_running = False
last_scheduled = None
last_viewport = last_action = None


def model(text):
    rows = [row for row in records if text.casefold() in row['text'].casefold()]
    return {'title': 'Catalog', 'purpose': 'list', 'rows': rows,
            'status': {'text': f'{len(rows)} matches · Tab offers Filter and Refresh', 'role': 'ordinary'},
            'detail': {'text': f'Query: {text or "(all)"}\nOnly Enter submits a query. Previous rows remain visible during lookup.', 'role': 'muted'},
            'actions': ['filter', 'refresh', 'inspect', 'observations']}


def open_view(context):
    global view, revision, current, subscription
    with lock:
        if view is None:
            current = model('')
            result = app.request('view.create', model=current)
            view, revision = result['view'], result['revision']
            subscription = app.subscribe([
                {'kind': 'view', 'view': view},
                {'kind': 'viewport', 'view': view, 'pane': context['pane']},
                {'kind': 'view_actions', 'view': view}], observed)['subscription']
        app.request('pane.show', invocation=context['invocation'], view=view)


def captured(context):
    token = query['revision'] if query else None
    if (context['view'] != view or context['model_revision'] != revision
            or context.get('query_revision') != token):
        raise PluginError('stale', 'Catalog changed; try again')


def filter_prompt(context):
    global pending_input
    with lock:
        captured(context)
        result = app.request('ui.prompt', invocation=context['invocation'], title='Filter catalog',
                             field={'id': 'query', 'label': 'Text (empty shows all records)',
                                    'kind': 'text', 'maximum_length': 256})
        pending_input = (result['surface'], dict(context))


def begin(context, text):
    global query
    captured(context)
    result = app.set_query(view, revision, text, expected_query_revision=context.get('query_revision'))
    query = result['query']


def submitted(context):
    global pending_input
    with lock:
        if pending_input is None or pending_input[0] != context['surface']:
            return
        _, original = pending_input
        pending_input = None
        if context['accepted'] and original['view'] == view:
            # Filtering is independent of the rows that happened to be visible
            # when the prompt opened. This submission is a fresh explicit intent.
            latest = {**original, 'model_revision': revision,
                      'query_revision': query['revision'] if query else None}
            begin(latest, context['values']['query'])


def refresh(context):
    with lock:
        begin(context, query['text'] if query else '')


def observed(event, sequence, data):
    global query, pending_search, search_running, last_scheduled, last_viewport, last_action
    with lock:
        if event == 'event.action':
            last_action = data['action']
            return
        if event == 'event.resync_required':
            app.resync(data['subscription'])
            return
        for state in data.get('sources', []):
            value = state['state']
            if state['source']['kind'] == 'viewport':
                last_viewport = value
            elif (state['source']['kind'] == 'view' and state['source']['view'] == view
                  and value.get('query', {}).get('pending')):
                candidate = value['query']
                if query is not None and candidate['revision'] != query['revision']:
                    continue
                if candidate['revision'] == last_scheduled:
                    continue
                query = candidate
                last_scheduled = candidate['revision']
                # One worker plus one latest intent: rapid queries never grow a queue.
                pending_search = (view, value['revision'], dict(candidate))
                if not search_running:
                    search_running = True
                    worker.submit(search)


def search():
    global pending_search, search_running, revision, current, query
    while True:
        with lock:
            captured_search = pending_search
            pending_search = None
            if captured_search is None:
                search_running = False
                return
        target, expected_revision, expected_query = captured_search
        # A finite lookup delay demonstrates stale-result rejection; no idle timer.
        time.sleep(0.2)
        candidate = model(expected_query['text'])
        with lock:
            if target != view:
                continue
            try:
                result = app.publish_model(target, expected_revision, candidate,
                                           expected_query_revision=expected_query['revision'])
            except PluginError:
                # Preserve the old display and pending query. Refresh is an explicit retry.
                continue
            revision, current, query = result['revision'], candidate, result['query']


def detail(context, observations=False):
    global revision, current
    with lock:
        captured(context)
        if query and query['pending']:
            raise PluginError('busy', 'Catalog lookup is pending')
        if observations:
            viewport = last_viewport or {}
            action = last_action or {}
            text = (f'Viewport: {viewport.get("top")} to {viewport.get("bottom")}\n'
                    f'Accepted action: {action.get("command", "none")} · {action.get("selected_count", 0)} selected rows')
        else:
            selected = set(context['rows'])
            row = next((row for row in current['rows'] if row['id'] in selected), None)
            if row is None:
                raise PluginError('invalid_argument', 'Select a catalog record')
            text = row['text']
        header = {key: value for key, value in current.items() if key != 'rows'}
        header['detail'] = {'text': text, 'role': 'heading'}
        result = app.patch_view(view, revision, [], header,
                                expected_query_revision=context.get('query_revision'))
        revision, current = result['revision'], {**header, 'rows': current['rows']}


def event(name, data):
    global view, revision, query, current, subscription, pending_input, pending_search, last_scheduled
    with lock:
        if name == 'view.closed' and data['view'] == view:
            view = revision = query = current = pending_input = pending_search = last_scheduled = None
            if subscription is not None:
                app.unsubscribe(subscription)
                subscription = None


app.handlers = {'open': open_view, 'filter': filter_prompt, 'refresh': refresh,
                'inspect': detail, 'observations': lambda context: detail(context, True)}
app.on_input = submitted
app.on_event = event
if __name__ == '__main__':
    try:
        app.run()
    finally:
        with lock:
            pending_search = None
        worker.shutdown(wait=True, cancel_futures=True)
