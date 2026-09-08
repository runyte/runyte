# SPDX-License-Identifier: MPL-2.0
"""Local file manager using public epoch 2 operations; Python standard library only."""
import threading
from pathlib import PurePosixPath
from application import Application, PluginError

def command(name, description, argument=None, primary=False, context='view'):
    result = {'name': name, 'description': description, 'context': context, 'primary': primary}
    if argument:
        result['arguments'] = [{'name': argument, 'type': 'string'}]
    return result

app = Application('Local files', [
    command('open', 'Browse a workspace-relative directory', 'path', context='workspace'),
    command('enter', 'Open the selected file or directory', primary=True),
    command('parent', 'Browse the parent directory'),
    command('refresh', 'Refresh the directory listing'),
    command('create', 'Review creating a file; colon argument: destination', 'destination'),
    command('mkdir', 'Review creating a directory; colon argument: destination', 'destination'),
    command('rename', 'Review renaming a file; colon argument: destination', 'destination'),
    command('copy', 'Review copying a file; colon argument: destination', 'destination'),
    command('trash', 'Review trashing the selected file'),
], ['views', 'filesystem', 'documents'])

lock = threading.RLock()
view = revision = directory = directory_revision = None
path = '.'
entries = {}

def listing(destination):
    rows, offset, expected = [], 0, None
    while True:
        page = app.request('filesystem.list', path=destination, offset=offset,
                           limit=128, expected_revision=expected)
        expected = page['revision']
        rows.extend(page['entries'])
        if page['next'] is None:
            return page['directory'], expected, rows
        offset = page['next']

def refresh(destination):
    global view, revision, directory, directory_revision, path, entries
    handle, baseline, rows = listing(destination)
    model = {'title': f'Files · {destination}', 'purpose': 'list', 'rows': [
        {'id': row['entry'], 'text': row['name'] + ('/' if row['kind'] == 'directory' else ''),
         'role': 'heading' if row['kind'] == 'directory' else 'ordinary'} for row in rows]}
    try:
        if view is None:
            result = app.request('view.create', model=model)
        else:
            result = app.request('view.publish', view=view, expected_revision=revision, model=model)
    except PluginError:
        if handle != directory:
            app.request('filesystem.release', directory=handle)
        raise
    if directory is not None and directory != handle:
        app.request('filesystem.release', directory=directory)
    view, revision = result['view'], result['revision']
    directory, directory_revision, path = handle, baseline, destination
    entries = {row['entry']: row for row in rows}

def current(context):
    if context.get('view') != view or context.get('model_revision') != revision:
        raise PluginError('stale', 'Directory view changed; try again')

def selected(context):
    current(context)
    if len(context['rows']) != 1 or context['rows'][0] not in entries:
        raise PluginError('invalid_argument', 'Select exactly one directory entry')
    return entries[context['rows'][0]]

def open_view(context):
    with lock:
        refresh(context['arguments']['path'])
        app.request('pane.show', invocation=context['invocation'], view=view)

def enter(context):
    with lock:
        row = selected(context)
        destination = str(PurePosixPath(path) / row['name'])
        if row['kind'] == 'directory':
            refresh(destination)
        elif row['kind'] == 'file':
            app.request('buffer.open', path=destination, invocation=context['invocation'])
        else:
            raise PluginError('unsupported', 'This example opens ordinary files and directories')

def navigate(context, parent=False):
    with lock:
        current(context)
        refresh(str(PurePosixPath(path).parent) if parent else path)

def mutate(context, operation):
    with lock:
        current(context)
        intent = {'operation': operation}
        if operation not in ('create_file', 'create_directory'):
            intent['entry'] = selected(context)['entry']
        if operation != 'trash':
            intent['destination'] = str(PurePosixPath(path) / context['arguments']['destination'])
        prepared = app.request('filesystem.prepare', directory=directory,
                               expected_revision=directory_revision, intent=intent)
        try:
            app.request('filesystem.apply', plan=prepared['plan'], invocation=context['invocation'])
        except PluginError:
            app.request('filesystem.cancel', plan=prepared['plan'])
            raise

def event(name, data):
    global view, directory
    with lock:
        if name == 'view.closed' and data['view'] == view:
            view = None
            if directory is not None:
                app.request('filesystem.release', directory=directory)
                directory = None
        elif name == 'filesystem.finished' and view is not None:
            # A refresh is explicit or follows an actual filesystem result;
            # this application has no periodic filesystem scan or idle timer.
            try:
                refresh(path)
            except PluginError:
                # Host confirmation already reports recovery details. Keep the
                # last readable model; Refresh obtains a new baseline later.
                pass

app.handlers = {
    'open': open_view, 'enter': enter,
    'parent': lambda context: navigate(context, True), 'refresh': navigate,
    'create': lambda context: mutate(context, 'create_file'),
    'mkdir': lambda context: mutate(context, 'create_directory'),
    'rename': lambda context: mutate(context, 'rename'),
    'copy': lambda context: mutate(context, 'copy'),
    'trash': lambda context: mutate(context, 'trash'),
}
app.on_event = event
if __name__ == '__main__':
    app.run()
