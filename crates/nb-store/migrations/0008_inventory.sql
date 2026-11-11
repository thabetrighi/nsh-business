-- Inventory (M6): reservations, transfers between warehouses, stock counts.

-- Quantities held for an order or a cart (M6.4): available = on hand -
-- active reservations.
CREATE TABLE stock_reservations (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    product_id    TEXT NOT NULL REFERENCES products(id),
    warehouse_id  TEXT NOT NULL REFERENCES warehouses(id),
    qty_milli     INTEGER NOT NULL CHECK (qty_milli > 0),
    source        TEXT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('active', 'released', 'consumed')),
    created_at    INTEGER NOT NULL,
    created_by    TEXT,
    closed_at     INTEGER
);
CREATE INDEX stock_reservations_active ON stock_reservations(product_id, warehouse_id) WHERE status = 'active';

-- Transfers (M6.5): sent (goods in transit), then received (differences
-- recorded as losses).
CREATE TABLE transfers (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    number        TEXT NOT NULL UNIQUE,
    from_warehouse TEXT NOT NULL REFERENCES warehouses(id),
    to_warehouse  TEXT NOT NULL REFERENCES warehouses(id),
    status        TEXT NOT NULL CHECK (status IN ('sent', 'received')),
    note          TEXT NOT NULL DEFAULT '',
    sent_at       INTEGER NOT NULL,
    sent_by       TEXT,
    received_at   INTEGER,
    received_by   TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    CHECK (from_warehouse != to_warehouse)
);

CREATE TABLE transfer_lines (
    transfer_id   TEXT NOT NULL REFERENCES transfers(id),
    position      INTEGER NOT NULL,
    product_id    TEXT NOT NULL REFERENCES products(id),
    qty_sent_milli INTEGER NOT NULL CHECK (qty_sent_milli > 0),
    qty_received_milli INTEGER,
    unit_cost_minor INTEGER NOT NULL,
    PRIMARY KEY (transfer_id, position)
);

-- Stock counts (M6.7): counted quantities, then one closing that books the
-- differences.
CREATE TABLE stock_counts (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    number        TEXT NOT NULL UNIQUE,
    warehouse_id  TEXT NOT NULL REFERENCES warehouses(id),
    category_id   TEXT REFERENCES categories(id),
    status        TEXT NOT NULL CHECK (status IN ('open', 'closed', 'cancelled')),
    note          TEXT NOT NULL DEFAULT '',
    opened_at     INTEGER NOT NULL,
    opened_by     TEXT,
    closed_at     INTEGER,
    closed_by     TEXT,
    device_id     TEXT NOT NULL
);
CREATE UNIQUE INDEX stock_counts_open ON stock_counts(warehouse_id) WHERE status = 'open';

CREATE TABLE stock_count_lines (
    count_id      TEXT NOT NULL REFERENCES stock_counts(id),
    product_id    TEXT NOT NULL REFERENCES products(id),
    counted_milli INTEGER NOT NULL CHECK (counted_milli >= 0),
    book_milli    INTEGER,
    counted_at    INTEGER NOT NULL,
    counted_by    TEXT,
    PRIMARY KEY (count_id, product_id)
);
