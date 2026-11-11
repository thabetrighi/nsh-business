//! Identity and access (M4): users and roles, the authorization matrix
//! applied by the use cases, sign-in with lockout, supervisor approvals,
//! the organization's settings, places and features.

mod common;

use std::sync::Arc;

use common::*;
use nb_core::FixedClock;
use nb_domain::UserId;
use nb_domain::access::{Permission, Target};
use nb_store::repo::access::{GrantRow, Secret};
use nb_store::usecases::access::{
    ApprovalOutcome, ApproveInput, RoleInput, SignInOutcome, UserInput, UserUpdate, approve,
    create_user, delete_role, list_roles, list_users, save_role, session_of, set_secret,
    sign_in_password, sign_in_pin, sign_in_required, supervisors, update_user,
};
use nb_store::usecases::org::{
    BusinessProfile, OrgSettings, create_branch, features, places, set_feature,
    update_business_profile, update_settings,
};
use nb_store::usecases::products::{Opening, archive_products, create_product};
use nb_store::usecases::setup::{SetupResult, setup_organization};
use nb_store::usecases::stock::{AdjustInput, adjust_stock};
use nb_store::{Error, OpMeta, Store, StoreOptions};

type R = Result<(), Error>;

fn codes(e: &Error) -> Vec<&'static str> {
    e.rule_codes().into_iter().map(|(_, c)| c).collect()
}

/// A store with a clock the test moves.
fn timed() -> Result<(Store, SetupResult, Arc<FixedClock>), Error> {
    let clock = Arc::new(FixedClock::new(T0));
    let mut s = Store::open_in_memory(StoreOptions {
        clock: clock.clone(),
        ..options("A")
    })?;
    let setup = setup_organization(&mut s, setup_input())?;
    Ok((s, setup, clock))
}

fn as_user(s: &SetupResult, user: &str, kind: &'static str) -> OpMeta {
    OpMeta::new(kind, Some(s.org), Some(user.parse().unwrap_or_default()))
}

fn user(
    name: &str,
    login: Option<&str>,
    password: Option<&str>,
    pin: Option<&str>,
    grants: Vec<GrantRow>,
) -> UserInput {
    UserInput {
        name: name.into(),
        phone: None,
        login: login.map(Into::into),
        password: password.map(Into::into),
        pin: pin.map(Into::into),
        grants,
    }
}

fn scoped(role: &str, kind: &str, id: String) -> GrantRow {
    GrantRow {
        role: role.into(),
        scope_kind: kind.into(),
        scope_id: Some(id),
    }
}

fn adjust_input(product: &str, wh: &str, approval: Option<String>) -> AdjustInput {
    AdjustInput {
        product_id: product.into(),
        warehouse_id: wh.into(),
        kind: "waste".into(),
        qty: "1".into(),
        unit: "pc".into(),
        unit_cost_minor: None,
        reason: "casse".into(),
        may_go_negative: false,
        approval,
    }
}

/// Owner, a cashier on the first register (PIN only), a branch manager with a PIN.
struct Team {
    cashier: String,
    manager: String,
    product: String,
}

fn team(s: &mut Store, setup: &SetupResult) -> Result<Team, Error> {
    let register = places(s)?[0].registers[0].id.clone();
    let cashier = create_user(
        s,
        meta(setup, "user.create"),
        user(
            "Karim",
            None,
            None,
            Some("2580"),
            vec![scoped("cashier", "register", register)],
        ),
    )?;
    let manager = create_user(
        s,
        meta(setup, "user.create"),
        user(
            "Samira",
            Some("samira"),
            Some("gerante-2026"),
            Some("7391"),
            vec![scoped("branch_manager", "branch", setup.branch.to_string())],
        ),
    )?;
    let mut p = product("Lait Soummam 1L", "", 12_000);
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "10".into(),
        unit_cost_minor: 9_000,
    });
    let product = create_product(s, meta(setup, "product.create"), p)?.id;
    Ok(Team {
        cashier: cashier.id,
        manager: manager.id,
        product,
    })
}

