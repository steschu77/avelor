//! Recurring payments (vault.md#recurring): a series is a file of its own in
//! `Accounts/` (`create_recurring`, `set_recurring`), and its bookings name
//! it (`set_booking_recurring`). The statements import puts each new booking
//! into the series it pays (`join_new`, import.md#statements), the days of
//! its schedule from av-utils' `recurrence`. The page reads a series'
//! occurrences with the calendar's expander.

use std::path::Path;

use av_utils::datetime::Date;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_utils::recurrence::occurrences;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{Empty, Recurring, Roles, Source, Transaction};
use av_vault::schema::layout;
use av_vault::shared::ContactRef;
use av_vault::vault::{Index, SaveOutcome};
use serde::Deserialize;

use crate::app::App;
use crate::bank::{self, day_of, to_cents};
use crate::protocol::{Booking, Req};
use crate::structure;

/// How many days from an occurrence a booking still pays it: a debit
/// booked after a weekend or a holiday. Half the gap to the next
/// occurrence, when that is less.
const PAY_DAYS: i32 = 7;
/// How far past the newest booking the occurrences are read, so the last
/// one's gap to the next is known.
const AHEAD: i32 = 62;

// ----------------------------------------------------------------------------
/// `create_recurring`'s args.
#[derive(Deserialize)]
pub(crate) struct NewRecurring {
    #[serde(default)]
    title: String,
    recurring: Recurring,
    #[serde(default)]
    linked_to: Option<Id>,
    #[serde(default)]
    projects: Vec<Id>,
    #[serde(default)]
    tags: Vec<String>,
}

// ----------------------------------------------------------------------------
/// `create_recurring {title, recurring, linked_to?, projects?, tags?}`: a new
/// series in `Accounts/`, dated its first occurrence. Answers `{id}`;
/// `invalid_argument` names what is missing or wrong (`check`), or
/// `linked_to` for an entity the vault doesn't have.
pub fn create_recurring(app: &mut App, req: Req, new: NewRecurring) {
    let title = new.title.trim().to_string();
    if title.is_empty() {
        return req.err(Error::InvalidArgument {
            arg: "title".into(),
        });
    }
    if let Err(arg) = check(app, &new.recurring) {
        return req.err(Error::InvalidArgument { arg: arg.into() });
    }
    if new
        .linked_to
        .is_some_and(|to| app.vault.index().get(&to).is_none())
    {
        return req.err(Error::InvalidArgument {
            arg: "linked_to".into(),
        });
    }
    let from = new.recurring.from;
    let roles = Roles {
        recurring: Some(new.recurring),
        ..Roles::default()
    };
    let source = Source::Created(Empty {});
    let mut doc = Document::create(roles, source, title, new.projects, FileFormat::YAML);
    doc.set_tags(new.tags);
    doc.set_link(new.linked_to);
    if let Some(from) = from {
        doc.set_date(from);
    }
    let created =
        app.edit(|index, changes| index.create(doc, Path::new(layout::BANK_ACCOUNTS), changes));
    let id = created.as_ref().ok().copied();
    structure::answer(app, req, created.map(|_| SaveOutcome::Saved), id);
}

// ----------------------------------------------------------------------------
/// `set_recurring`'s args.
#[derive(Deserialize)]
pub(crate) struct RecurringChange {
    id: Id,
    recurring: Recurring,
}

