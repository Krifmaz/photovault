//! The rule language.
//!
//! ```text
//! # comment
//! Album name: condition and condition and ...
//! ```
//!
//! A file goes into the album when **every** condition on the line holds.
//! Several lines with the same album name are alternatives (OR). Keywords
//! (`and`, `not`, condition names) are case-insensitive.
//!
//! | Condition                | Holds when                                                      |
//! |--------------------------|-----------------------------------------------------------------|
//! | `type=image`             | file type is `image`, `video` or `live_photo`                   |
//! | `folder=<glob>`          | device folder matches (case-insensitive)                        |
//! | `name=<glob>`            | file name matches (case-insensitive)                            |
//! | `year=2024`              | capture year (UTC); ranges `2019-2021` allowed                  |
//! | `month=6`                | capture month 1-12 (UTC); ranges `6-8` allowed                  |
//! | `favorite`               | file is a favourite                                             |
//! | `archived`               | file is archived                                                |
//! | `near=<lat>,<lon>,<km>`  | file has GPS within `km` of the point (great-circle distance)   |
//! | `not <condition>`        | the condition does not hold                                     |
//!
//! Every `key=value` condition accepts alternatives separated by `|`, e.g.
//! `type=video|live_photo` or `name=*.png|*.gif`. Globs support `*` (any
//! run of characters) and `?` (one character) and must match the whole name.
//! Album names cannot contain `:`.

use photovault_plugin_pdk::abi::{FileInfo, FileType};
use photovault_plugin_pdk::time::date_from_micros;
use std::fmt;

/// One parsed rule line.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// 1-based line number in the settings text.
    pub line: usize,
    /// Album the rule fills.
    pub album: String,
    /// All must hold.
    pub conditions: Vec<Condition>,
}

/// One condition, possibly negated.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// `not <condition>`.
    Not(Box<Condition>),
    /// `type=...`.
    Type(Vec<FileType>),
    /// `folder=...` globs.
    Folder(Vec<String>),
    /// `name=...` globs.
    Name(Vec<String>),
    /// `year=...` inclusive ranges.
    Year(Vec<(i32, i32)>),
    /// `month=...` inclusive ranges.
    Month(Vec<(u32, u32)>),
    /// `favorite`.
    Favorite,
    /// `archived`.
    Archived,
    /// `near=lat,lon,km`.
    Near {
        /// Latitude in degrees.
        lat: f64,
        /// Longitude in degrees.
        lon: f64,
        /// Radius in kilometres.
        km: f64,
    },
}

/// A rule line that could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line number.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// Parse the whole rules text. Returns every error, not just the first, so
/// the user can fix them all at once.
pub fn parse_rules(text: &str) -> Result<Vec<Rule>, Vec<ParseError>> {
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_line(line) {
            Ok((album, conditions)) => rules.push(Rule {
                line: i + 1,
                album,
                conditions,
            }),
            Err(message) => errors.push(ParseError {
                line: i + 1,
                message,
            }),
        }
    }
    if errors.is_empty() {
        Ok(rules)
    } else {
        Err(errors)
    }
}

fn parse_line(line: &str) -> Result<(String, Vec<Condition>), String> {
    let (album, rest) = line
        .split_once(':')
        .ok_or("expected `Album name: condition ...`")?;
    let album = album.trim();
    if album.is_empty() {
        return Err("album name is empty".into());
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return Err(format!("rule for `{album}` has no conditions"));
    }
    let conditions = split_and(rest)
        .into_iter()
        .map(parse_condition)
        .collect::<Result<_, _>>()?;
    Ok((album.to_owned(), conditions))
}

/// Split on the word `and` (any case) surrounded by whitespace.
fn split_and(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let lower = s.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let before_ws = i > 0 && bytes[i - 1].is_ascii_whitespace();
        let after_ws = i + 3 < bytes.len() && bytes[i + 3].is_ascii_whitespace();
        if before_ws && after_ws && &bytes[i..i + 3] == b"and" {
            parts.push(s[start..i].trim());
            start = i + 3;
            i += 3;
        } else {
            i += 1;
        }
    }
    parts.push(s[start..].trim());
    parts
}

