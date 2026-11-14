//! Two branches of one shop on two devices (M11/M3): goods sent from El
//! Oued's device reach Biskra's in transit; Biskra's device receives them
//! (one short) without the two ever being online together, and the sender
//! sees it received. A transfer received on both devices while apart: the
//! second is refused by the server, set aside on that device (undone there,
//! noted to review), and every copy ends the same.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nb_core::FixedClock;
use nb_server::{
    CURSOR_HEADER, EPOCH_HEADER, Invite, JoinRequest, Joined, Linked, Position, PullResult,
    PushRequest, PushResult, router,
};
use nb_store::usecases::inventory::{
    TransferInput, TransferLineInput, list_transfers, receive_transfer, send_transfer,
};
use nb_store::usecases::org::create_branch;
use nb_store::usecases::products::{Opening, ProductInput, create_product};
use nb_store::usecases::setup::{SetupInput, setup_organization};
use nb_store::{Error, OpMeta, Store, StoreOptions};
use tower::ServiceExt;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const KEY: &str = "enroll-key-0123456789";

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Vec<u8>,
) -> Res<Vec<u8>> {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    let res = app.clone().oneshot(req.body(Body::from(body))?).await?;
    let status = res.status();
    let bytes = res.into_body().collect().await?.to_bytes().to_vec();
    assert_eq!(
        status,
        StatusCode::OK,
        "{method} {path}: {}",
        String::from_utf8_lossy(&bytes)
    );
    Ok(bytes)
}

async fn copy(app: &axum::Router, token: &str) -> Res<(Vec<u8>, Position)> {
    let req = Request::builder()
        .uri("/v1/snapshot")
        .header("authorization", format!("Bearer {token}"));
    let res = app.clone().oneshot(req.body(Body::empty())?).await?;
    let num =
        |n: &str| -> Res<i64> { Ok(res.headers().get(n).ok_or("position")?.to_str()?.parse()?) };
    let at = Position {
        cursor: num(CURSOR_HEADER)?,
        epoch: num(EPOCH_HEADER)?,
    };
    Ok((res.into_body().collect().await?.to_bytes().to_vec(), at))
}

/// As the app does: sends what waits; an operation refused is set aside.
async fn push(app: &axum::Router, token: &str, s: &mut Store) -> Res<Vec<String>> {
    let mut refused = Vec::new();
    loop {
        let ops = s.unpushed(300)?;
        if ops.is_empty() {
            return Ok(refused);
        }
        let bytes = call(
            app,
            "POST",
            "/v1/push",
            token,
            serde_json::to_vec(&PushRequest { ops })?,
        )
        .await?;
        let r: PushResult = serde_json::from_slice(&bytes)?;
        s.mark_pushed(&r.accepted)?;
        if let Some(f) = r.failed {
            assert_eq!(f.code, "rejected", "{f:?}");
            refused.push(f.detail.clone());
            let (db, at) = copy(app, token).await?;
            s.set_aside(f.id, &f.detail, &db, at)?;
        }
    }
}

async fn pull(app: &axum::Router, token: &str, s: &mut Store) -> Res<PullResult> {
    loop {
        let at = s.pull_position()?;
        let bytes = call(
            app,
            "GET",
            &format!("/v1/pull?after={}&epoch={}", at.cursor, at.epoch),
            token,
            Vec::new(),
        )
        .await?;
        let r: PullResult = serde_json::from_slice(&bytes)?;
        s.apply_pulled(r.ops.clone(), r.cursor)?;
        if !r.more {
            return Ok(r);
        }
    }
}

fn stock(s: &Store, product: &str, warehouse: &str) -> Result<i64, Error> {
    s.read(|r| {
        Ok(r.conn().query_row(
            "SELECT COALESCE(SUM(on_hand_milli), 0) FROM stock_balances WHERE product_id = ?1 AND warehouse_id = ?2",
            [product, warehouse],
            |row| row.get(0),
        )?)
    })
}

