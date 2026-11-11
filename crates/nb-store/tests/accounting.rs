//! Basic accounting (M7): the chart and its groups, the roles of the entry
//! matrix, tills and banks, fiscal periods that refuse entries once
//! closed, and reports that always balance.

mod common;

use common::*;
use nb_store::Error;
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::accounting::*;
use nb_store::usecases::ledger::{EntryLineInput, post_entry, reverse_entry};
use nb_store::usecases::org::{settings, update_settings};
use nb_store::usecases::products::{Opening, create_product};
use nb_store::{OpMeta, Store};

type R = Result<(), Error>;

fn err<T: std::fmt::Debug>(r: Result<T, Error>) -> Vec<&'static str> {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.rule_codes().into_iter().map(|(_, c)| c).collect(),
    }
}

fn line(a: &str, d: i64, c: i64) -> EntryLineInput {
    EntryLineInput {
        account: a.into(),
        debit_minor: d,
        credit_minor: c,
    }
}

fn post(
    s: &mut Store,
    setup: &nb_store::usecases::setup::SetupResult,
    memo: &str,
    lines: Vec<EntryLineInput>,
) -> Result<String, Error> {
    Ok(post_entry(s, meta(setup, "journal.post"), memo, lines)?.id)
}

fn healthy(s: &Store) -> R {
    assert!(
        s.read(|r| nb_store::repo::ledger::unbalanced_entries(r.conn()))?
            .is_empty()
    );
    let tb = trial_balance(s, None, None)?;
    assert_eq!(tb.debit_minor, tb.credit_minor, "debits = credits");
    assert_eq!(tb.closing_debit_minor, tb.closing_credit_minor);
    assert_eq!(s.verify_audit_chain()?, None);
    Ok(())
}

#[test]
fn the_chart_is_a_tree_and_groups_total_their_accounts() -> R {
    let (mut s, setup) = store();
    let rows = chart(&s)?;
    // Every parent comes before its accounts, one level up.
    for (i, r) in rows.iter().enumerate() {
        if let Some(p) = &r.parent_code {
            let at = rows.iter().position(|x| &x.code == p).unwrap_or(usize::MAX);
            assert!(at < i, "{} after its group", r.code);
            assert_eq!(rows[at].depth + 1, r.depth);
            assert!(rows[at].is_group);
        }
    }
    // Every role posts to an active account that takes entries.
    for role in account_roles(&s)? {
        let code = role.account_code.unwrap_or_default();
        let row = rows
            .iter()
            .find(|r| r.code == code)
            .unwrap_or_else(|| panic!("{code}"));
        assert!(!row.is_group && row.active, "{}", role.role);
    }
    let mut p = product("Lait", "", 2_000);
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "10".into(),
        unit_cost_minor: 1_000,
        tracked: Default::default(),
    });
    create_product(&mut s, meta(&setup, "product.create"), p)?;
    let rows = chart(&s)?;
    let bal = |code: &str| {
        rows.iter()
            .find(|r| r.code == code)
            .map(ChartRow::balance_minor)
    };
    assert_eq!(
        (bal("30"), bal("3"), bal("101"), bal("1")),
        (Some(10_000), Some(10_000), Some(-10_000), Some(-10_000))
    );
    assert_eq!(
        rows.iter()
            .find(|r| r.code == "30")
            .map(|r| r.roles.clone()),
        Some(vec!["stock".to_owned()])
    );
    healthy(&s)
}

