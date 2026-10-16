// The segmented control (the `seg` component).

import { accEl } from '../core/dom.js';

// A segmented control of `options` ([value, label, { title, disabled }?]):
// `current` is pressed, a click on another option calls `onPick(value)`;
// `disabled` turns them all off (while a request runs).
export function accSeg(options, current, onPick, disabled = false) {
  const seg = accEl('span', 'seg');
  for (const [value, label, extra = {}] of options) {
    const option = accEl('button', '', label);
    option.type = 'button';
    option.setAttribute('aria-pressed', String(value === current));
    option.disabled = disabled || !!extra.disabled;
    if (extra.title) option.title = extra.title;
    option.addEventListener('click', () => {
      if (value !== current) onPick(value);
    });
    seg.append(option);
  }
  return seg;
}
