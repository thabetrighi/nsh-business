-- Accounting (M7): chart of accounts and journal. Posted entries are
-- immutable; corrections are reversals.

CREATE TABLE accounts (
    org_id        TEXT NOT NULL,
    code          TEXT NOT NULL,
    name          TEXT NOT NULL,
    parent_code   TEXT,
    kind          TEXT NOT NULL CHECK (kind IN ('asset', 'liability', 'equity', 'income', 'expense')),
    PRIMARY KEY (org_id, code)
);

CREATE TABLE journal_entries (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL,
    number        TEXT NOT NULL,
    date          TEXT NOT NULL,
    memo          TEXT NOT NULL,
    source        TEXT NOT NULL,
    reverses_id   TEXT REFERENCES journal_entries(id),
    total_minor   INTEGER NOT NULL,
    operation_id  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL
);

CREATE TABLE journal_lines (
    entry_id      TEXT NOT NULL REFERENCES journal_entries(id),
    position      INTEGER NOT NULL,
    account_code  TEXT NOT NULL,
    debit_minor   INTEGER NOT NULL CHECK (debit_minor >= 0),
    credit_minor  INTEGER NOT NULL CHECK (credit_minor >= 0),
    memo          TEXT,
    PRIMARY KEY (entry_id, position)
);

CREATE TRIGGER journal_entries_no_update BEFORE UPDATE ON journal_entries
BEGIN SELECT RAISE(ABORT, 'posted entries are immutable'); END;
CREATE TRIGGER journal_entries_no_delete BEFORE DELETE ON journal_entries
BEGIN SELECT RAISE(ABORT, 'posted entries are immutable'); END;
CREATE TRIGGER journal_lines_no_update BEFORE UPDATE ON journal_lines
BEGIN SELECT RAISE(ABORT, 'posted entries are immutable'); END;
CREATE TRIGGER journal_lines_no_delete BEFORE DELETE ON journal_lines
BEGIN SELECT RAISE(ABORT, 'posted entries are immutable'); END;
