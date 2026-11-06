//! comdirect's CSV export read, its rows as transactions: pure.

use av_utils::charsets::from_utf8_or_windows1252;
use av_utils::datetime::{Date, DateTime};
use av_utils::id::Id;
use av_vault::envelope::Transaction;
use av_vault::shared::ContactRef;

use super::some;
use crate::texts::Text;

/// The header row's cells before the amount's, which names the currency.
const HEADER: [&str; 4] = [
    "Buchungstag",
    "Wertstellung (Valuta)",
    "Vorgang",
    "Buchungstext",
];
/// A transfer's or direct debit's memo comes cut into lines of this many
/// characters, joined by a space.
const MEMO_LINE: usize = 35;

// ----------------------------------------------------------------------------
/// A statement as the bank exported it; amounts in cents.
#[derive(Debug, Default)]
pub(crate) struct Export {
    pub currency: Option<String>,
    /// The balance before its transactions, and after them.
    pub opening: Option<i64>,
    pub closing: Option<i64>,
    /// Newest first, as the bank lists them.
    pub rows: Vec<Row>,
}

// ----------------------------------------------------------------------------
/// A transaction of a statement.
#[derive(Debug, Default)]
pub(crate) struct Row {
    /// The day it was posted; `None` while it is pending.
    pub posted: Option<Date>,
    /// The bank's word for it ("Lastschrift / Belastung").
    pub vorgang: String,
    /// `card`, `direct-debit`, `transfer`, `cash` or `fee`.
    pub tx_type: Option<&'static str>,
    /// Who paid or was paid, as the bank names them.
    pub party: Option<String>,
    /// A card payment's merchant.
    pub merchant: Option<String>,
    pub memo: Option<String>,
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
    /// Its day: when it was posted, or the card's while it is pending.
    pub(super) fn day(&self) -> Option<Date> {
        self.posted.or(self.card_day)
    }
}

// ----------------------------------------------------------------------------
/// The statement in `bytes`, a CSV export of comdirect's ("Umsätze"):
/// UTF-8 or Windows-1252, quoted cells each followed by `;`, the balances
/// before and after the transactions, German dates and amounts. Or why it isn't
/// one.
pub(crate) fn read(bytes: &[u8]) -> Result<Export, Text> {
    let text = from_utf8_or_windows1252(bytes);
    let mut statement = Export::default();
    let (mut header, mut in_rows) = (false, false);
    for (n, line) in text.lines().enumerate() {
        let cells = cells(line);
        if in_rows {
            if cells.is_empty() {
                in_rows = false;
            } else {
                let unread = || Text::new("bank.line_unread").arg("line", n + 1);
                let row = row(&cells).ok_or_else(unread)?;
                statement.rows.push(row);
            }
            continue;
        }
        match cells.first().map(String::as_str) {
            Some(which @ ("Alter Kontostand" | "Neuer Kontostand")) => {
                let balance = cells.get(1).map(String::as_str).unwrap_or_default();
                let (amount, currency) = balance.split_once(' ').unwrap_or((balance, ""));
                let cents = cents(amount)
                    .ok_or_else(|| Text::new("bank.line_unread").arg("line", n + 1))?;
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
        Err(Text::new("bank.not_a_statement"))
    }
}

// ----------------------------------------------------------------------------
/// The cells of a line: each quoted (a `""` in it is a `"`) or not, each
/// followed by `;`.
pub(super) fn cells(line: &str) -> Vec<String> {
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
/// The transaction of a row's cells: its days, what it is, its text, its amount.
fn row(cells: &[String]) -> Option<Row> {
    let [posted, _value, vorgang, text, amount, ..] = cells else {
        return None;
    };
    let posted = match posted.trim() {
        "offen" => None,
        day => Some(german_day(day)?),
    };
    let mut row = Row {
        posted,
        vorgang: vorgang.trim().to_string(),
        cents: cents(amount)?,
        ..Row::default()
    };
    buchungstext(&mut row, text);
    row.tx_type = tx_type(&row);
    let memo = row.memo.take().unwrap_or_default();
    row.memo = match row.tx_type {
        Some("transfer" | "direct-debit") => some(&joined(&memo)),
        _ => some(&memo),
    };
    Some(row)
}

// ----------------------------------------------------------------------------
/// `dd.mm.yyyy`.
pub(super) fn german_day(text: &str) -> Option<Date> {
    let mut parts = text.split('.').map(|part| part.parse::<i32>().ok());
    let (day, month, year) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() {
        return None;
    }
    Date::from_ymd(year, month, day).ok()
}

// ----------------------------------------------------------------------------
/// A German amount in cents: `-1.927,35`, `100,00`.
pub(super) fn cents(text: &str) -> Option<i64> {
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
/// `Buchungstext:` the memo (a card payment's merchant line and the
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
    // One space follows the colon; the memo's own spaces count.
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
            row.memo = some(&line);
        }
        None => row.memo = Some(body.to_string()),
    }
}

// ----------------------------------------------------------------------------
/// A card's text: its merchant line, then the time it was used
/// (`2026-12-01T07:12:40`) and, once posted, `KFN n VJ mmyy`, the card's
/// sequence and expiry. The line and the card's day.
fn card(body: &str) -> Option<(String, Date)> {
    let words: Vec<&str> = body.split_whitespace().collect();
    let at = words.iter().position(|word| card_day(word).is_some())?;
    let tail = &words[at + 1..];
    let posted = tail.len() >= 4 && tail[0] == "KFN" && tail[2] == "VJ";
    if !tail.is_empty() && !posted {
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
fn tx_type(row: &Row) -> Option<&'static str> {
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
/// `memo` without the spaces the bank put between its lines.
pub(super) fn joined(memo: &str) -> String {
    let joint = |i: usize| i >= MEMO_LINE && (i - MEMO_LINE).is_multiple_of(MEMO_LINE + 1);
    memo.chars()
        .enumerate()
        .filter(|&(i, c)| !(c == ' ' && joint(i)))
        .map(|(_, c)| c)
        .collect()
}

// ----------------------------------------------------------------------------
/// The transaction of `row`, the account's `holder` on its side.
pub(super) fn transaction_of(row: &Row, holder: Option<&ContactRef>) -> Transaction {
    let other = row.party.clone().or_else(|| row.merchant.clone());
    let title = other.clone().unwrap_or_else(|| row.vorgang.clone());
    let other = other.map(ContactRef::Name);
    let (sender, receiver) = if row.cents < 0 {
        (holder.cloned(), other)
    } else {
        (other, holder.cloned())
    };
    let status = if row.posted.is_some() {
        "posted"
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
        memo: row.memo.clone(),
        tx_type: row.tx_type.map(str::to_string),
        reference: row.reference.clone(),
        iban: row.iban.clone(),
        bic: row.bic.clone(),
        ..Transaction::default()
    }
}
