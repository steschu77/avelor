// The popup menu every menu of the page uses (ui.md): `showMenu` opens one
// at its anchor, `closeMenu` closes it, and so do a click outside it and
// Esc (the `menu.close` action).

import { accEl } from '../core/dom.js';
import { iconEl } from './icons.js';
import { registerAction } from '../core/actions.js';

let openMenu = null;
let openMenuAnchor = null;

// The focus in the menu (its filter) goes back to the anchor.
function closeMenu() {
  if (openMenu) {
    const focused = openMenu.contains(document.activeElement);
    openMenu.remove();
    if (focused) openMenuAnchor?.focus();
    openMenuAnchor?.removeAttribute('aria-expanded');
    openMenu = null;
    openMenuAnchor = null;
  }
}

// `items`: { label, hint?, icon?, dim?, run? } or { head } -- `hint` is the
// small label on the right, `icon` an icon before the label (`#i-<name>`);
// `head` labels a section, the items up to the next head. `options.head` is
// a label over the items. `options.filter` ({ placeholder, extra(text) })
// puts a text box over the items, focused: typing keeps the items whose
// label contains the text (a head while an item of its section stays) and
// adds `extra(text)`'s; up and down move the highlight, Enter runs it.
// The anchor is marked `aria-expanded` while its menu is open. Showing the
// open menu's anchor again closes it (a toggle).
export function showMenu(anchor, items, options = {}) {
  if (openMenu && openMenuAnchor === anchor) {
    closeMenu();
    return;
  }
  closeMenu();
  const menu = accEl('div', 'menu');
  menu.setAttribute('role', 'menu');
  const menuHead = text => {
    const head = accEl('div', 'label menu__head', text);
    return head;
  };
  if (options.head) menu.appendChild(menuHead(options.head));
  const pick = it => {
    closeMenu();
    it.run();
  };
  const icons = items.some(it => it.icon);
  const menuItem = it => {
    if (it.head) return menuHead(it.head);
    const mi = accEl('div', 'menu__item');
    mi.setAttribute('role', 'menuitem');
    if (it.dim) mi.setAttribute('aria-disabled', 'true');
    if (icons) mi.appendChild(iconEl(it.icon ?? '', 'menu__icon'));
    const label = accEl('span', 'menu__label', it.label);
    mi.appendChild(label);
    if (it.hint) {
      const hint = accEl('span', 'menu__hint', it.hint);
      mi.appendChild(hint);
    }
    if (it.run) mi.addEventListener('click', () => pick(it));
    return mi;
  };

  // The items shown, and of them the ones that run (the highlight's); with
  // a filter, they scroll below it in a list of their own.
  const list = options.filter ? accEl('div') : menu;
  let shown = [];
  let runnable = [];
  let active = -1;
  const highlight = i => {
    runnable[active]?.el.classList.remove('is-active');
    active = i;
    runnable[active]?.el.classList.add('is-active');
    runnable[active]?.el.scrollIntoView({ block: 'nearest' });
  };
  const fill = its => {
    for (const el of shown) el.remove();
    shown = its.map(menuItem);
    list.append(...shown);
    runnable = its.map((it, i) => ({ it, el: shown[i] })).filter(({ it }) => it.run && !it.dim);
    active = -1;
    if (options.filter) {
      runnable.forEach(({ el }, i) => el.addEventListener('mousemove', () => highlight(i)));
      highlight(0);
    }
  };

  let filter = null;
  if (options.filter) {
    menu.classList.add('menu--filter');
    list.className = 'menu__list';
    filter = accEl('input', 'input menu__filter');
    filter.type = 'text';
    const { placeholder = '', extra } = options.filter;
    filter.placeholder = placeholder;
    const matching = () => {
      const text = filter.value.trim();
      const needle = text.toLowerCase();
      const kept = items.filter(it => it.head || it.label.toLowerCase().includes(needle));
      const its = kept.filter((it, i) => !it.head || (kept[i + 1] && !kept[i + 1].head));
      return [...its, ...(text && extra ? extra(text) : [])];
    };
    filter.addEventListener('input', () => fill(matching()));
    filter.addEventListener('keydown', e => {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const step = e.key === 'ArrowDown' ? 1 : -1;
        if (runnable.length) highlight((active + step + runnable.length) % runnable.length);
      } else if (e.key === 'Enter') {
        e.preventDefault();
        if (runnable[active]) pick(runnable[active].it);
      }
    });
    menu.append(filter, list);
    fill(matching());
  } else {
    fill(items);
  }

  const r = anchor.getBoundingClientRect();
  document.body.appendChild(menu);
  const width = menu.offsetWidth;
  const height = menu.offsetHeight;
  const left = Math.min(r.left, window.innerWidth - width - 8);
  // Below the anchor if it fits, else above; a menu that fits on neither
  // side (a long list of labels) takes the larger one and scrolls in it.
  // Above, its bottom edge is pinned, and a filtered menu keeps its width,
  // so it stays at the anchor as typing shrinks it.
  const below = window.innerHeight - 8 - (r.bottom + 4);
  const above = r.top - 4 - 8;
  const up = height > below && (height <= above || above > below);
  if (height > (up ? above : below)) menu.style.maxHeight = `${Math.floor(up ? above : below)}px`;
  if (up) menu.style.bottom = `${Math.round(window.innerHeight - r.top + 4)}px`;
  else menu.style.top = `${Math.round(r.bottom + 4)}px`;
  menu.style.left = `${Math.round(left)}px`;
  if (filter) {
    menu.style.width = `${width}px`;
    filter.focus();
  }
  openMenu = menu;
  openMenuAnchor = anchor;
  anchor.setAttribute('aria-expanded', 'true');
}

// close menu on outside click (a click on its anchor toggles it instead)
document.addEventListener('mousedown', e => {
  if (openMenu && !e.target.closest('.menu') && !openMenuAnchor?.contains(e.target))
    closeMenu();
});

registerAction({
  id: 'menu.close', title: 'Close the menu', shortcut: 'Escape',
  when: () => !!openMenu,
  run: () => closeMenu(),
});
