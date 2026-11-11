//! The catalog around products (M5): categories, brands, tags, tax
//! classes, price lists, scanning (barcodes and scale labels), variants,
//! pictures, unit changes and the CSV import.

mod common;

use common::*;
use nb_core::Quantity;
use nb_domain::ProductId;
use nb_domain::barcode::gtin_check_digit;
use nb_store::Error;
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::catalog::*;
use nb_store::usecases::products::{Opening, ProductInput, create_product, update_product};

type R = Result<(), Error>;

fn codes(e: &Error) -> Vec<&'static str> {
    e.rule_codes().into_iter().map(|(_, c)| c).collect()
}

fn err<T: std::fmt::Debug>(r: Result<T, Error>) -> Vec<&'static str> {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => codes(&e),
    }
}

fn id(s: &str) -> ProductId {
    s.parse().unwrap_or_default()
}

/// A valid EAN-13 from its first 12 digits.
fn ean13(body: &str) -> String {
    format!("{body}{}", gtin_check_digit(body).unwrap_or(0))
}

fn cat(name: &str, parent: Option<&str>) -> CategoryInput {
    CategoryInput {
        id: None,
        name: name.into(),
        parent_id: parent.map(Into::into),
    }
}

#[test]
fn categories_form_a_tree_without_loops() -> R {
    let (mut s, setup) = store();
    let food = save_category(
        &mut s,
        meta(&setup, "category.save"),
        cat("Alimentation", None),
    )?;
    let dairy = save_category(
        &mut s,
        meta(&setup, "category.save"),
        cat("Laitiers", Some(&food.id)),
    )?;
    let milk = save_category(
        &mut s,
        meta(&setup, "category.save"),
        cat("Lait", Some(&dairy.id)),
    )?;
    assert_eq!(milk.path, ["Alimentation", "Laitiers", "Lait"]);
    // Moving the top under its grandchild would loop.
    let looped = save_category(
        &mut s,
        meta(&setup, "category.save"),
        CategoryInput {
            id: Some(food.id.clone()),
            name: "Alimentation".into(),
            parent_id: Some(milk.id.clone()),
        },
    );
    assert_eq!(err(looped), ["category.cycle"]);
    let twin = save_category(
        &mut s,
        meta(&setup, "category.save"),
        cat(" laitiers ", Some(&food.id)),
    );
    assert_eq!(err(twin), ["category.name_taken"]);
    // The same name elsewhere in the tree is fine.
    save_category(&mut s, meta(&setup, "category.save"), cat("Laitiers", None))?;
    let mut parent = milk.id.clone();
    for i in 0..2 {
        parent = save_category(
            &mut s,
            meta(&setup, "category.save"),
            cat(&format!("N{i}"), Some(&parent)),
        )?
        .id;
    }
    assert_eq!(
        err(save_category(
            &mut s,
            meta(&setup, "category.save"),
            cat("Trop", Some(&parent))
        )),
        ["category.too_deep"]
    );
    // Listed in tree order with their paths.
    let list = list_categories(&s)?;
    let paths: Vec<String> = list.iter().map(|c| c.path.join(" › ")).collect();
    assert_eq!(
        paths[..3],
        [
            "Alimentation",
            "Alimentation › Laitiers",
            "Alimentation › Laitiers › Lait"
        ]
    );
    // A category with products or children stays.
    let mut p = product("Lait 1L", "", 12_000);
    p.category_id = Some(milk.id.clone());
    create_product(&mut s, meta(&setup, "product.create"), p)?;
    assert_eq!(
        err(archive_category(
            &mut s,
            meta(&setup, "category.archive"),
            &milk.id
        )),
        ["category.not_empty"]
    );
    assert_eq!(
        err(archive_category(
            &mut s,
            meta(&setup, "category.archive"),
            &food.id
        )),
        ["category.not_empty"]
    );
    Ok(())
}

