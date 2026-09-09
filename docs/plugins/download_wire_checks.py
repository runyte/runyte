# SPDX-License-Identifier: MPL-2.0
"""Public-wire binary download continuation for actual remote application checks."""
import hashlib


def check_download_wire(test, fixture, invocation, row, contents, send, receive, reply):
    # Keep the listed byte length while making the source unmistakably binary.
    binary = b'\xff\0' + bytes(range(len(contents) - 2))
    (fixture.root / 'é.txt').write_bytes(binary)
    send({'type': 'request', 'id': 'h:5', 'method': 'command.invoke', 'params': {
        **invocation['params'], 'command': 'download', 'context': 'view', 'arguments': {},
        'view': 'v:g:1', 'model_revision': 'm:1', 'rows': [row['id']]}})
    prompt = receive()
    test.assertEqual(prompt['method'], 'ui.prompt')
    test.assertEqual(prompt['params']['invocation'], 'h:5')
    reply(prompt, {'surface': 'u:g:1'})
    test.assertEqual(receive()['id'], 'h:5')
    send({'type': 'request', 'id': 'h:6', 'method': 'ui.submit', 'params': {
        'surface': 'u:g:1', 'accepted': True, 'values': {'destination': 'downloads/raw.bin'}}})
    job = receive()
    test.assertEqual(job['method'], 'job.create')
    reply(job, {'job': 'j:g:download', 'title': 'Download', 'state': 'running', 'progress': 0})
    original_receive = receive
    callback_finished = False
    status_ready = status_cleared = False
    revision = 1
    def asynchronous(message):
        nonlocal callback_finished, status_ready, status_cleared, revision
        if message.get('type') == 'response' and message.get('id') == 'h:6':
            test.assertEqual(message['result'], {'job': 'j:g:download'})
            callback_finished = True
            return True
        if message.get('method') == 'view.publish':
            test.assertEqual(message['params']['view'], 'v:g:1')
            test.assertEqual(message['params']['expected_revision'], f'm:{revision}')
            rows = message['params']['model']['rows']
            status = next((item for item in rows if item['id'] == 'download-status'), None)
            if status and status['text'].startswith('Download ready'):
                status_ready = True
                test.assertIn('.confirm-download', status['text'])
            if status_ready and status is None:
                status_cleared = True
            revision += 1
            reply(message, {'view': 'v:g:1', 'revision': f'm:{revision}', 'model': message['params']['model']})
            return True
        return False

    def receive():
        message = original_receive()
        while asynchronous(message):
            message = original_receive()
        return message
    create = receive()
    test.assertEqual(create['method'], 'staging.create')
    test.assertEqual(create['params'], {'job': 'j:g:download', 'bytes': len(binary)})
    stage = fixture.base / 'host-stage'
    stage.touch()
    stage.chmod(0o600)
    reply(create, {'staging': 't:g:1', 'path': str(stage)})
    listing = receive()
    while listing.get('method') == 'job.update':
        reply(listing, {'job': 'j:g:download', 'title': 'Download', 'state': 'running',
                        'progress': listing['params']['progress']})
        listing = receive()
    test.assertEqual(listing['method'], 'filesystem.list')
    test.assertEqual(listing['params']['path'], 'downloads')
    reply(listing, {'directory': 'd:g:1', 'revision': 'dr:1', 'entries': [], 'next': None})
    prepared = receive()
    test.assertEqual(prepared['method'], 'staging.prepare')
    test.assertEqual(prepared['params']['destination'], 'downloads/raw.bin')
    test.assertEqual(prepared['params']['sha256'], hashlib.sha256(binary).hexdigest())
    test.assertEqual(stage.read_bytes(), binary)
    reply(prepared, {'plan': 'f:g:1', 'operations': ['Create downloads/raw.bin']})
    release = receive()
    test.assertEqual(release['method'], 'filesystem.release')
    reply(release, {})
    ready = receive()
    test.assertEqual(ready['method'], 'job.update')
    test.assertEqual(ready['params']['progress'], 100)
    reply(ready, {'job': 'j:g:download', 'title': 'Download', 'state': 'running', 'progress': 100})
    while not status_ready:
        test.assertTrue(asynchronous(original_receive()), 'Unexpected output while waiting for ready status')
    test.assertTrue(callback_finished, 'Native submission remained open during the download')
    send({'type': 'request', 'id': 'h:7', 'method': 'command.invoke', 'params': {
        **invocation['params'], 'command': 'confirm-download', 'context': 'workspace', 'arguments': {}}})
    apply = receive()
    test.assertEqual(apply['method'], 'filesystem.apply')
    test.assertEqual(apply['params'], {'plan': 'f:g:1', 'invocation': 'h:7'})
    reply(apply, {})
    finished = receive()
    test.assertEqual(finished['method'], 'job.finish')
    test.assertEqual(finished['params'], {'job': 'j:g:download', 'state': 'succeeded'})
    reply(finished, {'job': 'j:g:download', 'title': 'Download', 'state': 'succeeded', 'progress': 100})
    test.assertEqual(receive(), {'type': 'response', 'id': 'h:7', 'result': {'job': 'j:g:download'}})
    while not status_cleared:
        test.assertTrue(asynchronous(original_receive()), 'Unexpected output while clearing transfer status')
