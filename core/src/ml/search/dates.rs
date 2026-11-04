//! Date search: Ente's free-form date parser (`DateParseService`), the
//! month and year matching of the search screen, and the day ranges a date
//! suggestion covers.
//!
//! Ported from the mobile app: `services/date_parse_service.dart` (parser,
//! kept rule for rule), `services/search_service.dart` (`getYearSearchResults`,
//! `getMonthSearchResults`, `getDateResults`,
//! `_getDurationsForCalendarDateInEveryYear`, `_getDurationsOfMonthInEveryYear`)
//! and `ui/viewer/search/search_query_utils.dart` (`isYearSearchQuery`).
//! Holidays are left out. Ranges are in the device's local time, like the
//! app's `DateTime(year, month, day)`, and are matched against the files'
//! capture time (edited time if set).
//!
//! Based on Ente (ente.com).

use chrono::{Datelike, NaiveDate, TimeZone};

/// First year of "every year" searches (`searchStartYear`).
pub(crate) const SEARCH_START_YEAR: i32 = 1970;
/// Smallest year a bare 4-digit query matches (`_minimumSearchYear`).
const MINIMUM_SEARCH_YEAR: i32 = 1900;

const MIN_YEAR: i64 = 1900;
const MAX_YEAR: i64 = 2100;
const TWO_DIGIT_YEAR_PIVOT: i64 = 50;

/// English month names (`DateParseService.monthNumberToName`).
pub(crate) const MONTH_NAMES: [&str; 12] = [
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

/// `DateParseService._monthMap`.
const MONTH_MAP: &[(&str, u32)] = &[
    ("january", 1),
    ("february", 2),
    ("march", 3),
    ("april", 4),
    ("may", 5),
    ("june", 6),
    ("july", 7),
    ("august", 8),
    ("september", 9),
    ("october", 10),
    ("november", 11),
    ("december", 12),
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("sept", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
    ("janu", 1),
    ("febr", 2),
    ("marc", 3),
    ("apri", 4),
    ("juli", 7),
    ("augu", 8),
    ("sepe", 9),
    ("octo", 10),
    ("nove", 11),
    ("dece", 12),
];

fn month_of(token: &str) -> Option<u32> {
    MONTH_MAP.iter().find(|(n, _)| *n == token).map(|(_, m)| *m)
}

/// A date with any of its parts missing (`PartialDate`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PartialDate {
    pub day: Option<u32>,
    pub month: Option<u32>,
    pub year: Option<i32>,
}

impl PartialDate {
    const EMPTY: Self = Self {
        day: None,
        month: None,
        year: None,
    };

    fn ymd(day: i64, month: i64, year: i64) -> Self {
        Self {
            day: Some(day as u32),
            month: Some(month as u32),
            year: Some(year as i32),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.day.is_none() && self.month.is_none() && self.year.is_none()
    }
}

/// `DateParseService.parse`. `today` is the local date, for "today",
/// "yesterday" and "tomorrow".
pub(crate) fn parse(input: &str, today: NaiveDate) -> PartialDate {
    if input.trim().is_empty() {
        return PartialDate::EMPTY;
    }
    let lower = input.to_lowercase();
    let r = parse_relative(&lower, today);
    if !r.is_empty() {
        return r;
    }
    let r = parse_structured(&lower);
    if !r.is_empty() {
        return r;
    }
    parse_tokenized(&normalize(&lower))
}

fn parse_relative(lower: &str, today: NaiveDate) -> PartialDate {
    let has = |w: &str| lower.contains(w);
    let (t, tm, y) = (has("today"), has("tomorrow"), has("yesterday"));
    if (t as u8 + tm as u8 + y as u8) > 1 {
        return PartialDate::EMPTY;
    }
    let d = if t {
        Some(today)
    } else if tm {
        today.succ_opt()
    } else if y {
        today.pred_opt()
    } else {
        None
    };
    d.map_or(PartialDate::EMPTY, |d| {
        PartialDate::ymd(d.day().into(), d.month().into(), d.year().into())
    })
}

fn convert_two_digit_year(year: i64) -> i64 {
    if year < TWO_DIGIT_YEAR_PIVOT {
        2000 + year
    } else {
        1900 + year
    }
}

/// Splits `s` into digit groups separated by single characters of `seps`;
/// `None` if anything else is in it. Each group is `(digits, value)`.
fn digit_groups<'a>(s: &'a str, seps: &[char]) -> Option<Vec<&'a str>> {
    if s.is_empty() {
        return None;
    }
    let groups: Vec<&str> = s.split(|c| seps.contains(&c)).collect();
    groups
        .iter()
        .all(|g| !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit()))
        .then_some(groups)
}

