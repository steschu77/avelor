import { equal, test } from './harness.js';
import { newProjectBlock } from '../../static/model/blueprint.js';

const BLUEPRINT = { name: 'move', checks: ['deadlines', 'documents'], labels: ['move'] };

test('a project takes the checks and the labels of its blueprint', () => {
  equal(newProjectBlock(BLUEPRINT), { blueprint: 'move', checks: ['deadlines', 'documents'], labels: ['move'] });
  equal(newProjectBlock({ name: 'plain', checks: [], labels: [] }), { blueprint: 'plain' });
});
