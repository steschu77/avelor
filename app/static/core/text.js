// The page's texts in the run's language (localization.md): its catalog,
// the language's texts over English's, which the host put into the page
// (#texts, page.rs), and its locale, the page's lang, which core/format.js
// formats its dates, numbers and sorts in, whatever the system's region.

import { $ } from './dom.js';

export const locale = document.documentElement.lang || 'en';
const catalog = JSON.parse($('texts')?.textContent || '{}');
const forms = new Intl.PluralRules(locale);
const ordinals = new Intl.PluralRules(locale, { type: 'ordinal' });
const number = new Intl.NumberFormat(locale);
const day = new Intl.DateTimeFormat(locale);

// ----------------------------------------------------------------------------
// The text of `key` with its placeholders, {name}, filled in from `args`, a
// number as the locale writes it; a count's forms picked by `args.n`. The
// key itself where the catalog has none, which the build rules out.
export function t(key, args = {}) {
  return lookup(key, args);
}

// ----------------------------------------------------------------------------
// The text of `key` as `t` gives it, in parts for `append()`: an argument
// that is a node (a picker within a sentence) stays that node.
export function parts(key, args = {}) {
  const nodes = {};
  const marked = {};
  for (const [name, value] of Object.entries(args)) {
    if (value instanceof Node) {
      nodes[name] = value;
      marked[name] = MARK + name + MARK;
    } else {
      marked[name] = value;
    }
  }
  return lookup(key, marked).split(MARK).map((piece, i) => (i % 2 ? nodes[piece] : piece)).filter(piece => piece !== '');
}

const MARK = String.fromCharCode(1);

// ----------------------------------------------------------------------------
// `n` as an ordinal, "2nd" in English, "2." in German: the catalog's forms
// picked by the locale's ordinal rules.
export const ordinal = n => t('ordinal', { n, ordinal: true });

// ----------------------------------------------------------------------------
// A text the host sent (texts.rs): `{ key, args }`, an argument a text too,
// a date (`{ date }`) or a sum of money (`{ amount, currency }`); a list of
// texts is a job's summary, its parts joined.
export function textOf(text) {
  if (Array.isArray(text)) return text.map(textOf).join(' \u00b7 ');
  if (!text?.key) return '';
  const args = {};
  for (const [name, value] of Object.entries(text.args ?? {})) args[name] = argOf(value);
  return lookup(text.key, args);
}

function argOf(value) {
  if (value?.key) return textOf(value);
  if (value?.date) {
    const [y, m, d] = value.date.split('-').map(Number);
    return day.format(new Date(y, m - 1, d));
  }
  if (value?.currency) {
    const money = new Intl.NumberFormat(locale, { style: 'currency', currency: value.currency });
    return money.format(value.amount);
  }
  return value;
}

function lookup(key, args) {
  const entry = catalog[key] ?? key;
  const rules = args.ordinal ? ordinals : forms;
  const text = typeof entry === 'string' ? entry : entry[rules.select(args.n)] ?? entry.other;
  return text.replace(/\{(\w+)\}/g, (placeholder, name) => {
    const value = args[name];
    if (value == null) return placeholder;
    return typeof value === 'number' ? number.format(value) : String(value);
  });
}
