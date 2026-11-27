import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { store } from '../../static/model/state.js';
import { currentDraft, dropDraft, openDraft } from '../../static/model/drafts.js';

const project = id => ({ id, kind: 'project', title: id, project: {} });

test('a draft starts in the selected project, or in the one New here names', () => {
  vault(project('p'), project('q'));
  store.setState({ selected_project: 'q' });
  openDraft('task');
  equal(currentDraft().projects, ['q']);
  openDraft('task', { project: 'p' });
  equal(currentDraft().projects, ['p']);
  store.setState({ selected_project: null });
  openDraft('document');
  equal(currentDraft().projects, []);
  dropDraft();
});
