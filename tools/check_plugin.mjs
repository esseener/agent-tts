// Validate the generated adapter without starting or modifying OpenCode.
import { readFileSync } from 'node:fs';
import { win32 } from 'node:path';
import assert from 'node:assert/strict';
const source = readFileSync(process.argv[2], 'utf8')
  .replace('import { Plugin } from "@opencode/plugin";', '')
  .replace('import { spawn } from "node:child_process";', '')
  .replace('import { win32 } from "node:path";', '')
  .replace('export default', 'return');
const calls = [];
const spawn = (executable, args, options) => {
  const child = { handlers: new Map(), on(name, fn) { this.handlers.set(name, fn); return this; }, unref() {} };
  calls.push({ executable, args, options, child });
  return child;
};
const load = () => new Function('Plugin', 'spawn', 'win32', source)({ define: value => value }, spawn, win32);
const location = { directory: 'C:/WORK/', workspaceID: 'wrk_here' };
const done = (id, seq, from = location) => ({
  id, location: from, type: 'session.execution.succeeded', data: { sessionID: 'ses_test' },
  durable: { aggregateID: 'ses_test', seq },
});
const ask = (id, permissionID) => ({
  id, type: 'permission.asked', data: { id: permissionID, sessionID: 'ses_test' },
});
const events = [
  done('evt_foreign', 1, { directory: 'C:/not-loaded' }),
  done('evt_wrong_workspace', 2, { ...location, workspaceID: 'wrk_other' }),
  done('evt_first', 3),
  done('evt_first_copy', 3),
  { ...done('evt_failure', 4), type: 'session.execution.failed' },
  done('evt_next_turn', 5),
  ask('evt_ask', 'per_first'),
  ask('evt_ask_copy', 'per_first'),
  ask('evt_ask_next', 'per_next'),
];
const signals = [];
const attach = (directory, stream = events) => load().setup({
  location: { directory, workspaceID: 'wrk_here' },
  event: {
    async *subscribe(options) {
      assert.deepEqual(options.types, ['session.execution.succeeded', 'session.execution.failed', 'permission.asked']);
      signals.push(options.signal);
      yield* stream;
    },
  },
});
const cleanups = ['C:/work', 'C:/elsewhere', 'C:/another', 'C:\\work'].map(directory => attach(directory));
await new Promise(resolve => setImmediate(resolve));
const kinds = calls.map(call => call.args[call.args.indexOf('--play-event') + 2]);
assert.equal(kinds[0], 'done', 'The first valid completion is not delayed');
assert.deepEqual(kinds.sort(), ['ask', 'ask', 'done', 'done', 'error'],
  'Ignore other locations and keep distinct turns and permissions');
assert(calls.every(call => call.options.windowsHide && call.options.stdio === 'ignore'));
const flag = (call, name) => call.args[call.args.indexOf(name) + 1];
assert.equal(new Set(calls.map(call => flag(call, '--event-id'))).size, 5,
  'Distinct turns and permissions keep distinct logical identities');
assert(calls.every(call => flag(call, '--session-id') === 'ses_test'));

calls[0].child.handlers.get('exit')(1);
cleanups.push(attach('C:/work', [events[2]]));
await new Promise(resolve => setImmediate(resolve));
assert.equal(calls.length, 6, 'A failed playback permits the same event to retry');
assert.equal(flag(calls[5], '--event-id'), flag(calls[0], '--event-id'));
calls[5].child.handlers.get('exit')(0);
cleanups.push(attach('C:/work', [events[2]]));
await new Promise(resolve => setImmediate(resolve));
assert.equal(calls.length, 6, 'A successful retry suppresses a subsequent replay');

calls[3].child.handlers.get('error')(new Error('simulated spawn failure'));
cleanups.push(attach('C:/work', [events[6]]));
await new Promise(resolve => setImmediate(resolve));
assert.equal(calls.length, 7, 'A failed process launch releases its event identity');
assert.equal(flag(calls[6], '--event-id'), flag(calls[3], '--event-id'));
for (const cleanup of cleanups) cleanup();
assert.equal(signals.length, cleanups.length);
assert(signals.every(signal => signal.aborted));
console.log('OpenCode V2 adapter isolates logical events, coalesces pending deliveries, retries failures, and cleans up subscriptions');
