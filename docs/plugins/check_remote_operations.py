# SPDX-License-Identifier: MPL-2.0
"""Remote mutation approval and cancellation through a fake public host port."""
import threading
import unicodedata
import unittest
from dataclasses import replace

from application import PluginError
from remote_operations import Operations
from transport import PreparedOperation, TransportError


class Port:
    def __init__(self):
        self.calls = []
        self.serial = 0
        self.failure = None
        self.hook = None

    def request(self, method, **params):
        self.calls.append((method, params))
        if method == 'ui.confirm':
            for field in ('title', 'message'):
                assert 0 < len(params[field].encode('utf-8')) <= 160
                assert not any(unicodedata.category(c) == 'Cc' for c in params[field])
        if self.failure and self.failure[0] == method:
            raise PluginError(self.failure[1], 'Injected refusal')
        if method in ('ui.prompt', 'ui.confirm', 'job.create'):
            self.serial += 1
            result = {'job' if method == 'job.create' else 'surface': f'id:{self.serial}'}
        else:
            result = {}
        if self.hook:
            self.hook(method, result)
        return result


class Transport:
    label = 'FTPS · test'
    connection_id = 'endpoint'

    def __init__(self):
        self.calls = []
        self.prepare_hook = self.apply_hook = None

    def prepare_operation(self, kind, source, destination, cancel):
        self.calls.append(('prepare', kind, source, destination))
        if self.prepare_hook:
            self.prepare_hook()
        if cancel.is_set():
            raise TransportError('cancelled', 'Cancelled')
        return PreparedOperation('endpoint', kind, source, destination, 'file', ('v1',),
                                 ('Remote checks are best effort; no undo.',))

    def apply_operation(self, prepared, cancel):
        self.calls.append(('apply', prepared))
        if self.apply_hook:
            return self.apply_hook(cancel)
        if cancel.is_set():
            raise TransportError('cancelled', 'Cancelled')
        return 'applied'


