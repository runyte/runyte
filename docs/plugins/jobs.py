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
last_job = None

def start(_context):
    global last_job
    job = app.request('job.create', title='Twelve-second task', deadline_seconds=60)['job']
    cancelled = threading.Event()
    with lock:
        cancellations[job] = cancelled
        last_job = job
    def work():
        state = 'cancelled' if cancelled.wait(12) else 'succeeded'
        try:
            app.request('job.finish', job=job, state=state)
        except PluginError:
            pass
        finally:
            with lock:
                cancellations.pop(job, None)
    threading.Thread(target=work, daemon=True).start()
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

app.handlers = {'start': start, 'cancel': cancel}
app.on_event = event
if __name__ == '__main__':
    app.run()
