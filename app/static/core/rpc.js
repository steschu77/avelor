// The page's one door to the host (api.md#the-page-side):
// `rpcCall` sends a command and returns a promise of its one reply;
// `rpcOn` listens for the events the host pushes.

// A failed call: `code` is the host's (`cancelled`, `bad_request`, ...),
// or `superseded` for a call a newer one on its channel replaced.
export class RpcError extends Error {
  constructor(cmd, code, message) {
    super(message || code);
    this.name = 'RpcError';
    this.cmd = cmd;
    this.code = code;
  }
}

const rpcBridge = window.host;
let rpcNextId = 1;
const rpcPending = new Map(); // id -> { cmd, channel, resolve, reject }
const rpcLatest = new Map(); // channel -> id of its latest call
const rpcListeners = new Map(); // event -> Set of listeners

// Send `cmd` with `args`. The promise resolves with the reply's value and
// rejects with an RpcError. Of the calls on one `channel` only the latest
// counts: the one before is rejected as `superseded` at once, and its late
// reply is ignored. Without a host (a test page) a call is `cancelled`.
export function rpcCall(cmd, args = {}, { channel } = {}) {
  const id = rpcNextId++;
  if (channel) {
    rpcSettle(rpcLatest.get(channel), { err: { code: 'superseded' } });
    rpcLatest.set(channel, id);
  }
  const reply = new Promise((resolve, reject) => rpcPending.set(id, { cmd, channel, resolve, reject }));
  if (rpcBridge) rpcBridge.post(JSON.stringify({ id, cmd, args }));
  else rpcSettle(id, { err: { code: 'cancelled', message: 'There is no host.' } });
  return reply;
}

// Call `fn(data)` for every `event` the host pushes, after the listeners
// that came before it.
export function rpcOn(event, fn) {
  if (!rpcListeners.has(event)) rpcListeners.set(event, new Set());
  rpcListeners.get(event).add(fn);
}

// Settle the call `id` with `reply`: `{ ok }` or `{ err: { code, message } }`.
// A call settled before (superseded) is no longer pending and stays as it is.
function rpcSettle(id, reply) {
  const call = rpcPending.get(id);
  if (!call) return;
  rpcPending.delete(id);
  if (call.channel && rpcLatest.get(call.channel) === id) rpcLatest.delete(call.channel);
  if (reply.err) call.reject(new RpcError(call.cmd, reply.err.code, reply.err.message));
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
