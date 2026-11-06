// The sources (ui.md#vault-panel): what was made in the app, each enabled
// folder watch's, each connected account's; the vault panel lists them
// (features/sidebar.js), the lens head names the one filtering
// (features/header.js). An account's without a row filters no more.

import { t } from '../core/text.js';
import { store } from '../model/state.js';
import { accounts, watchSourceKey, watches } from '../model/entities.js';
import { KIND_ICONS } from '../ui/icons.js';
import { watchName, watchType } from './workspace/folders.js';

// The sources as the vault panel lists them: what was made in the app; each
// enabled folder watch's, under its name (a statements watch's is its bank
// account's), what it imported; each connected account's.
export function vaultSources() {
  return [
    { key: 'created', name: t('sidebar.created_here'), icon: '#i-home' },
    ...[...watches.values()].filter(watch => watch.enabled).map(watch => {
      const { icon, also } = watchType(watch.type);
      return { key: watchSourceKey(watch.id), name: watchName(watch), icon, also, watch };
    }),
    ...[...accounts.values()].flatMap(accountSources),
  ];
}

// Sources: what a connected account feeds into the vault. Email follows the
// granted access; calendars are the ones checked on the Accounts page
// (`account.calendars`). `autoSync`: the host syncs it on its own.
export function accountSources(account) {
  if (account.kind !== 'google' || account.status !== 'connected') return [];
  const { id, scope = {}, email = t('sidebar.email') } = account;
  const sources = [];
  if (scope.email && scope.email !== 'off') {
    sources.push({
      key: `${id}\nemail`, kind: 'email', account: id, name: email, icon: KIND_ICONS.email.icon,
      also: t('sidebar.fetch_also'),
      autoSync: !!account.fetch_email?.auto_sync?.enabled,
    });
  }
  if (scope.calendar && scope.calendar !== 'off') {
    for (const calendar of account.calendars ?? []) {
      sources.push({
        key: `${id}\ncalendar\n${calendar.id}`, kind: 'calendar', account: id,
        calendar: calendar.id, name: calendar.name, icon: KIND_ICONS.calendar.icon,
        also: t('sidebar.sync_also'),
        autoSync: !!calendar.auto_sync,
      });
    }
  }
  return sources;
}

// An account's source that has no row any more -- the account removed or
// disconnected, its access or the calendar off -- is no filter either, as a
// watch's isn't (model/events.js); its key holds a newline.
store.subscribe(state => state.accounts_rev, () => {
  const { selected_source } = store.getState();
  if (!selected_source?.includes('\n')) return;
  if (!vaultSources().some(source => source.key === selected_source)) store.setState({ selected_source: null });
});
