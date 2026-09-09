# SPDX-License-Identifier: MPL-2.0
"""Explicit local document lifecycle. Run with python3; see applications.md."""
from application import Application

app = Application('Documents', [
    {'name': 'create', 'description': 'Create a named unsaved document', 'context': 'workspace',
     'arguments': [{'name': 'path', 'type': 'string'}, {'name': 'text', 'type': 'string'}]},
    {'name': 'save', 'description': 'Save the invoking document asynchronously', 'context': 'buffer'},
    {'name': 'close', 'description': 'Close the invoking clean document', 'context': 'buffer'},
], ['documents', 'jobs'])

def create(context):
    app.request('buffer.create', **context['arguments'], invocation=context['invocation'])

def save(context):
    result = app.request('buffer.save', buffer=context['buffer'],
                         expected_revision=context['buffer_revision'])
    return {'job': result['job']}

def close(context):
    app.request('buffer.close', buffer=context['buffer'],
                expected_revision=context['buffer_revision'])

app.handlers = {'create': create, 'save': save, 'close': close}
if __name__ == '__main__':
    app.run()
