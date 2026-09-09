# SPDX-License-Identifier: MPL-2.0
"""Explicit native terminal, system-handler, and retained-notification example."""
import os
from application import Application

app = Application('Handoffs', [
    {'name': 'notify', 'description': 'Publish an example notification', 'context': 'workspace'},
    {'name': 'terminal', 'description': 'Open a native shell terminal', 'context': 'workspace'},
    {'name': 'browser', 'description': 'Open an HTTP or HTTPS URL in the system browser',
     'context': 'workspace', 'arguments': [{'name': 'url', 'type': 'string'}]},
    {'name': 'file', 'description': 'Open a workspace file with its system handler',
     'context': 'workspace', 'arguments': [{'name': 'path', 'type': 'string'}]},
], ['notifications', 'terminals', 'external'])


def notify(context):
    app.publish_notification('info', 'Application notification',
                             'This retained message belongs to the configured plugin. Open :notifications to read it.')


def terminal(context):
    app.open_terminal(context['invocation'], 'Plugin shell', os.environ.get('SHELL') or '/bin/sh', ['-i'])


def browser(context):
    app.open_url(context['invocation'], context['arguments']['url'])


def file(context):
    app.open_file_externally(context['invocation'], context['arguments']['path'])


app.handlers = {'notify': notify, 'terminal': terminal, 'browser': browser, 'file': file}
if __name__ == '__main__':
    app.run()
