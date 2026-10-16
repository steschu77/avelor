// The page's one door to the host (api.md#the-page-side):
// `rpcCall` sends a command and returns a promise of its one reply;
// `rpcOn` listens for the events the host pushes.

import { textOf } from './text.js';

// A failed call: `code` is the host's (`cancelled`, `bad_request`, ...).
export class RpcError extends Error {
  constructor(cmd, code, text) {
    super(textOf(text) || code);
    this.name = 'RpcError';
    this.cmd = cmd;
    this.code = code;
  }
}

const rpcBridge = window.host;
let rpcNextId = 1;
const rpcPending = new Map(); // id -> { cmd, resolve, reject }
const rpcListeners = new Map(); // event -> Set of listeners

// Send `cmd` with `args`; `step` names the group of edits its edit is one
// step with (saving.md#undo). The promise resolves with the reply's value
// and rejects with an RpcError. Without a host (a test page) a call is
// `cancelled`.
export function rpcCall(cmd, args = {}, { channel, step } = {}) {
  const id = rpcNextId++;
  const reply = new Promise((resolve, reject) => rpcPending.set(id, { cmd, resolve, reject }));
  if (rpcBridge) rpcBridge.post(JSON.stringify({ id, cmd, args, step }));
  else rpcSettle(id, { err: { code: 'cancelled' } });
  return reply;
}

// Call `fn(data)` for every `event` the host pushes, after the listeners
// that came before it.
export function rpcOn(event, fn) {
  if (!rpcListeners.has(event)) rpcListeners.set(event, new Set());
  rpcListeners.get(event).add(fn);
}

// Settle the call `id` with `reply`: `{ ok }` or `{ err: { code, text } }`.
function rpcSettle(id, reply) {
  const call = rpcPending.get(id);
  if (!call) return;
  rpcPending.delete(id);
  if (reply.err) call.reject(new RpcError(call.cmd, reply.err.code, reply.err.text));
  else call.resolve(reply.ok);
}

if (rpcBridge) {
  rpcBridge.receive = text => {
    let msg;
    try {
      msg = JSON.parse(text);
    } catch (error) {
      console.warn('rpc: not JSON from the host', error);
      return;
    }
    if ('re' in msg) rpcSettle(msg.re, msg);
    else if ('ev' in msg) rpcListeners.get(msg.ev)?.forEach(fn => fn(msg.data));
  };
}
