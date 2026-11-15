// A list of texts as a field (panel.md#field-edits): a contact's emails,
// phones and addresses, a list's items. Each is typed in; Enter adds an
// empty one after it, Backspace in an empty one removes it, x removes one,
// + adds one at the end. `write(values)` gets the whole list, empty ones
// left out, once a value changed or one went; an empty one added waits to
// be typed in.

import { accEl, titled } from '../core/dom.js';
import { t } from '../core/text.js';
import { iconEl } from './icons.js';

// `values` as such a field; `key` finds its inputs again once their block
// is drawn anew (`${key}:${index}`), `type` and `placeholder` are each
// input's.
export function textListField(values, key, write, { type = 'text', placeholder = '' } = {}) {
  const field = accEl('span', 'text-list');
  const rows = values.length ? [...values] : [];
  const commit = () => write(rows.map(value => value.trim()).filter(Boolean));
  const add = accEl('button', 'tool label text-list__add', t('list.add'));
  add.type = 'button';
  add.dataset.focusKey = `${key}:add`;
  add.addEventListener('click', () => {
    rows.push('');
    render(rows.length - 1);
  });

  function render(focus = null) {
    field.replaceChildren(...rows.map(item), add);
    if (focus !== null) field.querySelectorAll('input')[focus]?.focus();
  }

  function item(value, i) {
    const el = accEl('span', 'text-list__item');
    const input = accEl('input', 'form-input');
    input.type = type;
    input.placeholder = placeholder;
    input.value = value;
    input.dataset.focusKey = `${key}:${i}`;
    input.addEventListener('change', () => {
      rows[i] = input.value;
      commit();
    });
    input.addEventListener('keydown', e => {
      if (e.key === 'Enter' && !e.ctrlKey) {
        e.preventDefault();
        rows[i] = input.value;
        commit();
        rows.splice(i + 1, 0, '');
        render(i + 1);
      } else if (e.key === 'Backspace' && !input.value) {
        e.preventDefault();
        rows.splice(i, 1);
        commit();
        render(Math.max(0, i - 1));
      }
    });
    const remove = accEl('button', 'tool text-list__remove');
    remove.type = 'button';
    remove.append(iconEl('#i-dismiss'));
    titled(remove, t('list.remove'));
    remove.addEventListener('click', () => {
      rows.splice(i, 1);
      commit();
      render();
    });
    el.append(input, remove);
    return el;
  }

  render();
  return field;
}
