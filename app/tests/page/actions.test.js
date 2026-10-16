import { equal, test } from './harness.js';
import { registerAction, runAction } from '../../static/core/actions.js';

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
