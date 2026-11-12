// The segmented control (the `seg` component), on the Workspace pages
// and in Fetch email.

import { accEl } from '../core/dom.js';
import { iconEl } from './icons.js';

// A segmented control of `options` ([value, label, { title, disabled, icon }?]):
// `current` is pressed, a click on another option calls `onPick(value)`;
// `disabled` turns them all off (while a request runs). An option with an
// icon shows it in place of its label, which is its tooltip (`seg--icons`).
export function accSeg(options, current, onPick, disabled = false) {
  const seg = accEl('span', 'seg');
  for (const [value, label, extra = {}] of options) {
    const option = accEl('button', '', extra.icon ? undefined : label);
    if (extra.icon) {
      option.append(iconEl(extra.icon));
      option.setAttribute('aria-label', label);
      seg.classList.add('seg--icons');
    }
    option.type = 'button';
    option.setAttribute('aria-pressed', String(value === current));
    option.disabled = disabled || !!extra.disabled;
    if (extra.title || extra.icon) option.title = extra.title ?? label;
    option.addEventListener('click', () => {
      if (value !== current) onPick(value);
    });
    seg.append(option);
  }
  return seg;
}

// ----------------------------------------------------------------------------
// The segmented control `seg` of `options` (as `accSeg` took them), its
// buttons found again by `<key>:<value>` once their block is drawn anew
// (`keepingFocus`, ui/field.js).
export function keyedSeg(seg, options, key) {
  [...seg.children].forEach((option, i) => { option.dataset.focusKey = `${key}:${options[i][0]}`; });
  return seg;
}

// ----------------------------------------------------------------------------
// A segmented control of `options` ([value, label]) any of which are
// pressed, `current` their values: a click presses or releases one and
// calls `onPick(values)`, but leaves one pressed at least.
export function accSegMany(options, current, onPick) {
  const seg = accEl('span', 'seg');
  for (const [value, label] of options) {
    const option = accEl('button', '', label);
    const on = current.includes(value);
    option.type = 'button';
    option.setAttribute('aria-pressed', String(on));
    option.addEventListener('click', () => {
      const next = on ? current.filter(v => v !== value) : [...current, value];
      if (next.length) onPick(next);
    });
    seg.append(option);
  }
  return seg;
}
