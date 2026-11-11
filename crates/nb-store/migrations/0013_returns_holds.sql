-- Returns tied to their sale (M8.8) and carts put aside at the till (M8.7).

CREATE TABLE sale_returns (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    branch_id       TEXT NOT NULL REFERENCES branches(id),
    sale_id         TEXT NOT NULL REFERENCES sales(id),
    register_id     TEXT REFERENCES cash_registers(id),
    shift_id        TEXT REFERENCES shifts(id),
    number          TEXT NOT NULL,
    refund_method   TEXT NOT NULL CHECK (refund_method IN ('cash', 'card', 'transfer', 'credit')),
    reason          TEXT NOT NULL,
    net_minor       INTEGER NOT NULL,
    tax_minor       INTEGER NOT NULL,
    total_minor     INTEGER NOT NULL,
    cost_minor      INTEGER NOT NULL,
    operation_id    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    created_by      TEXT,
    device_id       TEXT NOT NULL,
    UNIQUE (org_id, number)
);
CREATE INDEX sale_returns_sale ON sale_returns (sale_id);
CREATE TRIGGER sale_returns_no_update BEFORE UPDATE ON sale_returns
BEGIN SELECT RAISE(ABORT, 'returns are immutable'); END;
CREATE TRIGGER sale_returns_no_delete BEFORE DELETE ON sale_returns
BEGIN SELECT RAISE(ABORT, 'returns are immutable'); END;

CREATE TABLE sale_return_lines (
    return_id       TEXT NOT NULL REFERENCES sale_returns(id),
    sale_id         TEXT NOT NULL REFERENCES sales(id),
    position        INTEGER NOT NULL,
    qty_milli       INTEGER NOT NULL CHECK (qty_milli > 0),
    net_minor       INTEGER NOT NULL,
    tax_minor       INTEGER NOT NULL,
    cost_minor      INTEGER NOT NULL,
    restocked       INTEGER NOT NULL,
    movement_id     TEXT,
    PRIMARY KEY (return_id, position)
);
CREATE INDEX sale_return_lines_sale ON sale_return_lines (sale_id, position);
CREATE TRIGGER sale_return_lines_no_update BEFORE UPDATE ON sale_return_lines
BEGIN SELECT RAISE(ABORT, 'returns are immutable'); END;
CREATE TRIGGER sale_return_lines_no_delete BEFORE DELETE ON sale_return_lines
BEGIN SELECT RAISE(ABORT, 'returns are immutable'); END;

-- A cart put aside: what was in it, as the till left it (not financial: it
-- is taken back, then gone).
CREATE TABLE held_carts (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    register_id   TEXT NOT NULL REFERENCES cash_registers(id),
    label         TEXT NOT NULL,
    lines         INTEGER NOT NULL,
    total_minor   INTEGER NOT NULL,
    payload       TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    created_by    TEXT
);
CREATE INDEX held_carts_register ON held_carts (register_id, created_at);
