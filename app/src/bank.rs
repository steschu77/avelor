//! Account statements (import.md#statements): a statements watch imports the
//! CSV exports of its bank account, each new file a statement of the account
//! in `Accounts/` (`statements_import`, a job). One bank's format is read
//! (`read`: comdirect's export of an account's bookings); `plan` decides what
//! a statement adds to its account, and `pairs` which of its bookings are
//! transfers to or from the vault's other accounts. The account is a
//! bank-account entity (`new_account`, `set_bank_account`).

use std::collections::HashSet;
use std::path::Path;

use av_utils::charsets::from_utf8_or_windows1252;
use av_utils::datetime::{Date, DateTime, Time};
use av_utils::error::Error;
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{BankAccount, Empty, Import, Roles, Source, Transaction, Transactions};
use av_vault::schema::layout;
use av_vault::shared::ContactRef;
use serde::Deserialize;
use serde_json::json;

use crate::app::App;
use crate::jobs::{JobKind, count_summary};
use crate::protocol::Req;
use crate::structure;
use crate::watches::{self, WatchKind, WatchSettings};

/// The header row's cells before the amount's, which names the currency.
const HEADER: [&str; 4] = [
    "Buchungstag",
    "Wertstellung (Valuta)",
    "Vorgang",
    "Buchungstext",
];
/// A transfer's or direct debit's purpose comes cut into lines of this many
/// characters, joined by a space.
const PURPOSE_LINE: usize = 35;
/// How many days apart a transfer's two halves may be booked: another bank
/// books it a day, or a weekend, later.
const PAIR_DAYS: i32 = 3;
/// A bank account's kinds (`bank-account.type`).
const ACCOUNT_KINDS: [&str; 3] = ["checking", "savings", "credit-card"];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

// ----------------------------------------------------------------------------
/// A statement as the bank exported it; amounts in cents.
#[derive(Debug, Default)]
pub(crate) struct Statement {
    pub currency: Option<String>,
    /// The balance before its bookings, and after them.
    pub opening: Option<i64>,
    pub closing: Option<i64>,
    /// Newest first, as the bank lists them.
    pub rows: Vec<Row>,
}

// ----------------------------------------------------------------------------
/// A booking of a statement.
#[derive(Debug, Default)]
pub(crate) struct Row {
    /// The day it was booked; `None` while it is pending.
    pub booked: Option<Date>,
    /// The bank's word for it ("Lastschrift / Belastung").
    pub vorgang: String,
    /// `card`, `direct-debit`, `transfer`, `cash` or `fee`.
    pub kind: Option<&'static str>,
    /// Who paid or was paid, as the bank names them.
    pub party: Option<String>,
    /// A card payment's merchant.
    pub merchant: Option<String>,
    pub purpose: Option<String>,
    /// The day the card was used.
    pub card_day: Option<Date>,
    pub reference: Option<String>,
    /// The other side's account and bank, on a transfer out.
    pub iban: Option<String>,
    pub bic: Option<String>,
    /// The account's own number, which a pending row names.
    pub own: Option<String>,
    pub cents: i64,
}

impl Row {
    // ------------------------------------------------------------------------
    /// Its day: when it was booked, or the card's while it is pending.
    fn day(&self) -> Option<Date> {
        self.booked.or(self.card_day)
    }
}

// ----------------------------------------------------------------------------
/// The statement in `bytes`, a CSV export of comdirect's ("Umsätze"):
/// UTF-8 or Windows-1252, quoted cells each followed by `;`, the balances
/// before and after the bookings, German dates and amounts. Or why it isn't
/// one.
pub(crate) fn read(bytes: &[u8]) -> Result<Statement, String> {
    let text = from_utf8_or_windows1252(bytes);
    let mut statement = Statement::default();
    let (mut header, mut in_rows) = (false, false);
    for (n, line) in text.lines().enumerate() {
        let cells = cells(line);
        if in_rows {
            if cells.is_empty() {
                in_rows = false;
            } else {
                let row = row(&cells).ok_or_else(|| format!("Line {} can't be read.", n + 1))?;
                statement.rows.push(row);
            }
            continue;
        }
        match cells.first().map(String::as_str) {
            Some(which @ ("Alter Kontostand" | "Neuer Kontostand")) => {
                let balance = cells.get(1).map(String::as_str).unwrap_or_default();
                let (amount, currency) = balance.split_once(' ').unwrap_or((balance, ""));
                let cents =
                    cents(amount).ok_or_else(|| format!("Line {} can't be read.", n + 1))?;
                if which == "Alter Kontostand" {
                    statement.opening = Some(cents);
                } else {
                    statement.closing = Some(cents);
                }
                if !currency.trim().is_empty() {
                    statement.currency = Some(currency.trim().to_string());
                }
            }
            _ if is_header(&cells) => {
                header = true;
                in_rows = true;
                if let Some(currency) = cells[4].strip_prefix("Umsatz in ") {
                    statement
                        .currency
                        .get_or_insert_with(|| currency.trim().to_string());
                }
            }
            _ => {}
        }
    }
    if header {
        Ok(statement)
    } else {
        Err("It isn't a statement this watch reads.".to_string())
    }
}

// ----------------------------------------------------------------------------
/// The cells of a line: each quoted (a `""` in it is a `"`) or not, each
/// followed by `;`.
fn cells(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut chars = line.chars().peekable();
    while chars.peek().is_some() {
        let mut cell = String::new();
        if chars.next_if_eq(&'"').is_some() {
            while let Some(c) = chars.next() {
                match c {
                    '"' if chars.next_if_eq(&'"').is_some() => cell.push('"'),
                    '"' => break,
                    c => cell.push(c),
                }
            }
            for c in chars.by_ref() {
                if c == ';' {
                    break;
                }
            }
        } else {
            for c in chars.by_ref() {
                if c == ';' {
                    break;
                }
                cell.push(c);
            }
        }
        cells.push(cell);
    }
    cells
}

// ----------------------------------------------------------------------------
fn is_header(cells: &[String]) -> bool {
    cells.len() >= 5
        && cells.iter().zip(HEADER).all(|(cell, name)| cell == name)
        && cells[4].starts_with("Umsatz")
}

// ----------------------------------------------------------------------------
/// The booking of a row's cells: its days, what it is, its text, its amount.
fn row(cells: &[String]) -> Option<Row> {
    let [booked, _value, vorgang, text, amount, ..] = cells else {
        return None;
    };
    let booked = match booked.trim() {
        "offen" => None,
        day => Some(german_day(day)?),
    };
    let mut row = Row {
        booked,
        vorgang: vorgang.trim().to_string(),
        cents: cents(amount)?,
        ..Row::default()
    };
    buchungstext(&mut row, text);
    row.kind = kind(&row);
    let purpose = row.purpose.take().unwrap_or_default();
    row.purpose = match row.kind {
        Some("transfer" | "direct-debit") => some(&joined(&purpose)),
        _ => some(&purpose),
    };
    Some(row)
}

// ----------------------------------------------------------------------------
/// `dd.mm.yyyy`.
fn german_day(text: &str) -> Option<Date> {
    let mut parts = text.split('.').map(|part| part.parse::<i32>().ok());
    let (day, month, year) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() {
        return None;
    }
    Date::from_ymd(year, month, day).ok()
}

// ----------------------------------------------------------------------------
/// A German amount in cents: `-1.927,35`, `100,00`.
fn cents(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, number) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (whole, fraction) = number.split_once(',').unwrap_or((number, ""));
    let whole: String = whole.chars().filter(|&c| c != '.').collect();
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || fraction.len() > 2 || !all_digits(&whole) || !all_digits(fraction) {
        return None;
    }
    let fraction: i64 = format!("{fraction:0<2}").parse().ok()?;
    let value = whole
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(fraction)?;
    Some(if negative { -value } else { value })
}

