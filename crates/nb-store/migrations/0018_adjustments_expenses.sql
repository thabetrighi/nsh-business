-- Debts given up or adjusted (M9.9), and expenses and other income
-- (M9.11), each posted to the books in the same transaction.

-- The charges and income a shop records, in the proposed chart (SCF).
WITH chart(code, name, parent, kind, grp) AS (VALUES
    ('607', 'Achats non stockés (eau, énergie)', '60', 'expense', 0),
    ('613', 'Locations', '61', 'expense', 0),
    ('615', 'Entretien et réparations', '61', 'expense', 0),
    ('616', 'Primes d''assurance', '61', 'expense', 0),
    ('624', 'Transports', '62', 'expense', 0),
    ('626', 'Frais postaux et de télécommunications', '62', 'expense', 0),
    ('627', 'Services bancaires', '62', 'expense', 0),
    ('63', 'Charges de personnel', '6', 'expense', 1),
    ('631', 'Rémunérations du personnel', '63', 'expense', 0),
    ('64', 'Impôts, taxes et versements assimilés', '6', 'expense', 1),
    ('642', 'Impôts et taxes non récupérables', '64', 'expense', 0),
    ('654', 'Pertes sur créances irrécouvrables', '65', 'expense', 0),
    ('752', 'Revenus des immeubles (loyers perçus)', '75', 'income', 0)
)
INSERT OR IGNORE INTO accounts (org_id, code, name, parent_code, kind, is_group)
SELECT o.org_id, c.code, c.name, c.parent, c.kind, c.grp
FROM (SELECT DISTINCT org_id FROM accounts WHERE code = '531') o, chart c;

-- Services become headings of their accounts (when nothing was posted to them).
UPDATE accounts SET is_group = 1
WHERE code IN ('61', '62') AND org_id IN (SELECT org_id FROM accounts WHERE code = '531')
  AND NOT EXISTS (SELECT 1 FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
                  WHERE e.org_id = accounts.org_id AND l.account_code = accounts.code);

INSERT OR IGNORE INTO account_roles (org_id, role, account_code)
SELECT DISTINCT org_id, 'bad_debt', '654' FROM account_roles;

-- What a shop spends on (or earns besides sales), each on its account.
CREATE TABLE expense_categories (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    kind            TEXT NOT NULL CHECK (kind IN ('expense', 'income')),
    name            TEXT NOT NULL,
    account_code    TEXT NOT NULL,
    active          INTEGER NOT NULL DEFAULT 1,
    created_at      INTEGER NOT NULL,
    UNIQUE (org_id, kind, name)
);

WITH seed(kind, name, code) AS (VALUES
    ('expense', 'Loyer', '613'),
    ('expense', 'Électricité, eau et gaz', '607'),
    ('expense', 'Salaires', '631'),
    ('expense', 'Transport', '624'),
    ('expense', 'Téléphone et internet', '626'),
    ('expense', 'Entretien et réparations', '615'),
    ('expense', 'Frais bancaires', '627'),
    ('expense', 'Impôts et taxes', '642'),
    ('expense', 'Divers', '658'),
    ('income', 'Loyers perçus', '752'),
    ('income', 'Autres produits', '758')
)
INSERT OR IGNORE INTO expense_categories (id, org_id, kind, name, account_code, created_at)
SELECT lower(hex(randomblob(16))), o.org_id, s.kind, s.name, s.code, 0
FROM (SELECT DISTINCT org_id FROM accounts WHERE code = '531') o, seed s;

