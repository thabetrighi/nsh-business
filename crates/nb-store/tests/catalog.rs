//! Catalog use cases (M5) on a real database: creation with units, barcodes
//! and opening stock, uniqueness, search, filters, sort, update, archive.

mod common;

use common::*;
use nb_store::Error;
use nb_store::repo::catalog::{ProductQuery, ProductSort, StockState};
use nb_store::usecases::products::{
    Opening, ProductInput, archive_products, create_product, list_products, next_sku,
    update_product,
};

type R = Result<(), Error>;

fn codes(e: &Error) -> Vec<&'static str> {
    e.rule_codes().into_iter().map(|(_, c)| c).collect()
}

fn milk(setup: &nb_store::usecases::setup::SetupResult) -> ProductInput {
    let mut p = product("حليب كامل الدسم Soummam 1L", "", 12_000);
    p.base_unit = "pc".into();
    p.extra_units = vec![("pack".into(), "6".into()), ("ctn".into(), "72".into())];
    p.barcodes = vec!["6130000123455".into()];
    p.min_stock = Some("24".into());
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "10".into(),
        unit_cost_minor: 9_500,
        tracked: Default::default(),
    });
    p
}

#[test]
fn creates_a_product_with_units_barcodes_and_opening_stock() -> R {
    let (mut s, setup) = store();
    let p = create_product(&mut s, meta(&setup, "product.create"), milk(&setup))?;
    assert_eq!(p.sku, "P-00001");
    assert_eq!(p.units.len(), 3);
    assert_eq!(p.units[2].factor, "72");
    assert_eq!(p.barcodes, vec!["6130000123455".to_owned()]);
    assert_eq!(p.on_hand_milli, 10_000);
    assert_eq!(p.stock_state(), StockState::Low);
    assert_eq!(next_sku(&s)?, "P-00002");
    // One opening movement, numbered.
    assert_eq!(
        count(
            &s,
            "SELECT COUNT(*) FROM stock_movements WHERE kind = 'opening' AND doc_number = 'AJ-01-26-A00001'"
        ),
        1
    );
    assert_eq!(count(&s, "SELECT COUNT(*) FROM price_history"), 1);
    Ok(())
}

#[test]
fn codes_and_barcodes_are_unique() -> R {
    let (mut s, setup) = store();
    create_product(&mut s, meta(&setup, "product.create"), milk(&setup))?;
    let mut dup = product("Autre", "p-00001", 100);
    dup.barcodes = vec!["6130000123455".into()];
    let e = create_product(&mut s, meta(&setup, "product.create"), dup)
        .err()
        .ok_or(Error::NotFound("error"))?;
    assert_eq!(codes(&e), vec!["product.sku_taken", "barcode.taken"]);
    Ok(())
}

#[test]
fn every_form_error_is_reported() -> R {
    let (mut s, setup) = store();
    let mut bad = product(" ", "a b", -1);
    bad.extra_units = vec![("ctn".into(), "0".into())];
    let e = create_product(&mut s, meta(&setup, "product.create"), bad)
        .err()
        .ok_or(Error::NotFound("error"))?;
    assert_eq!(codes(&e), vec!["units.factor_positive"]);
    let mut bad = product(" ", "a b", -1);
    bad.min_stock = Some("x".into());
    let e = create_product(&mut s, meta(&setup, "product.create"), bad)
        .err()
        .ok_or(Error::NotFound("error"))?;
    assert_eq!(codes(&e), vec!["invalid_quantity"]);
    let bad = product(" ", "a b", -1);
    let e = create_product(&mut s, meta(&setup, "product.create"), bad)
        .err()
        .ok_or(Error::NotFound("error"))?;
    assert_eq!(
        codes(&e),
        vec!["required", "product.sku_chars", "product.price_negative"]
    );
    Ok(())
}

fn seed(s: &mut nb_store::Store, setup: &nb_store::usecases::setup::SetupResult) -> R {
    let items = [
        ("Lait entier Soummam", 12_000, Some("12"), "5"),
        ("Yaourt fraise Soummam", 4_500, None, "40"),
        ("Huile de table Elio 5L", 95_000, Some("4"), "0"),
        ("سكر أبيض 1kg", 11_000, Some("10"), "30"),
        ("Eau minérale Ifri 1,5L", 4_000, None, "120"),
    ];
    for (name, price, min, qty) in items {
        let mut p = product(name, "", price);
        p.min_stock = min.map(Into::into);
        if qty != "0" {
            p.opening = Some(Opening {
                warehouse_id: setup.warehouse.to_string(),
                qty: qty.into(),
                unit_cost_minor: price * 8 / 10,
                tracked: Default::default(),
            });
        }
        create_product(s, meta(setup, "product.create"), p)?;
    }
    Ok(())
}

