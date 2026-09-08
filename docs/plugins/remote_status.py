# SPDX-License-Identifier: MPL-2.0
"""Bounded phase-only publication of remote download status in an existing view."""
import threading
import time

from application import PluginError

STATUS_ROW = 'download-status'
STATUS_HEADROOM = 512


class DownloadStatus:
    def __init__(self, owner):
        self.owner = owner
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
        messages = {
            'transferring': ('Downloading remote file', 'muted'),
            'ready': (f'Download ready · run :plugin.{self.owner.plugin_id}.confirm-download', 'heading'),
            'failed': (f'Download failed · retry :plugin.{self.owner.plugin_id}.download', 'error'),
            'cancelled': ('Download cancelled', 'muted'),
        }
        if phase in messages:
            text, role = messages[phase]
            rows.append({'id': STATUS_ROW, 'text': text, 'role': role})
        return {**base, 'rows': rows}

    def __call__(self, phase):
        if phase not in ('transferring', 'ready', 'failed', 'cancelled', 'completed'):
            return
        # A Downloads callback can hold its own lock. Never acquire the browser
        # lock or wait for a host response at this boundary.
        with self.lock:
            if phase == 'transferring':
                self.generation += 1
            self.phase = phase
            self.serial += 1
            self.pending = (self.serial, self.generation, self.owner.view, phase)
            if self.active:
                return
            self.active = True
            self.idle.clear()
        try:
            threading.Thread(target=self.publish, name='runyte-download-status', daemon=True).start()
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
                            model=self.model(self.owner.model, phase))
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
