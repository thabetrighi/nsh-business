//! Distribution and field sales (M16): zones and customers on the map,
//! vehicles that carry their own stock (a warehouse each), tours of stops
//! (deliveries of approved orders, visits, van sales), where the vehicles
//! are, and the board that watches the day: what is on the road, what comes
//! next and when, and what needs someone's attention.
//!
//! Who does what: planning (zones, tours, stops) needs
//! `distribution.plan`; the driver works their own tour with
//! `distribution.drive` (a planner may work any tour); vehicles need
//! `distribution.fleet`; counting the cash a tour brought back needs
//! `distribution.settle`. Money collected at a stop is a customer receipt
//! (`debts.collect`) written in the same transaction as the stop.

use nb_domain::RuleError;
use nb_domain::access::{Permission, Target};
use nb_domain::geo::{self, Point};
use nb_domain::numbering::DocKind;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::repo::org::{self, Warehouse};
use crate::uow::{OpMeta, Writer};
use crate::usecases::debts::{ReceiptInput, collect_in};
use crate::usecases::sales::the_org;
use crate::{Error, Result, Store};

/// A vehicle within this distance of its next stop is "near" (the moment
/// to tell the customer).
pub const NEAR_M: u64 = 1_000;
/// No position for this long while on the road: the vehicle is "silent".
pub const SILENT_MS: i64 = 15 * 60_000;
/// Minutes spent at each stop when estimating arrival times.
pub const STOP_MIN: u32 = 5;

fn new_id() -> String {
    nb_core::uuid::Uuid::now_v7().to_string()
}

fn rule(field: &'static str, code: &'static str) -> Error {
    RuleError::on(field, code).into()
}

fn collect<T>(
    rows: impl Iterator<Item = rusqlite::Result<T>>,
) -> std::result::Result<Vec<T>, rusqlite::Error> {
    rows.collect()
}

// ---------------------------------------------------------------------------
// Zones

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Zone {
    pub id: String,
    pub name: String,
    /// A palette index the app colours the zone with.
    pub color: i64,
    pub note: String,
    pub customers: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ZoneInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub color: i64,
    #[serde(default)]
    pub note: String,
}

fn zones_in(conn: &Connection, org: &str) -> Result<Vec<Zone>> {
    let mut s = conn.prepare(
        "SELECT z.id, z.name, z.color, z.note,
                (SELECT COUNT(*) FROM customers c WHERE c.zone_id = z.id)
         FROM zones z WHERE z.org_id = ?1 AND z.deleted_at IS NULL ORDER BY z.name COLLATE NOCASE",
    )?;
    let rows = collect(s.query_map([org], |r| {
        Ok(Zone {
            id: r.get(0)?,
            name: r.get(1)?,
            color: r.get(2)?,
            note: r.get(3)?,
            customers: r.get(4)?,
        })
    })?)?;
    Ok(rows)
}

pub fn list_zones(store: &Store) -> Result<Vec<Zone>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        zones_in(r.conn(), &org.to_string())
    })
}

/// Creates a zone or renames one.
pub fn save_zone(store: &mut Store, meta: OpMeta, input: ZoneInput) -> Result<Zone> {
    let name = input.name.trim().to_owned();
    if name.is_empty() {
        return Err(rule("name", "required"));
    }
    store.write(meta, |w| {
        w.require(Permission::DistributionPlan, &Target::org())?;
        let org = w.org()?.to_string();
        let id = input
            .id
            .clone()
            .filter(|i| !i.is_empty())
            .unwrap_or_default();
        let taken: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM zones WHERE org_id = ?1 AND lower(name) = lower(?2)
                 AND deleted_at IS NULL AND id <> ?3",
            params![org, name, id],
            |r| r.get(0),
        )?;
        if taken > 0 {
            return Err(Error::Duplicate("name"));
        }
        let id = if id.is_empty() {
            let id = new_id();
            w.tx().execute(
                "INSERT INTO zones (id, org_id, name, color, note, created_at, updated_at, created_by, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
                params![
                    id,
                    org,
                    name,
                    input.color,
                    input.note.trim(),
                    w.now(),
                    w.actor_text(),
                    w.device().id.to_string()
                ],
            )?;
            w.audit("create", "zone", &id, &input)?;
            id
        } else {
            let n = w.tx().execute(
                "UPDATE zones SET name = ?3, color = ?4, note = ?5, updated_at = ?6, version = version + 1
                 WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
                params![id, org, name, input.color, input.note.trim(), w.now()],
            )?;
            if n == 0 {
                return Err(rule("zone", "invalid_value"));
            }
            w.audit("update", "zone", &id, &input)?;
            id
        };
        let z = zones_in(w.tx(), &org)?
            .into_iter()
            .find(|z| z.id == id)
            .ok_or(Error::NotFound("zone"))?;
        w.event("zone.saved", "zone", &id, &z)?;
        Ok(z)
    })
}

