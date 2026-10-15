// Date utilities based on Howard Hinnant's algorithms
// https://howardhinnant.github.io/date_algorithms.html

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------------------
const SECONDS_PER_DAY: u64 = 86_400;

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

// ----------------------------------------------------------------------------
impl TryFrom<i32> for Weekday {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self> {
        match value {
            0 => Ok(Weekday::Mon),
            1 => Ok(Weekday::Tue),
            2 => Ok(Weekday::Wed),
            3 => Ok(Weekday::Thu),
            4 => Ok(Weekday::Fri),
            5 => Ok(Weekday::Sat),
            6 => Ok(Weekday::Sun),
            _ => Err(Error::InvalidDate),
        }
    }
}

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug)]
pub enum Month {
    Jan = 1,
    Feb = 2,
    Mar = 3,
    Apr = 4,
    May = 5,
    Jun = 6,
    Jul = 7,
    Aug = 8,
    Sep = 9,
    Oct = 10,
    Nov = 11,
    Dec = 12,
}

// ----------------------------------------------------------------------------
impl From<Month> for i32 {
    fn from(value: Month) -> i32 {
        value as i32
    }
}

// ----------------------------------------------------------------------------
impl TryFrom<i32> for Month {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self> {
        match value {
            1 => Ok(Self::Jan),
            2 => Ok(Self::Feb),
            3 => Ok(Self::Mar),
            4 => Ok(Self::Apr),
            5 => Ok(Self::May),
            6 => Ok(Self::Jun),
            7 => Ok(Self::Jul),
            8 => Ok(Self::Aug),
            9 => Ok(Self::Sep),
            10 => Ok(Self::Oct),
            11 => Ok(Self::Nov),
            12 => Ok(Self::Dec),
            _ => Err(Error::InvalidDate),
        }
    }
}

// ----------------------------------------------------------------------------
impl fmt::Display for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Delegate, so width/fill flags like `{month:02}` apply.
        let num: i32 = (*self).into();
        fmt::Display::fmt(&num, f)
    }
}

// ----------------------------------------------------------------------------
const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ----------------------------------------------------------------------------
#[rustfmt::skip]
const fn days_in_month(year: i32, month: Month) -> Result<i32> {
    use Month::*;
    match month {
        Jan | Mar | May | Jul | Aug | Oct | Dec => Ok(31),
        Apr | Jun | Sep | Nov => Ok(30),
        Feb if is_leap_year(year) => Ok(29),
        Feb => Ok(28),
    }
}

// ----------------------------------------------------------------------------
// Using 32 bit arithmetic, overflow occurs at +/- 5.8 million years
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(i32);

impl Date {
    // ------------------------------------------------------------------------
    pub fn new(days: i32) -> Self {
        Date(days)
    }

    // ------------------------------------------------------------------------
    pub fn from_ymd(year: i32, month: i32, day: i32) -> Result<Self> {
        let dim = days_in_month(year, Month::try_from(month)?)?;
        if day < 1 || day > dim {
            return Err(Error::InvalidDate);
        }
        Ok(Date(days_from_gregorian(year, month, day)))
    }

    // ------------------------------------------------------------------------
    pub fn today() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);

        let today = now.as_secs().div_euclid(SECONDS_PER_DAY);

        // will not overflow for at least 5.8 million years
        Date(today as i32)
    }

    // ------------------------------------------------------------------------
    pub const fn to_ymd(self) -> (i32, Month, i32) {
        gregorian_from_days(self.0)
    }

    // ------------------------------------------------------------------------
    pub const fn weekday(&self) -> Weekday {
        // 1970-01-01 = Thursday = ISO weekday 3 (Mon=0)
        const THURSDAY: i32 = 3;
        let day = self.0 + THURSDAY;
        match day.rem_euclid(7) {
            0 => Weekday::Mon,
            1 => Weekday::Tue,
            2 => Weekday::Wed,
            3 => Weekday::Thu,
            4 => Weekday::Fri,
            5 => Weekday::Sat,
            6 => Weekday::Sun,
            _ => unreachable!(),
        }
    }
}

