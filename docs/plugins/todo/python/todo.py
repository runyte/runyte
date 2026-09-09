# SPDX-License-Identifier: MPL-2.0
"""Standalone epoch-2 todo showcase. Python standard library, Linux/macOS."""
import copy
import json
import os
import select
import time

VERSION = 'runyte-experimental-2'
LIMIT = 1024 * 1024
COMMANDS = [
    {'name': 'open', 'description': 'Open todo list', 'context': 'workspace'},
    {'name': 'add', 'description': 'Add a task', 'context': 'view',
     'arguments': [{'name': 'title', 'type': 'string'}]},
    {'name': 'toggle', 'description': 'Toggle selected tasks', 'context': 'view', 'primary': True},
    {'name': 'remove', 'description': 'Remove selected tasks', 'context': 'view'},
    {'name': 'filter', 'description': 'Toggle unfinished-only filter', 'context': 'view'},
]


def send(message):
    data = (json.dumps(message, ensure_ascii=False, separators=(',', ':')) + '\n').encode()
    if len(data) > LIMIT:
        raise ValueError('Output limit')
    deadline = time.monotonic() + 2
    while data:
        if time.monotonic() >= deadline:
            raise TimeoutError('Output stalled')
        if not select.select([], [1], [], max(0, deadline - time.monotonic()))[1]:
            raise TimeoutError('Output stalled')
        try:
            data = data[os.write(1, data):]
        except BlockingIOError:
            pass