// ----------------------------------------------------------------------------
/// `set_recurring {id, recurring}`: the series `id`'s role as given, checked
/// as `create_recurring` checks it. Answered as `set_title` is; `not_found`
/// for an entity that is no series.
pub fn set_recurring(app: &mut App, req: Req, change: RecurringChange) {
    let id = change.id;
    let doc = app.vault.index().get(&id);
    if doc.is_none_or(|doc| doc.roles().recurring.is_none()) {
        return req.err(Error::NotFound);
    }
    if let Err(arg) = check(app, &change.recurring) {
        return req.err(Error::InvalidArgument { arg: arg.into() });
    }
    let result = app.edit(|index, changes| {
        index.edit(&id, changes, |d| d.set_recurring(Some(change.recurring)))
    });
    structure::answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// What a series needs, or the name of the field that is missing or wrong:
/// a bank account of the vault, a schedule, its first occurrence, an amount,
/// and a `vary` that isn't negative.
fn check(app: &App, recurring: &Recurring) -> std::result::Result<(), &'static str> {
    if !recurring
        .account
        .is_some_and(|id| bank::is_account(app, id))
    {
        return Err("account");
    }
    if recurring.every.is_empty() {
        return Err("every");
    }
    if recurring.from.is_none() {
        return Err("from");
    }
    if recurring.amounts.is_empty() {
        return Err("amounts");
    }
    if recurring.vary.is_some_and(|vary| vary < 0.0) {
        return Err("vary");
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// `set_booking_recurring`'s args.
#[derive(Deserialize)]
pub(crate) struct BookingsSeries {
    bookings: Vec<Booking>,
    to: Option<Id>,
}

// ----------------------------------------------------------------------------
/// `set_booking_recurring {bookings: [{id, booking}], to}`: the bookings
/// named join the series `to`, or leave theirs with `null`
/// (`Index::set_booking_recurring`); each statement is written once.
/// Answered as the first statement's change.
pub fn set_booking_recurring(app: &mut App, req: Req, set: BookingsSeries) {
    let mut statements: Vec<(Id, Vec<Id>)> = Vec::new();
    for item in set.bookings {
        match statements.iter_mut().find(|(id, _)| *id == item.id) {
            Some((_, bookings)) => bookings.push(item.booking),
            None => statements.push((item.id, vec![item.booking])),
        }
    }
    let first = statements.first().map(|(id, _)| *id);
    let result = app.edit(|index, changes| -> Result<SaveOutcome> {
        let mut outcome = SaveOutcome::Unchanged;
        for (id, bookings) in &statements {
            outcome = outcome.and(index.set_booking_recurring(id, bookings, set.to, changes)?);
            if matches!(outcome, SaveOutcome::Conflict(_)) {
                break;
            }
        }
        Ok(outcome)
    });
    structure::answer(app, req, result, first);
}

// ----------------------------------------------------------------------------
/// Put the new bookings `bookings` of a statement of `account` that have no
/// series into the series they pay (`join`); how many joined.
pub(crate) fn join_new(app: &App, account: Id, bookings: &mut [Transaction]) -> usize {
    let Some(newest) = bookings.iter().filter_map(day_of).max() else {
        return 0;
    };
    let last = Date::new(newest.days() + AHEAD);
    let mut series = series_of(app.vault.index(), account, last);
    join(&mut series, bookings)
}

// ----------------------------------------------------------------------------
/// The series of `account` as `join` reads them, their occurrences up to
/// `last`; a series whose schedule `occurrences` doesn't read is left out.
fn series_of(index: &Index, account: Id, last: Date) -> Vec<Series> {
    let entries = index.all();
    let bookings: Vec<&Transaction> = entries
        .iter()
        .filter_map(|e| e.doc.roles().transactions.as_ref())
        .filter(|t| t.account == Some(account))
        .flat_map(|t| &t.transactions)
        .collect();
    let name = |party: &ContactRef| match party {
        ContactRef::Name(name) => Some(name.trim().to_lowercase()),
        ContactRef::Id(id) => index.get(id).map(|doc| doc.title().trim().to_lowercase()),
    };
    entries
        .iter()
        .filter_map(|e| {
            let recurring = e.doc.roles().recurring.as_ref()?;
            if recurring.account != Some(account) {
                return None;
            }
            let id = e.doc.id();
            let days = days_of(recurring, last)?;
            let own: Vec<&&Transaction> = bookings
                .iter()
                .filter(|b| b.recurring == Some(id))
                .collect();
            let parties = own.iter().filter_map(|b| party_of(b));
            let mut names: Vec<String> = Vec::new();
            for found in recurring.party.iter().chain(parties).filter_map(name) {
                if !names.contains(&found) {
                    names.push(found);
                }
            }
            let amounts = recurring
                .amounts
                .iter()
                .map(|a| (a.from.date, to_cents(a.amount)))
                .collect();
            let booked: Vec<Date> = own.iter().filter_map(|b| day_of(b)).collect();
            Some(Series::new(id, names, amounts, days, &booked))
        })
        .collect()
}

// ----------------------------------------------------------------------------
/// The days of `recurring`'s schedule up to `last`: of its one RRULE line;
/// `None` for a schedule of more lines.
fn days_of(recurring: &Recurring, last: Date) -> Option<Vec<Date>> {
    let [line] = recurring.every.as_slice() else {
        return None;
    };
    let until = recurring.until.map(|until| until.date);
    occurrences(line, recurring.from?.date, until, last)
}

// ----------------------------------------------------------------------------
/// The other side of `booking`: who was paid for money out, who paid for
/// money in.
fn party_of(booking: &Transaction) -> Option<&ContactRef> {
    if booking.amount.is_some_and(|amount| amount < 0.0) {
        booking.receiver.as_ref()
    } else {
        booking.sender.as_ref()
    }
}

// ----------------------------------------------------------------------------
/// A series as `join` reads it.
#[derive(Debug)]
pub(crate) struct Series {
    id: Id,
    /// Its party's name and its bookings' parties, lowercase.
    names: Vec<String>,
    /// Its amounts in cents from their days on, oldest first.
    amounts: Vec<(Date, i64)>,
    /// Its occurrences.
    days: Vec<Date>,
    /// The occurrences its bookings pay.
    paid: Vec<Date>,
}

impl Series {
    // ------------------------------------------------------------------------
    /// The series `id`, each of its bookings `booked` on a day paying the
    /// occurrence it is nearest to.
    fn new(
        id: Id,
        names: Vec<String>,
        amounts: Vec<(Date, i64)>,
        days: Vec<Date>,
        booked: &[Date],
    ) -> Series {
        let mut series = Series {
            id,
            names,
            amounts,
            days,
            paid: Vec::new(),
        };
        let mut booked = booked.to_vec();
        booked.sort_unstable();
        for day in booked {
            if let Some(occurrence) = series.open_near(day) {
                series.paid.push(occurrence);
            }
        }
        series
    }

    // ------------------------------------------------------------------------
    /// Its amount on `day`: the last one from that day or before, else the
    /// first.
    fn amount_on(&self, day: Date) -> Option<i64> {
        let current = self.amounts.iter().rev().find(|(from, _)| *from <= day);
        current.or(self.amounts.first()).map(|(_, cents)| *cents)
    }

    // ------------------------------------------------------------------------
    /// The occurrence nearest to `day` that no booking pays yet, within its
    /// reach (`reach`).
    fn open_near(&self, day: Date) -> Option<Date> {
        let apart = |occurrence: &Date| (occurrence.days() - day.days()).abs();
        self.days
            .iter()
            .enumerate()
            .filter(|(i, o)| !self.paid.contains(o) && apart(o) <= self.reach(*i))
            .min_by_key(|(_, o)| apart(o))
            .map(|(_, o)| *o)
    }

    // ------------------------------------------------------------------------
    /// How many days from its `i`th occurrence a booking still pays it.
    fn reach(&self, i: usize) -> i32 {
        let gap = |a: usize, b: usize| match (self.days.get(a), self.days.get(b)) {
            (Some(a), Some(b)) => b.days() - a.days(),
            _ => i32::MAX,
        };
        let before = i.checked_sub(1).map_or(i32::MAX, |j| gap(j, i));
        PAY_DAYS.min(before.min(gap(i, i + 1)) / 2)
    }
}

// ----------------------------------------------------------------------------
/// Put each booking of `bookings` without a series, in their order (a
/// statement's: oldest first), into the series it pays: on whose party it
/// names (case aside), going the same way, in or out, and near an
/// occurrence no booking pays yet (`Series::open_near`). Of several, the
/// one whose amount is nearest, then whose occurrence is. The amount
/// doesn't decide: one off the series' is off on its card. How many joined.
pub(crate) fn join(series: &mut [Series], bookings: &mut [Transaction]) -> usize {
    let mut joined = 0;
    for booking in bookings.iter_mut() {
        if booking.recurring.is_some() {
            continue;
        }
        let (Some(day), Some(cents)) = (day_of(booking), booking.amount.map(to_cents)) else {
            continue;
        };
        let Some(ContactRef::Name(party)) = party_of(booking) else {
            continue;
        };
        let party = party.trim().to_lowercase();
        let mut best: Option<(usize, Date, (i64, i32))> = None;
        for (s, one) in series.iter().enumerate() {
            let Some(expected) = one.amount_on(day) else {
                continue;
            };
            if !one.names.contains(&party) || (expected < 0) != (cents < 0) {
                continue;
            }
            let Some(occurrence) = one.open_near(day) else {
                continue;
            };
            let key = (
                (cents - expected).abs(),
                (occurrence.days() - day.days()).abs(),
            );
            if best.as_ref().is_none_or(|(.., found)| key < *found) {
                best = Some((s, occurrence, key));
            }
        }
        if let Some((s, occurrence, _)) = best {
            series[s].paid.push(occurrence);
            booking.recurring = Some(series[s].id);
            joined += 1;
        }
    }
    joined
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, shape};
    use av_utils::datetime::DateTime;
    use serde_json::json;
    use std::fs;

    fn day(text: &str) -> Date {
        DateTime::from_iso8601(text).unwrap().date
    }

    fn days(texts: &[&str]) -> Vec<Date> {
        texts.iter().map(|text| day(text)).collect()
    }

    fn every(rule: &str, from: &str, until: Option<&str>, last: &str) -> Option<Vec<Date>> {
        occurrences(
            &format!("RRULE:{rule}"),
            day(from),
            until.map(day),
            day(last),
        )
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_schedule_of_one_rrule_line_is_read() {
        let recurring = |every: &[&str]| Recurring {
            every: every.iter().map(|line| line.to_string()).collect(),
            from: Some(DateTime::from_iso8601("2026-12-01").unwrap()),
            ..Recurring::default()
        };
        let last = day("2027-01-31");
        let one = recurring(&["RRULE:FREQ=MONTHLY;BYMONTHDAY=1"]);
        assert_eq!(
            days_of(&one, last),
            Some(days(&["2026-12-01", "2027-01-01"]))
        );
        let two = recurring(&["RRULE:FREQ=MONTHLY;BYMONTHDAY=1", "EXDATE:20261201"]);
        assert_eq!(days_of(&two, last), None);
    }

    // ------------------------------------------------------------------------
    fn booking(on: &str, amount: f64, party: &str) -> Transaction {
        let party = Some(ContactRef::Name(party.to_string()));
        let (sender, receiver) = if amount < 0.0 {
            (None, party)
        } else {
            (party, None)
        };
        Transaction {
            date: Some(DateTime::from_iso8601(on).unwrap()),
            amount: Some(amount),
            sender,
            receiver,
            ..Transaction::default()
        }
    }

    fn monthly(id: Id, names: &[&str], amount: i64, booked: &[&str]) -> Series {
        let firsts = every(
            "FREQ=MONTHLY;BYMONTHDAY=1",
            "2026-08-01",
            None,
            "2027-03-01",
        )
        .unwrap();
        let names = names.iter().map(|n| n.to_lowercase()).collect();
        Series::new(
            id,
            names,
            vec![(day("2026-08-01"), amount)],
            firsts,
            &days(booked),
        )
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_booking_joins_the_series_it_pays() {
        let rent = Id::generate();
        let mut series = [monthly(rent, &["Shady Pines"], -115_000, &["2026-08-01"])];
        let mut bookings = [
            booking("2026-09-02", -1150.0, "SHADY PINES"),
            booking("2026-09-03", -75.0, "Shady Pines"),
            booking("2026-09-15", -1150.0, "Shady Pines"),
            booking("2026-10-01", 375.0, "Shady Pines"),
            booking("2026-08-02", -1150.0, "Shady Pines"),
            booking("2026-11-01", -1150.0, "Shady Pines Ltd."),
        ];
        assert_eq!(join(&mut series, &mut bookings), 1);
        let joined: Vec<bool> = bookings.iter().map(|b| b.recurring == Some(rent)).collect();
        assert_eq!(
            joined,
            [true, false, false, false, false, false],
            "the case aside; the occurrence paid; a week away; money in; August paid; another name"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_name_its_bookings_bring_is_known_and_the_nearest_amount_wins() {
        let (salary, bonus) = (Id::generate(), Id::generate());
        let mut series = [
            monthly(salary, &["Cogsworth", "Cogsworth Inc."], 187_246, &[]),
            monthly(bonus, &["Cogsworth Inc."], 50_000, &[]),
        ];
        let mut bookings = [
            booking("2026-12-01", 1872.46, "Cogsworth Inc."),
            booking("2026-12-02", 500.0, "Cogsworth Inc."),
        ];
        assert_eq!(join(&mut series, &mut bookings), 2);
        assert_eq!(bookings[0].recurring, Some(salary));
        assert_eq!(bookings[1].recurring, Some(bonus));

        let taken = Transaction {
            recurring: Some(bonus),
            ..booking("2027-01-01", 1872.46, "Cogsworth")
        };
        let mut bookings = [taken];
        assert_eq!(join(&mut series, &mut bookings), 0, "it has its series");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn twice_a_month_each_booking_pays_its_own_day() {
        let id = Id::generate();
        let twice = every(
            "FREQ=MONTHLY;BYMONTHDAY=15,-1",
            "2026-12-15",
            None,
            "2027-02-28",
        )
        .unwrap();
        let mut series = [Series::new(
            id,
            vec!["cogsworth".into()],
            vec![(day("2026-12-15"), 187_246)],
            twice,
            &[],
        )];
        assert_eq!(series[0].reach(0), 7);
        let mut bookings = [
            booking("2026-12-15", 1872.46, "Cogsworth"),
            booking("2026-12-22", 1872.46, "Cogsworth"),
            booking("2026-12-31", 1872.46, "Cogsworth"),
        ];
        assert_eq!(join(&mut series, &mut bookings), 2);
        let joined: Vec<bool> = bookings.iter().map(|b| b.recurring.is_some()).collect();
        assert_eq!(joined, [true, false, true], "a week from either day");
        assert_eq!(series[0].paid, days(&["2026-12-15", "2026-12-31"]));
    }

    // ------------------------------------------------------------------------
    const ACCOUNT: &str = "01955b3b-ca80-7fef-afbe-46d15556d9a1";
    const STATEMENT: &str = "01a0f61a-16a0-7a60-a0cd-3c68045b675b";
    const RENT: &str = "01a05bdf-f8c0-72e2-9833-76b178de839f";
    const FEE: &str = "01a0d314-5440-7f08-9d18-a0c75bd9e696";

    /// A vault with the checking account, a statement of it with the rent
    /// and a fee, and a document.
    fn with_account(name: &str) -> (Harness, Id) {
        let document = Id::generate();
        let files = [
            (
                "Accounts/checking.bnk.yaml".to_string(),
                format!(
                    "id: {ACCOUNT}\ntitle: Checking\ncreated: {{}}\nbank-account:\n  type: checking\n"
                ),
            ),
            (
                "Accounts/sep.txs.yaml".to_string(),
                format!(
                    "id: {STATEMENT}\ntitle: September\ntransactions:\n  account: {ACCOUNT}\n  transactions:\n  - id: {RENT}\n    date: 2026-09-01\n    amount: -1150.0\n    receiver: Shady Pines\n  - id: {FEE}\n    date: 2026-09-02\n    amount: -4.9\n"
                ),
            ),
            (
                "Documents/lease.doc.md".to_string(),
                format!("---\nid: {document}\ntitle: Lease\ndocument: {{}}\n---\n"),
            ),
        ];
        (Harness::with_vault(name, &files), document)
    }

    fn rent(account: &str) -> serde_json::Value {
        json!({
            "account": account,
            "party": "Shady Pines",
            "every": ["RRULE:FREQ=MONTHLY;BYMONTHDAY=1"],
            "from": "2026-09-01",
            "amounts": [{ "from": "2026-09-01", "amount": -1150.0 }],
            "category": "housing",
        })
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_series_is_made_in_accounts_and_its_bookings_join_it() {
        let (mut h, lease) = with_account("recurring-create");
        let args = json!({ "title": "Rent", "recurring": rent(ACCOUNT), "linked_to": lease });
        let sent = h.call("create_recurring", args);
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let id = Id::parse(sent[1]["ok"]["id"].as_str().unwrap()).unwrap();
        let entry = h.app.vault.index().entry(&id).unwrap();
        let path = entry.local.to_string_lossy().replace('\\', "/");
        assert!(path.starts_with("Accounts/20260901T0000.rent."), "{path}");
        assert!(path.ends_with(".rcp.yaml"), "{path}");
        assert_eq!(entry.doc.linked_to(), Some(lease));

        let bookings = json!([{ "id": STATEMENT, "booking": RENT }]);
        let sent = h.call(
            "set_booking_recurring",
            json!({ "bookings": bookings, "to": id }),
        );
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let text = fs::read_to_string(h.vault().join("Accounts/sep.txs.yaml")).unwrap();
        assert!(text.contains(&format!("recurring: {id}")), "{text}");

        let mut changed = rent(ACCOUNT);
        changed["amounts"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "from": "2027-01-01", "amount": -1190.0 }));
        let sent = h.call("set_recurring", json!({ "id": id, "recurring": changed }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let doc = h.app.vault.index().get(&id).unwrap();
        assert_eq!(doc.roles().recurring.as_ref().unwrap().amounts.len(), 2);

        let sent = h.call(
            "set_booking_recurring",
            json!({ "bookings": bookings, "to": null }),
        );
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let text = fs::read_to_string(h.vault().join("Accounts/sep.txs.yaml")).unwrap();
        assert!(!text.contains("recurring"), "{text}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_series_needs_its_account_schedule_start_and_amount() {
        let (mut h, _) = with_account("recurring-refused");
        // the reply's message names the argument
        let refused = |h: &mut Harness, args: serde_json::Value, arg: &str| {
            let sent = h.call("create_recurring", args);
            assert_eq!(sent[0]["err"]["code"], "invalid_argument", "{arg}");
            let message = sent[0]["err"]["message"].as_str().unwrap_or_default();
            assert!(message.contains(&format!("\"{arg}\"")), "{arg}: {message}");
        };
        for (key, value) in [
            ("account", json!(STATEMENT)),
            ("every", json!([])),
            ("from", json!(null)),
            ("amounts", json!([])),
            ("vary", json!(-1.0)),
        ] {
            let mut recurring = rent(ACCOUNT);
            recurring[key] = value;
            refused(
                &mut h,
                json!({ "title": "Rent", "recurring": recurring }),
                key,
            );
        }
        refused(
            &mut h,
            json!({ "title": " ", "recurring": rent(ACCOUNT) }),
            "title",
        );
        let unlinked =
            json!({ "title": "Rent", "recurring": rent(ACCOUNT), "linked_to": Id::generate() });
        refused(&mut h, unlinked, "linked_to");
        let other = json!({ "id": STATEMENT, "recurring": rent(ACCOUNT) });
        let sent = h.call("set_recurring", other);
        assert_eq!(sent[0]["err"]["code"], "not_found");
        assert!(
            h.app
                .vault
                .index()
                .all()
                .iter()
                .all(|e| e.doc.roles().recurring.is_none())
        );
    }
}
