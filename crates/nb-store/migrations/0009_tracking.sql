-- Batches with expiry dates and serial numbers (M6.8, M6.9). A product is
-- tracked by nothing, by lot, or by serial. A movement keeps its product
-- quantity as before; which lots and serials it moved is recorded beside
-- it, append-only, and lot balances are their projection.

ALTER TABLE products ADD COLUMN tracking TEXT NOT NULL DEFAULT 'none'
    CHECK (tracking IN ('none', 'lot', 'serial'));

CREATE TABLE lots (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    product_id    TEXT NOT NULL REFERENCES products(id),
    code          TEXT NOT NULL,
    -- ISO date (YYYY-MM-DD), or none for lots without expiry.
    expiry        TEXT,
    created_at    INTEGER NOT NULL,
    UNIQUE (product_id, code)
);

CREATE TABLE lot_balances (
    lot_id        TEXT NOT NULL REFERENCES lots(id),
    warehouse_id  TEXT NOT NULL REFERENCES warehouses(id),
    on_hand_milli INTEGER NOT NULL,
    PRIMARY KEY (lot_id, warehouse_id)
);

CREATE TABLE movement_lots (
    movement_id   TEXT NOT NULL REFERENCES stock_movements(id),
    lot_id        TEXT NOT NULL REFERENCES lots(id),
    -- Signed like the movement.
    qty_milli     INTEGER NOT NULL,
    PRIMARY KEY (movement_id, lot_id)
);
CREATE TRIGGER movement_lots_no_update BEFORE UPDATE ON movement_lots
BEGIN SELECT RAISE(ABORT, 'movement_lots is append-only'); END;
CREATE TRIGGER movement_lots_no_delete BEFORE DELETE ON movement_lots
BEGIN SELECT RAISE(ABORT, 'movement_lots is append-only'); END;

CREATE TABLE serials (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    product_id    TEXT NOT NULL REFERENCES products(id),
    serial        TEXT NOT NULL,
    -- Where it is now; none once it left the stock.
    warehouse_id  TEXT REFERENCES warehouses(id),
    status        TEXT NOT NULL CHECK (status IN ('in_stock', 'out')),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    UNIQUE (product_id, serial)
);

CREATE TABLE movement_serials (
    movement_id   TEXT NOT NULL REFERENCES stock_movements(id),
    serial_id     TEXT NOT NULL REFERENCES serials(id),
    PRIMARY KEY (movement_id, serial_id)
);
CREATE TRIGGER movement_serials_no_update BEFORE UPDATE ON movement_serials
BEGIN SELECT RAISE(ABORT, 'movement_serials is append-only'); END;
CREATE TRIGGER movement_serials_no_delete BEFORE DELETE ON movement_serials
BEGIN SELECT RAISE(ABORT, 'movement_serials is append-only'); END;

-- The movement that sent each transfer line, so receiving brings in the
-- same lots and serials.
ALTER TABLE transfer_lines ADD COLUMN out_movement_id TEXT REFERENCES stock_movements(id);
