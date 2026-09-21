# SPDX-License-Identifier: MPL-2.0
"""Bounded discovery readiness for real editor fixtures, independent of PTYs."""
import json
from pathlib import Path
import time


def wait_for_workspaces(discover, projects, *, seconds=15, clock=time.monotonic, sleep=time.sleep):
    expected = {Path(project) for project in projects}
    deadline = clock() + seconds
    inventory = {'workspaces': [], 'truncated': False}
    while True:
        remaining = deadline - clock()
        if remaining <= 0:
            break
        # The caller passes this remaining budget to its bounded RPC reader.
        # Errors propagate: only a successful but not-ready inventory is retried.
        inventory = discover(remaining)
        readable = {Path(row['root']) for row in inventory['workspaces'] if row['readable']}
        if expected <= readable and clock() < deadline:
            return inventory
        remaining = deadline - clock()
        if remaining > 0:
            sleep(min(.02, remaining))
    summary = {
        'expected_roots': [root.as_posix()[:200] for root in sorted(expected)[:8]],
        'workspaces': [
            {'root': str(row['root'])[:200], 'readable': bool(row['readable']),
             'unavailable_reason': str(row.get('unavailable_reason'))[:200]}
            for row in inventory['workspaces'][:8]
        ],
        'truncated': bool(inventory.get('truncated', False)),
    }
    raise AssertionError('Fixture workspace discovery exceeded startup deadline: ' + json.dumps(summary))