#[test]
fn the_owner_can_do_everything_without_signing_in_at_first() -> R {
    let (s, setup) = store();
    assert!(!sign_in_required(&s)?);
    let me = session_of(&s, setup.owner)?;
    assert_eq!(me.permissions.len(), Permission::ALL.len());
    assert_eq!(me.auto_lock_min, 5);
    let users = list_users(&s)?;
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].grants, vec![GrantRow::org("owner")]);
    // The setup made a first register in the first branch.
    let tree = places(&s)?;
    assert_eq!(
        (
            tree.len(),
            tree[0].warehouses.len(),
            tree[0].registers.len()
        ),
        (1, 1, 1)
    );
    assert_eq!(tree[0].registers[0].code, "01");
    Ok(())
}

#[test]
fn use_cases_apply_the_roles() -> R {
    let (mut s, setup) = store();
    let t = team(&mut s, &setup)?;
    assert!(sign_in_required(&s)?, "users now have secrets");

    // A cashier cannot archive products (not sensitive: plainly denied)…
    let id = t.product.parse().map_err(|_| Error::NotFound("id"))?;
    let e = archive_products(
        &mut s,
        as_user(&setup, &t.cashier, "product.archive"),
        &[id],
    );
    let e = e.err().ok_or(Error::NotFound("expected an error"))?;
    assert_eq!(
        e.rule_codes(),
        vec![(Some("products.archive"), "access.denied")]
    );
    // …nor adjust stock (sensitive: a supervisor may approve it).
    let wh = setup.warehouse.to_string();
    let e = adjust_stock(
        &mut s,
        as_user(&setup, &t.cashier, "stock.adjust"),
        adjust_input(&t.product, &wh, None),
    );
    let e = e.err().ok_or(Error::NotFound("expected an error"))?;
    assert_eq!(
        e.rule_codes(),
        vec![(Some("stock.adjust"), "access.needs_approval")]
    );
    // Refusals are audited, outside the failed operation, and the chain holds.
    assert_eq!(
        count(&s, "SELECT COUNT(*) FROM audit_log WHERE action = 'denied'"),
        2
    );
    assert_eq!(s.verify_audit_chain()?, None);
    // The branch manager may, in their branch.
    adjust_stock(
        &mut s,
        as_user(&setup, &t.manager, "stock.adjust"),
        adjust_input(&t.product, &wh, None),
    )?;
    // Nobody signed out of the organization can do anything.
    let stranger = UserId::new().to_string();
    let e = archive_products(&mut s, as_user(&setup, &stranger, "product.archive"), &[id]);
    assert!(matches!(e, Err(Error::Forbidden { .. })));
    Ok(())
}