#[test]
fn brands_tags_and_tax_classes() -> R {
    let (mut s, setup) = store();
    let brand = save_brand(&mut s, meta(&setup, "brand.save"), None, "Soummam")?;
    assert_eq!(
        err(save_brand(
            &mut s,
            meta(&setup, "brand.save"),
            None,
            "soummam"
        )),
        ["catalog.name_taken"]
    );
    let classes = list_tax_classes(&s)?;
    assert_eq!(
        classes.iter().map(|c| c.rate_bps).collect::<Vec<_>>(),
        [1_900, 900, 0]
    );
    let nine = classes[1].clone();
    let mut p = product("Yaourt", "", 4_000);
    p.brand_id = Some(brand.id.clone());
    p.tax_class_id = Some(nine.id.clone());
    p.tags = vec!["frais".into(), " Frais ".into(), "promo".into()];
    p.alt_name = Some("ياغورت".into());
    let row = create_product(&mut s, meta(&setup, "product.create"), p)?;
    assert_eq!(row.brand.as_deref(), Some("Soummam"));
    assert_eq!(row.tax_bps, 900, "the class's rate");
    assert_eq!(row.tags, ["frais", "promo"]);
    assert_eq!(list_tags(&s)?, ["frais", "promo"]);
    // The other name is searchable.
    let page = nb_store::usecases::products::list_products(
        &s,
        &nb_store::repo::catalog::ProductQuery {
            text: "ياغورت".into(),
            ..Default::default()
        },
    )?;
    assert_eq!(page.total, 1);
    // A class's new rate applies to its products.
    save_tax_class(
        &mut s,
        meta(&setup, "tax.save"),
        TaxClass {
            rate_bps: 1_000,
            ..nine
        },
    )?;
    assert_eq!(
        nb_store::repo::catalog::get(s.connection(), id(&row.id))?.tax_bps,
        1_000
    );
    let mut bad = product("X", "", 1);
    bad.brand_id = Some("nope".into());
    assert_eq!(
        err(create_product(&mut s, meta(&setup, "product.create"), bad)),
        ["not_found"]
    );
    Ok(())
}

#[test]
fn list_prices_breaks_and_history() -> R {
    let (mut s, setup) = store();
    let mut p = product("Huile 5L", "", 95_000);
    p.min_price_minor = Some(80_000);
    let row = create_product(&mut s, meta(&setup, "product.create"), p)?;
    let pid = id(&row.id);
    let semi = create_price_list(&mut s, meta(&setup, "price_list.create"), "Demi-gros")?;
    assert_eq!(semi.code, "c1");
    let price = |list: &str, minor: i64, min: &str| PriceInput {
        list: list.into(),
        price_minor: minor,
        min_qty: min.into(),
        branch_id: None,
        customer_id: None,
    };
    set_product_prices(
        &mut s,
        meta(&setup, "prices"),
        pid,
        vec![
            price("retail", 92_000, "12"),
            price("wholesale", 88_000, ""),
            price("wholesale", 85_000, "48"),
            price("c1", 90_000, ""),
        ],
    )?;
    let at = |list: &str, qty: i64| {
        resolve_price(&s, pid, list, Quantity::whole(qty), None, None).map(|m| m.minor())
    };
    assert_eq!(at("retail", 1)?, 95_000);
    assert_eq!(at("retail", 12)?, 92_000);
    assert_eq!(at("wholesale", 10)?, 88_000);
    assert_eq!(at("wholesale", 48)?, 85_000);
    assert_eq!(at("c1", 1)?, 90_000);
    // Under the floor, a duplicate rule, an unknown list: refused.
    assert_eq!(
        err(set_product_prices(
            &mut s,
            meta(&setup, "prices"),
            pid,
            vec![price("wholesale", 70_000, "")]
        )),
        ["price.below_floor"]
    );
    assert_eq!(
        err(set_product_prices(
            &mut s,
            meta(&setup, "prices"),
            pid,
            vec![price("retail", 90_000, "6"), price("retail", 91_000, "6,0")]
        )),
        ["price.duplicate"]
    );
    assert_eq!(
        err(set_product_prices(
            &mut s,
            meta(&setup, "prices"),
            pid,
            vec![price("c9", 90_000, "")]
        )),
        ["not_found"]
    );
    // Every change is kept: the creation and the four list prices.
    assert_eq!(price_history(&s, pid)?.len(), 5);
    // Only those holding products.price may change prices.
    let cashier = create_user(
        &mut s,
        meta(&setup, "user.create"),
        UserInput {
            name: "K".into(),
            phone: None,
            login: None,
            password: None,
            pin: None,
            grants: vec![GrantRow::org("cashier")],
        },
    )?;
    let as_cashier = nb_store::OpMeta::new(
        "prices",
        Some(setup.org),
        Some(cashier.id.parse().unwrap_or_default()),
    );
    assert!(matches!(
        set_product_prices(&mut s, as_cashier, pid, vec![]),
        Err(Error::Forbidden {
            permission: "products.price",
            ..
        })
    ));
    Ok(())
}

