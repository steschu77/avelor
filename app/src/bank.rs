//! Account statements (import.md#statements): a statements watch imports the
//! CSV exports of its bank account, each new file a statement of the account
//! in `Accounts/` (`statements_import`, a job). One bank's format is read
//! (`read`: comdirect's export of an account's transactions); `plan` decides what
//! a statement adds to its account, `pairs` which of its transactions are
//! transfers to or from the vault's other accounts, and
//! `recurring_payments::join_new` which recurring payments its new transactions pay. The account is a
//! bank-account entity (`new_account`, `set_bank_account`).

use std::collections::HashSet;
use std::path::Path;

use av_utils::datetime::{Date, DateTime, Time};
use av_utils::error::Error;
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{BankAccount, Bare, Import, Roles, Source, Transaction, to_cents};
use av_vault::schema::layout;
use av_vault::shared::ContactRef;
use serde::Deserialize;
use serde_json::json;

use crate::app::App;
use crate::jobs::{JobKind, count_summary};
use crate::protocol::Req;
use crate::recurring_payments;
use crate::structure;
use crate::texts::{self, Text};
use crate::watches::{self, WatchKind, WatchSettings};
use comdirect::{Export, Row, read};
use plan::{Known, OtherAccount, Pair, Plan, pairs, plan};

mod comdirect;
mod plan;
/// A bank account's types (`bank-account.type`).
const ACCOUNT_TYPES: [&str; 3] = ["checking", "savings", "credit-card"];
// ----------------------------------------------------------------------------
/// `text` trimmed, if anything is left.
fn some(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

// ----------------------------------------------------------------------------
/// The months from `first` to `last`: "December 2026", "Nov – Dec 2026",
/// "Dec 2026 – Jan 2027".
fn period(first: Date, last: Date) -> String {
    let (y1, m1, _) = first.to_ymd();
    let (y2, m2, _) = last.to_ymd();
    let (m1, m2): (i32, i32) = (m1.into(), m2.into());
    let (y1, y2) = (y1.to_string(), y2.to_string());
    let (from, to) = (short_month(m1), short_month(m2));
    match (y1 == y2, m1 == m2) {
        (true, true) => {
            let args = [("month", month(m1)), ("year", &y1)];
            texts::filled("bank.period.month", &args)
        }
        (true, false) => {
            let args = [("from", from), ("to", to), ("year", &y1)];
            texts::filled("bank.period.months", &args)
        }
        _ => {
            let args = [
                ("from", from),
                ("from_year", &y1),
                ("to", to),
                ("to_year", &y2),
            ];
            texts::filled("bank.period.years", &args)
        }
    }
}

/// The name of the month `m`, 1 to 12, in the run's language.
fn month(m: i32) -> &'static str {
    let names = [
        texts::text("month.1"),
        texts::text("month.2"),
        texts::text("month.3"),
        texts::text("month.4"),
        texts::text("month.5"),
        texts::text("month.6"),
        texts::text("month.7"),
        texts::text("month.8"),
        texts::text("month.9"),
        texts::text("month.10"),
        texts::text("month.11"),
        texts::text("month.12"),
    ];
    names[m as usize - 1]
}

/// The month `m`'s short name, 1 to 12, in the run's language.
fn short_month(m: i32) -> &'static str {
    let names = [
        texts::text("month.short.1"),
        texts::text("month.short.2"),
        texts::text("month.short.3"),
        texts::text("month.short.4"),
        texts::text("month.short.5"),
        texts::text("month.short.6"),
        texts::text("month.short.7"),
        texts::text("month.short.8"),
        texts::text("month.short.9"),
        texts::text("month.short.10"),
        texts::text("month.short.11"),
        texts::text("month.short.12"),
    ];
    names[m as usize - 1]
}

// ----------------------------------------------------------------------------
/// The bank account of a statements watch, as an import uses it.
struct Account {
    id: Id,
    title: String,
    number: Option<String>,
    holder: Option<ContactRef>,
}

// ----------------------------------------------------------------------------
/// The account the watch of `settings` imports into; or why there is none.
fn account_of(app: &App, settings: &WatchSettings) -> Result<Account, Text> {
    let gone = || Text::new("bank.watch_account_gone");
    let id = settings.account.ok_or_else(gone)?;
    let doc = app.vault.index().get(&id).ok_or_else(gone)?;
    let account = doc.roles().bank_account.as_ref().ok_or_else(gone)?;
    Ok(Account {
        id,
        title: doc.title().to_string(),
        number: account.number.clone(),
        holder: account.holder.clone(),
    })
}

