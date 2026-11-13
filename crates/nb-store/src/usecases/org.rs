//! The organization and its places (M4.1, M4.3, M4.4): settings, legal
//! profile, activity and feature switches, branches, warehouses and cash
//! registers. Changes need `settings.edit`.

use nb_domain::access::{Permission, Target};
use nb_domain::features::{Feature, FeatureSet, Profiles};
use nb_domain::{BranchId, RegisterId, RuleError, WarehouseId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::repo::access as access_repo;
use crate::repo::org::{self, Branch, Warehouse};
use crate::repo::parse_id;
use crate::uow::{OpMeta, Writer};
use crate::{Error, Result, Store};

/// Settings of the organization (the setup wizard's answers, editable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgSettings {
    pub name: String,
    pub activity: String,
    /// Default language of documents and new devices (`ar`, `fr`, `en`).
    pub language: String,
    /// First month of the fiscal year (1–12).
    pub fiscal_year_start: u8,
    /// Minutes of inactivity before the screen locks (0 = never, ≤ 240).
    pub auto_lock_min: u32,
}

/// Legal identity printed on documents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessProfile {
    pub legal_name: String,
    pub nif: String,
    pub nis: String,
    pub rc: String,
    pub ai: String,
    pub address: String,
    pub phone: String,
    pub email: String,
}

pub const LANGUAGES: [&str; 3] = ["ar", "fr", "en"];

fn the_org(conn: &Connection) -> Result<nb_core::OrganizationId> {
    org::the_org(conn)?
        .ok_or(Error::NotFound("organization"))?
        .org_id()
}

pub fn settings(store: &Store) -> Result<OrgSettings> {
    store.read(|r| {
        let id = the_org(r.conn())?;
        Ok(r.conn().query_row(
            "SELECT name, activity, language, fiscal_year_start, auto_lock_min FROM organizations WHERE id = ?1",
            [id.to_string()],
            |row| {
                Ok(OrgSettings {
                    name: row.get(0)?,
                    activity: row.get(1)?,
                    language: row.get(2)?,
                    fiscal_year_start: row.get(3)?,
                    auto_lock_min: row.get(4)?,
                })
            },
        )?)
    })
}

