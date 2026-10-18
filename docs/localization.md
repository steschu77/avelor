# Avelor — Localization

> The app's texts in the user's language: the catalogs, how the page and
> the host take a text from them, the language a run shows, the build's
> check, and how a language is added.

## The catalogs <a id="catalogs"></a>

Every text the app shows is in a catalog, one per language:
`app/texts/<language>.json`, named by the language's code (`de.json`).
A catalog is one JSON object of texts by key:

```json
{
  "status.dismiss": "Dismiss",
  "status.more": "+{n} more"
}
```

- **A key** names what the text means, not where it shows
  (`status.dismiss`), since a text can move from one module to another.
- **A text** is plain, with `{name}` placeholders for what it is given; a
  number given is written as the locale writes numbers.
- **A count's forms**: where a language says a count's noun in more than
  one way, the text is an object of its forms, keyed by the categories of
  `Intl.PluralRules` (`zero`, `one`, `two`, `few`, `many`, `other`), and
  the count is the argument `n`. `other` is there always and stands in for
  a form a catalog doesn't have.
- **English, German and Spanish** are the app's own: each has every key
  English has. A language from outside may lack some; English fills them
  in.
- **The way of address**: the catalogs address the user informally ("du",
  "tú"). A language's formal catalog (`de.formal.json`, `es.formal.json`)
  holds the texts that address the user, formally ("Wählen Sie ein
  Modell."); its language's catalog has the rest. A new text that
  addresses the user gets its formal version too.

The catalogs are UTF-8, outside `app/static` and its ASCII rule
(`architecture.md`, ASCII-only sources).

## The page <a id="page"></a>

The page shows `t(key, args)` (`core/text.js`): the text of `key` in the
run's language with `args` filled in. The host puts the language's
catalog into the page (the `texts` block, `page.rs`), the language's
texts over English's, and the language as the page's `lang`. A key is
written out where `t` is called (`t('status.dismiss')`), never built, so
the build can check it. A sentence with a live element in it, a picker,
is `parts(key, args)`: the text in pieces for `append()`, an argument
that is an element kept as it is.

The page's own markup (`index.html`) names its texts by key too, and the
host fills them in as it serves the page: `data-t="lens.bank"` is the
element's text, which it has none of in the markup, and
`data-t-title="editor.bold"` is its `title` (and so for any attribute).
A date, a count, a relative day, an amount and the order of texts follow
the language's locale through `Intl` (`core/format.js`), not the
system's region, which the web view would take.

## The host's texts <a id="host-texts"></a>

The host gives the page a text as its key and what its placeholders are
given, `{key, args}` (`texts::Text`): a notice, an error's reply (the
host's words, or the code's own text), a job's label and its summary (a
list of parts), and the words in its events (a scanned page that failed,
a reply that failed, a trashed file that can't be read). An argument is
a string, a number, a text itself or a sum of money (`texts::money`),
which the page writes as its locale does (`textOf`); a count's form
follows the language's rules, a contributed language's too.

What the host shows or writes itself it takes from the same catalogs,
plain texts only: the failed start's box, the folder picker's title and
the sign-in pages (`texts::text`, `texts::filled`), and what it writes
into the vault, in the run's language (`Text::render`): an email's "No
subject", the title of the email analysis's conversation.

## The language <a id="language"></a>

A run shows one language, chosen at its start:

1. `language` in `config.yaml` (`language: de`), if a catalog has it;
   after a comma, how the texts address the user (`language: de,
   informal`, the default, or `language: de, formal`, which takes the
   language's formal catalog where it has one); `language: system, formal`
   is the system's language formally;
2. else the first of the system's languages a catalog has: the display
   languages of Windows, in their order; on Linux `LANGUAGE`, `LC_ALL`,
   `LC_MESSAGES` and `LANG`, as GLib reads them;
3. else English.

The language keeps the system's region (`de` on a system in `de-AT` is
`de-AT`), and the web view is given it too: its dates and numbers, its
date inputs and its spelling follow it. Settings picks it
(`ui.md#settings`, `set_language`), and the page is told with the answer to
`ready` what is picked, what the run shows and what there is to pick: the
catalogs' languages, with the two forms of address of one that has both, each
in its own word (`language.form`: "du", "Sie") and with a sentence in it
(`language.form_says`). A region set in the file (`de-CH`) stays while the
language does. A change takes effect at the next start.

## The build's check <a id="check"></a>

The build reads the catalogs and the keys the page's script and the
host's sources use (`av-texts`), and fails, each problem listed with its
catalog, on:

- a catalog that isn't an object of texts, a count's forms without
  `other` or with a category that isn't one;
- a key used that English lacks, and one of English's used nowhere;
- a key a catalog has that English lacks, or a placeholder that English's
  text doesn't have;
- a key English has that German or Spanish lacks;
- a formal catalog without its language's, or without its own
  `language.form`, or a file named for another variant;
- an element of `index.html` with a key and a text of its own, and the
  page's script with a `data-t` mark, which the host would fill in;
- a count's forms for a text the host shows itself;
- a `t` or `parts` whose key isn't written out.

## A new language

A language comes as a pull request with its catalog:

1. Copy `app/texts/en.json` to the language's file (`fr.json`) and
   translate its texts; keep the keys and the placeholders. A count's
   forms take the categories the language has (CLDR's plural rules).
   Where the language has a formal way of address, the texts that address
   the user can go formally into `fr.formal.json` too, with what it is
   called (`language.form`, "vous").
2. Run `python tools/check_catalog.py app/texts/fr.json`: it lists the
   keys the catalog lacks, with how much of English it covers, and fails
   on a key English no longer has, a placeholder English's text doesn't
   have, or a form that isn't one. CI runs it on every pull request.
3. Pick the language in Settings, start the app again and look through it.

## Not localized

The vault's format (`Contacts/` stays `Contacts/`, the label
`conflict`), the docs, the code's names and comments, the models'
instructions, the log, and the messages only a developer sees ("There is
no host.").