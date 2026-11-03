//! Parsers for the metadata files in an iCloud Photos export.
//!
//! - `Photos/Photo Details*.csv`: one row per file with the header
//!   `imgName,fileChecksum,favorite,hidden,deleted,originalCreationDate,viewCount,importDate`.
//! - `Albums/<album>.csv` and `Memories/<memory>.csv`: one column
//!   (`imgName`, sometimes `imageName` or `Images`) listing member files.
//! - `Shared Albums/<album>/AlbumInfo.json`: `albumName`, `creationDate` and
//!   `photos: [{name, dateCreated, contributor, comments}]`.
//!
//! Dates look like `Saturday June 4,2022 12:11 PM GMT`.

use chrono::{FixedOffset, NaiveDateTime, TimeZone as _};

/// One row of a `Photo Details` CSV.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PhotoRow {
    /// File name as the user knew it (before any `-1` de-duplication suffix).
    pub img_name: String,
    /// Apple's base64 file checksum (not a plain hash of the file). Stable
    /// across exports, so we use it as the journal id.
    pub checksum: Option<String>,
    /// `favorite == yes`.
    pub favorite: bool,
    /// `hidden == yes`.
    pub hidden: bool,
    /// `deleted == yes` (in Recently Deleted).
    pub deleted: bool,
    /// `originalCreationDate`, µs since the epoch.
    pub original_creation_us: Option<i64>,
}

/// Normalise a CSV header for comparison (`imgName` == `img name` == `IMGNAME`).
fn norm_header(h: &str) -> String {
    h.trim_start_matches('\u{feff}')
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn yes(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "yes" | "true" | "1" | "y"
    )
}

/// Parse a `Photo Details` CSV. Unknown columns are ignored; an empty file
/// (Apple sometimes ships those) yields no rows.
pub(crate) fn parse_photo_details(data: &[u8]) -> anyhow::Result<Vec<PhotoRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(true)
        .from_reader(data);
    let headers: Vec<String> = match rdr.headers() {
        Ok(h) => h.iter().map(norm_header).collect(),
        Err(_) => return Ok(Vec::new()),
    };
    let col = |name: &str| headers.iter().position(|h| h == name);
    let Some(name_col) = col("imgname").or(col("imagename")) else {
        return Ok(Vec::new());
    };
    let (ck, fav, hid, del, date) = (
        col("filechecksum"),
        col("favorite"),
        col("hidden"),
        col("deleted"),
        col("originalcreationdate"),
    );
    let mut rows = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let get = |c: Option<usize>| c.and_then(|c| rec.get(c)).unwrap_or("");
        let img_name = get(Some(name_col)).trim().to_string();
        if img_name.is_empty() {
            continue;
        }
        let checksum = Some(get(ck).trim().to_string()).filter(|s| !s.is_empty());
        rows.push(PhotoRow {
            img_name,
            checksum,
            favorite: yes(get(fav)),
            hidden: yes(get(hid)),
            deleted: yes(get(del)),
            original_creation_us: parse_icloud_date(get(date)),
        });
    }
    Ok(rows)
}

/// Parse an album or memory CSV: the file names it lists.
pub(crate) fn parse_member_list(data: &[u8]) -> anyhow::Result<Vec<String>> {
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(true)
        .from_reader(data);
    let headers: Vec<String> = match rdr.headers() {
        Ok(h) => h.iter().map(norm_header).collect(),
        Err(_) => return Ok(Vec::new()),
    };
    let col = headers
        .iter()
        .position(|h| matches!(h.as_str(), "imgname" | "imagename" | "images" | "filename"))
        .unwrap_or(0);
    let mut names = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        if let Some(n) = rec.get(col).map(str::trim).filter(|n| !n.is_empty()) {
            names.push(n.to_string());
        }
    }
    Ok(names)
}

/// A shared album's `AlbumInfo.json`.
#[derive(Debug, Clone, Default)]
pub(crate) struct SharedAlbumInfo {
    /// `albumName`.
    pub name: Option<String>,
    /// `photos[]`.
    pub photos: Vec<SharedPhoto>,
}

