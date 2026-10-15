import { equal, posted, test } from './harness.js';
import { rpcCall, rpcOn } from '../../static/core/rpc.js';

const receive = message => globalThis.host.receive(JSON.stringify(message));

test('a call posts its command, and the reply settles it', async () => {
  const reply = rpcCall('ping', { n: 1 });
  const { id, cmd, args } = posted.at(-1);
  equal([cmd, args], ['ping', { n: 1 }]);
  receive({ re: id, ok: 'pong' });
  equal(await reply, 'pong');
});

test('a refused call rejects with its command and the code', async () => {
  const reply = rpcCall('open');
  receive({ re: posted.at(-1).id, err: { code: 'bad_request' } });
  const error = await reply.catch(e => e);
  equal([error.name, error.cmd, error.code], ['RpcError', 'open', 'bad_request']);
});

test('a call is settled once, and a reply to none is left out', async () => {
  const reply = rpcCall('once');
  const { id } = posted.at(-1);
  receive({ re: id, ok: 1 });
  receive({ re: id, ok: 2 });
  receive({ re: 4711, ok: 3 });
  equal(await reply, 1);
});

test('an event goes to its listeners in the order they came', () => {
  const heard = [];
  rpcOn('told', data => heard.push(['first', data]));
  rpcOn('told', data => heard.push(['second', data]));
  rpcOn('other', data => heard.push(['other', data]));
  receive({ ev: 'told', data: 7 });
  receive({ ev: 'unheard', data: 8 });
  equal(heard, [['first', 7], ['second', 7]]);
});
