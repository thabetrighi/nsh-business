//! The till survives what a shop goes through (M8.14, gate G3): the power
//! going off in the middle of a sale, the app starting again with no
//! network, and a full disk. Nothing committed is lost, nothing is sold
//! twice when the cashier taps again, and the books still add up.
//!
//! The power cut is real: a child process sells in a loop on a database
//! file and is killed (SIGKILL) at random moments; the test then opens the
//! file again, checks it end to end, and sends again the sale that was in
//! flight — as the till does when it comes back.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::*;
use nb_core::OrganizationId;
use nb_domain::ProductId;
use nb_domain::UserId;
use nb_store::usecases::inventory::available;
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::sales::*;
use nb_store::usecases::setup::setup_organization;
use nb_store::{Error, OpMeta, Store};

type R = Result<(), Error>;

/// What the seller needs, written by the test for the child process.
#[derive(serde::Serialize, serde::Deserialize)]
struct Plan {
    org: String,
    owner: String,
    register: String,
    warehouse: String,
    product: String,
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nb-{name}-{}",
        nb_core::uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("temp dir: {e}"));
    dir
}

/// A shop on a file: 1 000 units of milk, an open till.
fn shop(dir: &Path) -> Result<(Store, Plan), Error> {
    let mut s = Store::open(&dir.join("store.db"), options("A"))?;
    let setup = setup_organization(&mut s, setup_input())?;
    let mut p = product("Lait", "", 1_190);
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "1000".into(),
        unit_cost_minor: 800,
        tracked: Default::default(),
    });
    let milk = create_product(&mut s, meta(&setup, "product.create"), p)?.id;
    let register = s
        .read(|r| {
            let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
            nb_store::usecases::org::registers(r.conn(), o.org_id()?)
        })?
        .remove(0)
        .id;
    open_shift(&mut s, meta(&setup, "shift.open"), &register, 0)?;
    let plan = Plan {
        org: setup.org.to_string(),
        owner: setup.owner.to_string(),
        register,
        warehouse: setup.warehouse.to_string(),
        product: milk,
    };
    Ok((s, plan))
}

/// The `i`-th sale: 1 to 3 milk, paid exactly in cash.
fn nth_sale(plan: &Plan, i: u64) -> SaleInput {
    let qty = 1 + i % 3;
    SaleInput {
        register_id: plan.register.clone(),
        warehouse_id: plan.warehouse.clone(),
        lines: vec![SaleLineInput {
            product_id: plan.product.clone(),
            qty: qty.to_string(),
            ..Default::default()
        }],
        payments: vec![PaymentInput {
            method: "cash".into(),
            amount_minor: i64::try_from(qty).unwrap_or(1) * 1_190,
        }],
        ..Default::default()
    }
}

fn op(plan: &Plan, id: &str) -> OpMeta {
    OpMeta {
        id: id.parse().unwrap_or_else(|_| panic!("operation id {id}")),
        kind: "sale.complete",
        org: plan.org.parse::<OrganizationId>().ok(),
        actor: plan.owner.parse::<UserId>().ok(),
    }
}

fn sales_of(s: &Store, op_id: &str) -> i64 {
    s.connection()
        .query_row(
            "SELECT COUNT(*) FROM sales WHERE operation_id = ?1",
            [op_id],
            |r| r.get(0),
        )
        .unwrap_or(-1)
}