// ----------------------------------------------------------------------------
/// Whether `id` is a bank account in the vault.
pub(crate) fn is_account(app: &App, id: Id) -> bool {
    app.vault
        .index()
        .get(&id)
        .is_some_and(|doc| doc.roles().bank_account.is_some())
}

// ----------------------------------------------------------------------------
/// The statements of the account `account` in the vault.
fn known(app: &App, account: Id) -> Vec<Known> {
    let index = app.vault.index();
    index
        .statements_of(account)
        .map(|(doc, statement)| Known {
            id: doc.id(),
            items: statement.transactions.clone(),
            balance: statement.balance.map(to_cents),
        })
        .collect()
}

// ----------------------------------------------------------------------------
/// The files the statements of the account `account` came from: a file is
/// in the vault by its account and origin, whichever watch imported it.
fn account_origins(app: &App, account: Id) -> HashSet<String> {
    let index = app.vault.index();
    index
        .statements_of(account)
        .filter_map(|(doc, _)| match doc.source()? {
            Source::Import(import) => import.origin.clone(),
            _ => None,
        })
        .collect()
}

// ----------------------------------------------------------------------------
/// The vault's bank accounts but `account`, with their statements.
fn others_of(app: &App, account: Id) -> Vec<OtherAccount> {
    let index = app.vault.index();
    let others = index.all().iter().filter(|entry| entry.doc.id() != account);
    let other = |doc: &Document| {
        let fields = doc.roles().bank_account.as_ref()?;
        Some(OtherAccount {
            number: fields.number.clone(),
            statements: known(app, doc.id()),
        })
    };
    others.filter_map(|entry| other(&entry.doc)).collect()
}

// ----------------------------------------------------------------------------
/// Pair the transfers among `transactions`, new transactions of `account`, with the
/// vault's other accounts (`pairs`): each new half names its other half.
/// The other halves to write: each its statement, its id and the new half's.
fn pair_transfers(
    app: &App,
    account: &Account,
    transactions: &mut [Transaction],
) -> Vec<(Id, Id, Id)> {
    let found = pairs(
        account.number.as_deref(),
        transactions,
        &others_of(app, account.id),
    );
    let half = |pair: Pair| {
        let transaction = &mut transactions[pair.transaction];
        transaction.transfer = Some(pair.other);
        Some((pair.statement, pair.other, transaction.id?))
    };
    found.into_iter().filter_map(half).collect()
}

// ----------------------------------------------------------------------------
/// Write the other halves of new transfers (`pair_transfers`), each
/// statement once, each half naming its new one; how many were written.
fn write_halves(app: &mut App, halves: &[(Id, Id, Id)]) -> usize {
    let statements = by_statement(halves.iter().map(|(s, other, new)| (*s, (*other, *new))));
    let mut written = 0;
    for (statement, own) in statements {
        let others: Vec<Id> = own.iter().map(|(other, _)| *other).collect();
        let outcome = app.edit_written(|index, changes| {
            index.edit_transactions(&statement, &others, changes, |item| {
                if let Some((_, new)) = own.iter().find(|(other, _)| item.id == Some(*other)) {
                    item.transfer = Some(*new);
                }
            })
        });
        match outcome {
            Ok(_) => written += own.len(),
            Err(e) => {
                log::warn!("Statement import: could not pair the transfers of {statement}: {e}")
            }
        }
    }
    written
}

// ----------------------------------------------------------------------------
/// `items` grouped by their statement's id, the statements in the order
/// they first come.
pub(crate) fn by_statement<T>(items: impl IntoIterator<Item = (Id, T)>) -> Vec<(Id, Vec<T>)> {
    let mut statements: Vec<(Id, Vec<T>)> = Vec::new();
    for (id, item) in items {
        match statements.iter_mut().find(|(s, _)| *s == id) {
            Some((_, items)) => items.push(item),
            None => statements.push((id, vec![item])),
        }
    }
    statements
}

// ----------------------------------------------------------------------------
/// The own account number of a pending row of `statement` that isn't
/// `number` (`same_number`).
fn other_account<'a>(statement: &'a Export, number: Option<&str>) -> Option<&'a str> {
    let number = number.filter(|number| has_digits(number))?;
    let rows = statement.rows.iter();
    rows.filter_map(|row| row.own.as_deref())
        .find(|own| has_digits(own) && !same_number(own, number))
}