/// Lengths of the groups are within these inclusive ranges.
fn shape(groups: &[&str], lens: &[(usize, usize)]) -> bool {
    groups.len() == lens.len()
        && groups
            .iter()
            .zip(lens)
            .all(|(g, (lo, hi))| (*lo..=*hi).contains(&g.len()))
}

fn num(s: &str) -> i64 {
    // At most 8 ASCII digits: always fits.
    s.parse().unwrap_or(0)
}

fn parse_structured(input: &str) -> PartialDate {
    let clean: String = input.chars().filter(|c| *c != ' ').collect();
    let slash = digit_groups(&clean, &['/', '-']);
    let years = MIN_YEAR..=MAX_YEAR;
    let days = 1..=31;
    let months = 1..=12;

    // ^(\d{4})[\/-](\d{1,2})[\/-](\d{1,2})$
    if let Some(g) = slash
        .as_ref()
        .filter(|g| shape(g, &[(4, 4), (1, 2), (1, 2)]))
    {
        let (y, m, d) = (num(g[0]), num(g[1]), num(g[2]));
        if years.contains(&y) && months.contains(&m) && days.contains(&d) {
            return PartialDate::ymd(d, m, y);
        }
        return PartialDate::EMPTY;
    }
    // ^(\d{1,2})[\/-](\d{4})$
    if let Some(g) = slash.as_ref().filter(|g| shape(g, &[(1, 2), (4, 4)])) {
        let (m, y) = (num(g[0]), num(g[1]));
        if years.contains(&y) && months.contains(&m) {
            return PartialDate {
                day: None,
                month: Some(m as u32),
                year: Some(y as i32),
            };
        }
        return PartialDate::EMPTY;
    }
    // ^(\d{1,2})[\/-](\d{1,2})[\/-](\d{2,4})$
    if let Some(g) = slash
        .as_ref()
        .filter(|g| shape(g, &[(1, 2), (1, 2), (2, 4)]))
    {
        let (p1, p2, raw) = (num(g[0]), num(g[1]), num(g[2]));
        let y = if raw > 99 {
            raw
        } else {
            convert_two_digit_year(raw)
        };
        if !years.contains(&y) {
            return PartialDate::EMPTY;
        }
        if days.contains(&p1) && months.contains(&p2) {
            return PartialDate::ymd(p1, p2, y);
        } else if months.contains(&p1) && days.contains(&p2) {
            return PartialDate::ymd(p2, p1, y);
        }
        return PartialDate::EMPTY;
    }
    // ^(\d{1,2})[\/-](\d{1,2})$
    if let Some(g) = slash.as_ref().filter(|g| shape(g, &[(1, 2), (1, 2)])) {
        let (p1, p2) = (num(g[0]), num(g[1]));
        if days.contains(&p1) && months.contains(&p2) {
            return PartialDate {
                day: Some(p1 as u32),
                month: Some(p2 as u32),
                year: None,
            };
        } else if months.contains(&p1) && days.contains(&p2) {
            return PartialDate {
                day: Some(p2 as u32),
                month: Some(p1 as u32),
                year: None,
            };
        }
        return PartialDate::EMPTY;
    }
    // ^(\d{1,2})\.(\d{1,2})\.(\d{2,4})$
    if let Some(g) = digit_groups(&clean, &['.']).filter(|g| shape(g, &[(1, 2), (1, 2), (2, 4)])) {
        let (d, m, raw) = (num(g[0]), num(g[1]), num(g[2]));
        let y = if raw > 99 {
            raw
        } else {
            convert_two_digit_year(raw)
        };
        if years.contains(&y) && days.contains(&d) && months.contains(&m) {
            return PartialDate::ymd(d, m, y);
        }
        return PartialDate::EMPTY;
    }
    // ^(\d{8})$
    if clean.len() == 8 && clean.bytes().all(|b| b.is_ascii_digit()) {
        let (y, m, d) = (num(&clean[..4]), num(&clean[4..6]), num(&clean[6..8]));
        if years.contains(&y) && months.contains(&m) && days.contains(&d) {
            return PartialDate::ymd(d, m, y);
        }
        return PartialDate::EMPTY;
    }
    PartialDate::EMPTY
}

