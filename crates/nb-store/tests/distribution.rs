//! Distribution (M16): zones and customers on the map, vehicles with their
//! own stock, tours planned from waiting orders, worked stop by stop by
//! their driver, watched on the board, and settled.

mod common;

use common::*;
use nb_domain::UserId;
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::customers::{CustomerInput, get_customer, save_customer};
use nb_store::usecases::distribution::*;
use nb_store::usecases::documents::{DocumentInput, approve_document, save_document};
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::sales::{SaleLineInput, open_shift};
use nb_store::{Error, OpMeta};

type R = Result<(), Error>;

fn err<T: std::fmt::Debug>(r: Result<T, Error>) -> Vec<&'static str> {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.rule_codes().into_iter().map(|(_, c)| c).collect(),
    }
}

fn customer(s: &mut nb_store::Store, m: OpMeta, name: &str) -> Result<String, Error> {
    Ok(save_customer(
        s,
        m,
        CustomerInput {
            kind: "company".into(),
            name: name.into(),
            phone: "0550123456".into(),
            credit_limit_minor: 1_000_000,
            status: "active".into(),
            ..Default::default()
        },
    )?
    .id)
}

fn as_user(setup: &nb_store::usecases::setup::SetupResult, id: &str, kind: &'static str) -> OpMeta {
    let user: UserId = id.parse().unwrap_or(setup.owner);
    OpMeta::new(kind, Some(setup.org), Some(user))
}

