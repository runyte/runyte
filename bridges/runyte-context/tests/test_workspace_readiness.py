# SPDX-License-Identifier: MPL-2.0
"""Discovery readiness regression tests need neither an editor nor a native PTY."""
import unittest

from workspace_readiness import wait_for_workspaces


class Clock:
    def __init__(self):
        self.now = 0
        self.sleeps = []

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.sleeps.append(seconds)
        self.now += seconds


def inventory(*rows, truncated=False):
    return {'workspaces': list(rows), 'truncated': truncated}


def row(root, readable=True, reason=None):
    return {'root': root, 'readable': readable, 'unavailable_reason': reason}


class WorkspaceReadinessTests(unittest.TestCase):
    def test_delayed_discovery_requires_every_exact_root_and_readable_access(self):
        clock = Clock()
        ready = inventory(row('/fixtures/one'), row('/fixtures/two'))
        responses = iter([
            inventory(),
            inventory(row('/elsewhere/one'), row('/fixtures/two')),
            inventory(row('/fixtures/one', False, 'unavailable'), row('/fixtures/two')),
            ready,
        ])
        budgets = []

        def discover(seconds):
            budgets.append(seconds)
            return next(responses)

        self.assertIs(wait_for_workspaces(discover, ['/fixtures/one', '/fixtures/two'],
                                         clock=clock, sleep=clock.sleep), ready)
        self.assertEqual(len(clock.sleeps), 3)
        self.assertEqual(budgets[0], 15)
        self.assertTrue(all(a > b > 0 for a, b in zip(budgets, budgets[1:])))

    def test_missing_workspace_expires_with_bounded_selected_diagnostics(self):
        clock = Clock()
        current = inventory(*[row('/other/' + 'x' * 1000, False, 'denied' * 1000)] * 100,
                            truncated=True)
        current['private_credential'] = 'never include unrestricted inventory fields'
        with self.assertRaises(AssertionError) as failure:
            wait_for_workspaces(lambda seconds: current, ['/fixtures/one'], seconds=.05,
                                clock=clock, sleep=clock.sleep)
        message = str(failure.exception)
        self.assertIn('/fixtures/one', message)
        self.assertIn('"readable": false', message)
        self.assertIn('"unavailable_reason": "denied', message)
        self.assertIn('"truncated": true', message)
        self.assertNotIn('private_credential', message)
        self.assertNotIn('never include', message)
        self.assertLess(len(message), 5000)
        self.assertAlmostEqual(clock.now, .05)

    def test_discovery_errors_propagate_without_retry(self):
        clock = Clock()
        error = RuntimeError('RPC failure')

        def discover(seconds):
            raise error

        with self.assertRaises(RuntimeError) as failure:
            wait_for_workspaces(discover, ['/fixtures/one'], clock=clock, sleep=clock.sleep)
        self.assertIs(failure.exception, error)
        self.assertEqual(clock.sleeps, [])

    def test_late_success_cannot_extend_the_startup_deadline(self):
        clock = Clock()

        def discover(seconds):
            self.assertEqual(seconds, 15)
            clock.now += seconds
            return inventory(row('/fixtures/one'))

        with self.assertRaisesRegex(AssertionError, 'startup deadline'):
            wait_for_workspaces(discover, ['/fixtures/one'], clock=clock, sleep=clock.sleep)
        self.assertEqual(clock.sleeps, [])
