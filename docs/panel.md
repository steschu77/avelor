# Avelor — Side panel and structural edits

> What the user organizes: projects, and the structural edits (titles)
> with how they are saved. The lenses and the vault panel are `ui.md`.

## Projects <a id="projects"></a>

Projects are entities with the `project` role (`vault.md#project`); the
vault panel lists them and filters by them (`ui.md#vault-panel`).

- **Rename**: the pen on a sidebar project row (`editProject`), or the
  rename button in the vault header's project toolbar (`editProjectName`,
  which edits `#project-name` in place): the name
  becomes an input; Enter
  or leaving it sends `set_title` if the text changed, Esc cancels. The
  pushed `entity` bumps `projects_rev`, so the header follows.

## Saving structural edits <a id="saving-structure"></a>

The vault file is the source of truth, a file is written only when
something in it changed, and the host answers with what is now on disk.

| | Structure |
|---|---|
| The page sends | an intent: `set_title`, … |
| The host writes | the matching `Index` op → `Index::save` |
| The host answers | `entity` / `entity_deleted` per changed entity, then the command's answer |
| Stale-reply guard | none needed: nothing is applied optimistically |
| On failure | an error answer, shown in the status bar; the page shows whatever the host pushed, so it never shows unsaved structure |
| File changed outside the app | `ok {conflict: {id, changed}}`, nothing written; the dialog offers Use theirs, after which the user makes the edit again |

- **The host owns the rules.** The rules live in av-vault where
  they are unit-tested, not in the page. Each op works on any entity,
  built on a property every entity has (`architecture.md#conventions`).
- **Validate, then write.** Every entity an edit will rewrite must be
  writable (`Entry::is_writable`: has a file, not read-only, frontmatter
  parsed) before anything changes; otherwise nothing is written and the
  error is `NotWritable`. Each file to rewrite must also be as the app
  last read or wrote it (its `FileStamp`, `saving.md#the-stamp`); a
  changed or deleted one makes the edit a `SaveOutcome::Conflict` that
  writes nothing. A save that still fails (I/O) restores that entity's
  in-memory roles and envelope, so the index keeps matching the disk.
  What was written is recorded (`vault::Changes`) and pushed
  (`architecture.md#vault-edits`).

## Flows

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| C2 | Rename a project | pill pen or header rename → inline input, Enter/blur | `set_title` → `entity` | header follows (`projects_rev`) |
