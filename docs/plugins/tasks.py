# SPDX-License-Identifier: MPL-2.0
"""Native task-list application; standard library only, no persistent secrets."""
import threading
from application import Application, PluginError

app = Application('Tasks', [
    {'name': 'open', 'description': 'Open task list', 'context': 'workspace'},
    {'name': 'toggle', 'description': 'Toggle selected tasks', 'context': 'view', 'primary': True},
    {'name': 'filter', 'description': 'Toggle unfinished-only filter', 'context': 'view'},
], ['views'])
lock = threading.Lock()
view = None
revision = None
unfinished_only = False
tasks = [('read', 'Read the plugin guide'), ('edit', 'Edit a document'), ('split', 'Split this task list')]
done = set()

def model():
    return {'title': 'Tasks · unfinished' if unfinished_only else 'Tasks', 'purpose': 'list',
            'rows': [{'id': key, 'text': ('[x] ' if key in done else '[ ] ') + label,
                      'role': 'muted' if key in done else 'ordinary'}
                     for key, label in tasks if not unfinished_only or key not in done]}

def open_view(context):
    global view, revision
    with lock:
        if view is None:
            result = app.request('view.create', model=model())
            view, revision = result['view'], result['revision']
        app.request('pane.show', invocation=context['invocation'], view=view)

def update(context, filtering=False):
    global revision, unfinished_only
    with lock:
        if context['view'] != view or context['model_revision'] != revision:
            raise PluginError('stale', 'Task list changed; try again')
        previous_done, previous_filter = done.copy(), unfinished_only
        if filtering:
            unfinished_only = not unfinished_only
        else:
            for key in context['rows']:
                if key in done:
                    done.remove(key)
                else:
                    done.add(key)
        try:
            revision = app.request('view.publish', view=view, expected_revision=revision, model=model())['revision']
        except PluginError:
            done.clear()
            done.update(previous_done)
            unfinished_only = previous_filter
            raise

def event(name, data):
    global view
    if name == 'view.closed':
        with lock:
            if data['view'] == view:
                view = None

app.handlers = {'open': open_view, 'toggle': update, 'filter': lambda context: update(context, True)}
app.on_event = event
if __name__ == '__main__':
    app.run()
