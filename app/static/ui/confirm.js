// A button that asks by itself, without a dialog: the first click arms it
// (`data-armed`, filled red; `armedText` as its label, or as its title on
// an icon button), a second one runs `run`. It disarms after 3 s, when the
// pointer leaves it, or once it has run; while aria-disabled it does
// nothing. One button is armed at a time, page-wide. The card's delete and
// the project header's.

let armed = null; // { button, restore, timer } of the armed button

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
    armed = { button, restore, timer: setTimeout(disarm, 3000) };
  });
}
