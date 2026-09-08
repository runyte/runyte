# SPDX-License-Identifier: MPL-2.0
"""Explicit nonsecret preferences; storage is accessed only by a user command."""
import json
import threading
from application import Application, PluginError

app = Application('Preferences', [
    {'name': 'open', 'description': 'Inspect configured and saved preferences', 'context': 'workspace'},
    {'name': 'remember', 'description': 'Remember a nonsecret destination', 'context': 'workspace',
     'arguments': [{'name': 'destination', 'type': 'string'}]},
    {'name': 'forget', 'description': 'Delete this plugin’s saved preferences', 'context': 'workspace'},
], ['settings', 'state', 'views'], settings_schema={'fields': [
    {'name': 'default_destination', 'type': 'string', 'max_length': 256},
]})
lock = threading.Lock()
view = revision = None
opening = 0


def destination(info, settings):
    document = info['document']
    if document is None:
        return settings.get('default_destination', '.')
    if document['version'] != 1:
        raise PluginError('unsupported', 'Saved preferences require an explicit plugin migration')
    data = document['data']
    if (not isinstance(data, dict) or set(data) != {'destination'}
            or not isinstance(data['destination'], str) or len(data['destination']) > 256):
        raise PluginError('invalid_argument', 'Saved preferences have an unsupported shape')
    return data['destination']


def open_view(context):
    global view, revision, opening
    with lock:
        opening += 1
        captured_opening = opening
    settings = app.get_settings()
    info = app.get_state()
    current = destination(info, settings)
    model = {'title': 'Preferences', 'purpose': 'document', 'rows': [
        {'id': 'destination', 'text': 'Destination: ' + json.dumps(current, ensure_ascii=True), 'role': 'ordinary'},
        {'id': 'source', 'text': 'Saved workspace state' if info['document'] else 'Configured default', 'role': 'muted'},
        {'id': 'commands', 'text': 'Use remember <destination>, forget, then open to inspect.', 'role': 'ordinary'},
    ]}
    with lock:
        if captured_opening != opening:
            raise PluginError('context_changed', 'A newer preferences request superseded this view')
        if view is None:
            result = app.request('view.create', model=model)
            view, revision = result['view'], result['revision']
        else:
            revision = app.publish_model(view, revision, model)['revision']
        app.request('pane.show', invocation=context['invocation'], view=view)


def remember(context):
    value = context['arguments']['destination']
    if len(value) > 256:
        raise PluginError('invalid_argument', 'Destination exceeds 256 characters')
    current = app.get_state()
    destination(current, app.get_settings())  # Refuse unknown versions; never migrate implicitly.
    app.set_state(current['revision'], 1, {'destination': value})


def forget(_context):
    current = app.get_state()
    app.delete_state(current['revision'])


def event(name, data):
    global view, revision, opening
    if name == 'view.closed':
        with lock:
            if data['view'] == view:
                opening += 1
                view = revision = None


app.handlers = {'open': open_view, 'remember': remember, 'forget': forget}
app.on_event = event
if __name__ == '__main__':
    app.run()