// ----------------------------------------------------------------------------
/// Whether `a` and `b` name the same account, compared by their digits: one
/// may end the other, as an IBAN ends with the account's number.
fn same_number(a: &str, b: &str) -> bool {
    let digits = |text: &str| {
        text.chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
    };
    let (a, b) = (digits(a), digits(b));
    !a.is_empty() && !b.is_empty() && (a.ends_with(&b) || b.ends_with(&a))
}

// ----------------------------------------------------------------------------
/// Whether `text` has digits, which an account's number is compared by.
fn has_digits(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
}

// ----------------------------------------------------------------------------
/// A new bank account's fields (`create_watch`'s `new_account`): its name,
/// number and type, which it needs, and its bank.
#[derive(Deserialize)]
pub(crate) struct NewAccount {
    #[serde(default)]
    title: String,
    #[serde(default)]
    number: String,
    #[serde(rename = "type", default)]
    account_type: String,
    #[serde(default)]
    bank: String,
}

// ----------------------------------------------------------------------------
/// A new bank account in `Accounts/` with the fields of `new`; its id, or
/// why there is none, with what to tell the page. Nothing is written
/// without its name, number and type.
pub(crate) fn new_account(app: &mut App, new: NewAccount) -> Result<Id, (Error, Text)> {
    let invalid = |arg: &str, text: Text| {
        let arg = arg.to_string();
        (Error::InvalidArgument { arg }, text)
    };
    let Some(title) = some(&new.title) else {
        return Err(invalid("title", Text::new("bank.enter_name")));
    };
    let Some(number) = some(&new.number) else {
        return Err(invalid("number", Text::new("bank.enter_number")));
    };
    if !ACCOUNT_TYPES.contains(&new.account_type.as_str()) {
        return Err(invalid("type", Text::new("bank.choose_type")));
    }
    let account = BankAccount {
        account_type: Some(new.account_type),
        number: Some(number),
        bank: some(&new.bank),
        ..BankAccount::default()
    };
    let roles = Roles {
        bank_account: Some(account),
        ..Roles::default()
    };
    let source = Source::Created(Bare::default());
    let doc = Document::create(roles, source, title, Vec::new(), FileFormat::YAML);
    let created =
        app.edit(|index, changes| index.create(doc, Path::new(layout::BANK_ACCOUNTS), changes));
    created.map_err(|e| {
        log::warn!("Could not write the new bank account: {e}");
        (e, Text::new("bank.create_failed"))
    })
}

// ----------------------------------------------------------------------------
/// `set_bank_account`'s args: the account, and the fields that change.
#[derive(Deserialize)]
pub(crate) struct AccountChange {
    entity: Id,
    bank: Option<String>,
    number: Option<String>,
    #[serde(rename = "type")]
    account_type: Option<String>,
    holder: Option<String>,
}

