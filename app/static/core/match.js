// Names found by words typed (Go to, ui.md#go-to): every word, in any
// order, somewhere in the name or in the other words it is found by, case
// and accents aside. Folding a text is what costs, so a name is prepared
// once and the words' patterns made once, not for each name.

// `text` without case and accents, a character for a character, so a place
// in one is the same in the other.
function fold(text) {
  return text.replace(/[^ -~]/g, c => c.normalize('NFD')[0].toLowerCase()[0]).toLowerCase();
}

// `name` and the other words it is found by, `also`, ready to be matched.
export function prepare(name, also = []) {
  return { name, low: fold(name), also, lowAlso: also.map(fold) };
}

// The words typed, folded, each once, with what finds one at a word's start
// and as a whole word.
export function wordsOf(text) {
  return [...new Set(fold(text).split(/\s+/).filter(Boolean))].map(word => {
    const start = `(^|[^\\p{L}\\p{N}])${word.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`;
    return { word, start: new RegExp(start, 'u'), whole: new RegExp(`${start}($|[^\\p{L}\\p{N}])`, 'u') };
  });
}

// How well the words fit the prepared name, 0 if one is neither in it nor
// in `also`: the name's start counts more than a word's start, that more
// than the inside of a word, and `also` least; a whole word more than one it
// starts.
export function fit(words, { low, lowAlso }) {
  let sum = 0;
  for (const { word, start, whole } of words) {
    const at = low.indexOf(word);
    if (at < 0) {
      if (!lowAlso.some(other => other.includes(word))) return 0;
      sum += 1;
    } else if (start.test(low)) {
      sum += (at === 0 ? 4 : 3) + (whole.test(low) ? 0.5 : 0);
    } else {
      sum += 2;
    }
  }
  return words.length ? sum / words.length : 1;
}

// Of the prepared name's `also`, those found by a word that isn't in it.
export function foundBy(words, { low, also, lowAlso }) {
  const missing = words.filter(({ word }) => !low.includes(word));
  return also.filter((_, i) => missing.some(({ word }) => lowAlso[i].includes(word)));
}

// `text` cut where the words are in it: `{ text, hit }` in turn.
export function cutAtWords(words, text) {
  const low = fold(text);
  const on = new Array(text.length).fill(false);
  for (const { word } of words) {
    for (let at = low.indexOf(word); at >= 0; at = low.indexOf(word, at + 1)) on.fill(true, at, at + word.length);
  }
  const pieces = [];
  for (let from = 0, to = 0; from < text.length; from = to) {
    while (to < text.length && on[to] === on[from]) to++;
    pieces.push({ text: text.slice(from, to), hit: on[from] });
  }
  return pieces;
}