/// Removes a zone; its customers keep their place and lose the zone.
pub fn archive_zone(store: &mut Store, meta: OpMeta, id: &str) -> Result<()> {
    store.write(meta, |w| {
        w.require(Permission::DistributionPlan, &Target::org())?;
        let n = w.tx().execute(
            "UPDATE zones SET deleted_at = ?3, updated_at = ?3, version = version + 1
             WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
            params![id, w.org()?.to_string(), w.now()],
        )?;
        if n == 0 {
            return Err(rule("zone", "invalid_value"));
        }
        w.tx().execute(
            "UPDATE customers SET zone_id = NULL, updated_at = ?2, version = version + 1 WHERE zone_id = ?1",
            params![id, w.now()],
        )?;
        w.event("zone.archived", "zone", id, &())?;
        w.audit("archive", "zone", id, &())?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Customers on the map

/// Where a customer is and when it is visited.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CustomerPlace {
    #[serde(default)]
    pub zone_id: Option<String>,
    /// Typed or pasted ("36.75, 3.06" or a map link); empty clears it.
    #[serde(default)]
    pub position: String,
    /// Monday = 1, Tuesday = 2, … Sunday = 64.
    #[serde(default)]
    pub visit_days: i64,
    #[serde(default)]
    pub delivery_note: String,
}

/// Sets where a customer is (`distribution.plan` or `customers.edit`).
pub fn set_customer_place(
    store: &mut Store,
    meta: OpMeta,
    customer: &str,
    place: CustomerPlace,
) -> Result<RouteCustomer> {
    let point = match place.position.trim() {
        "" => None,
        t => Some(geo::parse_point(t)?),
    };
    if !(0..=127).contains(&place.visit_days) {
        return Err(rule("visit_days", "invalid_value"));
    }
    store.write(meta, |w| {
        if !w.allowed(Permission::DistributionPlan, &Target::org())? {
            w.require(Permission::CustomersEdit, &Target::org())?;
        }
        let org = w.org()?.to_string();
        let zone = place.zone_id.clone().filter(|z| !z.is_empty());
        if let Some(z) = &zone {
            let ok: i64 = w.tx().query_row(
                "SELECT COUNT(*) FROM zones WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
                params![z, org],
                |r| r.get(0),
            )?;
            if ok == 0 {
                return Err(rule("zone", "invalid_value"));
            }
        }
        let n = w.tx().execute(
            "UPDATE customers SET zone_id = ?3, lat_e6 = ?4, lng_e6 = ?5, visit_days = ?6, delivery_note = ?7,
                    updated_at = ?8, version = version + 1
             WHERE id = ?1 AND org_id = ?2",
            params![
                customer,
                org,
                zone,
                point.map(|p| p.lat_e6),
                point.map(|p| p.lng_e6),
                place.visit_days,
                place.delivery_note.trim(),
                w.now()
            ],
        )?;
        if n == 0 {
            return Err(rule("customer", "customer.unknown"));
        }
        let c = route_customer(w.tx(), customer)?;
        w.event("customer.placed", "customer", customer, &c)?;
        w.audit("place", "customer", customer, &place)?;
        Ok(c)
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteCustomer {
    pub id: String,
    pub code: String,
    pub name: String,
    pub phone: String,
    pub address: String,
    pub city: String,
    pub zone_id: Option<String>,
    pub zone: Option<String>,
    pub point: Option<Point>,
    pub visit_days: i64,
    pub delivery_note: String,
    pub balance_minor: i64,
    /// Approved orders of this customer not yet on a tour.
    pub open_orders: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RouteCustomerQuery {
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub zone_id: Option<String>,
    /// Only those visited on one of these days (0: any).
    #[serde(default)]
    pub days: i64,
    /// Only those with no place on the map.
    #[serde(default)]
    pub unplaced: bool,
    /// Only those with orders waiting.
    #[serde(default)]
    pub with_orders: bool,
}

/// An approved order still to be delivered: not on a tour that is not
/// cancelled (unless its stop there failed or was skipped).
const OPEN_ORDER: &str = "d.kind = 'order' AND d.status = 'approved' AND d.customer_id IS NOT NULL
     AND NOT EXISTS (SELECT 1 FROM tour_stops s JOIN tours t ON t.id = s.tour_id
                     WHERE s.document_id = d.id AND t.status <> 'cancelled'
                       AND s.status IN ('pending', 'arrived', 'done'))";

const ROUTE_SELECT: &str =
    "SELECT c.id, c.code, c.name, c.phone, c.address, c.city, c.zone_id, z.name,
        c.lat_e6, c.lng_e6, c.visit_days, c.delivery_note, c.balance_minor";

fn route_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RouteCustomer> {
    Ok(RouteCustomer {
        id: r.get(0)?,
        code: r.get(1)?,
        name: r.get(2)?,
        phone: r.get(3)?,
        address: r.get(4)?,
        city: r.get(5)?,
        zone_id: r.get(6)?,
        zone: r.get(7)?,
        point: Point::of(r.get(8)?, r.get(9)?),
        visit_days: r.get(10)?,
        delivery_note: r.get(11)?,
        balance_minor: r.get(12)?,
        open_orders: r.get(13)?,
    })
}

fn route_customer(conn: &Connection, id: &str) -> Result<RouteCustomer> {
    conn.query_row(
        &format!(
            "{ROUTE_SELECT}, (SELECT COUNT(*) FROM sales_documents d WHERE d.customer_id = c.id AND {OPEN_ORDER})
             FROM customers c LEFT JOIN zones z ON z.id = c.zone_id WHERE c.id = ?1"
        ),
        [id],
        route_row,
    )
    .optional()?
    .ok_or(Error::Rules(vec![RuleError::on(
        "customer",
        "customer.unknown",
    )]))
}

/// Customers for planning: by zone, visit day, missing place or waiting
/// orders (at most 500).
pub fn route_customers(store: &Store, q: &RouteCustomerQuery) -> Result<Vec<RouteCustomer>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(&format!(
            "SELECT * FROM ({ROUTE_SELECT},
                    (SELECT COUNT(*) FROM sales_documents d WHERE d.customer_id = c.id AND {OPEN_ORDER}) AS waiting
                 FROM customers c LEFT JOIN zones z ON z.id = c.zone_id
                 WHERE c.org_id = ?1 AND c.status = 'active'
                   AND (?2 = '' OR lower(c.name) LIKE '%' || ?2 || '%' OR lower(c.code) LIKE '%' || ?2 || '%'
                        OR c.phone LIKE '%' || ?2 || '%' OR lower(c.city) LIKE '%' || ?2 || '%')
                   AND (?3 = '' OR c.zone_id = ?3)
                   AND (?4 = 0 OR (c.visit_days & ?4) <> 0)
                   AND (?5 = 0 OR c.lat_e6 IS NULL))
             WHERE (?6 = 0 OR waiting > 0)
             ORDER BY 8 COLLATE NOCASE, 3 COLLATE NOCASE LIMIT 500"
        ))?;
        let rows = collect(s.query_map(
            params![
                org.to_string(),
                q.search.trim().to_lowercase(),
                q.zone_id.clone().unwrap_or_default(),
                q.days,
                q.unplaced,
                q.with_orders
            ],
            route_row,
        )?)?;
        Ok(rows)
    })
}

// ---------------------------------------------------------------------------
// Waiting orders

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenOrder {
    pub id: String,
    pub number: String,
    pub customer_id: String,
    pub customer: String,
    pub city: String,
    pub zone_id: Option<String>,
    pub zone: Option<String>,
    pub point: Option<Point>,
    pub total_minor: i64,
    pub created_at: i64,
}

/// Approved orders not yet on a tour, oldest first (in a zone, or all).
pub fn open_orders(store: &Store, zone: Option<&str>) -> Result<Vec<OpenOrder>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(&format!(
            "SELECT d.id, d.number, c.id, c.name, c.city, c.zone_id, z.name, c.lat_e6, c.lng_e6,
                    d.total_minor, d.created_at
             FROM sales_documents d JOIN customers c ON c.id = d.customer_id
             LEFT JOIN zones z ON z.id = c.zone_id
             WHERE d.org_id = ?1 AND {OPEN_ORDER} AND (?2 = '' OR c.zone_id = ?2)
             ORDER BY d.created_at LIMIT 500"
        ))?;
        let rows = collect(s.query_map(
            params![org.to_string(), zone.unwrap_or_default()],
            |r| {
                Ok(OpenOrder {
                    id: r.get(0)?,
                    number: r.get(1)?,
                    customer_id: r.get(2)?,
                    customer: r.get(3)?,
                    city: r.get(4)?,
                    zone_id: r.get(5)?,
                    zone: r.get(6)?,
                    point: Point::of(r.get(7)?, r.get(8)?),
                    total_minor: r.get(9)?,
                    created_at: r.get(10)?,
                })
            },
        )?)?;
        Ok(rows)
    })
}

// ---------------------------------------------------------------------------
// Vehicles