// ----------------------------------------------------------------------------
#[rustfmt::skip]
const fn days_from_gregorian(year: i32, month: i32, day: i32) -> i32 {
    // March-based year and month
    let y = year + if month > 2 { 0 } else { -1 };
    let m = month + if month > 2 { -3 } else { 9 };                 // [0, 11]

    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);                                    // [0, 399]

    let doy = (153 * m + 2) / 5 + day - 1;                          // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;                // [0, 146096]

    era * 146_097 + doe - 719_468
}

// ----------------------------------------------------------------------------
#[rustfmt::skip]
const fn gregorian_from_days(days: i32) -> (i32, Month, i32) {
    const MONTHS: [(Month, i32); 12] = [
        (Month::Mar, 0), (Month::Apr, 0), (Month::May, 0), (Month::Jun, 0), 
        (Month::Jul, 0), (Month::Aug, 0), (Month::Sep, 0), (Month::Oct, 0), 
        (Month::Nov, 0), (Month::Dec, 0), (Month::Jan, 1), (Month::Feb, 1),
    ];
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);                                // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);              // [0, 365]
    let mp = (5 * doy + 2) / 153;                                   // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1;                         // [1, 31]
    let (month, year_offset) = MONTHS[mp as usize];
    let year = y + year_offset;
    (year, month, day)
}

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(u32);

impl Time {
    // ------------------------------------------------------------------------
    pub fn new(seconds: u64) -> Result<Self> {
        if seconds >= SECONDS_PER_DAY {
            return Err(Error::InvalidTime);
        }
        Ok(Time(seconds as u32))
    }

    // ------------------------------------------------------------------------
    pub fn from_hms(hour: u32, minute: u32, second: u32) -> Result<Self> {
        if hour >= 24 || minute >= 60 || second >= 60 {
            return Err(Error::InvalidTime);
        }
        Ok(Time(hour * 3600 + minute * 60 + second))
    }

    // ------------------------------------------------------------------------
    pub fn now() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);

        let seconds = now.as_secs().rem_euclid(SECONDS_PER_DAY);

        // will never overflow because 0 ≤ seconds_today < SECONDS_PER_DAY
        Time(seconds as u32)
    }

    // ------------------------------------------------------------------------
    pub const fn to_hms(self) -> (u32, u32, u32) {
        let hour = self.0 / 3600;
        let minute = (self.0 % 3600) / 60;
        let second = self.0 % 60;
        (hour, minute, second)
    }
}

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime {
    pub date: Date,
    pub time: Option<Time>,
}

