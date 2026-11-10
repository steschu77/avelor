use super::comdirect::{Export, Row, cells, cents, german_day, joined, read};
use super::plan::{Known, OtherAccount, Pair, Plan, pairs, plan};
use super::*;
use crate::testing::{Harness, parse, reply, shape, shown, undoes};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// The forged vault's December statement, and what it was written from.
const DECEMBER: &[u8] = include_bytes!("../../../data/statements/statement_2026-12.csv");
const DECEMBER_JSON: &str = include_str!("../../../data/statements/statement_2026-12.json");
const ACCOUNT: &str = "01955b3b-ca80-7fef-afbe-46d15556d9a1";
const WATCH: &str = "01a0e1cc-a24c-7490-82fe-e75669985b60";

fn day(text: &str) -> Date {
    DateTime::from_iso8601(text).unwrap().date
}

fn december() -> Export {
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
    let pending = statement.rows.iter().filter(|r| r.posted.is_none()).count();
    assert_eq!(pending, 2);
    let posted: i64 = statement
        .rows
        .iter()
        .filter(|r| r.posted.is_some())
        .map(|r| r.cents)
        .sum();
    assert_eq!(3_205_928 + posted, 3_247_422);
}

// ------------------------------------------------------------------------
#[test]
fn rows_say_who_what_and_why() {
    let statement = december();
    let row = |cents: i64| statement.rows.iter().find(|r| r.cents == cents).unwrap();

    let donuts = &statement.rows[0];
    assert_eq!(donuts.posted, None);
    assert_eq!(donuts.own.as_deref(), Some("004273185520"));
    assert_eq!(donuts.merchant.as_deref(), Some("Big Dough Donuts"));
    assert_eq!(donuts.card_day, Some(day("2026-12-31")));
    assert_eq!(
        (donuts.tx_type, donuts.reference.as_deref()),
        (Some("card"), None)
    );

    let critters = row(-9500);
    assert_eq!(critters.posted, Some(day("2026-12-16")));
    assert_eq!(critters.party.as_deref(), Some("Critter Busters LLC"));
    assert_eq!(critters.merchant.as_deref(), Some("Critter Busters"));
    assert_eq!(
        critters.memo.as_deref(),
        Some("Critter Busters//Springfield/US")
    );
    assert_eq!(critters.card_day, Some(day("2026-12-15")));
    assert_eq!(
        critters.reference.as_deref(),
        Some("LVDQ2A4MX70PBBTP/360902")
    );

    let house = row(-50000);
    assert_eq!(house.tx_type, Some("transfer"));
    assert_eq!(house.party.as_deref(), Some("Alexander and Maya Yahnek"));
    assert_eq!(
        (house.iban.as_deref(), house.bic.as_deref()),
        (Some("004273185539"), Some("FBSPUS44XXX"))
    );
    assert_eq!(house.memo.as_deref(), Some("House fund, starting over"));

    let fee = row(-190);
    assert_eq!((fee.tx_type, fee.party.as_deref()), (Some("fee"), None));
    let memo = "Entgelt Visa-Kreditkarte Zeitraum: 01.11.2026 bis 30.11.2026";
    assert_eq!(fee.memo.as_deref(), Some(memo));
    assert_eq!(fee.posted, Some(day("2026-12-02")));

    assert_eq!(row(-20000).tx_type, Some("cash"));
    assert_eq!(row(-192_735).tx_type, Some("direct-debit"));
}

