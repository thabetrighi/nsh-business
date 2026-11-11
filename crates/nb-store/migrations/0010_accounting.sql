-- Basic accounting (M7): a hierarchical chart, the accounts each role of
-- the entry matrix uses, the tills and bank accounts, and fiscal years whose
-- closed periods refuse new entries.

ALTER TABLE accounts ADD COLUMN is_group INTEGER NOT NULL DEFAULT 0;
ALTER TABLE accounts ADD COLUMN active INTEGER NOT NULL DEFAULT 1;

-- The proposed Algerian chart (SCF) gains its classes and groups; the
-- organizations that have it are those with account 30.
WITH chart(code, name, parent, kind, grp) AS (VALUES
    ('1', 'Comptes de capitaux', NULL, 'equity', 1),
    ('10', 'Capital, réserves et assimilés', '1', 'equity', 1),
    ('12', 'Résultat de l''exercice', '1', 'equity', 0),
    ('3', 'Comptes de stocks et en-cours', NULL, 'asset', 1),
    ('4', 'Comptes de tiers', NULL, 'liability', 1),
    ('40', 'Fournisseurs et comptes rattachés', '4', 'liability', 1),
    ('41', 'Clients et comptes rattachés', '4', 'asset', 1),
    ('44', 'État, collectivités publiques', '4', 'liability', 1),
    ('445', 'État, taxes sur le chiffre d''affaires', '44', 'liability', 1),
    ('5', 'Comptes financiers', NULL, 'asset', 1),
    ('51', 'Banques, établissements financiers', '5', 'asset', 1),
    ('531', 'Caisse principale', '53', 'asset', 0),
    ('5121', 'Banque principale', '512', 'asset', 0),
    ('6', 'Comptes de charges', NULL, 'expense', 1),
    ('60', 'Achats consommés', '6', 'expense', 1),
    ('65', 'Autres charges opérationnelles', '6', 'expense', 1),
    ('7', 'Comptes de produits', NULL, 'income', 1),
    ('70', 'Ventes de marchandises et produits', '7', 'income', 1),
    ('75', 'Autres produits opérationnels', '7', 'income', 1)
)
INSERT OR IGNORE INTO accounts (org_id, code, name, parent_code, kind, is_group)
SELECT o.org_id, c.code, c.name, c.parent, c.kind, c.grp
FROM (SELECT DISTINCT org_id FROM accounts WHERE code = '30') o, chart c;

WITH parents(code, parent) AS (VALUES
    ('101', '10'), ('30', '3'), ('401', '40'), ('411', '41'), ('4456', '445'),
    ('4457', '445'), ('447', '44'), ('512', '51'), ('53', '5'), ('58', '5'),
    ('600', '60'), ('603', '60'), ('61', '6'), ('62', '6'), ('658', '65'),
    ('700', '70'), ('709', '70'), ('758', '75')
)
UPDATE accounts SET parent_code = (SELECT parent FROM parents p WHERE p.code = accounts.code)
WHERE parent_code IS NULL AND code IN (SELECT code FROM parents)
  AND org_id IN (SELECT org_id FROM accounts WHERE code = '531');

-- The till and the bank become groups of their accounts (none was posted to).
UPDATE accounts SET is_group = 1
WHERE code IN ('512', '53') AND org_id IN (SELECT org_id FROM accounts WHERE code = '531')
  AND NOT EXISTS (SELECT 1 FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
                  WHERE e.org_id = accounts.org_id AND l.account_code = accounts.code);

-- The account each role of the entry matrix posts to.
CREATE TABLE account_roles (
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    role          TEXT NOT NULL,
    account_code  TEXT NOT NULL,
    PRIMARY KEY (org_id, role)
);

WITH roles(role, code) AS (VALUES
    ('cash', '531'), ('bank', '5121'), ('receivable', '411'), ('payable', '401'),
    ('stock', '30'), ('sales', '700'), ('sales_returns', '709'), ('cost_of_sales', '600'),
    ('vat_out', '4457'), ('vat_in', '4456'), ('stamp_duty', '447'), ('opening_equity', '101'),
    ('stock_gain', '758'), ('stock_loss', '658'), ('cash_short', '658'), ('cash_over', '758')
)
INSERT OR IGNORE INTO account_roles (org_id, role, account_code)
SELECT o.org_id, r.role, r.code
FROM (SELECT DISTINCT org_id FROM accounts WHERE code = '531') o, roles r;

-- Tills and bank accounts, each with its own ledger account (M7.7).
CREATE TABLE money_accounts (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    branch_id     TEXT REFERENCES branches(id),
    kind          TEXT NOT NULL CHECK (kind IN ('cash', 'bank')),
    name          TEXT NOT NULL,
    account_code  TEXT NOT NULL,
    active        INTEGER NOT NULL DEFAULT 1,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER,
    UNIQUE (org_id, account_code)
);

WITH seed(kind, name, code) AS (VALUES
    ('cash', 'Caisse principale', '531'), ('bank', 'Banque principale', '5121')
)
INSERT INTO money_accounts (id, org_id, branch_id, kind, name, account_code, created_at, updated_at, device_id)
SELECT lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-7' || substr(hex(randomblob(2)), 2)
             || '-8' || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6))),
       o.org_id, NULL, s.kind, s.name, s.code,
       CAST(strftime('%s', 'now') AS INTEGER) * 1000, CAST(strftime('%s', 'now') AS INTEGER) * 1000,
       'migration'
FROM (SELECT DISTINCT org_id FROM accounts WHERE code = '531') o, seed s;

-- Fiscal years and their monthly periods (M7.2).
CREATE TABLE fiscal_years (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    name          TEXT NOT NULL,
    start_date    TEXT NOT NULL,
    end_date      TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    UNIQUE (org_id, start_date)
);

CREATE TABLE fiscal_periods (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    year_id       TEXT NOT NULL REFERENCES fiscal_years(id),
    number        INTEGER NOT NULL CHECK (number BETWEEN 1 AND 12),
    start_date    TEXT NOT NULL,
    end_date      TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
    closed_at     INTEGER,
    closed_by     TEXT,
    UNIQUE (year_id, number)
);
CREATE INDEX fiscal_periods_dates ON fiscal_periods (org_id, start_date, end_date);

-- The last guard: no entry lands in a closed period, whatever wrote it.
CREATE TRIGGER journal_entries_closed_period BEFORE INSERT ON journal_entries
WHEN EXISTS (SELECT 1 FROM fiscal_periods p WHERE p.org_id = NEW.org_id AND p.status = 'closed'
             AND NEW.date BETWEEN p.start_date AND p.end_date)
BEGIN SELECT RAISE(ABORT, 'period closed'); END;

CREATE INDEX journal_lines_account ON journal_lines (account_code);
CREATE INDEX journal_entries_date ON journal_entries (org_id, date);
