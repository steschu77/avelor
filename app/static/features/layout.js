// The shell's layout (architecture.md#page): the grips that resize the
// vault panel and the panel, with the widths kept in the preferences.

import { $ } from '../core/dom.js';
import { savePrefs, store } from '../model/state.js';

const root = document.documentElement;
const shell = document.querySelector(".shell");
const main = $("main");
const panel = $("panel");
const sidebarDivider = $("sidebar-divider");
const panelDivider = $("panel-divider");

let draggingSidebar = false;
let draggingPanel = false;
// the CSS defaults (--w-vault-list, --w-panel)
let sidebarWidth = 232;
let panelWidth = 400;

function applyVaultLayout() {
  root.style.setProperty("--w-vault-list", `${sidebarWidth}px`);
  root.style.setProperty("--w-sidebar", `${sidebarWidth}px`);
}

function applyPanelLayout() {
  root.style.setProperty("--w-panel", `${panelWidth}px`);
}

const clampSidebarWidth = w => Math.max(180, Math.min(420, w));
const clampPanelWidth = w => Math.max(300, Math.min(800, w));

// The widths saved in the preferences are the starting layout, not a
// change: applied before the first frame, without easing. A missing one
// keeps the default.
function applySavedWidths(prefs) {
  const saved = key => {
    const width = Number(prefs[key]);
    return width > 0 ? width : null;
  };
  const savedSidebar = saved('sidebar_width');
  const savedPanel = saved('panel_width');
  if (!savedSidebar && !savedPanel) return;

  const eased = [shell, main, panel];
  for (const el of eased) el.classList.add("is-resizing");
  if (savedSidebar) {
    sidebarWidth = clampSidebarWidth(savedSidebar);
    applyVaultLayout();
  }
  if (savedPanel) {
    panelWidth = clampPanelWidth(savedPanel);
    applyPanelLayout();
  }
  void shell.offsetWidth; // the widths take effect while easing is off
  for (const el of eased) el.classList.remove("is-resizing");
}
applySavedWidths(store.getState().prefs);

// Remember a dragged width, unless it is the one saved already.
function saveWidth(key, width) {
  const value = String(Math.round(width));
  if (store.getState().prefs[key] !== value) savePrefs({ [key]: value });
}

sidebarDivider.addEventListener('mousedown', e => {
  draggingSidebar = true;
  shell.classList.add("is-resizing");
  document.body.style.cursor = "col-resize";
  e.preventDefault();
});

panelDivider.addEventListener('mousedown', e => {
  draggingPanel = true;
  panel.classList.add("is-resizing");
  main.classList.add("is-resizing");
  document.body.style.cursor = "col-resize";
  e.preventDefault();
});

document.addEventListener('mousemove', e => {
  if (draggingSidebar) {
    sidebarWidth = clampSidebarWidth(e.clientX);
    applyVaultLayout();
  }
  if (draggingPanel) {
    panelWidth = clampPanelWidth(window.innerWidth - e.clientX);
    applyPanelLayout();
  }
});

document.addEventListener('mouseup', () => {
  if (draggingSidebar) saveWidth('sidebar_width', sidebarWidth);
  if (draggingPanel) saveWidth('panel_width', panelWidth);
  draggingSidebar = false;
  draggingPanel = false;
  shell.classList.remove("is-resizing");
  panel.classList.remove("is-resizing");
  main.classList.remove("is-resizing");
  document.body.style.cursor = "";
});
