-- Point of sale (M8): till sessions and their cash movements, completed
-- sales with their lines and payments. A sale, once completed, is never
-- edited: a return is a document of its own (M8.8).

-- A till's own ledger account (its money account); none: the cash role's.
ALTER TABLE cash_registers ADD COLUMN money_account_code TEXT;

-- The internal transfers account for money moved into or out of a till.
INSERT OR IGNORE INTO account_roles (org_id, role, account_code)
SELECT DISTINCT org_id, 'internal_transfer', '58' FROM account_roles;

CREATE TABLE shifts (
    id                TEXT PRIMARY KEY,
    org_id            TEXT NOT NULL REFERENCES organizations(id),
    branch_id         TEXT NOT NULL REFERENCES branches(id),
    register_id       TEXT NOT NULL REFERENCES cash_registers(id),
    account_code      TEXT NOT NULL,
    number            TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
    opening_minor     INTEGER NOT NULL CHECK (opening_minor >= 0),
    opened_at         INTEGER NOT NULL,
    opened_by         TEXT,
    closed_at         INTEGER,
    closed_by         TEXT,
    expected_minor    INTEGER,
    counted_minor     INTEGER,
    difference_minor  INTEGER,
    note              TEXT,
    device_id         TEXT NOT NULL
);
-- One open session per till.
CREATE UNIQUE INDEX shifts_one_open ON shifts (register_id) WHERE status = 'open';

CREATE TABLE cash_movements (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    shift_id      TEXT NOT NULL REFERENCES shifts(id),
    direction     TEXT NOT NULL CHECK (direction IN ('in', 'out')),
    amount_minor  INTEGER NOT NULL CHECK (amount_minor > 0),
    reason        TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL
);
CREATE TRIGGER cash_movements_no_update BEFORE UPDATE ON cash_movements
BEGIN SELECT RAISE(ABORT, 'cash_movements is append-only'); END;
CREATE TRIGGER cash_movements_no_delete BEFORE DELETE ON cash_movements
BEGIN SELECT RAISE(ABORT, 'cash_movements is append-only'); END;

CREATE TABLE sales (
    id                TEXT PRIMARY KEY,
    org_id            TEXT NOT NULL REFERENCES organizations(id),
    branch_id         TEXT NOT NULL REFERENCES branches(id),
    warehouse_id      TEXT NOT NULL REFERENCES warehouses(id),
    register_id       TEXT REFERENCES cash_registers(id),
    shift_id          TEXT REFERENCES shifts(id),
    number            TEXT NOT NULL,
    customer_id       TEXT,
    tax_mode          TEXT NOT NULL CHECK (tax_mode IN ('inclusive', 'exclusive')),
    gross_before_minor INTEGER NOT NULL,
    discount_minor    INTEGER NOT NULL,
    net_minor         INTEGER NOT NULL,
    tax_minor         INTEGER NOT NULL,
    stamp_minor       INTEGER NOT NULL DEFAULT 0,
    total_minor       INTEGER NOT NULL,
    cash_minor        INTEGER NOT NULL,
    card_minor        INTEGER NOT NULL,
    transfer_minor    INTEGER NOT NULL,
    credit_minor      INTEGER NOT NULL,
    change_minor      INTEGER NOT NULL,
    cost_minor        INTEGER NOT NULL,
    note              TEXT,
    operation_id      TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    created_by        TEXT,
    device_id         TEXT NOT NULL,
    UNIQUE (org_id, number)
);
CREATE INDEX sales_recent ON sales (org_id, created_at);
CREATE TRIGGER sales_no_update BEFORE UPDATE ON sales
BEGIN SELECT RAISE(ABORT, 'completed sales are immutable'); END;
CREATE TRIGGER sales_no_delete BEFORE DELETE ON sales
BEGIN SELECT RAISE(ABORT, 'completed sales are immutable'); END;

CREATE TABLE sale_lines (
    sale_id               TEXT NOT NULL REFERENCES sales(id),
    position              INTEGER NOT NULL,
    product_id            TEXT NOT NULL REFERENCES products(id),
    name                  TEXT NOT NULL,
    unit                  TEXT NOT NULL,
    qty_milli             INTEGER NOT NULL,
    base_qty_milli        INTEGER NOT NULL,
    unit_price_minor      INTEGER NOT NULL,
    line_discount_minor   INTEGER NOT NULL,
    sale_discount_minor   INTEGER NOT NULL,
    tax_bps               INTEGER NOT NULL,
    net_minor             INTEGER NOT NULL,
    tax_minor             INTEGER NOT NULL,
    gross_minor           INTEGER NOT NULL,
    unit_cost_minor       INTEGER NOT NULL,
    movement_id           TEXT,
    PRIMARY KEY (sale_id, position)
);
CREATE TRIGGER sale_lines_no_update BEFORE UPDATE ON sale_lines
BEGIN SELECT RAISE(ABORT, 'completed sales are immutable'); END;
CREATE TRIGGER sale_lines_no_delete BEFORE DELETE ON sale_lines
BEGIN SELECT RAISE(ABORT, 'completed sales are immutable'); END;

CREATE TABLE payments (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    sale_id       TEXT REFERENCES sales(id),
    shift_id      TEXT REFERENCES shifts(id),
    method        TEXT NOT NULL CHECK (method IN ('cash', 'card', 'transfer', 'credit')),
    amount_minor  INTEGER NOT NULL CHECK (amount_minor > 0),
    account_code  TEXT,
    created_at    INTEGER NOT NULL,
    device_id     TEXT NOT NULL
);
CREATE INDEX payments_shift ON payments (shift_id, method);
CREATE TRIGGER payments_no_update BEFORE UPDATE ON payments
BEGIN SELECT RAISE(ABORT, 'payments are append-only'); END;
CREATE TRIGGER payments_no_delete BEFORE DELETE ON payments
BEGIN SELECT RAISE(ABORT, 'payments are append-only'); END;
