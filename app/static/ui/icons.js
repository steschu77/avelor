// The icons (symbols of the page's sprite, build.rs): an entity's by its
// kind, a document's by its genre (`getDocIcon`), and `iconEl`, an icon
// made in a script.

const SVG_NS = 'http://www.w3.org/2000/svg';

// ----------------------------------------------------------------------------
// Keyed by the derived `DocumentType` (kebab-case, see av-vault envelope.rs).
export const KIND_ICONS = {
  document: { icon: '#i-note' },
  message: { icon: '#i-chat' },
  email: { icon: '#i-mention' },
  conversation: { icon: '#i-chat' },
  thread: { icon: '#i-chat' },
  project: { icon: '#i-folder' },
  transactions: { icon: '#i-payment' },
  'bank-account': { icon: '#i-payment' },
  calendar: { icon: '#i-calendar' },
  event: { icon: '#i-calendar' },
  task: { icon: '#i-checkmark' },
  contact: { icon: '#i-person' },
  record: { icon: '#i-lock-closed' },
  list: { icon: '#i-task-list-ltr' },
};

// ----------------------------------------------------------------------------
const DOC_SUB_ICONS = {
  note: { icon: '#i-note' },
  letter: { icon: '#i-mail' },
  contract: { icon: '#i-lock-closed' },
  bill: { icon: '#i-task-list-ltr' },
  memo: { icon: '#i-payment' },
  report: { icon: '#i-contact-card' },
  invoice: { icon: '#i-contact-card' },
  statement: { icon: '#i-document-table' },
};

// ----------------------------------------------------------------------------
// The icon of an entity: its kind's, or for a document its genre's, if it
// has one.
export function getDocIcon(kind, genre) {
  let icon = KIND_ICONS[kind]?.icon || '';
  if (kind === 'document' && genre) {
    icon = DOC_SUB_ICONS[genre]?.icon || icon;
  }
  return icon;
}

export const IMPORT_ICON = '#i-arrow-import';

// ----------------------------------------------------------------------------
// An `<svg class="icon">` showing the symbol `href` (`#i-<name>`): the
// markup's icons are written as such, a script's are made here, since an
// element made with `accEl` is no SVG element.
export function iconEl(href, className) {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('class', className ? `icon ${className}` : 'icon');
  const use = document.createElementNS(SVG_NS, 'use');
  use.setAttribute('href', href);
  svg.append(use);
  return svg;
}
