# SPDX-License-Identifier: MPL-2.0
"""Installed Claude + real Runyte, with mocked model and preview eligibility.

This deliberately seeds Claude's fixture-owned preview feature cache. It proves
the installed client's channel handling, NOT account rollout eligibility or a
real model's decisions. No real account, credentials or inference service is used.
Run with Python from the MCP acceptance environment and a built Runyte binary:
  python tests/mcp/claude_channel_probe.py /absolute/path/to/runyte
Local sockets and subprocess pipes must be available (the restricted Codex tool
sandbox can close Claude's stdio sockets before MCP initialization).
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import threading
import time

from durable_fixture import DurableFixture
from editor_fixture import CONTROL, screen_text
from native_pty import spawn
from startup import Terminal
import ptybench


class Model:
    def __init__(self, project):
        self.project = str(project)
        self.requests = []
        self.stage = 'join'
        self.question_id = None

    def answer(self, request):
        if not request.get('tools'):
            return {'type': 'text', 'text': 'Sandbox probe'}
        self.requests.append(request)
        messages = request.get('messages', [])
        conversation = json.dumps(messages)
        with (Path(self.project).parent / 'model-summary.jsonl').open('a') as trace:
            trace.write(json.dumps({'stage': self.stage,
                                    'contains_channel_event': '<channel ' in conversation}) + '\n')
        if self.stage == 'woken' and 'CHANNEL_PROBE_ASK' in conversation:
            self.stage = 'question'
        if self.stage == 'join':
            self.stage = 'ready'
            return {'type': 'tool_use', 'id': 'probe_join',
                    'name': 'mcp__runyte__join_channel', 'input': {
                        'workspace': self.project, 'provider': 'claude',
                        'task': 'isolated notification probe'}}
        if self.stage == 'ready':
            self.stage = 'waiting'
            return {'type': 'text', 'text': 'READY_FOR_NOTIFICATION'}
        if self.stage == 'waiting' and '<channel ' in conversation:
            self.stage = 'reading'
            return {'type': 'tool_use', 'id': 'probe_read',
                    'name': 'mcp__runyte__read_messages',
                    'input': {'workspace': self.project}}
        if self.stage == 'reading':
            assert 'CHANNEL_PROBE_PRIVATE_MESSAGE' in conversation, 'read did not contain posted message'
            self.stage = 'woken'
            return {'type': 'text', 'text': 'CLAUDE_CHANNEL_WAKE_CONFIRMED'}
        if self.stage == 'question':
            self.stage = 'question_pending'
            self.question_id = 'probe_question'
            return {'type': 'tool_use', 'id': self.question_id,
                    'name': 'AskUserQuestion', 'input': {'questions': [{
                        'question': 'Sandbox choice must remain pending?',
                        'header': 'Sandbox', 'multiSelect': False,
                        'options': [
                            {'label': 'Leave pending', 'description': 'This option must not be selected by a wake.'},
                            {'label': 'Other choice', 'description': 'This option also needs physical input.'},
                        ],
                    }]}}
        return {'type': 'text', 'text': 'UNEXPECTED_EXTRA_MODEL_TURN'}


def model_server(model):
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_arguments):
            pass

        def do_GET(self):
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(b'{}')

        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or '{}')
            if 'count_tokens' in self.path:
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"input_tokens":100}')
                return
            block = model.answer(request)
            tool = block['type'] == 'tool_use'
            start = {**block, 'input': {}} if tool else {'type': 'text', 'text': ''}
            delta = ({'type': 'input_json_delta', 'partial_json': json.dumps(block['input'])}
                     if tool else {'type': 'text_delta', 'text': block['text']})
            events = [
                ('message_start', {'type': 'message_start', 'message': {
                    'id': 'msg_local_probe', 'type': 'message', 'role': 'assistant',
                    'model': request.get('model', 'claude-opus-5-5'), 'content': [],
                    'stop_reason': None, 'stop_sequence': None,
                    'usage': {'input_tokens': 100, 'output_tokens': 1}}}),
                ('content_block_start', {'type': 'content_block_start', 'index': 0, 'content_block': start}),
                ('content_block_delta', {'type': 'content_block_delta', 'index': 0, 'delta': delta}),
                ('content_block_stop', {'type': 'content_block_stop', 'index': 0}),
                ('message_delta', {'type': 'message_delta', 'delta': {
                    'stop_reason': 'tool_use' if tool else 'end_turn', 'stop_sequence': None},
                    'usage': {'output_tokens': 10}}),
                ('message_stop', {'type': 'message_stop'}),
            ]
            output = ''.join('event: ' + name + '\ndata: ' + json.dumps(event) + '\n\n'
                             for name, event in events).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(output)))
            self.end_headers()
            self.wfile.write(output)

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    return server, worker


class ClaudePTY:
    def __init__(self, arguments, cwd, env):
        self.process, self.fd = spawn(arguments, cwd=cwd, env=env, configure=ptybench._configure)
        self.screen = Terminal()
        self.output = bytearray()
        self.writes = []
        self.consents = set()
        self.poll = selectors.DefaultSelector()
        self.poll.register(self.fd, selectors.EVENT_READ)

    def pump(self, seconds=.1):
        if self.poll.select(seconds):
            data = os.read(self.fd, 65536)
            if data:
                self.output.extend(data)
                # Claude's startup dialogs work with a basic VT terminal.
                # Do not advertise Runyte's extended keyboard capabilities.
                self.screen.feed(data)
        assert self.process.poll() is None, 'Claude exited before probe completed'

    def send(self, value, reason):
        self.writes.append(reason)
        os.write(self.fd, value)

    def wait(self, marker, seconds=15):
        until = time.monotonic() + seconds
        while time.monotonic() < until:
            text = screen_text(self.screen) or ''
            compact = ''.join(text.split())
            if ''.join(marker.split()) in compact:
                return
            if 'Loadingdevelopmentchannels' in compact and 'channel' not in self.consents:
                self.consents.add('channel')
                self.send(b'\r', 'local_development_consent')
            elif 'DoyouwanttousethisAPIkey?' in compact and 'key' not in self.consents:
                self.consents.add('key')
                self.send(b'\x1b[A', 'select_fake_api_key')
            elif '❯Yes' in compact and 'key' in self.consents and 'key_enter' not in self.consents:
                self.consents.add('key_enter')
                self.send(b'\r', 'fake_api_key_consent')
            elif 'Yes,Itrustthisfolder' in compact and 'trust' not in self.consents:
                self.consents.add('trust')
                self.send(b'\x1b[B', 'select_fixture_project')
            elif '❯Yes,Itrustthisfolder' in compact and 'trust_enter' not in self.consents:
                self.consents.add('trust_enter')
                self.send(b'\r', 'fixture_project_trust')
            self.pump()
        raise AssertionError('Claude did not show ' + marker + '; tail=' +
                             CONTROL.sub(b'', bytes(self.output[-4096:])).decode(errors='replace'))

    def close(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=5)
        self.poll.close()
        os.close(self.fd)


def run(binary, claude, directory=None):
    fixture = DurableFixture(binary, root=directory)
    server = worker = terminal = None
    try:
        fixture.add_identity('claude-wake', ('agent_channel', 'channel_nudge'))
        subprocess.run(['git', 'init', '--quiet', str(fixture.project)],
                       env=fixture.env, check=True, capture_output=True)
        fixture.start()
        sender = fixture.client('sender')
        sender.data('join_channel', workspace=str(fixture.project), provider='codex', task='probe sender')
        model = Model(fixture.project)
        server, worker = model_server(model)
        config = fixture.root / 'claude'
        config.mkdir(mode=0o700)
        (config / '.claude.json').write_text(json.dumps({
            'hasCompletedOnboarding': True, 'theme': 'dark',
            'cachedGrowthBookFeatures': {'tengu_harbor': True},
            'projects': {str(fixture.project): {'hasTrustDialogAccepted': True}},
            'customApiKeyResponses': {'approved': ['-invalid-local-probe'], 'rejected': []},
        }))
        mcp = fixture.root / 'claude-mcp.json'
        mcp.write_text(json.dumps({'mcpServers': {'runyte': {
            'command': str(fixture.binary), 'args': ['mcp', '--identity', 'claude-wake',
                                                     '--channel-notifications', 'claude'],
        }}}))
        env = {key: value for key, value in fixture.env.items()
               if key.startswith(('XDG_', 'RUNYTE_'))}
        env.update({
            'PATH': '/usr/bin:/bin', 'TERM': 'xterm-256color',
            'CLAUDE_CONFIG_DIR': str(config), 'ANTHROPIC_API_KEY': 'sk-ant-invalid-local-probe',
            'ANTHROPIC_BASE_URL': 'http://127.0.0.1:' + str(server.server_port),
            'DISABLE_TELEMETRY': '1', 'DISABLE_ERROR_REPORTING': '1', 'DISABLE_AUTOUPDATER': '1',
            'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC': '1',
            'CLAUDE_CODE_GB_DISK_CACHE_WHEN_TELEMETRY_OFF': '1',
        })
        terminal = ClaudePTY([
            claude, '--setting-sources', '', '--strict-mcp-config', '--mcp-config', str(mcp),
            '--dangerously-load-development-channels', 'server:runyte',
            '--permission-mode', 'manual', '--allowedTools',
            'mcp__runyte__join_channel,mcp__runyte__read_messages',
            '--debug-file', str(fixture.root / 'claude-debug.log'), 'START_CLAUDE_PROBE',
        ], fixture.project, env)
        terminal.wait('READY_FOR_NOTIFICATION', seconds=30)
        participants = sender.data('list_participants', workspace=str(fixture.project))['participants']
        recipient = next(row for row in participants if row['provider'] == 'claude')
        assert recipient['terminal'] is None, 'probe recipient must have no terminal binding'
        writes = len(terminal.writes)
        posted = sender.data('post_message', workspace=str(fixture.project),
                             text='CHANNEL_PROBE_PRIVATE_MESSAGE', nudge=[recipient['participant']])
        assert posted['nudges'][0]['state'] == 'queued', posted
        terminal.wait('CLAUDE_CHANNEL_WAKE_CONFIRMED')
        assert terminal.writes[writes:] == [], 'notification wake wrote terminal input'
        terminal.send(b'CHANNEL_PROBE_ASK\r', 'request_sandbox_choice')
        terminal.wait('Sandbox choice must remain pending?')
        terminal.wait('Leave pending')
        writes = len(terminal.writes)
        request_count = len(model.requests)
        posted = sender.data('post_message', workspace=str(fixture.project),
                             text='SECOND_MESSAGE_WHILE_QUESTION_PENDING', nudge=[recipient['participant']])
        assert posted['nudges'][0]['state'] == 'queued', posted
        until = time.monotonic() + 1.5
        while time.monotonic() < until:
            terminal.pump()
        assert terminal.writes[writes:] == [], 'nudge wrote bytes while choice was pending'
        assert len(model.requests) == request_count, 'choice completed or model resumed without physical answer'
        assert 'Sandboxchoicemustremainpending?' in ''.join((screen_text(terminal.screen) or '').split())
        summary = {'claude_native_client': 'passed',
                   'claude_version': subprocess.check_output([claude, '--version'], env=env, text=True).strip(),
                   'model_service': 'mock',
                   'preview_eligibility': 'mocked fixture-owned cache',
                   'real_runyte_host_and_adapter': True, 'terminal_binding': None,
                   'idle_wake': 'passed', 'pending_choice_preserved': 'passed',
                   'terminal_input_writes_during_wake': 0,
                   'model_calls_during_pending_choice': 0}
        print(json.dumps(summary, indent=2))
        return summary
    finally:
        if terminal:
            terminal.close()
        if server:
            server.shutdown()
            server.server_close()
            worker.join(timeout=5)
        fixture.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('--claude', default=shutil.which('claude'))
    parser.add_argument('--directory', type=Path, help='Keep fixture artifacts in a new temporary directory')
    args = parser.parse_args()
    assert args.claude, 'Claude Code is not installed'
    run(args.binary, args.claude, args.directory)