#[test]
fn scanning_barcodes_and_scale_labels() -> R {
    let (mut s, setup) = store();
    let piece = ean13("613000012345");
    let carton = ean13("613000099999");
    let mut p = product("Eau 1,5L", "", 4_000);
    p.extra_units = vec![("ctn".into(), "6".into())];
    p.barcodes = vec![piece.clone()];
    p.pack_barcodes = vec![(carton.clone(), "ctn".into())];
    create_product(&mut s, meta(&setup, "product.create"), p)?;
    let mut tomatoes = product("Tomates", "", 12_000);
    tomatoes.base_unit = "kg".into();
    tomatoes.base_precision = 3;
    tomatoes.plu = Some("00042".into());
    let t = create_product(&mut s, meta(&setup, "product.create"), tomatoes)?;

    assert!(matches!(scan(&s, &piece)?, Scan::Item { unit, .. } if unit == "pc"));
    assert!(matches!(scan(&s, &carton)?, Scan::Item { unit, .. } if unit == "ctn"));
    // The default layout: 21 + 5-digit article + 5-digit grams.
    let label = ean13("210004201235");
    match scan(&s, &label)? {
        Scan::Weighed { product, qty } => {
            assert_eq!(product.id, t.id);
            assert_eq!(qty, "1.235");
        }
        other => panic!("{other:?}"),
    }
    // A price layout added by the store: 22 + 5-digit article + price in centimes.
    set_scale_formats(
        &mut s,
        meta(&setup, "scale"),
        vec![
            ScaleFormatRow {
                prefix: "21".into(),
                item_digits: 5,
                value_digits: 5,
                value_kind: "weight".into(),
                decimals: 3,
            },
            ScaleFormatRow {
                prefix: "22".into(),
                item_digits: 5,
                value_digits: 5,
                value_kind: "price".into(),
                decimals: 2,
            },
        ],
    )?;
    match scan(&s, &ean13("220004218000"))? {
        Scan::Priced {
            amount_minor, qty, ..
        } => {
            assert_eq!(amount_minor, 18_000);
            assert_eq!(qty, "1.5", "180.00 DA at 120.00 DA/kg");
        }
        other => panic!("{other:?}"),
    }
    let bad = set_scale_formats(
        &mut s,
        meta(&setup, "scale"),
        vec![ScaleFormatRow {
            prefix: "2".into(),
            item_digits: 8,
            value_digits: 5,
            value_kind: "weight".into(),
            decimals: 3,
        }],
    );
    assert_eq!(err(bad), ["scale.layout"]);
    // Unknown codes can be linked to a product, then scan as it.
    let unknown = ean13("613000077777");
    assert_eq!(
        scan(&s, &unknown)?,
        Scan::Unknown {
            code: unknown.clone()
        }
    );
    link_barcode(
        &mut s,
        meta(&setup, "barcode.link"),
        id(&t.id),
        &unknown,
        "kg",
    )?;
    assert!(matches!(scan(&s, &unknown)?, Scan::Item { .. }));
    assert_eq!(
        err(link_barcode(
            &mut s,
            meta(&setup, "barcode.link"),
            id(&t.id),
            &piece,
            "kg"
        )),
        ["barcode.taken"]
    );
    // A weighed-article code belongs to one product.
    let mut twin = product("Courgettes", "", 9_000);
    twin.plu = Some("42".into());
    create_product(&mut s, meta(&setup, "product.create"), twin.clone())?;
    twin.sku = "COU-2".into();
    twin.plu = Some("00042".into());
    assert_eq!(
        err(create_product(&mut s, meta(&setup, "product.create"), twin)),
        ["product.plu_taken"]
    );
    Ok(())
}

