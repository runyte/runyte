# SPDX-License-Identifier: MPL-2.0
"""Deterministic conditional provider; configure id: memory.

No files, network or credentials. This process owns all remote state; restarting
it resets the demonstration resource, and no mutation can outlive the process.
"""
import hashlib
import threading
from collections import OrderedDict
from application import Application, PluginError

TEXT = ('# Provider document\r\n\r\nEditable Unicode: é猫 🦀\r\n' * 4096).encode('utf-8')
WEAK = False
app = Application('Memory provider', [
    {'name': 'open', 'description': 'Open a deterministic provider document',
     'context': 'workspace', 'arguments': [{'name': 'key', 'type': 'string'}]},
    {'name': 'save', 'description': 'Save the captured provider document', 'context': 'buffer'},
    {'name': 'rebind', 'description': 'Reconcile the provider document explicitly', 'context': 'buffer'},
    {'name': 'inspect', 'description': 'Compare fresh remote text with the provider document', 'context': 'buffer'},
], ['providers', 'documents', 'jobs'])
registered = False
registration_lock = threading.Lock()
state_lock = threading.Lock()
staging = {}
settled = OrderedDict()


def version():
    return hashlib.sha256(TEXT).hexdigest()


def remember(job, outcome):
    settled[job] = outcome
    while len(settled) > 128:
        settled.popitem(last=False)


def register():
    global registered
    with registration_lock:
        if not registered:
            app.request('provider.register', name='memory', conditional_write=not WEAK, atomic_replace=True)
            registered = True


def open_resource(context):
    register()
    result = app.request('resource.open', plugin='memory', provider='memory',
                         key=context['arguments']['key'], invocation=context['invocation'])
    return {'job': result['job']}


def save(context):
    result = app.request('buffer.save', buffer=context['buffer'], expected_revision=context['buffer_revision'])
    return {'job': result['job']}


def rebind(context):
    register()
    result = app.request('resource.rebind', buffer=context['buffer'], expected_revision=context['buffer_revision'])
    return {'job': result['job']}


def inspect(context):
    register()
    result = app.request('resource.inspect', buffer=context['buffer'],
                         expected_revision=context['buffer_revision'], invocation=context['invocation'])
    return {'job': result['job']}


def metadata(context):
    if context['provider'] != 'memory' or context['key'] not in ('notes', 'alias'):
        raise PluginError('not_found', 'Unknown memory resource')
    return {'key': 'notes', 'label': 'Memory notes', 'syntax_hint': 'markdown',
            'version': version(), 'encoding': 'utf-8', 'bytes': len(TEXT)}


def stat(context):
    with state_lock:
        return {'kind': 'stat', 'value': metadata(context)}


def reconcile(context):
    # The same lock serializes commit and reconciliation. A live staged upload
    # cannot be certified, and an aborted job cannot be revived by a late Begin.
    # An unknown old-generation job is safe ONLY because this memory transport
    # has no remote server or child whose write could outlive its process.
    with state_lock:
        if context['previous_write'] in staging:
            raise PluginError('outcome_unknown', 'Previous upload is still staged')
        return {'kind': 'reconciled', 'value': {'metadata': metadata(context),
                'previous_write': context['previous_write']}}


def read(context):
    with state_lock:
        metadata(context)
        if context['version'] != version():
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
        return {'kind': 'read', 'value': {'version': version(), 'offset': offset,
            'text': text, 'eof': end == len(TEXT)}}


def begin(context):
    with state_lock:
        metadata(context)
        mode = 'confirmed_best_effort' if WEAK else 'conditional'
        if context.get('mode') != mode:
            raise PluginError('invalid_argument', 'Unexpected write mode')
        job = context['job']
        if job in settled or job in staging:
            raise PluginError('conflict', 'Upload is already known')
        if len(staging) >= 4:
            raise PluginError('busy', 'Staging capacity reached')
        if context['encoding'] != 'utf-8' or not 0 <= context['bytes'] <= 8 * 1024 * 1024:
            raise PluginError('limit_exceeded', 'Unsupported upload size or encoding')
        if context['expected_version'] != version():
            raise PluginError('conflict', 'Memory resource changed')
        staging[job] = {'text': bytearray(), 'bytes': context['bytes'],
                        'version': context['expected_version'], 'mode': mode}
        return {'kind': 'write_started', 'value': {'upload': job}}


def upload_for(context):
    if context['upload'] != context['job'] or context['job'] not in staging:
        raise PluginError('not_found', 'Unknown staged upload')
    return staging[context['job']]


def write_chunk(context):
    with state_lock:
        upload = upload_for(context)
        data = context['text'].encode('utf-8')
        if len(data) > 128 * 1024 or context['offset'] != len(upload['text']) or len(upload['text']) + len(data) > upload['bytes']:
            raise PluginError('invalid_argument', 'Invalid upload range')
        upload['text'].extend(data)
        return {'kind': 'write_chunk', 'value': {'offset': len(upload['text'])}}


def commit(context):
    global TEXT
    with state_lock:
        upload = upload_for(context)
        if context.get('mode') != upload['mode']:
            raise PluginError('invalid_argument', 'Write mode changed after staging')
        if context['expected_version'] != upload['version'] or version() != upload['version'] or len(upload['text']) != upload['bytes']:
            del staging[context['job']]
            remember(context['job'], 'rejected')
            return {'kind': 'write_rejected', 'value': {'error': {'code': 'conflict', 'message': 'Memory resource changed or upload is incomplete'}}}
        # Precondition and replacement occur together under the same lock.
        # --weak advertises less than this local implementation guarantees so
        # native confirmation can be exercised without a real remote service.
        TEXT = bytes(upload['text'])
        del staging[context['job']]
        remember(context['job'], 'committed')
        return {'kind': 'write_committed', 'value': {'version': version()}}


def abort(context):
    with state_lock:
        if settled.get(context['job']) == 'committed':
            raise PluginError('outcome_unknown', 'Upload already committed')
        staging.pop(context['job'], None)
        remember(context['job'], 'aborted')
        return {'kind': 'write_aborted', 'value': {}}


app.handlers = {'open': open_resource, 'save': save, 'rebind': rebind, 'inspect': inspect}
app.resource_handlers = {'resource.stat': stat, 'resource.read': read, 'resource.reconcile': reconcile,
    'resource.write.begin': begin, 'resource.write.chunk': write_chunk,
    'resource.write.commit': commit, 'resource.write.abort': abort}
if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--weak', action='store_true',
                        help='advertise best-effort writes to exercise native confirmation')
    WEAK = parser.parse_args().weak
    app.run()
