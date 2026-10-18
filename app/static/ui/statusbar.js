// The status bar (index.html footer.statusbar), a snackbar: it floats over
// the bottom of the main area and slides in while the store's `notices` has
// one to show -- the newest -- and out when it has none. A notice is
// something the host reported:
// - an error (`showError`, model/state.js; the `notice` event at level
//   error, or a failed call nobody handled, below): a failed save,
//   settings it couldn't write, a mail a fetch couldn't write. It stays
//   until its x dismisses it; the same text again doesn't stack up (autosave
//   retries a failed save every 4 s).
// - what a job brought (the `notice` event at level info): new mail. It
//   goes on its own after `NOTICE_INFO_MS`, not while the mouse is over the
//   bar; one about a single entity shows that entity in the Documents lens
//   on a click (the `documents.show` action, features/lenses/documents.js's
//   `showInDocuments`).
// A file changed outside the app is asked about in a dialog instead
// (features/conflict.js).

import { accEl } from '../core/dom.js';
import { RpcError, rpcOn } from '../core/rpc.js';
import { runAction } from '../core/actions.js';
import { showError, store } from '../model/state.js';
import { setNotice } from '../model/state.js';

const NOTICE_INFO_MS = 6000;

const statusbarEl = document.querySelector('footer.statusbar');
const noticeTimers = new Map(); // key of an info notice -> the timeout that removes it
let statusbarHovered = false;

function clearNotice(key) {
  clearTimeout(noticeTimers.get(key));
  noticeTimers.delete(key);
  const { notices } = store.getState();
  if (notices.some(n => n.key === key)) {
    store.setState({ notices: notices.filter(n => n.key !== key) });
  }
}

// What a job brought; `entity`: the one a click on it shows in Documents.
function showInfo(text, entity = null) {
  const key = `info\n${text}`;
  setNotice({ key, text, level: 'info', entity });
  startNoticeTimer(key);
}

// (Re)start the time the info notice `key` is shown; while the mouse is
// over the bar, it waits until the mouse leaves.
function startNoticeTimer(key) {
  clearTimeout(noticeTimers.get(key));
  noticeTimers.delete(key);
  if (!statusbarHovered) noticeTimers.set(key, setTimeout(() => clearNotice(key), NOTICE_INFO_MS));
}

statusbarEl.addEventListener('mouseenter', () => {
  statusbarHovered = true;
  for (const timer of noticeTimers.values()) clearTimeout(timer);
  noticeTimers.clear();
});
statusbarEl.addEventListener('mouseleave', () => {
  statusbarHovered = false;
  for (const notice of store.getState().notices) {
    if (notice.level === 'info') startNoticeTimer(notice.key);
  }
});

function renderStatusbar(notices) {
  const notice = notices.at(-1);
  // Empty, it slides out with what it showed last.
  statusbarEl.classList.toggle('is-shown', !!notice);
  if (!notice) return;

  statusbarEl.replaceChildren();
  const el = accEl('div', notice.level === 'info' ? 'statusbar__notice' : 'statusbar__notice error-note');
  let text;
  if (notice.entity) {
    text = accEl('button', 'statusbar__text statusbar__link', notice.text);
    text.type = 'button';
    text.title = 'Show it in Documents';
    text.addEventListener('click', () => {
      clearNotice(notice.key);
      runAction('documents.show', notice.entity);
    });
  } else {
    text = accEl('span', 'statusbar__text', notice.text);
    text.title = notice.text;
  }
  const dismiss = accEl('button', 'tool', '\u00d7');
  dismiss.type = 'button';
  dismiss.title = 'Dismiss';
  dismiss.addEventListener('click', () => clearNotice(notice.key));
  el.append(text, dismiss);
  if (notices.length > 1) el.append(accEl('span', 'meta', `+${notices.length - 1} more`));
  statusbarEl.append(el);
}

store.subscribe(state => state.notices, renderStatusbar);
rpcOn('notice', ({ level, message, entity }) => {
  if (level === 'info') showInfo(message, entity);
  else showError(message);
});

// A failed call nobody handled is shown here too, so no failure is only a
// console line. Superseded and cancelled calls are expected, and stay
// quiet.
window.addEventListener('unhandledrejection', e => {
  const error = e.reason;
  if (!(error instanceof RpcError)) return;
  e.preventDefault();
  if (error.code === 'superseded' || error.code === 'cancelled') return;
  showError(`${error.cmd}: ${error.message}`);
});