// ----------------------------------------------------------------------------
/// What a row's Buchungstext says: who (`Auftraggeber:`, `Empfänger:` with
/// their account, or the account's own number on a pending row), then after
/// `Buchungstext:` the purpose (a card payment's merchant line and the
/// card's day), then the reference (`Ref.`).
fn buchungstext(row: &mut Row, text: &str) {
    let (head, rest) = text.split_once("Buchungstext:").unwrap_or(("", text));
    let head = head.trim();
    if let Some(party) = head.strip_prefix("Auftraggeber:") {
        row.party = some(party);
    } else if let Some(other) = head.strip_prefix("Empfänger:") {
        let (party, account) = other.split_once("Kto/IBAN:").unwrap_or((other, ""));
        let (iban, bic) = account.split_once("BLZ/BIC:").unwrap_or((account, ""));
        (row.party, row.iban, row.bic) = (some(party), some(iban), some(bic));
    } else if let Some(own) = head.strip_prefix("Kto/IBAN:") {
        row.own = some(own);
    }
    // One space follows the colon; the purpose's own spaces count.
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let body = match rest.rsplit_once(" Ref. ") {
        Some((body, reference)) => {
            row.reference = some(reference);
            body
        }
        None => rest.trim_end(),
    };
    match card(body) {
        Some((line, day)) => {
            row.card_day = Some(day);
            row.merchant = line.find(['>', '/']).and_then(|end| some(&line[..end]));
            row.purpose = some(&line);
        }
        None => row.purpose = Some(body.to_string()),
    }
}

// ----------------------------------------------------------------------------
/// A card's text: its merchant line, then the time it was used
/// (`2026-12-01T07:12:40`) and, once booked, `KFN n VJ mmyy`, the card's
/// sequence and expiry. The line and the card's day.
fn card(body: &str) -> Option<(String, Date)> {
    let words: Vec<&str> = body.split_whitespace().collect();
    let at = words.iter().position(|word| card_day(word).is_some())?;
    let tail = &words[at + 1..];
    let booked = tail.len() >= 4 && tail[0] == "KFN" && tail[2] == "VJ";
    if !tail.is_empty() && !booked {
        return None;
    }
    Some((words[..at].join(" "), card_day(words[at])?))
}

// ----------------------------------------------------------------------------
fn card_day(word: &str) -> Option<Date> {
    if word.len() != 19 || word.as_bytes()[10] != b'T' {
        return None;
    }
    DateTime::from_iso8601(word).ok().map(|time| time.date)
}

// ----------------------------------------------------------------------------
fn kind(row: &Row) -> Option<&'static str> {
    match row.vorgang.as_str() {
        "Kartenverfügung" => Some("card"),
        "Lastschrift / Belastung" if row.card_day.is_some() => Some("card"),
        "Lastschrift / Belastung" => Some("direct-debit"),
        "Übertrag / Überweisung" => Some("transfer"),
        "Auszahlung GAA" => Some("cash"),
        "Kontoführungsentgelt" => Some("fee"),
        _ => None,
    }
}

// ----------------------------------------------------------------------------
/// `purpose` without the spaces the bank put between its lines.
fn joined(purpose: &str) -> String {
    let joint = |i: usize| i >= PURPOSE_LINE && (i - PURPOSE_LINE).is_multiple_of(PURPOSE_LINE + 1);
    purpose
        .chars()
        .enumerate()
        .filter(|&(i, c)| !(c == ' ' && joint(i)))
        .map(|(_, c)| c)
        .collect()
}