#[test]
fn a_delivery_day_from_orders_to_settled_cash() -> R {
    let (mut s, setup) = store();
    let wh = setup.warehouse.to_string();
    let mut p = product("Eau 1.5L", "", 5_000);
    p.opening = Some(Opening {
        warehouse_id: wh.clone(),
        qty: "100".into(),
        unit_cost_minor: 3_000,
        tracked: Default::default(),
    });
    let water = create_product(&mut s, meta(&setup, "p"), p)?.id;

    // A zone and three customers, two of them placed on the map.
    let north = save_zone(
        &mut s,
        meta(&setup, "zone"),
        ZoneInput {
            name: "Nord".into(),
            color: 2,
            ..Default::default()
        },
    )?;
    assert_eq!(
        err(save_zone(
            &mut s,
            meta(&setup, "zone"),
            ZoneInput {
                name: "nord".into(),
                ..Default::default()
            }
        )),
        ["duplicate"]
    );
    let far = customer(&mut s, meta(&setup, "c"), "Épicerie Loin")?;
    let near = customer(&mut s, meta(&setup, "c"), "Superette Proche")?;
    let nowhere = customer(&mut s, meta(&setup, "c"), "Kiosque")?;
    for (c, at) in [
        (&far, "https://www.google.com/maps/@36.800000,3.000000,15z"),
        (&near, "36.710000, 3.000000"),
    ] {
        set_customer_place(
            &mut s,
            meta(&setup, "place"),
            c,
            CustomerPlace {
                zone_id: Some(north.id.clone()),
                position: at.into(),
                visit_days: 1 | 4, // Monday, Wednesday
                delivery_note: "Porte arrière".into(),
            },
        )?;
    }
    assert_eq!(
        err(set_customer_place(
            &mut s,
            meta(&setup, "place"),
            &nowhere,
            CustomerPlace {
                position: "à côté de la mosquée".into(),
                ..Default::default()
            }
        )),
        ["geo.unreadable"]
    );
    let monday = route_customers(
        &s,
        &RouteCustomerQuery {
            days: 1,
            ..Default::default()
        },
    )?;
    assert_eq!(monday.len(), 2);
    assert_eq!(list_zones(&s)?[0].customers, 2);
    let unplaced = route_customers(
        &s,
        &RouteCustomerQuery {
            unplaced: true,
            ..Default::default()
        },
    )?;
    assert_eq!(unplaced.len(), 1);

    // An approved order for each placed customer waits for a tour.
    let mut orders = Vec::new();
    for c in [&far, &near] {
        let d = save_document(
            &mut s,
            meta(&setup, "doc"),
            DocumentInput {
                kind: "order".into(),
                warehouse_id: wh.clone(),
                customer_id: Some(c.to_string()),
                lines: vec![SaleLineInput {
                    product_id: water.clone(),
                    qty: "10".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )?;
        orders.push(
            approve_document(&mut s, meta(&setup, "doc"), &d.row.id)?
                .row
                .id,
        );
    }
    assert_eq!(open_orders(&s, None)?.len(), 2);
    assert_eq!(open_orders(&s, Some(&north.id))?.len(), 2);

    // A van with its own warehouse, and its driver.
    let driver = create_user(
        &mut s,
        meta(&setup, "user"),
        UserInput {
            name: "Yacine".into(),
            phone: None,
            login: None,
            password: None,
            pin: Some("4826".into()),
            grants: vec![GrantRow::org("driver")],
        },
    )?;
    let other = create_user(
        &mut s,
        meta(&setup, "user"),
        UserInput {
            name: "Nadir".into(),
            phone: None,
            login: None,
            password: None,
            pin: Some("9153".into()),
            grants: vec![GrantRow::org("driver")],
        },
    )?;
    assert_eq!(
        err(save_vehicle(
            &mut s,
            as_user(&setup, &driver.id, "vehicle"),
            VehicleInput {
                name: "Camion 1".into(),
                kind: "truck".into(),
                ..Default::default()
            }
        )),
        ["access.denied"]
    );
    let van = save_vehicle(
        &mut s,
        meta(&setup, "vehicle"),
        VehicleInput {
            name: "Camion 1".into(),
            plate: "12345-116-39".into(),
            kind: "truck".into(),
            capacity_kg: 3_500,
            driver_id: Some(driver.id.clone()),
            speed_kmh: 40,
            ..Default::default()
        },
    )?;
    assert_ne!(van.warehouse_id, wh);
    assert_eq!(van.driver.as_deref(), Some("Yacine"));
    assert_eq!(list_vehicles(&s)?.len(), 1);

    // The tour: the orders plus a visit, put in the shortest order from
    // the first customer reached.
    assert_eq!(
        err(create_tour(
            &mut s,
            as_user(&setup, &driver.id, "tour"),
            TourInput {
                vehicle_id: van.id.clone(),
                kind: "delivery".into(),
                ..Default::default()
            }
        )),
        ["access.denied"]
    );
    let t = create_tour(
        &mut s,
        meta(&setup, "tour"),
        TourInput {
            vehicle_id: van.id.clone(),
            kind: "delivery".into(),
            orders: orders.clone(),
            customers: vec![near.clone(), nowhere.clone()],
            ..Default::default()
        },
    )?;
    assert!(t.tour.number.starts_with("TN-"), "{}", t.tour.number);
    assert_eq!(t.tour.driver_id.as_deref(), Some(driver.id.as_str()));
    assert_eq!(t.tour.day, "2026-11-10");
    // `near` has an order: no second stop for it; the kiosk has one.
    assert_eq!(t.stops.len(), 3);
    assert_eq!(t.tour.orders_minor, 2 * 10 * 5_000);
    // Ordered by waiting orders, the orders are no longer open.
    assert!(open_orders(&s, None)?.is_empty());
    assert_eq!(
        err(add_stops(
            &mut s,
            meta(&setup, "stops"),
            &t.tour.id,
            vec![],
            vec![orders[0].clone()]
        )),
        ["distribution.order_not_open"]
    );

    // Start from the depot near the "near" customer: nearest first.
    record_position(
        &mut s,
        as_user(&setup, &driver.id, "gps"),
        &van.id,
        "36.700000, 3.000000",
        Some(0),
    )?;
    let t = optimize_tour(&mut s, meta(&setup, "opt"), &t.tour.id)?;
    let names: Vec<&str> = t.stops.iter().map(|s| s.customer.as_str()).collect();
    assert_eq!(names, ["Superette Proche", "Épicerie Loin", "Kiosque"]);
    assert!(t.route_m > 9_000, "{}", t.route_m);

    // Another driver cannot work this tour; its own driver can.
    assert_eq!(
        err(start_tour(
            &mut s,
            as_user(&setup, &other.id, "start"),
            &t.tour.id,
            None
        )),
        ["distribution.not_your_tour"]
    );
    let t = start_tour(
        &mut s,
        as_user(&setup, &driver.id, "start"),
        &t.tour.id,
        Some(120_500),
    )?;
    assert_eq!(t.tour.status, "on_road");
    // The next stop is 1.1 km away: about 3 minutes at 40 km/h.
    let next = t.next().cloned().ok_or(Error::NotFound("next"))?;
    assert_eq!(next.customer, "Superette Proche");
    assert_eq!(next.eta_min, Some(3));

    // The board: on the road, the kiosk has no place.
    let b = distribution_board(&s)?;
    assert_eq!((b.on_road, b.stops_today, b.unplaced_customers), (1, 3, 0));
    assert!(
        b.alerts
            .iter()
            .any(|a| a.kind == "unplaced" && a.value == 1)
    );
    assert!(!b.alerts.iter().any(|a| a.kind == "near"));
    // Closer than a kilometre: tell the customer.
    record_position(
        &mut s,
        as_user(&setup, &driver.id, "gps"),
        &van.id,
        "36.706000, 3.000000",
        Some(30),
    )?;
    let b = distribution_board(&s)?;
    let near_alert = b
        .alerts
        .iter()
        .find(|a| a.kind == "near")
        .cloned()
        .ok_or(Error::NotFound("near"))?;
    assert_eq!(near_alert.customer.as_deref(), Some("Superette Proche"));
    mark_notified(
        &mut s,
        as_user(&setup, &driver.id, "notify"),
        near_alert.stop_id.as_deref().unwrap_or_default(),
    )?;
    assert!(
        !distribution_board(&s)?
            .alerts
            .iter()
            .any(|a| a.kind == "near")
    );

    // Delivered, with cash into the till.
    let reg = s
        .read(|r| {
            let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
            nb_store::usecases::org::registers(r.conn(), o.org_id()?)
        })?
        .remove(0)
        .id;
    open_shift(&mut s, meta(&setup, "shift"), &reg, 0)?;
    let before = get_customer(&s, &near)?.balance_minor;
    arrive_stop(&mut s, as_user(&setup, &driver.id, "arrive"), &next.id)?;
    let t = complete_stop(
        &mut s,
        as_user(&setup, &driver.id, "done"),
        &next.id,
        StopDone {
            receiver: "M. Saïd".into(),
            collected_minor: 20_000,
            method: "cash".into(),
            register_id: Some(reg.clone()),
            ..Default::default()
        },
    )?;
    assert_eq!(get_customer(&s, &near)?.balance_minor, before - 20_000);
    assert_eq!((t.tour.done, t.tour.collected_minor), (1, 20_000));
    assert_eq!(
        err(complete_stop(
            &mut s,
            as_user(&setup, &driver.id, "done"),
            &next.id,
            StopDone::default()
        )),
        ["distribution.stop_closed"]
    );
    // The far shop is closed: its order waits for another tour.
    let far_stop = t.stops[1].id.clone();
    assert_eq!(
        err(fail_stop(
            &mut s,
            as_user(&setup, &driver.id, "fail"),
            &far_stop,
            " ",
            false
        )),
        ["required"]
    );
    fail_stop(
        &mut s,
        as_user(&setup, &driver.id, "fail"),
        &far_stop,
        "Fermé",
        false,
    )?;
    assert_eq!(open_orders(&s, None)?.len(), 1);
    assert!(
        distribution_board(&s)?
            .alerts
            .iter()
            .any(|a| a.kind == "failed")
    );

    // Back: the kiosk is skipped, the cash is counted.
    assert_eq!(
        err(end_tour(
            &mut s,
            as_user(&setup, &driver.id, "end"),
            &t.tour.id,
            Some(100)
        )),
        ["distribution.km_back"]
    );
    let t = end_tour(
        &mut s,
        as_user(&setup, &driver.id, "end"),
        &t.tour.id,
        Some(120_620),
    )?;
    assert_eq!(t.tour.status, "done");
    assert_eq!(t.tour.cash_expected_minor, 20_000);
    assert_eq!(t.stops[2].status, "skipped");
    assert!(
        distribution_board(&s)?
            .alerts
            .iter()
            .any(|a| a.kind == "unsettled" && a.value == 20_000)
    );
    assert_eq!(
        err(settle_tour(
            &mut s,
            as_user(&setup, &driver.id, "settle"),
            &t.tour.id,
            20_000,
            ""
        )),
        ["access.needs_approval"]
    );
    let t = settle_tour(
        &mut s,
        meta(&setup, "settle"),
        &t.tour.id,
        19_500,
        "500 manquants",
    )?;
    assert_eq!(t.tour.cash_counted_minor, Some(19_500));
    assert!(
        !distribution_board(&s)?
            .alerts
            .iter()
            .any(|a| a.kind == "unsettled")
    );
    // The vehicle can retire only empty; it holds nothing.
    archive_vehicle(&mut s, meta(&setup, "v"), &van.id)?;
    assert!(list_vehicles(&s)?.is_empty());
    assert_eq!(vehicle_track(&s, &van.id, 10)?.len(), 2);
    Ok(())
}

#[test]
fn stops_move_and_tours_cancel() -> R {
    let (mut s, setup) = store();
    let a = customer(&mut s, meta(&setup, "c"), "A")?;
    let b = customer(&mut s, meta(&setup, "c"), "B")?;
    let c = customer(&mut s, meta(&setup, "c"), "C")?;
    let van = save_vehicle(
        &mut s,
        meta(&setup, "v"),
        VehicleInput {
            name: "Van".into(),
            kind: "van".into(),
            ..Default::default()
        },
    )?;
    assert_eq!(van.speed_kmh, 35);
    assert_eq!(
        err(create_tour(
            &mut s,
            meta(&setup, "t"),
            TourInput {
                vehicle_id: van.id.clone(),
                kind: "van".into(),
                day: "2026-02-30x".into(),
                ..Default::default()
            }
        )),
        ["invalid_value"]
    );
    let t = create_tour(
        &mut s,
        meta(&setup, "t"),
        TourInput {
            vehicle_id: van.id.clone(),
            kind: "van".into(),
            day: "2026-11-09".into(),
            customers: vec![a.clone(), b.clone(), c.clone()],
            ..Default::default()
        },
    )?;
    // Planned yesterday and not done: late.
    assert!(
        distribution_board(&s)?
            .alerts
            .iter()
            .any(|x| x.kind == "late" && x.value == 1)
    );
    let t = move_stop(&mut s, meta(&setup, "m"), &t.stops[2].id, 0)?;
    let order: Vec<&str> = t.stops.iter().map(|s| s.customer.as_str()).collect();
    assert_eq!(order, ["C", "A", "B"]);
    let t = remove_stop(&mut s, meta(&setup, "r"), &t.stops[1].id)?;
    assert_eq!(t.stops.len(), 2);
    assert_eq!(
        err(archive_vehicle(&mut s, meta(&setup, "v"), &van.id)),
        ["distribution.vehicle_busy"]
    );
    cancel_tour(&mut s, meta(&setup, "x"), &t.tour.id, "Panne")?;
    assert_eq!(
        err(start_tour(&mut s, meta(&setup, "s"), &t.tour.id, None)),
        ["distribution.tour_started"]
    );
    assert!(
        list_tours(
            &s,
            &TourQuery {
                status: "live".into(),
                limit: 10,
                ..Default::default()
            }
        )?
        .is_empty()
    );
    Ok(())
}
