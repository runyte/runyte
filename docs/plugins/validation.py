#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Account-free asynchronous validation example; Python 3.10+, no packages."""
import time
from application import Application

app = Application('Field validation', [
    {'name': 'open', 'description': 'Open the validation example', 'context': 'workspace'}
], ['interaction'])


def open_form(context):
    app.request('ui.form', invocation=context['invocation'], title='Field validation example', fields=[
        {'id': 'name', 'label': 'Name', 'kind': 'text', 'required': True,
         'validate': True, 'validation_message': 'The example reserves the name taken'},
        {'id': 'code', 'label': 'Example code', 'kind': 'secret', 'required': True,
         'validate': True, 'validation_message': 'Use demo-code for this account-free example'},
    ])


def validate(context):
    # Simulate a finite service request on the SDK's independent validation worker.
    # Real adapters should use bounded IO and honor ui.validation_cancelled.
    time.sleep(0.2)
    values = context['values']
    return {'name': 'invalid' if values['name'] == 'taken' else 'valid',
            'code': 'valid' if values['code'] == 'demo-code' else 'invalid'}


app.handlers['open'] = open_form
app.on_validate = validate

if __name__ == '__main__':
    app.run()