class Todo:
    def __init__(self):
        self.phase = 'hello'
        self.tasks = [{'id': 'task-1', 'title': 'Read the plugin guide', 'done': False}]
        self.next_task = 2
        self.filtered = False
        self.view = self.revision = None
        self.pending = None
        self.serial = 0
        self.deadline = time.monotonic() + 8
        self.uncertain = False

    def model(self, tasks, filtered):
        return {'title': 'Todo · Python' + (' · unfinished' if filtered else ''),
                'purpose': 'list', 'rows': [
                    {'id': task['id'], 'text': ('[x] ' if task['done'] else '[ ] ') + task['title'],
                     'role': 'muted' if task['done'] else 'ordinary'}
                    for task in tasks if not filtered or not task['done']]}

    def finish(self, request_id, code=None):
        send({'type': 'response', 'id': request_id,
              **({'error': {'code': code, 'message': {
                  'stale': 'Todo list changed; invoke the action again',
                  'busy': 'A todo update is pending',
                  'unavailable': 'Restart this plugin after an uncertain update',
                  'limit_exceeded': 'Todo limit reached',
              }.get(code, 'Todo request refused')}} if code else {'result': {'job': None}})})

    def request(self, stage, params):
        self.serial += 1
        self.pending.update(stage=stage, id=f'p:{self.serial}')
        send({'type': 'request', 'id': self.pending['id'], 'method': stage, 'params': params})

    def invoke(self, message):
        request_id, context = message['id'], message['params']
        if self.pending and request_id == self.pending['command']:
            raise ValueError('Duplicate command')
        if message['method'] != 'command.invoke':
            return self.finish(request_id, 'unsupported')
        if self.pending:
            return self.finish(request_id, 'busy')
        if self.uncertain:
            return self.finish(request_id, 'unavailable')
        command = context['command']
        if command == 'open':
            if context['context'] != 'workspace':
                return self.finish(request_id, 'invalid_argument')
            self.pending = {'command': request_id, 'closed': False}
            self.deadline = time.monotonic() + 8
            if self.view is None:
                self.request('view.create', {'model': self.model(self.tasks, self.filtered)})
            else:
                self.request('pane.show', {'invocation': request_id, 'view': self.view})
            return
        if command not in ('add', 'toggle', 'remove', 'filter'):
            return self.finish(request_id, 'not_found')
        if (self.view is None or context['context'] != 'view'
                or context.get('view') != self.view or context.get('model_revision') != self.revision):
            return self.finish(request_id, 'stale')
        candidate = copy.deepcopy(self.tasks)
        filtered = self.filtered
        if command == 'add':
            title = context.get('arguments', {}).get('title')
            if (not isinstance(title, str) or not title.strip(' ') or len(title.encode()) > 256
                    or any(ord(c) < 32 or 127 <= ord(c) <= 159 or c in '\u2028\u2029' for c in title)):
                return self.finish(request_id, 'invalid_argument')
            if len(candidate) >= 100 or self.next_task >= 2**53:
                return self.finish(request_id, 'limit_exceeded')
            candidate.append({'id': f'task-{self.next_task}', 'title': title, 'done': False})
        elif command == 'filter':
            filtered = not filtered
        else:
            rows = context.get('rows')
            visible = {t['id'] for t in candidate if not filtered or not t['done']}
            if (not isinstance(rows, list) or not rows or len(rows) > 100
                    or any(not isinstance(row, str) for row in rows)
                    or len(set(rows)) != len(rows) or not set(rows) <= visible):
                return self.finish(request_id, 'invalid_argument')
            if command == 'remove':
                candidate = [t for t in candidate if t['id'] not in rows]
            else:
                for task in candidate:
                    if task['id'] in rows:
                        task['done'] = not task['done']
        # Candidate data becomes authoritative only after a matching acknowledgement.
        self.pending = {'command': request_id, 'closed': False, 'tasks': candidate,
                        'filtered': filtered, 'added': command == 'add'}
        self.deadline = time.monotonic() + 8
        self.request('view.publish', {'view': self.view, 'expected_revision': self.revision,
                                     'model': self.model(candidate, filtered)})

    def receive(self, message):
        if not isinstance(message, dict):
            raise ValueError('Expected object')
        if self.phase != 'ready':
            expected = 'hello' if self.phase == 'hello' else 'registered'
            if message['type'] != expected or 'views' not in message['capabilities']:
                raise ValueError('Invalid handshake')
            if self.phase == 'hello':
                if message['version'] != VERSION:
                    raise ValueError('Wrong epoch')
                send({'type': 'register', 'version': VERSION, 'name': 'Todo · Python',
                      'commands': COMMANDS, 'required_capabilities': ['views'], 'optional_capabilities': []})
                self.phase = 'registering'
            else:
                self.phase, self.deadline = 'ready', None
            return
        if message['type'] == 'request':
            self.invoke(message)
        elif message['type'] == 'event':
            if message['event'] == 'view.closed' and message['data']['view'] == self.view:
                self.view = self.revision = None
                if self.pending:
                    self.pending['closed'] = True
        elif message['type'] == 'response':
            pending = self.pending
            if not pending or message['id'] != pending['id'] or ('error' in message) == ('result' in message):
                raise ValueError('Unmatched response')
            code = None
            if 'error' in message:
                code = message['error']['code']
                if code not in ('stale', 'closed', 'busy', 'context_changed', 'no_frontend',
                                'unavailable', 'limit_exceeded', 'timeout', 'outcome_unknown'):
                    code = 'unavailable'
                if code in ('timeout', 'outcome_unknown') and pending['stage'] != 'pane.show':
                    self.uncertain = True
            elif pending['closed']:
                code = 'stale'
            elif pending['stage'] == 'pane.show':
                pass
            else:
                result = message['result']
                view, revision = result.get('view'), result.get('revision')
                if (not isinstance(view, str) or not view or len(view.encode()) > 256
                        or not isinstance(revision, str) or not revision or len(revision.encode()) > 256
                        or (pending['stage'] == 'view.publish'
                            and (view != self.view or revision == self.revision))):
                    self.uncertain, code = True, 'outcome_unknown'
                elif pending['stage'] == 'view.create':
                    self.view, self.revision = view, revision
                    self.request('pane.show', {'invocation': pending['command'], 'view': view})
                    return
                else:
                    self.tasks, self.filtered = pending['tasks'], pending['filtered']
                    self.next_task += pending['added']
                    self.revision = revision
            self.finish(pending['command'], code)
            self.pending, self.deadline = None, None
        else:
            raise ValueError('Unexpected message')


def main():
    os.set_blocking(1, False)
    app, buffer = Todo(), bytearray()
    while True:
        if app.deadline is not None and time.monotonic() >= app.deadline:
            raise TimeoutError('Host response timed out')
        timeout = None if app.deadline is None else max(0, app.deadline - time.monotonic())
        if not select.select([0], [], [], timeout)[0]:
            raise TimeoutError('Host response timed out')
        chunk = os.read(0, 8192)
        if not chunk:
            if buffer:
                raise ValueError('Truncated frame')
            return
        buffer.extend(chunk)
        while b'\n' in buffer:
            line, _, buffer = buffer.partition(b'\n')
            if len(line) + 1 > LIMIT:
                raise ValueError('Frame limit')
            app.receive(json.loads(line.decode('utf-8')))
        if len(buffer) >= LIMIT:
            raise ValueError('Frame limit')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError, RecursionError):
        # Do not leak wire input through a traceback; exit retires uncertain work.
        raise SystemExit(1)