/// Run by [`a_power_cut_loses_no_sale_and_doubles_none`] in a process of
/// its own: sells until it is killed, saying which operation it starts
/// ("begin <i> <id>") and which committed ("done <id>").
#[test]
#[ignore = "the power-cut test runs it in a child process"]
fn sells_until_the_power_goes() -> R {
    let Ok(dir) = std::env::var("NB_CRASH_DIR") else {
        return Ok(());
    };
    let dir = PathBuf::from(dir);
    let plan: Plan = serde_json::from_str(
        &std::fs::read_to_string(dir.join("plan.json"))
            .map_err(|e| Error::Backup(e.to_string()))?,
    )?;
    let start: u64 = std::env::var("NB_CRASH_FROM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut s = Store::open(&dir.join("store.db"), options("A"))?;
    let mut out = std::io::stdout().lock();
    for i in start.. {
        let meta = OpMeta::new(
            "sale.complete",
            plan.org.parse().ok(),
            plan.owner.parse().ok(),
        );
        let _ = writeln!(out, "begin {i} {}", meta.id);
        let _ = out.flush();
        complete_sale(&mut s, meta, nth_sale(&plan, i))?;
        let _ = writeln!(out, "done {}", meta.id);
        let _ = out.flush();
    }
    Ok(())
}

#[test]
fn a_power_cut_loses_no_sale_and_doubles_none() -> R {
    let dir = temp_dir("power-cut");
    let (s, plan) = shop(&dir)?;
    drop(s);
    std::fs::write(dir.join("plan.json"), serde_json::to_string(&plan)?)
        .map_err(|e| Error::Backup(e.to_string()))?;
    let exe = std::env::current_exe().map_err(|e| Error::Backup(e.to_string()))?;

    let mut committed: Vec<String> = Vec::new();
    let mut next = 0u64;
    let mut cut_mid_sale = 0;
    // Cuts after a varying number of sales, at varying moments of the next.
    for (round, (after, wait_us)) in [(3usize, 0u64), (7, 150), (5, 600), (9, 40), (4, 1_200)]
        .into_iter()
        .enumerate()
    {
        let mut child = Command::new(&exe)
            .args([
                "sells_until_the_power_goes",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("NB_CRASH_DIR", &dir)
            .env("NB_CRASH_FROM", next.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Error::Backup(e.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(Error::Backup("no stdout".into()))?;
        let mut lines = BufReader::new(stdout).lines();
        let mut in_flight: Option<(u64, String)> = None;
        let mut done_here = 0;
        while done_here < after {
            let Some(Ok(line)) = lines.next() else {
                panic!("round {round}: the seller stopped early");
            };
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next(), parts.next()) {
                (Some("begin"), Some(i), Some(id)) => {
                    in_flight = Some((i.parse().unwrap_or(0), id.to_owned()));
                }
                (Some("done"), Some(id), None) => {
                    committed.push(id.to_owned());
                    in_flight = None;
                    done_here += 1;
                }
                _ => {}
            }
        }
        // The power goes off somewhere in the next sale.
        std::thread::sleep(std::time::Duration::from_micros(wait_us));
        child.kill().map_err(|e| Error::Backup(e.to_string()))?;
        // What the child wrote before dying.
        for line in lines.map_while(Result::ok) {
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next(), parts.next()) {
                (Some("begin"), Some(i), Some(id)) => {
                    in_flight = Some((i.parse().unwrap_or(0), id.to_owned()));
                }
                (Some("done"), Some(id), None) => {
                    committed.push(id.to_owned());
                    in_flight = None;
                }
                _ => {}
            }
        }
        let _ = child.wait();

        // The app starts again: the file is whole and adds up.
        let mut s = Store::open(&dir.join("store.db"), options("A"))?;
        let health = s.health()?;
        assert!(health.is_sound(), "round {round}: {health:?}");
        for id in &committed {
            assert_eq!(
                sales_of(&s, id),
                1,
                "round {round}: committed sale {id} lost"
            );
        }
        // The cashier taps "pay" again for the sale that was in flight:
        // it is there once, whether the cut came before or after its commit.
        if let Some((i, id)) = in_flight {
            cut_mid_sale += 1;
            let first = complete_sale(&mut s, op(&plan, &id), nth_sale(&plan, i))?;
            let again = complete_sale(&mut s, op(&plan, &id), nth_sale(&plan, i))?;
            assert_eq!(first, again);
            assert_eq!(
                sales_of(&s, &id),
                1,
                "round {round}: in-flight sale doubled"
            );
            committed.push(id);
            next = i + 1;
        } else {
            next += u64::try_from(after).unwrap_or(0);
        }
        drop(s);
    }

    assert!(cut_mid_sale > 0, "the power never went off during a sale");
    // Everything sold once, the stock and the cash follow, numbers run on.
    let s = Store::open(&dir.join("store.db"), options("A"))?;
    let sales: i64 = s
        .connection()
        .query_row("SELECT COUNT(*) FROM sales", [], |r| r.get(0))?;
    assert_eq!(
        usize::try_from(sales).unwrap_or(0),
        committed.len(),
        "every sale once"
    );
    let (qty, cash, total): (i64, i64, i64) = s.connection().query_row(
        "SELECT (SELECT SUM(qty_milli) FROM sale_lines),
                (SELECT SUM(amount_minor) FROM payments WHERE method = 'cash'),
                (SELECT SUM(total_minor) FROM sales)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    assert_eq!(cash, total, "every sale paid once");
    let milk: ProductId = plan
        .product
        .parse()
        .unwrap_or_else(|_| panic!("product id"));
    assert_eq!(
        nb_store::repo::to_milli(available(
            &s,
            milk,
            plan.warehouse
                .parse()
                .unwrap_or_else(|_| panic!("warehouse"))
        )?)?,
        1_000_000 - qty
    );
    // Invoice numbers: no gap, no repeat (a cut sale gives its number back).
    let numbers: Vec<String> = {
        let mut st = s
            .connection()
            .prepare("SELECT number FROM sales ORDER BY created_at, rowid")?;
        st.query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?
    };
    let mut seq: Vec<u32> = numbers
        .iter()
        .filter_map(|n| n.rsplit('A').next().and_then(|d| d.parse().ok()))
        .collect();
    seq.sort_unstable();
    assert_eq!(
        seq,
        (1..=u32::try_from(numbers.len()).unwrap_or(0)).collect::<Vec<_>>()
    );
    let shift = current_shift(&s, &plan.register)?.unwrap_or_else(|| panic!("open shift"));
    assert_eq!(shift_summary(&s, &shift.id)?.cash_sales_minor, total);
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn the_till_starts_again_offline_where_it_stopped() -> R {
    let dir = temp_dir("restart");
    let (mut s, plan) = shop(&dir)?;
    let first = complete_sale(
        &mut s,
        op(&plan, &nb_core::uuid::Uuid::now_v7().to_string()),
        nth_sale(&plan, 0),
    )?;
    let held = nb_store::usecases::returns::hold_cart(
        &mut s,
        op(&plan, &nb_core::uuid::Uuid::now_v7().to_string()),
        &plan.register,
        "Ali",
        1,
        1_190,
        "{}",
    )?;
    drop(s);
    // Closed and opened again (no network needed at any point).
    let mut s = Store::open(&dir.join("store.db"), options("A"))?;
    assert!(s.health()?.is_sound());
    let shift = current_shift(&s, &plan.register)?;
    assert!(shift.is_some(), "the till is still open");
    assert_eq!(
        nb_store::usecases::returns::held_carts(&s, &plan.register)?,
        vec![held]
    );
    let second = complete_sale(
        &mut s,
        op(&plan, &nb_core::uuid::Uuid::now_v7().to_string()),
        nth_sale(&plan, 1),
    )?;
    assert!(first.number.ends_with("A00001") && second.number.ends_with("A00002"));
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn a_full_disk_refuses_the_sale_and_keeps_the_rest() -> R {
    let dir = temp_dir("full");
    let (mut s, plan) = shop(&dir)?;
    // Fill the room: a sale no longer fits.
    let pages: i64 = s
        .connection()
        .query_row("PRAGMA page_count", [], |r| r.get(0))?;
    s.connection()
        .query_row(&format!("PRAGMA max_page_count = {pages}"), [], |r| {
            r.get::<_, i64>(0)
        })?;
    // Sales fill the free pages left, until one no longer fits.
    let mut refused = None;
    for i in 0..500 {
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        if let Err(e) = complete_sale(&mut s, op(&plan, &id), nth_sale(&plan, i)) {
            assert_eq!(e.rule_codes(), vec![(None, "storage.full")]);
            refused = Some(id);
            break;
        }
    }
    let id = refused.unwrap_or_else(|| panic!("the disk never filled up"));
    assert_eq!(sales_of(&s, &id), 0, "nothing of the refused sale");
    assert!(s.health()?.is_sound());
    // Room is made: the same tap goes through, once.
    s.connection()
        .query_row("PRAGMA max_page_count = 1073741823", [], |r| {
            r.get::<_, i64>(0)
        })?;
    complete_sale(&mut s, op(&plan, &id), nth_sale(&plan, 0))?;
    assert_eq!(sales_of(&s, &id), 1);
    assert!(s.health()?.is_sound());
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
