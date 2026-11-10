//! Organizations, branches, warehouses and users.

use nb_core::{Currency, OrganizationId};
use nb_domain::stock::NegativeStock;
use nb_domain::{BranchId, UserId, WarehouseId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::parse_id;
use crate::uow::Writer;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub country: String,
    pub currency: String,
    pub currency_exp: u8,
    pub activity: String,
}

impl Organization {
    pub fn org_id(&self) -> Result<OrganizationId> {
        parse_id(&self.id)
    }
    pub fn currency(&self) -> Result<Currency> {
        Currency::new(&self.currency, self.currency_exp).map_err(Error::from)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    pub id: String,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warehouse {
    pub id: String,
    pub branch_id: String,
    pub name: String,
    pub negative_stock: String,
}

impl Warehouse {
    pub fn policy(&self) -> NegativeStock {
        match self.negative_stock.as_str() {
            "forbid" => NegativeStock::Forbid,
            "allow" => NegativeStock::Allow,
            _ => NegativeStock::AllowWithPermission,
        }
    }
}

pub fn insert_org(w: &Writer<'_>, o: &Organization) -> Result<()> {
    w.tx().execute(
        "INSERT INTO organizations (id, name, country, currency, currency_exp, activity, created_at, updated_at, created_by, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?9)",
        params![o.id, o.name, o.country, o.currency, o.currency_exp, o.activity, w.now(), w.actor_text(), w.device().id.to_string()],
    )?;
    Ok(())
}

pub fn insert_branch(w: &Writer<'_>, org: OrganizationId, b: &Branch) -> Result<()> {
    w.tx().execute(
        "INSERT INTO branches (id, org_id, code, name, created_at, updated_at, created_by, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
        params![b.id, org.to_string(), b.code, b.name, w.now(), w.actor_text(), w.device().id.to_string()],
    )?;
    Ok(())
}

pub fn insert_warehouse(w: &Writer<'_>, org: OrganizationId, wh: &Warehouse) -> Result<()> {
    w.tx().execute(
        "INSERT INTO warehouses (id, org_id, branch_id, name, negative_stock, created_at, updated_at, created_by, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
        params![wh.id, org.to_string(), wh.branch_id, wh.name, wh.negative_stock, w.now(), w.actor_text(), w.device().id.to_string()],
    )?;
    Ok(())
}

pub fn insert_user(
    w: &Writer<'_>,
    org: OrganizationId,
    id: UserId,
    name: &str,
    role: &str,
) -> Result<()> {
    w.tx().execute(
        "INSERT INTO users (id, org_id, name, role, created_at, updated_at, created_by, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
        params![
            id.to_string(),
            org.to_string(),
            name,
            role,
            w.now(),
            w.actor_text(),
            w.device().id.to_string()
        ],
    )?;
    Ok(())
}

/// The organization of this database (one per file).
pub fn the_org(conn: &Connection) -> Result<Option<Organization>> {
    Ok(conn
        .query_row(
            "SELECT id, name, country, currency, currency_exp, activity FROM organizations WHERE deleted_at IS NULL ORDER BY created_at LIMIT 1",
            [],
            |r| {
                Ok(Organization {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    country: r.get(2)?,
                    currency: r.get(3)?,
                    currency_exp: r.get(4)?,
                    activity: r.get(5)?,
                })
            },
        )
        .optional()?)
}

pub fn branches(conn: &Connection, org: OrganizationId) -> Result<Vec<Branch>> {
    let mut s = conn.prepare(
        "SELECT id, code, name FROM branches WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY code",
    )?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok(Branch {
            id: r.get(0)?,
            code: r.get(1)?,
            name: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn branch(conn: &Connection, id: BranchId) -> Result<Branch> {
    conn.query_row(
        "SELECT id, code, name FROM branches WHERE id = ?1",
        [id.to_string()],
        |r| {
            Ok(Branch {
                id: r.get(0)?,
                code: r.get(1)?,
                name: r.get(2)?,
            })
        },
    )
    .optional()?
    .ok_or(Error::NotFound("branch"))
}

pub fn warehouses(conn: &Connection, org: OrganizationId) -> Result<Vec<Warehouse>> {
    let mut s = conn.prepare(
        "SELECT id, branch_id, name, negative_stock FROM warehouses WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY created_at",
    )?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok(Warehouse {
            id: r.get(0)?,
            branch_id: r.get(1)?,
            name: r.get(2)?,
            negative_stock: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn warehouse(conn: &Connection, id: WarehouseId) -> Result<Warehouse> {
    conn.query_row(
        "SELECT id, branch_id, name, negative_stock FROM warehouses WHERE id = ?1 AND deleted_at IS NULL",
        [id.to_string()],
        |r| {
            Ok(Warehouse {
                id: r.get(0)?,
                branch_id: r.get(1)?,
                name: r.get(2)?,
                negative_stock: r.get(3)?,
            })
        },
    )
    .optional()?
    .ok_or(Error::NotFound("warehouse"))
}
