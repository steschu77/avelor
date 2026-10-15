"""Checks a catalog of the app's texts against English (docs/localization.md).

Usage: python tools/check_catalog.py app/texts/fr.json [app/texts/pt.json ...]

Lists the keys a catalog lacks, with how much of English it covers; English
fills those in. Fails on what the build would refuse: a catalog that isn't
an object of texts, a count's forms without "other" or with a category
Intl.PluralRules doesn't have, a key English doesn't have (stale), and a
placeholder English's text doesn't have. The standard library only.
"""

import json
import re
import sys
from pathlib import Path

CATEGORIES = {'zero', 'one', 'two', 'few', 'many', 'other'}
PLACEHOLDER = re.compile(r'\{(\w+)\}')


def texts_of(path):
    """The catalog at `path`: its texts by key, or a SystemExit naming what's
    wrong with its shape."""
    try:
        catalog = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, ValueError) as e:
        sys.exit(f'{path}: {e}')
    if not isinstance(catalog, dict):
        sys.exit(f'{path}: not an object of texts')
    return catalog


def shape_problems(key, text):
    if isinstance(text, str):
        return []
    if not isinstance(text, dict):
        return [f'"{key}": neither a text nor a count\'s forms']
    problems = [f'"{key}": "{c}" isn\'t a plural category'
                for c in text if c not in CATEGORIES]
    problems += [f'"{key}": its form "{c}" isn\'t a text'
                 for c, form in text.items() if not isinstance(form, str)]
    if 'other' not in text:
        problems.append(f'"{key}": its forms have no "other"')
    return problems


def placeholders(text):
    forms = ([text] if isinstance(text, str)
             else [f for f in text.values() if isinstance(f, str)])
    return {name for form in forms for name in PLACEHOLDER.findall(form)}


def check(path, english):
    catalog = texts_of(path)
    problems = []
    for key, text in catalog.items():
        problems += shape_problems(key, text)
        if key not in english:
            problems.append(f'"{key}" isn\'t in en.json (stale: remove it)')
            continue
        extra = placeholders(text) - placeholders(english[key])
        if extra:
            names = ', '.join(f'{{{name}}}' for name in sorted(extra))
            problems.append(f'"{key}" has {names}, which en.json\'s hasn\'t')
    lacking = sorted(key for key in english if key not in catalog)
    covered = len(english) - len(lacking)
    share = 100 * covered // max(len(english), 1)
    print(f'{path.name}: {covered} of {len(english)} texts ({share}%)')
    for key in lacking:
        shown = json.dumps(english[key], ensure_ascii=False)
        print(f'  lacks "{key}": {shown}')
    for problem in problems:
        print(f'  {problem}')
    return not problems


def main(args):
    if not args:
        sys.exit(__doc__)
    ok = True
    for arg in args:
        path = Path(arg)
        if path.name == 'en.json':
            continue
        english = texts_of(path.with_name('en.json'))
        ok = check(path, english) and ok
    sys.exit(0 if ok else 1)


if __name__ == '__main__':
    main(sys.argv[1:])
