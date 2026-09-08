# SPDX-License-Identifier: MPL-2.0
"""Run with python3; enable explicitly as shown in applications.md."""
import threading
from application import Application, PluginError

app = Application('Background jobs', [
    {'name': 'start', 'description': 'Run a twelve-second background job', 'context': 'workspace'},
    {'name': 'cancel', 'description': 'Cancel the last background job', 'context': 'workspace'},
], ['jobs'])
lock = threading.Lock()
cancellations = {}
early_cancellations = {}
last_job = None

def start(_context):
    global last_job
    job = app.request('job.create', title='Twelve-second task', deadline_seconds=60)['job']
    cancelled = threading.Event()
    with lock:
        if early_cancellations.pop(job, False):
            cancelled.set()
        cancellations[job] = cancelled
        last_job = job
    def work():
        state = 'cancelled' if cancelled.wait(12) else 'succeeded'
        try:
            app.request('job.finish', job=job, state=state)
        except PluginError as error:
            if state == 'succeeded' and error.code == 'cancelled':
                # Cancellation can win after the local wait has finished.
                try:
                    app.request('job.finish', job=job, state='cancelled')
                except PluginError:
                    pass
        finally:
            with lock:
                cancellations.pop(job, None)
    try:
        threading.Thread(target=work, daemon=True).start()
    except Exception:
        with lock:
            cancellations.pop(job, None)
        try:
            app.request('job.finish', job=job, state='failed')
        except PluginError as error:
            if error.code == 'cancelled':
                app.request('job.finish', job=job, state='cancelled')
        raise
    return {'job': job}

def cancel(_context):
    with lock:
        job = last_job
    if job is not None:
        app.request('job.cancel', job=job)

def event(name, data):
    if name == 'job.cancel_requested':
        with lock:
            cancelled = cancellations.get(data['job'])
            if cancelled is not None:
                cancelled.set()
            else:
                # The SDK's cancellation lane can beat the job.create caller
                # after the reader receives its reply. Keep that early signal.
                early_cancellations[data['job']] = True
                while len(early_cancellations) > 16:
                    del early_cancellations[next(iter(early_cancellations))]

app.handlers = {'start': start, 'cancel': cancel}
app.on_event = event
if __name__ == '__main__':
    app.run()
