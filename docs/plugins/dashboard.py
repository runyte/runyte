# SPDX-License-Identifier: MPL-2.0
"""Retained dashboard, atomic row patches and staged large-model example."""
import threading
from application import Application, PluginError

app = Application('Dashboard', [
    {'name': 'open', 'description': 'Open the example dashboard', 'context': 'workspace'},
    {'name': 'large', 'description': 'Open 8000 staged dashboard rows', 'context': 'workspace'},
    {'name': 'toggle', 'description': 'Toggle selected rows', 'context': 'view', 'primary': True},
    {'name': 'reverse', 'description': 'Reverse rows atomically', 'context': 'view'},
    {'name': 'inspect', 'description': 'Show selected row detail', 'context': 'view'},
    {'name': 'verify', 'description': 'Read and verify one immutable model', 'context': 'view'},
], ['views'])
lock = threading.Lock()
view = None
revision = None
current = None


def model(count):
    return {'title': 'Dashboard', 'purpose': 'dashboard',
            'columns': [{'id': 'item', 'label': 'Item'}, {'id': 'state', 'label': 'State'}],
            'rows': [{'id': f'item-{index:04}', 'text': '', 'role': 'ordinary', 'cells': [
                {'text': f'Item {index:04} · ' + ('Retained Unicode é猫 example. ' * (7 if count > 100 else 1)), 'role': 'ordinary'},
                {'text': 'Ready', 'role': 'muted'}]} for index in range(count)],
            'status': {'text': f'{count} rows · Enter toggles a selected row; Tab opens actions', 'role': 'ordinary'},
            'detail': {'text': 'Rows have stable identities across patches and reordering.', 'role': 'muted'},
            'preview': {'text': 'Use inspect to read the complete selected label.\nColumn clipping never changes the retained model.', 'role': 'muted'},
            'actions': ['toggle', 'reverse', 'inspect', 'verify']}


def open_view(context, count=12):
    global view, revision, current
    with lock:
        candidate = model(count)
        if view is None:
            result = app.request('view.create', model={'title': 'Dashboard', 'purpose': 'dashboard', 'rows': []})
            view, revision = result['view'], result['revision']
        result = app.publish_model(view, revision, candidate)
        revision, current = result['revision'], candidate
        app.request('pane.show', invocation=context['invocation'], view=view)


def update(context, action):
    global revision, current
    with lock:
        if context['view'] != view or context['model_revision'] != revision:
            raise PluginError('stale', 'Dashboard changed; try again')
        header = {key: value for key, value in current.items() if key != 'rows'}
        rows = list(current['rows'])
        selected = set(context['rows'])
        operations = []
        if action == 'reverse':
            rows.reverse()
            operations = [{'kind': 'reorder', 'ids': [row['id'] for row in rows]}]
        elif action == 'toggle':
            if len(selected) > 1024:
                raise PluginError('limit_exceeded', 'Select at most 1024 rows for this action')
            for index, row in enumerate(rows):
                if row['id'] in selected:
                    done = row['cells'][1]['text'] != 'Done'
                    updated = {**row, 'cells': [row['cells'][0], {'text': 'Done' if done else 'Ready', 'role': 'heading' if done else 'muted'}]}
                    rows[index] = updated
                    operations.append({'kind': 'update', 'row': updated})
        elif action == 'inspect':
            row = next((row for row in rows if row['id'] in selected), None)
            if row is None:
                raise PluginError('invalid_argument', 'Select a dashboard row')
            header['preview'] = {'text': row['cells'][0]['text'], 'role': 'ordinary'}
        else:
            observed = app.get_model(view)
            if observed['revision'] != revision or observed['model'] != current:
                raise PluginError('stale', 'Dashboard changed during verification')
            header['detail'] = {'text': f'Verified {len(rows)} rows from one immutable model revision.', 'role': 'heading'}
        result = app.patch_view(view, revision, operations, header=header)
        revision = result['revision']
        current = {**header, 'rows': rows}


def event(name, data):
    global view, revision, current
    if name == 'view.closed':
        with lock:
            if data['view'] == view:
                view = revision = current = None


app.handlers = {'open': open_view, 'large': lambda context: open_view(context, 8000),
                **{action: (lambda context, action=action: update(context, action))
                   for action in ('toggle', 'reverse', 'inspect', 'verify')}}
app.on_event = event
if __name__ == '__main__':
    app.run()