// ------------------------------------------------------------------------
#[test]
fn memos_lose_the_banks_line_breaks() {
    let statement = december();
    let json: Value = serde_json::from_str(DECEMBER_JSON).unwrap();
    let mut checked = 0;
    for tx in json["transactions"].as_array().unwrap() {
        let kind = tx["kind"].as_str().unwrap();
        let Some(memo) = tx["purpose"].as_str() else {
            continue;
        };
        if !["direct-debit", "transfer-in", "transfer-out"].contains(&kind) {
            continue;
        }
        let cents = to_cents(tx["amount"].as_f64().unwrap());
        let row = |r: &&Row| r.cents == cents && r.memo.as_deref() == Some(memo);
        assert!(statement.rows.iter().any(|r| row(&r)), "{kind}: {memo}");
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
    assert_eq!(not.render(), "It isn't a statement this watch reads.");
    let head = r#""Buchungstag";"Wertstellung (Valuta)";"Vorgang";"Buchungstext";"Umsatz in EUR";"#;
    let broken = format!("{head}\n\"01.13.2026\";\"--\";\"x\";\"y\";\"1,00\";\n");
    assert_eq!(
        read(broken.as_bytes()).unwrap_err().render(),
        "Line 2 can't be read."
    );
}

// ------------------------------------------------------------------------
/// A posted row of `cents` on `posted`, with `reference`.
fn posted(posted: &str, cents: i64, reference: &str) -> Row {
    Row {
        posted: Some(day(posted)),
        vorgang: "Lastschrift / Belastung".into(),
        reference: some(reference),
        memo: Some(format!("memo {reference}")),
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
        balance: plan.statement.balance.map(to_cents),
        items: plan.statement.transactions,
    }
}

// ------------------------------------------------------------------------
#[test]
fn a_first_statement_brings_every_transaction() {
    let holder = ContactRef::Name("Alexander Yahnek".into());
    let plan = plan(Some(&holder), &[], &december());
    assert!(plan.newest);
    assert_eq!((plan.skipped, plan.problems.len()), (0, 0));
    assert_eq!(plan.statement.transactions.len(), 30);
    assert_eq!(plan.statement.amount, Some(414.94));
    assert_eq!(plan.statement.balance, Some(32474.22));
    assert_eq!(
        (plan.first, plan.last),
        (Some(day("2026-12-01")), Some(day("2026-12-31")))
    );
    let items = &plan.statement.transactions;
    let first = &items[0];
    assert_eq!(first.title.as_deref(), Some("First Bank of Springfield"));
    assert_eq!(first.sender, Some(holder.clone()), "money out");
    assert_eq!(first.amount, Some(-1927.35));
    let pay = items.iter().find(|t| t.amount == Some(794.88)).unwrap();
    let party = ContactRef::Name("Springfield Power Station".into());
    assert_eq!(
        (pay.sender.as_ref(), pay.receiver.as_ref()),
        (Some(&party), Some(&holder))
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
fn a_transaction_is_imported_once() {
    let first = plan(None, &[], &december());
    let known = [known_of(first)];
    let again = plan(None, &known, &december());
    assert_eq!(again.skipped, 28, "the posted ones");
    assert_eq!(again.statement.transactions.len(), 2, "the pending ones");
    assert_eq!(again.statement.amount, Some(0.0));
    let unreferenced = Export {
        rows: vec![posted("2026-12-02", -500, "")],
        ..Export::default()
    };
    let known = [known_of(plan(None, &[], &unreferenced))];
    let again = plan(None, &known, &unreferenced);
    assert_eq!(again.skipped, 1, "the same day, amount and memo");
}

// ------------------------------------------------------------------------
#[test]
fn the_newest_statement_replaces_the_pending_transactions() {
    let older = Export {
        rows: vec![
            pending("2026-12-30", -5995),
            posted("2026-12-28", -1599, "R1"),
        ],
        ..Export::default()
    };
    let mut first = known_of(plan(None, &[], &older));
    let (bill, recurring_payment) = (Id::generate(), Id::generate());
    first.items[1].linked_to = Some(bill);
    first.items[1].recurring_payment = Some(recurring_payment);
    let pending_id = first.items[1].id;
    let first_id = first.id;

    let mut paid = posted("2027-01-02", -5995, "R2");
    paid.card_day = Some(day("2026-12-30"));
    let newer = Export {
        rows: vec![
            pending("2027-01-03", -700),
            paid,
            posted("2026-12-28", -1599, "R1"),
        ],
        ..Export::default()
    };
    let known = [first];
    let next = plan(None, &known, &newer);
    assert!(next.newest);
    assert_eq!(next.skipped, 1, "R1 is there");
    let twin = next
        .statement
        .transactions
        .iter()
        .find(|t| t.reference.as_deref() == Some("R2"));
    let twin = twin.unwrap();
    assert_eq!(
        (twin.id, twin.linked_to, twin.recurring_payment),
        (pending_id, Some(bill), Some(recurring_payment)),
        "the pending one, posted"
    );
    assert_eq!(next.replaced.len(), 1);
    let (id, kept) = &next.replaced[0];
    assert_eq!(*id, first_id);
    assert_eq!(kept.len(), 1, "only the posted one stays");
    assert!(kept.iter().all(|t| !t.is_pending()));

    let late = plan(None, &[known_of(next)], &older);
    assert!(!late.newest);
    assert!(late.replaced.is_empty());
    assert!(late.statement.transactions.iter().all(|t| !t.is_pending()));
}

// ------------------------------------------------------------------------
#[test]
fn balances_that_dont_add_up_are_named() {
    let november = Export {
        opening: Some(10_000),
        closing: Some(9_000),
        rows: vec![posted("2026-11-20", -1_000, "N1")],
        ..Export::default()
    };
    let known = [known_of(plan(None, &[], &november))];
    let december = Export {
        opening: Some(8_000),
        closing: Some(7_500),
        rows: vec![posted("2026-12-03", -1_000, "D1")],
        ..Export::default()
    };
    let next = plan(None, &known, &december);
    let problems: Vec<String> = next.problems.iter().map(Text::render).collect();
    assert_eq!(
        problems,
        [
            "its transactions don't add up to its balances",
            "transactions are missing between 2026-11-20 and 2026-12-03",
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
/// A posted transaction of `cents` on `day`, to `iban` if given.
fn half(day: &str, cents: i64, iban: Option<&str>) -> Transaction {
    Transaction {
        id: Some(Id::generate()),
        date: Some(DateTime::from_iso8601(day).unwrap()),
        status: Some("posted".into()),
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
        transaction: 0,
        statement: others[1].statements[0].id,
        other: nearest.id.unwrap(),
    };
    assert_eq!(pairs(Some("0042-7318-5520"), &out, &others), [pair]);

    let sent = half("2026-12-11", -50_000, SAVINGS_IBAN);
    let checking = [other("0042-7318-5520", vec![sent.clone()])];
    let into = [half("2026-12-12", 50_000, None)];
    let found = pairs(Some("0042-7318-5539"), &into, &checking);
    assert_eq!(found.len(), 1, "money in, from a transaction naming it");
    assert_eq!(found[0].other, sent.id.unwrap());
    assert!(pairs(Some("0042-7318-5547"), &into, &checking).is_empty());
    assert!(pairs(None, &into, &checking).is_empty(), "no number");
}

// ------------------------------------------------------------------------
#[test]
fn a_transfer_pairs_once_within_its_days() {
    let savings = |items: Vec<Transaction>| [other("0042-7318-5539", items)];
    let none = |transactions: &[Transaction], items: Vec<Transaction>| {
        pairs(None, transactions, &savings(items)).is_empty()
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
        "id: {ACCOUNT}\ntitle: Checking account\ncreated: {{}}\nbank-account:\n  type: checking\n  number: {number}\n  holder: Alexander Yahnek\n"
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
    h.wait_indexed();
    h.take();
    (h, folder)
}

/// Import the watch's statements; the job's summary.
fn import(h: &mut Harness) -> String {
    import_from(h, &json!(WATCH))
}

/// Import the statements of the watch `watch`; the job's summary.
fn import_from(h: &mut Harness, watch: &Value) -> String {
    let done = |m: &String| m.contains(r#""state":"done""#);
    let args = json!({ "watch": watch });
    let sent = h.call_until("statements_import", args, |sent| sent.iter().any(done));
    let job = reply(&parse(&sent))["ok"]["job"].clone();
    assert!(job.is_string(), "{sent:?}");
    let ended = sent.iter().find(|m| done(m)).unwrap();
    let ended: Value = serde_json::from_str(ended).unwrap();
    assert_eq!(ended["data"]["id"], job);
    shown(&ended["data"]["summary"]).to_string()
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
    assert_eq!(note, "1 statement \u{b7} 28 transactions \u{b7} 2 pending");
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
    let again = reply(&sent)["ok"]["id"].clone();
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
    let posted: Vec<&[u8]> = DECEMBER
        .split(|b| *b == b'\n')
        .filter(|line| !line.starts_with(b"\"offen\""))
        .collect();
    fs::write(folder.join("copy_2026-12.csv"), posted.join(&b'\n')).unwrap();
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
        "id: {statement}\ntitle: House fund, December 2026\nstatement:\n  account: {SAVINGS}\n  transactions:\n  - id: {half}\n    title: Transfer from checking\n    date: 2026-12-12\n    status: posted\n    amount: 500.0\n"
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
    let items = &doc.roles().statement.as_ref().unwrap().transactions;
    let half = |item: &Transaction| Some((item.id?, item.transfer?));
    items.iter().filter_map(half).collect()
}

// ------------------------------------------------------------------------
#[test]
fn a_transfer_is_paired_with_the_other_accounts_half() {
    let (mut h, fund, half) = with_house_fund("statements-pair");
    assert_eq!(
        import(&mut h),
        "1 statement \u{b7} 28 transactions \u{b7} 2 pending \u{b7} 1 transfer paired"
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

    let unpaired = undoes(
        &mut h,
        "unpair_transfer",
        json!({ "id": checking, "transaction": sent }),
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
        "1 statement \u{b7} 28 transactions \u{b7} 2 pending"
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
/// The forged vault's checking statements and recurring payments, each
/// a path in the vault and its text.
fn forged_checking() -> Vec<(String, String)> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/forged/Accounts");
    let mut files = Vec::new();
    for entry in fs::read_dir(&folder).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.contains(".checking-account-") || name.ends_with(".rcp.yaml") {
            let text = fs::read_to_string(&path).unwrap();
            files.push((format!("Accounts/{name}"), text));
        }
    }
    files
}

// ------------------------------------------------------------------------
#[test]
fn decembers_transactions_join_the_forged_recurring_payments() {
    let forged = forged_checking();
    let more: Vec<(&str, String)> = forged
        .iter()
        .map(|(path, text)| (path.as_str(), text.clone()))
        .collect();
    let files = [("statement_2026-12.csv", DECEMBER)];
    let (mut h, _) = with_vault("statements-recurring", "0042-7318-5520", &files, &more);
    assert_eq!(
        import(&mut h),
        "1 statement \u{b7} 28 transactions \u{b7} 2 pending \u{b7} 4 in recurring payments"
    );
    let joined = |h: &Harness| {
        let index = h.app.vault.index();
        let december = statements(h).into_iter().max_by_key(|s| {
            let doc = index.get(&s.id).unwrap();
            doc.date()
        });
        let december = december.unwrap();
        let mut found = Vec::new();
        for item in &december.items {
            let Some(recurring_payment) = item.recurring_payment else {
                continue;
            };
            let (_, _, day) = item.day().unwrap().to_ymd();
            found.push((
                day,
                index.get(&recurring_payment).unwrap().title().to_string(),
                item.id,
            ));
        }
        found.sort();
        (december.id, found)
    };
    let (december, found) = joined(&h);
    let titles: Vec<(i32, &str)> = found.iter().map(|(d, t, _)| (*d, t.as_str())).collect();
    assert_eq!(
        titles,
        [
            (1, "Mortgage"),
            (10, "Springfield Power & Light"),
            (15, "Salary, Cogsworth"),
            (31, "Salary, Cogsworth"),
        ],
        "not the final pay after the plant's last day, nor the bank's cash on the 24th"
    );

    let power = found[1].2.unwrap();
    let out = json!({ "transactions": [{ "id": december, "transaction": power }], "to": null });
    h.call("set_transaction_recurring_payment", out);
    assert_eq!(import(&mut h), "No new statements");
    assert_eq!(joined(&h).1.len(), 3, "taken out by hand, it stays out");
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
        "bank": "First Bank of Springfield",
    });
    let sent = h.call("create_watch", new);
    assert_eq!(shape(&sent), ["entity", "watch", "reply"]);
    let account = &sent[0]["data"];
    assert_eq!(account["kind"], "bank-account");
    assert_eq!(account["title"], "House fund");
    assert_eq!(
        account["bank-account"],
        json!({ "type": "savings", "number": "0042-7318-5539", "bank": "First Bank of Springfield" })
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
        ("type", "loan", "Choose the account's type."),
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
        assert_eq!(shown(&sent[0]["err"]["text"]), message);
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
        shown(&sent[0]["err"]["text"]),
        "Choose the folder of the account's statements first."
    );
    let sent = h.call("set_watch", json!({ "watch": watch, "folder": folder }));
    assert_eq!(shape(&sent), ["watch", "reply"]);
    let done = |m: &String| m.contains(r#""state":"done""#);
    let args = json!({ "watch": watch });
    let sent = h.call_until("statements_import", args, |sent| sent.iter().any(done));
    assert!(reply(&parse(&sent))["ok"]["job"].is_string(), "{sent:?}");
    assert_eq!(statements(&h).len(), 1);
}

// ------------------------------------------------------------------------
#[test]
fn set_bank_account_changes_the_fields_given() {
    let (mut h, _) = with_statements("statements-account", "0042-7318-5520", &[]);
    let args = json!({ "entity": ACCOUNT, "bank": "First Bank of Springfield", "number": " ", "type": "savings" });
    let sent = undoes(&mut h, "set_bank_account", args);
    assert_eq!(shape(&sent), ["entity", "reply"]);
    let fields = account(&h);
    assert_eq!(fields.bank.as_deref(), Some("First Bank of Springfield"));
    assert_eq!(fields.number, None, "an empty one removes it");
    assert_eq!(fields.account_type.as_deref(), Some("savings"));
    assert!(fields.holder.is_some(), "the others stay");
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
