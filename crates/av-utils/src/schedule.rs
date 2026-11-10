//! Schedules (RFC 5545): the days a schedule's lines give, read as the page
//! reads them (`model/schedule.js`): an `RRULE` daily, weekly, monthly or
//! yearly, with `INTERVAL`, `COUNT`, `UNTIL`, `BYDAY` (weekdays; with an
//! ordinal such as `2TU` or `-1FR` in a month), `BYMONTHDAY` and
//! `BYMONTH`, and the days its `EXDATE` lines leave out.

use crate::datetime::{Date, Month, days_in_month};

const WEEKDAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];
/// How many periods are read at most, as on the page.
const MAX_PERIODS: i32 = 20000;

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// A `BYDAY` entry: a weekday, Monday 0, and in a month the nth of them,
/// counted from the end when negative; 0 for each.
#[derive(Clone, Copy, Debug)]
struct ByDay {
    weekday: i32,
    nth: i32,
}

/// An RRULE, as far as `occurrences` reads it.
#[derive(Debug)]
struct Rule {
    freq: Freq,
    interval: i32,
    /// `BYMONTHDAY`: a day of the month, counted back from its end when
    /// negative.
    month_days: Vec<i32>,
    by_day: Vec<ByDay>,
    /// `BYMONTH`: the months a day must fall in.
    months: Vec<Month>,
    count: Option<usize>,
    until: Option<Date>,
}

