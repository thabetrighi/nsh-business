-- Stock (M6): movements are append-only; balances are their projection,
-- updated in the same transaction and rebuildable. Quantities are stored
-- exactly as thousandths of the base unit (`*_milli`), since a base
-- quantity has at most 3 decimals.

CREATE TABLE stock_movements (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    id            TEXT NOT NULL UNIQUE,
    org_id        TEXT NOT NULL,
    warehouse_id  TEXT NOT NULL REFERENCES warehouses(id),
    product_id    TEXT NOT NULL REFERENCES products(id),
    kind          TEXT NOT NULL,
    qty_milli     INTEGER NOT NULL,
    unit_cost_minor INTEGER,
    doc_number    TEXT,
    reason        TEXT,
    operation_id  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL
);
CREATE INDEX stock_movements_item ON stock_movements(product_id, warehouse_id, seq);
CREATE TRIGGER stock_movements_no_update BEFORE UPDATE ON stock_movements
BEGIN SELECT RAISE(ABORT, 'stock_movements is append-only'); END;
CREATE TRIGGER stock_movements_no_delete BEFORE DELETE ON stock_movements
BEGIN SELECT RAISE(ABORT, 'stock_movements is append-only'); END;

CREATE TABLE stock_balances (
    product_id    TEXT NOT NULL REFERENCES products(id),
    warehouse_id  TEXT NOT NULL REFERENCES warehouses(id),
    on_hand_milli INTEGER NOT NULL,
    avg_cost_minor INTEGER NOT NULL,
    last_seq      INTEGER NOT NULL,
    PRIMARY KEY (product_id, warehouse_id)
);
