//! SAMPLE DATA for the screens until the sales, finance and catalog modules
//! (M5, M8, M9) serve real projections from SQLite. Deterministic, so
//! screenshots and tests are stable. Amounts are integer centimes.

use nb_i18n::Lang;

/// "Today" for the samples.
pub const TODAY: (i32, u32, u32) = (2026, 11, 10);

/// The period a dashboard shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Week,
    Month,
    Year,
}

impl Period {
    pub fn key(self) -> &'static str {
        match self {
            Period::Week => "period.7d",
            Period::Month => "period.30d",
            Period::Year => "period.12m",
        }
    }
    /// Points in the chart.
    pub fn points(self) -> usize {
        match self {
            Period::Week => 7,
            Period::Month => 30,
            Period::Year => 12,
        }
    }
}

/// A small deterministic generator (LCG).
struct Rng(u64);

impl Rng {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n.max(1)
    }
}

fn days_in(month: u32) -> u32 {
    match month {
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// (month, day) `back` days before today.
pub fn day_back(back: u32) -> (u32, u32) {
    let (_, mut m, mut d) = TODAY;
    for _ in 0..back {
        if d > 1 {
            d -= 1;
        } else {
            m = if m == 1 { 12 } else { m - 1 };
            d = days_in(m);
        }
    }
    (m, d)
}

/// Month number `back` months before this one.
pub fn month_back(back: u32) -> u32 {
    let m = TODAY.1 as i32 - back as i32;
    m.rem_euclid(12) as u32 + if m.rem_euclid(12) == 0 { 12 } else { 0 }
}

/// Sales per point (oldest first), in centimes, for this period and the one before.
pub fn sales(period: Period) -> (Vec<i64>, Vec<i64>) {
    let n = period.points();
    let mut rng = Rng(match period {
        Period::Week => 7,
        Period::Month => 30,
        Period::Year => 12,
    });
    let one = |rng: &mut Rng, i: usize, prev: bool| -> i64 {
        match period {
            Period::Year => {
                // Seasonal: Ramadan and the back-to-school months sell more.
                let base = 4_200_000 + rng.next(1_400_000) as i64 + (i as i64) * 90_000;
                let base = if prev { base * 88 / 100 } else { base };
                base * 100
            }
            _ => {
                let weekday = (i + 3) % 7;
                let base = 150_000 + rng.next(70_000) as i64 + (i as i64) * 1_100;
                let base = if weekday == 4 { base * 72 / 100 } else { base };
                let base = if prev { base * 90 / 100 } else { base };
                base * 100
            }
        }
    };
    let cur: Vec<i64> = (0..n).map(|i| one(&mut rng, i, false)).collect();
    let prev: Vec<i64> = (0..n).map(|i| one(&mut rng, i, true)).collect();
    (cur, prev)
}

/// Axis labels for the period (oldest first).
pub fn labels(period: Period, fmt: &nb_i18n::Format) -> Vec<String> {
    let n = period.points() as u32;
    (0..n)
        .rev()
        .map(|back| match period {
            Period::Year => fmt.month_short(month_back(back)).to_owned(),
            _ => {
                let (m, d) = day_back(back);
                fmt.day_month(m, d)
            }
        })
        .collect()
}

pub fn pick(lang: Lang, ar: &str, fr: &str) -> String {
    if lang == Lang::Ar {
        ar.to_owned()
    } else {
        fr.to_owned()
    }
}

/// Sales share per category: (ar, fr, centimes).
pub const CATEGORY_SALES: [(&str, &str, i64); 5] = [
    ("ألبان", "Produits laitiers", 168_420_000),
    ("زيوت وسكر", "Huiles et sucre", 121_780_000),
    ("مشروبات", "Boissons", 97_350_000),
    ("مخبوزات", "Boulangerie", 64_900_000),
    ("منظفات", "Entretien", 38_150_000),
];

/// Payment methods: (i18n key, centimes).
pub const PAYMENTS: [(&str, i64); 3] = [
    ("pay.cash", 318_400_000),
    ("pay.card", 74_600_000),
    ("pay.credit", 97_600_000),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoiceStatus {
    Paid,
    Partial,
    Unpaid,
    Overdue,
}

impl InvoiceStatus {
    pub fn key(self) -> &'static str {
        match self {
            InvoiceStatus::Paid => "status.paid",
            InvoiceStatus::Partial => "status.partial",
            InvoiceStatus::Unpaid => "status.unpaid",
            InvoiceStatus::Overdue => "status.overdue",
        }
    }
}

pub struct Invoice {
    pub number: &'static str,
    pub ar: &'static str,
    pub fr: &'static str,
    pub back_days: u32,
    pub hour: u32,
    pub minute: u32,
    pub total: i64,
    pub status: InvoiceStatus,
}

pub const INVOICES: [Invoice; 7] = [
    Invoice {
        number: "F-01482",
        ar: "كريم بوزيد",
        fr: "Karim Bouzid",
        back_days: 0,
        hour: 11,
        minute: 42,
        total: 1_845_000,
        status: InvoiceStatus::Paid,
    },
    Invoice {
        number: "F-01481",
        ar: "مخبزة الأمل",
        fr: "Boulangerie El Amel",
        back_days: 0,
        hour: 10,
        minute: 15,
        total: 4_260_050,
        status: InvoiceStatus::Partial,
    },
    Invoice {
        number: "F-01480",
        ar: "زبون نقدي",
        fr: "Client comptoir",
        back_days: 0,
        hour: 9,
        minute: 3,
        total: 356_000,
        status: InvoiceStatus::Paid,
    },
    Invoice {
        number: "F-01479",
        ar: "سمير حداد",
        fr: "Samir Haddad",
        back_days: 1,
        hour: 18,
        minute: 27,
        total: 1_212_500,
        status: InvoiceStatus::Unpaid,
    },
    Invoice {
        number: "F-01478",
        ar: "محل الوفاء",
        fr: "Magasin El Wafa",
        back_days: 1,
        hour: 16,
        minute: 50,
        total: 8_930_000,
        status: InvoiceStatus::Paid,
    },
    Invoice {
        number: "F-01471",
        ar: "مطعم الصحراء",
        fr: "Restaurant Sahara",
        back_days: 9,
        hour: 13,
        minute: 5,
        total: 6_400_000,
        status: InvoiceStatus::Overdue,
    },
    Invoice {
        number: "F-01466",
        ar: "نادية مرابط",
        fr: "Nadia Merabet",
        back_days: 12,
        hour: 12,
        minute: 31,
        total: 980_000,
        status: InvoiceStatus::Overdue,
    },
];

/// Items running low: (ar, fr, left, minimum).
pub const LOW_STOCK: [(&str, &str, i64, i64); 4] = [
    ("زيت مائدة Elio 5L", "Huile Elio 5L", 0, 12),
    ("حليب Soummam 1L", "Lait Soummam 1L", 6, 24),
    ("سكر أبيض 1kg", "Sucre blanc 1kg", 9, 20),
    ("ماء معدني Ifri 1.5L", "Eau Ifri 1,5L", 14, 30),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_steps_back_across_months() {
        assert_eq!(day_back(0), (11, 10));
        assert_eq!(day_back(10), (10, 31));
        assert_eq!(day_back(40), (10, 1));
        assert_eq!(month_back(0), 11);
        assert_eq!(month_back(11), 12);
        assert_eq!(month_back(10), 1);
    }

    #[test]
    fn samples_are_stable_and_positive() {
        for p in [Period::Week, Period::Month, Period::Year] {
            let (a, b) = sales(p);
            assert_eq!(a.len(), p.points());
            assert_eq!(b.len(), p.points());
            assert!(a.iter().chain(&b).all(|&v| v > 0));
            assert_eq!(sales(p), (a, b));
        }
    }
}
