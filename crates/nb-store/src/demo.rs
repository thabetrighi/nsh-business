//! Demonstration and test data (M2.12): a deterministic catalog with
//! categories, prices and opening stock, written in one operation. Used on
//! a first start without data, for screenshots and UI tests, and for load
//! tests (`n` up to 100k).

use nb_core::{Decimal, Money, Quantity, Rate};
use nb_domain::ledger::{Entry, Line};
use nb_domain::numbering::DocKind;
use nb_domain::product::{ProductDraft, ProductKind};
use nb_domain::stock::{Balance, MovementKind, NegativeStock};
use nb_domain::units::UnitSet;
use nb_domain::{CategoryId, ProductId};
use serde::{Deserialize, Serialize};

use crate::repo::catalog::{self, Category};
use crate::repo::{ledger, org, parse_id, stock};
use crate::uow::OpMeta;
use crate::usecases::setup::SetupResult;
use crate::{Error, Result, Store};

/// Language of the demo names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemoLang {
    Ar,
    Fr,
}

const CATEGORIES: [(&str, &str); 6] = [
    ("ألبان", "Produits laitiers"),
    ("مخبوزات", "Boulangerie"),
    ("زيوت وسكر", "Huiles et sucre"),
    ("مشروبات", "Boissons"),
    ("منظفات", "Entretien"),
    ("خضر وفواكه", "Fruits et légumes"),
];

/// (ar, fr, category, unit code, precision).
const NAMES: [(&str, &str, usize, &str, u32); 12] = [
    ("حليب كامل الدسم", "Lait entier", 0, "l", 0),
    ("ياغورت بالفراولة", "Yaourt fraise", 0, "pc", 0),
    ("جبن مثلثات", "Fromage portions", 0, "pc", 0),
    ("خبز تقليدي", "Pain traditionnel", 1, "pc", 0),
    ("بسكويت بالشوكولاتة", "Biscuits chocolat", 1, "pc", 0),
    ("زيت مائدة", "Huile de table", 2, "l", 0),
    ("سكر أبيض", "Sucre blanc", 2, "kg", 3),
    ("ماء معدني", "Eau minérale", 3, "l", 0),
    ("مشروب غازي", "Boisson gazeuse", 3, "l", 0),
    ("مسحوق غسيل", "Lessive en poudre", 4, "kg", 3),
    ("طماطم طازجة", "Tomates fraîches", 5, "kg", 3),
    ("بطاطا", "Pommes de terre", 5, "kg", 3),
];
const BRANDS: [&str; 6] = ["Soummam", "Cevital", "Ifri", "Elio", "Hamoud", "Bimo"];

/// (Arabic name, French name, kind, city, Arabic city, credit limit in centimes).
const CUSTOMERS: [(&str, &str, &str, &str, &str, i64); 8] = [
    (
        "مخبزة الأمل",
        "Boulangerie El Amel",
        "company",
        "El Oued",
        "الوادي",
        5_000_000,
    ),
    (
        "كريم بوزيد",
        "Karim Bouzid",
        "person",
        "Guemar",
        "قمار",
        2_000_000,
    ),
    (
        "مطعم الصحراء",
        "Restaurant Sahara",
        "company",
        "El Oued",
        "الوادي",
        10_000_000,
    ),
    (
        "نادية مرابط",
        "Nadia Merabet",
        "person",
        "Debila",
        "الدبيلة",
        1_000_000,
    ),
    (
        "سمير حداد",
        "Samir Haddad",
        "person",
        "Kouinine",
        "كوينين",
        0,
    ),
    (
        "محل الوفاء",
        "Magasin El Wafa",
        "company",
        "Robbah",
        "الرباح",
        3_000_000,
    ),
    (
        "فاطمة الزهراء بن علي",
        "Fatima Zohra Benali",
        "person",
        "El Oued",
        "الوادي",
        0,
    ),
    (
        "مقهى النخيل",
        "Café Ennakhil",
        "company",
        "Bayadha",
        "البياضة",
        1_500_000,
    ),
];