#[tokio::test]
async fn goods_travel_between_branches_on_different_devices() -> Res<()> {
    let dir = std::env::temp_dir().join(format!("nb-branches-{}", nb_core::OperationId::new()));
    let app = router(nb_server::Server::open(&dir, KEY).map_err(|e| format!("{e:?}"))?);
    let clock = Arc::new(FixedClock::new(1_794_304_800_000));
    let options = || StoreOptions {
        clock: clock.clone(),
        ..Default::default()
    };

    // El Oued's device: the shop, 20 bags, a second branch (Biskra).
    let mut oued = Store::open_in_memory(options())?;
    let setup = setup_organization(
        &mut oued,
        SetupInput {
            org_name: "Magasin En-Nour".into(),
            activity: "grocery".into(),
            country: "DZ".into(),
            currency: "DZD".into(),
            currency_exp: 2,
            branch_name: "El Oued".into(),
            owner_name: "Ahmed".into(),
            language: "fr".into(),
            fiscal_year_start: 1,
        },
    )?;
    let meta = |k: &'static str| OpMeta::new(k, Some(setup.org), Some(setup.owner));
    let home = setup.warehouse.to_string();
    let coffee = create_product(
        &mut oued,
        meta("p"),
        ProductInput {
            name: "Café 250g".into(),
            kind: "goods".into(),
            base_unit: "pc".into(),
            price_minor: 45_000,
            opening: Some(Opening {
                warehouse_id: home.clone(),
                qty: "20".into(),
                unit_cost_minor: 30_000,
                tracked: Default::default(),
            }),
            ..Default::default()
        },
    )?
    .id;
    let biskra = create_branch(&mut oued, meta("branch"), "Biskra")?;
    let away = biskra.warehouses[0].id.clone();
    let linked: Linked =
        serde_json::from_slice(&call(&app, "POST", "/v1/link", KEY, oued.snapshot()?).await?)?;
    oued.set_sync_link("http://server", &linked.token)?;

    // Biskra's device joins.
    let invite: Invite = serde_json::from_slice(
        &call(&app, "POST", "/v1/invite", &linked.token, Vec::new()).await?,
    )?;
    let mut bis = Store::open_in_memory(options())?;
    let joined: Joined = serde_json::from_slice(
        &call(
            &app,
            "POST",
            "/v1/join",
            "",
            serde_json::to_vec(&JoinRequest {
                code: invite.code,
                name: "Biskra".into(),
                device_id: Some(bis.device().id.to_string()),
            })?,
        )
        .await?,
    )?;
    let (db, at) = copy(&app, &joined.token).await?;
    bis.join_with(&db, at, &joined, "http://server")?;

    // El Oued sends 10; the goods leave now, in transit.
    let line = |q: &str| TransferLineInput {
        product_id: coffee.clone(),
        qty: q.into(),
        unit: String::new(),
        tracked: Default::default(),
    };
    let sent = send_transfer(
        &mut oued,
        meta("transfer"),
        TransferInput {
            from_warehouse: home.clone(),
            to_warehouse: away.clone(),
            lines: vec![line("10")],
            note: String::new(),
        },
    )?;
    assert_eq!(stock(&oued, &coffee, &home)?, 10_000);
    assert!(push(&app, &linked.token, &mut oued).await?.is_empty());

    // Biskra, later and alone, takes it and receives 9 (one broke).
    clock.advance(3_600_000);
    pull(&app, &joined.token, &mut bis).await?;
    let waiting = list_transfers(&bis, 10)?;
    assert_eq!(waiting[0].status, "sent");
    receive_transfer(&mut bis, meta("receive"), &sent.id, vec![(0, "9".into())])?;
    assert_eq!(stock(&bis, &coffee, &away)?, 9_000);
    assert!(push(&app, &joined.token, &mut bis).await?.is_empty());

    // El Oued sees it received, and the same books.
    let r = pull(&app, &linked.token, &mut oued).await?;
    let t = list_transfers(&oued, 10)?;
    assert_eq!(t[0].status, "received");
    assert_eq!(t[0].lines[0].received_milli, Some(9_000));
    assert_eq!(stock(&oued, &coffee, &away)?, 9_000);
    assert_eq!(oued.fingerprint()?, r.fingerprint);
    assert_eq!(bis.fingerprint()?, oued.fingerprint()?);

    // A second transfer received on both devices while apart: the one that
    // reaches the server second is refused, undone there, noted.
    let second = send_transfer(
        &mut oued,
        meta("transfer"),
        TransferInput {
            from_warehouse: home.clone(),
            to_warehouse: away.clone(),
            lines: vec![line("4")],
            note: String::new(),
        },
    )?;
    push(&app, &linked.token, &mut oued).await?;
    pull(&app, &joined.token, &mut bis).await?;
    receive_transfer(&mut bis, meta("receive"), &second.id, vec![])?;
    receive_transfer(
        &mut oued,
        meta("receive"),
        &second.id,
        vec![(0, "3".into())],
    )?;
    assert!(push(&app, &joined.token, &mut bis).await?.is_empty());
    let refused = push(&app, &linked.token, &mut oued).await?;
    assert_eq!(refused.len(), 1, "the second receipt is refused");
    let r = pull(&app, &linked.token, &mut oued).await?;
    assert_eq!(oued.fingerprint()?, r.fingerprint, "undone on El Oued");
    pull(&app, &joined.token, &mut bis).await?;
    assert_eq!(bis.fingerprint()?, oued.fingerprint()?);
    assert_eq!(
        stock(&oued, &coffee, &away)?,
        13_000,
        "Biskra's receipt stands"
    );
    let items = nb_store::usecases::reviews::review_items(&oued)?;
    assert!(
        items
            .iter()
            .any(|i| i.kind == "refused" && i.subject.contains("receive")),
        "{items:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
