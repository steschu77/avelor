//! Recurrence rules (RFC 5545): the days an RRULE line gives, as far as a
//! recurring payment's schedule goes (weekly with `BYDAY`, monthly with
//! `INTERVAL` and `BYMONTHDAY`, yearly with `BYMONTH` and `BYMONTHDAY`,
//! `COUNT`, `UNTIL`). The page expands an event's rule itself.

use crate::datetime::{Date, Month, days_in_month};

const WEEKDAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
enum Freq {
    Weekly,
    Monthly,
    Yearly,
}

/// An RRULE line, as far as `occurrences` reads it.
#[derive(Debug)]
struct Rule {
    freq: Freq,
    interval: i32,
    /// `BYMONTHDAY`: a day of the month, counted back from its end when
    /// negative.
    month_days: Vec<i32>,
    /// `BYDAY` of a weekly rule, Monday 0.
    week_days: Vec<i32>,
    /// `BYMONTH` of a yearly rule.
    months: Vec<Month>,
    count: Option<usize>,
    until: Option<Date>,
}

// ----------------------------------------------------------------------------
/// The rule of `line`: weekly, monthly or yearly periods, with `INTERVAL`,
/// `BYMONTHDAY`, a weekly rule's `BYDAY`, a yearly rule's `BYMONTH`,
/// `COUNT`, `UNTIL` and `WKST=MO`; `None` for anything else.
fn rule(line: &str) -> Option<Rule> {
    let mut rule = Rule {
        freq: Freq::Monthly,
        interval: 1,
        month_days: Vec::new(),
        week_days: Vec::new(),
        months: Vec::new(),
        count: None,
        until: None,
    };
    let mut freq = None;
    for part in line.strip_prefix("RRULE:")?.split(';') {
        let (key, value) = part.split_once('=')?;
        let numbers = || -> Option<Vec<i32>> { value.split(',').map(|n| n.parse().ok()).collect() };
        match key {
            "FREQ" => {
                freq = Some(match value {
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                });
            }
            "INTERVAL" => rule.interval = value.parse().ok().filter(|n| *n > 0)?,
            "BYMONTHDAY" => rule.month_days = numbers()?,
            "BYMONTH" => {
                let months = numbers()?.into_iter().map(|m| Month::try_from(m).ok());
                rule.months = months.collect::<Option<_>>()?;
            }
            "BYDAY" => {
                let day = |code| WEEKDAYS.iter().position(|d| *d == code);
                let days: Option<Vec<usize>> = value.split(',').map(day).collect();
                rule.week_days = days?.into_iter().map(|d| d as i32).collect();
            }
            "COUNT" => rule.count = Some(value.parse().ok()?),
            "UNTIL" => {
                let digits = |range: std::ops::Range<usize>| value.get(range)?.parse().ok();
                let date = Date::from_ymd(digits(0..4)?, digits(4..6)?, digits(6..8)?);
                rule.until = Some(date.ok()?);
            }
            "WKST" if value == "MO" => {}
            _ => return None,
        }
    }
    rule.freq = freq?;
    let weekly = rule.freq == Freq::Weekly;
    let fits = (weekly || rule.week_days.is_empty())
        && (!weekly || rule.month_days.is_empty())
        && (rule.freq == Freq::Yearly || rule.months.is_empty());
    fits.then_some(rule)
}

// ----------------------------------------------------------------------------
/// The days of the RRULE `line` from `from` on, to `until` and `last`;
/// `None` for a rule `rule` doesn't read. Without `BYMONTHDAY`, `BYDAY` or
/// `BYMONTH`, `from`'s day, weekday or month; a day a month doesn't have is
/// left out.
pub fn occurrences(line: &str, from: Date, until: Option<Date>, last: Date) -> Option<Vec<Date>> {
    let rule = rule(line)?;
    let last = [Some(last), until, rule.until]
        .into_iter()
        .flatten()
        .min()?;
    let (year, month, day) = from.to_ymd();
    let weekday = i32::from(from.weekday());
    let mut out = Vec::new();
    for period in 0.. {
        let step = period * rule.interval;
        let mut days: Vec<Date> = match rule.freq {
            Freq::Weekly => {
                let monday = from.days() - weekday + step * 7;
                if monday > last.days() {
                    break;
                }
                let week = or_one(&rule.week_days, weekday);
                week.iter().map(|d| Date::new(monday + d)).collect()
            }
            Freq::Monthly => {
                let (year, month) = months_on(year, month, step);
                if Date::from_ymd(year, i32::from(month), 1).ok()? > last {
                    break;
                }
                month_days(year, month, &rule.month_days, day)
            }
            Freq::Yearly => {
                let year = year + step;
                if Date::from_ymd(year, 1, 1).ok()? > last {
                    break;
                }
                let months = if rule.months.is_empty() {
                    vec![month]
                } else {
                    rule.months.clone()
                };
                let days = months
                    .into_iter()
                    .map(|month| month_days(year, month, &rule.month_days, day));
                days.flatten().collect()
            }
        };
        days.sort_unstable();
        for date in days.into_iter().filter(|date| *date >= from) {
            if date > last || rule.count.is_some_and(|count| out.len() >= count) {
                return Some(out);
            }
            out.push(date);
        }
    }
    Some(out)
}