#[test]
fn variants_are_generated_from_the_matrix() -> R {
    let (mut s, setup) = store();
    let mut p = product("T-shirt", "TSH-01", 150_000);
    p.tags = vec!["été".into()];
    let parent = create_product(&mut s, meta(&setup, "product.create"), p)?;
    let pid = id(&parent.id);
    let attrs = |sizes: &[&str]| {
        vec![
            AttributeInput {
                name: "Couleur".into(),
                values: vec!["Rouge".into(), "Bleu".into()],
            },
            AttributeInput {
                name: "Taille".into(),
                values: sizes.iter().map(|v| (*v).to_owned()).collect(),
            },
        ]
    };
    let made = create_variants(
        &mut s,
        meta(&setup, "variants"),
        pid,
        attrs(&["S", "M", "L"]),
    )?;
    assert_eq!(made.len(), 6);
    assert_eq!(made[0].sku, "TSH-01-ROUGE-S");
    assert_eq!(made[5].name, "T-shirt · Bleu · L");
    assert!(
        made.iter()
            .all(|v| v.parent_id.as_deref() == Some(parent.id.as_str())
                && v.price_minor == 150_000
                && v.tags == ["été"])
    );
    // Adding a size makes only the new combinations.
    let more = create_variants(
        &mut s,
        meta(&setup, "variants"),
        pid,
        attrs(&["S", "M", "L", "XL"]),
    )?;
    assert_eq!(
        more.iter().map(|v| v.sku.as_str()).collect::<Vec<_>>(),
        ["TSH-01-ROUGE-XL", "TSH-01-BLEU-XL"]
    );
    assert_eq!(variants_of(&s, pid)?.len(), 8);
    assert_eq!(
        err(create_variants(
            &mut s,
            meta(&setup, "variants"),
            id(&made[0].id),
            attrs(&["S"])
        )),
        ["variants.of_variant"]
    );
    Ok(())
}

#[test]
fn pictures_are_checked_and_limited() -> R {
    let (mut s, setup) = store();
    let p = create_product(
        &mut s,
        meta(&setup, "product.create"),
        product("Café", "", 30_000),
    )?;
    let pid = id(&p.id);
    let png = [
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..],
        &[0u8; 64],
    ]
    .concat();
    let img = add_image(&mut s, meta(&setup, "image"), pid, png.clone())?;
    assert_eq!(img.mime, "image/png");
    assert_eq!(image_bytes(&s, &img.id)?, png);
    assert_eq!(
        err(add_image(
            &mut s,
            meta(&setup, "image"),
            pid,
            b"GIF89a....".to_vec()
        )),
        ["image.format"]
    );
    assert_eq!(
        err(add_image(
            &mut s,
            meta(&setup, "image"),
            pid,
            [&png[..], &vec![0; IMAGE_MAX_BYTES]].concat()
        )),
        ["image.too_large"]
    );
    for _ in 1..IMAGES_MAX {
        add_image(&mut s, meta(&setup, "image"), pid, png.clone())?;
    }
    assert_eq!(
        err(add_image(&mut s, meta(&setup, "image"), pid, png.clone())),
        ["image.too_many"]
    );
    remove_image(&mut s, meta(&setup, "image"), &img.id)?;
    assert_eq!(list_images(&s, pid)?.len(), IMAGES_MAX - 1);
    Ok(())
}