pub const VEHICLE_KINDS: [&str; 5] = ["van", "truck", "car", "motorbike", "bike"];

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VehicleInput {
    #[serde(default)]
    pub id: Option<String>,
    /// The branch it belongs to (a new vehicle; default: the first).
    #[serde(default)]
    pub branch_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub plate: String,
    pub kind: String,
    #[serde(default)]
    pub capacity_kg: i64,
    #[serde(default)]
    pub driver_id: Option<String>,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub speed_kmh: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VehicleRow {
    pub id: String,
    pub branch_id: String,
    /// Its own stock.
    pub warehouse_id: String,
    pub name: String,
    pub plate: String,
    pub kind: String,
    pub capacity_kg: i64,
    pub driver_id: Option<String>,
    pub driver: Option<String>,
    pub phone: String,
    pub speed_kmh: i64,
    pub point: Option<Point>,
    pub seen_at: Option<i64>,
    /// What its stock is worth at cost, and how many products it holds.
    pub stock_value_minor: i64,
    pub products: u32,
    /// The tour it is on, or the next one planned.
    pub tour_id: Option<String>,
    pub tour_number: Option<String>,
    pub tour_status: Option<String>,
}

const VEHICLE_SELECT: &str = "SELECT v.id, v.branch_id, v.warehouse_id, v.name, v.plate, v.kind, v.capacity_kg,
        v.driver_id, u.name, v.phone, v.speed_kmh, v.lat_e6, v.lng_e6, v.seen_at,
        (SELECT COALESCE(SUM((x.on_hand_milli * x.avg_cost_minor + 500) / 1000), 0)
           FROM stock_balances x WHERE x.warehouse_id = v.warehouse_id AND x.on_hand_milli > 0),
        (SELECT COUNT(*) FROM stock_balances x WHERE x.warehouse_id = v.warehouse_id AND x.on_hand_milli > 0),
        t.id, t.number, t.status
     FROM vehicles v LEFT JOIN users u ON u.id = v.driver_id
     LEFT JOIN tours t ON t.id = (SELECT t2.id FROM tours t2 WHERE t2.vehicle_id = v.id
                                     AND t2.status IN ('on_road', 'planned')
                                   ORDER BY t2.status = 'on_road' DESC, t2.day, t2.created_at LIMIT 1)";

fn vehicle_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<VehicleRow> {
    Ok(VehicleRow {
        id: r.get(0)?,
        branch_id: r.get(1)?,
        warehouse_id: r.get(2)?,
        name: r.get(3)?,
        plate: r.get(4)?,
        kind: r.get(5)?,
        capacity_kg: r.get(6)?,
        driver_id: r.get(7)?,
        driver: r.get(8)?,
        phone: r.get(9)?,
        speed_kmh: r.get(10)?,
        point: Point::of(r.get(11)?, r.get(12)?),
        seen_at: r.get(13)?,
        stock_value_minor: r.get(14)?,
        products: r.get(15)?,
        tour_id: r.get(16)?,
        tour_number: r.get(17)?,
        tour_status: r.get(18)?,
    })
}

fn vehicle(conn: &Connection, id: &str) -> Result<VehicleRow> {
    conn.query_row(
        &format!("{VEHICLE_SELECT} WHERE v.id = ?1 AND v.deleted_at IS NULL"),
        [id],
        vehicle_row,
    )
    .optional()?
    .ok_or(Error::Rules(vec![RuleError::on(
        "vehicle",
        "distribution.vehicle_unknown",
    )]))
}

pub fn list_vehicles(store: &Store) -> Result<Vec<VehicleRow>> {
    store.read(|r| {
        let Some(o) = org::the_org(r.conn())? else {
            return Ok(Vec::new());
        };
        let mut s = r.conn().prepare(&format!(
            "{VEHICLE_SELECT} WHERE v.org_id = ?1 AND v.deleted_at IS NULL ORDER BY v.name COLLATE NOCASE"
        ))?;
        let rows = collect(s.query_map([o.org_id()?.to_string()], vehicle_row)?)?;
        Ok(rows)
    })
}

/// The vehicle `user` is driving now on a van-sales tour (on the road),
/// if any: the till sells from its stock. Delivery tours carry orders
/// invoiced from the shop's stock.
pub fn driving_vehicle(store: &Store, user: &str) -> Result<Option<VehicleRow>> {
    store.read(|r| {
        let id: Option<String> = r
            .conn()
            .query_row(
                "SELECT vehicle_id FROM tours WHERE driver_id = ?1 AND status = 'on_road' AND kind = 'van'
                   AND deleted_at IS NULL ORDER BY started_at DESC LIMIT 1",
                [user],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| vehicle(r.conn(), &id)).transpose()
    })
}

pub fn get_vehicle(store: &Store, id: &str) -> Result<VehicleRow> {
    store.read(|r| vehicle(r.conn(), id))
}

fn check_user(w: &Writer<'_>, user: &str) -> Result<()> {
    let n: i64 = w.tx().query_row(
        "SELECT COUNT(*) FROM users WHERE id = ?1 AND org_id = ?2",
        params![user, w.org()?.to_string()],
        |r| r.get(0),
    )?;
    if n == 0 {
        return Err(rule("driver", "invalid_value"));
    }
    Ok(())
}

/// Adds a vehicle (with a warehouse of its own for what it carries) or
/// changes one (`distribution.fleet`).
pub fn save_vehicle(store: &mut Store, meta: OpMeta, input: VehicleInput) -> Result<VehicleRow> {
    let mut errors = Vec::new();
    if input.name.trim().is_empty() {
        errors.push(RuleError::on("name", "required"));
    }
    if !VEHICLE_KINDS.contains(&input.kind.as_str()) {
        errors.push(RuleError::on("kind", "invalid_value"));
    }
    if !(0..=100_000).contains(&input.capacity_kg) {
        errors.push(RuleError::on("capacity", "invalid_value"));
    }
    if !(0..=200).contains(&input.speed_kmh) {
        errors.push(RuleError::on("speed", "invalid_value"));
    }
    if !errors.is_empty() {
        return Err(errors.into());
    }
    let speed = if input.speed_kmh == 0 {
        35
    } else {
        input.speed_kmh
    };
    store.write(meta, |w| {
        w.require(Permission::DistributionFleet, &Target::org())?;
        let org_id = w.org()?;
        let org = org_id.to_string();
        let driver = input.driver_id.clone().filter(|d| !d.is_empty());
        if let Some(d) = &driver {
            check_user(w, d)?;
        }
        let name = input.name.trim().to_owned();
        let id = match input.id.as_deref().filter(|i| !i.is_empty()) {
            Some(id) => {
                let before = vehicle(w.tx(), id)?;
                w.tx().execute(
                    "UPDATE vehicles SET name = ?2, plate = ?3, kind = ?4, capacity_kg = ?5, driver_id = ?6,
                            phone = ?7, speed_kmh = ?8, updated_at = ?9, version = version + 1
                     WHERE id = ?1",
                    params![
                        id,
                        name,
                        input.plate.trim(),
                        input.kind,
                        input.capacity_kg,
                        driver,
                        input.phone.trim(),
                        speed,
                        w.now()
                    ],
                )?;
                if before.name != name {
                    w.tx().execute(
                        "UPDATE warehouses SET name = ?2, updated_at = ?3, version = version + 1 WHERE id = ?1",
                        params![before.warehouse_id, name, w.now()],
                    )?;
                }
                w.audit("update", "vehicle", id, &input)?;
                id.to_owned()
            }
            None => {
                let branch = match input.branch_id.as_deref().filter(|b| !b.is_empty()) {
                    Some(b) => b.to_owned(),
                    None => w
                        .tx()
                        .query_row(
                            "SELECT id FROM branches WHERE org_id = ?1 AND deleted_at IS NULL
                             ORDER BY created_at LIMIT 1",
                            [&org],
                            |r| r.get::<_, String>(0),
                        )
                        .optional()?
                        .ok_or(Error::NotFound("branch"))?,
                };
                let b = org::branch(w.tx(), branch.parse().map_err(|_| rule("branch", "invalid_value"))?)?;
                let wh = Warehouse {
                    id: nb_core::uuid::Uuid::now_v7().to_string(),
                    branch_id: b.id.clone(),
                    name: name.clone(),
                    negative_stock: "forbid".into(),
                };
                org::insert_warehouse(w, org_id, &wh)?;
                w.event("warehouse.created", "warehouse", &wh.id, &wh)?;
                let id = new_id();
                w.tx().execute(
                    "INSERT INTO vehicles (id, org_id, branch_id, warehouse_id, name, plate, kind, capacity_kg,
                            driver_id, phone, speed_kmh, created_at, updated_at, created_by, device_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?13, ?14)",
                    params![
                        id,
                        org,
                        b.id,
                        wh.id,
                        name,
                        input.plate.trim(),
                        input.kind,
                        input.capacity_kg,
                        driver,
                        input.phone.trim(),
                        speed,
                        w.now(),
                        w.actor_text(),
                        w.device().id.to_string()
                    ],
                )?;
                w.audit("create", "vehicle", &id, &input)?;
                id
            }
        };
        let v = vehicle(w.tx(), &id)?;
        w.event("vehicle.saved", "vehicle", &id, &v)?;
        Ok(v)
    })
}

/// Retires a vehicle: it must carry nothing and have no tour left to do.
pub fn archive_vehicle(store: &mut Store, meta: OpMeta, id: &str) -> Result<()> {
    store.write(meta, |w| {
        w.require(Permission::DistributionFleet, &Target::org())?;
        let v = vehicle(w.tx(), id)?;
        if v.tour_id.is_some() {
            return Err(rule("vehicle", "distribution.vehicle_busy"));
        }
        let holds: bool = w.tx().query_row(
            "SELECT EXISTS (SELECT 1 FROM stock_balances WHERE warehouse_id = ?1 AND on_hand_milli <> 0)
                 OR EXISTS (SELECT 1 FROM stock_reservations WHERE warehouse_id = ?1 AND status = 'active')",
            [&v.warehouse_id],
            |r| r.get(0),
        )?;
        if holds {
            return Err(rule("vehicle", "warehouse.not_empty"));
        }
        w.tx().execute(
            "UPDATE vehicles SET deleted_at = ?2, updated_at = ?2, version = version + 1 WHERE id = ?1",
            params![id, w.now()],
        )?;
        w.tx().execute(
            "UPDATE warehouses SET deleted_at = ?2, updated_at = ?2, version = version + 1 WHERE id = ?1",
            params![v.warehouse_id, w.now()],
        )?;
        w.event("vehicle.archived", "vehicle", id, &())?;
        w.audit("archive", "vehicle", id, &())?;
        Ok(())
    })
}