#[test]
fn accounts_are_added_under_their_group() -> R {
    let (mut s, setup) = store();
    let input = |code: &str, parent: &str| AccountInput {
        code: code.into(),
        name: "Pertes diverses".into(),
        parent_code: Some(parent.into()),
        kind: None,
        is_group: false,
    };
    assert_eq!(
        err(create_account(
            &mut s,
            meta(&setup, "account"),
            input("6A", "65")
        )),
        ["account.code_format"]
    );
    assert_eq!(
        err(create_account(
            &mut s,
            meta(&setup, "account"),
            input("6201", "62")
        )),
        ["account.parent_not_group"]
    );
    assert_eq!(
        err(create_account(
            &mut s,
            meta(&setup, "account"),
            input("7001", "65")
        )),
        ["account.code_prefix"]
    );
    assert_eq!(
        err(create_account(
            &mut s,
            meta(&setup, "account"),
            input("658", "65")
        )),
        ["account.duplicate"]
    );
    let a = create_account(&mut s, meta(&setup, "account"), input("6581", "65"))?;
    assert_eq!(
        (a.kind.as_str(), a.parent_code.as_deref()),
        ("expense", Some("65"))
    );
    // A cashier keeps out of the chart.
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
    let as_cashier = OpMeta::new(
        "account",
        Some(setup.org),
        Some(cashier.id.parse().unwrap_or_default()),
    );
    assert!(matches!(
        create_account(&mut s, as_cashier, input("6582", "65")),
        Err(Error::Forbidden {
            permission: "accounting.chart",
            ..
        })
    ));

    // Accounts in use stay active; an idle one may rest.
    assert_eq!(
        err(update_account(
            &mut s,
            meta(&setup, "account"),
            "658",
            "Pertes",
            false
        )),
        ["account.role_in_use"]
    );
    assert_eq!(
        err(update_account(
            &mut s,
            meta(&setup, "account"),
            "65",
            "Charges",
            false
        )),
        ["account.has_active_children"]
    );
    post(
        &mut s,
        &setup,
        "Frais",
        vec![line("6581", 300, 0), line("531", 0, 300)],
    )?;
    assert_eq!(
        err(update_account(
            &mut s,
            meta(&setup, "account"),
            "6581",
            "Pertes",
            false
        )),
        ["account.balance_not_zero"]
    );
    create_account(&mut s, meta(&setup, "account"), input("6582", "65"))?;
    update_account(&mut s, meta(&setup, "account"), "6582", "Divers", false)?;
    assert_eq!(
        err(post(
            &mut s,
            &setup,
            "Frais",
            vec![line("6582", 300, 0), line("531", 0, 300)]
        )),
        ["ledger.inactive_account"]
    );
    healthy(&s)
}

#[test]
fn tills_and_banks_have_their_accounts_and_roles_can_move() -> R {
    let (mut s, setup) = store();
    let list = money_accounts(&s)?;
    assert_eq!(
        list.iter()
            .map(|m| (m.kind.as_str(), m.account_code.as_str()))
            .collect::<Vec<_>>(),
        [("cash", "531"), ("bank", "5121")]
    );
    let till = create_money_account(
        &mut s,
        meta(&setup, "money"),
        "cash",
        "Caisse 2",
        Some(setup.branch.to_string()),
    )?;
    let bank = create_money_account(&mut s, meta(&setup, "money"), "bank", "CPA", None)?;
    assert_eq!(
        (till.account_code.as_str(), bank.account_code.as_str()),
        ("532", "5122")
    );
    assert_eq!(
        err(create_money_account(
            &mut s,
            meta(&setup, "money"),
            "safe",
            "X",
            None
        )),
        ["invalid_value"]
    );
    post(
        &mut s,
        &setup,
        "Versement",
        vec![line("5122", 50_000, 0), line("532", 0, 50_000)],
    )?;
    let list = money_accounts(&s)?;
    let bal = |code: &str| {
        list.iter()
            .find(|m| m.account_code == code)
            .map(|m| m.balance_minor)
    };
    assert_eq!((bal("5122"), bal("532")), (Some(50_000), Some(-50_000)));

    // The till of automatic entries can change; a group cannot take it.
    assert_eq!(
        err(set_account_role(&mut s, meta(&setup, "role"), "cash", "53")),
        ["ledger.group_account"]
    );
    assert_eq!(
        err(set_account_role(
            &mut s,
            meta(&setup, "role"),
            "nope",
            "532"
        )),
        ["invalid_value"]
    );
    set_account_role(&mut s, meta(&setup, "role"), "cash", "532")?;
    let roles = account_roles(&s)?;
    assert_eq!(
        roles
            .iter()
            .find(|r| r.role == "cash")
            .and_then(|r| r.account_code.clone()),
        Some("532".into())
    );
    healthy(&s)
}