/// `_normalizeDateString`: ordinals to numbers, "of", commas, dots and
/// runs of whitespace to single spaces.
fn normalize(input: &str) -> String {
    let spaced: String = input
        .chars()
        .map(|c| {
            if c == ',' || c == '.' || c.is_whitespace() {
                ' '
            } else {
                c
            }
        })
        .collect();
    spaced
        .split(' ')
        .filter(|t| !t.is_empty() && *t != "of")
        .map(|t| {
            // \b(\d{1,2})(st|nd|rd|th)\b
            let digits = t.bytes().take_while(u8::is_ascii_digit).count();
            let rest = &t[digits..];
            if (1..=2).contains(&digits) && ["st", "nd", "rd", "th"].contains(&rest) {
                &t[..digits]
            } else {
                t
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_tokenized(normalized: &str) -> PartialDate {
    let mut day: Option<i64> = None;
    let mut month: Option<i64> = None;
    let mut year: Option<i64> = None;
    let mut numbers: Vec<i64> = Vec::new();
    let mut month_names: Vec<u32> = Vec::new();
    for token in normalized.split(' ').filter(|s| !s.is_empty()) {
        if let Some(m) = month_of(token) {
            month_names.push(m);
        } else if let Ok(v) = token.parse::<i64>() {
            numbers.push(v);
        }
    }
    let years = MIN_YEAR..=MAX_YEAR;
    let days = 1..=31;
    let months = 1..=12;
    if month_names.len() == 1 {
        month = Some(month_names[0].into());
    }
    match numbers.len() {
        3 => {
            let (pd, pm, py) = (numbers[0], numbers[1], numbers[2]);
            if years.contains(&py) {
                year = Some(py);
                if days.contains(&pd) && months.contains(&pm) {
                    day = Some(pd);
                    month = Some(pm);
                }
            }
        }
        2 => {
            let (pd, pm) = (numbers[0], numbers[1]);
            if days.contains(&pd) && months.contains(&pm) {
                day = Some(pd);
                month = Some(pm);
            } else if months.contains(&pd) && days.contains(&pm) {
                day = Some(pm);
                month = Some(pd);
            }
        }
        1 => {
            let v = numbers[0];
            if years.contains(&v) {
                year = Some(v);
            } else if days.contains(&v) && month.is_none() {
                day = Some(v);
            } else if months.contains(&v) && month.is_none() {
                month = Some(v);
            }
        }
        _ => {}
    }
    if month.is_some() {
        match numbers.len() {
            2 => {
                let (n1, n2) = (numbers[0], numbers[1]);
                if years.contains(&n1) {
                    year = Some(n1);
                    if days.contains(&n2) {
                        day = Some(n2);
                    }
                } else if years.contains(&n2) {
                    year = Some(n2);
                    if days.contains(&n1) {
                        day = Some(n1);
                    }
                }
            }
            1 => {
                let n = numbers[0];
                if days.contains(&n) {
                    day = Some(n);
                } else if years.contains(&n) {
                    year = Some(n);
                }
            }
            _ => {}
        }
    }
    let day = day.filter(|d| days.contains(d));
    let month = month.filter(|m| months.contains(m));
    PartialDate {
        day: day.map(|d| d as u32),
        month: month.map(|m| m as u32),
        year: year.map(|y| y as i32),
    }
}

// ---------------------------------------------------------------- keys

/// What a date suggestion stands for (the `SearchKind::Date` key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DateKey {
    /// `y:2024`.
    Year(i32),
    /// `ym:2024-05`.
    YearMonth(i32, u32),
    /// `ymd:2024-05-17`.
    Day(i32, u32, u32),
    /// `md:05-17`: this day in every year.
    MonthDay(u32, u32),
    /// `m:05`: this month in every year.
    Month(u32),
}

impl DateKey {
    /// Parse a key; `None` if malformed or not a real date.
    pub(crate) fn parse(key: &str) -> Option<Self> {
        let (tag, rest) = key.split_once(':')?;
        let parts: Vec<&str> = rest.split('-').collect();
        let int = |s: &str| -> Option<i64> {
            (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && s.len() <= 4)
                .then(|| s.parse().ok())
                .flatten()
        };
        let month_ok = |m: i64| (1..=12).contains(&m);
        let k = match (tag, parts.as_slice()) {
            ("y", [y]) => Self::Year(int(y)? as i32),
            ("ym", [y, m]) if month_ok(int(m)?) => Self::YearMonth(int(y)? as i32, int(m)? as u32),
            ("ymd", [y, m, d]) => {
                let (y, m, d) = (int(y)? as i32, int(m)? as u32, int(d)? as u32);
                NaiveDate::from_ymd_opt(y, m, d)?;
                Self::Day(y, m, d)
            }
            ("md", [m, d]) => {
                let (m, d) = (int(m)? as u32, int(d)? as u32);
                // Valid in some year (29 February in leap years).
                NaiveDate::from_ymd_opt(2000, m, d)?;
                Self::MonthDay(m, d)
            }
            ("m", [m]) if month_ok(int(m)?) => Self::Month(int(m)? as u32),
            _ => return None,
        };
        Some(k)
    }

    /// The key string.
    pub(crate) fn key(&self) -> String {
        match *self {
            Self::Year(y) => format!("y:{y:04}"),
            Self::YearMonth(y, m) => format!("ym:{y:04}-{m:02}"),
            Self::Day(y, m, d) => format!("ymd:{y:04}-{m:02}-{d:02}"),
            Self::MonthDay(m, d) => format!("md:{m:02}-{d:02}"),
            Self::Month(m) => format!("m:{m:02}"),
        }
    }

    /// The label the apps show ("2024", "May", "May 2024", "17 May 2024",
    /// "17 May").
    pub(crate) fn label(&self) -> String {
        let name = |m: u32| MONTH_NAMES[(m - 1) as usize];
        match *self {
            Self::Year(y) => y.to_string(),
            Self::Month(m) => name(m).to_string(),
            Self::YearMonth(y, m) => format!("{} {y}", name(m)),
            Self::Day(y, m, d) => format!("{d} {} {y}", name(m)),
            Self::MonthDay(m, d) => format!("{d} {}", name(m)),
        }
    }

    /// The `[start, end)` ranges (µs since the epoch, local time of `tz`)
    /// this key covers, newest first. "Every year" keys run from
    /// [`SEARCH_START_YEAR`] to `current_year`.
    pub(crate) fn ranges<Tz: TimeZone>(&self, tz: &Tz, current_year: i32) -> Vec<(i64, i64)> {
        let day = |y: i32, m: u32, d: u32| NaiveDate::from_ymd_opt(y, m, d);
        let month_start = |y: i32, m: u32| day(y, m, 1);
        let next_month = |y: i32, m: u32| {
            if m == 12 {
                day(y + 1, 1, 1)
            } else {
                day(y, m + 1, 1)
            }
        };
        let span = |a: Option<NaiveDate>, b: Option<NaiveDate>| -> Option<(i64, i64)> {
            Some((local_start(tz, a?), local_start(tz, b?)))
        };
        let mut out: Vec<(i64, i64)> = match *self {
            Self::Year(y) => span(day(y, 1, 1), day(y + 1, 1, 1)).into_iter().collect(),
            Self::YearMonth(y, m) => span(month_start(y, m), next_month(y, m))
                .into_iter()
                .collect(),
            Self::Day(y, m, d) => {
                let a = day(y, m, d);
                span(a, a.and_then(|a| a.succ_opt())).into_iter().collect()
            }
            Self::MonthDay(m, d) => (SEARCH_START_YEAR..=current_year)
                .filter_map(|y| {
                    let a = day(y, m, d);
                    span(a, a.and_then(|a| a.succ_opt()))
                })
                .collect(),
            Self::Month(m) => (SEARCH_START_YEAR..=current_year)
                .filter_map(|y| span(month_start(y, m), next_month(y, m)))
                .collect(),
        };
        out.reverse();
        out
    }
}

/// Local midnight starting `d`, µs since the epoch. A midnight skipped by a
/// DST change starts at the first local time that exists that day.
fn local_start<Tz: TimeZone>(tz: &Tz, d: NaiveDate) -> i64 {
    let midnight = d.and_hms_opt(0, 0, 0).expect("midnight");
    for h in 0..=3 {
        let t = midnight + chrono::Duration::hours(h);
        if let Some(dt) = tz.from_local_datetime(&t).earliest() {
            return dt.timestamp_micros();
        }
    }
    midnight.and_utc().timestamp_micros()
}

/// `isYearSearchQuery`.
pub(crate) fn is_year_query(q: &str, current_year: i32) -> Option<i32> {
    if q.len() != 4 || !q.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i32 = q.parse().ok()?;
    (MINIMUM_SEARCH_YEAR..=current_year)
        .contains(&y)
        .then_some(y)
}

/// Months whose name starts with the (lowercase) query, in calendar order
/// (`_getMatchingMonths`).
pub(crate) fn matching_months(lower: &str) -> Vec<u32> {
    if lower.is_empty() {
        return Vec::new();
    }
    (1..=12u32)
        .filter(|m| {
            MONTH_NAMES[(*m - 1) as usize]
                .to_lowercase()
                .starts_with(lower)
        })
        .collect()
}

/// The date keys a query suggests, in the app's order: the year, matching
/// months, then the parsed date (`getYearSearchResults`,
/// `getMonthSearchResults`, `getDateResults`).
pub(crate) fn query_keys(query: &str, today: NaiveDate) -> Vec<DateKey> {
    let lower = query.trim().to_lowercase();
    let mut out = Vec::new();
    if let Some(y) = is_year_query(&lower, today.year()) {
        out.push(DateKey::Year(y));
    }
    out.extend(matching_months(&lower).into_iter().map(DateKey::Month));
    let p = parse(&lower, today);
    match (p.day, p.month, p.year) {
        (None, Some(m), Some(y)) => out.push(DateKey::YearMonth(y, m)),
        (Some(d), Some(m), Some(y)) if NaiveDate::from_ymd_opt(y, m, d).is_some() => {
            out.push(DateKey::Day(y, m, d));
        }
        // Valid in some year (29 February in leap years).
        (Some(d), Some(m), None) if NaiveDate::from_ymd_opt(2000, m, d).is_some() => {
            out.push(DateKey::MonthDay(m, d));
        }
        _ => {}
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|k| seen.insert(*k));
    out
}

/// `_looksLikeMomentQuery`: a year, a parsed day or month, or the start of
/// a month name (3+ letters).
pub(crate) fn looks_like_moment(lower: &str, today: NaiveDate) -> bool {
    if lower.is_empty() {
        return false;
    }
    if is_year_query(lower, today.year()).is_some() {
        return true;
    }
    let p = parse(lower, today);
    if !p.is_empty() && (p.day.is_some() || p.month.is_some()) {
        return true;
    }
    lower.chars().count() >= 3 && !matching_months(lower).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
    }

    fn p(s: &str) -> (Option<u32>, Option<u32>, Option<i32>) {
        let d = parse(s, today());
        (d.day, d.month, d.year)
    }

    #[test]
    fn parses_like_date_parse_service() {
        // Relative.
        assert_eq!(p("today"), (Some(4), Some(11), Some(2026)));
        assert_eq!(p("Yesterday"), (Some(3), Some(11), Some(2026)));
        assert_eq!(p("tomorrow"), (Some(5), Some(11), Some(2026)));
        assert_eq!(p("today or yesterday"), (None, None, None));
        // Structured.
        assert_eq!(p("2024-05-17"), (Some(17), Some(5), Some(2024)));
        assert_eq!(p("2024/5/7"), (Some(7), Some(5), Some(2024)));
        assert_eq!(p("2024-13-01"), (None, None, None));
        assert_eq!(p("05/2024"), (None, Some(5), Some(2024)));
        assert_eq!(p("17/05/2024"), (Some(17), Some(5), Some(2024)));
        assert_eq!(p("05/17/2024"), (Some(17), Some(5), Some(2024)));
        assert_eq!(p("17-05-24"), (Some(17), Some(5), Some(2024)));
        assert_eq!(p("17-05-99"), (Some(17), Some(5), Some(1999)));
        assert_eq!(p("17/05"), (Some(17), Some(5), None));
        assert_eq!(p("5/17"), (Some(17), Some(5), None));
        assert_eq!(p("17.05.2024"), (Some(17), Some(5), Some(2024)));
        assert_eq!(p("20240517"), (Some(17), Some(5), Some(2024)));
        assert_eq!(p("20241317"), (None, None, None));
        // Tokenized.
        assert_eq!(p("17th of May, 2024"), (Some(17), Some(5), Some(2024)));
        assert_eq!(p("May 17"), (Some(17), Some(5), None));
        assert_eq!(p("may 2024"), (None, Some(5), Some(2024)));
        assert_eq!(p("2024 sept 3rd"), (Some(3), Some(9), Some(2024)));
        assert_eq!(p("dec"), (None, Some(12), None));
        assert_eq!(p("2024"), (None, None, Some(2024)));
        assert_eq!(p("17"), (Some(17), None, None));
        assert_eq!(p("17 5 2024"), (Some(17), Some(5), Some(2024)));
        assert_eq!(p("may june 3"), (Some(3), None, None));
        assert_eq!(p("beach"), (None, None, None));
        assert_eq!(p("  "), (None, None, None));
    }

    #[test]
    fn query_keys_follow_the_app() {
        let k = |s: &str| query_keys(s, today());
        assert_eq!(k("2024"), vec![DateKey::Year(2024)]);
        assert_eq!(k("2030"), vec![]); // future year: no year result
        assert_eq!(k("ma"), vec![DateKey::Month(3), DateKey::Month(5)]);
        assert_eq!(k("may"), vec![DateKey::Month(5)]);
        assert_eq!(
            k("may 2024"),
            vec![DateKey::YearMonth(2024, 5)],
            "parsed month + year"
        );
        assert_eq!(k("17 may"), vec![DateKey::MonthDay(5, 17)]);
        assert_eq!(k("17 may 2024"), vec![DateKey::Day(2024, 5, 17)]);
        assert_eq!(k("31/02/2024"), vec![], "not a real day");
        assert_eq!(k("29/02"), vec![DateKey::MonthDay(2, 29)]);
        assert!(looks_like_moment("2024", today()));
        assert!(looks_like_moment("17 may", today()));
        assert!(looks_like_moment("sep", today()));
        assert!(!looks_like_moment("se", today()));
        assert!(!looks_like_moment("beach", today()));
    }

    #[test]
    fn keys_round_trip_and_label() {
        for k in [
            DateKey::Year(2024),
            DateKey::YearMonth(2024, 5),
            DateKey::Day(2024, 5, 17),
            DateKey::MonthDay(2, 29),
            DateKey::Month(12),
        ] {
            assert_eq!(DateKey::parse(&k.key()), Some(k));
        }
        assert_eq!(DateKey::Day(2024, 5, 17).label(), "17 May 2024");
        assert_eq!(DateKey::MonthDay(5, 17).label(), "17 May");
        assert_eq!(DateKey::YearMonth(2024, 5).label(), "May 2024");
        assert_eq!(DateKey::parse("ymd:2023-02-29"), None);
        assert_eq!(DateKey::parse("m:13"), None);
        assert_eq!(DateKey::parse("x:1"), None);
        assert_eq!(DateKey::parse("y:"), None);
    }

    #[test]
    fn ranges_are_local_days_newest_first() {
        let tz = FixedOffset::east_opt(2 * 3600).unwrap();
        let r = DateKey::Day(2024, 5, 17).ranges(&tz, 2026);
        let start = 1_715_896_800_000_000; // 2024-05-17T00:00+02:00
        assert_eq!(r, vec![(start, start + 86_400_000_000)]);
        let r = DateKey::MonthDay(2, 29).ranges(&Utc, 2026);
        // Leap years 1972..=2024.
        assert_eq!(r.len(), 14);
        assert!(r[0].0 > r[1].0);
        let r = DateKey::Month(1).ranges(&Utc, 2026);
        assert_eq!(r.len(), (2026 - 1970 + 1) as usize);
        assert_eq!(r.last().unwrap().0, 0);
        assert_eq!(r.last().unwrap().1, 31 * 86_400_000_000);
    }
}
