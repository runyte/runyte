#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Uppercase native selection spans in one revision-checked transaction."""
from application import Application, PluginError

app = Application('Uppercase', [
    {'name': 'uppercase', 'alias': 'uppercase', 'description': 'Uppercase every selection',
     'context': 'buffer'},
], ['text', 'selections'], runyte='>=0.3.0, <0.4.0')


def uppercase(context):
    selection = app.request('selection.get', pane=context['pane'])
    if (selection['buffer'] != context['buffer']
            or selection['revision'] != context['selection_revision']):
        raise PluginError('stale', 'Selection changed; invoke uppercase again')
    changes = []
    # Native spans already account for selection direction and inclusive heads.
    # Python strings count Unicode scalar values, as the public protocol does.
    for span in selection['spans']:
        text = app.request('buffer.read', buffer=context['buffer'],
                           expected_revision=context['buffer_revision'], **span)
        if (text['revision'] != context['buffer_revision']
                or text['from'] != span['from'] or text['to'] != span['to']):
            raise PluginError('stale', 'Buffer changed; invoke uppercase again')
        changes.append({**span, 'text': text['text'].upper()})
    # A single edit preserves atomicity and one undo step, including expansions
    # such as the one-scalar ß becoming SS. Never replay a failed transaction.
    app.request('buffer.edit', buffer=context['buffer'],
                expected_revision=context['buffer_revision'], changes=changes)


app.handlers['uppercase'] = uppercase

if __name__ == '__main__':
    app.run()