fn parse_condition(s: &str) -> Result<Condition, String> {
    if s.is_empty() {
        return Err("empty condition (a stray `and`?)".into());
    }
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("not")
        && rest.starts_with(char::is_whitespace)
    {
        // Keep the original case of the operand (globs are case-insensitive
        // anyway, but error messages read better).
        let inner = s[3..].trim_start();
        return Ok(Condition::Not(Box::new(parse_condition(inner)?)));
    }
    let Some((key, value)) = s.split_once('=') else {
        return match lower.as_str() {
            "favorite" | "favourite" => Ok(Condition::Favorite),
            "archived" => Ok(Condition::Archived),
            _ => Err(format!("unknown condition `{s}`")),
        };
    };
    let key = key.trim().to_ascii_lowercase();
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("`{key}=` needs a value"));
    }
    let alts = || value.split('|').map(str::trim);
    match key.as_str() {
        "type" => alts()
            .map(|t| match t.to_ascii_lowercase().as_str() {
                "image" | "photo" => Ok(FileType::Image),
                "video" => Ok(FileType::Video),
                "live_photo" | "live" => Ok(FileType::LivePhoto),
                _ => Err(format!(
                    "unknown type `{t}` (use image, video or live_photo)"
                )),
            })
            .collect::<Result<_, _>>()
            .map(Condition::Type),
        "folder" => Ok(Condition::Folder(alts().map(str::to_lowercase).collect())),
        "name" => Ok(Condition::Name(alts().map(str::to_lowercase).collect())),
        "year" => alts()
            .map(|v| parse_range(v, 1800, 2200, "year"))
            .collect::<Result<_, _>>()
            .map(Condition::Year),
        "month" => alts()
            .map(|v| parse_range(v, 1, 12, "month").map(|(a, b)| (a as u32, b as u32)))
            .collect::<Result<_, _>>()
            .map(Condition::Month),
        "near" => parse_near(value),
        _ => Err(format!("unknown condition `{key}`")),
    }
}

fn parse_range(v: &str, min: i32, max: i32, what: &str) -> Result<(i32, i32), String> {
    let bad = || {
        format!("{what} `{v}` must be a number from {min} to {max}, or a range like {min}-{max}")
    };
    let num = |s: &str| -> Result<i32, String> {
        let n: i32 = s.trim().parse().map_err(|_| bad())?;
        if (min..=max).contains(&n) {
            Ok(n)
        } else {
            Err(bad())
        }
    };
    let (a, b) = match v.split_once('-') {
        Some((a, b)) => (num(a)?, num(b)?),
        None => {
            let n = num(v)?;
            (n, n)
        }
    };
    if a > b {
        return Err(format!("{what} range `{v}` is backwards"));
    }
    Ok((a, b))
}

fn parse_near(v: &str) -> Result<Condition, String> {
    let parts: Vec<f64> = v
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("near=`{v}` must be <lat>,<lon>,<km>"))?;
    let [lat, lon, km] = parts[..] else {
        return Err(format!("near=`{v}` must be <lat>,<lon>,<km>"));
    };
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return Err(format!("near=`{v}`: latitude or longitude out of range"));
    }
    if !(km > 0.0 && km.is_finite()) {
        return Err(format!("near=`{v}`: radius must be greater than 0 km"));
    }
    Ok(Condition::Near { lat, lon, km })
}

impl Condition {
    /// Whether the condition holds for `file`.
    pub fn matches(&self, file: &FileInfo) -> bool {
        match self {
            Condition::Not(c) => !c.matches(file),
            Condition::Type(types) => types.contains(&file.file_type),
            Condition::Folder(globs) => file.device_folder.as_deref().is_some_and(|folder| {
                let folder = folder.to_lowercase();
                globs.iter().any(|g| glob_match(g, &folder))
            }),
            Condition::Name(globs) => {
                let name = file.title.to_lowercase();
                globs.iter().any(|g| glob_match(g, &name))
            }
            Condition::Year(ranges) => {
                let y = date_from_micros(file.creation_time).year;
                ranges.iter().any(|&(a, b)| (a..=b).contains(&y))
            }
            Condition::Month(ranges) => {
                let m = date_from_micros(file.creation_time).month;
                ranges.iter().any(|&(a, b)| (a..=b).contains(&m))
            }
            Condition::Favorite => file.is_favorite,
            Condition::Archived => file.is_archived,
            Condition::Near { lat, lon, km } => match (file.latitude, file.longitude) {
                (Some(flat), Some(flon)) => haversine_km(*lat, *lon, flat, flon) <= *km,
                _ => false,
            },
        }
    }
}