CREATE TABLE expenses (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    branch_id       TEXT NOT NULL REFERENCES branches(id),
    category_id     TEXT NOT NULL REFERENCES expense_categories(id),
    kind            TEXT NOT NULL CHECK (kind IN ('expense', 'income')),
    number          TEXT NOT NULL,
    method          TEXT NOT NULL CHECK (method IN ('cash', 'bank')),
    register_id     TEXT REFERENCES cash_registers(id),
    shift_id        TEXT REFERENCES shifts(id),
    net_minor       INTEGER NOT NULL CHECK (net_minor > 0),
    vat_minor       INTEGER NOT NULL CHECK (vat_minor >= 0),
    total_minor     INTEGER NOT NULL,
    payee           TEXT NOT NULL DEFAULT '',
    reference       TEXT NOT NULL DEFAULT '',
    note            TEXT NOT NULL DEFAULT '',
    entry_number    TEXT,
    operation_id    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    created_by      TEXT,
    device_id       TEXT NOT NULL,
    UNIQUE (org_id, number)
);
CREATE INDEX expenses_date ON expenses (org_id, created_at);
CREATE INDEX expenses_shift ON expenses (shift_id) WHERE shift_id IS NOT NULL;
CREATE TRIGGER expenses_no_update BEFORE UPDATE ON expenses
BEGIN SELECT RAISE(ABORT, 'expenses are immutable'); END;
CREATE TRIGGER expenses_no_delete BEFORE DELETE ON expenses
BEGIN SELECT RAISE(ABORT, 'expenses are immutable'); END;

-- A customer's debt given up, an allowance on it, or what it owed before.
CREATE TABLE customer_adjustments (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    customer_id     TEXT NOT NULL REFERENCES customers(id),
    kind            TEXT NOT NULL CHECK (kind IN ('write_off', 'allowance', 'opening')),
    -- Positive: lowers what the customer owes (opening: raises it).
    amount_minor    INTEGER NOT NULL CHECK (amount_minor <> 0),
    reason          TEXT NOT NULL,
    approval_id     TEXT,
    entry_number    TEXT,
    operation_id    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    created_by      TEXT,
    device_id       TEXT NOT NULL
);
CREATE TRIGGER customer_adjustments_no_update BEFORE UPDATE ON customer_adjustments
BEGIN SELECT RAISE(ABORT, 'adjustments are immutable'); END;
CREATE TRIGGER customer_adjustments_no_delete BEFORE DELETE ON customer_adjustments
BEGIN SELECT RAISE(ABORT, 'adjustments are immutable'); END;

-- How a write-off or an allowance settles the invoices, oldest first.
CREATE TABLE adjustment_allocations (
    adjustment_id   TEXT NOT NULL REFERENCES customer_adjustments(id),
    sale_id         TEXT NOT NULL REFERENCES sales(id),
    amount_minor    INTEGER NOT NULL CHECK (amount_minor > 0),
    PRIMARY KEY (adjustment_id, sale_id)
);
CREATE TRIGGER adjustment_allocations_no_update BEFORE UPDATE ON adjustment_allocations
BEGIN SELECT RAISE(ABORT, 'allocations are immutable'); END;
CREATE TRIGGER adjustment_allocations_no_delete BEFORE DELETE ON adjustment_allocations
BEGIN SELECT RAISE(ABORT, 'allocations are immutable'); END;

-- What an adjustment took off an invoice paid in instalments.
CREATE TABLE installment_adjustments (
    adjustment_id   TEXT NOT NULL REFERENCES customer_adjustments(id),
    plan_id         TEXT NOT NULL,
    seq             INTEGER NOT NULL,
    amount_minor    INTEGER NOT NULL CHECK (amount_minor > 0),
    PRIMARY KEY (adjustment_id, plan_id, seq),
    FOREIGN KEY (plan_id, seq) REFERENCES installments (plan_id, seq)
);
CREATE TRIGGER installment_adjustments_no_update BEFORE UPDATE ON installment_adjustments
BEGIN SELECT RAISE(ABORT, 'adjustments are immutable'); END;
CREATE TRIGGER installment_adjustments_no_delete BEFORE DELETE ON installment_adjustments
BEGIN SELECT RAISE(ABORT, 'adjustments are immutable'); END;