/// One entry of `AlbumInfo.json` `photos`.
#[derive(Debug, Clone, Default)]
pub(crate) struct SharedPhoto {
    /// Full original name (the file on disk may be truncated).
    pub name: String,
    /// `dateCreated`, µs since the epoch.
    pub date_us: Option<i64>,
    /// A caption, if the export carries one (`caption` or `description`).
    pub caption: Option<String>,
}

/// Parse `AlbumInfo.json`, tolerating missing fields.
pub(crate) fn parse_album_info(data: &[u8]) -> anyhow::Result<SharedAlbumInfo> {
    let v: serde_json::Value = serde_json::from_slice(data)?;
    let s = |v: &serde_json::Value, k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
    };
    let photos = v
        .get("photos")
        .and_then(|p| p.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(SharedPhoto {
                        name: s(p, "name")?,
                        date_us: s(p, "dateCreated").and_then(|d| parse_icloud_date(&d)),
                        caption: s(p, "caption").or_else(|| s(p, "description")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(SharedAlbumInfo {
        name: s(&v, "albumName"),
        photos,
    })
}

const WEEKDAYS: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "mon",
    "tue",
    "tues",
    "wed",
    "thu",
    "thur",
    "thurs",
    "fri",
    "sat",
    "sun",
];

/// Parse an iCloud export date such as `Saturday June 4,2022 12:11 PM GMT`
/// into µs since the epoch.
///
/// Tolerates: a missing or abbreviated weekday, `4,2022` or `4, 2022`,
/// abbreviated months, 12- or 24-hour clocks, optional seconds, and a zone of
/// `GMT`, `UTC`, `Z`, `GMT+2`, `GMT+02:00`, `+0200` or `+02:00`. Apple writes
/// `GMT`, and the times are UTC; a date without a zone is also read as UTC.
pub fn parse_icloud_date(s: &str) -> Option<i64> {
    let s = s.trim().trim_matches('"');
    if s.is_empty() {
        return None;
    }
    let s = s.replace(',', ", ");
    let mut tokens: Vec<&str> = s.split_whitespace().collect();
    if tokens
        .first()
        .is_some_and(|t| WEEKDAYS.contains(&t.trim_end_matches(',').to_ascii_lowercase().as_str()))
    {
        tokens.remove(0);
    }
    // Zone: last token unless it is AM/PM or part of the time.
    let mut offset = FixedOffset::east_opt(0)?;
    if let Some(last) = tokens.last() {
        if let Some(off) = parse_zone(last) {
            offset = off;
            tokens.pop();
        } else if last.chars().any(|c| c.is_ascii_alphabetic())
            && !matches!(last.to_ascii_uppercase().as_str(), "AM" | "PM")
        {
            // An unknown zone name: ignore it and keep UTC.
            tokens.pop();
        }
    }
    let body = tokens.join(" ");
    const FORMATS: &[&str] = &[
        "%B %d, %Y %I:%M %p",
        "%B %d, %Y %I:%M:%S %p",
        "%B %d, %Y %H:%M",
        "%B %d, %Y %H:%M:%S",
        "%b %d, %Y %I:%M %p",
        "%b %d, %Y %I:%M:%S %p",
        "%b %d, %Y %H:%M",
        "%b %d, %Y %H:%M:%S",
    ];
    let naive = FORMATS
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(&body, f).ok())?;
    let ts = offset
        .from_local_datetime(&naive)
        .single()?
        .timestamp_micros();
    (ts != 0).then_some(ts)
}

/// `GMT`, `UTC`, `Z`, `GMT+2`, `UTC-05:30`, `+0200`, `+02:00`.
fn parse_zone(t: &str) -> Option<FixedOffset> {
    let u = t.to_ascii_uppercase();
    let rest = u
        .strip_prefix("GMT")
        .or_else(|| u.strip_prefix("UTC"))
        .unwrap_or(&u);
    if rest.is_empty() || rest == "Z" {
        return (u != rest || rest == "Z")
            .then(|| FixedOffset::east_opt(0))
            .flatten();
    }
    let (sign, digits) = match rest.as_bytes()[0] {
        b'+' => (1, &rest[1..]),
        b'-' => (-1, &rest[1..]),
        _ => return None,
    };
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) => (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?),
        None if digits.len() <= 2 => (digits.parse::<i32>().ok()?, 0),
        None if digits.len() == 4 => (digits[..2].parse().ok()?, digits[2..].parse().ok()?),
        None => return None,
    };
    if h > 14 || m > 59 {
        return None;
    }
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        chrono::Utc
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp_micros()
    }

    #[test]
    fn apple_dates() {
        assert_eq!(
            parse_icloud_date("Saturday June 4,2022 12:11 PM GMT"),
            Some(utc(2022, 6, 4, 12, 11))
        );
        assert_eq!(
            parse_icloud_date("Thursday August 2,2018 11:19 AM GMT"),
            Some(utc(2018, 8, 2, 11, 19))
        );
        assert_eq!(
            parse_icloud_date("Wednesday August 25,2021 3:35 PM GMT"),
            Some(utc(2021, 8, 25, 15, 35))
        );
        assert_eq!(
            parse_icloud_date("Friday January 8,2021 12:16 AM GMT"),
            Some(utc(2021, 1, 8, 0, 16))
        );
        assert_eq!(
            parse_icloud_date("Mon Aug 17, 2015 12:52 PM UTC+02:00"),
            Some(utc(2015, 8, 17, 10, 52))
        );
        assert_eq!(
            parse_icloud_date("August 17,2015 13:52 GMT-1"),
            Some(utc(2015, 8, 17, 14, 52))
        );
        assert_eq!(
            parse_icloud_date("August 17,2015 13:52"),
            Some(utc(2015, 8, 17, 13, 52))
        );
        assert_eq!(parse_icloud_date(""), None);
        assert_eq!(parse_icloud_date("garbage"), None);
    }

    #[test]
    fn details_csv() {
        let csv = "\u{feff}imgName,fileChecksum,favorite,hidden,deleted,originalCreationDate,viewCount,importDate\n\
            IMG_7938.HEIC,AfQj57ORF2JIumUCjO+PawZ9nqPg,yes,no,no,\"Saturday June 4,2022 12:11 PM GMT\",10,\"Saturday June 4,2022 12:11 PM GMT\"\n\
            IMG_7939.MOV,,no,yes,yes,,0,\n";
        let rows = parse_photo_details(csv.as_bytes()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].img_name, "IMG_7938.HEIC");
        assert!(rows[0].favorite && !rows[0].hidden && !rows[0].deleted);
        assert_eq!(
            rows[0].checksum.as_deref(),
            Some("AfQj57ORF2JIumUCjO+PawZ9nqPg")
        );
        assert_eq!(rows[0].original_creation_us, Some(utc(2022, 6, 4, 12, 11)));
        assert!(rows[1].hidden && rows[1].deleted && rows[1].checksum.is_none());
        assert!(parse_photo_details(b"").unwrap().is_empty());
    }

    #[test]
    fn member_lists() {
        assert_eq!(
            parse_member_list(b"imgName\nphoto1.jpg\nphoto2.jpg\n").unwrap(),
            vec!["photo1.jpg", "photo2.jpg"]
        );
        assert_eq!(
            parse_member_list(b"Images\na.jpg\n").unwrap(),
            vec!["a.jpg"]
        );
        assert!(parse_member_list(b"").unwrap().is_empty());
    }

    #[test]
    fn album_info() {
        let j = br#"{"albumName":"Family","creationDate":"Monday August 17,2015 12:52 PM GMT",
            "allowContributions":false,"isPublic":false,"owner":"x","participants":[],
            "photos":[{"name":"IMG_0003.JPG","dateCreated":"Friday January 8,2021 1:16 AM GMT","contributor":"x","comments":[]}]}"#;
        let a = parse_album_info(j).unwrap();
        assert_eq!(a.name.as_deref(), Some("Family"));
        assert_eq!(a.photos.len(), 1);
        assert_eq!(a.photos[0].date_us, Some(utc(2021, 1, 8, 1, 16)));
    }
}
