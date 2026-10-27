// The words of the vault panel's filter, marked (ui.md#words): the terms
// as the host searched them (`search`), in any case, also inside a longer
// word; the lenses' rows mark them with <mark>, the panel's card with
// highlights (features/panel/find.js).

import { accEl } from '../core/dom.js';

// A pattern finding any of `terms`, the longest first, so a term inside
// another is marked as the longer one.
export function termsRegex(terms) {
  const escaped = [...terms].sort((a, b) => b.length - a.length)
    .map(term => term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  return new RegExp(escaped.join('|'), 'giu');
}

// `text` as nodes: the terms in <mark>, the rest as text.
export function markedNodes(text, terms) {
  if (!terms?.length) return [document.createTextNode(text)];
  const nodes = [];
  let at = 0;
  for (const match of text.matchAll(termsRegex(terms))) {
    if (match.index > at) nodes.push(document.createTextNode(text.slice(at, match.index)));
    nodes.push(accEl('mark', '', match[0]));
    at = match.index + match[0].length;
  }
  if (at < text.length || !nodes.length) nodes.push(document.createTextNode(text.slice(at)));
  return nodes;
}

// A hit's excerpt as nodes: where it is (a label), then its text marked.
export function excerptNodes(hit, terms) {
  const nodes = markedNodes(hit.excerpt, terms);
  if (hit.where) nodes.unshift(accEl('span', 'label', hit.where));
  return nodes;
}