#[test]
fn search_filters_sorts_and_counts() -> R {
    let (mut s, setup) = store();
    seed(&mut s, &setup)?;
    let all = list_products(&s, &ProductQuery::default())?;
    assert_eq!(all.total, 5);
    assert_eq!(all.counts, [5, 3, 1, 1]);
    // Prefix search, case and accent insensitive, any word order.
    let q = |text: &str| ProductQuery {
        text: text.into(),
        ..ProductQuery::default()
    };
    let names = |p: nb_store::repo::catalog::ProductPage| {
        p.rows.into_iter().map(|r| r.name).collect::<Vec<_>>()
    };
    assert_eq!(
        names(list_products(&s, &q("soum lait"))?),
        vec!["Lait entier Soummam"]
    );
    assert_eq!(
        names(list_products(&s, &q("MINERALE"))?),
        vec!["Eau minérale Ifri 1,5L"]
    );
    assert_eq!(names(list_products(&s, &q("سكر"))?), vec!["سكر أبيض 1kg"]);
    assert_eq!(list_products(&s, &q("P-00003"))?.total, 1);
    assert_eq!(
        list_products(&s, &q("\"*) OR ("))?.total,
        0,
        "FTS syntax in input is harmless"
    );
    // Stock state filter and its counts under the text filter.
    let low = list_products(
        &s,
        &ProductQuery {
            state: Some(StockState::Low),
            ..ProductQuery::default()
        },
    )?;
    assert_eq!(names(low), vec!["Lait entier Soummam"]);
    let soum = list_products(&s, &q("soummam"))?;
    assert_eq!(soum.counts, [2, 1, 1, 0]);
    // Sort.
    let by_price = list_products(
        &s,
        &ProductQuery {
            sort: ProductSort::Price,
            desc: true,
            ..ProductQuery::default()
        },
    )?;
    assert_eq!(by_price.rows[0].name, "Huile de table Elio 5L");
    let by_stock = list_products(
        &s,
        &ProductQuery {
            sort: ProductSort::Stock,
            ..ProductQuery::default()
        },
    )?;
    assert_eq!(by_stock.rows[0].on_hand_milli, 0);
    // Paging.
    let page = list_products(
        &s,
        &ProductQuery {
            sort: ProductSort::Name,
            limit: 2,
            offset: 2,
            ..ProductQuery::default()
        },
    )?;
    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.total, 5);
    Ok(())
}

#[test]
fn updates_check_the_version_and_keep_price_history() -> R {
    let (mut s, setup) = store();
    let p = create_product(
        &mut s,
        meta(&setup, "product.create"),
        product("Lait", "", 12_000),
    )?;
    let id = p.product_id()?;
    let mut input = product("Lait entier", &p.sku, 12_500);
    input.barcodes = vec!["6130000123455".into()];
    let p2 = update_product(
        &mut s,
        meta(&setup, "product.update"),
        id,
        p.version,
        input.clone(),
    )?;
    assert_eq!(
        (p2.name.as_str(), p2.price_minor, p2.version),
        ("Lait entier", 12_500, 2)
    );
    assert_eq!(count(&s, "SELECT COUNT(*) FROM price_history"), 2);
    // Someone else's change in between: refused, nothing lost.
    let e = update_product(&mut s, meta(&setup, "product.update"), id, p.version, input)
        .err()
        .ok_or(Error::NotFound("error"))?;
    assert_eq!(codes(&e), vec!["stale_version"]);
    let found = list_products(
        &s,
        &ProductQuery {
            text: "6130000123455".into(),
            ..ProductQuery::default()
        },
    )?;
    assert_eq!(found.total, 1, "barcodes are searchable");
    Ok(())
}

