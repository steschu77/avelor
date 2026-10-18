# Avelor — Email

> Email from a Google account into the vault: Fetch email, the auto-sync,
> the new-email notices and the email analysis. The
> account and its `fetch-email:` keys are `accounts.md`; the commands,
> `api.md#email`.

## Fetch email <a id="fetch-email"></a>

The Email source's sync button opens Fetch email (`features/import/email.js`) in
the expanded vault area, filled with the account's last filter
(`accounts.md#google-file`).

- **The filter**: labels (any or all of them), starred, with attachment,
  unread, important, sent by the account, from and to (addresses or
  domains, any of which may match), words, and a date preset ("Since last
  time", a day, a week, a month) or a range. It compiles to a Gmail query
  (`av_google::gmail::EmailFilter`).
- **The preview** lists the matches — headers only, 50 per page, at most
  2000 matches — and marks those already in the vault, by their Gmail id
  or Message-ID. It updates as the filter is edited (Auto-update, 500 ms
  after the last change) or on request (Manual).
- **Fetch** (or Ctrl+Enter) downloads the selected emails (select new, all
  or none), converts each body to markdown — plain text: the HTML reduced
  to text, quoted replies as quotes, links kept, attached files named at
  the end — and writes it as an email entity to `Email/` in the vault. A
  selected email that is already in the vault gets its body replaced,
  keeping its frontmatter; the page asks for a second click before doing
  that. The fetch is a job (`ui.md#jobs-panel`); the page closes, and
  Fetch email reads its matches again when the job ends. Esc closes the
  page.
- **Analysis** (section 2): Off or Lite, with the model it runs
  with; the new emails are analysed after the fetch (`email.md#analysis`).

## Email auto-sync <a id="email-auto-sync"></a>

The Email row of a connected account's access (Workspace → Accounts) has an
Auto-sync check. Ticked, the account fetches new email on its own: the email
that matches the filter set up underneath (Fetch email's filter form,
`renderEmailFilterForm`, without the date) and arrived since auto-sync was
switched on, then since its last complete run.

- It runs once the vault is open and indexed, then every 15 minutes while
  the app runs, one account at a time and never beside a fetch of the page
  (`architecture.md#the-clock`).
- A run that finds nothing new leaves no trace; one that does is a
  "Syncing email · <email>" job, and the account's `last-sync` moves on.
- Emails already in the vault are skipped, never replaced.
- A failed run is shown once in the status bar, not again until a run
  succeeds.
- The Email source's sync button stays shown, pressed, while auto-sync is
  on; it still opens Fetch email.

## New-email notices

After a fetch or an auto-sync run wrote new email, the status bar says so
(`email::fetch::new_email_notice` → `App::inform`, an `info` notice;
`ui.md#status-bar`).

## Email analysis <a id="analysis"></a>

A filter can have its new email analysed: Fetch email's (section 2,
Analysis, remembered with the filter as `fetch-email.analysis`;
`accounts.md#google-file`). It is off by default. A filter's analysis has
a level, Off or Lite, and the model it runs with, picked as every feature
picks its model (`accounts.md#model-choice`).

- **Explicit picks**: with the analysis on, Fetch waits for a pick that
  works; the page says why, and the host refuses `email_fetch` too, before
  anything is fetched.
- **The job**: when a fetch ends (not when it was stopped), the emails it
  created go to "Analysing email · <email>", one email after another; Stop
  ends it, and so does a vault switch. The spend limit is checked before
  each email; once it is reached, the rest is left and a notice says so.
  The summary: "18 analysed · 1 failed".
- **Lite**: one request per email (`analysis::request::request_for`). It lists the
  labels in use (the 100 used most) and the active projects (the first
  50), then the email: its subject as it came (`email.subject` once the
  title differs), sender, recipients, date, and the body cut at 8,000
  characters. It asks for one JSON object `{type, title, labels,
  projects}`: the email's type if it is a newsletter (news, offers or
  marketing sent to many) or a notification (sent automatically: receipts,
  shipping, alerts, reminders), none if unsure and none for any other; a
  clean title in the email's language (no "Re:", "Fwd:", reference numbers
  or tags, at most 80 characters); up to three labels
  and two projects from the lists, none if unsure. The model's reasoning
  is turned off (`think: false`; sorting an email needs none), and Ollama
  runs it with a context of 16,384 tokens (`ai::CONTEXT_TOKENS`): its default of 4,096 left a long email no room for
  the answer, the reasoning filling the rest.
