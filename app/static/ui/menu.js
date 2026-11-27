// The popup menu every menu of the page uses (ui.md): `showMenu` opens one
// at its anchor, `closeMenu` closes it, and so do a click outside it and
// Esc (the `menu.close` action, which runs before the vault pages' Esc,
// ui/page.js, as app.js imports this file first). An item can open a
// submenu beside it, one level deep, and ask once more before it runs.

import { accEl } from '../core/dom.js';
import { iconEl } from './icons.js';
import { registerAction } from '../core/actions.js';
import { t } from '../core/text.js';

let openMenu = null;
let openMenuAnchor = null;
let openMenuFocus = null; // what has the focus in the open menu: it, or its filter
let openSub = null; // { menu, entry, focus, highlightFirst }: the submenu open beside `entry`
let armedItem = null; // { el, restore, timer }: the item asking once more
let itemIds = 0; // for aria-activedescendant

function disarmItem() {
  if (!armedItem) return;
  clearTimeout(armedItem.timer);
  armedItem.restore();
  armedItem = null;
}

// The submenu closes; its entry's menu takes the focus back if it had it,
// or always with `focusMenu`.
function closeSub(focusMenu = false) {
  if (!openSub) return;
  const { menu, entry } = openSub;
  const focused = menu.contains(document.activeElement);
  disarmItem();
  menu.remove();
  entry.setAttribute('aria-expanded', 'false');
  openSub = null;
  if (focused || focusMenu) openMenuFocus?.focus({ preventScroll: true });
}

// The focus in the menu (it, its submenu or its filter) goes back to the
// anchor.
function closeMenu() {
  if (openMenu) {
    const focused = openMenu.contains(document.activeElement) || openSub?.menu.contains(document.activeElement);
    closeSub();
    disarmItem();
    openMenu.remove();
    if (focused) openMenuAnchor?.focus();
    openMenuAnchor?.removeAttribute('aria-expanded');
    openMenu = null;
    openMenuAnchor = null;
    openMenuFocus = null;
  }
}

// `items`: { label, hint?, desc?, icon?, checked?, separator?, dim?, run?,
// items?, armed? } or { head, separator? } -- `hint` is the small label on
// the right (a key), `desc` a short note there on what the item makes,
// `icon` an icon before the label (`#i-<name>`), `checked` shows a check
// column, `separator` draws a line above the item or the head; `head` labels
// a section, the items up to the next head. `items` makes the item a
// submenu's (marked with a triangle): hovering it, Enter or Right opens them
// beside it, Left and Esc go back. `armed`: the label the item shows after
// a first press, which keeps the menu open; a second press runs it, and
// moving off it, or 3 s, takes it back. `options.head` is a label over the
// items; `options.matchWidth` makes the menu at least as wide as `anchor`;
// `options.at` ({ x, y }, a right-click's point) opens it there rather than
// at the anchor. `options.filter` ({ placeholder, extra(text) }) puts a text
// box over the items, focused: typing keeps the items whose label contains
// the text (a head while an item of its section stays) and adds
// `extra(text)`'s. A menu without one takes the focus itself. Up and down
// move the highlight, Enter runs it (in a menu without a filter Space too).
// The anchor is marked `aria-expanded` while its menu is open. Showing the
// open menu's anchor again closes it (a toggle), unless at a point.
export function showMenu(anchor, items, options = {}) {
  if (openMenu && openMenuAnchor === anchor && !options.at) {
    closeMenu();
    return;
  }
  closeMenu();
  const { menu, focus } = buildMenu(items, options, false);
  const r = options.at
    ? { left: options.at.x, right: options.at.x, top: options.at.y, bottom: options.at.y, width: 0 }
    : anchor.getBoundingClientRect();
  if (options.matchWidth) menu.style.minWidth = `${Math.round(r.width)}px`;
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
  if (options.filter) menu.style.width = `${width}px`;
  focus.focus({ preventScroll: true });
  openMenu = menu;
  openMenuAnchor = anchor;
  openMenuFocus = focus;
  anchor.setAttribute('aria-expanded', 'true');
}