/// Checks settings (shared with the setup wizard).
pub fn check_settings(s: &OrgSettings) -> std::result::Result<(), Vec<RuleError>> {
    let mut errors = Vec::new();
    if s.name.trim().is_empty() {
        errors.push(RuleError::on("name", "required"));
    }
    if !Profiles::builtin().has(&s.activity) {
        errors.push(RuleError::on("activity", "invalid_value"));
    }
    if !LANGUAGES.contains(&s.language.as_str()) {
        errors.push(RuleError::on("language", "invalid_value"));
    }
    if !(1..=12).contains(&s.fiscal_year_start) {
        errors.push(RuleError::on("fiscal_year_start", "invalid_value"));
    }
    if s.auto_lock_min > 240 {
        errors.push(RuleError::on("auto_lock_min", "invalid_value"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn update_settings(store: &mut Store, meta: OpMeta, s: OrgSettings) -> Result<OrgSettings> {
    check_settings(&s)?;
    let before = settings(store)?;
    store.write(meta, |w| {
        let id = w.org()?;
        w.require(Permission::SettingsEdit, &Target::org())?;
        // The fiscal calendar moves only while nothing is posted: its years
        // are then made again on the new start month.
        if s.fiscal_year_start != before.fiscal_year_start {
            let posted: i64 = w.tx().query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE org_id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )?;
            if posted > 0 {
                return Err(RuleError::on("fiscal_year_start", "fiscal.start_in_use").into());
            }
            w.tx().execute("DELETE FROM fiscal_periods WHERE org_id = ?1", [id.to_string()])?;
            w.tx().execute("DELETE FROM fiscal_years WHERE org_id = ?1", [id.to_string()])?;
        }
        w.tx().execute(
            "UPDATE organizations SET name = ?2, activity = ?3, language = ?4, fiscal_year_start = ?5,
                auto_lock_min = ?6, updated_at = ?7, version = version + 1 WHERE id = ?1",
            params![
                id.to_string(),
                s.name.trim(),
                s.activity,
                s.language,
                s.fiscal_year_start,
                s.auto_lock_min,
                w.now()
            ],
        )?;
        w.event("org.updated", "organization", &id.to_string(), &s)?;
        w.audit(
            "update",
            "organization",
            &id.to_string(),
            &serde_json::json!({ "before": before, "after": s }),
        )?;
        Ok(s)
    })
}

pub fn business_profile(store: &Store) -> Result<BusinessProfile> {
    store.read(|r| {
        let id = the_org(r.conn())?;
        Ok(r.conn()
            .query_row(
                "SELECT legal_name, nif, nis, rc, ai, address, phone, email FROM business_profiles WHERE org_id = ?1",
                [id.to_string()],
                |row| {
                    Ok(BusinessProfile {
                        legal_name: row.get(0)?,
                        nif: row.get(1)?,
                        nis: row.get(2)?,
                        rc: row.get(3)?,
                        ai: row.get(4)?,
                        address: row.get(5)?,
                        phone: row.get(6)?,
                        email: row.get(7)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    })
}

/// Algerian tax identifiers: NIF 15 digits (20 for some), NIS 15 digits;
/// empty is allowed until documents need them.
fn check_profile(p: &BusinessProfile) -> std::result::Result<(), Vec<RuleError>> {
    let digits = |s: &str, lens: &[usize]| {
        s.is_empty() || (s.chars().all(|c| c.is_ascii_digit()) && lens.contains(&s.len()))
    };
    let mut errors = Vec::new();
    if !digits(p.nif.trim(), &[15, 20]) {
        errors.push(RuleError::on("nif", "org.nif_format"));
    }
    if !digits(p.nis.trim(), &[15]) {
        errors.push(RuleError::on("nis", "org.nis_format"));
    }
    let email = p.email.trim();
    if !email.is_empty()
        && !email
            .split_once('@')
            .is_some_and(|(a, b)| !a.is_empty() && b.contains('.'))
    {
        errors.push(RuleError::on("email", "org.email_format"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn update_business_profile(
    store: &mut Store,
    meta: OpMeta,
    p: BusinessProfile,
) -> Result<BusinessProfile> {
    let p = BusinessProfile {
        legal_name: p.legal_name.trim().into(),
        nif: p.nif.trim().into(),
        nis: p.nis.trim().into(),
        rc: p.rc.trim().into(),
        ai: p.ai.trim().into(),
        address: p.address.trim().into(),
        phone: p.phone.trim().into(),
        email: p.email.trim().into(),
    };
    check_profile(&p)?;
    store.write(meta, |w| {
        let id = w.org()?;
        w.require(Permission::SettingsEdit, &Target::org())?;
        w.tx().execute(
            "INSERT INTO business_profiles (org_id, legal_name, nif, nis, rc, ai, address, phone, email, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (org_id) DO UPDATE SET legal_name = ?2, nif = ?3, nis = ?4, rc = ?5, ai = ?6,
                address = ?7, phone = ?8, email = ?9, updated_at = ?10, version = version + 1",
            params![id.to_string(), p.legal_name, p.nif, p.nis, p.rc, p.ai, p.address, p.phone, p.email, w.now()],
        )?;
        w.event("org.profile_updated", "organization", &id.to_string(), &p)?;
        w.audit("update_profile", "organization", &id.to_string(), &p)?;
        Ok(p)
    })
}

/// The features on for this organization (activity profile + switches).
pub fn features(store: &Store) -> Result<FeatureSet> {
    store.read(|r| {
        let Some(o) = org::the_org(r.conn())? else {
            return Ok(FeatureSet::default());
        };
        let overrides = access_repo::feature_overrides(r.conn(), o.org_id()?)?;
        Ok(Profiles::builtin().effective(&o.activity, &overrides))
    })
}

/// Switches a feature on or off, or back to the profile's choice (`None`).
/// Core features cannot be switched off.
pub fn set_feature(
    store: &mut Store,
    meta: OpMeta,
    code: &str,
    on: Option<bool>,
) -> Result<FeatureSet> {
    let f =
        Feature::from_code(code).ok_or(Error::from(RuleError::on("feature", "invalid_value")))?;
    if on == Some(false) && Profiles::builtin().is_core(f) {
        return Err(RuleError::on("feature", "features.core").into());
    }
    store.write(meta, |w| {
        let id = w.org()?;
        w.require(Permission::SettingsEdit, &Target::org())?;
        access_repo::set_feature(w, id, f, on)?;
        w.event("org.feature", "organization", &id.to_string(), &(code, on))?;
        w.audit("feature", "organization", &id.to_string(), &(code, on))?;
        Ok(())
    })?;
    features(store)
}

// ---------------------------------------------------------------------------
// Places

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Register {
    pub id: String,
    pub branch_id: String,
    pub code: String,
    pub name: String,
}

/// Branches with their warehouses and registers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchTree {
    pub branch: Branch,
    pub warehouses: Vec<Warehouse>,
    pub registers: Vec<Register>,
}

pub fn registers(conn: &Connection, org_id: nb_core::OrganizationId) -> Result<Vec<Register>> {
    let mut s = conn.prepare(
        "SELECT id, branch_id, code, name FROM cash_registers WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY code",
    )?;
    let rows = s.query_map([org_id.to_string()], |r| {
        Ok(Register {
            id: r.get(0)?,
            branch_id: r.get(1)?,
            code: r.get(2)?,
            name: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn places(store: &Store) -> Result<Vec<BranchTree>> {
    store.read(|r| {
        let id = the_org(r.conn())?;
        let warehouses = org::warehouses(r.conn(), id)?;
        let registers = registers(r.conn(), id)?;
        Ok(org::branches(r.conn(), id)?
            .into_iter()
            .map(|b| BranchTree {
                warehouses: warehouses
                    .iter()
                    .filter(|w| w.branch_id == b.id)
                    .cloned()
                    .collect(),
                registers: registers
                    .iter()
                    .filter(|x| x.branch_id == b.id)
                    .cloned()
                    .collect(),
                branch: b,
            })
            .collect())
    })
}

fn next_code(w: &Writer<'_>, sql: &str, scope: &str) -> Result<String> {
    let n: i64 = w.tx().query_row(sql, [scope], |r| r.get(0))?;
    Ok(format!("{:02}", n + 1))
}

fn insert_register(
    w: &Writer<'_>,
    org_id: nb_core::OrganizationId,
    branch: &str,
    name: &str,
) -> Result<Register> {
    let code = next_code(
        w,
        "SELECT COUNT(*) FROM cash_registers WHERE branch_id = ?1",
        branch,
    )?;
    let r = Register {
        id: RegisterId::new().to_string(),
        branch_id: branch.into(),
        code,
        name: name.trim().into(),
    };
    w.tx().execute(
        "INSERT INTO cash_registers (id, org_id, branch_id, code, name, created_at, updated_at, created_by, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
        params![r.id, org_id.to_string(), r.branch_id, r.code, r.name, w.now(), w.actor_text(), w.device().id.to_string()],
    )?;
    Ok(r)
}

/// The first cash register of a new branch (setup, new branches).
pub(crate) fn first_register(
    w: &Writer<'_>,
    org_id: nb_core::OrganizationId,
    branch: &str,
    name: &str,
) -> Result<Register> {
    insert_register(w, org_id, branch, name)
}

fn required(field: &'static str, v: &str) -> Result<()> {
    if v.trim().is_empty() {
        Err(RuleError::on(field, "required").into())
    } else {
        Ok(())
    }
}

/// A new branch, with its first warehouse and cash register.
pub fn create_branch(store: &mut Store, meta: OpMeta, name: &str) -> Result<BranchTree> {
    required("name", name)?;
    store.write(meta, |w| {
        let id = w.org()?;
        w.require(Permission::SettingsEdit, &Target::org())?;
        let code = next_code(
            w,
            "SELECT COUNT(*) FROM branches WHERE org_id = ?1",
            &id.to_string(),
        )?;
        let branch = Branch {
            id: BranchId::new().to_string(),
            code,
            name: name.trim().into(),
        };
        org::insert_branch(w, id, &branch)?;
        let wh = Warehouse {
            id: WarehouseId::new().to_string(),
            branch_id: branch.id.clone(),
            name: name.trim().into(),
            negative_stock: "permission".into(),
        };
        org::insert_warehouse(w, id, &wh)?;
        let reg = first_register(w, id, &branch.id, "01")?;
        let tree = BranchTree {
            branch,
            warehouses: vec![wh],
            registers: vec![reg],
        };
        w.event("branch.created", "branch", &tree.branch.id, &tree)?;
        w.audit("create", "branch", &tree.branch.id, &tree)?;
        Ok(tree)
    })
}

pub fn rename_branch(store: &mut Store, meta: OpMeta, branch: BranchId, name: &str) -> Result<()> {
    required("name", name)?;
    store.write(meta, |w| {
        let id = w.org()?;
        w.require(Permission::SettingsEdit, &Target::org())?;
        let n = w.tx().execute(
            "UPDATE branches SET name = ?3, updated_at = ?4, version = version + 1 WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
            params![branch.to_string(), id.to_string(), name.trim(), w.now()],
        )?;
        if n == 0 {
            return Err(Error::NotFound("branch"));
        }
        w.event("branch.renamed", "branch", &branch.to_string(), &name.trim())?;
        w.audit("rename", "branch", &branch.to_string(), &name.trim())?;
        Ok(())
    })
}

pub fn create_warehouse(
    store: &mut Store,
    meta: OpMeta,
    branch: BranchId,
    name: &str,
    negative_stock: &str,
) -> Result<Warehouse> {
    required("name", name)?;
    if !["forbid", "permission", "allow"].contains(&negative_stock) {
        return Err(RuleError::on("negative_stock", "invalid_value").into());
    }
    store.write(meta, |w| {
        let id = w.org()?;
        w.require(Permission::SettingsEdit, &Target::org())?;
        let b = org::branch(w.tx(), branch)?;
        let wh = Warehouse {
            id: WarehouseId::new().to_string(),
            branch_id: b.id,
            name: name.trim().into(),
            negative_stock: negative_stock.into(),
        };
        org::insert_warehouse(w, id, &wh)?;
        w.event("warehouse.created", "warehouse", &wh.id, &wh)?;
        w.audit("create", "warehouse", &wh.id, &wh)?;
        Ok(wh)
    })
}

/// Changes a warehouse's name and negative-stock policy.
pub fn update_warehouse(
    store: &mut Store,
    meta: OpMeta,
    warehouse: WarehouseId,
    name: &str,
    negative_stock: &str,
) -> Result<Warehouse> {
    required("name", name)?;
    if !["forbid", "permission", "allow"].contains(&negative_stock) {
        return Err(RuleError::on("negative_stock", "invalid_value").into());
    }
    store.write(meta, |w| {
        w.require(Permission::SettingsEdit, &Target::org())?;
        let before = org::warehouse(w.tx(), warehouse)?;
        w.tx().execute(
            "UPDATE warehouses SET name = ?2, negative_stock = ?3, updated_at = ?4, version = version + 1 WHERE id = ?1",
            params![warehouse.to_string(), name.trim(), negative_stock, w.now()],
        )?;
        let after = org::warehouse(w.tx(), warehouse)?;
        w.event("warehouse.updated", "warehouse", &after.id, &after)?;
        w.audit(
            "update",
            "warehouse",
            &after.id,
            &serde_json::json!({ "before": before, "after": after }),
        )?;
        Ok(after)
    })
}

/// A warehouse as its page shows it: where, who, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WarehouseRow {
    pub id: String,
    pub name: String,
    pub branch_id: String,
    pub branch: String,
    pub negative_stock: String,
    pub address: String,
    pub phone: String,
    pub keeper: String,
    pub note: String,
    /// Products with something on hand.
    pub products: u32,
    /// Products at or under their minimum (or none left).
    pub low: u32,
    /// On hand at its average cost.
    pub value_minor: i64,
    /// Held for orders.
    pub reserved_milli: i64,
    /// Stock movements in or out over the last 30 days.
    pub moves_30d: u32,
}

/// The warehouses with what each holds.
pub fn warehouse_overview(store: &Store) -> Result<Vec<WarehouseRow>> {
    let since = store.now_ms() - 30 * 86_400_000;
    store.read(|r| {
        let conn = r.conn();
        let Some(o) = org::the_org(conn)? else {
            return Ok(Vec::new());
        };
        let mut s = conn.prepare(
            "SELECT w.id, w.name, w.branch_id, b.name, w.negative_stock, w.address, w.phone, w.keeper, w.note,
                    (SELECT COUNT(*) FROM stock_balances x WHERE x.warehouse_id = w.id AND x.on_hand_milli > 0),
                    (SELECT COUNT(*) FROM stock_balances x JOIN products p ON p.id = x.product_id
                      WHERE x.warehouse_id = w.id AND p.kind <> 'service' AND p.deleted_at IS NULL
                        AND (x.on_hand_milli <= 0 OR (p.min_stock_milli IS NOT NULL AND x.on_hand_milli <= p.min_stock_milli))),
                    (SELECT COALESCE(SUM((x.on_hand_milli * x.avg_cost_minor + 500) / 1000), 0)
                       FROM stock_balances x WHERE x.warehouse_id = w.id AND x.on_hand_milli > 0),
                    (SELECT COALESCE(SUM(x.qty_milli), 0) FROM stock_reservations x WHERE x.warehouse_id = w.id AND x.status = 'active'),
                    (SELECT COUNT(*) FROM stock_movements m WHERE m.warehouse_id = w.id AND m.created_at >= ?2)
             FROM warehouses w JOIN branches b ON b.id = w.branch_id
             WHERE w.org_id = ?1 AND w.deleted_at IS NULL ORDER BY w.created_at",
        )?;
        let rows = s
            .query_map(params![o.org_id()?.to_string(), since], |r| {
                Ok(WarehouseRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    branch_id: r.get(2)?,
                    branch: r.get(3)?,
                    negative_stock: r.get(4)?,
                    address: r.get(5)?,
                    phone: r.get(6)?,
                    keeper: r.get(7)?,
                    note: r.get(8)?,
                    products: r.get(9)?,
                    low: r.get(10)?,
                    value_minor: r.get(11)?,
                    reserved_milli: r.get(12)?,
                    moves_30d: r.get(13)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    })
}

/// Where a warehouse is and who keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WarehouseDetails {
    pub address: String,
    pub phone: String,
    pub keeper: String,
    pub note: String,
}

pub fn set_warehouse_details(
    store: &mut Store,
    meta: OpMeta,
    warehouse: WarehouseId,
    d: &WarehouseDetails,
) -> Result<()> {
    store.write(meta, |w| {
        w.require(Permission::SettingsEdit, &Target::org())?;
        org::warehouse(w.tx(), warehouse)?;
        w.tx().execute(
            "UPDATE warehouses SET address = ?2, phone = ?3, keeper = ?4, note = ?5,
                    updated_at = ?6, version = version + 1 WHERE id = ?1",
            params![
                warehouse.to_string(),
                d.address.trim(),
                d.phone.trim(),
                d.keeper.trim(),
                d.note.trim(),
                w.now()
            ],
        )?;
        w.event("warehouse.details", "warehouse", &warehouse.to_string(), d)?;
        w.audit("update", "warehouse", &warehouse.to_string(), d)?;
        Ok(())
    })
}

/// Retires a warehouse: only one left empty (nothing on hand, nothing
/// held), and never the last one. Its history stays.
pub fn archive_warehouse(store: &mut Store, meta: OpMeta, warehouse: WarehouseId) -> Result<()> {
    store.write(meta, |w| {
        let id = w.org()?;
        w.require(Permission::SettingsEdit, &Target::org())?;
        org::warehouse(w.tx(), warehouse)?;
        let left = org::warehouses(w.tx(), id)?.len();
        if left <= 1 {
            return Err(RuleError::on("warehouse", "warehouse.last").into());
        }
        let holds: bool = w.tx().query_row(
            "SELECT EXISTS (SELECT 1 FROM stock_balances WHERE warehouse_id = ?1 AND on_hand_milli <> 0)
                 OR EXISTS (SELECT 1 FROM stock_reservations WHERE warehouse_id = ?1 AND status = 'active')",
            [warehouse.to_string()],
            |r| r.get(0),
        )?;
        if holds {
            return Err(RuleError::on("warehouse", "warehouse.not_empty").into());
        }
        w.tx().execute(
            "UPDATE warehouses SET deleted_at = ?2, updated_at = ?2, version = version + 1 WHERE id = ?1",
            params![warehouse.to_string(), w.now()],
        )?;
        w.event("warehouse.archived", "warehouse", &warehouse.to_string(), &())?;
        w.audit("archive", "warehouse", &warehouse.to_string(), &())?;
        Ok(())
    })
}

pub fn create_register(
    store: &mut Store,
    meta: OpMeta,
    branch: BranchId,
    name: &str,
) -> Result<Register> {
    required("name", name)?;
    store.write(meta, |w| {
        let id = w.org()?;
        w.require(Permission::SettingsEdit, &Target::org())?;
        let b = org::branch(w.tx(), branch)?;
        let r = insert_register(w, id, &b.id, name)?;
        w.event("register.created", "register", &r.id, &r)?;
        w.audit("create", "register", &r.id, &r)?;
        Ok(r)
    })
}

/// Retires a cash register (its history stays).
pub fn archive_register(store: &mut Store, meta: OpMeta, register: RegisterId) -> Result<()> {
    store.write(meta, |w| {
        let id = w.org()?;
        w.require(Permission::SettingsEdit, &Target::org())?;
        let n = w.tx().execute(
            "UPDATE cash_registers SET deleted_at = ?3, updated_at = ?3, version = version + 1
             WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
            params![register.to_string(), id.to_string(), w.now()],
        )?;
        if n == 0 {
            return Err(Error::NotFound("register"));
        }
        w.event("register.archived", "register", &register.to_string(), &())?;
        w.audit("archive", "register", &register.to_string(), &())?;
        Ok(())
    })
}

/// The branch a register belongs to (to build a request's target).
pub fn register_branch(conn: &Connection, register: RegisterId) -> Result<BranchId> {
    let b: String = conn
        .query_row(
            "SELECT branch_id FROM cash_registers WHERE id = ?1",
            [register.to_string()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound("register"))?;
    parse_id(&b)
}

/// How stock issues are costed (M6.10): `average` or `fifo`.
pub fn valuation(store: &Store) -> Result<String> {
    store.read(|r| Ok(crate::repo::stock::method(r.conn())?.code().to_owned()))
}

/// Changes the valuation method: what goes out from now on is costed by
/// it. The FIFO layers are always kept, so either works at once.
pub fn set_valuation(store: &mut Store, meta: OpMeta, code: &str) -> Result<String> {
    let m = nb_domain::valuation::Method::from_code(code)?;
    store.write(meta, |w| {
        let id = the_org(w.tx())?;
        w.require(
            nb_domain::access::Permission::SettingsEdit,
            &nb_domain::access::Target::org(),
        )?;
        w.tx().execute(
            "UPDATE organizations SET valuation = ?2 WHERE id = ?1",
            params![id.to_string(), m.code()],
        )?;
        w.event(
            "org.valuation_changed",
            "organization",
            &id.to_string(),
            &m.code(),
        )?;
        w.audit("set_valuation", "organization", &id.to_string(), &m.code())?;
        Ok(m.code().to_owned())
    })
}
