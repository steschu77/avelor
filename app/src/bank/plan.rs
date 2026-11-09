//! What a statement's import adds to its account: pure.

use std::collections::HashSet;

use av_utils::datetime::Date;
use av_utils::id::Id;
use av_vault::envelope::{Statement, Transaction};
use av_vault::shared::ContactRef;

use super::comdirect::{Export, Row, transaction_of};
use super::same_number;
use crate::texts::{self, Text};

/// How many days apart a transfer's two halves may be posted: another bank
/// books it a day, or a weekend, later.
const PAIR_DAYS: i32 = 3;

// ----------------------------------------------------------------------------
/// A statement of the account in the vault, as an import sees it.
pub(crate) struct Known {
    pub id: Id,
    pub items: Vec<Transaction>,
    pub balance: Option<i64>,
}

// ----------------------------------------------------------------------------
/// What importing a statement adds to its account.
pub(crate) struct Plan {
    /// Its transactions, oldest first; its posted sum and its balance.
    pub statement: Statement,
    /// The days of its first and last transactions.
    pub first: Option<Date>,
    pub last: Option<Date>,
    /// Whether it is the account's newest statement: its pending transactions
    /// are the account's, its balance the account's.
    pub newest: bool,
    /// The older statements whose pending transactions it replaces, each with
    /// what is left of its transactions.
    pub replaced: Vec<(Id, Vec<Transaction>)>,
    /// The transactions the account has already.
    pub skipped: usize,
    /// What doesn't add up.
    pub problems: Vec<Text>,
}

