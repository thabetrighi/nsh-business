-- Suppliers and what is owed to them (M9.10), purchase invoices that bring
-- goods in at their landed cost (M10.2–M10.5), returns to suppliers
-- (M10.6) and payments to them (M10.7). Everything posted is kept as it
-- was; a supplier's balance is the projection of its payable lines.

CREATE TABLE suppliers (
    id                  TEXT PRIMARY KEY,
    org_id              TEXT NOT NULL REFERENCES organizations(id),
    code                TEXT NOT NULL,
    kind                TEXT NOT NULL CHECK (kind IN ('person', 'company')),
    name                TEXT NOT NULL,
    phone               TEXT NOT NULL DEFAULT '',
    email               TEXT NOT NULL DEFAULT '',
    address             TEXT NOT NULL DEFAULT '',
    city                TEXT NOT NULL DEFAULT '',
    nif                 TEXT NOT NULL DEFAULT '',
    nis                 TEXT NOT NULL DEFAULT '',
    rc                  TEXT NOT NULL DEFAULT '',
    ai                  TEXT NOT NULL DEFAULT '',
    payment_days        INTEGER NOT NULL DEFAULT 0 CHECK (payment_days >= 0),
    status              TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'blocked')),
    note                TEXT NOT NULL DEFAULT '',
    -- What is owed to the supplier (negative: an advance paid to it).
    balance_minor       INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    created_by          TEXT,
    updated_at          INTEGER NOT NULL,
    device_id           TEXT NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1,
    UNIQUE (org_id, code)
);
CREATE INDEX suppliers_name ON suppliers (org_id, name);
CREATE TABLE supplier_counters (
    org_id  TEXT PRIMARY KEY REFERENCES organizations(id),
    last    INTEGER NOT NULL
);

CREATE TABLE purchases (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    branch_id       TEXT NOT NULL REFERENCES branches(id),
    warehouse_id    TEXT NOT NULL REFERENCES warehouses(id),
    supplier_id     TEXT NOT NULL REFERENCES suppliers(id),
    number          TEXT NOT NULL,
    -- The supplier's own invoice number.
    supplier_ref    TEXT NOT NULL DEFAULT '',
    goods_minor     INTEGER NOT NULL,
    -- Transport and other costs of bringing the goods in, spread on them.
    charges_minor   INTEGER NOT NULL DEFAULT 0,
    -- 'value' or 'qty': how the charges are spread.
    charges_by      TEXT NOT NULL DEFAULT 'value' CHECK (charges_by IN ('value', 'qty')),
    net_minor       INTEGER NOT NULL,
    vat_minor       INTEGER NOT NULL,
    total_minor     INTEGER NOT NULL,
    cash_minor      INTEGER NOT NULL DEFAULT 0,
    bank_minor      INTEGER NOT NULL DEFAULT 0,
    credit_minor    INTEGER NOT NULL DEFAULT 0,
    register_id     TEXT REFERENCES cash_registers(id),
    shift_id        TEXT REFERENCES shifts(id),
    order_id        TEXT,
    note            TEXT NOT NULL DEFAULT '',
    operation_id    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    created_by      TEXT,
    device_id       TEXT NOT NULL,
    UNIQUE (org_id, number)
);
CREATE INDEX purchases_supplier ON purchases (supplier_id, created_at);
CREATE INDEX purchases_shift ON purchases (shift_id) WHERE shift_id IS NOT NULL;
CREATE TRIGGER purchases_no_update BEFORE UPDATE ON purchases
BEGIN SELECT RAISE(ABORT, 'purchases are immutable'); END;
CREATE TRIGGER purchases_no_delete BEFORE DELETE ON purchases
BEGIN SELECT RAISE(ABORT, 'purchases are immutable'); END;

CREATE TABLE purchase_lines (
    purchase_id     TEXT NOT NULL REFERENCES purchases(id),
    position        INTEGER NOT NULL,
    product_id      TEXT NOT NULL REFERENCES products(id),
    name            TEXT NOT NULL,
    unit            TEXT NOT NULL,
    qty_milli       INTEGER NOT NULL CHECK (qty_milli > 0),
    base_qty_milli  INTEGER NOT NULL,
    -- As invoiced, before tax, per unit of the line.
    unit_cost_minor INTEGER NOT NULL,
    tax_bps         INTEGER NOT NULL,
    net_minor       INTEGER NOT NULL,
    tax_minor       INTEGER NOT NULL,
    -- Its share of the charges, and what one base unit cost landed.
    charges_minor   INTEGER NOT NULL,
    landed_cost_minor INTEGER NOT NULL,
    movement_id     TEXT,
    PRIMARY KEY (purchase_id, position)
);
CREATE TRIGGER purchase_lines_no_update BEFORE UPDATE ON purchase_lines
BEGIN SELECT RAISE(ABORT, 'purchases are immutable'); END;
CREATE TRIGGER purchase_lines_no_delete BEFORE DELETE ON purchase_lines
BEGIN SELECT RAISE(ABORT, 'purchases are immutable'); END;