- **The answer** is read by a pure parser (`analysis::answer::parse`): the JSON
  object in it (code fences and text around it ignored); the title trimmed
  and cut at 120 characters, no change if it is empty or the email's own;
  the type one of the two in any case, anything else left out; labels and
  projects matched to the lists in any case and kept in the vault's
  spelling, unknown ones and repeats dropped. An answer without
  such an object is asked again, the answer sent back with a reminder of
  the object asked for (an empty one just again), three times in all;
  then the email counts as failed ("… (asked 3 times)"). An answer that was
  cut off (`StopReason::MaxTokens`) isn't asked again, as it would be cut
  off the same way: "The model ran out of room before it finished its
  answer". Every answer is in the log whole, with the model's reasoning
  (`ChatResponse::thinking`): at debug level, or at info when it couldn't
  be read.
- **Written at once**, one write per email (`analysis::apply::apply_lite`): the new
  title, the subject kept as `email.subject` (`vault.md#email`), and the type as
  `document: {type}` beside `email:` (`vault.md#document`), unless the
  email has one (the card shows it after the sender,
  `panel.md#cards`); the
  labels added to the email's own, the projects joined. A title the user
  changed meanwhile stays. A wrong label or
  project is removed on the card like any other.
- **Failures** (the provider's error, an unreadable answer, a failed
  write of the email) are counted in the job's summary and
  told in one notice at its end:
  "Email analysis · jan@gmail.com: 2 of 20 emails couldn't be analysed;
  “<title>”: <the first error>".
- The Accounts page's "Used by" names "Fetch email · <email>" while its
  analysis is on.

## Flows

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| G3 | Fetch email | Email source sync button → `openEmailFetch` (`features/import/email.js`) in the expanded vault area → `email_labels`; filter edits (500 ms debounce) → `email_search` (Auto-update; Manual: on Update); paging → `email_headers`; the analysis's level and model; Fetch or Ctrl+Enter → `email_fetch` | answers `{labels}`, `{matches, truncated, headers}`, `{headers}`; then `email_fetch` → `ok {job}` (page closes), job events, `entity` / `content` per email, `account`; the job's end reads the matches again | emails in `Email/`; filter remembered |
| G4 | Auto-sync email | Email row of the account pane → Auto-sync check, filter form (500 ms debounce) | `set_email_sync {account, enabled?, filter?}` → `account`; later, on the clock: a quiet `list_message_ids` → for new email a "Syncing email" job, `entity` per email, `account` | emails in `Email/` on their own |
| G5 | Analyse new email | a fetch with its analysis on ends with new emails | `analysis::analyse` → `AiAccounts::choose` (refused: a `notice`) → job "Analysing email"; per email `check_limit`, `chat_stream` on a worker, `parse`, `apply_lite` through the vault edit → `entity`, `job`, `account`; at the end one `notice` for the failures | emails retitled, labelled, in projects, their type kept; `email.subject` kept |

## Design <a id="design"></a>

```mermaid
sequenceDiagram
    participant P as page
    participant S as App
    participant W as workers (≤ 4)
    P->>S: email_fetch {account, ids, replace, filter}
    S->>S: plan_fetch (skip known Gmail ids)
    S-->>P: job (running), ok {job}
    loop until all ids done (or cancel_job)
        S->>W: get_raw_message(id)
        W-->>S: Downloaded::parse (av_vault::email → markdown)
        S->>S: fetch_action → create / replace / skip in Email/
        S-->>P: entity (+ content) or notice, job
    end
    S->>S: google::finish_sync (last-sync, last-fetch)
    S-->>P: account, job (done)
```

The fetch is plan → run → apply (`architecture.md#conventions`):

- **plan** on the UI thread: `plan_fetch` skips the Gmail ids the vault
  knows;
- **run** on up to 4 workers: the raw message, parsed by
  `Downloaded::parse` — av-vault's MIME parser (`email.rs`) and its HTML →
  markdown converter (`av_vault::html::to_markdown`);
- **apply** on the UI thread: `fetch_action` decides create, replace or
  skip, and `email_entity` maps the email to an entity (pure, tested); the
  write goes through the vault edit.

The analysis (`analysis.rs`) is plan → run → apply as well: the plan picks
the model and gathers the vault's context and each email's text on the UI
thread; the run is one request at a time under the job's `Cancel`
(streamed like the assistant's, the pieces unused); the apply reads the
answer and writes it through the vault edit. Each analysis is its own
job in `AnalysisState`.

`KnownEmails` (`email.rs`) tells which emails are in the vault: by `gmail-id`
from the same account, or by Message-ID from any source. The preview's
header pages are fetched 8 in parallel (av-google's `gmail::get_headers`, scoped
threads). The fetch's state lives in `EmailState`; a newer search
supersedes a running one on the page's `email-preview` channel. The
auto-sync (`auto_sync`, `sync_query`) runs from the clock with the
schedule it shares with the calendar (`SYNC_EVERY`, `AutoSyncs` in
`google.rs`): a quiet search first, and a fetch job only for new email. A
404 from Gmail is read by matching `Error::Http { status, .. }` in
`email.rs`.
