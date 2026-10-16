import { accEl } from '../../core/dom.js';

// A grid of form rows; `row(label, ...content)` adds one and returns its
// field.
export function watchFormGrid() {
  const grid = accEl('div', 'form-grid');
  const row = (label, ...content) => {
    const field = accEl('div', 'form-field');
    field.append(...content);
    grid.append(accEl('span', 'label', label), field);
    return field;
  };
  return [grid, row];
}