#[test]
fn a_supervisor_approves_once_for_a_short_while() -> R {
    let (mut s, setup, clock) = timed()?;
    let t = team(&mut s, &setup)?;
    let wh = setup.warehouse.to_string();
    let ask = |pin: &str| ApproveInput {
        supervisor_id: t.manager.clone(),
        pin: pin.into(),
        permission: "stock.adjust".into(),
        branch_id: Some(setup.branch.to_string()),
        warehouse_id: Some(wh.clone()),
        register_id: None,
        reason: "bouteille cassée".into(),
    };
    // The dialog lists who can approve: the owner has no PIN, the manager has.
    let sup = supervisors(
        &s,
        Permission::StockAdjust,
        Target::warehouse(setup.branch, setup.warehouse),
    )?;
    assert_eq!(
        sup.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
        ["Samira"]
    );

    let wrong = approve(&mut s, as_user(&setup, &t.cashier, "approval"), ask("1111"))?;
    assert_eq!(
        wrong,
        ApprovalOutcome::Refused {
            code: "access.bad_credentials".into()
        }
    );
    let ApprovalOutcome::Approved { id, expires_at } =
        approve(&mut s, as_user(&setup, &t.cashier, "approval"), ask("7391"))?
    else {
        return Err(Error::NotFound("approval"));
    };
    assert_eq!(expires_at, T0 + 5 * 60_000);
    // Used by the operation it was given for…
    adjust_stock(
        &mut s,
        as_user(&setup, &t.cashier, "stock.adjust"),
        adjust_input(&t.product, &wh, Some(id.clone())),
    )?;
    // …and only once.
    let again = adjust_stock(
        &mut s,
        as_user(&setup, &t.cashier, "stock.adjust"),
        adjust_input(&t.product, &wh, Some(id.clone())),
    );
    assert_eq!(
        codes(&again.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.approval_invalid"]
    );
    // Another user cannot use someone else's approval; an expired one is refused.
    let ApprovalOutcome::Approved { id, .. } =
        approve(&mut s, as_user(&setup, &t.cashier, "approval"), ask("7391"))?
    else {
        return Err(Error::NotFound("approval"));
    };
    clock.advance(5 * 60_000);
    let late = adjust_stock(
        &mut s,
        as_user(&setup, &t.cashier, "stock.adjust"),
        adjust_input(&t.product, &wh, Some(id)),
    );
    assert_eq!(
        codes(&late.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.approval_invalid"]
    );
    // Nobody approves for themselves; a cashier is no supervisor.
    let mut own = ask("7391");
    own.supervisor_id = t.cashier.clone();
    own.pin = "2580".into();
    let r = approve(&mut s, as_user(&setup, &t.manager, "approval"), own)?;
    assert_eq!(
        r,
        ApprovalOutcome::Refused {
            code: "access.not_supervisor".into()
        }
    );
    let me = approve(&mut s, as_user(&setup, &t.manager, "approval"), ask("7391"));
    assert_eq!(
        codes(&me.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.self_approval"]
    );
    assert_eq!(
        count(
            &s,
            "SELECT COUNT(*) FROM audit_log WHERE action = 'approve'"
        ),
        2
    );
    assert_eq!(
        count(
            &s,
            "SELECT COUNT(*) FROM audit_log WHERE action = 'approval.use'"
        ),
        1
    );
    // Approvals cannot be rewritten.
    assert!(
        s.connection()
            .execute("UPDATE approvals SET used_at = NULL", [])
            .is_err()
    );
    assert!(s.connection().execute("DELETE FROM approvals", []).is_err());
    Ok(())
}

#[test]
fn sign_in_counts_failures_and_locks_for_a_while() -> R {
    let (mut s, setup, clock) = timed()?;
    let t = team(&mut s, &setup)?;
    let ok = sign_in_password(&mut s, "  Samira ", "gerante-2026")?;
    let SignInOutcome::SignedIn(me) = ok else {
        return Err(Error::NotFound("session"));
    };
    assert_eq!(
        (me.name.as_str(), me.user_id.as_str()),
        ("Samira", t.manager.as_str())
    );
    assert!(me.permissions.contains(&"stock.adjust".to_owned()));
    assert!(!me.permissions.contains(&"users.manage".to_owned()));

    let refused = |code: &str| SignInOutcome::Refused {
        code: code.into(),
        locked_until: None,
    };
    assert_eq!(
        sign_in_password(&mut s, "nobody", "gerante-2026")?,
        refused("access.bad_credentials")
    );
    for _ in 0..4 {
        assert_eq!(
            sign_in_password(&mut s, "samira", "wrong-pass")?,
            refused("access.bad_credentials")
        );
    }
    let locked = SignInOutcome::Refused {
        code: "access.locked".into(),
        locked_until: Some(T0 + 60_000),
    };
    assert_eq!(sign_in_password(&mut s, "samira", "wrong-pass")?, locked);
    // Even the right password waits for the lock to end.
    assert_eq!(sign_in_password(&mut s, "samira", "gerante-2026")?, locked);
    clock.advance(60_000);
    assert!(matches!(
        sign_in_password(&mut s, "samira", "gerante-2026")?,
        SignInOutcome::SignedIn(_)
    ));
    // Cashiers sign in with their PIN.
    let cashier: UserId = t.cashier.parse().map_err(|_| Error::NotFound("id"))?;
    assert!(matches!(
        sign_in_pin(&mut s, cashier, "2580")?,
        SignInOutcome::SignedIn(_)
    ));
    assert_eq!(
        sign_in_pin(&mut s, cashier, "0000")?,
        refused("access.bad_credentials")
    );
    // A deactivated user is refused (said only to whoever knows the secret).
    let mut u = list_users(&s)?
        .into_iter()
        .find(|u| u.id == t.cashier)
        .ok_or(Error::NotFound("user"))?;
    u.active = false;
    update_user(
        &mut s,
        meta(&setup, "user.update"),
        UserUpdate {
            id: u.id,
            name: u.name,
            phone: None,
            login: None,
            active: false,
            grants: u.grants,
        },
    )?;
    assert_eq!(
        sign_in_pin(&mut s, cashier, "2580")?,
        refused("access.inactive")
    );
    assert_eq!(
        count(
            &s,
            "SELECT COUNT(*) FROM audit_log WHERE action = 'sign_in.refused'"
        ),
        9
    );
    // Secrets never leave the credentials table.
    for table in [
        "SELECT COUNT(*) FROM audit_log WHERE detail LIKE '%argon2%' OR detail LIKE '%2580%'",
        "SELECT COUNT(*) FROM outbox_events WHERE payload LIKE '%argon2%' OR payload LIKE '%gerante%'",
    ] {
        assert_eq!(count(&s, table), 0, "{table}");
    }
    Ok(())
}

#[test]
fn nobody_hands_out_more_than_they_hold() -> R {
    let (mut s, setup) = store();
    let gm = create_user(
        &mut s,
        meta(&setup, "user.create"),
        user(
            "Nadia",
            Some("nadia"),
            Some("manager-2026"),
            None,
            vec![GrantRow::org("general_manager")],
        ),
    )?;
    // A general manager manages users but not roles: no owner, no custom roles.
    let e = create_user(
        &mut s,
        as_user(&setup, &gm.id, "user.create"),
        user("X", None, None, None, vec![GrantRow::org("owner")]),
    );
    assert_eq!(
        codes(&e.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.escalation"]
    );
    let cashier = create_user(
        &mut s,
        as_user(&setup, &gm.id, "user.create"),
        user(
            "Yacine",
            None,
            None,
            Some("4826"),
            vec![GrantRow::org("cashier")],
        ),
    )?;
    let role = RoleInput {
        id: None,
        name: "Caissier chef".into(),
        permissions: vec!["sales.sell".into(), "sales.return".into()],
    };
    let e = save_role(&mut s, as_user(&setup, &gm.id, "role.save"), role.clone());
    assert!(matches!(
        e,
        Err(Error::Forbidden {
            permission: "roles.manage",
            ..
        })
    ));
    // The owner defines it; its name is unique; it is assigned; then it cannot go.
    let r = save_role(&mut s, meta(&setup, "role.save"), role.clone())?;
    assert!(!r.builtin);
    assert_eq!(r.permissions, ["sales.sell", "sales.return"]);
    let dup = save_role(
        &mut s,
        meta(&setup, "role.save"),
        RoleInput {
            name: "caissier CHEF".into(),
            ..role
        },
    );
    assert_eq!(
        codes(&dup.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.role_name_taken"]
    );
    update_user(
        &mut s,
        meta(&setup, "user.update"),
        UserUpdate {
            id: cashier.id.clone(),
            name: "Yacine".into(),
            phone: None,
            login: None,
            active: true,
            grants: vec![GrantRow::org(&r.id)],
        },
    )?;
    let roles = list_roles(&s)?;
    assert_eq!(roles.len(), 13);
    assert_eq!(
        roles.iter().find(|x| x.id == r.id).map(|x| x.users),
        Some(1)
    );
    let e = delete_role(&mut s, meta(&setup, "role.delete"), &r.id);
    assert_eq!(
        codes(&e.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.role_in_use"]
    );
    let e = delete_role(&mut s, meta(&setup, "role.delete"), "cashier");
    assert_eq!(
        codes(&e.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.builtin_role"]
    );
    // Unknown permission codes are refused, not dropped.
    let bad = save_role(
        &mut s,
        meta(&setup, "role.save"),
        RoleInput {
            id: None,
            name: "Z".into(),
            permissions: vec!["sales.teleport".into()],
        },
    );
    assert_eq!(
        codes(&bad.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.unknown_permission"]
    );
    Ok(())
}

#[test]
fn the_organization_keeps_an_owner() -> R {
    let (mut s, setup) = store();
    let me = UserUpdate {
        id: setup.owner.to_string(),
        name: "Ahmed".into(),
        phone: Some("0661 00 00 00".into()),
        login: Some("ahmed".into()),
        active: false,
        grants: vec![GrantRow::org("owner")],
    };
    let e = update_user(&mut s, meta(&setup, "user.update"), me.clone());
    assert_eq!(
        codes(&e.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.last_owner"]
    );
    let e = update_user(
        &mut s,
        meta(&setup, "user.update"),
        UserUpdate {
            active: true,
            grants: vec![GrantRow::org("cashier")],
            ..me.clone()
        },
    );
    assert_eq!(
        codes(&e.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.last_owner"]
    );
    let row = update_user(
        &mut s,
        meta(&setup, "user.update"),
        UserUpdate { active: true, ..me },
    )?;
    assert_eq!(
        (row.login.as_deref(), row.phone.as_deref()),
        (Some("ahmed"), Some("0661 00 00 00"))
    );
    // Users set their own secrets, with the policy checked.
    let weak = set_secret(
        &mut s,
        meta(&setup, "user.secret"),
        setup.owner,
        Secret::Pin,
        Some("1234".into()),
    );
    assert_eq!(
        codes(&weak.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.pin_weak"]
    );
    set_secret(
        &mut s,
        meta(&setup, "user.secret"),
        setup.owner,
        Secret::Password,
        Some("mon-magasin".into()),
    )?;
    assert!(matches!(
        sign_in_password(&mut s, "AHMED", "mon-magasin")?,
        SignInOutcome::SignedIn(_)
    ));
    // Logins are unique and well-formed.
    let e = create_user(
        &mut s,
        meta(&setup, "user.create"),
        user(
            "B",
            Some("ahmed"),
            None,
            None,
            vec![GrantRow::org("cashier")],
        ),
    );
    assert_eq!(
        codes(&e.err().ok_or(Error::NotFound("expected an error"))?),
        ["access.login_taken"]
    );
    let e = create_user(
        &mut s,
        meta(&setup, "user.create"),
        user("", Some("a b"), Some("x"), Some("12"), vec![]),
    );
    assert_eq!(
        codes(&e.err().ok_or(Error::NotFound("expected an error"))?),
        [
            "required",
            "access.login_format",
            "access.password_short",
            "access.pin_format",
            "access.no_role"
        ]
    );
    Ok(())
}

#[test]
fn settings_places_and_features() -> R {
    let (mut s, setup) = store();
    let f = features(&s)?;
    assert!(
        f.has(nb_domain::features::Feature::Scale) && !f.has(nb_domain::features::Feature::Routes)
    );
    // Changing the activity changes what shows (M4.3).
    let settings = OrgSettings {
        name: "En-Nour Distribution".into(),
        activity: "wholesale".into(),
        language: "fr".into(),
        fiscal_year_start: 1,
        auto_lock_min: 10,
    };
    update_settings(&mut s, meta(&setup, "org.update"), settings.clone())?;
    let f = features(&s)?;
    assert!(
        f.has(nb_domain::features::Feature::Routes) && !f.has(nb_domain::features::Feature::Scale)
    );
    let f = set_feature(&mut s, meta(&setup, "org.feature"), "scale", Some(true))?;
    assert!(f.has(nb_domain::features::Feature::Scale));
    let f = set_feature(&mut s, meta(&setup, "org.feature"), "scale", None)?;
    assert!(!f.has(nb_domain::features::Feature::Scale));
    let e = set_feature(&mut s, meta(&setup, "org.feature"), "pos", Some(false));
    assert_eq!(
        codes(&e.err().ok_or(Error::NotFound("expected an error"))?),
        ["features.core"]
    );
    let e = update_settings(
        &mut s,
        meta(&setup, "org.update"),
        OrgSettings {
            activity: "casino".into(),
            fiscal_year_start: 13,
            ..settings
        },
    );
    assert_eq!(
        codes(&e.err().ok_or(Error::NotFound("expected an error"))?),
        ["invalid_value", "invalid_value"]
    );
    // A new branch comes with its warehouse and register, numbered.
    let tree = create_branch(&mut s, meta(&setup, "branch.create"), "Touggourt")?;
    assert_eq!(tree.branch.code, "02");
    assert_eq!(
        (tree.warehouses.len(), tree.registers[0].code.as_str()),
        (1, "01")
    );
    assert_eq!(places(&s)?.len(), 2);
    // Legal identifiers are checked.
    let e = update_business_profile(
        &mut s,
        meta(&setup, "org.profile"),
        BusinessProfile {
            nif: "123".into(),
            email: "x@y".into(),
            ..Default::default()
        },
    );
    assert_eq!(
        codes(&e.err().ok_or(Error::NotFound("expected an error"))?),
        ["org.nif_format", "org.email_format"]
    );
    update_business_profile(
        &mut s,
        meta(&setup, "org.profile"),
        BusinessProfile {
            nif: "000139001234567".into(),
            nis: "000139012345678".into(),
            ..Default::default()
        },
    )?;
    // A user without settings.edit cannot change any of it.
    let c = create_user(
        &mut s,
        meta(&setup, "user.create"),
        user(
            "C",
            None,
            None,
            Some("4826"),
            vec![GrantRow::org("cashier")],
        ),
    )?;
    let e = create_branch(&mut s, as_user(&setup, &c.id, "branch.create"), "X");
    assert!(matches!(
        e,
        Err(Error::Forbidden {
            permission: "settings.edit",
            ..
        })
    ));
    Ok(())
}

#[test]
fn users_of_an_older_database_keep_their_role() -> R {
    use nb_store::migrate::{latest, migrate_to};
    let mut conn = rusqlite::Connection::open_in_memory()?;
    migrate_to(&mut conn, 5, None)?;
    conn.execute_batch(
        "INSERT INTO organizations (id, name, country, currency, currency_exp, activity, created_at, updated_at, device_id)
           VALUES ('o1', 'Old', 'DZ', 'DZD', 2, 'grocery', 0, 0, 'd');
         INSERT INTO users (id, org_id, name, role, created_at, updated_at, device_id)
           VALUES ('u1', 'o1', 'Old owner', 'owner', 0, 0, 'd');",
    )?;
    migrate_to(&mut conn, latest(), None)?;
    let (role, scope): (String, String) = conn.query_row(
        "SELECT role, scope_kind FROM user_roles WHERE user_id = 'u1'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    assert_eq!((role.as_str(), scope.as_str()), ("owner", "org"));
    let legal: String =
        conn.query_row("SELECT legal_name FROM business_profiles", [], |r| r.get(0))?;
    assert_eq!(legal, "Old");
    Ok(())
}
