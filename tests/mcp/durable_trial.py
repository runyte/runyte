# SPDX-License-Identifier: MPL-2.0
"""Manual subagent trial driver. stdin requests are MCP actions, not polling jobs.

host BINARY starts an isolated host, reports its manifest, then accepts actions.
agent MANIFEST NAME connects one adapter with the named pre-provisioned profile.
All output is JSON. Each input causes exactly one explicit action; no channel
checks or acknowledgements happen without a caller request.
"""
import json
import os
from pathlib import Path
import sqlite3
import sys

from durable_fixture import DurableFixture, call, post, private_json
from mcp_client import RealMCPClient


def main():
    fixture = None
    if sys.argv[1] == 'host':
        fixture = DurableFixture(sys.argv[2])
        for name in ('coordinator', 'receiver', 'peer'):
            fixture.profile(name)
        fixture.start()
        client = fixture.client('coordinator')
        manifest = fixture.root / 'trial.json'
        keys = ['HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR', 'XDG_STATE_HOME',
                'XDG_DATA_HOME', 'RUNYTE_ALL_HOSTS_DIR', 'RUNYTE_CONTEXT_HOME', 'RUNYTE_CHANNEL_HOME']
        private_json(manifest, {'binary': str(fixture.binary), 'project': str(fixture.project),
                                'profiles': str(fixture.profiles), 'environment': {k: fixture.env[k] for k in keys}})
        print(json.dumps({'manifest': str(manifest)}), flush=True)
    else:
        manifest = json.loads(Path(sys.argv[2]).read_text())
        env = dict(os.environ)
        env.update(manifest['environment'])
        env.pop('RUNYTE_PARENT_CONTEXT', None)
        client = RealMCPClient(manifest['binary'], manifest['project'], env, 'trial',
                               channel_profile=Path(manifest['profiles']) / (sys.argv[3] + '.json'))
        print(json.dumps({'connected': sys.argv[3]}), flush=True)
    try:
        for line in sys.stdin:
            try:
                request = json.loads(line)
                action = request.pop('action')
                if action == 'restart_host' and fixture:
                    fixture.stop()
                    fixture.start()
                    result = {'restarted': True}
                elif action == 'restart_adapter':
                    client.close()
                    if fixture:
                        client = fixture.client('coordinator')
                    else:
                        client = RealMCPClient(manifest['binary'], manifest['project'], env, 'trial',
                                               channel_profile=Path(manifest['profiles']) / (sys.argv[3] + '.json'))
                    result = {'restarted': True}
                elif action == 'storage_counts' and fixture:
                    with sqlite3.connect(fixture.store / 'agent-channels.sqlite3') as db:
                        result = {'messages': dict(db.execute('SELECT channel,count(*) FROM messages GROUP BY channel')),
                                  'pending': dict(db.execute('SELECT participant,count(*) FROM pending GROUP BY participant')),
                                  'integrity': db.execute('PRAGMA integrity_check').fetchone()[0]}
                elif action == 'post_auto_key':
                    directory = fixture.project if fixture else manifest['project']
                    reply, retry = post(client, request['channel'], request['text'], directory)
                    result = {'reply': reply, 'retry': retry}
                else:
                    result = call(client, action, **request)
                print(json.dumps({'result': result}), flush=True)
            except Exception as error:
                print(json.dumps({'error': str(error)}), flush=True)
    finally:
        client.close()
        if fixture:
            fixture.close()


if __name__ == '__main__':
    main()