// ----------------------------------------------------------------------------
/// What `statement` adds to the account whose statements are `known`, the
/// account's `holder` on its side of each transaction: a transaction the account has
/// (its reference, or without one its day, amount and memo) is left out;
/// as the newest statement it replaces the account's pending transactions, a new
/// transaction of a replaced one's card day and amount taking its id and link,
/// and as an older one it brings none; its balances are checked against its
/// transactions and the statement before it.
pub(crate) fn plan(holder: Option<&ContactRef>, known: &[Known], statement: &Export) -> Plan {
    let all_items = || known.iter().flat_map(|s| s.items.iter());
    let refs: HashSet<&str> = all_items()
        .filter(|item| !item.is_pending())
        .filter_map(|item| item.reference.as_deref())
        .collect();
    let unreferenced: HashSet<(Date, i64, &str)> = all_items()
        .filter(|item| !item.is_pending() && item.reference.is_none())
        .filter_map(|item| {
            let memo = item.memo.as_deref().unwrap_or_default();
            Some((item.day()?, item.cents()?, memo))
        })
        .collect();
    let last_known = all_items().filter_map(Transaction::day).max();
    let last = statement.rows.iter().filter_map(Row::day).max();
    let newest = match (last, last_known) {
        (Some(last), Some(known)) => last >= known,
        (_, None) => true,
        (None, Some(_)) => false,
    };

    let mut skipped = 0;
    let mut rows = Vec::new();
    for row in statement.rows.iter().rev() {
        let known_already = match (row.posted, &row.reference) {
            (Some(_), Some(reference)) => refs.contains(reference.as_str()),
            (Some(day), None) => {
                let memo = row.memo.as_deref().unwrap_or_default();
                unreferenced.contains(&(day, row.cents, memo))
            }
            (None, _) => !newest,
        };
        if known_already {
            skipped += 1;
        } else {
            rows.push((row, transaction_of(row, holder), false));
        }
    }

    let mut replaced = Vec::new();
    if newest {
        for older in known
            .iter()
            .filter(|s| s.items.iter().any(Transaction::is_pending))
        {
            let (pending, kept): (Vec<_>, Vec<_>) = older
                .items
                .iter()
                .cloned()
                .partition(Transaction::is_pending);
            for item in pending {
                let (day, cents) = (item.day(), item.cents());
                let twin = rows.iter_mut().find(|(row, _, taken)| {
                    !taken
                        && row.card_day.is_some()
                        && row.card_day == day
                        && Some(row.cents) == cents
                });
                if let Some((_, transaction, taken)) = twin {
                    (transaction.id, transaction.linked_to, *taken) =
                        (item.id, item.linked_to, true);
                }
            }
            replaced.push((older.id, kept));
        }
    }

    let mut problems = Vec::new();
    let posted_sum: i64 = statement
        .rows
        .iter()
        .filter(|r| r.posted.is_some())
        .map(|r| r.cents)
        .sum();
    if let (Some(opening), Some(closing)) = (statement.opening, statement.closing) {
        if opening + posted_sum != closing {
            problems.push(Text::new("bank.not_adding_up"));
        }
    }
    let first_posted = statement.rows.iter().filter_map(|r| r.posted).min();
    if let (Some(first), Some(opening)) = (first_posted, statement.opening) {
        let ends = |s: &Known| {
            s.items
                .iter()
                .filter(|i| !i.is_pending())
                .filter_map(Transaction::day)
                .max()
        };
        let before = known
            .iter()
            .filter_map(|s| Some((ends(s)?, s)))
            .filter(|(end, _)| *end < first)
            .max_by_key(|(end, _)| *end);
        if let Some((end, s)) = before {
            if s.balance.is_some_and(|balance| balance != opening) {
                let text = Text::new("bank.missing_between").arg("from", texts::date(end));
                problems.push(text.arg("to", texts::date(first)));
            }
        }
    }

    let written: i64 = rows
        .iter()
        .filter(|(r, ..)| r.posted.is_some())
        .map(|(r, ..)| r.cents)
        .sum();
    let role = Statement {
        amount: Some(written as f64 / 100.0),
        balance: statement.closing.map(|cents| cents as f64 / 100.0),
        transactions: rows
            .into_iter()
            .map(|(_, transaction, _)| transaction)
            .collect(),
        ..Statement::default()
    };
    Plan {
        first: statement.rows.iter().filter_map(Row::day).min(),
        last,
        statement: role,
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
/// A transfer paired: a new transaction, by its index, and its other half, a
/// transaction of another account's statement.
#[derive(Debug, PartialEq)]
pub(crate) struct Pair {
    pub transaction: usize,
    pub statement: Id,
    pub other: Id,
}

// ----------------------------------------------------------------------------
/// The transfers between `transactions`, new transactions of the account numbered
/// `number`, and the transactions of `others`, the vault's other accounts: money
/// out whose `iban` names the account it goes to (`same_number`), and the
/// same amount into that account, at most `PAIR_DAYS` apart, the nearest
/// first. Only posted transactions without a pair pair, each once.
pub(crate) fn pairs(
    number: Option<&str>,
    transactions: &[Transaction],
    others: &[OtherAccount],
) -> Vec<Pair> {
    let names = |iban: Option<&str>, number: Option<&str>| {
        iban.zip(number)
            .is_some_and(|(iban, number)| same_number(iban, number))
    };
    let open = |item: &Transaction| !item.is_pending() && item.transfer.is_none();
    let mut pairs: Vec<Pair> = Vec::new();
    for (i, transaction) in transactions.iter().enumerate() {
        let (Some(day), Some(cents)) = (transaction.day(), transaction.cents()) else {
            continue;
        };
        if cents == 0 || !open(transaction) {
            continue;
        }
        let apart = |item: &Transaction| item.day().map(|d| (d.days() - day.days()).abs());
        let half = |other: &OtherAccount, item: &Transaction| {
            let named = if cents < 0 {
                names(transaction.iban.as_deref(), other.number.as_deref())
            } else {
                names(item.iban.as_deref(), number)
            };
            named
                && open(item)
                && item.cents() == Some(-cents)
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
                transaction: i,
                statement,
                other,
            });
        }
    }
    pairs
}

// ----------------------------------------------------------------------------