impl Rule {
    /// Whether every condition holds for `file`.
    pub fn matches(&self, file: &FileInfo) -> bool {
        self.conditions.iter().all(|c| c.matches(file))
    }
}

/// Album names (deduplicated, in rule order) that `file` belongs in.
pub fn albums_for<'r>(rules: &'r [Rule], file: &FileInfo) -> Vec<&'r str> {
    let mut out: Vec<&str> = Vec::new();
    for r in rules {
        if !out.contains(&r.album.as_str()) && r.matches(file) {
            out.push(&r.album);
        }
    }
    out
}

/// Great-circle distance in kilometres (haversine, mean Earth radius).
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6371.0088;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().min(1.0).asin()
}

/// Match `text` against a glob with `*` and `?` (whole string, by character).
/// Callers lowercase both sides for case-insensitive matching.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // Iterative matcher with single-star backtracking: O(len(p) * len(t)).
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use photovault_plugin_pdk::testing::file;

    fn one(rule: &str) -> Rule {
        parse_rules(rule).unwrap().remove(0)
    }

    #[test]
    fn globs() {
        assert!(glob_match("*dog*", "my dog.jpg"));
        assert!(glob_match("img_????.jpg", "img_0001.jpg"));
        assert!(!glob_match("img_????.jpg", "img_001.jpg"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("a*b*c", "aXXbYY"));
        assert!(glob_match("ünï*", "ünïcode"));
    }

    #[test]
    fn distance() {
        // Rome to Paris is about 1106 km.
        let d = haversine_km(41.9028, 12.4964, 48.8566, 2.3522);
        assert!((d - 1106.0).abs() < 5.0, "{d}");
        assert!(haversine_km(10.0, 10.0, 10.0, 10.0).abs() < 1e-9);
    }

    #[test]
    fn parses_and_reports_all_errors() {
        let text = "\
# my rules
Dogs: name=*dog* and type=image
  Summer: month=6-8 AND year=2020|2022-2024
Bad line without colon
: name=x
Empty:
X: colour=red
Y: month=13
Z: near=1,2
W: name=a and  and type=video
V: type=gif
";
        let errs = parse_rules(text).unwrap_err();
        let lines: Vec<usize> = errs.iter().map(|e| e.line).collect();
        assert_eq!(lines, [4, 5, 6, 7, 8, 9, 10, 11]);
        assert!(errs[3].to_string().contains("unknown condition `colour`"));

        let rules = parse_rules(
            "Dogs: name=*dog* and type=image\n#x\nS: month=6-8 AND year=2020|2022-2024",
        )
        .unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[1].line, 3);
        assert_eq!(
            rules[1].conditions,
            [
                Condition::Month(vec![(6, 8)]),
                Condition::Year(vec![(2020, 2020), (2022, 2024)])
            ]
        );
        // "and" inside a word is not a separator.
        let r = one("Band: name=*band*");
        assert_eq!(r.conditions, [Condition::Name(vec!["*band*".into()])]);
    }

    #[test]
    fn matching() {
        let mut f = file(1, "Dog_Park.JPG");
        f.device_folder = Some("Camera".into());
        f.creation_time = 1_719_792_000_000_000; // 2024-07-01
        f.latitude = Some(41.9);
        f.longitude = Some(12.5);

        assert!(one("A: name=*dog*").matches(&f));
        assert!(one("A: folder=cam*").matches(&f));
        assert!(!one("A: folder=Screenshots").matches(&f));
        assert!(one("A: year=2024 and month=7").matches(&f));
        assert!(!one("A: year=2023").matches(&f));
        assert!(one("A: near=41.89,12.49,5").matches(&f));
        assert!(!one("A: near=48.85,2.35,100").matches(&f));
        assert!(one("A: type=image|video").matches(&f));
        assert!(!one("A: type=live_photo").matches(&f));
        assert!(one("A: not favorite").matches(&f));
        assert!(!one("A: favorite").matches(&f));
        assert!(one("A: not not name=dog*").matches(&f));
        assert!(!one("A: name=*dog* and not folder=camera").matches(&f));

        f.latitude = None;
        assert!(!one("A: near=41.89,12.49,5").matches(&f));
        assert!(one("A: not near=41.89,12.49,5").matches(&f));
        f.device_folder = None;
        assert!(!one("A: folder=*").matches(&f));

        let rules = parse_rules("X: name=*dog*\nY: favorite\nX: type=image").unwrap();
        assert_eq!(albums_for(&rules, &f), ["X"]);
    }
}