// ----------------------------------------------------------------------------
/// `text` trimmed, if anything is left.
fn some(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

// ----------------------------------------------------------------------------
fn to_cents(amount: f64) -> i64 {
    (amount * 100.0).round() as i64
}

// ----------------------------------------------------------------------------
/// A statement of the account in the vault, as an import sees it.
pub(crate) struct Known {
    pub id: Id,
    pub items: Vec<Transaction>,
    pub balance: Option<i64>,
}

// ----------------------------------------------------------------------------
fn is_pending(item: &Transaction) -> bool {
    item.status.as_deref() == Some("pending")
}

// ----------------------------------------------------------------------------
fn day_of(item: &Transaction) -> Option<Date> {
    item.date.map(|date| date.date)
}

// ----------------------------------------------------------------------------
/// What importing a statement adds to its account.
pub(crate) struct Plan {
    /// Its bookings, oldest first; its booked sum and its balance.
    pub transactions: Transactions,
    /// The days of its first and last bookings.
    pub first: Option<Date>,
    pub last: Option<Date>,
    /// Whether it is the account's newest statement: its pending bookings
    /// are the account's, its balance the account's.
    pub newest: bool,
    /// The older statements whose pending bookings it replaces, each with
    /// what is left of its bookings.
    pub replaced: Vec<(Id, Vec<Transaction>)>,
    /// The bookings the account has already.
    pub skipped: usize,
    /// What doesn't add up.
    pub problems: Vec<String>,
}

// ----------------------------------------------------------------------------
/// What `statement` adds to the account whose statements are `known`, the
/// account's `owner` on its side of each booking: a booking the account has
/// (its reference, or without one its day, amount and purpose) is left out;
/// as the newest statement it replaces the account's pending bookings, a new
/// booking of a replaced one's card day and amount taking its id and link,
/// and as an older one it brings none; its balances are checked against its
/// bookings and the statement before it.
pub(crate) fn plan(owner: Option<&ContactRef>, known: &[Known], statement: &Statement) -> Plan {
    let all_items = || known.iter().flat_map(|s| s.items.iter());
    let refs: HashSet<&str> = all_items()
        .filter(|item| !is_pending(item))
        .filter_map(|item| item.reference.as_deref())
        .collect();
    let unreferenced: HashSet<(Date, i64, &str)> = all_items()
        .filter(|item| !is_pending(item) && item.reference.is_none())
        .filter_map(|item| {
            let purpose = item.purpose.as_deref().unwrap_or_default();
            Some((day_of(item)?, to_cents(item.amount?), purpose))
        })
        .collect();
    let last_known = all_items().filter_map(day_of).max();
    let last = statement.rows.iter().filter_map(Row::day).max();
    let newest = match (last, last_known) {
        (Some(last), Some(known)) => last >= known,
        (_, None) => true,
        (None, Some(_)) => false,
    };

    let mut skipped = 0;
    let mut rows = Vec::new();
    for row in statement.rows.iter().rev() {
        let known_already = match (row.booked, &row.reference) {
            (Some(_), Some(reference)) => refs.contains(reference.as_str()),
            (Some(day), None) => {
                let purpose = row.purpose.as_deref().unwrap_or_default();
                unreferenced.contains(&(day, row.cents, purpose))
            }
            (None, _) => !newest,
        };
        if known_already {
            skipped += 1;
        } else {
            rows.push((row, transaction(row, owner), false));
        }
    }

    let mut replaced = Vec::new();
    if newest {
        for older in known.iter().filter(|s| s.items.iter().any(is_pending)) {
            let (pending, kept): (Vec<_>, Vec<_>) =
                older.items.iter().cloned().partition(is_pending);
            for item in pending {
                let (day, cents) = (day_of(&item), item.amount.map(to_cents));
                let twin = rows.iter_mut().find(|(row, _, taken)| {
                    !taken
                        && row.card_day.is_some()
                        && row.card_day == day
                        && Some(row.cents) == cents
                });
                if let Some((_, booking, taken)) = twin {
                    (booking.id, booking.linked_to, *taken) = (item.id, item.linked_to, true);
                }
            }
            replaced.push((older.id, kept));
        }
    }

    let mut problems = Vec::new();
    let booked_sum: i64 = statement
        .rows
        .iter()
        .filter(|r| r.booked.is_some())
        .map(|r| r.cents)
        .sum();
    if let (Some(opening), Some(closing)) = (statement.opening, statement.closing) {
        if opening + booked_sum != closing {
            problems.push("its bookings don't add up to its balances".to_string());
        }
    }
    let first_booked = statement.rows.iter().filter_map(|r| r.booked).min();
    if let (Some(first), Some(opening)) = (first_booked, statement.opening) {
        let ends = |s: &Known| {
            s.items
                .iter()
                .filter(|i| !is_pending(i))
                .filter_map(day_of)
                .max()
        };
        let before = known
            .iter()
            .filter_map(|s| Some((ends(s)?, s)))
            .filter(|(end, _)| *end < first)
            .max_by_key(|(end, _)| *end);
        if let Some((end, s)) = before {
            if s.balance.is_some_and(|balance| balance != opening) {
                let (end, first) = (day_text(end), day_text(first));
                problems.push(format!("bookings are missing between {end} and {first}"));
            }
        }
    }

    let written: i64 = rows
        .iter()
        .filter(|(r, ..)| r.booked.is_some())
        .map(|(r, ..)| r.cents)
        .sum();
    let transactions = Transactions {
        account: None,
        amount: Some(written as f64 / 100.0),
        balance: statement.closing.map(|cents| cents as f64 / 100.0),
        transactions: rows.into_iter().map(|(_, booking, _)| booking).collect(),
    };
    Plan {
        first: statement.rows.iter().filter_map(Row::day).min(),
        last,
        transactions,
        newest,
        replaced,
        skipped,
        problems,
    }
}

// ----------------------------------------------------------------------------
/// Another bank account of the vault, as pairing transfers sees it.
pub(crate) struct OtherAccount {
    pub number: Option<String>,
    pub statements: Vec<Known>,
}

// ----------------------------------------------------------------------------
/// A transfer paired: a new booking, by its index, and its other half, a
/// booking of another account's statement.
#[derive(Debug, PartialEq)]
pub(crate) struct Pair {
    pub booking: usize,
    pub statement: Id,
    pub other: Id,
}

// ----------------------------------------------------------------------------
/// The transfers between `bookings`, new bookings of the account numbered
/// `number`, and the bookings of `others`, the vault's other accounts: money
/// out whose `iban` names the account it goes to (`same_number`), and the
/// same amount into that account, at most `PAIR_DAYS` apart, the nearest
/// first. Only booked bookings without a pair pair, each once.
pub(crate) fn pairs(
    number: Option<&str>,
    bookings: &[Transaction],
    others: &[OtherAccount],
) -> Vec<Pair> {
    let names = |iban: Option<&str>, number: Option<&str>| {
        iban.zip(number)
            .is_some_and(|(iban, number)| same_number(iban, number))
    };
    let open = |item: &Transaction| !is_pending(item) && item.transfer.is_none();
    let mut pairs: Vec<Pair> = Vec::new();
    for (i, booking) in bookings.iter().enumerate() {
        let (Some(day), Some(cents)) = (day_of(booking), booking.amount.map(to_cents)) else {
            continue;
        };
        if cents == 0 || !open(booking) {
            continue;
        }
        let apart = |item: &Transaction| day_of(item).map(|d| (d.days() - day.days()).abs());
        let half = |other: &OtherAccount, item: &Transaction| {
            let named = if cents < 0 {
                names(booking.iban.as_deref(), other.number.as_deref())
            } else {
                names(item.iban.as_deref(), number)
            };
            named
                && open(item)
                && item.amount.map(to_cents) == Some(-cents)
                && item
                    .id
                    .is_some_and(|id| !pairs.iter().any(|pair| pair.other == id))
                && apart(item).is_some_and(|days| days <= PAIR_DAYS)
        };
        let nearest = others
            .iter()
            .flat_map(|other| {
                let items = other.statements.iter();
                items.flat_map(move |s| s.items.iter().map(move |item| (other, s.id, item)))
            })
            .filter(|(other, _, item)| half(other, item))
            .min_by_key(|(_, _, item)| apart(item));
        if let Some((_, statement, Some(other))) = nearest.map(|(o, s, item)| (o, s, item.id)) {
            pairs.push(Pair {
                booking: i,
                statement,
                other,
            });
        }
    }
    pairs
}

// ----------------------------------------------------------------------------
/// The booking of `row`, the account's `owner` on its side.
fn transaction(row: &Row, owner: Option<&ContactRef>) -> Transaction {
    let other = row.party.clone().or_else(|| row.merchant.clone());
    let title = other.clone().unwrap_or_else(|| row.vorgang.clone());
    let other = other.map(ContactRef::Name);
    let (sender, receiver) = if row.cents < 0 {
        (owner.cloned(), other)
    } else {
        (other, owner.cloned())
    };
    let status = if row.booked.is_some() {
        "booked"
    } else {
        "pending"
    };
    Transaction {
        id: Some(Id::generate()),
        title: Some(title),
        date: row.day().map(|date| DateTime { date, time: None }),
        status: Some(status.to_string()),
        amount: Some(row.cents as f64 / 100.0),
        sender,
        receiver,
        purpose: row.purpose.clone(),
        kind: row.kind.map(str::to_string),
        reference: row.reference.clone(),
        iban: row.iban.clone(),
        bic: row.bic.clone(),
        ..Transaction::default()
    }
}

// ----------------------------------------------------------------------------
/// "1 Dec 2026".
fn day_text(date: Date) -> String {
    let (year, month, day) = date.to_ymd();
    let month: i32 = month.into();
    format!("{day} {} {year}", &MONTHS[month as usize - 1][..3])
}

// ----------------------------------------------------------------------------
/// The months from `first` to `last`: "December 2026", "Nov – Dec 2026",
/// "Dec 2026 – Jan 2027".
fn period(first: Date, last: Date) -> String {
    let (y1, m1, _) = first.to_ymd();
    let (y2, m2, _) = last.to_ymd();
    let (m1, m2): (i32, i32) = (m1.into(), m2.into());
    let name = |m: i32| MONTHS[m as usize - 1];
    let abbr = |m: i32| &MONTHS[m as usize - 1][..3];
    match (y1 == y2, m1 == m2) {
        (true, true) => format!("{} {y1}", name(m1)),
        (true, false) => format!("{} \u{2013} {} {y1}", abbr(m1), abbr(m2)),
        _ => format!("{} {y1} \u{2013} {} {y2}", abbr(m1), abbr(m2)),
    }
}

// ----------------------------------------------------------------------------
/// The bank account of a statements watch, as an import uses it.
struct Account {
    id: Id,
    title: String,
    number: Option<String>,
    owner: Option<ContactRef>,
}

// ----------------------------------------------------------------------------
/// The account the watch of `settings` imports into; or why there is none.
fn account_of(app: &App, settings: &WatchSettings) -> Result<Account, String> {
    let gone = || "The watch's bank account isn't in the vault.".to_string();
    let id = settings.account.ok_or_else(gone)?;
    let doc = app.vault.index().get(&id).ok_or_else(gone)?;
    let account = doc.roles().bank_account.as_ref().ok_or_else(gone)?;
    Ok(Account {
        id,
        title: doc.title().to_string(),
        number: account.number.clone(),
        owner: account.owner.clone(),
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
    let statement = |doc: &Document| {
        let transactions = doc.roles().transactions.as_ref()?;
        (transactions.account == Some(account)).then(|| Known {
            id: doc.id(),
            items: transactions.transactions.clone(),
            balance: transactions.balance.map(to_cents),
        })
    };
    let index = app.vault.index();
    index
        .all()
        .iter()
        .filter_map(|entry| statement(&entry.doc))
        .collect()
}

// ----------------------------------------------------------------------------
/// The files the statements of the account `account` came from: a file is
/// in the vault by its account and origin, whichever watch imported it.
fn account_origins(app: &App, account: Id) -> HashSet<String> {
    let origin = |doc: &Document| {
        let transactions = doc.roles().transactions.as_ref()?;
        if transactions.account != Some(account) {
            return None;
        }
        match doc.source()? {
            Source::Import(import) => import.origin.clone(),
            _ => None,
        }
    };
    let index = app.vault.index();
    index
        .all()
        .iter()
        .filter_map(|entry| origin(&entry.doc))
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
/// Pair the transfers among `bookings`, new bookings of `account`, with the
/// vault's other accounts (`pairs`): each new half names its other half.
/// The other halves to write: each its statement, its id and the new half's.
fn pair_transfers(app: &App, account: &Account, bookings: &mut [Transaction]) -> Vec<(Id, Id, Id)> {
    let found = pairs(
        account.number.as_deref(),
        bookings,
        &others_of(app, account.id),
    );
    let half = |pair: Pair| {
        let booking = &mut bookings[pair.booking];
        booking.transfer = Some(pair.other);
        Some((pair.statement, pair.other, booking.id?))
    };
    found.into_iter().filter_map(half).collect()
}

// ----------------------------------------------------------------------------
/// Write the other halves of new transfers (`pair_transfers`), each
/// statement once, each half naming its new one; how many were written.
fn write_halves(app: &mut App, halves: &[(Id, Id, Id)]) -> usize {
    let mut statements: Vec<Id> = Vec::new();
    for (statement, ..) in halves {
        if !statements.contains(statement) {
            statements.push(*statement);
        }
    }
    let mut written = 0;
    for statement in statements {
        let own: Vec<(Id, Id)> = halves
            .iter()
            .filter(|(s, ..)| *s == statement)
            .map(|(_, other, new)| (*other, *new))
            .collect();
        let outcome = app.edit(|index, changes| {
            index.edit(&statement, changes, |d| {
                if let Some(mut transactions) = d.roles().transactions.clone() {
                    for item in &mut transactions.transactions {
                        let new = own.iter().find(|(other, _)| item.id == Some(*other));
                        if let Some((_, new)) = new {
                            item.transfer = Some(*new);
                        }
                    }
                    d.set_transactions(Some(transactions));
                }
            })
        });
        match outcome.and_then(|outcome| app.vault.require_written(outcome)) {
            Ok(_) => written += own.len(),
            Err(e) => {
                log::warn!("Statement import: could not pair the transfers of {statement}: {e}")
            }
        }
    }
    written
}

// ----------------------------------------------------------------------------
/// The own account number of a pending row of `statement` that isn't
/// `number` (`same_number`).
fn other_account<'a>(statement: &'a Statement, number: Option<&str>) -> Option<&'a str> {
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
/// number and kind, which it needs, and its bank.
#[derive(Deserialize)]
pub(crate) struct NewAccount {
    #[serde(default)]
    title: String,
    #[serde(default)]
    number: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    institution: String,
}

// ----------------------------------------------------------------------------
/// A new bank account in `Accounts/` with the fields of `new`; its id, or
/// why there is none, with what to tell the page. Nothing is written
/// without its name, number and kind.
pub(crate) fn new_account(app: &mut App, new: NewAccount) -> Result<Id, (Error, String)> {
    let invalid = |arg: &str, message: &str| {
        let arg = arg.to_string();
        (Error::InvalidArgument { arg }, message.to_string())
    };
    let Some(title) = some(&new.title) else {
        return Err(invalid("title", "Enter the account's name."));
    };
    let Some(number) = some(&new.number) else {
        return Err(invalid("number", "Enter the account's number."));
    };
    if !ACCOUNT_KINDS.contains(&new.kind.as_str()) {
        return Err(invalid("type", "Choose the account's kind."));
    }
    let account = BankAccount {
        account_type: Some(new.kind),
        number: Some(number),
        institution: some(&new.institution),
        ..BankAccount::default()
    };
    let roles = Roles {
        bank_account: Some(account),
        ..Roles::default()
    };
    let source = Source::Created(Empty {});
    let doc = Document::create(roles, source, title, Vec::new(), FileFormat::YAML);
    let created =
        app.edit(|index, changes| index.create(doc, Path::new(layout::BANK_ACCOUNTS), changes));
    created.map_err(|e| {
        log::warn!("Could not write the new bank account: {e}");
        (e, "Could not write the new bank account.".to_string())
    })
}

// ----------------------------------------------------------------------------
/// `set_bank_account`'s args: the account, and the fields that change.
#[derive(Deserialize)]
pub(crate) struct AccountChange {
    entity: Id,
    institution: Option<String>,
    number: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

// ----------------------------------------------------------------------------
/// `set_bank_account {entity, institution?, number?, type?}`: change the
/// bank account's fields given, an empty one removing its key; `type` is
/// `checking`, `savings` or `credit-card`. Answered as `set_title` is.
pub fn set_bank_account(app: &mut App, req: Req, change: AccountChange) {
    let id = change.entity;
    if !is_account(app, id) {
        return req.err(Error::NotFound);
    }
    if let Some(kind) = change.kind.as_deref() {
        if !ACCOUNT_KINDS.contains(&kind) {
            return req.err(Error::InvalidArgument { arg: "type".into() });
        }
    }
    let result = app.edit(|index, changes| {
        index.edit(&id, changes, |d| {
            let mut account = d.roles().bank_account.clone().unwrap_or_default();
            if let Some(institution) = change.institution {
                account.institution = some(&institution);
            }
            if let Some(number) = change.number {
                account.number = some(&number);
            }
            if let Some(kind) = change.kind {
                account.account_type = Some(kind);
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
    statement: Result<Statement, String>,
}

// ----------------------------------------------------------------------------
/// What an import wrote, for its job's note.
#[derive(Default)]
struct Counts {
    statements: usize,
    booked: usize,
    pending: usize,
    transfers: usize,
    skipped: usize,
}

// ----------------------------------------------------------------------------
/// `statements_import {watch}`: import the new files of a statements watch
/// into its account as a job, "Importing statements · <account>": listed and
/// read on a worker, then written oldest first (`plan`). Answers `{job}`;
/// the job's note counts what came in and names each file left out and each
/// statement whose balances don't add up.
pub fn statements_import(app: &mut App, req: Req, watch: Id) {
    let settings = match app.watches.get(watch) {
        Ok(found) => found.settings().clone(),
        Err(e) => return req.err(e),
    };
    if settings.kind != WatchKind::Statements {
        let what = "importing statements from this watch".to_string();
        let message = "Only a statements watch imports statements.";
        return req.fail(Error::Unsupported { what }, message);
    }
    if settings.folder.is_empty() {
        let arg = "folder".to_string();
        let message = "Choose the folder of the account's statements first.";
        return req.fail(Error::InvalidArgument { arg }, message);
    }
    let account = match account_of(app, &settings) {
        Ok(account) => account,
        Err(message) => return req.fail(Error::NotFound, message),
    };
    let in_vault = account_origins(app, account.id);
    let label = format!("Importing statements \u{b7} {}", account.title);
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
            let note = match read {
                _ if app.job_stopping(job) => "Stopped".to_string(),
                Ok(files) => import_files(app, watch, files),
                Err(Error::FileNotFound { path }) => {
                    format!("There's no folder at {}.", path.display())
                }
                Err(e) => {
                    log::warn!("Could not list the folder of the watch {watch}: {e}");
                    "Could not list the folder.".to_string()
                }
            };
            log::info!("Statement import {job}: {note}");
            app.end_job(job, &note);
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
            Err("It can't be read.".to_string())
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
/// oldest first, but a file whose bookings the account has all; the job's
/// note.
fn import_files(app: &mut App, watch: Id, mut files: Vec<NewFile>) -> String {
    let settings = match app.watches.get(watch) {
        Ok(found) => found.settings().clone(),
        Err(_) => return "The watch was removed.".to_string(),
    };
    let account = match account_of(app, &settings) {
        Ok(account) => account,
        Err(message) => return message,
    };
    let last_day = |file: &NewFile| {
        let statement = file.statement.as_ref().ok()?;
        statement.rows.iter().filter_map(Row::day).max()
    };
    files.sort_unstable_by_key(last_day);
    let mut counts = Counts::default();
    let mut problems = Vec::new();
    for file in &files {
        let named = |problem: &str| format!("{}: {problem}", file.name);
        if account_origins(app, account.id).contains(&file.name) {
            continue;
        }
        let statement = match &file.statement {
            Ok(statement) => statement,
            Err(why) => {
                problems.push(named(why));
                continue;
            }
        };
        if let Some(other) = other_account(statement, account.number.as_deref()) {
            problems.push(named(&format!("the statement of another account, {other}")));
            continue;
        }
        let mut plan = plan(account.owner.as_ref(), &known(app, account.id), statement);
        problems.extend(plan.problems.iter().map(|problem| named(problem)));
        if plan.transactions.transactions.is_empty() {
            counts.skipped += plan.skipped;
            continue;
        }
        let halves = if settings.pair_transfers {
            pair_transfers(app, &account, &mut plan.transactions.transactions)
        } else {
            Vec::new()
        };
        match write_statement(app, watch, &account, file, statement, plan, &mut counts) {
            Ok(()) => {
                counts.statements += 1;
                counts.transfers += write_halves(app, &halves);
            }
            Err(why) => problems.push(named(&why)),
        }
    }
    let mut note = count_summary(
        &[
            (counts.statements, "statement", "statements"),
            (counts.booked, "booking", "bookings"),
            (counts.pending, "pending", "pending"),
            (counts.transfers, "transfer paired", "transfers paired"),
            (
                counts.skipped,
                "in the vault already",
                "in the vault already",
            ),
        ],
        "No new statements",
    );
    for problem in problems {
        note.push_str(" \u{b7} ");
        note.push_str(&problem);
    }
    note
}

// ----------------------------------------------------------------------------
/// Write the statement of `file` as `plan` says: the new statement, then
/// the older ones without the pending bookings it replaces, then the
/// account's balance (from its newest statement) and its currency (if it
/// has none). Only the new statement's write failing fails it.
fn write_statement(
    app: &mut App,
    watch: Id,
    account: &Account,
    file: &NewFile,
    statement: &Statement,
    plan: Plan,
    counts: &mut Counts,
) -> Result<(), String> {
    let first = plan.first.unwrap_or(file.taken.date);
    let period = period(first, plan.last.unwrap_or(first));
    let mut transactions = plan.transactions;
    transactions.account = Some(account.id);
    let pending = transactions
        .transactions
        .iter()
        .filter(|t| is_pending(t))
        .count();
    let booked = transactions.transactions.len() - pending;
    let roles = Roles {
        transactions: Some(transactions),
        ..Roles::default()
    };
    let source = Source::Import(Import {
        watch: Some(watch),
        origin: Some(file.name.clone()),
    });
    let title = format!("{}, {period}", account.title);
    let mut doc = Document::create(roles, source, title, Vec::new(), FileFormat::YAML);
    doc.set_date(DateTime {
        date: first,
        time: Time::new(0).ok(),
    });
    let folder = Path::new(layout::BANK_ACCOUNTS);
    app.edit(|index, changes| index.create(doc, folder, changes))
        .map_err(|e| {
            log::warn!("Statement import: could not write {}: {e}", file.name);
            "Could not write the statement.".to_string()
        })?;
    (counts.booked, counts.pending) = (counts.booked + booked, counts.pending + pending);
    counts.skipped += plan.skipped;

    for (id, items) in plan.replaced {
        let outcome = app.edit(|index, changes| {
            index.edit(&id, changes, |d| {
                if let Some(mut older) = d.roles().transactions.clone() {
                    older.transactions = items;
                    d.set_transactions(Some(older));
                }
            })
        });
        if let Err(e) = outcome.and_then(|outcome| app.vault.require_written(outcome)) {
            log::warn!("Statement import: could not take the pending bookings out of {id}: {e}");
        }
    }
    let balance = statement.closing.filter(|_| plan.newest);
    let currency = statement.currency.clone();
    let outcome = app.edit(|index, changes| {
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
    if let Err(e) = outcome.and_then(|outcome| app.vault.require_written(outcome)) {
        log::warn!(
            "Statement import: could not write the balance of {}: {e}",
            account.id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, shape};
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;

    /// The forged vault's December statement, and what it was written from.
    const DECEMBER: &[u8] = include_bytes!("../../data/statements/statement_2026-12.csv");
    const DECEMBER_JSON: &str = include_str!("../../data/statements/statement_2026-12.json");
    const ACCOUNT: &str = "01955b3b-ca80-7fef-afbe-46d15556d9a1";
    const WATCH: &str = "01a0e1cc-a24c-7490-82fe-e75669985b60";

    fn day(text: &str) -> Date {
        DateTime::from_iso8601(text).unwrap().date
    }

    fn december() -> Statement {
        read(DECEMBER).unwrap()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_statement_has_its_balances_and_its_rows() {
        let statement = december();
        assert_eq!(statement.currency.as_deref(), Some("EUR"));
        assert_eq!(
            (statement.opening, statement.closing),
            (Some(3_205_928), Some(3_247_422))
        );
        assert_eq!(statement.rows.len(), 30);
        let pending = statement.rows.iter().filter(|r| r.booked.is_none()).count();
        assert_eq!(pending, 2);
        let booked: i64 = statement
            .rows
            .iter()
            .filter(|r| r.booked.is_some())
            .map(|r| r.cents)
            .sum();
        assert_eq!(3_205_928 + booked, 3_247_422);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn rows_say_who_what_and_why() {
        let statement = december();
        let row = |cents: i64| statement.rows.iter().find(|r| r.cents == cents).unwrap();

        let donuts = &statement.rows[0];
        assert_eq!(donuts.booked, None);
        assert_eq!(donuts.own.as_deref(), Some("004273185520"));
        assert_eq!(donuts.merchant.as_deref(), Some("Big Dough Donuts"));
        assert_eq!(donuts.card_day, Some(day("2026-12-31")));
        assert_eq!(
            (donuts.kind, donuts.reference.as_deref()),
            (Some("card"), None)
        );

        let critters = row(-9500);
        assert_eq!(critters.booked, Some(day("2026-12-16")));
        assert_eq!(critters.party.as_deref(), Some("Critter Busters LLC"));
        assert_eq!(critters.merchant.as_deref(), Some("Critter Busters"));
        assert_eq!(
            critters.purpose.as_deref(),
            Some("Critter Busters//Springfield/US")
        );
        assert_eq!(critters.card_day, Some(day("2026-12-15")));
        assert_eq!(
            critters.reference.as_deref(),
            Some("LVDQ2A4MX70PBBTP/360902")
        );

        let house = row(-50000);
        assert_eq!(house.kind, Some("transfer"));
        assert_eq!(house.party.as_deref(), Some("Alexander and Maya Yahnek"));
        assert_eq!(
            (house.iban.as_deref(), house.bic.as_deref()),
            (Some("004273185539"), Some("FBSPUS44XXX"))
        );
        assert_eq!(house.purpose.as_deref(), Some("House fund, starting over"));

        let fee = row(-190);
        assert_eq!((fee.kind, fee.party.as_deref()), (Some("fee"), None));
        let purpose = "Entgelt Visa-Kreditkarte Zeitraum: 01.11.2026 bis 30.11.2026";
        assert_eq!(fee.purpose.as_deref(), Some(purpose));
        assert_eq!(fee.booked, Some(day("2026-12-02")));

        assert_eq!(row(-20000).kind, Some("cash"));
        assert_eq!(row(-192_735).kind, Some("direct-debit"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn purposes_lose_the_banks_line_breaks() {
        let statement = december();
        let json: Value = serde_json::from_str(DECEMBER_JSON).unwrap();
        let mut checked = 0;
        for tx in json["transactions"].as_array().unwrap() {
            let kind = tx["kind"].as_str().unwrap();
            let Some(purpose) = tx["purpose"].as_str() else {
                continue;
            };
            if !["direct-debit", "transfer-in", "transfer-out"].contains(&kind) {
                continue;
            }
            let cents = to_cents(tx["amount"].as_f64().unwrap());
            let row = |r: &&Row| r.cents == cents && r.purpose.as_deref() == Some(purpose);
            assert!(statement.rows.iter().any(|r| row(&r)), "{kind}: {purpose}");
            checked += 1;
        }
        assert_eq!(checked, 12);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn cells_amounts_and_days_read_as_the_bank_writes_them() {
        assert_eq!(
            cells(r#""a";"say ""hi""";plain;"";"#),
            ["a", r#"say "hi""#, "plain", ""]
        );
        assert!(cells("").is_empty());
        assert_eq!(cents("-1.927,35"), Some(-192_735));
        assert_eq!(cents("100,00"), Some(10_000));
        assert_eq!(cents("1,5"), Some(150));
        assert_eq!(cents("12"), Some(1_200));
        assert_eq!(cents("1,234"), None);
        assert_eq!(cents("EUR"), None);
        assert_eq!(german_day("31.12.2026"), Some(day("2026-12-31")));
        assert_eq!(german_day("31.02.2026"), None);
        assert_eq!(german_day("--"), None);
        assert_eq!(joined("x".repeat(35).as_str()), "x".repeat(35));
        let cut = format!("{} {}", "a".repeat(35), "b");
        assert_eq!(joined(&cut), format!("{}b", "a".repeat(35)));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn other_files_arent_statements() {
        let not = read(b"Name;Amount\nRent;-1150\n").unwrap_err();
        assert_eq!(not, "It isn't a statement this watch reads.");
        let head =
            r#""Buchungstag";"Wertstellung (Valuta)";"Vorgang";"Buchungstext";"Umsatz in EUR";"#;
        let broken = format!("{head}\n\"01.13.2026\";\"--\";\"x\";\"y\";\"1,00\";\n");
        assert_eq!(
            read(broken.as_bytes()).unwrap_err(),
            "Line 2 can't be read."
        );
    }

    // ------------------------------------------------------------------------
    /// A booked row of `cents` on `booked`, with `reference`.
    fn booked(booked: &str, cents: i64, reference: &str) -> Row {
        Row {
            booked: Some(day(booked)),
            vorgang: "Lastschrift / Belastung".into(),
            reference: some(reference),
            purpose: Some(format!("purpose {reference}")),
            cents,
            ..Row::default()
        }
    }

    /// A pending card payment of `cents`, the card used on `card`.
    fn pending(card: &str, cents: i64) -> Row {
        Row {
            vorgang: "Kartenverfügung".into(),
            card_day: Some(day(card)),
            cents,
            ..Row::default()
        }
    }

    /// `plan` as a known statement of the account.
    fn known_of(plan: Plan) -> Known {
        Known {
            id: Id::generate(),
            balance: plan.transactions.balance.map(to_cents),
            items: plan.transactions.transactions,
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_first_statement_brings_every_booking() {
        let owner = ContactRef::Name("Alexander Yahnek".into());
        let plan = plan(Some(&owner), &[], &december());
        assert!(plan.newest);
        assert_eq!((plan.skipped, plan.problems.len()), (0, 0));
        assert_eq!(plan.transactions.transactions.len(), 30);
        assert_eq!(plan.transactions.amount, Some(414.94));
        assert_eq!(plan.transactions.balance, Some(32474.22));
        assert_eq!(
            (plan.first, plan.last),
            (Some(day("2026-12-01")), Some(day("2026-12-31")))
        );
        let items = &plan.transactions.transactions;
        let first = &items[0];
        assert_eq!(first.title.as_deref(), Some("First Bank of Springfield"));
        assert_eq!(first.sender, Some(owner.clone()), "money out");
        assert_eq!(first.amount, Some(-1927.35));
        let pay = items.iter().find(|t| t.amount == Some(794.88)).unwrap();
        let party = ContactRef::Name("Springfield Power Station".into());
        assert_eq!(
            (pay.sender.as_ref(), pay.receiver.as_ref()),
            (Some(&party), Some(&owner))
        );
        let last = items.last().unwrap();
        assert_eq!(last.status.as_deref(), Some("pending"), "pending ones last");
        assert_eq!(last.title.as_deref(), Some("Big Dough Donuts"));
        assert_eq!(
            last.date,
            Some(DateTime {
                date: day("2026-12-31"),
                time: None
            })
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_booking_is_imported_once() {
        let first = plan(None, &[], &december());
        let known = [known_of(first)];
        let again = plan(None, &known, &december());
        assert_eq!(again.skipped, 28, "the booked ones");
        assert_eq!(again.transactions.transactions.len(), 2, "the pending ones");
        assert_eq!(again.transactions.amount, Some(0.0));
        let unreferenced = Statement {
            rows: vec![booked("2026-12-02", -500, "")],
            ..Statement::default()
        };
        let known = [known_of(plan(None, &[], &unreferenced))];
        let again = plan(None, &known, &unreferenced);
        assert_eq!(again.skipped, 1, "the same day, amount and purpose");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_newest_statement_replaces_the_pending_bookings() {
        let older = Statement {
            rows: vec![
                pending("2026-12-30", -5995),
                booked("2026-12-28", -1599, "R1"),
            ],
            ..Statement::default()
        };
        let mut first = known_of(plan(None, &[], &older));
        let bill = Id::generate();
        first.items[1].linked_to = Some(bill);
        let pending_id = first.items[1].id;
        let first_id = first.id;

        let mut paid = booked("2027-01-02", -5995, "R2");
        paid.card_day = Some(day("2026-12-30"));
        let newer = Statement {
            rows: vec![
                pending("2027-01-03", -700),
                paid,
                booked("2026-12-28", -1599, "R1"),
            ],
            ..Statement::default()
        };
        let known = [first];
        let next = plan(None, &known, &newer);
        assert!(next.newest);
        assert_eq!(next.skipped, 1, "R1 is there");
        let twin = next
            .transactions
            .transactions
            .iter()
            .find(|t| t.reference.as_deref() == Some("R2"));
        let twin = twin.unwrap();
        assert_eq!(
            (twin.id, twin.linked_to),
            (pending_id, Some(bill)),
            "the pending one, booked"
        );
        assert_eq!(next.replaced.len(), 1);
        let (id, kept) = &next.replaced[0];
        assert_eq!(*id, first_id);
        assert_eq!(kept.len(), 1, "only the booked one stays");
        assert!(kept.iter().all(|t| !is_pending(t)));

        let late = plan(None, &[known_of(next)], &older);
        assert!(!late.newest);
        assert!(late.replaced.is_empty());
        assert!(
            late.transactions
                .transactions
                .iter()
                .all(|t| !is_pending(t))
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn balances_that_dont_add_up_are_named() {
        let november = Statement {
            opening: Some(10_000),
            closing: Some(9_000),
            rows: vec![booked("2026-11-20", -1_000, "N1")],
            ..Statement::default()
        };
        let known = [known_of(plan(None, &[], &november))];
        let december = Statement {
            opening: Some(8_000),
            closing: Some(7_500),
            rows: vec![booked("2026-12-03", -1_000, "D1")],
            ..Statement::default()
        };
        let next = plan(None, &known, &december);
        assert_eq!(
            next.problems,
            [
                "its bookings don't add up to its balances",
                "bookings are missing between 20 Nov 2026 and 3 Dec 2026",
            ]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_statement_is_named_by_its_months() {
        let (nov, dec, jan) = (day("2026-11-15"), day("2026-12-14"), day("2027-01-02"));
        assert_eq!(period(dec, dec), "December 2026");
        assert_eq!(period(nov, dec), "Nov \u{2013} Dec 2026");
        assert_eq!(period(dec, jan), "Dec 2026 \u{2013} Jan 2027");
        assert_eq!(day_text(jan), "2 Jan 2027");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn another_accounts_statement_is_told_by_its_number() {
        let statement = december();
        assert_eq!(other_account(&statement, Some("0042-7318-5520")), None);
        assert_eq!(
            other_account(&statement, Some("DE12 5001 0517 0042 7318 5520")),
            None
        );
        assert_eq!(other_account(&statement, None), None);
        assert_eq!(
            other_account(&statement, Some("0042-7318-5539")),
            Some("004273185520")
        );
    }

    // ------------------------------------------------------------------------
    /// A booked booking of `cents` on `day`, to `iban` if given.
    fn half(day: &str, cents: i64, iban: Option<&str>) -> Transaction {
        Transaction {
            id: Some(Id::generate()),
            date: Some(DateTime::from_iso8601(day).unwrap()),
            status: Some("booked".into()),
            amount: Some(cents as f64 / 100.0),
            iban: iban.map(str::to_string),
            ..Transaction::default()
        }
    }

    /// An account numbered `number`, with a statement of `items`.
    fn other(number: &str, items: Vec<Transaction>) -> OtherAccount {
        let statement = Known {
            id: Id::generate(),
            items,
            balance: None,
        };
        OtherAccount {
            number: Some(number.into()),
            statements: vec![statement],
        }
    }

    const SAVINGS_IBAN: Option<&str> = Some("DE12 5001 0517 0042 7318 5539");

    // ------------------------------------------------------------------------
    #[test]
    fn a_transfer_pairs_with_the_account_its_iban_names() {
        let nearest = half("2026-12-12", 50_000, None);
        let savings = other(
            "0042-7318-5539",
            vec![
                half("2026-12-14", 50_000, None),
                nearest.clone(),
                half("2026-12-12", 49_900, None),
            ],
        );
        let unnamed = other("0042-7318-5547", vec![half("2026-12-11", 50_000, None)]);
        let others = [unnamed, savings];
        let out = [half("2026-12-11", -50_000, SAVINGS_IBAN)];
        let pair = Pair {
            booking: 0,
            statement: others[1].statements[0].id,
            other: nearest.id.unwrap(),
        };
        assert_eq!(pairs(Some("0042-7318-5520"), &out, &others), [pair]);

        let sent = half("2026-12-11", -50_000, SAVINGS_IBAN);
        let checking = [other("0042-7318-5520", vec![sent.clone()])];
        let into = [half("2026-12-12", 50_000, None)];
        let found = pairs(Some("0042-7318-5539"), &into, &checking);
        assert_eq!(found.len(), 1, "money in, from a booking naming it");
        assert_eq!(found[0].other, sent.id.unwrap());
        assert!(pairs(Some("0042-7318-5547"), &into, &checking).is_empty());
        assert!(pairs(None, &into, &checking).is_empty(), "no number");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_transfer_pairs_once_within_its_days() {
        let savings = |items: Vec<Transaction>| [other("0042-7318-5539", items)];
        let none = |bookings: &[Transaction], items: Vec<Transaction>| {
            pairs(None, bookings, &savings(items)).is_empty()
        };
        let into = |day: &str| half(day, 50_000, None);
        let out = [half("2026-12-11", -50_000, SAVINGS_IBAN)];
        let pending = |t: Transaction| Transaction {
            status: Some("pending".into()),
            ..t
        };
        let paired = Transaction {
            transfer: Some(Id::generate()),
            ..into("2026-12-11")
        };
        assert!(!none(&out, vec![into("2026-12-08")]), "3 days");
        assert!(none(&out, vec![into("2026-12-15")]), "4 days");
        assert!(none(&out, vec![pending(into("2026-12-11"))]));
        assert!(none(&[pending(out[0].clone())], vec![into("2026-12-11")]));
        assert!(none(&out, vec![paired]), "paired already");
        let unnamed = half("2026-12-11", -50_000, None);
        assert!(none(&[unnamed], vec![into("2026-12-11")]), "no IBAN");
        let two = [out[0].clone(), half("2026-12-11", -50_000, SAVINGS_IBAN)];
        let found = pairs(None, &two, &savings(vec![into("2026-12-11")]));
        assert_eq!(found.len(), 1, "each half once");
    }

    // ------------------------------------------------------------------------
    /// A harness whose vault has the bank account `ACCOUNT` (number
    /// `number`) and a statements watch on the folder `bank` beside the
    /// vault, holding `files`; and that folder.
    fn with_statements(name: &str, number: &str, files: &[(&str, &[u8])]) -> (Harness, PathBuf) {
        with_vault(name, number, files, &[])
    }

    /// As `with_statements`, the vault holding `more` too, each a file's path
    /// in it and its text.
    fn with_vault(
        name: &str,
        number: &str,
        files: &[(&str, &[u8])],
        more: &[(&str, String)],
    ) -> (Harness, PathBuf) {
        let mut h = Harness::new(name);
        let root = h.vault();
        let folder = root.with_file_name("bank");
        fs::create_dir_all(&folder).unwrap();
        for (file, bytes) in files {
            fs::write(folder.join(file), bytes).unwrap();
        }
        let account = format!(
            "id: {ACCOUNT}\ntitle: Checking account\ncreated: {{}}\nbank-account:\n  type: checking\n  number: {number}\n  owner: Alexander Yahnek\n"
        );
        let watch = format!(
            "id: {WATCH}\nfolder-watch:\n  type: statements\n  folder: '{}'\n  patterns: ['*.csv']\n  account: {ACCOUNT}\n",
            folder.display()
        );
        let own = [
            ("Accounts/checking.bnk.yaml", account),
            (".avelor/watches/watch_statements.yaml", watch),
        ];
        for (path, text) in own.iter().chain(more) {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        crate::api::switch_to(&mut h.app, root.clone());
        h.wait_scanned();
        h.take();
        (h, folder)
    }

    /// Import the watch's statements; the job's note.
    fn import(h: &mut Harness) -> String {
        import_from(h, &json!(WATCH))
    }

    /// Import the statements of the watch `watch`; the job's note.
    fn import_from(h: &mut Harness, watch: &Value) -> String {
        let sent = h.call("statements_import", json!({ "watch": watch }));
        let job = sent.last().unwrap()["ok"]["job"].clone();
        assert!(job.is_string(), "{sent:?}");
        let done = |m: &String| m.contains(r#""state":"done""#);
        let sent = h.wait_for(|sent| sent.iter().any(done));
        let ended = sent.iter().find(|m| done(m)).unwrap();
        let ended: Value = serde_json::from_str(ended).unwrap();
        assert_eq!(ended["data"]["id"], job);
        ended["data"]["note"].as_str().unwrap().to_string()
    }

    fn account(h: &Harness) -> BankAccount {
        let id = Id::parse(ACCOUNT).unwrap();
        let doc = h.app.vault.index().get(&id).unwrap();
        doc.roles().bank_account.clone().unwrap()
    }

    fn statements(h: &Harness) -> Vec<Known> {
        known(&h.app, Id::parse(ACCOUNT).unwrap())
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_statement_goes_into_its_account_once() {
        let files = [
            ("statement_2026-12.csv", DECEMBER),
            ("notes.txt", b"x".as_slice()),
        ];
        let (mut h, _) = with_statements("statements-import", "0042-7318-5520", &files);
        let note = import(&mut h);
        assert_eq!(note, "1 statement \u{b7} 28 bookings \u{b7} 2 pending");
        let known = statements(&h);
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].items.len(), 30);
        assert_eq!(known[0].balance, Some(3_247_422));
        let entry = h.app.vault.index().entry(&known[0].id).unwrap();
        let path = entry.local.to_string_lossy().replace('\\', "/");
        assert!(
            path.starts_with("Accounts/20261201T0000.checking-account"),
            "{path}"
        );
        assert_eq!(entry.doc.title(), "Checking account, December 2026");
        let fields = account(&h);
        assert_eq!(
            (fields.balance, fields.currency.as_deref()),
            (Some(32474.22), Some("EUR"))
        );
        let text = fs::read_to_string(&entry.path).unwrap();
        assert!(
            text.contains(&format!(
                "import:\n  watch: {WATCH}\n  origin: statement_2026-12.csv\n"
            )),
            "{text}"
        );
        assert!(text.contains("  ref: LVDQ2A4MX70PBBTP/360902\n"), "{text}");

        assert_eq!(import(&mut h), "No new statements");
        assert_eq!(statements(&h).len(), 1);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_watch_added_again_knows_its_accounts_statements() {
        let files = [("statement_2026-12.csv", DECEMBER)];
        let (mut h, folder) = with_statements("statements-again", "0042-7318-5520", &files);
        import(&mut h);
        h.call("remove_watch", json!({ "watch": WATCH }));
        let args = json!({ "type": "statements", "folder": folder, "patterns": ["*.csv"], "account": ACCOUNT });
        let sent = h.call("create_watch", args);
        let again = sent.last().unwrap()["ok"]["id"].clone();
        assert!(again.is_string() && again != json!(WATCH), "{sent:?}");
        assert_eq!(import_from(&mut h, &again), "No new statements");
        assert_eq!(statements(&h).len(), 1);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_file_with_nothing_new_writes_no_statement() {
        let files = [("statement_2026-12.csv", DECEMBER)];
        let (mut h, folder) = with_statements("statements-nothing-new", "0042-7318-5520", &files);
        import(&mut h);
        let booked: Vec<&[u8]> = DECEMBER
            .split(|b| *b == b'\n')
            .filter(|line| !line.starts_with(b"\"offen\""))
            .collect();
        fs::write(folder.join("copy_2026-12.csv"), booked.join(&b'\n')).unwrap();
        assert_eq!(import(&mut h), "28 in the vault already");
        assert_eq!(statements(&h).len(), 1);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn another_accounts_statement_is_left_out() {
        let files = [("statement_2026-12.csv", DECEMBER)];
        let (mut h, _) = with_statements("statements-other", "0042-7318-5539", &files);
        let note = import(&mut h);
        assert_eq!(
            note,
            "No new statements \u{b7} statement_2026-12.csv: the statement of another account, 004273185520"
        );
        assert!(statements(&h).is_empty());
        assert_eq!(account(&h).balance, None);
    }

    // ------------------------------------------------------------------------
    const SAVINGS: &str = "018572ff-1d80-78ca-9bbc-6ffd7edc2636";

    /// A harness as `with_statements` makes it with the December statement,
    /// and the House fund beside the account: numbered as December's
    /// transfer to it names it, a statement of it holding that transfer's
    /// other half; and that statement's id and the half's.
    fn with_house_fund(name: &str) -> (Harness, Id, Id) {
        let (statement, half) = (Id::generate(), Id::generate());
        let fund = format!(
            "id: {SAVINGS}\ntitle: House fund\ncreated: {{}}\nbank-account:\n  type: savings\n  number: 0042-7318-5539\n"
        );
        let december = format!(
            "id: {statement}\ntitle: House fund, December 2026\ntransactions:\n  account: {SAVINGS}\n  transactions:\n  - id: {half}\n    title: Transfer from checking\n    date: 2026-12-12\n    status: booked\n    amount: 500.0\n"
        );
        let more = [
            ("Accounts/house-fund.bnk.yaml", fund),
            ("Accounts/house-fund-dec.txs.yaml", december),
        ];
        let files = [("statement_2026-12.csv", DECEMBER)];
        let (h, _) = with_vault(name, "0042-7318-5520", &files, &more);
        (h, statement, half)
    }

    /// The transfers of the statement `id`: each half's id, and the id of
    /// the half it names.
    fn transfers(h: &Harness, id: Id) -> Vec<(Id, Id)> {
        let doc = h.app.vault.index().get(&id).unwrap();
        let items = &doc.roles().transactions.as_ref().unwrap().transactions;
        let half = |item: &Transaction| Some((item.id?, item.transfer?));
        items.iter().filter_map(half).collect()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_transfer_is_paired_with_the_other_accounts_half() {
        let (mut h, fund, half) = with_house_fund("statements-pair");
        assert_eq!(
            import(&mut h),
            "1 statement \u{b7} 28 bookings \u{b7} 2 pending \u{b7} 1 transfer paired"
        );
        let checking = statements(&h)[0].id;
        let paired = transfers(&h, checking);
        assert_eq!(paired.len(), 1);
        let (sent, named) = paired[0];
        assert_eq!(named, half);
        assert_eq!(transfers(&h, fund), [(half, sent)]);
        let path = h.app.vault.index().entry(&fund).unwrap().path.clone();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(&format!("transfer: {sent}")), "{text}");

        let unpaired = h.call(
            "unpair_transfer",
            json!({ "id": checking, "booking": sent }),
        );
        assert_eq!(shape(&unpaired), ["entity", "entity", "reply"]);
        assert!(transfers(&h, checking).is_empty());
        assert!(transfers(&h, fund).is_empty());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_watch_may_leave_transfers_unpaired() {
        let (mut h, fund, _) = with_house_fund("statements-no-pair");
        let off = json!({ "watch": WATCH, "pair_transfers": false });
        let sent = h.call("set_watch", off);
        assert_eq!(shape(&sent), ["watch", "reply"]);
        assert_eq!(sent[0]["data"]["pair_transfers"], false);
        let file = h.vault().join(".avelor/watches/watch_statements.yaml");
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("pair-transfers: false"), "{text}");

        assert_eq!(
            import(&mut h),
            "1 statement \u{b7} 28 bookings \u{b7} 2 pending"
        );
        assert!(transfers(&h, statements(&h)[0].id).is_empty());
        assert!(transfers(&h, fund).is_empty());
        h.call(
            "set_watch",
            json!({ "watch": WATCH, "pair_transfers": true }),
        );
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("pair-transfers"),
            "written only when off: {text}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_statements_watch_is_no_scans_watch() {
        let (mut h, _) = with_statements("statements-no-scans", "0042-7318-5520", &[]);
        let read = json!({ "watch": WATCH, "files": ["a.csv"] });
        let import = json!({ "watch": WATCH, "docs": [{ "title": "", "files": ["a.csv"] }] });
        for (cmd, args) in [("scan_read", read), ("scan_import", import)] {
            let sent = h.call(cmd, args);
            assert_eq!(sent[0]["err"]["code"], "unsupported", "{cmd}");
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_watch_takes_an_account_or_makes_one() {
        let (mut h, folder) = with_statements("statements-create", "0042-7318-5520", &[]);
        let args = json!({ "type": "statements", "folder": folder, "patterns": ["*.csv"] });
        let mut new = args.clone();
        new["new_account"] = json!({
            "title": " House fund ",
            "number": "0042-7318-5539",
            "type": "savings",
            "institution": "First Bank of Springfield",
        });
        let sent = h.call("create_watch", new);
        assert_eq!(shape(&sent), ["entity", "watch", "reply"]);
        let account = &sent[0]["data"];
        assert_eq!(account["kind"], "bank-account");
        assert_eq!(account["title"], "House fund");
        assert_eq!(
            account["bank-account"],
            json!({ "type": "savings", "number": "0042-7318-5539", "institution": "First Bank of Springfield" })
        );
        let path = account["path"].as_str().unwrap().replace('\\', "/");
        assert!(path.starts_with("Accounts/"), "{path}");
        assert_eq!(sent[1]["data"]["account"], account["id"]);

        let mut taken = args.clone();
        taken["account"] = json!(ACCOUNT);
        let sent = h.call("create_watch", taken.clone());
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        h.call("remove_watch", json!({ "watch": WATCH }));
        let sent = h.call("create_watch", taken);
        assert_eq!(shape(&sent), ["watch", "reply"], "a free account is taken");
        assert_eq!(sent[0]["data"]["account"], ACCOUNT);
        let mut gone = args;
        gone["account"] = json!(Id::generate());
        let sent = h.call("create_watch", gone);
        assert_eq!(sent[0]["err"]["code"], "not_found");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_new_account_needs_its_name_number_and_kind() {
        let (mut h, _) = with_statements("statements-new-account", "0042-7318-5520", &[]);
        let full = json!({ "title": "House fund", "number": "0042-7318-5539", "type": "savings" });
        let mut cases = Vec::new();
        for (field, value, message) in [
            ("title", " ", "Enter the account's name."),
            ("number", "", "Enter the account's number."),
            ("type", "loan", "Choose the account's kind."),
        ] {
            let mut account = full.clone();
            account[field] = json!(value);
            cases.push((
                json!({ "type": "statements", "new_account": account }),
                message,
            ));
        }
        let both = json!({ "type": "statements", "account": ACCOUNT, "new_account": full });
        cases.push((json!({ "type": "statements" }), "Choose the bank account."));
        cases.push((both, "Choose the bank account."));
        for (args, message) in cases {
            let sent = h.call("create_watch", args);
            assert_eq!(shape(&sent), ["reply"], "{message}");
            assert_eq!(sent[0]["err"]["code"], "invalid_argument", "{message}");
            assert_eq!(sent[0]["err"]["message"], message);
        }
        assert_eq!(h.app.watches.iter().count(), 1, "no watch is saved");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_statements_watch_may_wait_for_its_folder() {
        let files = [("statement_2026-12.csv", DECEMBER)];
        let (mut h, folder) = with_statements("statements-no-folder", "0042-7318-5520", &files);
        h.call("remove_watch", json!({ "watch": WATCH }));
        let args = json!({ "type": "statements", "patterns": ["*.csv"], "account": ACCOUNT });
        let sent = h.call("create_watch", args);
        assert_eq!(shape(&sent), ["watch", "reply"]);
        assert_eq!(sent[0]["data"]["folder"], "");
        let watch = sent[1]["ok"]["id"].clone();
        let files = fs::read_dir(h.vault().join(".avelor/watches")).unwrap();
        let text: String = files
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        assert!(!text.contains("\n  folder:"), "{text}");

        let sent = h.call("statements_import", json!({ "watch": watch }));
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        assert_eq!(
            sent[0]["err"]["message"],
            "Choose the folder of the account's statements first."
        );
        let sent = h.call("set_watch", json!({ "watch": watch, "folder": folder }));
        assert_eq!(shape(&sent), ["watch", "reply"]);
        let sent = h.call("statements_import", json!({ "watch": watch }));
        assert!(sent.last().unwrap()["ok"]["job"].is_string(), "{sent:?}");
        let done = |m: &String| m.contains(r#""state":"done""#);
        h.wait_for(|sent| sent.iter().any(done));
        assert_eq!(statements(&h).len(), 1);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn set_bank_account_changes_the_fields_given() {
        let (mut h, _) = with_statements("statements-account", "0042-7318-5520", &[]);
        let args = json!({ "entity": ACCOUNT, "institution": "First Bank of Springfield", "number": " ", "type": "savings" });
        let sent = h.call("set_bank_account", args);
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let fields = account(&h);
        assert_eq!(
            fields.institution.as_deref(),
            Some("First Bank of Springfield")
        );
        assert_eq!(fields.number, None, "an empty one removes it");
        assert_eq!(fields.account_type.as_deref(), Some("savings"));
        assert!(fields.owner.is_some(), "the others stay");
        let sent = h.call(
            "set_bank_account",
            json!({ "entity": ACCOUNT, "type": "loan" }),
        );
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        let sent = h.call(
            "set_bank_account",
            json!({ "entity": WATCH, "type": "savings" }),
        );
        assert_eq!(sent[0]["err"]["code"], "not_found");
    }
}
