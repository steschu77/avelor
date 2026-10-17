// A button that asks by itself, without a dialog: the first click arms it
// (`data-armed`, filled red; `armedText` as its label, or as its title on
// an icon button), a second one runs `run`. It disarms after 3 s, when the
// pointer leaves it, or once it has run; while aria-disabled it does
// nothing. One button is armed at a time, page-wide. A page rebuilt on
// every change names the button by `key`: the one rebuilt with the armed
// button's key comes back armed. The card's delete and the project
// header's, the trash's, the Workspace pages' Disconnect and Remove.

let armed = null; // { key, button, restore, timer } of the armed button

function disarm() {
  if (!armed) return;
  clearTimeout(armed.timer);
  armed.restore();
  armed = null;
}

export function armedButton(button, armedText, run, key = null) {
  // an icon button shows its sprite icon (ui/icons.js), no text
  const icon = Boolean(button.querySelector(':scope > .icon'));
  let shown = null; // the title and label to come back to
  const arm = () => {
    shown = { title: button.title, text: button.textContent };
    button.setAttribute('data-armed', '');
    if (icon) button.title = armedText;
    else button.textContent = armedText;
  };
  const restore = () => {
    button.removeAttribute('data-armed');
    button.title = shown.title;
    if (!icon) button.textContent = shown.text;
  };

  if (key !== null && armed?.key === key) {
    arm();
    Object.assign(armed, { button, restore });
  }
  button.addEventListener('mouseleave', () => {
    if (armed?.button === button) disarm();
  });
  button.addEventListener('click', () => {
    if (button.getAttribute('aria-disabled') === 'true') return;
    if (armed?.button === button) {
      disarm();
      run();
      return;
    }
    disarm();
    arm();
    armed = { key, button, restore, timer: setTimeout(disarm, 3000) };
  });
}