// ----------------------------------------------------------------------------
/// The rule of an RRULE line's `value`; `None` for a part or a value the
/// page doesn't read, and for a yearly rule picking days without months.
fn rule(value: &str) -> Option<Rule> {
    let mut rule = Rule {
        freq: Freq::Monthly,
        interval: 1,
        month_days: Vec::new(),
        by_day: Vec::new(),
        months: Vec::new(),
        count: None,
        until: None,
    };
    let mut freq = None;
    for part in value.split(';') {
        let (key, value) = part.split_once('=')?;
        let numbers = || -> Option<Vec<i32>> { value.split(',').map(|n| n.parse().ok()).collect() };
        match key {
            "FREQ" => {
                freq = Some(match value {
                    "DAILY" => Freq::Daily,
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
            "BYDAY" => rule.by_day = value.split(',').map(by_day).collect::<Option<_>>()?,
            "COUNT" => rule.count = Some(value.parse().ok()?),
            "UNTIL" => rule.until = Some(ical_day(value)?),
            // Weeks start on Monday, whatever it names, as on the page.
            "WKST" => {}
            _ => return None,
        }
    }
    rule.freq = freq?;
    let yearly_without_months = rule.freq == Freq::Yearly
        && rule.months.is_empty()
        && !(rule.by_day.is_empty() && rule.month_days.is_empty());
    (!yearly_without_months).then_some(rule)
}

/// A `BYDAY` entry: `MO`, `2TU`, `-1FR`.
fn by_day(entry: &str) -> Option<ByDay> {
    let split = entry.len().checked_sub(2)?;
    let (nth, code) = (entry.get(..split)?, entry.get(split..)?);
    let weekday = WEEKDAYS.iter().position(|d| *d == code)? as i32;
    let nth = if nth.is_empty() { 0 } else { nth.parse().ok()? };
    Some(ByDay { weekday, nth })
}

/// The day of an iCalendar date (`20261005`) or date-time
/// (`20261005T100000`, `…Z`).
fn ical_day(text: &str) -> Option<Date> {
    let digits = |range: std::ops::Range<usize>| text.get(range)?.parse().ok();
    Date::from_ymd(digits(0..4)?, digits(4..6)?, digits(6..8)?).ok()
}

// ----------------------------------------------------------------------------
/// The days of the schedule `lines` from `from` on, to `until` and `last`;
/// `None` without an RRULE `rule` reads. Without `BYMONTHDAY`, `BYDAY` or
/// `BYMONTH`, `from`'s day, weekday or month; a day a month doesn't have is
/// left out. `COUNT` counts the days an `EXDATE` leaves out too.
pub fn occurrences(
    lines: &[String],
    from: Date,
    until: Option<Date>,
    last: Date,
) -> Option<Vec<Date>> {
    let mut found = None;
    let mut excluded = Vec::new();
    for (name, value) in lines.iter().filter_map(|line| line.split_once(':')) {
        let name = name.split(';').next().unwrap_or_default();
        if name.eq_ignore_ascii_case("RRULE") {
            found = Some(value);
        } else if name.eq_ignore_ascii_case("EXDATE") {
            excluded.extend(value.split(',').filter_map(ical_day));
        }
    }
    let rule = rule(found?)?;
    let last = [Some(last), until, rule.until]
        .into_iter()
        .flatten()
        .min()?;
    let (year, month, day) = from.to_ymd();
    let weekday = i32::from(from.weekday());
    let mut out = Vec::new();
    let mut counted = 0;
    for period in 0..MAX_PERIODS {
        let step = period * rule.interval;
        let (start, mut days) = match rule.freq {
            Freq::Daily => {
                let date = Date::new(from.days() + step);
                let picked = rule.by_day.is_empty()
                    || rule
                        .by_day
                        .iter()
                        .any(|b| b.weekday == i32::from(date.weekday()));
                (date, if picked { vec![date] } else { Vec::new() })
            }
            Freq::Weekly => {
                let monday = from.days() - weekday + step * 7;
                let week: Vec<i32> = if rule.by_day.is_empty() {
                    vec![weekday]
                } else {
                    rule.by_day.iter().map(|b| b.weekday).collect()
                };
                let days = week.iter().map(|d| Date::new(monday + d)).collect();
                (Date::new(monday), days)
            }
            Freq::Monthly => {
                let (year, month) = months_on(year, month, step);
                let start = Date::from_ymd(year, i32::from(month), 1).ok()?;
                (start, month_days(year, month, &rule, day))
            }
            Freq::Yearly => {
                let year = year + step;
                let months = if rule.months.is_empty() {
                    vec![month]
                } else {
                    rule.months.clone()
                };
                let days = months
                    .into_iter()
                    .flat_map(|month| month_days(year, month, &rule, day));
                (Date::from_ymd(year, 1, 1).ok()?, days.collect())
            }
        };
        if start > last {
            break;
        }
        days.sort_unstable();
        for date in days {
            if date < from || !(rule.months.is_empty() || rule.months.contains(&date.to_ymd().1)) {
                continue;
            }
            if date > last || rule.count.is_some_and(|count| counted >= count) {
                return Some(out);
            }
            counted += 1;
            if !excluded.contains(&date) {
                out.push(date);
            }
        }
    }
    Some(out)
}

// ----------------------------------------------------------------------------
/// The year and month `n` months after `month` of `year`.
fn months_on(year: i32, month: Month, n: i32) -> (i32, Month) {
    let index = i32::from(month) - 1 + n;
    let month = Month::try_from(index.rem_euclid(12) + 1).unwrap_or(Month::Jan);
    (year + index.div_euclid(12), month)
}

// ----------------------------------------------------------------------------
/// The days of `month` of `year` the rule picks: by `BYMONTHDAY` (those on
/// a `BYDAY` weekday, when both are given), by `BYDAY`, or `day`; a day the
/// month doesn't have left out.
fn month_days(year: i32, month: Month, rule: &Rule, day: i32) -> Vec<Date> {
    let length = days_in_month(year, month);
    let date = |d: i32| Date::from_ymd(year, i32::from(month), d).ok();
    let days: Vec<i32> = if !rule.month_days.is_empty() {
        let on_weekday = |d: i32| {
            let weekday = date(d).map(|date| i32::from(date.weekday()));
            rule.by_day.is_empty() || rule.by_day.iter().any(|b| Some(b.weekday) == weekday)
        };
        let days = rule
            .month_days
            .iter()
            .map(|d| if *d < 0 { length + 1 + d } else { *d });
        days.filter(|d| on_weekday(*d)).collect()
    } else if !rule.by_day.is_empty() {
        let weekdays = rule.by_day.iter();
        weekdays
            .flat_map(|b| weekdays_in_month(year, month, length, *b))
            .collect()
    } else {
        vec![day]
    };
    let mut dates: Vec<Date> = days
        .into_iter()
        .filter(|d| (1..=length).contains(d))
        .filter_map(date)
        .collect();
    dates.sort_unstable();
    dates
}

// ----------------------------------------------------------------------------
/// The days of `month` that are `by`'s weekday: each of them, or only the
/// nth (counted from the end if negative).
fn weekdays_in_month(year: i32, month: Month, length: i32, by: ByDay) -> Vec<i32> {
    let Ok(first) = Date::from_ymd(year, i32::from(month), 1) else {
        return Vec::new();
    };
    let first_day = 1 + (by.weekday - i32::from(first.weekday())).rem_euclid(7);
    let each: Vec<i32> = (first_day..=length).step_by(7).collect();
    let nth = match by.nth {
        0 => return each,
        n if n > 0 => each.get(n as usize - 1),
        n => each
            .len()
            .checked_sub(n.unsigned_abs() as usize)
            .and_then(|i| each.get(i)),
    };
    nth.copied().into_iter().collect()
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
            &[format!("RRULE:{rule}")],
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
            "FREQ=HOURLY",
            "FREQ=MONTHLY;BYSETPOS=1",
            "FREQ=YEARLY;BYMONTHDAY=1",
            "FREQ=YEARLY;BYMONTH=13",
        ] {
            assert_eq!(
                every(unread, "2026-12-01", None, "2027-01-01"),
                None,
                "{unread}"
            );
        }
        let only_exdate = ["EXDATE:20261201".to_string()];
        assert_eq!(
            occurrences(&only_exdate, day("2026-12-01"), None, day("2027-01-01")),
            None
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_schedule_reads_days_ordinals_and_exdates_as_the_page() {
        let daily = every("FREQ=DAILY;INTERVAL=2", "2026-12-01", None, "2026-12-07");
        let expected = ["2026-12-01", "2026-12-03", "2026-12-05", "2026-12-07"];
        assert_eq!(daily, Some(days(&expected)));
        let workdays = every("FREQ=DAILY;BYDAY=MO,FR", "2026-12-01", None, "2026-12-08");
        assert_eq!(workdays, Some(days(&["2026-12-04", "2026-12-07"])));

        let second_tuesday = every("FREQ=MONTHLY;BYDAY=2TU", "2026-12-01", None, "2027-02-28");
        let expected = ["2026-12-08", "2027-01-12", "2027-02-09"];
        assert_eq!(second_tuesday, Some(days(&expected)));
        let last_friday = every("FREQ=MONTHLY;BYDAY=-1FR", "2026-12-01", None, "2027-01-31");
        assert_eq!(last_friday, Some(days(&["2026-12-25", "2027-01-29"])));
        let friday_13th = every(
            "FREQ=MONTHLY;BYMONTHDAY=13;BYDAY=FR",
            "2026-01-01",
            None,
            "2026-12-31",
        );
        let expected = ["2026-02-13", "2026-03-13", "2026-11-13"];
        assert_eq!(friday_13th, Some(days(&expected)));

        let lines = |rule: &str, exdate: &str| [format!("RRULE:{rule}"), exdate.to_string()];
        let skipped = lines("FREQ=MONTHLY;BYMONTHDAY=1", "EXDATE:20270101");
        let found = occurrences(&skipped, day("2026-12-01"), None, day("2027-02-28"));
        assert_eq!(found, Some(days(&["2026-12-01", "2027-02-01"])));
        let counted = lines(
            "FREQ=MONTHLY;BYMONTHDAY=1;COUNT=2",
            "EXDATE;VALUE=DATE:20270101",
        );
        let found = occurrences(&counted, day("2026-12-01"), None, day("2027-12-31"));
        assert_eq!(
            found,
            Some(days(&["2026-12-01"])),
            "the left-out day counts"
        );
    }
}
