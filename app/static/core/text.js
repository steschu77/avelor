// The page's texts in the run's language (localization.md): its catalog,
// the language's texts over English's, which the host put into the page
// (#texts, page.rs), and its locale, the page's lang. The web view is given
// the same locale, so the dates and numbers of core/format.js follow it.

import { $ } from './dom.js';

const locale = document.documentElement.lang || 'en';
const catalog = JSON.parse($('texts')?.textContent || '{}');
const forms = new Intl.PluralRules(locale);
const number = new Intl.NumberFormat(locale);

// ----------------------------------------------------------------------------
// The text of `key` with its placeholders, {name}, filled in from `args`, a
// number as the locale writes it; a count's forms picked by `args.n`. The
// key itself where the catalog has none, which the build rules out.
export function t(key, args = {}) {
  return lookup(key, args);
}

// ----------------------------------------------------------------------------
// A text the host sent (texts.rs): `{ key, args }`, an argument a text too.
export function textOf(text) {
  if (!text?.key) return '';
  const args = {};
  for (const [name, value] of Object.entries(text.args ?? {})) args[name] = argOf(value);
  return lookup(text.key, args);
}

function argOf(value) {
  if (value?.key) return textOf(value);
  return value;
}

function lookup(key, args) {
  const entry = catalog[key] ?? key;
  const text = typeof entry === 'string' ? entry : entry[forms.select(args.n)] ?? entry.other;
  return text.replace(/\{(\w+)\}/g, (placeholder, name) => {
    const value = args[name];
    if (value == null) return placeholder;
    return typeof value === 'number' ? number.format(value) : String(value);
  });
}