CREATE TABLE purchase_returns (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    purchase_id     TEXT NOT NULL REFERENCES purchases(id),
    number          TEXT NOT NULL,
    refund_method   TEXT NOT NULL CHECK (refund_method IN ('cash', 'bank', 'credit')),
    reason          TEXT NOT NULL,
    net_minor       INTEGER NOT NULL,
    tax_minor       INTEGER NOT NULL,
    total_minor     INTEGER NOT NULL,
    stock_value_minor INTEGER NOT NULL,
    shift_id        TEXT REFERENCES shifts(id),
    operation_id    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    created_by      TEXT,
    device_id       TEXT NOT NULL,
    UNIQUE (org_id, number)
);
CREATE TRIGGER purchase_returns_no_update BEFORE UPDATE ON purchase_returns
BEGIN SELECT RAISE(ABORT, 'returns are immutable'); END;
CREATE TRIGGER purchase_returns_no_delete BEFORE DELETE ON purchase_returns
BEGIN SELECT RAISE(ABORT, 'returns are immutable'); END;

CREATE TABLE purchase_return_lines (
    return_id       TEXT NOT NULL REFERENCES purchase_returns(id),
    purchase_id     TEXT NOT NULL REFERENCES purchases(id),
    position        INTEGER NOT NULL,
    qty_milli       INTEGER NOT NULL CHECK (qty_milli > 0),
    net_minor       INTEGER NOT NULL,
    tax_minor       INTEGER NOT NULL,
    stock_value_minor INTEGER NOT NULL,
    movement_id     TEXT,
    PRIMARY KEY (return_id, position)
);
CREATE TRIGGER purchase_return_lines_no_update BEFORE UPDATE ON purchase_return_lines
BEGIN SELECT RAISE(ABORT, 'returns are immutable'); END;
CREATE TRIGGER purchase_return_lines_no_delete BEFORE DELETE ON purchase_return_lines
BEGIN SELECT RAISE(ABORT, 'returns are immutable'); END;

CREATE TABLE supplier_payments (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    branch_id       TEXT NOT NULL REFERENCES branches(id),
    supplier_id     TEXT NOT NULL REFERENCES suppliers(id),
    number          TEXT NOT NULL,
    method          TEXT NOT NULL CHECK (method IN ('cash', 'bank')),
    register_id     TEXT REFERENCES cash_registers(id),
    shift_id        TEXT REFERENCES shifts(id),
    amount_minor    INTEGER NOT NULL CHECK (amount_minor > 0),
    unallocated_minor INTEGER NOT NULL CHECK (unallocated_minor >= 0),
    reference       TEXT NOT NULL DEFAULT '',
    operation_id    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    created_by      TEXT,
    device_id       TEXT NOT NULL,
    UNIQUE (org_id, number)
);
CREATE INDEX supplier_payments_shift ON supplier_payments (shift_id) WHERE shift_id IS NOT NULL;
CREATE TRIGGER supplier_payments_no_update BEFORE UPDATE ON supplier_payments
BEGIN SELECT RAISE(ABORT, 'payments are immutable'); END;
CREATE TRIGGER supplier_payments_no_delete BEFORE DELETE ON supplier_payments
BEGIN SELECT RAISE(ABORT, 'payments are immutable'); END;

CREATE TABLE supplier_allocations (
    payment_id      TEXT NOT NULL REFERENCES supplier_payments(id),
    purchase_id     TEXT NOT NULL REFERENCES purchases(id),
    amount_minor    INTEGER NOT NULL CHECK (amount_minor > 0),
    PRIMARY KEY (payment_id, purchase_id)
);
CREATE TRIGGER supplier_allocations_no_update BEFORE UPDATE ON supplier_allocations
BEGIN SELECT RAISE(ABORT, 'allocations are immutable'); END;
CREATE TRIGGER supplier_allocations_no_delete BEFORE DELETE ON supplier_allocations
BEGIN SELECT RAISE(ABORT, 'allocations are immutable'); END;
