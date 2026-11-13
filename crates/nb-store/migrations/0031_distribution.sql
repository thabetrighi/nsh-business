-- Distribution and field sales (M16, owner notes): zones, customers on the
-- map, vehicles with stock of their own, tours of stops (deliveries,
-- visits, van sales), where the vehicles have been. Places are kept as
-- whole micro-degrees (no floating point).

CREATE TABLE zones (
    id          TEXT PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES organizations(id),
    name        TEXT NOT NULL,
    color       INTEGER NOT NULL DEFAULT 0,
    note        TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    created_by  TEXT,
    device_id   TEXT NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1,
    deleted_at  INTEGER
);
CREATE UNIQUE INDEX zones_name ON zones(org_id, name) WHERE deleted_at IS NULL;

-- Where a customer is, its zone, the days it is visited (Monday = 1,
-- Tuesday = 2, … Sunday = 64), what the driver should know.
ALTER TABLE customers ADD COLUMN zone_id TEXT REFERENCES zones(id);
ALTER TABLE customers ADD COLUMN lat_e6 INTEGER;
ALTER TABLE customers ADD COLUMN lng_e6 INTEGER;
ALTER TABLE customers ADD COLUMN visit_days INTEGER NOT NULL DEFAULT 0;
ALTER TABLE customers ADD COLUMN delivery_note TEXT NOT NULL DEFAULT '';

-- A vehicle carries its own stock: a warehouse of its own.
CREATE TABLE vehicles (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    branch_id     TEXT NOT NULL REFERENCES branches(id),
    warehouse_id  TEXT NOT NULL REFERENCES warehouses(id),
    name          TEXT NOT NULL,
    plate         TEXT NOT NULL DEFAULT '',
    kind          TEXT NOT NULL DEFAULT 'van'
                  CHECK (kind IN ('van', 'truck', 'car', 'motorbike', 'bike')),
    capacity_kg   INTEGER NOT NULL DEFAULT 0,
    driver_id     TEXT REFERENCES users(id),
    phone         TEXT NOT NULL DEFAULT '',
    speed_kmh     INTEGER NOT NULL DEFAULT 35,
    lat_e6        INTEGER,
    lng_e6        INTEGER,
    seen_at       INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER
);

-- A tour: a vehicle, a driver, a day, its stops in order.
CREATE TABLE tours (
    id                  TEXT PRIMARY KEY,
    org_id              TEXT NOT NULL REFERENCES organizations(id),
    branch_id           TEXT NOT NULL REFERENCES branches(id),
    number              TEXT NOT NULL,
    vehicle_id          TEXT NOT NULL REFERENCES vehicles(id),
    driver_id           TEXT REFERENCES users(id),
    day                 TEXT NOT NULL,
    kind                TEXT NOT NULL DEFAULT 'delivery'
                        CHECK (kind IN ('delivery', 'van', 'presell')),
    status              TEXT NOT NULL DEFAULT 'planned'
                        CHECK (status IN ('planned', 'on_road', 'done', 'cancelled')),
    started_at          INTEGER,
    ended_at            INTEGER,
    km_start            INTEGER,
    km_end              INTEGER,
    cash_expected_minor INTEGER NOT NULL DEFAULT 0,
    cash_counted_minor  INTEGER,
    note                TEXT NOT NULL DEFAULT '',
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    created_by          TEXT,
    device_id           TEXT NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1,
    deleted_at          INTEGER
);
CREATE INDEX tours_day ON tours(org_id, day);
CREATE INDEX tours_vehicle ON tours(vehicle_id, status);

CREATE TABLE tour_stops (
    id              TEXT PRIMARY KEY,
    tour_id         TEXT NOT NULL REFERENCES tours(id),
    seq             INTEGER NOT NULL,
    customer_id     TEXT NOT NULL REFERENCES customers(id),
    document_id     TEXT,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'arrived', 'done', 'failed', 'skipped')),
    arrived_at      INTEGER,
    done_at         INTEGER,
    fail_reason     TEXT NOT NULL DEFAULT '',
    receiver        TEXT NOT NULL DEFAULT '',
    collected_minor INTEGER NOT NULL DEFAULT 0 CHECK (collected_minor >= 0),
    lat_e6          INTEGER,
    lng_e6          INTEGER,
    notified_at     INTEGER,
    note            TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX tour_stops_tour ON tour_stops(tour_id, seq);
CREATE INDEX tour_stops_document ON tour_stops(document_id);

-- Where a vehicle has been (append-only).
CREATE TABLE vehicle_positions (
    id          TEXT PRIMARY KEY,
    vehicle_id  TEXT NOT NULL REFERENCES vehicles(id),
    tour_id     TEXT REFERENCES tours(id),
    lat_e6      INTEGER NOT NULL,
    lng_e6      INTEGER NOT NULL,
    speed_kmh   INTEGER,
    at          INTEGER NOT NULL
);
CREATE INDEX vehicle_positions_vehicle ON vehicle_positions(vehicle_id, at);
