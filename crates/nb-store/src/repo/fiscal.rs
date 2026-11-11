//! Fiscal years and periods (M7.2).

use nb_core::OrganizationId;
use nb_domain::fiscal::{Date, year_of};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::uow::Writer;

/// The period an entry date falls in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodRef {
    pub id: String,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodRow {
    pub id: String,
    pub number: u32,
    pub start: String,
    pub end: String,
    pub closed: bool,
    pub closed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YearRow {
    pub id: String,
    pub name: String,
    pub start: String,
    pub end: String,
    pub periods: Vec<PeriodRow>,
}

fn find(conn: &Connection, org: OrganizationId, date: &str) -> Result<Option<PeriodRef>> {
    Ok(conn
        .query_row(
            "SELECT id, status FROM fiscal_periods WHERE org_id = ?1 AND ?2 BETWEEN start_date AND end_date",
            params![org.to_string(), date],
            |r| {
                Ok(PeriodRef {
                    id: r.get(0)?,
                    closed: r.get::<_, String>(1)? == "closed",
                })
            },
        )
        .optional()?)
}

/// The period of `date`, making its fiscal year (and twelve periods) if it
/// has none yet.
pub fn ensure_period(w: &Writer<'_>, org: OrganizationId, date: &str) -> Result<PeriodRef> {
    if let Some(p) = find(w.tx(), org, date)? {
        return Ok(p);
    }
    let start_month: u32 = w.tx().query_row(
        "SELECT fiscal_year_start FROM organizations WHERE id = ?1",
        [org.to_string()],
        |r| r.get(0),
    )?;
    let year = year_of(Date::parse(date)?, start_month)?;
    let year_id = nb_core::uuid::Uuid::now_v7().to_string();
    w.tx().execute(
        "INSERT INTO fiscal_years (id, org_id, name, start_date, end_date, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![year_id, org.to_string(), year.name, year.start.iso(), year.end.iso(), w.now()],
    )?;
    for p in &year.periods {
        w.tx().execute(
            "INSERT INTO fiscal_periods (id, org_id, year_id, number, start_date, end_date) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                nb_core::uuid::Uuid::now_v7().to_string(),
                org.to_string(),
                year_id,
                p.number,
                p.start.iso(),
                p.end.iso()
            ],
        )?;
    }
    find(w.tx(), org, date)?.ok_or(crate::Error::NotFound("fiscal period"))
}

/// Every fiscal year, latest first, with its periods.
pub fn years(conn: &Connection, org: OrganizationId) -> Result<Vec<YearRow>> {
    let mut s = conn.prepare(
        "SELECT id, name, start_date, end_date FROM fiscal_years WHERE org_id = ?1 ORDER BY start_date DESC",
    )?;
    let mut years: Vec<YearRow> = s
        .query_map([org.to_string()], |r| {
            Ok(YearRow {
                id: r.get(0)?,
                name: r.get(1)?,
                start: r.get(2)?,
                end: r.get(3)?,
                periods: vec![],
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut p = conn.prepare(
        "SELECT id, number, start_date, end_date, status, closed_at FROM fiscal_periods WHERE year_id = ?1 ORDER BY number",
    )?;
    for y in &mut years {
        y.periods = p
            .query_map([&y.id], |r| {
                Ok(PeriodRow {
                    id: r.get(0)?,
                    number: r.get(1)?,
                    start: r.get(2)?,
                    end: r.get(3)?,
                    closed: r.get::<_, String>(4)? == "closed",
                    closed_at: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
    }
    Ok(years)
}

/// The year of a period, and whether each of its periods is closed.
pub fn year_states(conn: &Connection, period_id: &str) -> Result<Option<(String, u32, Vec<bool>)>> {
    let Some((year, number)): Option<(String, u32)> = conn
        .query_row(
            "SELECT year_id, number FROM fiscal_periods WHERE id = ?1",
            [period_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut s =
        conn.prepare("SELECT status FROM fiscal_periods WHERE year_id = ?1 ORDER BY number")?;
    let states = s
        .query_map([&year], |r| Ok(r.get::<_, String>(0)? == "closed"))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(Some((year, number, states)))
}

pub fn set_closed(w: &Writer<'_>, period_id: &str, closed: bool) -> Result<()> {
    if closed {
        w.tx().execute(
            "UPDATE fiscal_periods SET status = 'closed', closed_at = ?2, closed_by = ?3 WHERE id = ?1",
            params![period_id, w.now(), w.actor_text()],
        )?;
    } else {
        w.tx().execute(
            "UPDATE fiscal_periods SET status = 'open', closed_at = NULL, closed_by = NULL WHERE id = ?1",
            [period_id],
        )?;
    }
    Ok(())
}
