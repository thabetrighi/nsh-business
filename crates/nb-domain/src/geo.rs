//! Places on the map for distribution (M16): a point kept as whole
//! micro-degrees (no floating point stored), the distance between two
//! points, the order a tour visits its stops in, and when it should get
//! there. Geometry only — no money or quantity here.

use crate::RuleError;

/// A place: latitude and longitude in millionths of a degree (about 11 cm).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Point {
    pub lat_e6: i64,
    pub lng_e6: i64,
}

impl Point {
    pub fn new(lat_e6: i64, lng_e6: i64) -> Result<Self, RuleError> {
        if !(-90_000_000..=90_000_000).contains(&lat_e6)
            || !(-180_000_000..=180_000_000).contains(&lng_e6)
        {
            return Err(RuleError::on("position", "geo.out_of_range"));
        }
        Ok(Point { lat_e6, lng_e6 })
    }

    /// From stored columns (both or none).
    pub fn of(lat: Option<i64>, lng: Option<i64>) -> Option<Point> {
        match (lat, lng) {
            (Some(lat_e6), Some(lng_e6)) => Point::new(lat_e6, lng_e6).ok(),
            _ => None,
        }
    }

    /// "36.752887, 3.042048" (what maps show and copy).
    pub fn text(self) -> String {
        format!("{}, {}", decimal(self.lat_e6), decimal(self.lng_e6))
    }

    /// A link that opens the place in a map (any phone or browser).
    pub fn map_url(self) -> String {
        format!(
            "https://www.google.com/maps/search/?api=1&query={},{}",
            decimal(self.lat_e6),
            decimal(self.lng_e6)
        )
    }

    /// Directions from wherever the phone is to here.
    pub fn directions_url(self) -> String {
        format!(
            "https://www.google.com/maps/dir/?api=1&destination={},{}",
            decimal(self.lat_e6),
            decimal(self.lng_e6)
        )
    }
}

fn decimal(e6: i64) -> String {
    let sign = if e6 < 0 { "-" } else { "" };
    let a = e6.unsigned_abs();
    format!("{sign}{}.{:06}", a / 1_000_000, a % 1_000_000)
}

/// A decimal degree ("36.75", "-0.6417") as micro-degrees, without
/// floating point.
fn parse_degrees(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = int.parse().ok()?;
    let mut digits: String = frac.chars().take(6).collect();
    while digits.len() < 6 {
        digits.push('0');
    }
    let micro: i64 = digits.parse().ok()?;
    let v = whole.checked_mul(1_000_000)?.checked_add(micro)?;
    Some(if neg { -v } else { v })
}

/// A place typed or pasted: "36.75, 3.06", "36.75 3.06", or a map link
/// (`…/@36.75,3.06,15z`, `…?q=36.75,3.06`, `…&query=36.75,3.06`,
/// `…?ll=36.75,3.06`, `geo:36.75,3.06`).
pub fn parse_point(text: &str) -> Result<Point, RuleError> {
    let t = text.trim();
    let bad = || RuleError::on("position", "geo.unreadable");
    let candidates: Vec<&str> = {
        let mut v = Vec::new();
        if let Some((_, after)) = t.split_once('@') {
            v.push(after);
        }
        for key in ["query=", "q=", "ll=", "destination=", "geo:"] {
            if let Some((_, after)) = t.split_once(key) {
                v.push(after);
            }
        }
        v.push(t);
        v
    };
    for c in candidates {
        let c = c.split(['&', '/', '?', ';']).next().unwrap_or("");
        let mut parts = c
            .split(|ch: char| ch == ',' || ch.is_whitespace())
            .filter(|p| !p.is_empty());
        let (Some(a), Some(b)) = (parts.next(), parts.next()) else {
            continue;
        };
        if let (Some(lat), Some(lng)) = (parse_degrees(a), parse_degrees(b)) {
            return Point::new(lat, lng).map_err(|_| bad());
        }
    }
    Err(bad())
}

/// Metres between two places (great-circle distance).
// Trigonometry needs floating point; it measures roads, never money or
// quantities, and its result is rounded to whole metres before leaving.
#[allow(clippy::float_arithmetic)]
pub fn distance_m(a: Point, b: Point) -> u64 {
    const R: f64 = 6_371_000.0;
    let rad = |e6: i64| (e6 as f64 / 1_000_000.0).to_radians();
    let (la1, la2) = (rad(a.lat_e6), rad(b.lat_e6));
    let dla = la2 - la1;
    let dlo = rad(b.lng_e6) - rad(a.lng_e6);
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    let d = 2.0 * R * h.sqrt().min(1.0).asin();
    d.round().max(0.0) as u64
}

