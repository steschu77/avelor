// The draft card (panel.md#draft): the side panel's card of an entity not
// made yet (model/drafts.js), drawn in a view of its own while the panel's
// mode is `draft` (`panelMode`, features/selection.js), as a transaction's card
// is. The parts of a made entity's card (features/panel/panel.js): the
// title typed in at its head; the details and their editors
// (`renderCardDetails`), which write to the draft through
// model/structure.js; the projects and labels; the body's editor, editable
// from the start and kept from the host until Create. Then Create
// (Ctrl+Enter), which selects the new entity, and Discard (Esc, with the
// focus outside a field or the editor); the host's refusal under them. The
// card stays built while the draft is kept, hidden or shown.

import { registerAction, runAction, shortcutText } from '../../core/actions.js';
import { $, accEl } from '../../core/dom.js';
import { t } from '../../core/text.js';
import { createEditor } from '../../editor/editor.js';
import { serializeMarkdown } from '../../editor/markdown.js';
import { changeDraft, createDraft, currentDraft, dropDraft } from '../../model/drafts.js';
import { store } from '../../model/state.js';
import { joinProjectLocal, leaveProjectLocal, setLabelsLocal } from '../../model/structure.js';
import { keepingFocus } from '../../ui/field.js';
import { docIcon, iconEl } from '../../ui/icons.js';
import { createEditorToolbar } from '../../ui/toolbar.js';
import { NEW_TYPES } from '../new.js';
import { renderCardDetails } from './details.js';
import { addChip, labelChips, projectChips } from './panel.js';

const viewEl = $('panel-draft-view');

// the card built: the draft's id, its type (`NEW_TYPES`), its editor (none
// for a list or a record), its parts, whether Create runs and what the host
// refused
let shown = null;

// ----------------------------------------------------------------------------
function render() {
  const doc = currentDraft();
  if (!doc) teardown();
  viewEl.hidden = !doc || !store.getState().selected_draft;
  if (!doc) return;
  if (shown?.id !== doc.doc_id) build(doc);
  refresh(doc);
}

function build(doc) {
  teardown();
  const type = NEW_TYPES.find(type => type.kind === doc.kind);
  const card = accEl('div', 'msg');

  const head = accEl('div', 'pane-section pane-section--ruled doc-head');
  const headTitle = accEl('div', 'doc-head__title');
  const icon = iconEl('');
  const title = accEl('input', 'input doc-title draft__title');
  title.placeholder = type.hint;
  title.setAttribute('aria-label', type.label);
  title.addEventListener('input', () => changeDraft({ title: title.value }));
  headTitle.append(icon, title);
  const headSub = accEl('div', 'doc-head__sub');
  headSub.append(accEl('span', 'label', type.name), accEl('span', 'meta truncate-start', `${type.folder}\\`));
  head.append(headTitle, headSub);

  const details = accEl('div', 'pane-section props');
  const chips = accEl('div', 'pane-section msg__chips');
  const toolbar = createEditorToolbar();
  const root = accEl('div', 'pane-section editor prose');
  const foot = accEl('div', 'pane-section draft__foot');
  const error = accEl('p', 'error-note');
  // a list or a record has no body, so no editor, whose toolbar would show
  const hasBody = !['list', 'record'].includes(doc.kind);
  toolbar.hidden = !hasBody;
  root.hidden = !hasBody;
  const editor = hasBody ? createEditor(root, {
    toolbar,
    onEscape: () => foot.querySelector('.btn--primary')?.focus(),
    onChange: () => {},
    onLinkClick: () => {},
  }) : null;
  editor?.setEditable(true, false);

  card.append(head, details, chips, toolbar, root, foot, error);
  viewEl.replaceChildren(card);
  shown = { id: doc.doc_id, type, editor, hasBody, busy: false, error: null, els: { icon, details, chips, root, foot, error } };
  title.focus();
}

function refresh(doc) {
  const { els } = shown;
  els.icon.firstChild.setAttribute('href', docIcon(doc));
  renderCardDetails(els.details, doc);
  const id = doc.doc_id;
  els.chips.replaceChildren(
    ...projectChips(doc.projects, project => leaveProjectLocal(id, project)),
    ...labelChips(doc.labels, labels => setLabelsLocal(id, labels)),
    addChip(doc, {
      onJoin: project => joinProjectLocal(id, project),
      onAddLabel: label => setLabelsLocal(id, [...doc.labels, label]),
    }),
  );
  renderFoot(doc);
}

// Create, off while the title is empty, saying so; Discard; what the host
// refused.
function renderFoot(doc) {
  const { type, els } = shown;
  const missing = doc.title?.trim() ? null : type.missing;
  keepingFocus(els.foot, () => {
    const create = accEl('button', 'btn btn--primary', shown.busy ? t('new.creating') : type.create);
    create.type = 'button';
    create.dataset.focusKey = 'draft:create';
    create.disabled = Boolean(missing) || shown.busy;
    create.title = missing ?? type.createTitle;
    create.addEventListener('click', create_);
    const discard = accEl('button', 'btn btn--ghost push', t('draft.discard'));
    discard.type = 'button';
    discard.dataset.focusKey = 'draft:discard';
    discard.addEventListener('click', dropDraft);
    els.foot.replaceChildren(create, accEl('span', 'meta', shortcutText('Ctrl+Enter')), discard, accEl('span', 'meta', shortcutText('Escape')));
  });
  els.error.textContent = shown.error ?? '';
  els.error.hidden = !shown.error;
}

// Make the draft, its body as markdown, and show the new entity where it
// lives (`documents.show_made`); refused, the card says why and stays.
async function create_() {
  const doc = currentDraft();
  if (!shown || shown.busy || !doc?.title?.trim()) return;
  shown.busy = true;
  shown.error = null;
  renderFoot(doc);
  try {
    const id = await createDraft(shown.hasBody ? serializeMarkdown(shown.els.root) : '');
    runAction('documents.show_made', id);
  } catch (e) {
    if (!shown) return;
    shown.busy = false;
    shown.error = e.message;
    renderFoot(currentDraft());
  }
}

function teardown() {
  shown?.editor?.destroy();
  shown = null;
  viewEl.replaceChildren();
}

// ----------------------------------------------------------------------------
// The keys, while the draft card shows: Ctrl+Enter creates, Esc outside a
// field or the editor discards.
const showing = () => Boolean(store.getState().selected_draft) && !viewEl.hidden;

registerAction({ id: 'draft.create', shortcut: 'Ctrl+Enter', when: showing, run: create_ });
registerAction({ id: 'draft.discard', shortcut: 'Escape', outsideFields: true, when: showing, run: dropDraft });

store.subscribe([state => state.selected_draft, state => state.drafts_rev], render);