#[test]
fn a_conversion_in_use_cannot_change() -> R {
    let (mut s, setup) = store();
    let mut p = product("Jus", "JUS", 15_000);
    p.extra_units = vec![("ctn".into(), "12".into())];
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "24".into(),
        unit_cost_minor: 10_000,
    });
    let row = create_product(&mut s, meta(&setup, "product.create"), p.clone())?;
    p.opening = None;
    p.extra_units = vec![("ctn".into(), "6".into())];
    let changed = update_product(
        &mut s,
        meta(&setup, "product.update"),
        id(&row.id),
        row.version,
        p.clone(),
    );
    assert_eq!(err(changed), ["units.in_use"]);
    // Adding a unit keeps every past movement's meaning.
    p.extra_units = vec![("ctn".into(), "12".into()), ("pal".into(), "480".into())];
    let row = update_product(
        &mut s,
        meta(&setup, "product.update"),
        id(&row.id),
        row.version,
        p,
    )?;
    assert_eq!(row.units.len(), 3);
    Ok(())
}

#[test]
fn csv_import_is_all_or_nothing() -> R {
    let (mut s, setup) = store();
    let good = format!(
        "Désignation;Code-barres;Prix;Prix d'achat;Unité;Catégorie;Stock;TVA\n\
         Huile Elio 5L;{};950,00;800;;Huiles;10;19\n\
         Sucre 1kg;;120;95;kg;Épicerie;12,5;9\n\
         Sel;;35;;;Épicerie;;0\n",
        ean13("613000011111")
    );
    let bad = "Nom;Prix;Stock\nA;12;1\n;10;\nC;abc;\nD;5;1,5\n";
    let report = import_products(&mut s, meta(&setup, "import"), bad, setup.warehouse)?;
    let lines: Vec<(usize, &str)> = report
        .errors
        .iter()
        .map(|e| (e.line, e.code.as_str()))
        .collect();
    assert_eq!(
        lines,
        [
            (3, "required"),
            (4, "price.invalid"),
            (5, "units.too_precise")
        ]
    );
    assert_eq!(
        count(&s, "SELECT COUNT(*) FROM products"),
        0,
        "nothing written"
    );
    let report = import_products(&mut s, meta(&setup, "import"), &good, setup.warehouse)?;
    assert_eq!(
        (
            report.created,
            report.categories_created,
            report.errors.len()
        ),
        (3, 2, 0)
    );
    let sugar = nb_store::usecases::products::list_products(
        &s,
        &nb_store::repo::catalog::ProductQuery {
            text: "sucre".into(),
            ..Default::default()
        },
    )?;
    let sugar = &sugar.rows[0];
    assert_eq!(
        (sugar.tax_bps, sugar.on_hand_milli, sugar.cost_minor),
        (900, 12_500, Some(9_500))
    );
    assert_eq!(sugar.category.as_deref(), Some("Épicerie"));
    // One opening entry for the whole file: 10 × 800 + 12.5 × 95.
    let value: i64 = s.connection().query_row(
        "SELECT SUM(debit_minor) FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id WHERE e.memo = 'import'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(value, 800_000 + 118_750);
    // Importing the same file again: its codes are taken now.
    let again = import_products(&mut s, meta(&setup, "import"), &good, setup.warehouse)?;
    assert_eq!(
        again
            .errors
            .iter()
            .map(|e| (e.line, e.code.as_str()))
            .collect::<Vec<_>>(),
        [(2, "barcode.taken")]
    );
    assert!(
        s.read(|r| nb_store::repo::ledger::unbalanced_entries(r.conn()))?
            .is_empty()
    );
    Ok(())
}

#[test]
fn older_databases_get_lists_and_classes() -> R {
    use nb_store::migrate::{latest, migrate_to};
    let mut conn = rusqlite::Connection::open_in_memory()?;
    migrate_to(&mut conn, 6, None)?;
    conn.execute_batch(
        "INSERT INTO organizations (id, name, country, currency, currency_exp, activity, created_at, updated_at, device_id)
           VALUES ('o1', 'Old', 'DZ', 'DZD', 2, 'grocery', 0, 0, 'd');",
    )?;
    migrate_to(&mut conn, latest(), None)?;
    let n = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0));
    assert_eq!(
        n("SELECT COUNT(*) FROM price_lists WHERE org_id = 'o1'")?,
        2
    );
    assert_eq!(
        n("SELECT COUNT(*) FROM tax_classes WHERE org_id = 'o1'")?,
        3
    );
    assert_eq!(
        n("SELECT COUNT(*) FROM scale_formats WHERE org_id = 'o1'")?,
        1
    );
    Ok(())
}

