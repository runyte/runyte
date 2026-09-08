# SPDX-License-Identifier: MPL-2.0
"""Deterministic provider example: no files, network or credentials.

Configure this script with id: memory. Invoke plugin.memory.open notes.
"""
import hashlib
import threading
from application import Application, PluginError

TEXT = ('# Provider document\r\n\r\nEditable Unicode: é猫 🦀\r\n' * 4096).encode('utf-8')
VERSION = hashlib.sha256(TEXT).hexdigest()
app = Application('Memory provider', [
    {'name': 'open', 'description': 'Open a deterministic provider document',
     'context': 'workspace', 'arguments': [{'name': 'key', 'type': 'string'}]},
], ['providers', 'documents', 'jobs'])
registered = False
registration_lock = threading.Lock()

def open_resource(context):
    global registered
    with registration_lock:
        if not registered:
            app.request('provider.register', name='memory', conditional_write=False, atomic_replace=False)
            registered = True
    result = app.request('resource.open', plugin='memory', provider='memory',
                         key=context['arguments']['key'], invocation=context['invocation'])
    return {'job': result['job']}

def stat(context):
    if context['provider'] != 'memory' or context['key'] not in ('notes', 'alias'):
        raise PluginError('not_found', 'Unknown memory resource')
    return {'kind': 'stat', 'value': {'key': 'notes', 'label': 'Memory notes',
        'syntax_hint': 'markdown', 'version': VERSION, 'encoding': 'utf-8', 'bytes': len(TEXT)}}

def read(context):
    if context['provider'] != 'memory' or context['key'] != 'notes':
        raise PluginError('not_found', 'Unknown memory resource')
    if context['version'] != VERSION:
        raise PluginError('stale', 'Memory resource changed')
    offset, limit = context['offset'], context['limit']
    if not 0 <= offset <= len(TEXT) or not 1 <= limit <= 128 * 1024:
        raise PluginError('invalid_argument', 'Invalid resource range')
    end = min(offset + limit, len(TEXT))
    while end < len(TEXT) and TEXT[end] & 0xc0 == 0x80:
        end -= 1
    try:
        text = TEXT[offset:end].decode('utf-8')
    except UnicodeDecodeError as error:
        raise PluginError('invalid_argument', 'Range splits UTF-8') from error
    return {'kind': 'read', 'value': {'version': VERSION, 'offset': offset,
        'text': text, 'eof': end == len(TEXT)}}

app.handlers = {'open': open_resource}
app.resource_handlers = {'resource.stat': stat, 'resource.read': read}
if __name__ == '__main__':
    app.run()
