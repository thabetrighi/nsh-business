-- Customers (M9.1) and their sub-ledger (M9.2).
--
-- A customer's debt is not a figure kept by hand: it is the sum of the
-- receivable lines of the entries tagged with that customer (party_id),
-- set when the entry is posted. `balance_minor` is its projection, kept in
-- the same transaction and rebuilt from the lines.

CREATE TABLE customer_groups (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    name            TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    UNIQUE (org_id, name)
);

CREATE TABLE customers (
    id                  TEXT PRIMARY KEY,
    org_id              TEXT NOT NULL REFERENCES organizations(id),
    code                TEXT NOT NULL,
    kind                TEXT NOT NULL CHECK (kind IN ('person', 'company')),
    name                TEXT NOT NULL,
    group_id            TEXT REFERENCES customer_groups(id),
    phone               TEXT NOT NULL DEFAULT '',
    email               TEXT NOT NULL DEFAULT '',
    address             TEXT NOT NULL DEFAULT '',
    city                TEXT NOT NULL DEFAULT '',
    -- Algerian identifiers (companies): NIF, NIS, RC, AI.
    nif                 TEXT NOT NULL DEFAULT '',
    nis                 TEXT NOT NULL DEFAULT '',
    rc                  TEXT NOT NULL DEFAULT '',
    ai                  TEXT NOT NULL DEFAULT '',
    -- What may be owed without a supervisor (0: no credit).
    credit_limit_minor  INTEGER NOT NULL DEFAULT 0 CHECK (credit_limit_minor >= 0),
    -- Days to pay an invoice on credit.
    payment_days        INTEGER NOT NULL DEFAULT 0 CHECK (payment_days >= 0),
    status              TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'blocked')),
    note                TEXT NOT NULL DEFAULT '',
    balance_minor       INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    created_by          TEXT,
    updated_at          INTEGER NOT NULL,
    device_id           TEXT NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1,
    UNIQUE (org_id, code)
);
CREATE INDEX customers_name ON customers (org_id, name);

-- The party of a receivable line: who owes it.
ALTER TABLE journal_lines ADD COLUMN party_id TEXT;
CREATE INDEX journal_lines_party ON journal_lines (party_id) WHERE party_id IS NOT NULL;

-- Customers numbered C-00001 per organization.
CREATE TABLE customer_counters (
    org_id  TEXT PRIMARY KEY REFERENCES organizations(id),
    last    INTEGER NOT NULL
);