impl DateTime {
    // ------------------------------------------------------------------------
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
    }

    // ------------------------------------------------------------------------
    /// The UTC date-time of `time` (e.g. a file's), truncated to the second;
    /// a time before 1970 is the epoch.
    pub fn from_system_time(time: SystemTime) -> Self {
        let since = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);

        let days = since.as_secs().div_euclid(SECONDS_PER_DAY);
        let seconds = since.as_secs().rem_euclid(SECONDS_PER_DAY);

        DateTime {
            date: Date(days as i32),
            time: Some(Time(seconds as u32)),
        }
    }

    // ------------------------------------------------------------------------
    pub fn today() -> Self {
        DateTime {
            date: Self::now().date,
            time: None,
        }
    }

    // ------------------------------------------------------------------------
    /// The UTC date-time of a unix timestamp in milliseconds (e.g. Gmail's
    /// `internalDate`), truncated to the second.
    pub fn from_unix_millis(millis: i64) -> Self {
        let secs = millis.div_euclid(1000);
        let days = secs.div_euclid(SECONDS_PER_DAY as i64);
        let seconds = secs.rem_euclid(SECONDS_PER_DAY as i64);
        DateTime {
            date: Date(days as i32),
            time: Some(Time(seconds as u32)),
        }
    }

    // ------------------------------------------------------------------------
    /// Seconds since the unix epoch; a plain date counts from its midnight UTC.
    pub fn as_unix(&self) -> i64 {
        let seconds = self.time.map_or(0, |t| t.0 as i64);
        self.date.0 as i64 * SECONDS_PER_DAY as i64 + seconds
    }

    // ------------------------------------------------------------------------
    pub fn from_ymd_hms(
        year: i32,
        month: i32,
        day: i32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Result<Self> {
        let date = Date::from_ymd(year, month, day)?;
        let time = Some(Time::from_hms(hour, minute, second)?);
        Ok(Self { date, time })
    }

    // ------------------------------------------------------------------------
    /// `YYYY-MM-DD`, or `YYYY-MM-DDTHH:MM:SS` with optional fractional
    /// seconds (dropped) and a `Z` or `±hh:mm`/`±hhmm` offset (converted to UTC).
    pub fn from_iso8601(s: &str) -> Result<Self> {
        let s = s.as_bytes();

        // The ASCII digits in `s[range]`; anything else (including bytes of
        // a multi-byte character) is `err`.
        fn digits<T: TryFrom<i32>>(s: &[u8], range: std::ops::Range<usize>) -> Option<T> {
            s.get(range)
                .filter(|p| p.iter().all(u8::is_ascii_digit))
                .map(|p| p.iter().fold(0_i32, |a, b| a * 10 + i32::from(b - b'0')))
                .and_then(|n| T::try_from(n).ok())
        }

        let (year, month, day) = (digits(s, 0..4), digits(s, 5..7), digits(s, 8..10));
        let (Some(year), Some(month), Some(day)) = (year, month, day) else {
            return Err(Error::InvalidDate);
        };
        let date = Date::from_ymd(year, month, day)?;

        // Without a full HH:MM:SS the value is read as a plain date.
        if s.len() < 19 {
            return Ok(Self { date, time: None });
        }
        let (hour, minute, second) = (digits(s, 11..13), digits(s, 14..16), digits(s, 17..19));
        let (Some(hour), Some(minute), Some(second)) = (hour, minute, second) else {
            return Err(Error::InvalidTime);
        };
        let time = Some(Time::from_hms(hour, minute, second)?);
        let local = Self { date, time };

        // What follows the seconds: a fraction, then the offset from UTC.
        let mut rest = &s[19..];
        if let Some(fraction) = rest.strip_prefix(b".") {
            let n = fraction.iter().take_while(|b| b.is_ascii_digit()).count();
            rest = &fraction[n..];
        }
        let offset = match rest {
            [sign @ (b'+' | b'-'), ..] => {
                let hours: i64 = digits(rest, 1..3).ok_or(Error::InvalidTime)?;
                let minutes_at = if rest.get(3) == Some(&b':') {
                    4..6
                } else {
                    3..5
                };
                let minutes: i64 = digits(rest, minutes_at).ok_or(Error::InvalidTime)?;
                let offset = hours * 3600 + minutes * 60;
                if *sign == b'-' { -offset } else { offset }
            }
            _ => 0, // `Z`, or nothing
        };
        if offset == 0 {
            return Ok(local);
        }
        Ok(Self::from_unix_millis((local.as_unix() - offset) * 1000))
    }

    // ------------------------------------------------------------------------
    pub fn as_iso8601(&self) -> String {
        let (year, month, day) = self.date.to_ymd();
        let month: i32 = month.into();
        if let Some((hour, minute, second)) = self.time.map(|t| t.to_hms()) {
            format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
        } else {
            format!("{year:04}-{month:02}-{day:02}")
        }
    }

    // ------------------------------------------------------------------------
    pub fn as_timestamp(&self) -> String {
        let (year, month, day) = self.date.to_ymd();
        let month: i32 = month.into();
        if let Some((hour, minute, second)) = self.time.map(|t| t.to_hms()) {
            format!("{year:04}{month:02}{day:02}_{hour:02}{minute:02}{second:02}")
        } else {
            format!("{year:04}{month:02}{day:02}")
        }
    }
}

// ----------------------------------------------------------------------------
impl<'a> Deserialize<'a> for DateTime {
    fn deserialize<D: serde::Deserializer<'a>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let s: String = Deserialize::deserialize(deserializer)?;
        DateTime::from_iso8601(&s).map_err(serde::de::Error::custom)
    }
}