// A menu of `items` (see `showMenu`), not placed yet: the element, and what
// takes its focus, and what highlights its first item. `sub`: a submenu's,
// whose Left goes back.
function buildMenu(items, options, sub) {
  const menu = accEl('div', 'menu');
  menu.setAttribute('role', 'menu');
  const menuHead = (text, separator = false) =>
    accEl('div', 'label menu__head' + (separator ? ' menu__head--sep' : ''), text);
  if (options.head) menu.appendChild(menuHead(options.head));
  const icons = items.some(it => it.icon);
  const checks = items.some(it => it.checked !== undefined);
  let filter = null;

  // A press on `it` (its element `el`; `keys`: by the keyboard): a
  // submenu's opens, an armed item asks first, the others run.
  const press = (it, el, keys) => {
    if (it.items) return openSubmenu(el, it.items, keys);
    if (it.armed && armedItem?.el !== el) {
      disarmItem();
      const label = el.querySelector('.menu__label');
      label.textContent = it.armed;
      el.setAttribute('data-armed', '');
      armedItem = {
        el,
        restore: () => {
          label.textContent = it.label;
          el.removeAttribute('data-armed');
        },
        timer: setTimeout(disarmItem, 3000),
      };
      return;
    }
    closeMenu();
    it.run();
  };
  const menuItem = it => {
    if (it.head) return menuHead(it.head, it.separator);
    const mi = accEl('div', 'menu__item' + (it.separator ? ' menu__item--sep' : ''));
    mi.id = `menu-item-${++itemIds}`;
    mi.setAttribute('role', checks ? 'menuitemcheckbox' : 'menuitem');
    if (checks) mi.setAttribute('aria-checked', String(!!it.checked));
    if (it.dim) mi.setAttribute('aria-disabled', 'true');
    if (it.items) {
      mi.setAttribute('aria-haspopup', 'menu');
      mi.setAttribute('aria-expanded', 'false');
    }
    if (checks) {
      const check = accEl('span', 'menu__check', it.checked ? '\u2713' : '');
      mi.appendChild(check);
    }
    if (icons) mi.appendChild(iconEl(it.icon ?? '', 'menu__icon'));
    const label = accEl('span', 'menu__label', it.label);
    mi.appendChild(label);
    if (it.hint) {
      const hint = accEl('span', 'menu__hint', it.hint);
      mi.appendChild(hint);
    }
    if (it.desc) mi.appendChild(accEl('span', 'note menu__desc', it.desc));
    if (it.items) mi.appendChild(accEl('span', 'menu__more', '\u25b8'));
    if (it.run || it.items) mi.addEventListener('click', () => press(it, mi, false));
    return mi;
  };

  // The items shown, and of them the ones that run or open a submenu (the
  // highlight's); with a filter, they scroll below it in a list of their
  // own. Moving the highlight off an item takes back what it opened or
  // armed; the mouse on a submenu's item opens it.
  const list = options.filter ? accEl('div') : menu;
  let shown = [];
  let runnable = [];
  let active = -1;
  const highlight = i => {
    runnable[active]?.el.classList.remove('is-active');
    active = i;
    const el = runnable[active]?.el;
    if (armedItem && armedItem.el !== el && menu.contains(armedItem.el)) disarmItem();
    if (!sub && openSub && openSub.entry !== el) closeSub();
    el?.classList.add('is-active');
    el?.scrollIntoView({ block: 'nearest' });
    if (el) (filter ?? menu).setAttribute('aria-activedescendant', el.id);
    else (filter ?? menu).removeAttribute('aria-activedescendant');
  };
  const fill = its => {
    for (const el of shown) el.remove();
    shown = its.map(menuItem);
    list.append(...shown);
    runnable = its.map((it, i) => ({ it, el: shown[i] })).filter(({ it }) => (it.run || it.items) && !it.dim);
    active = -1;
    runnable.forEach(({ it, el }, i) => el.addEventListener('mousemove', () => {
      if (active !== i) highlight(i);
      if (it.items && openSub?.entry !== el) openSubmenu(el, it.items, false);
    }));
    if (options.filter) highlight(0);
  };
  menu.addEventListener('keydown', e => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const step = e.key === 'ArrowDown' ? 1 : -1;
      const n = runnable.length;
      if (n) highlight(active < 0 ? (step > 0 ? 0 : n - 1) : (active + step + n) % n);
    } else if (e.key === 'Enter' || (e.key === ' ' && !filter)) {
      e.preventDefault();
      if (runnable[active]) press(runnable[active].it, runnable[active].el, true);
    } else if (e.key === 'ArrowRight' && !filter && runnable[active]?.it.items) {
      e.preventDefault();
      openSubmenu(runnable[active].el, runnable[active].it.items, true);
    } else if (e.key === 'ArrowLeft' && sub) {
      e.preventDefault();
      closeSub(true);
    }
  });

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
    menu.append(filter, list);
    fill(matching());
  } else {
    fill(items);
    menu.tabIndex = -1;
  }
  return { menu, focus: filter ?? menu, highlightFirst: () => highlight(0) };
}

// The submenu of `items` beside `entry`, an item of the open menu, on its
// right or, where that has no room, on its left; `keys`: opened by the
// keyboard, which takes the focus into it.
function openSubmenu(entry, items, keys) {
  if (openSub?.entry !== entry) {
    closeSub();
    const { menu, focus, highlightFirst } = buildMenu(items, {}, true);
    document.body.appendChild(menu);
    const parent = entry.closest('.menu').getBoundingClientRect();
    const r = entry.getBoundingClientRect();
    const width = menu.offsetWidth;
    const right = parent.right - 2;
    const left = right + width > window.innerWidth - 8 ? Math.max(8, parent.left - width + 2) : right;
    const top = Math.max(8, Math.min(r.top - 5, window.innerHeight - 8 - menu.offsetHeight));
    menu.style.left = `${Math.round(left)}px`;
    menu.style.top = `${Math.round(top)}px`;
    menu.addEventListener('mouseenter', () => focus.focus({ preventScroll: true }));
    entry.setAttribute('aria-expanded', 'true');
    openSub = { menu, entry, focus, highlightFirst };
  }
  if (keys) {
    openSub.focus.focus({ preventScroll: true });
    openSub.highlightFirst();
  }
}

// ----------------------------------------------------------------------------
// `button` as a drop-down box: what is picked (`label`), a `hint`, the caret;
// its click opens its menu.
export function dropBox(button, label, hint = '') {
  button.type = 'button';
  button.className = 'drop-box';
  button.setAttribute('aria-haspopup', 'menu');
  button.replaceChildren(accEl('span', 'drop-box__label', label), ...(hint ? [accEl('span', 'meta', hint)] : []),
    iconEl('#i-chevron-down', 'drop-box__caret'));
  return button;
}

// ----------------------------------------------------------------------------
// close menu on outside click (a click on its anchor toggles it instead)
document.addEventListener('mousedown', e => {
  if (openMenu && !e.target.closest('.menu') && !openMenuAnchor?.contains(e.target))
    closeMenu();
});

// Esc closes a submenu first, back in its entry's menu.
registerAction({
  id: 'menu.close', title: t('menu.close'), shortcut: 'Escape',
  when: () => !!openMenu,
  run: () => (openSub ? closeSub(true) : closeMenu()),
});