#[test]
fn closed_periods_refuse_entries() -> R {
    let (mut s, setup) = store();
    let years = fiscal_years(&s)?;
    assert_eq!(years.len(), 1);
    let year = &years[0];
    assert_eq!(
        (year.name.as_str(), year.start.as_str(), year.end.as_str()),
        ("2026", "2026-01-01", "2026-12-31")
    );
    let p = |n: usize| year.periods[n - 1].id.clone();
    post(
        &mut s,
        &setup,
        "Apport",
        vec![line("531", 1_000, 0), line("101", 0, 1_000)],
    )?;

    assert_eq!(
        err(close_period(&mut s, meta(&setup, "close"), &p(2))),
        ["fiscal.close_in_order"]
    );
    for n in 1..=11 {
        close_period(&mut s, meta(&setup, "close"), &p(n))?;
    }
    // Today (10 November) is in period 11: nothing posts, by any path.
    assert_eq!(
        err(post(
            &mut s,
            &setup,
            "Apport",
            vec![line("531", 1_000, 0), line("101", 0, 1_000)]
        )),
        ["fiscal.period_closed"]
    );
    let mut lait = product("Lait", "", 2_000);
    lait.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "1".into(),
        unit_cost_minor: 1_000,
        tracked: Default::default(),
    });
    assert_eq!(
        err(create_product(
            &mut s,
            meta(&setup, "product.create"),
            lait.clone()
        )),
        ["fiscal.period_closed"]
    );
    assert!(
        s.connection()
            .execute(
                "INSERT INTO journal_entries (id, org_id, number, date, memo, source, total_minor, operation_id, created_at, device_id)
                 SELECT 'x', org_id, 'X', '2026-11-05', 'm', 'manual', 0, 'op', 0, 'd' FROM journal_entries LIMIT 1",
                [],
            )
            .is_err(),
        "the database refuses it too"
    );
    assert_eq!(
        err(reopen_period(
            &mut s,
            meta(&setup, "reopen"),
            &p(10),
            "erreur"
        )),
        ["fiscal.reopen_in_order"]
    );
    assert_eq!(
        err(reopen_period(&mut s, meta(&setup, "reopen"), &p(11), " ")),
        ["required"]
    );
    reopen_period(&mut s, meta(&setup, "reopen"), &p(11), "facture oubliée")?;
    create_product(&mut s, meta(&setup, "product.create"), lait)?;

    // The calendar moves only while nothing is posted.
    let mut set = settings(&s)?;
    set.fiscal_year_start = 7;
    assert_eq!(
        err(update_settings(&mut s, meta(&setup, "settings"), set)),
        ["fiscal.start_in_use"]
    );
    healthy(&s)
}

#[test]
fn a_new_store_may_move_its_fiscal_calendar() -> R {
    let (mut s, setup) = store();
    let mut set = settings(&s)?;
    set.fiscal_year_start = 7;
    update_settings(&mut s, meta(&setup, "settings"), set)?;
    assert!(fiscal_years(&s)?.is_empty());
    post(
        &mut s,
        &setup,
        "Apport",
        vec![line("531", 1_000, 0), line("101", 0, 1_000)],
    )?;
    let years = fiscal_years(&s)?;
    assert_eq!(
        (years[0].name.as_str(), years[0].start.as_str()),
        ("2026-2027", "2026-07-01")
    );
    Ok(())
}

#[test]
fn journal_trial_balance_and_statement_agree() -> R {
    let (mut s, setup) = store();
    post(
        &mut s,
        &setup,
        "Apport en caisse",
        vec![line("531", 100_000, 0), line("101", 0, 100_000)],
    )?;
    let fee = post(
        &mut s,
        &setup,
        "Frais bancaires",
        vec![line("62", 2_500, 0), line("531", 0, 2_500)],
    )?;
    reverse_entry(&mut s, meta(&setup, "journal.reverse"), &fee, "doublon")?;
    post(
        &mut s,
        &setup,
        "Frais bancaires",
        vec![line("62", 1_500, 0), line("5121", 0, 1_500)],
    )?;

    let page = journal(
        &s,
        &JournalQuery {
            limit: 10,
            ..Default::default()
        },
    )?;
    assert_eq!(page.total, 4);
    let reversed = page
        .rows
        .iter()
        .find(|e| e.id == fee)
        .map(|e| e.reversed_by.is_some());
    assert_eq!(reversed, Some(true));
    assert_eq!(page.rows.iter().filter(|e| e.reverses.is_some()).count(), 1);
    let by_account = journal(
        &s,
        &JournalQuery {
            account: Some("5121".into()),
            limit: 10,
            ..Default::default()
        },
    )?;
    assert_eq!(by_account.total, 1);
    assert_eq!(
        by_account.rows[0].lines[1].account_name,
        "Banque principale"
    );
    let by_text = journal(
        &s,
        &JournalQuery {
            text: "apport".into(),
            limit: 10,
            ..Default::default()
        },
    )?;
    assert_eq!(by_text.total, 1);
    assert_eq!(
        err(journal(
            &s,
            &JournalQuery {
                from: Some("10/11/2026".into()),
                limit: 10,
                ..Default::default()
            }
        )),
        ["date.format"]
    );

    let tb = trial_balance(&s, Some("2026-01-01"), Some("2026-12-31"))?;
    assert_eq!((tb.debit_minor, tb.credit_minor), (106_500, 106_500));
    let row = |code: &str| {
        tb.rows
            .iter()
            .find(|r| r.code == code)
            .map(|r| r.closing_minor)
    };
    assert_eq!(
        (row("531"), row("53"), row("5"), row("62")),
        (Some(100_000), Some(100_000), Some(98_500), Some(1_500))
    );
    // Next year brings this year's balances forward.
    let next = trial_balance(&s, Some("2027-01-01"), None)?;
    assert_eq!(
        next.rows
            .iter()
            .find(|r| r.code == "531")
            .map(|r| (r.opening_minor, r.debit_minor)),
        Some((100_000, 0))
    );
    assert_eq!(
        err(trial_balance(&s, Some("2026-12-31"), Some("2026-01-01"))),
        ["date.range"]
    );

    let st = statement(&s, "531", None, None)?;
    assert_eq!(
        st.lines.iter().map(|l| l.balance_minor).collect::<Vec<_>>(),
        [100_000, 97_500, 100_000]
    );
    assert_eq!(
        (st.debit_minor, st.credit_minor, st.closing_minor),
        (102_500, 2_500, 100_000)
    );
    // A group's ledger holds its accounts' lines.
    let st = statement(&s, "5", None, None)?;
    assert_eq!((st.lines.len(), st.closing_minor), (4, 98_500));
    let later = statement(&s, "531", Some("2027-01-01"), None)?;
    assert_eq!((later.opening_minor, later.lines.len()), (100_000, 0));
    assert!(matches!(
        statement(&s, "999", None, None),
        Err(Error::NotFound(_))
    ));
    healthy(&s)
}

