# Avelor — Import from folder watches

> Folder watches: the watch files. The commands are `api.md#watches`.

## Overview

A folder watch names a folder on this PC, the files to take from it
(patterns, subfolders) and how to read them (its type).

## Folder watches <a id="folder-watches"></a>

Watches belong to a vault, like accounts. Opening another vault switches to its
watches (`folders::open`, on every vault open), and without an open vault
there are none.

### Watch files <a id="watch-files"></a>

Each watch is a YAML file in the vault's `.avelor/watches/` folder, named
`watch_<type>.yaml`, then `watch_<type>_2.yaml`, …. It is read as an
account file is (`accounts.md#vault-accounts`). The file contains:

* `id`: UUIDv7, unique identifier for the watch
* `folder-watch`:
  * `type`: what the watch takes, chosen when it is added: `images` (scanned
    pages, OCR; several images can be one document)
  * `name`: shown in the sidebar; absent: the folder's name
  * `folder`: as entered; `%VAR%`s such as `%USERPROFILE%` are expanded
    where it is used (on Linux a leading `~` and `$NAME`s). A folder
    picked with Browse under the user's profile is written as
    `%USERPROFILE%\…` (on Linux `~/…`; `sys::collapse_env`).
  * `subfolders`: `true` if files in subfolders are taken too (absent:
    only the folder's own files)
  * `patterns`: file name patterns (`*.jpg`); a file matching any of them
    is taken
  * `collect`: `images` only: `together` (images taken within 2 minutes of
    each other are the pages of one document) or `single` (one document
    per image)
  * `models`: the model each step that sends a file to an AI picked,
    `{account, model}` (`accounts.md#model-choice`), by step: `read` for
    reading images. A step without an entry has no pick yet
  * `enabled`: `false` while the watch is off
  * `added`: when the watch was added

```yaml
id: 0193a7c1-4b0e-7c1a-9d2f-3a51c8e05be0
folder-watch:
  type: images
  name: Scanner
  folder: '%USERPROFILE%\Scans'
  patterns: ['*.jpg', '*.jpeg', '*.png']
  collect: together
  models:
    read:
      account: 019ff8a6-aa60-7e31-ab4b-3e861cb75fe1
      model: gemini-3.8-flash
  added: 2026-01-03T10:00:00Z
```
