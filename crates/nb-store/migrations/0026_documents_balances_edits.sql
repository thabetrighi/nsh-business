-- What the shop's documents show (M12.8), the customer's balance as each
-- sale left it ("bon par bon"), and receipts corrected at the till.

-- One row per organization: the document options and the logo (a grey
-- PNG, at most 576 dots wide, made by the app).
CREATE TABLE document_settings (
    org_id      TEXT PRIMARY KEY REFERENCES organizations(id),
    tagline     TEXT NOT NULL DEFAULT '',
    footer      TEXT NOT NULL DEFAULT '',
    notice      TEXT NOT NULL DEFAULT '',
    legal_ids   INTEGER NOT NULL DEFAULT 1 CHECK (legal_ids IN (0, 1)),
    vat         INTEGER NOT NULL DEFAULT 1 CHECK (vat IN (0, 1)),
    cashier     INTEGER NOT NULL DEFAULT 1 CHECK (cashier IN (0, 1)),
    table_lines INTEGER NOT NULL DEFAULT 1 CHECK (table_lines IN (0, 1)),
    item_count  INTEGER NOT NULL DEFAULT 1 CHECK (item_count IN (0, 1)),
    balance     INTEGER NOT NULL DEFAULT 1 CHECK (balance IN (0, 1)),
    barcode     INTEGER NOT NULL DEFAULT 1 CHECK (barcode IN (0, 1)),
    logo        BLOB,
    updated_at  INTEGER NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1
);

-- A customer's balance before and after a sale, and what of the payment
-- settled older debt (a receipt written with the sale).
CREATE TABLE sale_balances (
    sale_id        TEXT PRIMARY KEY REFERENCES sales(id),
    customer_id    TEXT NOT NULL REFERENCES customers(id),
    before_minor   INTEGER NOT NULL,
    after_minor    INTEGER NOT NULL,
    settled_minor  INTEGER NOT NULL DEFAULT 0 CHECK (settled_minor >= 0),
    receipt_id     TEXT REFERENCES receipts(id)
);
CREATE TRIGGER sale_balances_no_update BEFORE UPDATE ON sale_balances
BEGIN SELECT RAISE(ABORT, 'sale balances are immutable'); END;
CREATE TRIGGER sale_balances_no_delete BEFORE DELETE ON sale_balances
BEGIN SELECT RAISE(ABORT, 'sale balances are immutable'); END;

-- A receipt corrected: what was left of the original came back (a return)
-- and the corrected sale replaced it, in one operation. A sale is
-- corrected once; a later correction starts from the new one.
CREATE TABLE sale_edits (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    original_id   TEXT NOT NULL UNIQUE REFERENCES sales(id),
    return_id     TEXT NOT NULL UNIQUE REFERENCES sale_returns(id),
    sale_id       TEXT NOT NULL UNIQUE REFERENCES sales(id),
    reason        TEXT NOT NULL DEFAULT '',
    operation_id  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    created_by    TEXT
);
CREATE TRIGGER sale_edits_no_update BEFORE UPDATE ON sale_edits
BEGIN SELECT RAISE(ABORT, 'sale edits are immutable'); END;
CREATE TRIGGER sale_edits_no_delete BEFORE DELETE ON sale_edits
BEGIN SELECT RAISE(ABORT, 'sale edits are immutable'); END;
