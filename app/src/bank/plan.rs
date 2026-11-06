//! What a statement's import adds to its account: pure.

use std::collections::HashSet;

use av_utils::datetime::Date;
use av_utils::id::Id;
use av_vault::envelope::{Booking, Statement};
use av_vault::shared::ContactRef;

use super::comdirect::{Export, Row, booking_of};
use crate::texts::{self, Text};

// ----------------------------------------------------------------------------
/// A statement of the account in the vault, as an import sees it.
pub(crate) struct Known {
    pub id: Id,
    pub items: Vec<Booking>,
    pub balance: Option<i64>,
}

// ----------------------------------------------------------------------------
/// What importing a statement adds to its account.
pub(crate) struct Plan {
    /// Its bookings, oldest first; its booked sum and its balance.
    pub statement: Statement,
    /// The days of its first and last bookings.
    pub first: Option<Date>,
    pub last: Option<Date>,
    /// Whether it is the account's newest statement: its pending bookings
    /// are the account's, its balance the account's.
    pub newest: bool,
    /// The older statements whose pending bookings it replaces, each with
    /// what is left of its bookings.
    pub replaced: Vec<(Id, Vec<Booking>)>,
    /// The bookings the account has already.
    pub skipped: usize,
    /// What doesn't add up.
    pub problems: Vec<Text>,
}

// ----------------------------------------------------------------------------
/// What `statement` adds to the account whose statements are `known`, the
/// account's `owner` on its side of each booking: a booking the account has
/// (its reference, or without one its day, amount and purpose) is left out;
/// as the newest statement it replaces the account's pending bookings, a new
/// booking of a replaced one's card day and amount taking its id and link,
/// and as an older one it brings none; its balances are checked against its
/// bookings and the statement before it.
pub(crate) fn plan(owner: Option<&ContactRef>, known: &[Known], statement: &Export) -> Plan {
    let all_items = || known.iter().flat_map(|s| s.items.iter());
    let refs: HashSet<&str> = all_items()
        .filter(|item| !item.is_pending())
        .filter_map(|item| item.reference.as_deref())
        .collect();
    let unreferenced: HashSet<(Date, i64, &str)> = all_items()
        .filter(|item| !item.is_pending() && item.reference.is_none())
        .filter_map(|item| {
            let purpose = item.purpose.as_deref().unwrap_or_default();
            Some((item.day()?, item.cents()?, purpose))
        })
        .collect();
    let last_known = all_items().filter_map(Booking::day).max();
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
            rows.push((row, booking_of(row, owner), false));
        }
    }

    let mut replaced = Vec::new();
    if newest {
        for older in known
            .iter()
            .filter(|s| s.items.iter().any(Booking::is_pending))
        {
            let (pending, kept): (Vec<_>, Vec<_>) =
                older.items.iter().cloned().partition(Booking::is_pending);
            for item in pending {
                let (day, cents) = (item.day(), item.cents());
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
            problems.push(Text::new("bank.not_adding_up"));
        }
    }
    let first_booked = statement.rows.iter().filter_map(|r| r.booked).min();
    if let (Some(first), Some(opening)) = (first_booked, statement.opening) {
        let ends = |s: &Known| {
            s.items
                .iter()
                .filter(|i| !i.is_pending())
                .filter_map(Booking::day)
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
        .filter(|(r, ..)| r.booked.is_some())
        .map(|(r, ..)| r.cents)
        .sum();
    let role = Statement {
        account: None,
        amount: Some(written as f64 / 100.0),
        balance: statement.closing.map(|cents| cents as f64 / 100.0),
        bookings: rows.into_iter().map(|(_, booking, _)| booking).collect(),
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
