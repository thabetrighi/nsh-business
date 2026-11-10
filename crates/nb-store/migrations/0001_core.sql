-- Core of the local engine: this device, idempotent operations, the
-- outbox for sync, the tamper-evident audit log, document counters.

CREATE TABLE device (
    id            TEXT PRIMARY KEY,
    prefix        TEXT NOT NULL CHECK (length(prefix) BETWEEN 1 AND 3),
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

-- One row per business operation (M2.6). Running an operation again with
-- the same id returns the stored result instead of doing it twice.
CREATE TABLE operations (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,
    org_id        TEXT,
    actor_id      TEXT,
    result        TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

-- Events written in the same transaction as the change (M2.5).
CREATE TABLE outbox_events (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    id            TEXT NOT NULL UNIQUE,
    operation_id  TEXT NOT NULL REFERENCES operations(id),
    org_id        TEXT,
    kind          TEXT NOT NULL,
    entity        TEXT NOT NULL,
    entity_id     TEXT NOT NULL,
    payload       TEXT NOT NULL,
    hlc           TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    sent_at       INTEGER
);
CREATE INDEX outbox_pending ON outbox_events(seq) WHERE sent_at IS NULL;

-- Audit log with a hash chain (M2.7): each row's hash covers its content
-- and the previous row's hash, so any edit or deletion breaks the chain.
CREATE TABLE audit_log (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id  TEXT NOT NULL,
    org_id        TEXT,
    actor_id      TEXT,
    action        TEXT NOT NULL,
    entity        TEXT NOT NULL,
    entity_id     TEXT NOT NULL,
    detail        TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    prev_hash     TEXT NOT NULL,
    hash          TEXT NOT NULL
);
CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;

-- Offline document numbering (M2.8): per kind, branch and year.
CREATE TABLE doc_counters (
    kind          TEXT NOT NULL,
    branch_id     TEXT NOT NULL,
    year          INTEGER NOT NULL,
    last          INTEGER NOT NULL,
    PRIMARY KEY (kind, branch_id, year)
);
