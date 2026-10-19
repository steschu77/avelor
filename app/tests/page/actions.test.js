import { equal, test } from './harness.js';
import {
  keyedActions, listedActions, registerAction, registerActions, runAction, shortcutHint, shortcutKeys, shortcutOf, shortcutText,
} from '../../static/core/actions.js';

const press = (key, held = {}) => shortcutOf({ key, ctrlKey: false, altKey: false, shiftKey: false, ...held });

test('the shortcut a key event stands for', () => {
  equal([press('n', { ctrlKey: true }), press('Z', { ctrlKey: true, shiftKey: true }), press('Escape'), press('1', { ctrlKey: true, altKey: true })],
    ['Ctrl+N', 'Ctrl+Shift+Z', 'Escape', 'Ctrl+Alt+1']);
});

test("a shortcut's keys as the run's language names them", () => {
  equal([shortcutKeys('Ctrl+Enter'), shortcutText('Ctrl+Shift+Z')], [['key.ctrl', 'key.enter'], 'key.ctrl+key.shift+Z']);
});

test('the keys told by a sign', () => {
  equal(shortcutKeys('Ctrl+Plus+Minus+Up+Left'), ['key.ctrl', '+', '\u2212', '\u2191', '\u2190']);
});

test('an action runs while it applies, with what its caller gives', () => {
  const ran = [];
  let applies = false;
  registerAction({ id: 'run.it', when: () => applies, run: (...args) => ran.push(args) });
  runAction('run.it', 1);
  applies = true;
  runAction('run.it', 2, 3);
  runAction('run.gone');
  equal(ran, [[2, 3]]);
});

test('the actions listed are those with a group that apply, what the vault has first', () => {
  registerAction({ id: 'own', group: 'window', title: () => 'Own', also: 'one, two', shortcut: 'Ctrl+M', run() {} });
  registerAction({ id: 'not now', group: 'window', title: 'Not now', when: () => false, run() {} });
  registerAction({ id: 'keyed', title: 'Keyed', shortcut: 'Ctrl+K', run() {} });
  registerActions(() => [{ id: 'source', group: 'get', title: 'Source', run() {} }]);
  equal(listedActions().map(({ id, title, also, shortcuts }) => [id, title, also, shortcuts]),
    [['source', 'Source', [], []], ['own', 'Own', ['one', 'two'], ['Ctrl+M']]]);
  equal(shortcutHint('own'), 'KEY.CTRL+M');
});

test('the actions with a key are listed though they do not apply, without what they run', () => {
  registerAction({ id: 'asleep', group: 'go', title: 'Asleep', shortcut: ['F8', 'Ctrl+8'], when: () => false, run() {} });
  equal(keyedActions().map(({ id, shortcuts, run }) => [id, shortcuts, typeof run]).filter(([id]) => id === 'asleep'), [['asleep', ['F8', 'Ctrl+8'], 'object']]);
});

test('a place is listed without a group', () => {
  registerAction({ id: 'place', place: 'lens', title: 'Lens', run() {} });
  equal(listedActions().some(action => action.id === 'place'), true);
});
