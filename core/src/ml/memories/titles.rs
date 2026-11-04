//! Memory titles: the English strings of Ente's photos app
//! (`createTitle` of each memory model, `strings_en.arb`).
//!
//! Based on Ente (ente.com).

use super::cache::StoredKind;

const MONTHS: [&str; 12] = [
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

/// `{count} year(s) ago`.
fn years_ago(count: i32) -> String {
    if count == 1 {
        "1 year ago".to_string()
    } else {
        format!("{count} years ago")
    }
}

fn month_name(month: i32) -> &'static str {
    MONTHS[(month.clamp(1, 12) - 1) as usize]
}

/// `DateFormat.MMMd("en")`: "Nov 4".
fn month_day(month: i32, day: i32) -> String {
    format!("{} {day}", &month_name(month)[..3])
}

/// The title of a memory. `person_name` is the person's current name
/// (`None` for an unnamed cluster or a person no longer known);
/// `current_year` the viewer's local year.
pub(crate) fn title(
    kind: &StoredKind,
    person_name: Option<&str>,
    is_me: bool,
    current_year: i32,
) -> String {
    match kind {
        StoredKind::OnThisDay => "On this day".to_string(),
        StoredKind::Filler { years_ago: n } => years_ago(*n),
        StoredKind::LastWeek => "Last week".to_string(),
        StoredKind::LastMonth => "Last month".to_string(),
        StoredKind::TimeDay {
            month,
            day,
            years_ago: n,
            ..
        } => {
            let date = month_day(*month, *day);
            match n {
                Some(n) => format!("{date}, {}", years_ago(*n)),
                None => format!("{date} through the years"),
            }
        }
        StoredKind::TimeMonth {
            month,
            years_ago: n,
            ..
        } => {
            let name = month_name(*month);
            match n {
                Some(n) => format!("{name}, {}", years_ago(*n)),
                None => format!("{name} through the years"),
            }
        }
        StoredKind::TimeWeek { years_ago: Some(1) } => "This week, 1 year ago".to_string(),
        StoredKind::TimeWeek { years_ago: Some(n) } => format!("This week, {n} years ago"),
        StoredKind::TimeWeek { years_ago: None } => "This week through the years".to_string(),
        StoredKind::Trip { name, year, .. } => match name {
            Some(name) => format!("Trip to {name}"),
            None if *year == current_year - 1 => "Last year's trip".to_string(),
            None => format!("Trip in {year}"),
        },
        StoredKind::Clip { clip } => clip_title(clip).to_string(),
        StoredKind::People {
            unnamed,
            memory_type,
            activity,
            birthday,
            ..
        } => {
            let name = person_name.filter(|_| !*unnamed);
            people_title(memory_type, activity.as_deref(), *birthday, name, is_me)
        }
    }
}

fn people_title(
    memory_type: &str,
    activity: Option<&str>,
    birthday: Option<(bool, i32)>,
    name: Option<&str>,
    is_me: bool,
) -> String {
    let Some(name) = name else {
        // Ente's unnamed-cluster strings (also used when the person is gone).
        return match memory_type {
            "YouAndThem" => "You two",
            "DoingSomethingTogether" => "Doing it together",
            "LastTimeYouSawThem" => "Throwback together",
            _ if is_me => "Spotlight on yourself",
            _ => "A familiar face",
        }
        .to_string();
    };
    match memory_type {
        "YouAndThem" => format!("You and {name}"),
        "DoingSomethingTogether" => activity_title(activity.unwrap_or_default(), name),
        "LastTimeYouSawThem" => format!("Throwback with {name}"),
        _ if is_me => "Spotlight on yourself".to_string(),
        _ => match birthday {
            None => format!("Spotlight on {name}"),
            Some((true, age)) => format!("{name} turning {age} soon"),
            Some((false, age)) => format!("{name} is {age}!"),
        },
    }
}

fn activity_title(activity: &str, name: &str) -> String {
    match activity {
        "Admiring" => format!("Admiring {name}"),
        "Embracing" => format!("Embracing {name}"),
        "Party" => format!("Party with {name}"),
        "Hiking" => format!("Hiking with {name}"),
        "Feast" => format!("Feasting with {name}"),
        "Selfies" => format!("Selfies with {name}"),
        "Posing" => format!("Posing with {name}"),
        "Background" => format!("Beautiful views with {name}"),
        "Sports" => format!("Sports with {name}"),
        "Roadtrip" => format!("Road trip with {name}"),
        _ => format!("You and {name}"),
    }
}