#[test]
fn an_older_database_gains_groups_roles_and_tills() -> R {
    use nb_store::migrate::{latest, migrate_to};
    let mut conn = rusqlite::Connection::open_in_memory()?;
    migrate_to(&mut conn, 9, None)?;
    conn.execute_batch(
        "INSERT INTO organizations (id, name, country, currency, currency_exp, activity, created_at, updated_at, device_id)
           VALUES ('o1', 'Old', 'DZ', 'DZD', 2, 'grocery', 0, 0, 'd');
         INSERT INTO accounts (org_id, code, name, kind) VALUES
           ('o1', '101', 'Capital', 'equity'), ('o1', '30', 'Stocks', 'asset'),
           ('o1', '53', 'Caisse', 'asset'), ('o1', '512', 'Banque', 'asset'),
           ('o1', '658', 'Pertes', 'expense'), ('o1', '758', 'Gains', 'income');",
    )?;
    migrate_to(&mut conn, latest(), None)?;
    let parent = |code: &str| -> Option<String> {
        conn.query_row(
            "SELECT parent_code FROM accounts WHERE code = ?1",
            [code],
            |r| r.get(0),
        )
        .ok()
        .flatten()
    };
    assert_eq!(
        (parent("30"), parent("531"), parent("53"), parent("5121")),
        (
            Some("3".into()),
            Some("53".into()),
            Some("5".into()),
            Some("512".into())
        )
    );
    let groups: i64 = conn.query_row(
        "SELECT COUNT(*) FROM accounts WHERE code IN ('53', '512') AND is_group = 1",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(groups, 2);
    let roles: i64 = conn.query_row("SELECT COUNT(*) FROM account_roles", [], |r| r.get(0))?;
    assert_eq!(roles, 16);
    let tills: Vec<String> = conn
        .prepare("SELECT id FROM money_accounts ORDER BY kind")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    assert_eq!(tills.len(), 2);
    assert!(
        tills
            .iter()
            .all(|id| id.parse::<nb_core::uuid::Uuid>().is_ok()),
        "{tills:?}"
    );
    Ok(())
}

#[test]
fn the_chart_speaks_the_store_language() -> R {
    use nb_store::usecases::setup::{DZ_CHART, DZ_CHART_AR};
    for (code, ..) in DZ_CHART {
        assert!(DZ_CHART_AR.iter().any(|(c, _)| c == code), "{code}");
    }
    let mut s = Store::open_in_memory(options("A"))?;
    let mut input = setup_input();
    input.language = "ar".into();
    nb_store::usecases::setup::setup_organization(&mut s, input)?;
    let rows = chart(&s)?;
    assert_eq!(
        rows.iter()
            .find(|r| r.code == "531")
            .map(|r| r.name.as_str()),
        Some("الصندوق الرئيسي")
    );
    assert_eq!(money_accounts(&s)?[0].name, "الصندوق الرئيسي");
    Ok(())
}
