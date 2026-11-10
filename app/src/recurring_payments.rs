//! Recurring payments (vault.md#recurring): a recurring payment is a file of its own in
//! `Accounts/` (`create_recurring_payment`, `set_recurring_payment`), and its transactions name
//! it (`set_transaction_recurring_payment`). The statements import puts each new transaction
//! into the recurring payment it pays (`join_new`, import.md#statements), the days of
//! its schedule from av-utils' `schedule`. The page reads a recurring payment's
//! occurrences with the calendar's expander.

use std::path::Path;

use av_utils::datetime::Date;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_utils::schedule::occurrences;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{Bare, RecurringPayment, Roles, Source, Transaction, to_cents};
use av_vault::schema::layout;
use av_vault::shared::ContactRef;
use av_vault::vault::{Index, SaveOutcome};
use serde::Deserialize;

use crate::app::App;
use crate::bank;
use crate::protocol::{Req, TransactionRef};
use crate::structure;
use crate::texts::Text;

/// How many days from an occurrence a transaction still pays it: a debit
/// posted after a weekend or a holiday. Half the gap to the next
/// occurrence, when that is less.
const PAY_DAYS: i32 = 7;
/// How far past the newest transaction the occurrences are read, so the last
/// one's gap to the next is known.
const AHEAD: i32 = 62;

// ----------------------------------------------------------------------------
/// `create_recurring_payment`'s args.
#[derive(Deserialize)]
pub(crate) struct NewRecurringPayment {
    #[serde(default)]
    title: String,
    recurring_payment: RecurringPayment,
    #[serde(default)]
    linked_to: Option<Id>,
    #[serde(default)]
    projects: Vec<Id>,
    #[serde(default)]
    labels: Vec<String>,
}