#[test]
fn in_store_barcodes_follow_each_other() -> R {
    let (mut s, setup) = store();
    let first = next_internal_barcodes(&s, 2)?;
    assert_eq!(first[0], ean13("200000000001"));
    assert_eq!(first[1], ean13("200000000002"));
    let mut p = product("Pain maison", "", 3_000);
    p.barcodes = vec![first[1].clone()];
    create_product(&mut s, meta(&setup, "product.create"), p)?;
    assert_eq!(next_internal_barcodes(&s, 1)?, [ean13("200000000003")]);
    Ok(())
}

#[test]
fn the_editor_saves_everything_at_once() -> R {
    let (mut s, setup) = store();
    let png = [
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..],
        &[0u8; 16],
    ]
    .concat();
    let mut input = product("Polo", "POLO", 200_000);
    input.min_price_minor = Some(150_000);
    let save = ProductSave {
        existing: None,
        input: input.clone(),
        prices: vec![PriceInput {
            list: "wholesale".into(),
            price_minor: 170_000,
            min_qty: String::new(),
            branch_id: None,
            customer_id: None,
        }],
        variants: vec![AttributeInput {
            name: "Taille".into(),
            values: vec!["M".into(), "L".into()],
        }],
        add_images: vec![png.clone()],
        remove_images: vec![],
    };
    let m = meta(&setup, "product.save");
    let row = save_product(&mut s, m, save.clone())?;
    // A second tap gives the same product, made once.
    assert_eq!(save_product(&mut s, m, save.clone())?.id, row.id);
    let d = product_detail(&s, id(&row.id))?;
    assert_eq!(
        (d.prices.len(), d.variants.len(), d.images.len()),
        (1, 2, 1)
    );
    // Editing: a price under the floor fails, and nothing of that save stays.
    let mut edit = ProductSave {
        existing: Some((row.id.clone(), d.product.version)),
        input: ProductInput {
            name: "Polo coton".into(),
            ..input.clone()
        },
        prices: vec![PriceInput {
            list: "wholesale".into(),
            price_minor: 100_000,
            min_qty: String::new(),
            branch_id: None,
            customer_id: None,
        }],
        remove_images: vec![d.images[0].id.clone()],
        ..Default::default()
    };
    assert_eq!(
        err(save_product(
            &mut s,
            meta(&setup, "product.save"),
            edit.clone()
        )),
        ["price.below_floor"]
    );
    let d2 = product_detail(&s, id(&row.id))?;
    assert_eq!((d2.product.name.as_str(), d2.images.len()), ("Polo", 1));
    edit.prices[0].price_minor = 160_000;
    let after = save_product(&mut s, meta(&setup, "product.save"), edit)?;
    let d3 = product_detail(&s, id(&after.id))?;
    assert_eq!(
        (
            d3.product.name.as_str(),
            d3.images.len(),
            d3.prices[0].price_minor
        ),
        ("Polo coton", 0, 160_000)
    );
    Ok(())
}

#[test]
fn a_new_product_with_everything() -> R {
    let (mut s, setup) = store();
    let code = next_internal_barcodes(&s, 1)?.remove(0);
    let mut input = product("T-shirt coton", "TSH", 150_000);
    input.cost_minor = Some(90_000);
    input.extra_units = vec![("ctn".into(), "12".into())];
    input.barcodes = vec![code];
    let save = ProductSave {
        input,
        prices: vec![PriceInput {
            list: "retail".into(),
            price_minor: 140_000,
            min_qty: "10".into(),
            branch_id: None,
            customer_id: None,
        }],
        variants: vec![AttributeInput {
            name: "Taille".into(),
            values: vec!["S".into(), "M".into(), "L".into()],
        }],
        ..Default::default()
    };
    let row = save_product(&mut s, meta(&setup, "product.save"), save)?;
    assert_eq!(product_detail(&s, id(&row.id))?.variants.len(), 3);
    Ok(())
}
