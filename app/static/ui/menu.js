// The popup menu every menu of the page uses (ui.md): `showMenu` opens one
// at its anchor, `closeMenu` closes it, and so do a click outside it and
// Esc (the `menu.close` action).

import { accEl } from '../core/dom.js';
import { registerAction } from '../core/actions.js';

let openMenu = null;
let openMenuAnchor = null;

function closeMenu() {
  if (openMenu) {
    openMenu.remove();
    openMenuAnchor?.removeAttribute('aria-expanded');
    openMenu = null;
    openMenuAnchor = null;
  }
}

// `items`: { label, run }.
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
  const pick = it => {
    closeMenu();
    it.run();
  };
  const menuItem = it => {
    const mi = accEl('div', 'menu__item');
    mi.setAttribute('role', 'menuitem');
    const label = accEl('span', 'menu__label', it.label);
    mi.appendChild(label);
    mi.addEventListener('click', () => pick(it));
    return mi;
  };
  menu.append(...items.map(menuItem));

  const r = anchor.getBoundingClientRect();
  document.body.appendChild(menu);
  const width = menu.offsetWidth;
  const height = menu.offsetHeight;
  const left = Math.min(r.left, window.innerWidth - width - 8);
  // Below the anchor if it fits, else above; a menu that fits on neither
  // side (a long list of labels) takes the larger one and scrolls in it.
  // Above, its bottom edge is pinned.
  const below = window.innerHeight - 8 - (r.bottom + 4);
  const above = r.top - 4 - 8;
  const up = height > below && (height <= above || above > below);
  if (height > (up ? above : below)) menu.style.maxHeight = `${Math.floor(up ? above : below)}px`;
  if (up) menu.style.bottom = `${Math.round(window.innerHeight - r.top + 4)}px`;
  else menu.style.top = `${Math.round(r.bottom + 4)}px`;
  menu.style.left = `${Math.round(left)}px`;
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
