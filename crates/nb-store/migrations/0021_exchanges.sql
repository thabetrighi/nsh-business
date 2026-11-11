-- An exchange (M8.8): goods brought back and a new sale, in one operation.
-- The value of what came back pays the new sale; the part of it that the
-- new sale used is kept here, so the till shows it apart from cash
-- refunds and cash sales.

CREATE TABLE sale_exchanges (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    return_id       TEXT NOT NULL UNIQUE REFERENCES sale_returns(id),
    sale_id         TEXT NOT NULL UNIQUE REFERENCES sales(id),
    shift_id        TEXT NOT NULL REFERENCES shifts(id),
    -- What came back, and how much of it the new sale used.
    returned_minor  INTEGER NOT NULL CHECK (returned_minor >= 0),
    used_minor      INTEGER NOT NULL CHECK (used_minor >= 0 AND used_minor <= returned_minor),
    operation_id    TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX sale_exchanges_shift ON sale_exchanges (shift_id);
CREATE TRIGGER sale_exchanges_no_update BEFORE UPDATE ON sale_exchanges
BEGIN SELECT RAISE(ABORT, 'exchanges are immutable'); END;
CREATE TRIGGER sale_exchanges_no_delete BEFORE DELETE ON sale_exchanges
BEGIN SELECT RAISE(ABORT, 'exchanges are immutable'); END;
