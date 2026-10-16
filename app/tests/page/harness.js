// The page's tests (development.md#checks): what the page's modules need of
// a page as they load, and `test` and `equal`. A test file imports it first;
// app/tests/page.rs bundles the file and runs it in node. `t` gives a text's
// key, as the catalog has only the tests' own texts, so a wording breaks no
// test.

const { deepStrictEqual } = require('node:assert');

const TEXTS = {
  'test.plain': 'plain',
  'test.named': '{name} of {n}',
  'test.count': { one: '{n} entry', other: '{n} entries' },
  ordinal: { one: '{n}st', two: '{n}nd', few: '{n}rd', other: '{n}th' },
};

// What the page posted to the host, as objects, oldest first.
export const posted = [];

globalThis.window = globalThis;
globalThis.host = { post: text => posted.push(JSON.parse(text)) };
globalThis.document = {
  documentElement: { lang: 'en' },
  getElementById: id => (id === 'texts' ? { textContent: JSON.stringify(TEXTS) } : null),
  querySelector: () => null,
  querySelectorAll: () => [],
  addEventListener() {},
};

// ----------------------------------------------------------------------------
const tests = [];

export function test(name, run) {
  tests.push({ name, run });
}

export function equal(got, want) {
  deepStrictEqual(got, want);
}

let running = 'nothing';

setImmediate(async () => {
  let failed = 0;
  equal(Intl.DateTimeFormat().resolvedOptions().timeZone, process.env.TZ); // the zone page.rs picked is taken
  for (const { name, run } of tests) {
    running = name;
    try {
      await run();
    } catch (e) {
      failed++;
      console.log(`FAIL ${name}\n${e.code === 'ERR_ASSERTION' ? e.message : e.stack}\n`);
    }
  }
  running = null;
  console.log(`${tests.length - failed} of ${tests.length} passed`);
  process.exit(failed || !tests.length ? 1 : 0);
});

// node ends by itself, and without an error, while a test still waits.
process.on('exit', () => {
  if (running === null) return;
  console.log(`FAIL ${running}\nit never ends`);
  process.exitCode = 1;
});
