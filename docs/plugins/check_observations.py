# SPDX-License-Identifier: MPL-2.0
"""Ordered SDK observations and subscription handshake regression checks."""
import queue
import threading
import unittest
from application import Application, PluginError


class ObservationTests(unittest.TestCase):
    def setUp(self):
        self.app = Application('Observations', [], ['workspace'])
        self.requests = queue.Queue()
        self.app._write = self.requests.put
        self.received = queue.Queue()
        self.threads = []
        self.release = threading.Event()

    def tearDown(self):
        self.release.set()
        with self.app._lock:
            for future in self.app._pending.values():
                future.set_exception(PluginError('unavailable', 'Test ended'))
        for thread in self.threads:
            thread.join(2)
        for executor in (self.app._executor, self.app._resource_executor,
                         self.app._control, self.app._observations, self.app._validation_executor):
            executor.shutdown(wait=True, cancel_futures=True)

    def call(self, function):
        result = queue.Queue()
        def run():
            try:
                result.put(function())
            except Exception as error:
                result.put(error)
        thread = threading.Thread(target=run)
        self.threads.append(thread)
        thread.start()
        return self.requests.get(timeout=2), result

    def reply(self, request, result):
        self.app._response({'type': 'response', 'id': request['id'], 'result': result})

    def event(self, sequence, event='event.changed'):
        self.app._submit({'type': 'event', 'event': event, 'sequence': sequence,
                          'data': {'subscription': 's:1', 'sources': []}})

    def callback(self, event, sequence, data):
        self.received.put((event, sequence, data))

    def test_baseline_is_queued_before_events_even_before_subscribe_returns(self):
        request, result = self.call(lambda: self.app.subscribe([{'kind': 'buffers'}], self.callback))
        self.assertEqual(request['method'], 'event.subscribe')
        baseline = {'subscription': 's:1', 'sequence': 'e:0', 'sources': []}
        self.reply(request, baseline)
        self.event('e:1')
        self.assertEqual(result.get(timeout=2), baseline)
        self.assertEqual(self.received.get(timeout=2)[:2], ('event.baseline', 'e:0'))
        self.assertEqual(self.received.get(timeout=2)[:2], ('event.changed', 'e:1'))

    def test_resync_baseline_is_ordered_and_unsubscribe_retires_callback(self):
        request, result = self.call(lambda: self.app.subscribe([{'kind': 'attachment'}], self.callback))
        self.reply(request, {'subscription': 's:1', 'sequence': 'e:0', 'sources': []})
        result.get(timeout=2)
        self.received.get(timeout=2)
        self.event('e:1', 'event.resync_required')
        request, result = self.call(lambda: self.app.resync('s:1'))
        self.assertEqual(request['method'], 'event.resync')
        self.reply(request, {'subscription': 's:1', 'sequence': 'e:1', 'sources': []})
        result.get(timeout=2)
        self.event('e:2')
        request, result = self.call(lambda: self.app.unsubscribe('s:1'))
        self.reply(request, {})
        result.get(timeout=2)
        self.event('e:3')
        self.assertEqual([self.received.get(timeout=2)[:2] for _ in range(3)],
                         [('event.resync_required', 'e:1'), ('event.baseline', 'e:1'), ('event.changed', 'e:2')])
        self.app._observations.submit(lambda: None).result(timeout=2)
        self.assertTrue(self.received.empty())

    def test_failed_subscribe_does_not_install_a_callback(self):
        request, result = self.call(lambda: self.app.subscribe([], self.callback))
        self.app._response({'type': 'response', 'id': request['id'],
                            'error': {'code': 'invalid_argument', 'message': 'Empty filters'}})
        self.assertEqual(result.get(timeout=2).code, 'invalid_argument')
        self.assertFalse(self.app._subscriptions)
        self.assertFalse(self.app._accept)

    def test_generic_subscription_events_reach_the_public_event_handler(self):
        self.app.on_event = lambda event, data: self.received.put((event, data))
        self.event('e:1')
        self.assertEqual(self.received.get(timeout=2), ('event.changed', {'subscription': 's:1', 'sources': []}))
        self.app.on_observation = self.callback
        self.event('e:2')
        self.assertEqual(self.received.get(timeout=2)[:2], ('event.changed', 'e:2'))

    def test_slow_observation_cannot_starve_cancellation_and_is_bounded(self):
        entered = threading.Event()
        cancelled = threading.Event()
        def slow(*_):
            entered.set()
            self.release.wait(2)
        self.app._subscriptions['s:1'] = slow
        self.event('e:1')
        self.assertTrue(entered.wait(2))
        for index in range(31):
            self.event(f'e:{index + 2}')
        with self.assertRaises(PluginError) as error:
            self.event('e:33')
        self.assertEqual(error.exception.code, 'busy')
        self.app.on_event = lambda *_: cancelled.set()
        self.app._submit({'type': 'event', 'event': 'job.cancel_requested', 'data': {'job': 'j:1'}})
        self.assertTrue(cancelled.wait(2))


if __name__ == '__main__':
    unittest.main(verbosity=2)