// ----------------------------------------------------------------------------
/// `list`, or `one` alone when it is empty.
fn or_one(list: &[i32], one: i32) -> Vec<i32> {
    if list.is_empty() {
        vec![one]
    } else {
        list.to_vec()
    }
}

// ----------------------------------------------------------------------------
/// The year and month `n` months after `month` of `year`.
fn months_on(year: i32, month: Month, n: i32) -> (i32, Month) {
    let index = i32::from(month) - 1 + n;
    let month = Month::try_from(index.rem_euclid(12) + 1).unwrap_or(Month::Jan);
    (year + index.div_euclid(12), month)
}

// ----------------------------------------------------------------------------
/// The days `by` of `month` of `year` (`day` without any), a negative one
/// counted back from its end; one the month doesn't have left out.
fn month_days(year: i32, month: Month, by: &[i32], day: i32) -> Vec<Date> {
    let length = days_in_month(year, month);
    let days = or_one(by, day)
        .into_iter()
        .map(|d| if d < 0 { length + 1 + d } else { d });
    days.filter_map(|d| Date::from_ymd(year, i32::from(month), d).ok())
        .collect()
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::datetime::DateTime;

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
    fn a_schedule_gives_its_days() {
        let monthly = every(
            "FREQ=MONTHLY;BYMONTHDAY=15",
            "2026-08-15",
            None,
            "2026-11-30",
        );
        let expected = ["2026-08-15", "2026-09-15", "2026-10-15", "2026-11-15"];
        assert_eq!(monthly, Some(days(&expected)));

        let twice = every(
            "FREQ=MONTHLY;BYMONTHDAY=15,-1",
            "2026-12-15",
            None,
            "2027-02-28",
        );
        let expected = [
            "2026-12-15",
            "2026-12-31",
            "2027-01-15",
            "2027-01-31",
            "2027-02-15",
            "2027-02-28",
        ];
        assert_eq!(twice, Some(days(&expected)));

        let quarterly = every("FREQ=MONTHLY;INTERVAL=3", "2026-01-31", None, "2026-12-31");
        let expected = ["2026-01-31", "2026-07-31", "2026-10-31"];
        assert_eq!(quarterly, Some(days(&expected)), "no 31 April");

        let weekly = every("FREQ=WEEKLY;BYDAY=TU,FR", "2026-12-02", None, "2026-12-12");
        let expected = ["2026-12-04", "2026-12-08", "2026-12-11"];
        assert_eq!(weekly, Some(days(&expected)));

        let yearly = every(
            "FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=1",
            "2024-03-01",
            None,
            "2026-06-01",
        );
        let expected = ["2024-03-01", "2025-03-01", "2026-03-01"];
        assert_eq!(yearly, Some(days(&expected)));

        let counted = every(
            "FREQ=MONTHLY;BYMONTHDAY=1;COUNT=2",
            "2026-12-01",
            None,
            "2027-12-31",
        );
        assert_eq!(counted, Some(days(&["2026-12-01", "2027-01-01"])));
        let rule_until = "FREQ=MONTHLY;BYMONTHDAY=1;UNTIL=20270215T235959Z";
        let until = every(rule_until, "2026-12-01", Some("2027-01-31"), "2027-12-31");
        assert_eq!(
            until,
            Some(days(&["2026-12-01", "2027-01-01"])),
            "the earlier end"
        );

        for unread in [
            "FREQ=DAILY",
            "FREQ=MONTHLY;BYDAY=1MO",
            "FREQ=WEEKLY;BYMONTHDAY=1",
            "FREQ=YEARLY;BYMONTH=13",
        ] {
            assert_eq!(
                every(unread, "2026-12-01", None, "2027-01-01"),
                None,
                "{unread}"
            );
        }
        assert_eq!(
            occurrences(
                "EXDATE:20261201",
                day("2026-12-01"),
                None,
                day("2027-01-01")
            ),
            None
        );
    }
}
