// The shell's layout (architecture.md#page): the grips that resize the
// vault panel, the panel and its context column, with the widths kept in
// the preferences; the panel's layout, which follows `panel_size`; the
// sidebar's sections; the panel's close and expand buttons.

import { $ } from '../core/dom.js';
import { savePrefs, store } from '../model/state.js';

const root = document.documentElement;
const shell = document.querySelector(".shell");
const main = $("main");
const panel = $("panel");
const sidebar = $("sidebar");
const sidebarDivider = $("sidebar-divider");
const panelDivider = $("panel-divider");
const contextDivider = $("context-divider");
const btnClose = $("doc-close-btn");
const btnExpand = $("doc-expand-btn");

let draggingSidebar = false;
let draggingPanel = false;
let draggingContext = false;
// the CSS defaults (--w-vault-list, --w-panel, --w-expanded-context)
let sidebarWidth = 232;
let panelWidth = 400;
let contextWidth = 400;

function applyVaultLayout() {
  root.style.setProperty("--w-vault-list", `${sidebarWidth}px`);
  root.style.setProperty("--w-sidebar", `${sidebarWidth}px`);
}

function applyPanelLayout() {
  const { panel_size } = store.getState();
  panel.classList.toggle("is-expanded", panel_size === "expanded");
  if (panel_size === "closed") {
    root.style.setProperty("--w-panel", "0px");
    root.style.setProperty("--o-panel", `${panelWidth}px`);

    /* disable resizing */
    panelDivider.style.display = "none";
  }
  else if (panel_size === "expanded") {
    root.style.setProperty("--w-panel", "100vw");
    root.style.setProperty("--o-panel", "0px");

    /* disable resizing */
    panelDivider.style.display = "none";
  }
  else {
    root.style.setProperty("--w-panel", `${panelWidth}px`);
    root.style.setProperty("--o-panel", "0px");
    panelDivider.style.display = "";
  }
}

const clampSidebarWidth = w => Math.max(180, Math.min(420, w));
const clampPanelWidth = w => Math.max(300, Math.min(800, w));
const clampContextWidth = w => Math.max(240, Math.min(800, w));

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
  const savedContext = saved('context_width');
  if (!savedSidebar && !savedPanel && !savedContext) return;

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
  if (savedContext) {
    contextWidth = clampContextWidth(savedContext);
    root.style.setProperty("--w-expanded-context", `${contextWidth}px`);
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
  if (store.getState().panel_size === "normal") {
    draggingPanel = true;
    panel.classList.add("is-resizing");
    main.classList.add("is-resizing");
    document.body.style.cursor = "col-resize";
    e.preventDefault();
  }
});

contextDivider.addEventListener('mousedown', e => {
  if (store.getState().panel_size === "expanded") {
    draggingContext = true;
    panel.classList.add("is-resizing");
    document.body.style.cursor = "col-resize";
    e.preventDefault();
  }
});

document.addEventListener('mousemove', e => {
  if (draggingContext) {
    /* the panel ends at the window's right edge, so the context's width is the distance to it */
    contextWidth = clampContextWidth(window.innerWidth - e.clientX);
    root.style.setProperty("--w-expanded-context", `${contextWidth}px`);
  }
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
  if (draggingContext) saveWidth('context_width', contextWidth);
  draggingSidebar = false;
  draggingPanel = false;
  draggingContext = false;
  shell.classList.remove("is-resizing");
  panel.classList.remove("is-resizing");
  main.classList.remove("is-resizing");
  document.body.style.cursor = "";
});

// collapsible sections: a label owns the siblings up to the next label
for (const section of sidebar.querySelectorAll(".nav-section")) {
  section.setAttribute("aria-expanded", "true");
  section.addEventListener('click', () => {
    const expanded = section.getAttribute("aria-expanded") !== "true";
    section.setAttribute("aria-expanded", String(expanded));
    for (let el = section.nextElementSibling; el && !el.classList.contains("nav-section"); el = el.nextElementSibling) {
      el.hidden = !expanded;
    }
  });
}

for (const btn of sidebar.querySelectorAll(".nav-section__add")) {
  btn.addEventListener('click', e => e.stopPropagation());
}

btnClose.addEventListener('click', () => store.setState({ panel_size: "closed" }));

btnExpand.addEventListener('click', () => {
  store.setState({ panel_size: store.getState().panel_size === "expanded" ? "normal" : "expanded" });
});

// The layout follows `panel_size`; the expand button shows what it does.
store.subscribe(state => state.panel_size, panel_size => {
  const icon = panel_size === "expanded" ? "#i-panel-right-contract" : "#i-panel-right-expand";
  btnExpand.querySelector("use").setAttribute("href", icon);
  applyPanelLayout();
});