fn clip_title(clip: &str) -> &'static str {
    match clip {
        "Sunrise" => "On the horizon",
        "Mountains" => "Over the hills",
        "Greenery" => "The green life",
        "Beach" => "Sand and sea",
        "City" => "In the city",
        "Moon" => "In the moonlight",
        "OnTheRoad" => "On the road again",
        "Food" => "Culinary delight",
        "Pets" => "Furry companions",
        "Festivities" => "Festive cheer",
        "SnowAdventures" => "Winter adventures",
        "Waterfalls" => "Chasing waterfalls",
        "Wildlife" => "Into the wild",
        "Flowers" => "Blooming beauties",
        "NightLights" => "Neon nights",
        "Architecture" => "Design icons",
        "AutumnColors" => "Autumn splendor",
        "DesertDreams" => "Desert dreams",
        "Stargazing" => "Under the stars",
        "Lakeside" => "Lakeside calm",
        "RainyDays" => "Rainy day feels",
        "SportsAction" => "Game on",
        "StreetArt" => "Street art stories",
        "FamilyMoments" => "Family time",
        "Fireworks" => "Firework finale",
        "HistoricSites" => "Echoes of history",
        "TropicalParadise" => "Tropical escapes",
        "ForestTrails" => "Forest strolls",
        "CitySunsets" => "City sunset glow",
        "ColorfulMarkets" => "Vibrant markets",
        "CozyCafes" => "Cozy cafe corners",
        "VintageVibes" => "Vintage vibes",
        "AerialViews" => "Aerial perspectives",
        "ArtisticPortraits" => "Artful portraits",
        "StreetFood" => "Street food bites",
        "RiverCruises" => "River cruise views",
        "PlayfulKids" => "Playful kids",
        "CoastalCliffs" => "Cliffs by the sea",
        "CandidLaughter" => "Burst of laughter",
        "GroupSelfies" => "All of us",
        "BirthdayJoy" => "Birthday joy",
        "WeddingMoments" => "Wedding day glow",
        "GoldenHourPortraits" => "Golden hour faces",
        "ConcertNights" => "Concert nights",
        "PoolDays" => "Poolside days",
        "PicnicDays" => "Picnic memories",
        "CampingMoments" => "Campfire moments",
        "BabySmiles" => "Baby smiles",
        _ => "Memories",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ml::memories::engine::ClipMemoryType;

    #[test]
    fn english_titles() {
        let t = |k: StoredKind| title(&k, Some("Ana"), false, 2026);
        assert_eq!(t(StoredKind::OnThisDay), "On this day");
        assert_eq!(t(StoredKind::Filler { years_ago: 1 }), "1 year ago");
        assert_eq!(
            t(StoredKind::TimeDay {
                year: 2020,
                month: 11,
                day: 4,
                years_ago: Some(6)
            }),
            "Nov 4, 6 years ago"
        );
        assert_eq!(
            t(StoredKind::TimeMonth {
                year: 2026,
                month: 5,
                years_ago: None
            }),
            "May through the years"
        );
        assert_eq!(
            t(StoredKind::TimeWeek { years_ago: Some(2) }),
            "This week, 2 years ago"
        );
        let trip = |name: Option<&str>, year| StoredKind::Trip {
            key: "k".into(),
            latitude: 0.0,
            longitude: 0.0,
            name: name.map(str::to_owned),
            year,
        };
        assert_eq!(t(trip(Some("Lisbon"), 2020)), "Trip to Lisbon");
        assert_eq!(t(trip(None, 2025)), "Last year's trip");
        assert_eq!(t(trip(None, 2021)), "Trip in 2021");
        let people = |memory_type: &str, birthday| StoredKind::People {
            person_id: "p".into(),
            unnamed: false,
            memory_type: memory_type.into(),
            activity: Some("Hiking".into()),
            birthday,
        };
        assert_eq!(t(people("Spotlight", None)), "Spotlight on Ana");
        assert_eq!(
            t(people("Spotlight", Some((true, 30)))),
            "Ana turning 30 soon"
        );
        assert_eq!(t(people("Spotlight", Some((false, 30)))), "Ana is 30!");
        assert_eq!(t(people("YouAndThem", None)), "You and Ana");
        assert_eq!(t(people("DoingSomethingTogether", None)), "Hiking with Ana");
        assert_eq!(
            title(&people("Spotlight", None), Some("Me"), true, 2026),
            "Spotlight on yourself"
        );
        assert_eq!(
            title(&people("LastTimeYouSawThem", None), None, false, 2026),
            "Throwback together"
        );
        for kind in ClipMemoryType::ALL {
            assert_ne!(
                t(StoredKind::Clip {
                    clip: format!("{kind:?}")
                }),
                "Memories"
            );
        }
    }
}
