# SPDX-License-Identifier: MPL-2.0
"""Bounded phase publication for remote application work in an existing view."""
import threading
import time

from application import PluginError

STATUS_ROW = 'download-status'
STATUS_HEADROOM = 512


class PhaseStatus:
    def __init__(self, owner, row, messages, start):
        self.owner, self.row, self.messages, self.start = owner, row, messages, start
        self.lock = threading.Lock()
        self.phase = None
        self.generation = self.serial = 0
        self.pending = None
        self.active = False
        self.idle = threading.Event()
        self.idle.set()

    def model(self, base, phase=None):
        if phase is None:
            with self.lock:
                phase = self.phase
        rows = list(base['rows'])
        if phase in self.messages:
            text, role = self.messages[phase]
            rows.append({'id': self.row, 'text': text, 'role': role})
        return {**base, 'rows': rows}

    def __call__(self, phase):
        if phase not in self.messages and phase != 'completed':
            return
        # A Downloads callback can hold its own lock. Never acquire the browser
        # lock or wait for a host response at this boundary.
        with self.lock:
            if phase == self.start:
                self.generation += 1
            self.phase = phase
            self.serial += 1
            self.pending = (self.serial, self.generation, self.owner.view, phase)
            if self.active:
                return
            self.active = True
            self.idle.clear()
        try:
            threading.Thread(target=self.publish, name='runyte-remote-status', daemon=True).start()
        except Exception:
            with self.lock:
                self.active = False
                self.idle.set()

    def publish(self):
        attempted, retries = None, 0
        while True:
            with self.lock:
                pending = self.pending
            serial, generation, view, phase = pending
            if serial != attempted:
                attempted, retries = serial, 0
            busy = False
            try:
                with self.owner.lock:
                    with self.lock:
                        if self.pending != pending or self.generation != generation:
                            continue
                    # A close/recreate cannot receive an old publication. A
                    # concurrent directory refresh uses the latest retained rows.
                    if view is not None and self.owner.view == view and self.owner.model is not None:
                        result = self.owner.app.request('view.publish', view=view,
                            expected_revision=self.owner.revision,
                            model=(self.owner.status_model((self, phase))
                                   if hasattr(self.owner, 'status_model')
                                   else self.model(self.owner.model, phase)))
                        self.owner.revision = result['revision']
            except PluginError as error:
                busy = error.code == 'busy'
            except Exception:
                # Status cannot invalidate a safely prepared transfer.
                pass
            if busy and retries < 2:
                retries += 1
                # Active-work backoff only: at most two retries for a phase,
                # then no further wakeups until another actual phase changes.
                time.sleep(0.1)
                continue
            with self.lock:
                if self.pending == pending:
                    self.pending = None
                    self.active = False
                    self.idle.set()
                    return


class DownloadStatus(PhaseStatus):
    def __init__(self, owner):
        super().__init__(owner, STATUS_ROW, {
            'transferring': ('Downloading remote file', 'muted'),
            'ready': (f'Download ready · run :plugin.{owner.plugin_id}.confirm-download', 'heading'),
            'failed': (f'Download failed · retry :plugin.{owner.plugin_id}.download', 'error'),
            'cancelled': ('Download cancelled', 'muted'),
        }, 'transferring')


class OperationStatus(PhaseStatus):
    def __init__(self, owner):
        super().__init__(owner, 'operation-status', {
            'preparing': ('Inspecting remote operation', 'muted'),
            'ready': (f'Remote operation ready · run :plugin.{owner.plugin_id}.confirm-operation', 'heading'),
            'confirming': ('Review the remote operation in the native confirmation', 'muted'),
            'applying': ('Applying remote operation', 'muted'),
            'completed': (f'Remote operation completed · run :plugin.{owner.plugin_id}.refresh', 'heading'),
            'failed': ('Remote operation failed · inspect remote state before trying again', 'error'),
            'cancelled': ('Remote operation cancelled', 'muted'),
            'outcome_unknown': ('Remote operation outcome unknown · inspect remote state; do not retry', 'error'),
        }, 'preparing')
