// The status bar (index.html footer.statusbar), a snackbar: it floats over
// the bottom of the main area and slides in while the store's `notices` has
// one to show -- the newest -- and out when it has none. A notice is
// something the host reported: an error (`showError`, model/state.js; the
// `notice` event, or a failed call nobody handled, below), like settings
// it couldn't write. It stays until its x dismisses it; the same text again
// doesn't stack up.

import { accEl } from '../core/dom.js';
import { RpcError, rpcOn } from '../core/rpc.js';
import { showError, store } from '../model/state.js';

const statusbarEl = document.querySelector('footer.statusbar');

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
  if (!notice) return;

  statusbarEl.replaceChildren();
  const el = accEl('div', 'statusbar__notice error-note');
  const text = accEl('span', 'statusbar__text', notice.text);
  text.title = notice.text;
  const dismiss = accEl('button', 'tool', '\u00d7');
  dismiss.type = 'button';
  dismiss.title = 'Dismiss';
  dismiss.addEventListener('click', () => clearNotice(notice.key));
  el.append(text, dismiss);
  if (notices.length > 1) el.append(accEl('span', 'meta', `+${notices.length - 1} more`));
  statusbarEl.append(el);
}

store.subscribe(state => state.notices, renderStatusbar);
rpcOn('notice', ({ message }) => showError(message));

// A failed call nobody handled is shown here too, so no failure is only a
// console line. Cancelled calls are expected, and stay quiet.
window.addEventListener('unhandledrejection', e => {
  const error = e.reason;
  if (!(error instanceof RpcError)) return;
  e.preventDefault();
  if (error.code === 'cancelled') return;
  showError(`${error.cmd}: ${error.message}`);
});