/// Where a vehicle is now (typed, pasted from a map, or sent by a phone):
/// kept on the vehicle and added to its track (`distribution.drive`).
pub fn record_position(
    store: &mut Store,
    meta: OpMeta,
    vehicle_id: &str,
    position: &str,
    speed_kmh: Option<i64>,
) -> Result<Point> {
    let p = geo::parse_point(position)?;
    store.write(meta, |w| {
        w.require(Permission::DistributionDrive, &Target::org())?;
        let v = vehicle(w.tx(), vehicle_id)?;
        let tour = v
            .tour_id
            .clone()
            .filter(|_| v.tour_status.as_deref() == Some("on_road"));
        w.tx().execute(
            "UPDATE vehicles SET lat_e6 = ?2, lng_e6 = ?3, seen_at = ?4 WHERE id = ?1",
            params![vehicle_id, p.lat_e6, p.lng_e6, w.now()],
        )?;
        w.tx().execute(
            "INSERT INTO vehicle_positions (id, vehicle_id, tour_id, lat_e6, lng_e6, speed_kmh, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new_id(),
                vehicle_id,
                tour,
                p.lat_e6,
                p.lng_e6,
                speed_kmh,
                w.now()
            ],
        )?;
        w.event("vehicle.position", "vehicle", vehicle_id, &p)?;
        Ok(p)
    })
}

