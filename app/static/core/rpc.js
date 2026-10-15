// The page's one door to the host (api.md#the-page-side):
// `rpcCall` sends a command and returns a promise of its one reply.

// A failed call: `code` is the host's (`cancelled`, `bad_request`, ...).
class RpcError extends Error {
  constructor(cmd, code, message) {
    super(message || code);
    this.name = 'RpcError';
    this.cmd = cmd;
    this.code = code;
  }
}

const rpcBridge = window.host;
let rpcNextId = 1;
const rpcPending = new Map(); // id -> { cmd, resolve, reject }

// Send `cmd` with `args`. The promise resolves with the reply's value and
// rejects with an RpcError. Without a host (a test page) a call is
// `cancelled`.
export function rpcCall(cmd, args = {}, { channel } = {}) {
  const id = rpcNextId++;
  const reply = new Promise((resolve, reject) => rpcPending.set(id, { cmd, resolve, reject }));
  if (rpcBridge) rpcBridge.post(JSON.stringify({ id, cmd, args }));
  else rpcSettle(id, { err: { code: 'cancelled', message: 'There is no host.' } });
  return reply;
}

// Settle the call `id` with `reply`: `{ ok }` or `{ err: { code, message } }`.
function rpcSettle(id, reply) {
  const call = rpcPending.get(id);
  if (!call) return;
  rpcPending.delete(id);
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
  };
}
