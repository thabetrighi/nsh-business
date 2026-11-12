-- Backups (M14.1): how and where the store backs itself up, and a log of
-- backups, checks and restores.

CREATE TABLE backup_settings (
    org_id          TEXT PRIMARY KEY REFERENCES organizations(id),
    -- `salt:key` (hex): the key derived from the owner's passphrase. It
    -- travels inside each backup, sealed by itself, so a backup file alone
    -- never opens without the passphrase. (M14.4 moves it to the system's
    -- keystore.)
    key             TEXT NOT NULL,
    schedule        TEXT NOT NULL CHECK (schedule IN ('off', 'daily', 'weekly')),
    keep            INTEGER NOT NULL CHECK (keep BETWEEN 1 AND 365),
    directory       TEXT NOT NULL,
    updated_at      INTEGER NOT NULL,
    updated_by      TEXT
);

CREATE TABLE backup_log (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    kind            TEXT NOT NULL CHECK (kind IN ('backup', 'verify', 'restore')),
    path            TEXT NOT NULL,
    size            INTEGER NOT NULL DEFAULT 0,
    ok              INTEGER NOT NULL CHECK (ok IN (0, 1)),
    message         TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    created_by      TEXT
);
CREATE INDEX backup_log_recent ON backup_log (org_id, kind, created_at);
CREATE TRIGGER backup_log_no_update BEFORE UPDATE ON backup_log
BEGIN SELECT RAISE(ABORT, 'the backup log is append-only'); END;
CREATE TRIGGER backup_log_no_delete BEFORE DELETE ON backup_log
BEGIN SELECT RAISE(ABORT, 'the backup log is append-only'); END;