/// The last `limit` places a vehicle reported, newest first.
pub fn vehicle_track(store: &Store, vehicle_id: &str, limit: u32) -> Result<Vec<(Point, i64)>> {
    store.read(|r| {
        let mut s = r.conn().prepare(
            "SELECT lat_e6, lng_e6, at FROM vehicle_positions WHERE vehicle_id = ?1 ORDER BY at DESC LIMIT ?2",
        )?;
        let rows = collect(s.query_map(params![vehicle_id, limit.clamp(1, 1000)], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?)?;
        Ok(rows
            .into_iter()
            .filter_map(|(a, b, at)| Point::of(Some(a), Some(b)).map(|p| (p, at)))
            .collect())
    })
}

// ---------------------------------------------------------------------------
// Tours

pub const TOUR_KINDS: [&str; 3] = ["delivery", "van", "presell"];

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TourInput {
    pub vehicle_id: String,
    /// Default: the vehicle's driver.
    #[serde(default)]
    pub driver_id: Option<String>,
    /// `YYYY-MM-DD`; empty: today.
    #[serde(default)]
    pub day: String,
    /// `delivery` (orders), `van` (selling from the vehicle) or `presell`
    /// (taking orders).
    pub kind: String,
    /// Customers to visit.
    #[serde(default)]
    pub customers: Vec<String>,
    /// Approved orders to deliver (each its own stop).
    #[serde(default)]
    pub orders: Vec<String>,
    /// Put the stops in the shortest order (nearest next).
    #[serde(default)]
    pub optimize: bool,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TourRow {
    pub id: String,
    pub number: String,
    pub vehicle_id: String,
    pub vehicle: String,
    pub plate: String,
    pub driver_id: Option<String>,
    pub driver: Option<String>,
    pub day: String,
    pub kind: String,
    pub status: String,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub km_start: Option<i64>,
    pub km_end: Option<i64>,
    /// The cash collected at its stops, and what was counted back.
    pub cash_expected_minor: i64,
    pub cash_counted_minor: Option<i64>,
    pub note: String,
    pub stops: u32,
    pub done: u32,
    pub failed: u32,
    pub collected_minor: i64,
    /// What the orders it carries are worth.
    pub orders_minor: i64,
}

impl TourRow {
    /// Stops not yet done, failed or skipped.
    pub fn left(&self) -> u32 {
        self.stops.saturating_sub(self.done + self.failed)
    }
    pub fn settled(&self) -> bool {
        self.cash_counted_minor.is_some()
    }
}

const TOUR_SELECT: &str = "SELECT t.id, t.number, t.vehicle_id, v.name, v.plate, t.driver_id, u.name, t.day, t.kind,
        t.status, t.started_at, t.ended_at, t.km_start, t.km_end, t.cash_expected_minor, t.cash_counted_minor,
        t.note,
        (SELECT COUNT(*) FROM tour_stops s WHERE s.tour_id = t.id),
        (SELECT COUNT(*) FROM tour_stops s WHERE s.tour_id = t.id AND s.status = 'done'),
        (SELECT COUNT(*) FROM tour_stops s WHERE s.tour_id = t.id AND s.status IN ('failed', 'skipped')),
        (SELECT COALESCE(SUM(s.collected_minor), 0) FROM tour_stops s WHERE s.tour_id = t.id),
        (SELECT COALESCE(SUM(d.total_minor), 0) FROM tour_stops s JOIN sales_documents d ON d.id = s.document_id
          WHERE s.tour_id = t.id)
     FROM tours t JOIN vehicles v ON v.id = t.vehicle_id LEFT JOIN users u ON u.id = t.driver_id";

fn tour_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TourRow> {
    Ok(TourRow {
        id: r.get(0)?,
        number: r.get(1)?,
        vehicle_id: r.get(2)?,
        vehicle: r.get(3)?,
        plate: r.get(4)?,
        driver_id: r.get(5)?,
        driver: r.get(6)?,
        day: r.get(7)?,
        kind: r.get(8)?,
        status: r.get(9)?,
        started_at: r.get(10)?,
        ended_at: r.get(11)?,
        km_start: r.get(12)?,
        km_end: r.get(13)?,
        cash_expected_minor: r.get(14)?,
        cash_counted_minor: r.get(15)?,
        note: r.get(16)?,
        stops: r.get(17)?,
        done: r.get(18)?,
        failed: r.get(19)?,
        collected_minor: r.get(20)?,
        orders_minor: r.get(21)?,
    })
}

fn tour(conn: &Connection, id: &str) -> Result<TourRow> {
    conn.query_row(
        &format!("{TOUR_SELECT} WHERE t.id = ?1 AND t.deleted_at IS NULL"),
        [id],
        tour_row,
    )
    .optional()?
    .ok_or(Error::Rules(vec![RuleError::on(
        "tour",
        "distribution.tour_unknown",
    )]))
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TourQuery {
    /// One day (`YYYY-MM-DD`), or any.
    #[serde(default)]
    pub day: String,
    /// ``, `live` (planned or on the road), `planned`, `on_road`, `done`,
    /// `unsettled` (done, cash not counted) or `cancelled`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub vehicle_id: Option<String>,
    /// Only the tours of this driver.
    #[serde(default)]
    pub driver_id: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

pub fn list_tours(store: &Store, q: &TourQuery) -> Result<Vec<TourRow>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(&format!(
            "{TOUR_SELECT}
             WHERE t.org_id = ?1 AND t.deleted_at IS NULL
               AND (?2 = '' OR t.day = ?2)
               AND (?3 = '' OR (?3 = 'live' AND t.status IN ('planned', 'on_road'))
                    OR (?3 = 'unsettled' AND t.status = 'done' AND t.cash_counted_minor IS NULL)
                    OR t.status = ?3)
               AND (?4 = '' OR t.vehicle_id = ?4)
               AND (?5 = '' OR t.driver_id = ?5)
             ORDER BY t.status = 'on_road' DESC, t.day DESC, t.created_at DESC LIMIT ?6 OFFSET ?7"
        ))?;
        let rows = collect(s.query_map(
            params![
                org.to_string(),
                q.day.trim(),
                q.status,
                q.vehicle_id.clone().unwrap_or_default(),
                q.driver_id.clone().unwrap_or_default(),
                q.limit.clamp(1, 500),
                q.offset
            ],
            tour_row,
        )?)?;
        Ok(rows)
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopRow {
    pub id: String,
    pub seq: i64,
    pub customer_id: String,
    pub customer: String,
    pub phone: String,
    pub address: String,
    pub city: String,
    pub zone: Option<String>,
    pub point: Option<Point>,
    pub delivery_note: String,
    pub balance_minor: i64,
    pub document_id: Option<String>,
    pub document_number: Option<String>,
    pub document_total_minor: Option<i64>,
    /// The order's state (approved, invoiced, …).
    pub document_status: Option<String>,
    pub status: String,
    pub arrived_at: Option<i64>,
    pub done_at: Option<i64>,
    pub fail_reason: String,
    pub receiver: String,
    pub collected_minor: i64,
    pub notified_at: Option<i64>,
    pub note: String,
    /// From the previous place (the vehicle for the next stop).
    pub leg_m: Option<u64>,
    /// Minutes from now to get there (stops still to do).
    pub eta_min: Option<u32>,
}

impl StopRow {
    pub fn open(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "arrived")
    }
}

const STOP_SELECT: &str = "SELECT s.id, s.seq, c.id, c.name, c.phone, c.address, c.city, z.name, c.lat_e6, c.lng_e6,
        c.delivery_note, c.balance_minor, s.document_id, d.number, d.total_minor, d.status, s.status,
        s.arrived_at, s.done_at, s.fail_reason, s.receiver, s.collected_minor, s.notified_at, s.note
     FROM tour_stops s JOIN customers c ON c.id = s.customer_id
     LEFT JOIN zones z ON z.id = c.zone_id LEFT JOIN sales_documents d ON d.id = s.document_id";

fn stop_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StopRow> {
    Ok(StopRow {
        id: r.get(0)?,
        seq: r.get(1)?,
        customer_id: r.get(2)?,
        customer: r.get(3)?,
        phone: r.get(4)?,
        address: r.get(5)?,
        city: r.get(6)?,
        zone: r.get(7)?,
        point: Point::of(r.get(8)?, r.get(9)?),
        delivery_note: r.get(10)?,
        balance_minor: r.get(11)?,
        document_id: r.get(12)?,
        document_number: r.get(13)?,
        document_total_minor: r.get(14)?,
        document_status: r.get(15)?,
        status: r.get(16)?,
        arrived_at: r.get(17)?,
        done_at: r.get(18)?,
        fail_reason: r.get(19)?,
        receiver: r.get(20)?,
        collected_minor: r.get(21)?,
        notified_at: r.get(22)?,
        note: r.get(23)?,
        leg_m: None,
        eta_min: None,
    })
}

fn stops_of(conn: &Connection, tour: &str) -> Result<Vec<StopRow>> {
    let mut s = conn.prepare(&format!(
        "{STOP_SELECT} WHERE s.tour_id = ?1 ORDER BY s.seq"
    ))?;
    let rows = collect(s.query_map([tour], stop_row)?)?;
    Ok(rows)
}

/// A tour with its stops, the distance between them and when each stop
/// still to do should be reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TourDetail {
    pub tour: TourRow,
    /// Where the vehicle is (if known) and since when.
    pub vehicle_point: Option<Point>,
    pub seen_at: Option<i64>,
    pub speed_kmh: i64,
    pub stops: Vec<StopRow>,
    /// The whole route, and what is left of it.
    pub route_m: u64,
    pub left_m: u64,
}

impl TourDetail {
    pub fn next(&self) -> Option<&StopRow> {
        self.stops.iter().find(|s| s.open())
    }
}

fn detail(conn: &Connection, id: &str) -> Result<TourDetail> {
    let t = tour(conn, id)?;
    let v = vehicle(conn, &t.vehicle_id)?;
    let mut stops = stops_of(conn, id)?;
    let points: Vec<Option<Point>> = stops.iter().map(|s| s.point).collect();
    let route_m = geo::route_m(None, &points);
    // Legs: each stop from the place before it.
    let mut prev: Option<Point> = None;
    for s in stops.iter_mut() {
        s.leg_m = match (prev, s.point) {
            (Some(a), Some(b)) => Some(geo::distance_m(a, b)),
            _ => None,
        };
        if s.point.is_some() {
            prev = s.point;
        }
    }
    // What is left: from the vehicle (or the last stop reached) through the
    // open stops, with a few minutes at each.
    let speed = u32::try_from(v.speed_kmh).unwrap_or(35);
    let mut here = if t.status == "on_road" {
        v.point.or_else(|| {
            stops
                .iter()
                .filter(|s| !s.open())
                .filter_map(|s| s.point)
                .next_back()
        })
    } else {
        None
    };
    let mut minutes = 0u32;
    let mut left_m = 0u64;
    let mut first = true;
    for s in stops.iter_mut().filter(|s| s.open()) {
        if let (Some(h), Some(p)) = (here, s.point) {
            let d = geo::distance_m(h, p);
            if first {
                s.leg_m = Some(d);
            }
            left_m += d;
            minutes += geo::drive_minutes(d, speed);
            if t.status == "on_road" {
                s.eta_min = Some(minutes);
            }
        }
        if s.status == "arrived" {
            s.eta_min = Some(0);
        }
        minutes += STOP_MIN;
        if s.point.is_some() {
            here = s.point;
        }
        first = false;
    }
    Ok(TourDetail {
        vehicle_point: v.point,
        seen_at: v.seen_at,
        speed_kmh: v.speed_kmh,
        tour: t,
        stops,
        route_m,
        left_m,
    })
}

pub fn get_tour(store: &Store, id: &str) -> Result<TourDetail> {
    store.read(|r| detail(r.conn(), id))
}

fn valid_day(day: &str) -> bool {
    let b = day.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && (1..=12).contains(&day[5..7].parse::<u32>().unwrap_or(0))
        && (1..=31).contains(&day[8..10].parse::<u32>().unwrap_or(0))
}

fn today(w: &Writer<'_>) -> String {
    let (y, m, d) = w.date();
    format!("{y:04}-{m:02}-{d:02}")
}

/// Adds stops at the end of a tour: one per order, then one per customer
/// not already visited by it. Returns how many were added.
fn add_stops_in(
    w: &mut Writer<'_>,
    tour_id: &str,
    customers: &[String],
    orders: &[String],
) -> Result<u32> {
    let org = w.org()?.to_string();
    let mut seq: i64 = w.tx().query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM tour_stops WHERE tour_id = ?1",
        [tour_id],
        |r| r.get(0),
    )?;
    let mut added = 0;
    let mut insert = |w: &mut Writer<'_>, customer: &str, doc: Option<&str>| -> Result<()> {
        seq += 1;
        w.tx().execute(
            "INSERT INTO tour_stops (id, tour_id, seq, customer_id, document_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![new_id(), tour_id, seq, customer, doc, w.now()],
        )?;
        added += 1;
        Ok(())
    };
    for o in orders {
        let customer: Option<String> = w
            .tx()
            .query_row(
                &format!(
                    "SELECT d.customer_id FROM sales_documents d WHERE d.id = ?1 AND d.org_id = ?2 AND {OPEN_ORDER}"
                ),
                params![o, org],
                |r| r.get(0),
            )
            .optional()?;
        let Some(customer) = customer else {
            return Err(rule("order", "distribution.order_not_open"));
        };
        insert(w, &customer, Some(o))?;
    }
    for c in customers {
        let known: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM customers WHERE id = ?1 AND org_id = ?2",
            params![c, org],
            |r| r.get(0),
        )?;
        if known == 0 {
            return Err(rule("customer", "customer.unknown"));
        }
        let visited: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM tour_stops WHERE tour_id = ?1 AND customer_id = ?2",
            params![tour_id, c],
            |r| r.get(0),
        )?;
        if visited == 0 {
            insert(w, c, None)?;
        }
    }
    Ok(added)
}

