// SPDX-License-Identifier: MPL-2.0
// Epoch 2 without an SDK: Node built-ins, continuous bounded reader, async handlers.
// Run with `node docs/plugins/tasks.mjs`; stdout belongs exclusively to the protocol.
const VERSION = 'runyte-experimental-2';
const LIMIT = 1024 * 1024;
const MAX_REQUESTS = 16;
const decoder = new TextDecoder('utf-8', {fatal: true});
const input = Buffer.alloc(LIMIT);
let used = 0, phase = 'hello', closed = false, sequence = 0, dispatching = 0;
let writing = false, outgoingBytes = 0, outgoingCount = 0;
const outbox = [], pending = new Map(), activeCommands = new Set();
const tasks = [
  ['read', 'Read the plugin guide'],
  ['edit', 'Edit a document'],
  ['split', 'Split this task list'],
];
let done = new Set(), view = null, revision = null, generation = 0, busy = false;
let creation = null, uncertain = false;

class Refusal extends Error {
  constructor(code, message) { super(message); this.code = code; }
}
function shutdown(failed = false) {
  if (closed) return;
  closed = true;
  process.exitCode = failed ? 1 : 0;
  for (const entry of pending.values()) {
    clearTimeout(entry.timer);
    entry.reject(new Refusal('unavailable', 'Host disconnected'));
  }
  pending.clear();
  outbox.length = 0;
  process.stdin.destroy();
  process.stdout.destroy();
}
function flush() {
  if (closed || writing || outbox.length === 0) return;
  const bytes = outbox.shift();
  writing = true;
  // One outstanding write keeps Node's own Writable queue bounded too. Charge
  // the in-flight bytes/message until its callback, not merely until dequeue.
  process.stdout.write(bytes, error => {
    outgoingBytes -= bytes.length;
    outgoingCount--;
    writing = false;
    if (error) shutdown(true);
    else flush();
  });
}
function send(message) {
  if (closed) throw new Refusal('unavailable', 'Host disconnected');
  const bytes = Buffer.from(JSON.stringify(message) + '\n');
  if (bytes.length > LIMIT || outgoingCount >= 16 || outgoingBytes + bytes.length > LIMIT) {
    shutdown(true);
    throw new Refusal('limit_exceeded', 'Protocol output capacity exceeded');
  }
  outgoingBytes += bytes.length;
  outgoingCount++;
  outbox.push(bytes);
  flush();
}
function request(method, params, deadline) {
  if (closed) return Promise.reject(new Refusal('unavailable', 'Host disconnected'));
  if (pending.size >= MAX_REQUESTS || sequence >= Number.MAX_SAFE_INTEGER) {
    return Promise.reject(new Refusal('busy', 'Too many host requests'));
  }
  const id = `p:${++sequence}`;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Refusal('timeout', 'Host request timed out'));
    }, Math.max(0, deadline - performance.now()));
    pending.set(id, {resolve, reject, timer, method});
    try { send({type: 'request', id, method, params}); }
    catch (error) { clearTimeout(timer); pending.delete(id); reject(error); }
  });
}
function model(candidate = done) {
  return {title: 'Node tasks', purpose: 'list', rows: tasks.map(([id, label]) => ({
    id, text: `${candidate.has(id) ? '[x]' : '[ ]'} ${label}`,
    role: candidate.has(id) ? 'muted' : 'ordinary',
  }))};
}
function validView(result) {
  return result && typeof result.view === 'string' && result.view.length <= 256
    && typeof result.revision === 'string' && /^m:\d+$/.test(result.revision);
}
async function command(message) {
  const context = message.params;
  if (!context || typeof context !== 'object' || message.method !== 'command.invoke') {
    throw new Refusal('unsupported', 'Only registered commands are supported');
  }
  if (busy) throw new Refusal('busy', 'A task update is already pending');
  busy = true;
  const capturedGeneration = generation;
  // Leave headroom inside the host's ten-second callback lifetime. Both create
  // and presentation share this one deadline; neither can extend the callback.
  const deadline = performance.now() + 8000;
  try {
    if (uncertain) throw new Refusal('unavailable', 'Task publication is uncertain; restart this plugin');
    if (context.command === 'open') {
      if (context.context !== 'workspace') throw new Refusal('invalid_argument', 'Open needs workspace context');
      if (view === null) {
        creation = {view: null};
        const created = await request('view.create', {model: model()}, deadline);
        if (!validView(created)) throw new Refusal('invalid_argument', 'Invalid view result');
        if (generation !== capturedGeneration) throw new Refusal('stale', 'Task view closed while opening');
        view = created.view;
        revision = created.revision;
        creation = null;
      }
      // The original command remains pending until this response. The host
      // owns the foreground check; no later invocation or fallback is invented.
      await request('pane.show', {invocation: message.id, view}, deadline);
    } else if (context.command === 'toggle') {
      if (context.context !== 'view' || view === null || context.view !== view || context.model_revision !== revision) {
        throw new Refusal('stale', 'Task view changed; invoke the action again');
      }
      const ids = new Set(tasks.map(([id]) => id));
      if (!Array.isArray(context.rows) || context.rows.length < 1 || context.rows.length > tasks.length
          || new Set(context.rows).size !== context.rows.length || context.rows.some(id => !ids.has(id))) {
        throw new Refusal('invalid_argument', 'Select current task rows');
      }
      const candidate = new Set(done);
      for (const id of context.rows) {
        if (candidate.has(id)) candidate.delete(id); else candidate.add(id);
      }
      const capturedView = view, capturedRevision = revision;
      let published;
      try {
        published = await request('view.publish', {
          view: capturedView, expected_revision: capturedRevision, model: model(candidate),
        }, deadline);
      } catch (error) {
        if (['timeout', 'outcome_unknown'].includes(error.code)) uncertain = true;
        throw error;
      }
      if (generation !== capturedGeneration || view !== capturedView || revision !== capturedRevision) {
        throw new Refusal('stale', 'Task view closed while publishing');
      }
      if (!validView(published) || published.view !== capturedView || published.revision === capturedRevision) {
        uncertain = true;
        throw new Refusal('outcome_unknown', 'Invalid publication acknowledgement; restart this plugin');
      }
      // Commit only after accepted publication. Failed/stale publication leaves
      // the candidate isolated, so the next attempt cannot double-toggle data.
      done = candidate;
      revision = published.revision;
    } else {
      throw new Refusal('not_found', 'Unknown task command');
    }
  } catch (error) {
    if (creation && ['timeout', 'outcome_unknown'].includes(error.code)) uncertain = true;
    throw error;
  } finally { creation = null; busy = false; }
}
function dispatch(message) {
  if (typeof message.id !== 'string' || !/^h:\d+$/.test(message.id) || message.id.length > 128
      || activeCommands.has(message.id) || dispatching >= 16) {
    shutdown(true);
    return;
  }
  dispatching++;
  activeCommands.add(message.id);
  // Do not await this from the reader: its host responses arrive through that
  // same reader. A separate bounded guard serializes actual checklist changes.
  command(message).then(() => {
    if (!closed) send({type: 'response', id: message.id, result: {job: null}});
  }).catch(error => {
    if (!closed) send({type: 'response', id: message.id, error: {
      code: error instanceof Refusal ? error.code : 'internal',
      message: error instanceof Refusal ? error.message : 'Task command failed',
    }});
  }).catch(() => shutdown(true)).finally(() => {
    dispatching--;
    activeCommands.delete(message.id);
  });
}
function receive(message) {
  if (!message || typeof message !== 'object' || Array.isArray(message)) throw new Error();
  if (phase === 'hello') {
    if (message.type !== 'hello' || message.version !== VERSION
        || !Array.isArray(message.capabilities) || !message.capabilities.includes('views')) throw new Error();
    send({type: 'register', version: VERSION, name: 'Node tasks', commands: [
      {name: 'open', description: 'Open task list', context: 'workspace'},
      {name: 'toggle', description: 'Toggle selected tasks', context: 'view', primary: true},
    ], required_capabilities: ['views'], optional_capabilities: []});
    phase = 'registering';
    return;
  }
  if (phase === 'registering') {
    if (message.type !== 'registered' || !Array.isArray(message.capabilities)
        || !message.capabilities.includes('views')) throw new Error();
    phase = 'ready';
    return;
  }
  if (message.type === 'response') {
    const entry = pending.get(message.id);
    if (!entry) {
      // Timed-out requests may settle later. Never use their result to mutate
      // the retained model or to manufacture a new foreground authorization.
      const match = typeof message.id === 'string' && /^p:(\d+)$/.exec(message.id);
      if (!match || Number(match[1]) > sequence) throw new Error();
      return;
    }
    if (Object.hasOwn(message, 'result') === Object.hasOwn(message, 'error')) throw new Error();
    pending.delete(message.id);
    clearTimeout(entry.timer);
    if (Object.hasOwn(message, 'result')) {
      // Record the issued handle before Promise continuations run: a following
      // FIFO close in this same input chunk must invalidate that exact create.
      if (entry.method === 'view.create' && creation && validView(message.result)) {
        creation.view = message.result.view;
      }
      entry.resolve(message.result);
    }
    else {
      const allowed = new Set(['stale', 'context_changed', 'closed', 'busy', 'unavailable', 'limit_exceeded', 'timeout', 'outcome_unknown']);
      entry.reject(new Refusal(allowed.has(message.error?.code) ? message.error.code : 'unavailable',
                               'Host refused the task request'));
    }
  } else if (message.type === 'request') dispatch(message);
  else if (message.type === 'event') {
    if (message.event === 'view.closed') {
      if (typeof message.data?.view !== 'string') throw new Error();
      if (message.data.view === view || message.data.view === creation?.view) {
        generation++;
        if (message.data.view === view) { view = null; revision = null; }
      }
    }
  } else throw new Error();
}
process.stdin.on('data', chunk => {
  try {
    let offset = 0;
    while (offset < chunk.length && !closed) {
      const newline = chunk.indexOf(10, offset);
      const end = newline < 0 ? chunk.length : newline + 1;
      const count = end - offset;
      if (used + count > LIMIT) throw new Error();
      chunk.copy(input, used, offset, end);
      used += count;
      offset = end;
      if (newline >= 0) {
        receive(JSON.parse(decoder.decode(input.subarray(0, used - 1))));
        used = 0;
      } else if (used === LIMIT) throw new Error();
    }
  } catch { shutdown(true); }
});
process.stdin.on('end', () => shutdown(used !== 0));
process.stdin.on('error', () => shutdown(true));
process.stdout.on('error', () => shutdown(true));
