-- Money received from customers (M9.4): the receipt, and how it settles
-- their invoices on account (payment_allocations). Both are records of
-- what happened: never changed, never deleted (a mistake is reversed by
-- its entry).

CREATE TABLE receipts (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    branch_id       TEXT NOT NULL REFERENCES branches(id),
    customer_id     TEXT NOT NULL REFERENCES customers(id),
    number          TEXT NOT NULL,
    method          TEXT NOT NULL CHECK (method IN ('cash', 'card', 'transfer')),
    -- Cash goes into an open till.
    register_id     TEXT REFERENCES cash_registers(id),
    shift_id        TEXT REFERENCES shifts(id),
    amount_minor    INTEGER NOT NULL CHECK (amount_minor > 0),
    -- What no invoice took (an advance on the next ones).
    unallocated_minor INTEGER NOT NULL CHECK (unallocated_minor >= 0),
    reference       TEXT NOT NULL DEFAULT '',
    note            TEXT NOT NULL DEFAULT '',
    operation_id    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    created_by      TEXT,
    device_id       TEXT NOT NULL,
    UNIQUE (org_id, number)
);
CREATE INDEX receipts_customer ON receipts (customer_id, created_at);
CREATE INDEX receipts_shift ON receipts (shift_id) WHERE shift_id IS NOT NULL;
CREATE TRIGGER receipts_no_update BEFORE UPDATE ON receipts
BEGIN SELECT RAISE(ABORT, 'receipts are immutable'); END;
CREATE TRIGGER receipts_no_delete BEFORE DELETE ON receipts
BEGIN SELECT RAISE(ABORT, 'receipts are immutable'); END;

CREATE TABLE payment_allocations (
    receipt_id      TEXT NOT NULL REFERENCES receipts(id),
    sale_id         TEXT NOT NULL REFERENCES sales(id),
    amount_minor    INTEGER NOT NULL CHECK (amount_minor > 0),
    PRIMARY KEY (receipt_id, sale_id)
);
CREATE INDEX payment_allocations_sale ON payment_allocations (sale_id);
CREATE TRIGGER payment_allocations_no_update BEFORE UPDATE ON payment_allocations
BEGIN SELECT RAISE(ABORT, 'allocations are immutable'); END;
CREATE TRIGGER payment_allocations_no_delete BEFORE DELETE ON payment_allocations
BEGIN SELECT RAISE(ABORT, 'allocations are immutable'); END;