class Tests(unittest.TestCase):
    def setUp(self):
        self.port, self.transport = Port(), Transport()
        self.phases = []
        self.ops = Operations(self.port, self.transport, self.phases.append)
        self.source = {'path': '/remote/文.txt', 'kind': 'file', 'size': 2}
        self.context = {'invocation': 'fresh:1'}

    def submit(self, accepted=True, **values):
        return self.ops.submitted({'surface': self.ops.flow['surface'], 'accepted': accepted,
                                  'invocation': 'fresh:2', 'values': values})

    def prepare(self, kind='delete'):
        source = self.source if kind != 'mkdir' else {'path': '/remote', 'kind': 'directory'}
        self.ops.start(self.context, kind, source)
        flow = self.ops.flow
        if kind != 'delete':
            self.submit(destination='new')
        self.assertTrue(flow['done'].wait(2))
        self.assertEqual(flow['phase'], 'ready')
        return flow

    def apply(self):
        flow = self.ops.flow
        self.ops.confirm(self.context)
        result = self.submit()
        self.assertEqual(result, {'job': flow['job']})
        self.assertTrue(flow['done'].wait(2))
        return flow

    def test_prepare_only_reads_and_fresh_exact_confirmation_is_required(self):
        flow = self.prepare('rename')
        self.source['path'] = '/different'
        self.assertEqual(self.transport.calls, [('prepare', 'rename', '/remote/文.txt', '/remote/new')])
        self.assertIsNone(self.ops.submitted({'surface': 'forged', 'accepted': True}))
        self.ops.confirm({'invocation': 'fresh:99'})
        method, params = self.port.calls[-1]
        self.assertEqual(method, 'ui.confirm')
        self.assertEqual(params['invocation'], 'fresh:99')
        self.assertIn('/remote/文.txt', params['message'])
        self.assertIn('/remote/new', params['message'])
        self.assertIn('no undo', params['message'])
        self.assertNotIn('\n', params['message'])
        surface = flow['surface']
        self.submit()
        self.assertTrue(flow['done'].wait(2))
        self.assertEqual(self.transport.calls[-1], ('apply', flow['prepared']))
        self.assertIsNone(self.ops.submitted({'surface': surface, 'accepted': True}))
        self.assertEqual(sum(call[0] == 'apply' for call in self.transport.calls), 1)
        self.assertEqual(self.phases[-1], 'completed')

    def test_mkdir_uses_captured_directory_and_cancelled_prompt_does_nothing(self):
        self.ops.start(self.context, 'mkdir', {'path': '/remote', 'kind': 'directory'})
        self.submit(False)
        self.assertEqual(self.transport.calls, [])
        self.assertFalse(any(method == 'job.create' for method, _ in self.port.calls))
        self.prepare('mkdir')
        self.assertEqual(self.transport.calls, [('prepare', 'mkdir', '/remote/new', None)])

    def test_confirm_cancel_never_calls_transport_mutation(self):
        self.prepare()
        self.ops.confirm(self.context)
        self.submit(False)
        self.assertEqual(len(self.transport.calls), 1)
        self.assertEqual(self.port.calls[-1][1]['state'], 'cancelled')
        self.assertIsNone(self.ops.flow)

    def test_stale_confirmation_retains_same_plan_for_explicit_retry(self):
        flow = self.prepare()
        self.port.failure = ('ui.confirm', 'context_changed')
        with self.assertRaises(PluginError):
            self.ops.confirm(self.context)
        self.assertIs(self.ops.flow, flow)
        self.assertEqual(flow['phase'], 'ready')
        self.port.failure = None
        self.apply()
        self.assertEqual(sum(call[0] == 'prepare' for call in self.transport.calls), 1)

    def test_mutation_callback_returns_before_transport_finishes_and_keeps_worker_reserved(self):
        entered, resume = threading.Event(), threading.Event()
        def blocked(cancel):
            entered.set()
            resume.wait(2)
            raise TransportError('cancelled', 'Cancelled', outcome_unknown=True)
        self.transport.apply_hook = blocked
        flow = self.prepare()
        self.ops.confirm(self.context)
        self.submit()
        self.assertTrue(entered.wait(1))
        self.ops.event('job.changed', {'job': flow['job'], 'state': 'cancelled'})
        with self.assertRaises(PluginError) as raised:
            self.ops.start(self.context, 'delete', self.source)
        self.assertEqual(raised.exception.code, 'busy')
        resume.set()
        self.assertTrue(flow['done'].wait(2))
        self.assertEqual(self.phases[-1], 'outcome_unknown')
        self.assertEqual(self.port.calls[-1][1]['state'], 'outcome_unknown')

    def test_acknowledged_success_wins_over_late_cancel(self):
        flow = self.prepare()
        def completed(cancel):
            self.ops.event('job.cancel_requested', {'job': flow['job']})
            return 'applied'
        self.transport.apply_hook = completed
        self.apply()
        self.assertEqual(self.phases[-1], 'completed')

    def test_host_cancellation_before_success_finish_gets_terminal_unknown_ack(self):
        self.prepare()
        request = self.port.request
        def cancelling(method, **params):
            result = request(method, **params)
            if method == 'job.finish' and params['state'] == 'succeeded':
                raise PluginError('cancelled', 'Cancellation already accepted')
            return result
        self.port.request = cancelling
        self.apply()
        states = [params['state'] for method, params in self.port.calls if method == 'job.finish']
        self.assertEqual(states, ['succeeded', 'outcome_unknown'])
        self.assertEqual(self.phases[-1], 'outcome_unknown')
        self.assertIsNone(self.ops.flow)

    def test_known_refusal_and_unknown_exception_are_not_retried(self):
        for error, expected in [(TransportError('conflict', 'Changed'), 'failed'),
                                (TransportError('timeout', 'Lost', settled=False), 'outcome_unknown'),
                                (RuntimeError('Unexpected worker error'), 'outcome_unknown')]:
            with self.subTest(expected=expected):
                self.setUp()
                self.prepare()
                def fail(cancel):
                    raise error
                self.transport.apply_hook = fail
                self.apply()
                self.assertEqual(self.phases[-1], expected)
                self.assertEqual(sum(call[0] == 'apply' for call in self.transport.calls), 1)
                self.assertIsNone(self.ops.flow)

    def test_early_job_cancellation_prevents_preparation(self):
        def hook(method, result):
            if method == 'job.create':
                self.ops.event('job.cancel_requested', result)
        self.port.hook = hook
        with self.assertRaises(PluginError):
            self.ops.start(self.context, 'delete', self.source)
        self.assertEqual(self.transport.calls, [])
        self.assertIsNone(self.ops.flow)

    def test_retained_plan_deadline_cleanup_does_not_block_event_dispatch(self):
        flow = self.prepare()
        self.ops.event('job.changed', {'job': flow['job'], 'state': 'cancelled'})
        self.assertTrue(flow['done'].wait(2))
        self.assertIsNone(self.ops.flow)
        self.assertEqual(len(self.transport.calls), 1)

    def test_late_surface_after_cancellation_is_dismissed(self):
        entered, resume = threading.Event(), threading.Event()
        def hook(method, result):
            if method == 'ui.prompt':
                entered.set()
                resume.wait(2)
        self.port.hook = hook
        worker = threading.Thread(target=self.ops.start, args=(self.context, 'rename', self.source))
        worker.start()
        self.assertTrue(entered.wait(1))
        self.ops.cancel({})
        resume.set()
        worker.join(2)
        self.assertFalse(worker.is_alive())
        self.assertEqual(self.port.calls[-1][0], 'ui.dismiss')
        self.assertIsNone(self.ops.flow)

    def test_invalid_destination_does_not_start_job(self):
        for destination in ('/absolute', '../escape', 'x\nsecret', '\ud800', 'x' * 3801, ''):
            with self.subTest(destination=repr(destination)):
                self.ops.start(self.context, 'rename', self.source)
                with self.assertRaises(PluginError):
                    self.submit(destination=destination)
                self.assertIsNone(self.ops.flow)
        self.assertFalse(any(method == 'job.create' for method, _ in self.port.calls))

    def test_full_labels_and_warnings_are_not_silently_truncated(self):
        flow = self.prepare()
        flow['prepared'] = replace(flow['prepared'], source='/remote/' + '文' * 60)
        with self.assertRaises(PluginError) as raised:
            self.ops.confirm(self.context)
        self.assertEqual(raised.exception.code, 'limit_exceeded')
        self.assertFalse(any(method == 'ui.confirm' for method, _ in self.port.calls))

    def test_cancellation_while_preparation_publishes_ready_cleans_up(self):
        def status(phase):
            self.phases.append(phase)
            if phase == 'ready':
                self.ops.event('job.cancel_requested', {'job': self.ops.flow['job']})
        self.ops.on_status = status
        entered, resume = threading.Event(), threading.Event()
        def prepare():
            entered.set()
            resume.wait(2)
        self.transport.prepare_hook = prepare
        self.ops.start(self.context, 'delete', self.source)
        self.assertTrue(entered.wait(1))
        flow = self.ops.flow
        resume.set()
        self.assertTrue(flow['done'].wait(2))
        self.assertIsNone(self.ops.flow)
        self.assertEqual(self.phases[-1], 'cancelled')
        self.assertFalse(self.ops.worker_active)

    def test_ambiguous_remote_names_are_quoted_and_controls_escaped(self):
        flow = self.prepare()
        flow['prepared'] = replace(flow['prepared'], source='/r/a" → b\\c',
                                   destination='/r/\u202ename')
        self.ops.confirm(self.context)
        message = self.port.calls[-1][1]['message']
        self.assertIn('\\"', message)
        self.assertIn('\\u202e', message)
        self.assertNotIn('\u202e', message)

    def test_cancellation_overtakes_confirmation_reply_and_dismisses_surface(self):
        self.prepare()
        entered, resume = threading.Event(), threading.Event()
        def hook(method, result):
            if method == 'ui.confirm':
                entered.set()
                resume.wait(2)
        self.port.hook = hook
        worker = threading.Thread(target=self.ops.confirm, args=(self.context,))
        worker.start()
        self.assertTrue(entered.wait(1))
        self.ops.cancel({})
        resume.set()
        worker.join(2)
        self.assertFalse(worker.is_alive())
        self.assertEqual(self.port.calls[-1][0], 'ui.dismiss')
        self.assertIsNone(self.ops.flow)
        self.assertEqual(len(self.transport.calls), 1)

    def test_busy_and_symlink_admission_are_bounded(self):
        self.prepare()
        with self.assertRaises(PluginError):
            self.ops.start(self.context, 'delete', self.source)
        self.ops.cancel({})
        with self.assertRaises(PluginError):
            self.ops.start(self.context, 'delete', {**self.source, 'kind': 'symlink'})
        for number in range(50):
            self.ops.event('job.cancel_requested', {'job': str(number)})
        self.assertEqual(len(self.ops.ended), 16)


if __name__ == '__main__':
    unittest.main()
