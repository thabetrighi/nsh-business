-- Purchase orders (M10.1) and the goods received against them (M10.2,
-- M10.3). A draft changes freely; an approved order keeps its contents
-- and only its lines' received quantities move, by the purchase invoices
-- that receive them; a received, closed or cancelled order never changes.

CREATE TABLE purchase_orders (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    branch_id       TEXT NOT NULL REFERENCES branches(id),
    warehouse_id    TEXT NOT NULL REFERENCES warehouses(id),
    supplier_id     TEXT NOT NULL REFERENCES suppliers(id),
    number          TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('draft', 'approved', 'partial', 'received', 'closed', 'cancelled')),
    -- `YYYY-MM-DD`: when the goods are expected.
    expected_date   TEXT,
    net_minor       INTEGER NOT NULL,
    vat_minor       INTEGER NOT NULL,
    total_minor     INTEGER NOT NULL,
    note            TEXT NOT NULL DEFAULT '',
    -- Why it was cancelled or closed.
    reason          TEXT NOT NULL DEFAULT '',
    version         INTEGER NOT NULL DEFAULT 1,
    created_at      INTEGER NOT NULL,
    created_by      TEXT,
    updated_at      INTEGER NOT NULL,
    approved_at     INTEGER,
    approved_by     TEXT,
    UNIQUE (org_id, number)
);
CREATE INDEX purchase_orders_list ON purchase_orders (org_id, status, created_at);
CREATE INDEX purchase_orders_supplier ON purchase_orders (supplier_id, status);

CREATE TRIGGER purchase_orders_final BEFORE UPDATE ON purchase_orders
WHEN OLD.status IN ('received', 'closed', 'cancelled')
BEGIN SELECT RAISE(ABORT, 'this order is final'); END;
CREATE TRIGGER purchase_orders_approved BEFORE UPDATE ON purchase_orders
WHEN OLD.status <> 'draft' AND (
    NEW.total_minor IS NOT OLD.total_minor OR NEW.supplier_id IS NOT OLD.supplier_id
    OR NEW.warehouse_id IS NOT OLD.warehouse_id OR NEW.version IS NOT OLD.version)
BEGIN SELECT RAISE(ABORT, 'an approved order keeps its contents'); END;
CREATE TRIGGER purchase_orders_no_delete BEFORE DELETE ON purchase_orders
BEGIN SELECT RAISE(ABORT, 'orders are cancelled, not deleted'); END;

CREATE TABLE purchase_order_lines (
    order_id        TEXT NOT NULL REFERENCES purchase_orders(id),
    position        INTEGER NOT NULL,
    product_id      TEXT NOT NULL REFERENCES products(id),
    name            TEXT NOT NULL,
    unit            TEXT NOT NULL,
    qty_milli       INTEGER NOT NULL CHECK (qty_milli > 0),
    unit_cost_minor INTEGER NOT NULL CHECK (unit_cost_minor >= 0),
    tax_bps         INTEGER NOT NULL,
    net_minor       INTEGER NOT NULL,
    tax_minor       INTEGER NOT NULL,
    -- In the line's unit, by the invoices that received it.
    received_milli  INTEGER NOT NULL DEFAULT 0 CHECK (received_milli >= 0 AND received_milli <= qty_milli),
    PRIMARY KEY (order_id, position)
);
-- Only a draft's lines come and go; afterwards only what is received moves,
-- and only up.
CREATE TRIGGER purchase_order_lines_insert BEFORE INSERT ON purchase_order_lines
WHEN (SELECT status FROM purchase_orders WHERE id = NEW.order_id) <> 'draft'
BEGIN SELECT RAISE(ABORT, 'only a draft changes'); END;
CREATE TRIGGER purchase_order_lines_delete BEFORE DELETE ON purchase_order_lines
WHEN (SELECT status FROM purchase_orders WHERE id = OLD.order_id) <> 'draft'
BEGIN SELECT RAISE(ABORT, 'only a draft changes'); END;
CREATE TRIGGER purchase_order_lines_update BEFORE UPDATE ON purchase_order_lines
WHEN (SELECT status FROM purchase_orders WHERE id = OLD.order_id) <> 'draft' AND (
    NEW.qty_milli IS NOT OLD.qty_milli OR NEW.unit_cost_minor IS NOT OLD.unit_cost_minor
    OR NEW.product_id IS NOT OLD.product_id OR NEW.unit IS NOT OLD.unit
    OR NEW.received_milli < OLD.received_milli)
BEGIN SELECT RAISE(ABORT, 'an approved order keeps its contents'); END;

-- An invoice line received against an order line, at the cost ordered.
ALTER TABLE purchase_lines ADD COLUMN order_position INTEGER;
ALTER TABLE purchase_lines ADD COLUMN ordered_cost_minor INTEGER;
