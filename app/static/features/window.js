// The page's zoom and full screen from Go to (ui.md#go-to): the host takes
// their keys before the page (`host`), and does the same for `window`.

import { registerAction } from '../core/actions.js';
import { rpcCall } from '../core/rpc.js';
import { t } from '../core/text.js';

for (const [step, title, icon, shortcut, also] of [
  ['zoom_in', t('window.zoom_in'), '#i-search', 'Ctrl+Plus', t('window.zoom_in_also')],
  ['zoom_out', t('window.zoom_out'), '#i-search', 'Ctrl+Minus', t('window.zoom_out_also')],
  ['zoom_reset', t('window.zoom_reset'), '#i-search', 'Ctrl+0'],
  ['fullscreen', t('window.fullscreen'), '#i-open', 'F11'],
]) {
  registerAction({
    id: `window.${step}`, group: 'window', title, icon, also, shortcut, host: true,
    run: () => rpcCall('window', { step }),
  });
}