/// Writes `n` demo products (codes `P-01000`…) with their opening stock.
pub fn seed(store: &mut Store, setup: &SetupResult, lang: DemoLang, n: usize) -> Result<u32> {
    let currency = store
        .read(|r| org::the_org(r.conn()))?
        .ok_or(Error::NotFound("organization"))?
        .currency()?;
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    let meta = OpMeta::new("demo.seed", Some(setup.org), Some(setup.owner));
    let warehouse = setup.warehouse;
    store.write(meta, |w| {
        let org_id = w.org()?;
        let wh = org::warehouse(w.tx(), warehouse)?;
        let branch = org::branch(w.tx(), parse_id(&wh.branch_id)?)?;
        let number = w.next_number(DocKind::Adjustment, &branch.id, &branch.code)?;
        let mut cats = Vec::new();
        for (ar, fr) in CATEGORIES {
            let c = Category {
                id: CategoryId::new().to_string(),
                name: if lang == DemoLang::Ar { ar } else { fr }.to_owned(),
            };
            catalog::insert_category(w, org_id, &c)?;
            cats.push(c.id);
        }
        let mut opening_value = Money::zero(currency);
        for i in 0..n {
            let (ar, fr, cat, unit, precision) = NAMES[i % NAMES.len()];
            let brand = BRANDS[(i / NAMES.len()) % BRANDS.len()];
            let variant = i / (NAMES.len() * BRANDS.len());
            let base = if lang == DemoLang::Ar { ar } else { fr };
            let name = if variant > 0 {
                format!("{base} {brand} #{variant}")
            } else {
                format!("{base} {brand}")
            };
            let price = 2_500 + ((i * 7_919) % 180) as i64 * 250;
            let stock_units = ((i * 37) % 140) as i64 - 4;
            let draft = ProductDraft {
                name,
                sku: format!("P-{:05}", 1000 + i),
                kind: ProductKind::Goods,
                units: UnitSet::base(unit, precision)?,
                barcodes: vec![],
                pack_barcodes: vec![],
                tax_rate: Rate::from_bps(if cat == 5 { 0 } else { 1_900 }),
                price: Money::from_minor(price, currency),
                min_price: None,
                cost: Some(Money::from_minor(price * 7 / 10, currency)),
                min_stock: Some(Quantity::whole(10)),
            };
            let valid = draft.validate()?;
            let id = ProductId::new();
            let extra = catalog::ProductExtra {
                category_id: Some(cats[cat].clone()),
                ..Default::default()
            };
            catalog::insert(w, org_id, id, &valid, &extra)?;
            if stock_units > 0 {
                let qty = Quantity::new(Decimal::from(stock_units));
                let cost = Money::from_minor(price * 7 / 10, currency);
                let after = Balance::empty(Money::zero(currency)).apply(
                    MovementKind::Opening,
                    qty,
                    Some(cost),
                    NegativeStock::Allow,
                    true,
                )?;
                stock::append(
                    w,
                    id,
                    warehouse,
                    MovementKind::Opening,
                    qty,
                    Some(cost),
                    Some(&number),
                    None,
                    after,
                )?;
                opening_value = opening_value.checked_add(qty.times_price(cost)?)?;
            }
        }
        // One opening entry for the whole stock (30 / 101, as for one product).
        if !opening_value.is_zero() {
            let entry = Entry::new(vec![
                Line::debit("30", opening_value),
                Line::credit("101", opening_value),
            ])?;
            let jnumber = w.next_number(DocKind::Journal, &branch.id, &branch.code)?;
            let date = crate::time::iso_date(w.now(), offset);
            let header = ledger::Header {
                number: &jnumber,
                date: &date,
                memo: &number,
                source: "stock.opening",
                reverses: None,
                party: None,
            };
            ledger::post(w, org_id, header, &entry)?;
        }
        // Customers of a neighbourhood shop: a few with credit.
        for (i, (ar, fr, kind, city, city_ar, limit)) in CUSTOMERS.iter().enumerate() {
            w.tx().execute(
                "INSERT INTO customers (id, org_id, code, kind, name, city, phone, credit_limit_minor, payment_days,
                        created_at, updated_at, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11)",
                rusqlite::params![
                    nb_core::uuid::Uuid::now_v7().to_string(),
                    org_id.to_string(),
                    format!("C-{:05}", i + 1),
                    kind,
                    if lang == DemoLang::Ar { ar } else { fr },
                    if lang == DemoLang::Ar { city_ar } else { city },
                    format!("06{:02} {:02} {:02} {:02}", 61 + i, 10 + i * 3, 20 + i, 40 + i * 2),
                    limit,
                    if *limit > 0 { 30 } else { 0 },
                    w.now(),
                    w.device().id.to_string()
                ],
            )?;
        }
        w.tx().execute(
            "INSERT INTO customer_counters (org_id, last) VALUES (?1, ?2)",
            rusqlite::params![org_id.to_string(), i64::try_from(CUSTOMERS.len()).unwrap_or(0)],
        )?;
        w.event("demo.seeded", "organization", &org_id.to_string(), &n)?;
        w.audit("seed", "organization", &org_id.to_string(), &n)?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoreOptions;
    use crate::repo::catalog::ProductQuery;
    use crate::usecases::products::list_products;
    use crate::usecases::setup::{SetupInput, setup_organization};

    #[test]
    fn the_demo_catalog_matches_the_screens() -> Result<()> {
        let mut s = Store::open_in_memory(StoreOptions::default())?;
        let setup = setup_organization(
            &mut s,
            SetupInput {
                org_name: "Demo".into(),
                activity: "grocery".into(),
                country: "DZ".into(),
                currency: "DZD".into(),
                currency_exp: 2,
                branch_name: "Demo".into(),
                owner_name: "Owner".into(),
                language: "fr".into(),
                fiscal_year_start: 1,
            },
        )?;
        assert_eq!(seed(&mut s, &setup, DemoLang::Fr, 2_000)?, 2_000);
        let page = list_products(&s, &ProductQuery::default())?;
        assert_eq!(page.counts, [2_000, 1_799, 129, 72]);
        assert!(crate::usecases::stock::rebuild_balances(&s)?.is_empty());
        assert!(
            s.read(|r| crate::repo::ledger::unbalanced_entries(r.conn()))?
                .is_empty()
        );
        Ok(())
    }
}