/// Puts the stops still to do in nearest-next order, from where the
/// vehicle is (the depot before it leaves) or the last stop reached.
fn optimize_in(w: &mut Writer<'_>, tour_id: &str) -> Result<()> {
    let d = detail(w.tx(), tour_id)?;
    let done: Vec<&StopRow> = d.stops.iter().filter(|s| !s.open()).collect();
    let open: Vec<&StopRow> = d.stops.iter().filter(|s| s.open()).collect();
    let start = d
        .vehicle_point
        .or_else(|| done.iter().filter_map(|s| s.point).next_back());
    let points: Vec<Option<Point>> = open.iter().map(|s| s.point).collect();
    let order = geo::visit_order(start, &points);
    let mut seq = i64::try_from(done.len()).unwrap_or(0);
    // Done stops keep their place first, in the order they were reached.
    for (k, s) in done.iter().enumerate() {
        w.tx().execute(
            "UPDATE tour_stops SET seq = ?2 WHERE id = ?1",
            params![s.id, i64::try_from(k).unwrap_or(0) + 1],
        )?;
    }
    for i in order {
        seq += 1;
        w.tx().execute(
            "UPDATE tour_stops SET seq = ?2, updated_at = ?3, version = version + 1 WHERE id = ?1",
            params![open[i].id, seq, w.now()],
        )?;
    }
    Ok(())
}

/// Plans a tour (`distribution.plan`): a vehicle, a day and its stops.
pub fn create_tour(store: &mut Store, meta: OpMeta, input: TourInput) -> Result<TourDetail> {
    if !TOUR_KINDS.contains(&input.kind.as_str()) {
        return Err(rule("kind", "invalid_value"));
    }
    if !input.day.trim().is_empty() && !valid_day(input.day.trim()) {
        return Err(rule("day", "invalid_value"));
    }
    let id = store.write(meta, |w| {
        w.require(Permission::DistributionPlan, &Target::org())?;
        let v = vehicle(w.tx(), &input.vehicle_id)?;
        let driver = input
            .driver_id
            .clone()
            .filter(|d| !d.is_empty())
            .or(v.driver_id.clone());
        if let Some(d) = &driver {
            check_user(w, d)?;
        }
        let day = match input.day.trim() {
            "" => today(w),
            d => d.to_owned(),
        };
        let b = org::branch(
            w.tx(),
            v.branch_id
                .parse()
                .map_err(|_| rule("branch", "invalid_value"))?,
        )?;
        let number = w.next_number(DocKind::Tour, &b.id, &b.code)?;
        let id = new_id();
        w.tx().execute(
            "INSERT INTO tours (id, org_id, branch_id, number, vehicle_id, driver_id, day, kind, note,
                    created_at, updated_at, created_by, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11, ?12)",
            params![
                id,
                w.org()?.to_string(),
                b.id,
                number,
                v.id,
                driver,
                day,
                input.kind,
                input.note.trim(),
                w.now(),
                w.actor_text(),
                w.device().id.to_string()
            ],
        )?;
        add_stops_in(w, &id, &input.customers, &input.orders)?;
        if input.optimize {
            optimize_in(w, &id)?;
        }
        let t = tour(w.tx(), &id)?;
        w.event("tour.planned", "tour", &id, &t)?;
        w.audit("create", "tour", &id, &input)?;
        Ok(id)
    })?;
    get_tour(store, &id)
}

/// A tour that can still be changed (planned or on the road).
fn live_tour(w: &Writer<'_>, id: &str) -> Result<TourRow> {
    let t = tour(w.tx(), id)?;
    if !matches!(t.status.as_str(), "planned" | "on_road") {
        return Err(rule("tour", "distribution.tour_closed"));
    }
    Ok(t)
}

/// The driver works their own tour; a planner may work any.
fn require_driver(w: &mut Writer<'_>, t: &TourRow) -> Result<()> {
    w.require(Permission::DistributionDrive, &Target::org())?;
    let me = w.actor()?.to_string();
    if t.driver_id.as_deref() != Some(me.as_str())
        && !w.allowed(Permission::DistributionPlan, &Target::org())?
    {
        return Err(rule("tour", "distribution.not_your_tour"));
    }
    Ok(())
}

pub fn add_stops(
    store: &mut Store,
    meta: OpMeta,
    tour_id: &str,
    customers: Vec<String>,
    orders: Vec<String>,
) -> Result<TourDetail> {
    store.write(meta, |w| {
        w.require(Permission::DistributionPlan, &Target::org())?;
        live_tour(w, tour_id)?;
        let n = add_stops_in(w, tour_id, &customers, &orders)?;
        w.event("tour.stops_added", "tour", tour_id, &(&customers, &orders))?;
        w.audit("add_stops", "tour", tour_id, &n)?;
        Ok(())
    })?;
    get_tour(store, tour_id)
}

fn stop_tour(w: &Writer<'_>, stop: &str) -> Result<(String, String)> {
    w.tx()
        .query_row(
            "SELECT s.tour_id, s.status FROM tour_stops s JOIN tours t ON t.id = s.tour_id
             WHERE s.id = ?1 AND t.org_id = ?2",
            params![stop, w.org()?.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::Rules(vec![RuleError::on(
            "stop",
            "distribution.stop_unknown",
        )]))
}

/// Takes a stop not yet reached off its tour.
pub fn remove_stop(store: &mut Store, meta: OpMeta, stop: &str) -> Result<TourDetail> {
    let tour_id = store.write(meta, |w| {
        w.require(Permission::DistributionPlan, &Target::org())?;
        let (tour_id, status) = stop_tour(w, stop)?;
        live_tour(w, &tour_id)?;
        if status != "pending" {
            return Err(rule("stop", "distribution.stop_reached"));
        }
        w.tx()
            .execute("DELETE FROM tour_stops WHERE id = ?1", [stop])?;
        w.event("tour.stop_removed", "tour", &tour_id, &stop)?;
        w.audit("remove_stop", "tour", &tour_id, &stop)?;
        Ok(tour_id)
    })?;
    get_tour(store, &tour_id)
}

/// Moves a stop to `index` (0-based) among the tour's stops.
pub fn move_stop(store: &mut Store, meta: OpMeta, stop: &str, index: usize) -> Result<TourDetail> {
    let tour_id = store.write(meta, |w| {
        let (tour_id, _) = stop_tour(w, stop)?;
        let t = live_tour(w, &tour_id)?;
        // The driver may reorder their own day; others need to plan.
        if !w.allowed(Permission::DistributionPlan, &Target::org())? {
            require_driver(w, &t)?;
        }
        let mut ids: Vec<String> = stops_of(w.tx(), &tour_id)?
            .into_iter()
            .map(|s| s.id)
            .collect();
        let Some(from) = ids.iter().position(|i| i == stop) else {
            return Err(rule("stop", "distribution.stop_unknown"));
        };
        let id = ids.remove(from);
        ids.insert(index.min(ids.len()), id);
        for (k, id) in ids.iter().enumerate() {
            w.tx().execute(
                "UPDATE tour_stops SET seq = ?2 WHERE id = ?1",
                params![id, i64::try_from(k).unwrap_or(0) + 1],
            )?;
        }
        w.event("tour.reordered", "tour", &tour_id, &ids)?;
        w.audit("reorder", "tour", &tour_id, &ids)?;
        Ok(tour_id)
    })?;
    get_tour(store, &tour_id)
}

/// Reorders the stops still to do for the shortest drive (nearest next).
pub fn optimize_tour(store: &mut Store, meta: OpMeta, tour_id: &str) -> Result<TourDetail> {
    store.write(meta, |w| {
        let t = live_tour(w, tour_id)?;
        if !w.allowed(Permission::DistributionPlan, &Target::org())? {
            require_driver(w, &t)?;
        }
        optimize_in(w, tour_id)?;
        w.event("tour.optimized", "tour", tour_id, &())?;
        w.audit("optimize", "tour", tour_id, &())?;
        Ok(())
    })?;
    get_tour(store, tour_id)
}

/// Cancels a tour not yet started; its orders wait for another.
pub fn cancel_tour(store: &mut Store, meta: OpMeta, tour_id: &str, reason: &str) -> Result<()> {
    store.write(meta, |w| {
        w.require(Permission::DistributionPlan, &Target::org())?;
        let t = tour(w.tx(), tour_id)?;
        if t.status != "planned" {
            return Err(rule("tour", "distribution.tour_started"));
        }
        w.tx().execute(
            "UPDATE tours SET status = 'cancelled', note = trim(note || ' ' || ?2), updated_at = ?3,
                    version = version + 1 WHERE id = ?1",
            params![tour_id, reason.trim(), w.now()],
        )?;
        w.event("tour.cancelled", "tour", tour_id, &reason)?;
        w.audit("cancel", "tour", tour_id, &reason)?;
        Ok(())
    })
}

/// The vehicle leaves (`distribution.drive`).
pub fn start_tour(
    store: &mut Store,
    meta: OpMeta,
    tour_id: &str,
    km_start: Option<i64>,
) -> Result<TourDetail> {
    if km_start.is_some_and(|k| k < 0) {
        return Err(rule("km", "invalid_value"));
    }
    store.write(meta, |w| {
        let t = tour(w.tx(), tour_id)?;
        require_driver(w, &t)?;
        if t.status != "planned" {
            return Err(rule("tour", "distribution.tour_started"));
        }
        let busy: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM tours WHERE vehicle_id = ?1 AND status = 'on_road'",
            [&t.vehicle_id],
            |r| r.get(0),
        )?;
        if busy > 0 {
            return Err(rule("vehicle", "distribution.vehicle_busy"));
        }
        w.tx().execute(
            "UPDATE tours SET status = 'on_road', started_at = ?2, km_start = ?3, updated_at = ?2,
                    version = version + 1 WHERE id = ?1",
            params![tour_id, w.now(), km_start],
        )?;
        w.event("tour.started", "tour", tour_id, &km_start)?;
        w.audit("start", "tour", tour_id, &km_start)?;
        Ok(())
    })?;
    get_tour(store, tour_id)
}

