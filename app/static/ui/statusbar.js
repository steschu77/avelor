// The status bar (index.html footer.statusbar), a snackbar: it floats over
// the bottom of the main area and slides in while the store's `notices` has
// one to show -- the newest -- and out when it has none. A notice is
// something the host reported: an error (`showError`, model/state.js; the
// `notice` event, or a failed call nobody handled, below): a failed save,
// settings it couldn't write. It stays until its x dismisses it; the same
// text again doesn't stack up (autosave retries a failed save every 4 s).
// A file changed outside the app is asked about in a dialog instead
// (features/conflict.js).

import { accEl } from '../core/dom.js';
import { titled } from '../core/dom.js';
import { RpcError, rpcOn } from '../core/rpc.js';
import { t, textOf } from '../core/text.js';
import { showError, store } from '../model/state.js';

const statusbarEl = document.querySelector('footer.statusbar');
// The live region that reads a new notice out: always there, unlike the
// bar, which is hidden while it has nothing to tell.
const saidEl = document.getElementById('statusbar-said');

function clearNotice(key) {
  const { notices } = store.getState();
  if (notices.some(n => n.key === key)) {
    store.setState({ notices: notices.filter(n => n.key !== key) });
  }
}

function renderStatusbar(notices) {
  const notice = notices.at(-1);
  // Empty, it slides out with what it showed last.
  statusbarEl.classList.toggle('is-shown', !!notice);
  saidEl.textContent = notice?.text ?? '';
  if (!notice) return;

  statusbarEl.replaceChildren();
  const el = accEl('div', 'statusbar__notice error-note');
  const text = accEl('span', 'statusbar__text', notice.text);
  text.title = notice.text;
  const dismiss = accEl('button', 'tool', '\u00d7');
  dismiss.type = 'button';
  titled(dismiss, t('status.dismiss'));
  dismiss.addEventListener('click', () => clearNotice(notice.key));
  el.append(text, dismiss);
  if (notices.length > 1) el.append(accEl('span', 'meta', t('status.more', { n: notices.length - 1 })));
  statusbarEl.append(el);
}

store.subscribe(state => state.notices, renderStatusbar);
rpcOn('notice', ({ text }) => showError(textOf(text)));

// A failed call nobody handled is shown here too, so no failure is only a
// console line. Cancelled calls are expected, and stay quiet.
window.addEventListener('unhandledrejection', e => {
  const error = e.reason;
  if (!(error instanceof RpcError)) return;
  e.preventDefault();
  if (error.code === 'cancelled') return;
  showError(`${error.cmd}: ${error.message}`);
});