// ----------------------------------------------------------------------------
impl Serialize for DateTime {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_iso8601())
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// (days_since_1970, year, month, day, weekday)
    #[rustfmt::skip]
    pub const TEST_DATES: [(i32, i32, i32, i32, Weekday); 16] = [
        // Epoch
        (   0,  1970,  1,  1, Weekday::Thu),
        (   1,  1970,  1,  2, Weekday::Fri),
        (  30,  1970,  1, 31, Weekday::Sat),
        (  31,  1970,  2,  1, Weekday::Sun),
        (  59,  1970,  3,  1, Weekday::Sun),
        ( 364,  1970, 12, 31, Weekday::Thu),

        // Year transitions
        ( 365,  1971,  1,  1, Weekday::Fri),
        ( 730,  1972,  1,  1, Weekday::Sat),
        ( 731,  1972,  1,  2, Weekday::Sun),
        (1095,  1972, 12, 31, Weekday::Sun),

        // Millennium leap year
        (10957, 2000,  1,  1, Weekday::Sat),
        (11322, 2000, 12, 31, Weekday::Sun),

        // Negative days (before 1970)
        (-1,    1969, 12, 31, Weekday::Wed),
        (-365,  1969,  1,  1, Weekday::Wed),
        (-366,  1968, 12, 31, Weekday::Tue), // (leap year)
        (-719528,  0,  1,  1, Weekday::Sat), // (proleptic Gregorian start)
    ];

    #[test]
    fn test_date_conversion() {
        for (days, year, month, day, weekday) in TEST_DATES.iter() {
            let date = Date(*days);
            let (y, m, d) = date.to_ymd();
            assert_eq!((*year, *month, *day), (y, m.into(), d));

            let computed_days = days_from_gregorian(y, m.into(), d);
            assert_eq!(*days, computed_days);

            let wd = date.weekday();
            assert_eq!(*weekday, wd, "Weekday mismatch for {year}-{month}-{day}",);
        }
    }

    #[test]
    fn unix_timestamps_round_trip() {
        let dt = DateTime::from_unix_millis(1_790_503_530_999);
        assert_eq!(dt.as_unix(), 1_790_503_530);
    }

    #[test]
    fn from_iso8601_parses_and_never_panics() {
        let dt = DateTime::from_iso8601("2026-09-27T10:05:30Z").unwrap();
        assert_eq!(dt.as_iso8601(), "2026-09-27T10:05:30Z");
        let d = DateTime::from_iso8601("2026-09-27").unwrap();
        assert_eq!((d.as_iso8601(), d.time), ("2026-09-27".to_string(), None));

        // Multi-byte characters where fixed byte offsets would split them.
        for bad in ["202ä-01-01", "2026-0ä-01", "2026-01-01T1ä:00:00", "2026"] {
            assert!(DateTime::from_iso8601(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn from_iso8601_converts_offsets_to_utc() {
        let utc = |s: &str| DateTime::from_iso8601(s).unwrap().as_iso8601();
        assert_eq!(utc("2026-09-24T12:10:00+02:00"), "2026-09-24T10:10:00Z");
        assert_eq!(utc("2026-09-24T01:30:00+0200"), "2026-09-23T23:30:00Z");
        assert_eq!(utc("2026-12-31T20:00:00-05:00"), "2027-01-01T01:00:00Z");
        assert_eq!(utc("2026-09-24T10:10:00.123Z"), "2026-09-24T10:10:00Z");
        assert_eq!(utc("2026-09-24T10:10:00.5+01:00"), "2026-09-24T09:10:00Z");
        assert_eq!(utc("2026-09-24T10:10:00"), "2026-09-24T10:10:00Z");
        assert!(DateTime::from_iso8601("2026-09-24T10:10:00+0ä:00").is_err());
    }

    #[test]
    fn month_display_honors_width() {
        assert_eq!(format!("{:02}", Month::Mar), "03");
        assert_eq!(format!("{}", Month::Dec), "12");
    }
}