/// A stop of a tour on the road, for its driver.
fn driving_stop(w: &mut Writer<'_>, stop: &str) -> Result<(TourRow, String)> {
    let (tour_id, status) = stop_tour(w, stop)?;
    let t = tour(w.tx(), &tour_id)?;
    require_driver(w, &t)?;
    if t.status != "on_road" {
        return Err(rule("tour", "distribution.tour_not_started"));
    }
    Ok((t, status))
}

/// The vehicle is at the customer.
pub fn arrive_stop(store: &mut Store, meta: OpMeta, stop: &str) -> Result<TourDetail> {
    let tour_id = store.write(meta, |w| {
        let (t, status) = driving_stop(w, stop)?;
        if status != "pending" {
            return Err(rule("stop", "distribution.stop_reached"));
        }
        w.tx().execute(
            "UPDATE tour_stops SET status = 'arrived', arrived_at = ?2, updated_at = ?2, version = version + 1
             WHERE id = ?1",
            params![stop, w.now()],
        )?;
        w.event("tour.arrived", "tour", &t.id, &stop)?;
        w.audit("arrive", "tour_stop", stop, &())?;
        Ok(t.id)
    })?;
    get_tour(store, &tour_id)
}

/// What happened at a stop that went well.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StopDone {
    /// Who took the goods.
    #[serde(default)]
    pub receiver: String,
    /// Money taken from the customer (a receipt on its account).
    #[serde(default)]
    pub collected_minor: i64,
    /// `cash`, `card` or `transfer`.
    #[serde(default)]
    pub method: String,
    /// The till the cash goes to.
    #[serde(default)]
    pub register_id: Option<String>,
    #[serde(default)]
    pub note: String,
}

/// The stop is done: delivered, visited or sold to; money collected there
/// is written as the customer's receipt in the same transaction.
pub fn complete_stop(
    store: &mut Store,
    meta: OpMeta,
    stop: &str,
    done: StopDone,
) -> Result<TourDetail> {
    if done.collected_minor < 0 {
        return Err(rule("collected", "ledger.negative"));
    }
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    let tour_id = store.write(meta, |w| {
        let (t, status) = driving_stop(w, stop)?;
        if !matches!(status.as_str(), "pending" | "arrived") {
            return Err(rule("stop", "distribution.stop_closed"));
        }
        let customer: String = w.tx().query_row(
            "SELECT customer_id FROM tour_stops WHERE id = ?1",
            [stop],
            |r| r.get(0),
        )?;
        if done.collected_minor > 0 {
            let method = if done.method.is_empty() {
                "cash".to_owned()
            } else {
                done.method.clone()
            };
            collect_in(
                w,
                &ReceiptInput {
                    customer_id: customer,
                    amount_minor: done.collected_minor,
                    method,
                    register_id: done.register_id.clone(),
                    allocations: Vec::new(),
                    unallocated: false,
                    reference: t.number.clone(),
                    note: done.note.trim().to_owned(),
                },
                currency,
                offset,
            )?;
        }
        w.tx().execute(
            "UPDATE tour_stops SET status = 'done', done_at = ?2, arrived_at = COALESCE(arrived_at, ?2),
                    receiver = ?3, collected_minor = ?4, note = ?5, updated_at = ?2, version = version + 1
             WHERE id = ?1",
            params![
                stop,
                w.now(),
                done.receiver.trim(),
                done.collected_minor,
                done.note.trim()
            ],
        )?;
        w.event("tour.stop_done", "tour", &t.id, &(stop, &done))?;
        w.audit("complete", "tour_stop", stop, &done)?;
        Ok(t.id)
    })?;
    get_tour(store, &tour_id)
}

/// The stop could not be done (closed, refused, absent…), or is left for
/// another day (`skip`). An order on it waits for another tour.
pub fn fail_stop(
    store: &mut Store,
    meta: OpMeta,
    stop: &str,
    reason: &str,
    skip: bool,
) -> Result<TourDetail> {
    if !skip && reason.trim().is_empty() {
        return Err(rule("reason", "required"));
    }
    let tour_id = store.write(meta, |w| {
        let (t, status) = driving_stop(w, stop)?;
        if !matches!(status.as_str(), "pending" | "arrived") {
            return Err(rule("stop", "distribution.stop_closed"));
        }
        w.tx().execute(
            "UPDATE tour_stops SET status = ?2, fail_reason = ?3, done_at = ?4, updated_at = ?4,
                    version = version + 1
             WHERE id = ?1",
            params![
                stop,
                if skip { "skipped" } else { "failed" },
                reason.trim(),
                w.now()
            ],
        )?;
        w.event("tour.stop_failed", "tour", &t.id, &(stop, reason, skip))?;
        w.audit(
            if skip { "skip" } else { "fail" },
            "tour_stop",
            stop,
            &reason,
        )?;
        Ok(t.id)
    })?;
    get_tour(store, &tour_id)
}

/// Notes that the customer was told the vehicle is coming.
pub fn mark_notified(store: &mut Store, meta: OpMeta, stop: &str) -> Result<()> {
    store.write(meta, |w| {
        let (tour_id, _) = stop_tour(w, stop)?;
        let t = tour(w.tx(), &tour_id)?;
        if !w.allowed(Permission::DistributionPlan, &Target::org())? {
            require_driver(w, &t)?;
        }
        w.tx().execute(
            "UPDATE tour_stops SET notified_at = ?2, updated_at = ?2 WHERE id = ?1",
            params![stop, w.now()],
        )?;
        w.event("tour.notified", "tour", &tour_id, &stop)?;
        Ok(())
    })
}

/// The vehicle is back: stops not reached are skipped, the cash collected
/// is what the driver should hand over.
pub fn end_tour(
    store: &mut Store,
    meta: OpMeta,
    tour_id: &str,
    km_end: Option<i64>,
) -> Result<TourDetail> {
    store.write(meta, |w| {
        let t = tour(w.tx(), tour_id)?;
        require_driver(w, &t)?;
        if t.status != "on_road" {
            return Err(rule("tour", "distribution.tour_not_started"));
        }
        if let (Some(a), Some(b)) = (t.km_start, km_end)
            && b < a
        {
            return Err(rule("km", "distribution.km_back"));
        }
        w.tx().execute(
            "UPDATE tour_stops SET status = 'skipped', fail_reason = 'tour_ended', done_at = ?2, updated_at = ?2
             WHERE tour_id = ?1 AND status IN ('pending', 'arrived')",
            params![tour_id, w.now()],
        )?;
        let cash: i64 = w.tx().query_row(
            "SELECT COALESCE(SUM(collected_minor), 0) FROM tour_stops WHERE tour_id = ?1",
            [tour_id],
            |r| r.get(0),
        )?;
        w.tx().execute(
            "UPDATE tours SET status = 'done', ended_at = ?2, km_end = ?3, cash_expected_minor = ?4,
                    updated_at = ?2, version = version + 1 WHERE id = ?1",
            params![tour_id, w.now(), km_end, cash],
        )?;
        w.event("tour.ended", "tour", tour_id, &(km_end, cash))?;
        w.audit("end", "tour", tour_id, &(km_end, cash))?;
        Ok(())
    })?;
    get_tour(store, tour_id)
}

