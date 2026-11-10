-- Organizations, branches, warehouses, users (M4.1 basis).
-- Mutable entities carry the common columns (M2.3):
-- id, org_id, created_at, updated_at, created_by, device_id, version, deleted_at.

CREATE TABLE organizations (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    country       TEXT NOT NULL,
    currency      TEXT NOT NULL,
    currency_exp  INTEGER NOT NULL,
    activity      TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER
);

CREATE TABLE branches (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    code          TEXT NOT NULL,
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER,
    UNIQUE (org_id, code)
);

CREATE TABLE warehouses (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    branch_id     TEXT NOT NULL REFERENCES branches(id),
    name          TEXT NOT NULL,
    negative_stock TEXT NOT NULL DEFAULT 'permission'
                  CHECK (negative_stock IN ('forbid', 'permission', 'allow')),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER
);

CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    name          TEXT NOT NULL,
    role          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER
);
