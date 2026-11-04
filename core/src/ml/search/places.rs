//! Places: Ente's location tags (user entities of type `location`) and the
//! file locations they are matched against.
//!
//! A tag is `{name, radius, aSquare, bSquare, centerPoint: {latitude,
//! longitude}}` (mobile `LocationTag`, web `RemoteLocationTagData`). A file
//! is inside a tag when it lies in the ellipse the apps draw for a circle
//! of `radius` km on the Mercator map: semi-axes `radius * sec(lat) / 111.16`
//! degrees of latitude and `radius / 111.16` degrees of longitude, exactly
//! as `isFileInsideLocationTag` (mobile `location_service.dart`) and
//! `isWithinRadius` (web `search/worker.ts`) compute it. The file's location
//! is the edited one from public magic metadata (`lat`, `long`) when both
//! are set, else the one in its metadata, and `(0, 0)` means none (web
//! `fileLocation`).
//!
//! Based on Ente (ente.com).

use serde_json::{Map, Value};

/// `kilometersPerDegree` (mobile `core/constants.dart`, web `kmsPerDegree`).
pub(crate) const KILOMETERS_PER_DEGREE: f64 = 111.16;

/// A location tag.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocationTag {
    /// Entity id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Radius in km.
    pub radius: f64,
    /// Centre latitude.
    pub latitude: f64,
    /// Centre longitude.
    pub longitude: f64,
}

impl LocationTag {
    /// Read a tag from its entity data; `None` if it does not validate
    /// like the web client's schema (name, radius and centre required).
    pub(crate) fn from_entity(id: &str, data: &Value) -> Option<Self> {
        let o = data.as_object()?;
        let c = o.get("centerPoint")?.as_object()?;
        let tag = Self {
            id: id.to_string(),
            name: o.get("name")?.as_str()?.to_string(),
            radius: o.get("radius")?.as_f64()?,
            latitude: c.get("latitude")?.as_f64()?,
            longitude: c.get("longitude")?.as_f64()?,
        };
        (tag.radius.is_finite() && tag.latitude.is_finite() && tag.longitude.is_finite())
            .then_some(tag)
    }

    /// Whether a location lies inside this tag (`isFileInsideLocationTag`).
    pub(crate) fn contains(&self, latitude: f64, longitude: f64) -> bool {
        let a = (self.radius * scale_factor(self.latitude)) / KILOMETERS_PER_DEGREE;
        let b = self.radius / KILOMETERS_PER_DEGREE;
        let x = self.latitude - latitude;
        let y = self.longitude - longitude;
        (x * x) / (a * a) + (y * y) / (b * b) <= 1.0
    }
}

/// Mercator stretches longitude by `sec(latitude)` (`scaleFactor`).
fn scale_factor(lat: f64) -> f64 {
    1.0 / (lat * (std::f64::consts::PI / 180.0)).cos()
}

/// A file's location (web `fileLocation`): edited `lat`/`long` from public
/// magic metadata if both are set, else `latitude`/`longitude` from its
/// metadata; `None` when missing, NaN or `(0, 0)`.
pub(crate) fn file_location(
    metadata: &Value,
    pub_magic: &Map<String, Value>,
) -> Option<(f64, f64)> {
    let num = |v: Option<&Value>| v.and_then(Value::as_f64);
    let (lat, long) = match (num(pub_magic.get("lat")), num(pub_magic.get("long"))) {
        (Some(a), Some(b)) => (Some(a), Some(b)),
        _ => (
            num(metadata.get("latitude")),
            num(metadata.get("longitude")),
        ),
    };
    let (lat, long) = (lat?, long?);
    if lat.is_nan() || long.is_nan() || (lat == 0.0 && long == 0.0) {
        return None;
    }
    Some((lat, long))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tag(lat: f64, long: f64, radius: f64) -> LocationTag {
        LocationTag::from_entity(
            "t",
            &json!({"name": "Home", "radius": radius, "aSquare": 0.0, "bSquare": 0.0,
                    "centerPoint": {"latitude": lat, "longitude": long}}),
        )
        .unwrap()
    }

    #[test]
    fn ellipse_matches_the_apps() {
        // Lisbon, 2 km.
        let t = tag(38.7223, -9.1393, 2.0);
        assert!(t.contains(38.7223, -9.1393));
        // Along latitude the semi-axis is radius * sec(lat) / 111.16 degrees
        // (≈ 0.02312° here), along longitude radius / 111.16 (≈ 0.01799°).
        let a = 2.0 / (38.7223f64.to_radians().cos()) / KILOMETERS_PER_DEGREE;
        let b = 2.0 / KILOMETERS_PER_DEGREE;
        assert!(t.contains(38.7223 + a * 0.999, -9.1393));
        assert!(!t.contains(38.7223 + a * 1.001, -9.1393));
        assert!(t.contains(38.7223, -9.1393 - b * 0.999));
        assert!(!t.contains(38.7223, -9.1393 - b * 1.001));
        // Inside the latitude reach but outside the ellipse diagonally.
        assert!(!t.contains(38.7223 + a * 0.8, -9.1393 + b * 0.8));
        assert!(t.contains(38.7223 + a * 0.7, -9.1393 + b * 0.7));
        // Equator: a == b.
        let e = tag(0.0, 10.0, 11.116);
        assert!(e.contains(0.0999, 10.0));
        assert!(!e.contains(0.1001, 10.0));
    }

    #[test]
    fn invalid_tags_are_skipped() {
        assert!(LocationTag::from_entity("x", &json!({"name": "a"})).is_none());
        assert!(
            LocationTag::from_entity(
                "x",
                &json!({"name": "a", "radius": "2", "centerPoint": {"latitude": 1, "longitude": 2}})
            )
            .is_none()
        );
    }

    #[test]
    fn file_location_prefers_edited_and_skips_null_island() {
        let md = json!({"latitude": 1.5, "longitude": 2.5});
        assert_eq!(file_location(&md, &Map::new()), Some((1.5, 2.5)));
        let mut pm = Map::new();
        pm.insert("lat".into(), json!(3.0));
        pm.insert("long".into(), json!(4.0));
        assert_eq!(file_location(&md, &pm), Some((3.0, 4.0)));
        pm.remove("long");
        assert_eq!(file_location(&md, &pm), Some((1.5, 2.5)));
        assert_eq!(
            file_location(&json!({"latitude": 0.0, "longitude": 0.0}), &Map::new()),
            None
        );
        assert_eq!(file_location(&json!({"latitude": 1.0}), &Map::new()), None);
    }
}