/// Minutes to drive `metres` at `kmh` (roads wind: a straight line times
/// 1.3), at least one minute when not there yet.
pub fn drive_minutes(metres: u64, kmh: u32) -> u32 {
    if metres == 0 {
        return 0;
    }
    let kmh = u64::from(kmh.max(5));
    let road = metres.saturating_mul(13) / 10;
    // minutes = metres / (kmh × 1000 / 60), rounded up.
    let per_min = kmh * 1000;
    u32::try_from((road * 60).div_ceil(per_min).max(1)).unwrap_or(u32::MAX)
}

/// The order to visit `stops` in, starting from `start` (the depot or the
/// van): each time the nearest place not yet visited. Stops with no place
/// keep their order, after the others.
pub fn visit_order(start: Option<Point>, stops: &[Option<Point>]) -> Vec<usize> {
    let mut left: Vec<usize> = (0..stops.len()).filter(|i| stops[*i].is_some()).collect();
    let mut out = Vec::with_capacity(stops.len());
    let mut here = start.or_else(|| left.first().and_then(|i| stops[*i]));
    while !left.is_empty() {
        let Some(h) = here else { break };
        let (k, _) = left
            .iter()
            .enumerate()
            .filter_map(|(k, i)| stops[*i].map(|p| (k, distance_m(h, p))))
            .min_by_key(|(_, d)| *d)
            .unwrap_or((0, 0));
        let i = left.remove(k);
        here = stops[i];
        out.push(i);
    }
    out.extend((0..stops.len()).filter(|i| stops[*i].is_none()));
    out
}

/// The length of a tour visiting `stops` in their order from `start`.
pub fn route_m(start: Option<Point>, stops: &[Option<Point>]) -> u64 {
    let mut total = 0;
    let mut here = start;
    for p in stops.iter().flatten() {
        if let Some(h) = here {
            total += distance_m(h, *p);
        }
        here = Some(*p);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A place in micro-degrees.
    fn p(lat_e6: i64, lng_e6: i64) -> Point {
        Point::new(lat_e6, lng_e6).unwrap_or(Point {
            lat_e6: 0,
            lng_e6: 0,
        })
    }

    #[test]
    fn places_typed_or_pasted() -> Result<(), RuleError> {
        let algiers = Point::new(36_752_887, 3_042_048)?;
        assert_eq!(parse_point("36.752887, 3.042048")?, algiers);
        assert_eq!(parse_point("36.752887 3.042048")?, algiers);
        assert_eq!(
            parse_point("https://www.google.com/maps/@36.752887,3.042048,15z")?,
            algiers
        );
        assert_eq!(
            parse_point("https://maps.google.com/?q=36.752887,3.042048")?,
            algiers
        );
        assert_eq!(parse_point("geo:36.752887,3.042048")?, algiers);
        assert_eq!(parse_point(&algiers.text())?, algiers);
        assert_eq!(
            parse_point("35.7, -0.6417")?,
            Point::new(35_700_000, -641_700)?
        );
        assert!(parse_point("near the mosque").is_err());
        assert!(parse_point("95.0, 3.0").is_err());
        assert!(algiers.map_url().contains("36.752887,3.042048"));
        Ok(())
    }

    #[test]
    fn distances_and_times() {
        // Algiers – Blida, about 41 km as the crow flies.
        let d = distance_m(p(36_752_900, 3_042_000), p(36_470_100, 2_827_700));
        assert!((35_000..=40_000).contains(&d), "{d}");
        assert_eq!(
            distance_m(p(36_000_000, 3_000_000), p(36_000_000, 3_000_000)),
            0
        );
        // 10 km at 30 km/h on roads (13 km): 26 minutes.
        assert_eq!(drive_minutes(10_000, 30), 26);
        assert_eq!(drive_minutes(0, 30), 0);
        assert_eq!(drive_minutes(10, 30), 1);
    }

    #[test]
    fn the_nearest_next() {
        let depot = p(36_700_000, 3_000_000);
        let stops = [
            Some(p(36_900_000, 3_000_000)), // far
            None,                           // no place
            Some(p(36_710_000, 3_000_000)), // nearest
            Some(p(36_800_000, 3_000_000)), // between
        ];
        assert_eq!(visit_order(Some(depot), &stops), vec![2, 3, 0, 1]);
        let ordered: Vec<Option<Point>> = visit_order(Some(depot), &stops)
            .iter()
            .map(|i| stops[*i])
            .collect();
        assert!(route_m(Some(depot), &ordered) < route_m(Some(depot), &stops));
        assert_eq!(visit_order(None, &[None, None]), vec![0, 1]);
    }
}