/// Counts what the driver handed over against what was collected
/// (`distribution.settle`); a difference is kept in the note and the audit.
pub fn settle_tour(
    store: &mut Store,
    meta: OpMeta,
    tour_id: &str,
    counted_minor: i64,
    note: &str,
) -> Result<TourDetail> {
    if counted_minor < 0 {
        return Err(rule("counted", "ledger.negative"));
    }
    store.write(meta, |w| {
        w.require(Permission::DistributionSettle, &Target::org())?;
        let t = tour(w.tx(), tour_id)?;
        if t.status != "done" {
            return Err(rule("tour", "distribution.tour_not_ended"));
        }
        if t.settled() {
            return Err(rule("tour", "distribution.tour_settled"));
        }
        w.tx().execute(
            "UPDATE tours SET cash_counted_minor = ?2, note = trim(note || ' ' || ?3), updated_at = ?4,
                    version = version + 1 WHERE id = ?1",
            params![tour_id, counted_minor, note.trim(), w.now()],
        )?;
        let diff = counted_minor - t.cash_expected_minor;
        w.event("tour.settled", "tour", tour_id, &(counted_minor, diff))?;
        w.audit("settle", "tour", tour_id, &(counted_minor, diff, note))?;
        Ok(())
    })?;
    get_tour(store, tour_id)
}

// ---------------------------------------------------------------------------
// The board

/// Something on the road that needs attention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    /// `near` (vehicle close to a customer not yet told), `silent` (no
    /// position for a while), `late` (planned for an earlier day and not
    /// finished), `failed` (stops that could not be done), `unsettled`
    /// (back, cash not counted), `unplaced` (stops with no place).
    pub kind: String,
    pub tour_id: String,
    pub tour_number: String,
    pub vehicle: String,
    pub stop_id: Option<String>,
    pub customer: Option<String>,
    pub phone: Option<String>,
    /// Metres (near), minutes (silent), days (late), stops (failed,
    /// unplaced) or money (unsettled).
    pub value: i64,
}

/// A tour on the road (or planned for today) and its next stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveTour {
    pub detail: TourDetail,
    pub next: Option<StopRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Board {
    pub today: String,
    pub live: Vec<LiveTour>,
    pub alerts: Vec<Alert>,
    pub vehicles: u32,
    pub on_road: u32,
    pub stops_today: u32,
    pub done_today: u32,
    pub failed_today: u32,
    pub collected_today_minor: i64,
    pub open_orders: u32,
    pub open_orders_minor: i64,
    pub unplaced_customers: u32,
}

/// The day at a glance: what is on the road, where, what comes next and
/// when, and the alerts.
pub fn distribution_board(store: &Store) -> Result<Board> {
    let now = store.now_ms();
    store.read(|r| {
        let conn = r.conn();
        let Some(o) = org::the_org(conn)? else {
            return Ok(Board::default());
        };
        let org = o.org_id()?.to_string();
        let (y, m, d) = crate::time::civil_date(now, r.utc_offset_min);
        let today = format!("{y:04}-{m:02}-{d:02}");
        let mut b = Board {
            today: today.clone(),
            ..Board::default()
        };
        b.vehicles = conn.query_row(
            "SELECT COUNT(*) FROM vehicles WHERE org_id = ?1 AND deleted_at IS NULL",
            [&org],
            |r| r.get(0),
        )?;
        (b.open_orders, b.open_orders_minor) = conn.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(d.total_minor), 0) FROM sales_documents d WHERE d.org_id = ?1 AND {OPEN_ORDER}"
            ),
            [&org],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        b.unplaced_customers = conn.query_row(
            "SELECT COUNT(*) FROM customers WHERE org_id = ?1 AND status = 'active' AND lat_e6 IS NULL
                 AND (visit_days <> 0 OR zone_id IS NOT NULL)",
            [&org],
            |r| r.get(0),
        )?;
        let ids: Vec<String> = {
            let mut s = conn.prepare(
                "SELECT id FROM tours WHERE org_id = ?1 AND deleted_at IS NULL
                   AND (status = 'on_road' OR (status = 'planned' AND day <= ?2)
                        OR (status = 'done' AND (day = ?2 OR cash_counted_minor IS NULL)))
                 ORDER BY status = 'on_road' DESC, day, number",
            )?;
            collect(s.query_map(params![org, today], |r| r.get(0))?)?
        };
        for id in ids {
            let d = detail(conn, &id)?;
            let t = &d.tour;
            if t.day == today {
                b.stops_today += t.stops;
                b.done_today += t.done;
                b.failed_today += t.failed;
                b.collected_today_minor += t.collected_minor;
            }
            let alert = |kind: &str, stop: Option<&StopRow>, value: i64| Alert {
                kind: kind.into(),
                tour_id: t.id.clone(),
                tour_number: t.number.clone(),
                vehicle: t.vehicle.clone(),
                stop_id: stop.map(|s| s.id.clone()),
                customer: stop.map(|s| s.customer.clone()),
                phone: stop.map(|s| s.phone.clone()),
                value,
            };
            if t.status == "done" {
                if !t.settled() && t.cash_expected_minor > 0 {
                    b.alerts.push(alert("unsettled", None, t.cash_expected_minor));
                }
                continue;
            }
            if t.day < today {
                let days = days_between(&t.day, &today);
                b.alerts.push(alert("late", None, days));
            }
            let failed = d.stops.iter().filter(|s| s.status == "failed").count();
            if failed > 0 {
                b.alerts
                    .push(alert("failed", None, i64::try_from(failed).unwrap_or(0)));
            }
            let unplaced = d
                .stops
                .iter()
                .filter(|s| s.open() && s.point.is_none())
                .count();
            if unplaced > 0 {
                b.alerts
                    .push(alert("unplaced", None, i64::try_from(unplaced).unwrap_or(0)));
            }
            if t.status == "on_road" {
                b.on_road += 1;
                match d.seen_at {
                    Some(at) if now - at <= SILENT_MS => {}
                    seen => {
                        let since = seen.unwrap_or(t.started_at.unwrap_or(now));
                        let minutes = (now - since).max(0) / 60_000;
                        if seen.is_none() || minutes > 0 {
                            b.alerts.push(alert("silent", None, minutes));
                        }
                    }
                }
                if let (Some(v), Some(next)) = (d.vehicle_point, d.next())
                    && let Some(p) = next.point
                {
                    let m = geo::distance_m(v, p);
                    if m <= NEAR_M && next.notified_at.is_none() && next.status == "pending" {
                        b.alerts
                            .push(alert("near", Some(next), i64::try_from(m).unwrap_or(0)));
                    }
                }
            }
            let next = d.next().cloned();
            b.live.push(LiveTour { detail: d, next });
        }
        Ok(b)
    })
}

fn days_between(a: &str, b: &str) -> i64 {
    let n = |s: &str| -> Option<i64> {
        let y: i64 = s.get(0..4)?.parse().ok()?;
        let m: i64 = s.get(5..7)?.parse().ok()?;
        let d: i64 = s.get(8..10)?.parse().ok()?;
        // Days from civil (Howard Hinnant).
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Some(era * 146_097 + doe - 719_468)
    };
    match (n(a), n(b)) {
        (Some(x), Some(y)) => y - x,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days() {
        assert!(valid_day("2026-11-13"));
        assert!(!valid_day("2026-13-01"));
        assert!(!valid_day("26-11-13"));
        assert_eq!(days_between("2026-11-10", "2026-11-13"), 3);
        assert_eq!(days_between("2026-12-31", "2027-01-01"), 1);
        assert_eq!(days_between("2024-02-28", "2024-03-01"), 2);
    }
}
