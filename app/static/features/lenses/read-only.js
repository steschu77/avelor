// A lens row's read-only tag (ui.md#documents, ui.md#timeline): after the
// title of an entity whose file the host can't write, or of an event Google
// keeps as it is (`googleLocked`), its tooltip saying which.

import { accEl } from '../../core/dom.js';
import { t } from '../../core/text.js';
import { googleKept, googleLocked } from '../../model/calendar.js';

// The tag of `doc`, as the nodes after its title; none for one it can
// change.
export function readOnlyTag(doc) {
  const why = !doc.writable ? t('lens.read_only_file')
    : googleKept(doc) ? t('event.kept_title')
      : googleLocked(doc) ? t('event.calendar_read_only_title') : null;
  if (!why) return [];
  const tag = accEl('span', 'label', t('lens.read_only'));
  tag.title = why;
  return [' ', tag];
}