// ----------------------------------------------------------------------------
/// `create_recurring_payment {title, recurring_payment, linked_to?, projects?, labels?}`: a new
/// recurring payment in `Accounts/`, dated its first occurrence. Answers `{id}`;
/// `invalid_argument` names what is missing or wrong (`check`), or
/// `linked_to` for an entity the vault doesn't have.
pub fn create_recurring_payment(app: &mut App, req: Req, new: NewRecurringPayment) {
    let title = new.title.trim().to_string();
    if title.is_empty() {
        return req.err(Error::InvalidArgument {
            arg: "title".into(),
        });
    }
    if let Err(arg) = check(app, &new.recurring_payment) {
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
    let step = Text::new("undo.create_recurring_payment").arg("title", title.clone());
    let from = new.recurring_payment.from;
    let roles = Roles {
        recurring_payment: Some(new.recurring_payment),
        ..Roles::default()
    };
    let source = Source::Created(Bare::default());
    let mut doc = Document::create(roles, source, title, new.projects, FileFormat::YAML);
    doc.set_labels(new.labels);
    doc.set_link(new.linked_to);
    if let Some(from) = from {
        doc.set_date(from);
    }
    let created = app.step(&req, step, |index, changes| {
        index.create(doc, Path::new(layout::BANK_ACCOUNTS), changes)
    });
    let id = created.as_ref().ok().copied();
    structure::answer(app, req, created.map(|_| SaveOutcome::Saved), id);
}

// ----------------------------------------------------------------------------
/// `set_recurring_payment`'s args.
#[derive(Deserialize)]
pub(crate) struct RecurringPaymentChange {
    id: Id,
    recurring_payment: RecurringPayment,
}

// ----------------------------------------------------------------------------
/// `set_recurring_payment {id, recurring_payment}`: the recurring payment `id`'s role as given, checked
/// as `create_recurring_payment` checks it. Answered as `set_title` is; `not_found`
/// for an entity that is no recurring payment.
pub fn set_recurring_payment(app: &mut App, req: Req, change: RecurringPaymentChange) {
    let id = change.id;
    let doc = app.vault.index().get(&id);
    if doc.is_none_or(|doc| doc.roles().recurring_payment.is_none()) {
        return req.err(Error::NotFound);
    }
    if let Err(arg) = check(app, &change.recurring_payment) {
        return req.err(Error::InvalidArgument { arg: arg.into() });
    }
    let step = Text::new("undo.set_recurring_payment").arg("title", structure::named(app, &id, ""));
    let result = app.step(&req, step, |index, changes| {
        index.edit(&id, changes, |d| {
            d.set_recurring_payment(Some(change.recurring_payment))
        })
    });
    structure::answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// What a recurring payment needs, or the name of the field that is missing or wrong:
/// a bank account of the vault, a schedule, its first occurrence, an amount,
/// and a `vary` that isn't negative.
fn check(app: &App, recurring: &RecurringPayment) -> std::result::Result<(), &'static str> {
    if !recurring
        .account
        .is_some_and(|id| bank::is_account(app, id))
    {
        return Err("account");
    }
    if recurring.schedule.is_empty() {
        return Err("schedule");
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
/// `set_transaction_recurring_payment`'s args.
#[derive(Deserialize)]
pub(crate) struct TransactionsRecurringPayment {
    transactions: Vec<TransactionRef>,
    to: Option<Id>,
}

// ----------------------------------------------------------------------------
/// `set_transaction_recurring_payment {transactions: [{id, transaction}], to}`: the transactions
/// named join the recurring payment `to`, or leave theirs with `null`
/// (`Index::set_transaction_recurring_payment`); each statement is written once.
/// Answered as the first statement's change.
pub fn set_transaction_recurring_payment(
    app: &mut App,
    req: Req,
    set: TransactionsRecurringPayment,
) {
    let statements = bank::by_statement(set.transactions.iter().map(|b| (b.id, b.transaction)));
    let first = statements.first().map(|(id, _)| *id);
    let step = Text::new("undo.set_transaction_recurring_payment").arg("n", set.transactions.len());
    let result = app.step(&req, step, |index, changes| -> Result<SaveOutcome> {
        let mut outcome = SaveOutcome::Unchanged;
        for (id, transactions) in &statements {
            outcome = outcome.and(index.set_transaction_recurring_payment(
                id,
                transactions,
                set.to,
                changes,
            )?);
            if matches!(outcome, SaveOutcome::Conflict(_)) {
                break;
            }
        }
        Ok(outcome)
    });
    structure::answer(app, req, result, first);
}

// ----------------------------------------------------------------------------
/// Put the new transactions `transactions` of a statement of `account` that have no
/// recurring payment into the recurring payments they pay (`join`); how many joined.
pub(crate) fn join_new(app: &App, account: Id, transactions: &mut [Transaction]) -> usize {
    let Some(newest) = transactions.iter().filter_map(Transaction::day).max() else {
        return 0;
    };
    let last = Date::new(newest.days() + AHEAD);
    let mut payments = payments_of(app.vault.index(), account, last);
    join(&mut payments, transactions)
}

// ----------------------------------------------------------------------------
/// The recurring payments of `account` as `join` reads them, their occurrences up to
/// `last`; a recurring payment whose schedule `occurrences` doesn't read is left out.
fn payments_of(index: &Index, account: Id, last: Date) -> Vec<Joinable> {
    let entries = index.all();
    let transactions: Vec<&Transaction> = index
        .statements_of(account)
        .flat_map(|(_, statement)| &statement.transactions)
        .collect();
    let name = |party: &ContactRef| match party {
        ContactRef::Name(name) => Some(name.trim().to_lowercase()),
        ContactRef::Id(id) => index.get(id).map(|doc| doc.title().trim().to_lowercase()),
    };
    entries
        .iter()
        .filter_map(|e| {
            let recurring = e.doc.roles().recurring_payment.as_ref()?;
            if recurring.account != Some(account) {
                return None;
            }
            let id = e.doc.id();
            let days = days_of(recurring, last)?;
            let own: Vec<&&Transaction> = transactions
                .iter()
                .filter(|b| b.recurring_payment == Some(id))
                .collect();
            let parties = own.iter().filter_map(|b| b.party());
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
            let posted: Vec<Date> = own.iter().filter_map(|b| b.day()).collect();
            Some(Joinable::new(id, names, amounts, days, &posted))
        })
        .collect()
}

// ----------------------------------------------------------------------------
/// The days of `recurring`'s schedule up to `last`: of its one RRULE line;
/// `None` for a schedule of more lines.
fn days_of(recurring: &RecurringPayment, last: Date) -> Option<Vec<Date>> {
    let until = recurring.until.map(|until| until.date);
    occurrences(&recurring.schedule, recurring.from?.date, until, last)
}

// ----------------------------------------------------------------------------
/// A recurring payment as `join` reads it.
#[derive(Debug)]
pub(crate) struct Joinable {
    id: Id,
    /// Its party's name and its transactions' parties, lowercase.
    names: Vec<String>,
    /// Its amounts in cents from their days on, oldest first.
    amounts: Vec<(Date, i64)>,
    /// Its occurrences.
    days: Vec<Date>,
    /// The occurrences its transactions pay.
    paid: Vec<Date>,
}

impl Joinable {
    // ------------------------------------------------------------------------
    /// The recurring payment `id`, each of its transactions `posted` on a day paying the
    /// occurrence it is nearest to.
    fn new(
        id: Id,
        names: Vec<String>,
        amounts: Vec<(Date, i64)>,
        days: Vec<Date>,
        posted: &[Date],
    ) -> Joinable {
        let mut payment = Joinable {
            id,
            names,
            amounts,
            days,
            paid: Vec::new(),
        };
        let mut posted = posted.to_vec();
        posted.sort_unstable();
        for day in posted {
            if let Some(occurrence) = payment.open_near(day) {
                payment.paid.push(occurrence);
            }
        }
        payment
    }

    // ------------------------------------------------------------------------
    /// Its amount on `day`: the last one from that day or before, else the
    /// first.
    fn amount_on(&self, day: Date) -> Option<i64> {
        let current = self.amounts.iter().rev().find(|(from, _)| *from <= day);
        current.or(self.amounts.first()).map(|(_, cents)| *cents)
    }

    // ------------------------------------------------------------------------
    /// The occurrence nearest to `day` that no transaction pays yet, within its
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
    /// How many days from its `i`th occurrence a transaction still pays it.
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
/// Put each transaction of `transactions` without a recurring payment, in their order (a
/// statement's: oldest first), into the recurring payment it pays: on whose party it
/// names (case aside), going the same way, in or out, and near an
/// occurrence no transaction pays yet (`Joinable::open_near`). Of several, the
/// one whose amount is nearest, then whose occurrence is. The amount
/// doesn't decide: one off the recurring payment's is off on its card. How many joined.
pub(crate) fn join(payments: &mut [Joinable], transactions: &mut [Transaction]) -> usize {
    let mut joined = 0;
    for transaction in transactions.iter_mut() {
        if transaction.recurring_payment.is_some() {
            continue;
        }
        let (Some(day), Some(cents)) = (transaction.day(), transaction.cents()) else {
            continue;
        };
        let Some(ContactRef::Name(party)) = transaction.party() else {
            continue;
        };
        let party = party.trim().to_lowercase();
        let mut best: Option<(usize, Date, (i64, i32))> = None;
        for (s, one) in payments.iter().enumerate() {
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
            payments[s].paid.push(occurrence);
            transaction.recurring_payment = Some(payments[s].id);
            joined += 1;
        }
    }
    joined
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, parse, reply, shape, shown, undoes};
    use av_utils::datetime::DateTime;
    use serde_json::Value;
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
            &[format!("RRULE:{rule}")],
            day(from),
            until.map(day),
            day(last),
        )
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_schedule_is_read_with_its_exdates() {
        let recurring = |every: &[&str]| RecurringPayment {
            schedule: every.iter().map(|line| line.to_string()).collect(),
            from: Some(DateTime::from_iso8601("2026-12-01").unwrap()),
            ..RecurringPayment::default()
        };
        let last = day("2027-01-31");
        let one = recurring(&["RRULE:FREQ=MONTHLY;BYMONTHDAY=1"]);
        assert_eq!(
            days_of(&one, last),
            Some(days(&["2026-12-01", "2027-01-01"]))
        );
        let two = recurring(&["RRULE:FREQ=MONTHLY;BYMONTHDAY=1", "EXDATE:20261201"]);
        assert_eq!(days_of(&two, last), Some(days(&["2027-01-01"])));
    }

    // ------------------------------------------------------------------------
    fn transaction(on: &str, amount: f64, party: &str) -> Transaction {
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

    fn monthly(id: Id, names: &[&str], amount: i64, posted: &[&str]) -> Joinable {
        let firsts = every(
            "FREQ=MONTHLY;BYMONTHDAY=1",
            "2026-08-01",
            None,
            "2027-03-01",
        )
        .unwrap();
        let names = names.iter().map(|n| n.to_lowercase()).collect();
        Joinable::new(
            id,
            names,
            vec![(day("2026-08-01"), amount)],
            firsts,
            &days(posted),
        )
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_transaction_joins_the_recurring_payment_it_pays() {
        let rent = Id::generate();
        let mut payments = [monthly(rent, &["Shady Pines"], -115_000, &["2026-08-01"])];
        let mut transactions = [
            transaction("2026-09-02", -1150.0, "SHADY PINES"),
            transaction("2026-09-03", -75.0, "Shady Pines"),
            transaction("2026-09-15", -1150.0, "Shady Pines"),
            transaction("2026-10-01", 375.0, "Shady Pines"),
            transaction("2026-08-02", -1150.0, "Shady Pines"),
            transaction("2026-11-01", -1150.0, "Shady Pines Ltd."),
        ];
        assert_eq!(join(&mut payments, &mut transactions), 1);
        let joined: Vec<bool> = transactions
            .iter()
            .map(|b| b.recurring_payment == Some(rent))
            .collect();
        assert_eq!(
            joined,
            [true, false, false, false, false, false],
            "the case aside; the occurrence paid; a week away; money in; August paid; another name"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_name_its_transactions_bring_is_known_and_the_nearest_amount_wins() {
        let (salary, bonus) = (Id::generate(), Id::generate());
        let mut payments = [
            monthly(salary, &["Cogsworth", "Cogsworth Inc."], 187_246, &[]),
            monthly(bonus, &["Cogsworth Inc."], 50_000, &[]),
        ];
        let mut transactions = [
            transaction("2026-12-01", 1872.46, "Cogsworth Inc."),
            transaction("2026-12-02", 500.0, "Cogsworth Inc."),
        ];
        assert_eq!(join(&mut payments, &mut transactions), 2);
        assert_eq!(transactions[0].recurring_payment, Some(salary));
        assert_eq!(transactions[1].recurring_payment, Some(bonus));

        let taken = Transaction {
            recurring_payment: Some(bonus),
            ..transaction("2027-01-01", 1872.46, "Cogsworth")
        };
        let mut transactions = [taken];
        assert_eq!(
            join(&mut payments, &mut transactions),
            0,
            "it has its recurring payment"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn twice_a_month_each_transaction_pays_its_own_day() {
        let id = Id::generate();
        let twice = every(
            "FREQ=MONTHLY;BYMONTHDAY=15,-1",
            "2026-12-15",
            None,
            "2027-02-28",
        )
        .unwrap();
        let mut payments = [Joinable::new(
            id,
            vec!["cogsworth".into()],
            vec![(day("2026-12-15"), 187_246)],
            twice,
            &[],
        )];
        assert_eq!(payments[0].reach(0), 7);
        let mut transactions = [
            transaction("2026-12-15", 1872.46, "Cogsworth"),
            transaction("2026-12-22", 1872.46, "Cogsworth"),
            transaction("2026-12-31", 1872.46, "Cogsworth"),
        ];
        assert_eq!(join(&mut payments, &mut transactions), 2);
        let joined: Vec<bool> = transactions
            .iter()
            .map(|b| b.recurring_payment.is_some())
            .collect();
        assert_eq!(joined, [true, false, true], "a week from either day");
        assert_eq!(payments[0].paid, days(&["2026-12-15", "2026-12-31"]));
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
                    "id: {STATEMENT}\ntitle: September\nstatement:\n  account: {ACCOUNT}\n  transactions:\n  - id: {RENT}\n    date: 2026-09-01\n    amount: -1150.0\n    receiver: Shady Pines\n  - id: {FEE}\n    date: 2026-09-02\n    amount: -4.9\n"
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
            "schedule": ["RRULE:FREQ=MONTHLY;BYMONTHDAY=1"],
            "from": "2026-09-01",
            "amounts": [{ "from": "2026-09-01", "amount": -1150.0 }],
            "category": "housing",
        })
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_recurring_payment_is_made_in_accounts_and_its_transactions_join_it() {
        let (mut h, lease) = with_account("recurring-create");
        let args =
            json!({ "title": "Rent", "recurring_payment": rent(ACCOUNT), "linked_to": lease });
        let sent = undoes(&mut h, "create_recurring_payment", args);
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let id = Id::parse(sent[1]["ok"]["id"].as_str().unwrap()).unwrap();
        let entry = h.app.vault.index().entry(&id).unwrap();
        let path = entry.local.to_string_lossy().replace('\\', "/");
        assert!(path.starts_with("Accounts/20260901T0000.rent."), "{path}");
        assert!(path.ends_with(".rcp.yaml"), "{path}");
        assert_eq!(entry.doc.linked_to(), Some(lease));

        let transactions = json!([{ "id": STATEMENT, "transaction": RENT }]);
        let sent = undoes(
            &mut h,
            "set_transaction_recurring_payment",
            json!({ "transactions": transactions, "to": id }),
        );
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let text = fs::read_to_string(h.vault().join("Accounts/sep.txs.yaml")).unwrap();
        assert!(text.contains(&format!("recurring-payment: {id}")), "{text}");

        let mut changed = rent(ACCOUNT);
        changed["amounts"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "from": "2027-01-01", "amount": -1190.0 }));
        let sent = undoes(
            &mut h,
            "set_recurring_payment",
            json!({ "id": id, "recurring_payment": changed }),
        );
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let doc = h.app.vault.index().get(&id).unwrap();
        assert_eq!(
            doc.roles()
                .recurring_payment
                .as_ref()
                .unwrap()
                .amounts
                .len(),
            2
        );

        let sent = h.call(
            "set_transaction_recurring_payment",
            json!({ "transactions": transactions, "to": null }),
        );
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let text = fs::read_to_string(h.vault().join("Accounts/sep.txs.yaml")).unwrap();
        assert!(!text.contains("recurring-payment"), "{text}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_recurring_payment_made_with_its_transactions_is_one_step() {
        let (mut h, _) = with_account("recurring-create-step");
        let before = h.files();
        let send = |h: &mut Harness, id: u32, cmd: &str, args: Value| {
            let request =
                json!({ "id": id, "cmd": cmd, "args": args, "step": "recurring-payment:1" });
            parse(&h.send(&request.to_string()))
        };
        let args = json!({ "title": "Rent", "recurring_payment": rent(ACCOUNT) });
        let sent = send(&mut h, 1, "create_recurring_payment", args);
        let id = reply(&sent)["ok"]["id"].clone();
        let transactions = json!([{ "id": STATEMENT, "transaction": RENT }]);
        let args = json!({ "transactions": transactions, "to": id });
        send(&mut h, 2, "set_transaction_recurring_payment", args);

        let sent = parse(&h.send(r#"{"id": 3, "cmd": "undo"}"#));
        let notice = sent.iter().find(|m| m["ev"] == "notice").unwrap();
        assert_eq!(shown(&notice["data"]["text"]), "Undone: Created Rent");
        assert_eq!(h.files(), before, "the payment and its transaction's join");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_recurring_payment_needs_its_account_schedule_start_and_amount() {
        let (mut h, _) = with_account("recurring-refused");
        // the reply's message names the argument
        let refused = |h: &mut Harness, args: serde_json::Value, arg: &str| {
            let sent = h.call("create_recurring_payment", args);
            assert_eq!(sent[0]["err"]["code"], "invalid_argument", "{arg}");
            assert_eq!(sent[0]["err"]["text"]["args"]["arg"], arg, "{arg}");
        };
        for (key, value) in [
            ("account", json!(STATEMENT)),
            ("schedule", json!([])),
            ("from", json!(null)),
            ("amounts", json!([])),
            ("vary", json!(-1.0)),
        ] {
            let mut recurring = rent(ACCOUNT);
            recurring[key] = value;
            refused(
                &mut h,
                json!({ "title": "Rent", "recurring_payment": recurring }),
                key,
            );
        }
        refused(
            &mut h,
            json!({ "title": " ", "recurring_payment": rent(ACCOUNT) }),
            "title",
        );
        let unlinked = json!({ "title": "Rent", "recurring_payment": rent(ACCOUNT), "linked_to": Id::generate() });
        refused(&mut h, unlinked, "linked_to");
        let other = json!({ "id": STATEMENT, "recurring_payment": rent(ACCOUNT) });
        let sent = h.call("set_recurring_payment", other);
        assert_eq!(sent[0]["err"]["code"], "not_found");
        assert!(
            h.app
                .vault
                .index()
                .all()
                .iter()
                .all(|e| e.doc.roles().recurring_payment.is_none())
        );
    }
}