#[test]
fn archived_products_leave_lists_and_free_their_codes() -> R {
    let (mut s, setup) = store();
    let mut p = product("Lait", "L-1", 12_000);
    p.barcodes = vec!["6130000123455".into()];
    let row = create_product(&mut s, meta(&setup, "product.create"), p.clone())?;
    assert_eq!(
        archive_products(
            &mut s,
            meta(&setup, "product.archive"),
            &[row.product_id()?]
        )?,
        1
    );
    assert_eq!(list_products(&s, &ProductQuery::default())?.total, 0);
    assert_eq!(
        list_products(
            &s,
            &ProductQuery {
                text: "lait".into(),
                ..ProductQuery::default()
            }
        )?
        .total,
        0
    );
    create_product(&mut s, meta(&setup, "product.create"), p)?;
    Ok(())
}

#[test]
fn ten_thousand_products_search_quickly() -> R {
    let (mut s, setup) = store();
    let names = [
        "Lait", "Yaourt", "Fromage", "Pain", "Biscuit", "Huile", "Sucre", "Eau", "Café", "Thé",
    ];
    for i in 0..10_000 {
        let p = product(
            &format!(
                "{} {} #{i}",
                names[i % 10],
                ["Soummam", "Cevital", "Ifri"][i % 3]
            ),
            &format!("P-{:05}", i + 1),
            1_000,
        );
        create_product(&mut s, meta(&setup, "product.create"), p)?;
    }
    let t = std::time::Instant::now();
    let page = list_products(
        &s,
        &ProductQuery {
            text: "biscuit cev".into(),
            limit: 50,
            ..ProductQuery::default()
        },
    )?;
    let took = t.elapsed();
    assert!(page.total > 0);
    // PRD: search < 150 ms. Debug builds are slower; keep a margin.
    assert!(took.as_millis() < 1_500, "search took {took:?}");
    Ok(())
}

/// Several products move to a category at once (the list's bulk action).
#[test]
fn products_move_to_a_category_together() -> R {
    use nb_store::usecases::catalog::{CategoryInput, save_category};
    use nb_store::usecases::products::set_products_category;
    let (mut s, setup) = store();
    let a = create_product(&mut s, meta(&setup, "p"), product("Lait", "", 100))?;
    let b = create_product(&mut s, meta(&setup, "p"), product("Yaourt", "", 50))?;
    let dairy = save_category(
        &mut s,
        meta(&setup, "c"),
        CategoryInput {
            id: None,
            name: "Laitiers".into(),
            parent_id: None,
        },
    )?;
    let ids = [a.product_id()?, b.product_id()?];
    assert_eq!(
        set_products_category(&mut s, meta(&setup, "m"), &ids, Some(&dairy.id))?,
        2
    );
    let page = list_products(
        &s,
        &ProductQuery {
            category_id: Some(dairy.id.clone()),
            ..Default::default()
        },
    )?;
    assert_eq!(page.rows.len(), 2);
    // Out of any category again; an unknown category is refused.
    set_products_category(&mut s, meta(&setup, "m"), &ids, None)?;
    assert!(set_products_category(&mut s, meta(&setup, "m"), &ids, Some("nope")).is_err());
    assert_eq!(
        count(
            &s,
            "SELECT COUNT(*) FROM audit_log WHERE action = 'set_category'"
        ),
        4
    );
    Ok(())
}

#[test]
fn favourites_are_marked_and_listed_on_their_own() -> R {
    use nb_store::usecases::products::set_favorite;
    let (mut s, setup) = store();
    let a = create_product(&mut s, meta(&setup, "p"), product("Lait", "", 100))?;
    create_product(&mut s, meta(&setup, "p"), product("Yaourt", "", 50))?;
    let fav = |s: &nb_store::Store| -> Result<Vec<String>, nb_store::Error> {
        let q = ProductQuery {
            favorites: true,
            ..Default::default()
        };
        Ok(list_products(s, &q)?
            .rows
            .into_iter()
            .map(|r| r.name)
            .collect())
    };
    assert!(fav(&s)?.is_empty());
    set_favorite(&mut s, meta(&setup, "f"), &[a.product_id()?], true)?;
    assert_eq!(fav(&s)?, ["Lait"]);
    let all = list_products(&s, &ProductQuery::default())?;
    assert!(all.rows.iter().any(|r| r.name == "Lait" && r.favorite));
    set_favorite(&mut s, meta(&setup, "f"), &[a.product_id()?], false)?;
    assert!(fav(&s)?.is_empty());
    assert_eq!(
        count(
            &s,
            "SELECT COUNT(*) FROM audit_log WHERE action = 'set_favorite'"
        ),
        2
    );
    Ok(())
}