// ----------------------------------------------------------------------------
/// `set_bank_account {entity, bank?, number?, type?, holder?}`:
/// change the bank account's fields given, an empty one removing its key;
/// `type` is `checking`, `savings` or `credit-card`, `holder` a contact of
/// the vault (`@<uuid>`) or a name. Answered as `set_title` is.
pub fn set_bank_account(app: &mut App, req: Req, change: AccountChange) {
    let id = change.entity;
    if !is_account(app, id) {
        return req.err(Error::NotFound);
    }
    if let Some(account_type) = change.account_type.as_deref() {
        if !ACCOUNT_TYPES.contains(&account_type) {
            return req.err(Error::InvalidArgument { arg: "type".into() });
        }
    }
    let step = Text::new("undo.set_bank_account").arg("title", structure::named(app, &id, ""));
    let result = app.step(&req, step, |index, changes| {
        index.edit(&id, changes, |d| {
            let mut account = d.roles().bank_account.clone().unwrap_or_default();
            if let Some(bank) = change.bank {
                account.bank = some(&bank);
            }
            if let Some(number) = change.number {
                account.number = some(&number);
            }
            if let Some(account_type) = change.account_type {
                account.account_type = Some(account_type);
            }
            if let Some(holder) = change.holder {
                account.holder = some(&holder).as_deref().map(ContactRef::parse);
            }
            d.set_bank_account(Some(account));
        })
    });
    structure::answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// A file of a statements watch's folder that isn't in the vault, as it was
/// read.
struct NewFile {
    name: String,
    taken: DateTime,
    statement: Result<Export, Text>,
}

// ----------------------------------------------------------------------------
/// What an import wrote, for its job's summary.
#[derive(Default)]
struct Counts {
    statements: usize,
    posted: usize,
    pending: usize,
    transfers: usize,
    recurring_payment: usize,
    skipped: usize,
}

// ----------------------------------------------------------------------------
/// `statements_import {watch}`: import the new files of a statements watch
/// into its account as a job, "Importing statements · <account>": listed and
/// read on a worker, then written oldest first (`plan`). Answers `{job}`;
/// the job's summary counts what came in and names each file left out and each
/// statement whose balances don't add up.
pub fn statements_import(app: &mut App, req: Req, watch: Id) {
    let settings = match app.watches.get(watch) {
        Ok(found) => found.settings().clone(),
        Err(e) => return req.err(e),
    };
    if settings.kind != WatchKind::Statements {
        let what = "importing statements from this watch".to_string();
        let text = Text::new("bank.not_a_statements_watch");
        return req.fail(Error::Unsupported { what }, text);
    }
    if settings.folder.is_empty() {
        let arg = "folder".to_string();
        let text = Text::new("bank.choose_folder");
        return req.fail(Error::InvalidArgument { arg }, text);
    }
    let account = match account_of(app, &settings) {
        Ok(account) => account,
        Err(text) => return req.fail(Error::NotFound, text),
    };
    let in_vault = account_origins(app, account.id);
    let label = Text::new("bank.importing").arg("account", account.title.as_str());
    let job = app.start_job(JobKind::StatementImport, label, 1);
    log::info!(
        "Statement import {job}: {} into {}",
        settings.folder,
        account.title
    );
    req.ok(json!({ "job": job }));
    app.run_job(
        job,
        move |_| read_new(&settings, &in_vault),
        move |app, read| {
            let summary = match read {
                _ if app.job_stopping(job) => vec![Text::new("job.stopped")],
                Ok(files) => import_files(app, watch, files),
                Err(Error::FileNotFound { path }) => {
                    let path = path.display().to_string();
                    vec![Text::new("watch.no_folder").arg("path", path)]
                }
                Err(e) => {
                    log::warn!("Could not list the folder of the watch {watch}: {e}");
                    vec![Text::new("watch.list_failed")]
                }
            };
            app.end_job(job, summary);
        },
    );
}

// ----------------------------------------------------------------------------
/// The files of the watch of `settings` not in `in_vault`, read.
fn read_new(settings: &WatchSettings, in_vault: &HashSet<String>) -> Result<Vec<NewFile>, Error> {
    let files = watches::list_files(settings)?;
    let new = files
        .into_iter()
        .filter(|file| !in_vault.contains(&file.name));
    let read_file = |path: &Path| match std::fs::read(path) {
        Ok(bytes) => read(&bytes),
        Err(e) => {
            log::warn!("Could not read the statement {}: {e}", path.display());
            Err(Text::new("bank.file_unread"))
        }
    };
    Ok(new
        .map(|file| NewFile {
            statement: read_file(&file.path),
            name: file.name,
            taken: file.taken,
        })
        .collect())
}

// ----------------------------------------------------------------------------
/// Write the statements of `files` into the account of the watch `watch`,
/// oldest first, but a file whose transactions the account has all; the job's
/// note.
fn import_files(app: &mut App, watch: Id, mut files: Vec<NewFile>) -> Vec<Text> {
    let settings = match app.watches.get(watch) {
        Ok(found) => found.settings().clone(),
        Err(_) => return vec![Text::new("bank.watch_removed")],
    };
    let account = match account_of(app, &settings) {
        Ok(account) => account,
        Err(text) => return vec![text],
    };
    let last_day = |file: &NewFile| {
        let statement = file.statement.as_ref().ok()?;
        statement.rows.iter().filter_map(Row::day).max()
    };
    files.sort_unstable_by_key(last_day);
    let mut counts = Counts::default();
    let mut problems = Vec::new();
    for file in &files {
        let named = |problem: Text| {
            let text = Text::new("bank.file_problem").arg("file", file.name.as_str());
            text.arg("problem", problem)
        };
        if account_origins(app, account.id).contains(&file.name) {
            continue;
        }
        let statement = match &file.statement {
            Ok(statement) => statement,
            Err(why) => {
                problems.push(named(why.clone()));
                continue;
            }
        };
        if let Some(other) = other_account(statement, account.number.as_deref()) {
            let text = Text::new("bank.other_account").arg("number", other);
            problems.push(named(text));
            continue;
        }
        let mut plan = plan(account.holder.as_ref(), &known(app, account.id), statement);
        problems.extend(plan.problems.iter().map(|problem| named(problem.clone())));
        if plan.statement.transactions.is_empty() {
            counts.skipped += plan.skipped;
            continue;
        }
        let halves = if settings.pair_transfers {
            pair_transfers(app, &account, &mut plan.statement.transactions)
        } else {
            Vec::new()
        };
        let joined =
            recurring_payments::join_new(app, account.id, &mut plan.statement.transactions);
        match write_statement(app, watch, &account, file, statement, plan, &mut counts) {
            Ok(()) => {
                counts.statements += 1;
                counts.transfers += write_halves(app, &halves);
                counts.recurring_payment += joined;
            }
            Err(why) => problems.push(named(why)),
        }
    }
    let mut note = count_summary(
        vec![
            (counts.statements, Text::new("bank.count.statements")),
            (counts.posted, Text::new("bank.count.transactions")),
            (counts.pending, Text::new("bank.count.pending")),
            (counts.transfers, Text::new("bank.count.transfers")),
            (counts.recurring_payment, Text::new("bank.count.recurring")),
            (counts.skipped, Text::new("bank.count.known")),
        ],
        Text::new("bank.no_new_statements"),
    );
    note.extend(problems);
    note
}

// ----------------------------------------------------------------------------
/// Write the statement of `file` as `plan` says: the new statement, then
/// the older ones without the pending transactions it replaces, then the
/// account's balance (from its newest statement) and its currency (if it
/// has none). Only the new statement's write failing fails it.
fn write_statement(
    app: &mut App,
    watch: Id,
    account: &Account,
    file: &NewFile,
    statement: &Export,
    plan: Plan,
    counts: &mut Counts,
) -> Result<(), Text> {
    let first = plan.first.unwrap_or(file.taken.date);
    let period = period(first, plan.last.unwrap_or(first));
    let mut role = plan.statement;
    role.account = Some(account.id);
    let pending = role.transactions.iter().filter(|t| t.is_pending()).count();
    let posted = role.transactions.len() - pending;
    let roles = Roles {
        statement: Some(role),
        ..Roles::default()
    };
    let source = Source::Import(Import {
        watch: Some(watch),
        origin: Some(file.name.clone()),
        ..Import::default()
    });
    let args = [("account", account.title.as_str()), ("period", &period)];
    let title = texts::filled("bank.statement_title", &args);
    let mut doc = Document::create(roles, source, title, Vec::new(), FileFormat::YAML);
    doc.set_date(DateTime {
        date: first,
        time: Time::new(0).ok(),
    });
    let folder = Path::new(layout::BANK_ACCOUNTS);
    app.edit(|index, changes| index.create(doc, folder, changes))
        .map_err(|e| {
            log::warn!("Statement import: could not write {}: {e}", file.name);
            Text::new("bank.write_failed")
        })?;
    (counts.posted, counts.pending) = (counts.posted + posted, counts.pending + pending);
    counts.skipped += plan.skipped;

    for (id, items) in plan.replaced {
        let outcome = app.edit_written(|index, changes| {
            index.edit(&id, changes, |d| {
                if let Some(mut older) = d.roles().statement.clone() {
                    older.transactions = items;
                    d.set_statement(Some(older));
                }
            })
        });
        if let Err(e) = outcome {
            log::warn!(
                "Statement import: could not take the pending transactions out of {id}: {e}"
            );
        }
    }
    let balance = statement.closing.filter(|_| plan.newest);
    let currency = statement.currency.clone();
    let outcome = app.edit_written(|index, changes| {
        index.edit(&account.id, changes, |d| {
            let mut fields = d.roles().bank_account.clone().unwrap_or_default();
            if let Some(cents) = balance {
                fields.balance = Some(cents as f64 / 100.0);
            }
            if fields.currency.is_none() {
                fields.currency = currency;
            }
            d.set_bank_account(Some(fields));
        })
    });
    if let Err(e) = outcome {
        log::warn!(
            "Statement import: could not write the balance of {}: {e}",
            account.id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
